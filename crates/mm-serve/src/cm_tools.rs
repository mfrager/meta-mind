//! The conversation manager's tool surface.
//!
//! Exactly the seventeen reviewed schemas are advertised. Phase 2 implements
//! the five conversation-core handlers; the remaining twelve are visible stubs
//! so the model always sees the same tool contract. Every result is a full
//! [`ToolEnvelope`] carrying the post-call conversation state.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mm_agent::{
    AgentCall, AgentError, AgentKind, AgentOrigin, AgentRunStatus, AgentRuntime, ExternalWorkPort,
};
use mm_cme::{CmeError, FrameError, Result as CmeResult, ToolHandler};
use mm_data::{ConversationError, SessionError, StateOrigin, StatusValue};
use mm_llm::ToolSchema;
use serde_json::{json, Value};
use tokio::time::timeout;

use crate::cme::CONSOLE_USER;
use crate::data_tools::{is_data_tool, DataToolError, DataTools};
use crate::metamind_tools::{is_metamind_tool, MetamindToolError, MetamindTools};
use crate::state_service::{StateService, StateServiceError};
use crate::tool_schemas::{
    IndexStatus, ToolArgumentError, ToolCatalog, ToolEnvelope, ToolErrorBody, ToolResultEnvelope,
};

/// State reads/writes must not stall a turn.
const STATE_TOOL_TIMEOUT: Duration = Duration::from_secs(2);
/// A downstream agent turn gets the design's one-turn budget.
const AGENT_TOOL_TIMEOUT: Duration = Duration::from_secs(125);
/// A data write is local RDF plus one index commit.
const DATA_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// Reads are a query or a bounded search.
const DATA_READ_TIMEOUT: Duration = Duration::from_secs(5);
/// A stub still returns quickly with a typed error.
const STUB_TOOL_TIMEOUT: Duration = Duration::from_secs(5);

/// Keys owned by the session/frame system; `conversation.status` may not write
/// them through its arbitrary-values path.
const TOPOLOGY_KEYS: &[&str] = &[
    "topic",
    "depth",
    "working",
    "presence",
    "lifecycle",
    "expires_at",
    "session_id",
    "revision",
    "traces",
    "main_trace",
    "frame_stack",
    "frame_id",
    "frame_template",
    "frames",
    "stack",
];

/// The CM tool handler.
#[derive(Debug)]
pub struct CmTools {
    state: Arc<StateService>,
    data: Option<DataTools>,
    metamind: Option<MetamindTools>,
    agent: Option<Arc<AgentRuntime>>,
    session_id: Option<String>,
}

impl CmTools {
    /// Build over the shared state service.
    pub fn new(state: Arc<StateService>) -> Self {
        Self {
            state,
            data: None,
            metamind: None,
            agent: None,
            session_id: None,
        }
    }

    /// Attach the managed-data service, enabling the seven `data.*` tools.
    pub fn with_data(mut self, service: Arc<mm_data::DataService>) -> Self {
        self.data = Some(DataTools::new(service));
        self
    }

    /// Attach the downstream agent runtime, enabling `agent.run`.
    pub fn with_agent(mut self, agent: Arc<AgentRuntime>) -> Self {
        if let Some(metamind) = &mut self.metamind {
            metamind.set_agent(Arc::clone(&agent));
        }
        self.agent = Some(agent);
        self
    }

    /// Attach the metamind services, enabling the five `metamind.*` tools.
    pub fn with_metamind(
        mut self,
        engine: Arc<mm_data::RdfEngine>,
        data: Arc<mm_data::DataService>,
        prompts: Arc<mm_prompt::PromptStore>,
        client: Option<Arc<dyn mm_llm::LlmClient>>,
        model: Option<String>,
    ) -> Self {
        self.metamind = Some(MetamindTools::new(
            engine,
            data,
            Arc::clone(&self.state),
            prompts,
            client,
            model,
        ));
        self
    }

    /// Bind a default session, for callers that do not use the executor's
    /// session-aware dispatch.
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// The state service behind the tools.
    pub fn state(&self) -> &Arc<StateService> {
        &self.state
    }

    /// The default session bound for direct tool calls.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// The advertised schemas, without requiring the trait in scope.
    pub fn schemas(&self) -> Vec<ToolSchema> {
        <Self as ToolHandler>::schemas(self)
    }

    fn session<'a>(&'a self, supplied: Option<&'a str>) -> CmeResult<&'a str> {
        supplied
            .or(self.session_id.as_deref())
            .ok_or_else(|| CmeError::Tool("conversation tools require a session id".into()))
    }

    async fn dispatch(
        &self,
        session_id: &str,
        name: &str,
        arguments: &Value,
        action: Option<&str>,
    ) -> std::result::Result<Value, ToolFailure> {
        match name {
            "conversation.topic" => self.topic(session_id, arguments, action).await,
            "conversation.depth" => self.depth(session_id, arguments, action).await,
            "conversation.status" => self.status(session_id, arguments, action).await,
            "conversation.frame" => self.frame(session_id, arguments, action).await,
            "agent.run" => self.agent_run(session_id, arguments).await,
            name if is_data_tool(name) => match &self.data {
                Some(data) => data
                    .dispatch(name, arguments)
                    .await
                    .map_err(ToolFailure::from),
                None => Err(ToolFailure::new(
                    "NOT_AVAILABLE",
                    format!("{name} requires the managed-data service"),
                )),
            },
            name if is_metamind_tool(name) => match &self.metamind {
                Some(metamind) => metamind
                    .dispatch(session_id, name, arguments)
                    .await
                    .map_err(ToolFailure::from),
                None => Err(ToolFailure::new(
                    "NOT_AVAILABLE",
                    format!("{name} requires the metamind services"),
                )),
            },
            _ => Err(ToolFailure::new(
                "NOT_AVAILABLE",
                format!("{name} is not available in the conversation core yet"),
            )),
        }
    }

    async fn topic(
        &self,
        session_id: &str,
        arguments: &Value,
        action: Option<&str>,
    ) -> std::result::Result<Value, ToolFailure> {
        let action = action.unwrap_or("get");
        match action {
            "get" => {
                let state = self.state.conversation(session_id).await?;
                Ok(json!({ "topic": state.topic, "depth": state.depth }))
            }
            "set" => {
                let expected = expected_revision(arguments)?;
                let topic = string_field(arguments, "topic")?.to_owned();
                let before = self.state.conversation(session_id).await?.topic;
                let state = self
                    .state
                    .set_topic(
                        session_id,
                        Some(topic),
                        expected,
                        StateOrigin::Cm,
                        mm_data::rdf::now_ms(),
                    )
                    .await?;
                Ok(json!({
                    "topic": state.topic,
                    "depth": state.depth,
                    "unchanged": before == state.topic,
                }))
            }
            other => Err(ToolFailure::invalid(
                Some("action"),
                format!("unsupported action {other}"),
            )),
        }
    }

    async fn depth(
        &self,
        session_id: &str,
        arguments: &Value,
        action: Option<&str>,
    ) -> std::result::Result<Value, ToolFailure> {
        let action = action.unwrap_or("get");
        match action {
            "get" => {
                let state = self.state.conversation(session_id).await?;
                Ok(json!({ "topic": state.topic, "depth": state.depth }))
            }
            "set" => {
                let expected = expected_revision(arguments)?;
                let depth = arguments
                    .get("depth")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| ToolFailure::missing("depth"))?;
                let depth = u8::try_from(depth)
                    .map_err(|_| ToolFailure::invalid(Some("depth"), "depth is out of range"))?;
                let before = self.state.conversation(session_id).await?.depth;
                let state = self
                    .state
                    .set_depth(
                        session_id,
                        depth,
                        expected,
                        StateOrigin::Cm,
                        mm_data::rdf::now_ms(),
                    )
                    .await?;
                Ok(json!({
                    "topic": state.topic,
                    "depth": state.depth,
                    "unchanged": before == state.depth,
                }))
            }
            other => Err(ToolFailure::invalid(
                Some("action"),
                format!("unsupported action {other}"),
            )),
        }
    }

    async fn status(
        &self,
        session_id: &str,
        arguments: &Value,
        action: Option<&str>,
    ) -> std::result::Result<Value, ToolFailure> {
        let action = action.unwrap_or("get");
        match action {
            "get" => {
                let state = self.state.conversation(session_id).await?;
                Ok(json!({
                    "status": state.status,
                    "count": state.status.len(),
                    "bytes": state.status_bytes().map_err(ToolFailure::from)?,
                }))
            }
            "set" => {
                let expected = expected_revision(arguments)?;
                let values = arguments
                    .get("values")
                    .and_then(Value::as_object)
                    .ok_or_else(|| ToolFailure::missing("values"))?;
                let mut updates = BTreeMap::new();
                for (key, value) in values {
                    if TOPOLOGY_KEYS.contains(&key.as_str()) {
                        return Err(ToolFailure::invalid(
                            Some(&format!("values.{key}")),
                            format!("{key} is not writable through conversation.status"),
                        ));
                    }
                    let value: StatusValue =
                        serde_json::from_value(value.clone()).map_err(|_| {
                            ToolFailure::invalid(
                                Some(&format!("values.{key}")),
                                "status value has an unsupported shape",
                            )
                        })?;
                    updates.insert(key.clone(), value);
                }
                self.validate_frame_status(session_id, &updates).await?;
                let before = self.state.conversation(session_id).await?.status;
                let state = self
                    .state
                    .update_status(
                        session_id,
                        updates.clone(),
                        &[],
                        expected,
                        StateOrigin::Cm,
                        mm_data::rdf::now_ms(),
                    )
                    .await?;
                Ok(json!({
                    "status": state.status,
                    "count": state.status.len(),
                    "bytes": state.status_bytes().map_err(ToolFailure::from)?,
                    "set": updates.keys().cloned().collect::<Vec<_>>(),
                    "unset": Vec::<String>::new(),
                    "unchanged": before == state.status,
                }))
            }
            "unset" => {
                let expected = expected_revision(arguments)?;
                let keys = string_array(arguments, "keys")?;
                let before = self.state.conversation(session_id).await?.status;
                let state = self
                    .state
                    .update_status(
                        session_id,
                        BTreeMap::new(),
                        &keys,
                        expected,
                        StateOrigin::Cm,
                        mm_data::rdf::now_ms(),
                    )
                    .await?;
                Ok(json!({
                    "status": state.status,
                    "count": state.status.len(),
                    "bytes": state.status_bytes().map_err(ToolFailure::from)?,
                    "set": Vec::<String>::new(),
                    "unset": keys,
                    "unchanged": before == state.status,
                }))
            }
            other => Err(ToolFailure::invalid(
                Some("action"),
                format!("unsupported action {other}"),
            )),
        }
    }

    async fn validate_frame_status(
        &self,
        session_id: &str,
        values: &BTreeMap<String, StatusValue>,
    ) -> std::result::Result<(), ToolFailure> {
        if let Some(StatusValue::Integer(attempt)) = values.get("frame_attempt") {
            if !(0..=5).contains(attempt) {
                return Err(ToolFailure::invalid(
                    Some("values.frame_attempt"),
                    "frame_attempt must be between 0 and 5",
                ));
            }
        }
        if let Some(StatusValue::Text(step)) = values.get("frame_step") {
            let current = self.state.current_frame_step(session_id).await?;
            if step != &current {
                return Err(ToolFailure::invalid(
                    Some("values.frame_step"),
                    format!("frame_step {step:?} does not match the top frame step {current:?}"),
                ));
            }
        }
        Ok(())
    }

    async fn frame(
        &self,
        session_id: &str,
        arguments: &Value,
        action: Option<&str>,
    ) -> std::result::Result<Value, ToolFailure> {
        let action = action.unwrap_or("get");
        match action {
            "get" => Ok(frame_result(&self.state.load(session_id).await?)),
            "push" | "replace" => {
                let expected = expected_revision(arguments)?;
                let template = string_field(arguments, "template_id")?.to_owned();
                let variables = string_map(arguments, "variables")?;
                if let Some(key) = variables
                    .keys()
                    .find(|key| TOPOLOGY_KEYS.contains(&key.as_str()))
                {
                    return Err(ToolFailure::invalid(
                        Some(&format!("variables.{key}")),
                        format!("{key} is not a frame variable"),
                    ));
                }
                let before = self.state.load(session_id).await?;
                if action == "push" {
                    self.state
                        .push_frame(session_id, &template, expected, mm_data::rdf::now_ms())
                        .await?;
                } else {
                    self.state
                        .replace_frame(session_id, &template, expected, mm_data::rdf::now_ms())
                        .await?;
                }
                if !variables.is_empty() {
                    let updates = variables
                        .into_iter()
                        .map(|(key, value)| (key, StatusValue::Text(value)))
                        .collect();
                    let current = self.state.conversation(session_id).await?;
                    self.state
                        .update_status(
                            session_id,
                            updates,
                            &[],
                            current.revision,
                            StateOrigin::Cm,
                            mm_data::rdf::now_ms(),
                        )
                        .await?;
                }
                let bundle = self.state.load(session_id).await?;
                if bundle.frames.revision == before.frames.revision {
                    return Err(ToolFailure::invalid(
                        Some("template_id"),
                        "the frame stack did not change",
                    ));
                }
                Ok(frame_result(&bundle))
            }
            "pop" => {
                let expected = expected_revision(arguments)?;
                self.state
                    .pop_frame(session_id, expected, mm_data::rdf::now_ms())
                    .await?;
                Ok(frame_result(&self.state.load(session_id).await?))
            }
            "schedule" => {
                let expected = expected_revision(arguments)?;
                let delay = arguments.get("delay_ms").and_then(Value::as_u64);
                let reason = arguments
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("step");
                self.state
                    .schedule_utterance(session_id, delay, reason, expected, mm_data::rdf::now_ms())
                    .await?;
                Ok(frame_result(&self.state.load(session_id).await?))
            }
            "cancel" => {
                let expected = expected_revision(arguments)?;
                self.state
                    .cancel_utterance(session_id, expected, mm_data::rdf::now_ms())
                    .await?;
                Ok(frame_result(&self.state.load(session_id).await?))
            }
            other => Err(ToolFailure::invalid(
                Some("action"),
                format!("unsupported action {other}"),
            )),
        }
    }

    async fn agent_run(
        &self,
        session_id: &str,
        arguments: &Value,
    ) -> std::result::Result<Value, ToolFailure> {
        let input = string_field(arguments, "input")?.trim().to_owned();
        if input.is_empty() {
            return Err(ToolFailure::invalid(
                Some("input"),
                "agent input must not be empty",
            ));
        }
        if input.chars().count() > 8_000 {
            return Err(ToolFailure::invalid(
                Some("input"),
                "agent input exceeds 8000 characters",
            ));
        }
        let kind = match arguments.get("agent").and_then(Value::as_str) {
            None | Some("hermes") => AgentKind::Hermes,
            Some("pi") => AgentKind::Pi,
            Some(other) => {
                return Err(ToolFailure::invalid(
                    Some("agent"),
                    format!("unknown agent {other}"),
                ))
            }
        };
        let run_id = arguments
            .get("run_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let workspace = arguments
            .get("workspace")
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from);
        let agent = self.agent.as_ref().ok_or_else(|| {
            ToolFailure::new(
                "NOT_AVAILABLE",
                "agent.run requires the downstream agent runtime",
            )
        })?;
        let result = agent
            .run(AgentCall {
                input,
                run_id,
                kind,
                workspace,
                parent_session: session_id.to_owned(),
                user_id: CONSOLE_USER.to_owned(),
                origin: AgentOrigin::Cm,
            })
            .await
            .map_err(agent_failure)?;
        Ok(json!({
            "runId": result.run_id,
            "kind": match kind {
                AgentKind::Hermes => "hermes",
                AgentKind::Pi => "pi",
            },
            "status": serde_json::to_value(result.status).unwrap_or_else(|_| json!("unknown")),
            "turn": result.turn,
            "workspace": result.workspace.display().to_string(),
            "next": next_label(result.status, result.next.as_deref()),
            "message": result.message,
            "compactSummary": result.summary,
            "summary": result.summary_detail,
            "artifacts": result.artifacts,
            "toolCalls": result.tool_calls,
            "usage": {
                "promptTokens": result.usage.prompt_tokens,
                "completionTokens": result.usage.completion_tokens,
            },
            "streamed": result.streamed,
            "model": result.model,
            "rawRef": result.raw_ref,
            "error": result.error,
        }))
    }

    async fn envelope(
        &self,
        session_id: &str,
        tool: &str,
        action: Option<String>,
        outcome: std::result::Result<Value, ToolFailure>,
    ) -> ToolEnvelope {
        let (state_revision, state) = match self.state.load(session_id).await {
            Ok(bundle) => (
                bundle.conversation.revision,
                serde_json::to_value(&bundle.conversation).unwrap_or(Value::Null),
            ),
            Err(_) => (0, Value::Null),
        };
        let mut envelope = match outcome {
            Ok(result) => ToolEnvelope::success(tool, action, state_revision, state, result),
            Err(failure) => {
                ToolEnvelope::failure(tool, action, state_revision, state, failure.body())
            }
        };
        if let Some(result) = &envelope.result {
            if let Some(revision) = result.get("dataRevision").and_then(Value::as_u64) {
                envelope.data_revision = Some(revision);
            }
            if let Some(pending) = result.get("indexPending").and_then(Value::as_bool) {
                envelope.index = Some(IndexStatus {
                    pending: u64::from(pending),
                    lag_ms: 0,
                });
            }
        }
        envelope
    }
}

#[async_trait]
impl ToolHandler for CmTools {
    fn schemas(&self) -> Vec<ToolSchema> {
        ToolCatalog::embedded()
            .map(|catalog| {
                catalog
                    .schemas()
                    .iter()
                    .map(|schema| ToolSchema {
                        name: schema.name.clone(),
                        description: schema.description.clone(),
                        parameters: schema.parameters.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn call(&self, name: &str, arguments: &str) -> CmeResult<String> {
        let session = self.session(None)?;
        self.call_for_session(session, name, arguments).await
    }

    async fn call_for_session(
        &self,
        session_id: &str,
        name: &str,
        arguments: &str,
    ) -> CmeResult<String> {
        let now = mm_data::rdf::now_ms();
        if let Err(error) = self
            .state
            .ensure_session(session_id, CONSOLE_USER, now)
            .await
        {
            return serde_json::to_string(&ToolEnvelope::failure(
                name,
                None,
                0,
                Value::Null,
                ToolFailure::from(error).body(),
            ))
            .map_err(|error| CmeError::Tool(error.to_string()));
        }

        let action = action_of(arguments);
        let catalog = match ToolCatalog::embedded() {
            Ok(catalog) => catalog,
            Err(error) => {
                let envelope = self
                    .envelope(
                        session_id,
                        name,
                        action,
                        Err(ToolFailure::new("INTERNAL", error.to_string())),
                    )
                    .await;
                return serde_json::to_string(&envelope)
                    .map_err(|error| CmeError::Tool(error.to_string()));
            }
        };
        let parsed = match catalog.parse(name, arguments) {
            Ok(parsed) => parsed,
            Err(error) => {
                let envelope = self
                    .envelope(
                        session_id,
                        name,
                        action,
                        Err(ToolFailure::from_argument(error)),
                    )
                    .await;
                return serde_json::to_string(&envelope)
                    .map_err(|error| CmeError::Tool(error.to_string()));
            }
        };

        let budget = match name {
            "agent.run" => AGENT_TOOL_TIMEOUT,
            "conversation.status"
            | "conversation.topic"
            | "conversation.depth"
            | "conversation.frame" => STATE_TOOL_TIMEOUT,
            "data.create" | "data.update" | "data.delete" | "data.schema" | "data.namespace" => {
                DATA_WRITE_TIMEOUT
            }
            "data.query" | "data.search" => DATA_READ_TIMEOUT,
            "metamind.plan" | "metamind.think" => Duration::from_secs(90),
            "metamind.memory" => match action.as_deref() {
                Some("remember") | Some("forget") => DATA_WRITE_TIMEOUT,
                _ => DATA_READ_TIMEOUT,
            },
            "metamind.log" => match action.as_deref() {
                Some("append") => DATA_WRITE_TIMEOUT,
                _ => DATA_READ_TIMEOUT,
            },
            "metamind.act" => match action.as_deref() {
                Some("plan_start")
                | Some("plan_stop")
                | Some("plan_pause")
                | Some("plan_resume")
                | Some("plan_retry_step")
                | Some("plan_advance_step")
                | Some("plan_complete")
                | Some("plan_abandon")
                | Some("cancel") => DATA_READ_TIMEOUT,
                _ => Duration::from_secs(30),
            },
            _ => STUB_TOOL_TIMEOUT,
        };
        let action_for_call = action.clone();
        let result = match timeout(
            budget,
            self.dispatch(session_id, name, &parsed, action_for_call.as_deref()),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(ToolFailure::new(
                "TIMEOUT",
                format!("tool {name} exceeded its {}s timeout", budget.as_secs()),
            )
            .retryable()),
        };
        let envelope = self.envelope(session_id, name, action, result).await;
        serde_json::to_string(&envelope).map_err(|error| CmeError::Tool(error.to_string()))
    }
}

fn frame_result(bundle: &mm_data::SessionBundle) -> Value {
    json!({
        "stack": bundle.frames,
        "top": bundle.frames.top(),
        "utterance": bundle.session.pending_utterance,
    })
}

fn action_of(raw: &str) -> Option<String> {
    serde_json::from_str::<Value>(raw).ok().and_then(|value| {
        value
            .get("action")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

fn expected_revision(arguments: &Value) -> std::result::Result<u64, ToolFailure> {
    arguments
        .get("expected_revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| ToolFailure::missing("expected_revision"))
}

fn string_field<'a>(
    arguments: &'a Value,
    field: &str,
) -> std::result::Result<&'a str, ToolFailure> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::missing(field))
}

fn string_array(arguments: &Value, field: &str) -> std::result::Result<Vec<String>, ToolFailure> {
    let values = arguments
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| ToolFailure::missing(field))?;
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(
            value
                .as_str()
                .ok_or_else(|| {
                    ToolFailure::invalid(Some(field), format!("{field} must contain strings"))
                })?
                .to_owned(),
        );
    }
    Ok(out)
}

fn string_map(
    arguments: &Value,
    field: &str,
) -> std::result::Result<BTreeMap<String, String>, ToolFailure> {
    let Some(values) = arguments.get(field) else {
        return Ok(BTreeMap::new());
    };
    let Some(values) = values.as_object() else {
        return Err(ToolFailure::invalid(Some(field), "must be an object"));
    };
    let mut out = BTreeMap::new();
    for (key, value) in values {
        out.insert(
            key.clone(),
            value
                .as_str()
                .ok_or_else(|| ToolFailure::invalid(Some(field), "values must be strings"))?
                .to_owned(),
        );
    }
    Ok(out)
}

/// A typed failure converted to the envelope body.
#[derive(Debug, Clone)]
struct ToolFailure {
    code: String,
    message: String,
    field: Option<String>,
    retryable: bool,
    next: Option<String>,
}

impl ToolFailure {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            field: None,
            retryable: false,
            next: None,
        }
    }

    fn invalid(field: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_ARGUMENTS".into(),
            message: message.into(),
            field: field.map(str::to_owned),
            retryable: false,
            next: Some("fix the call and retry".into()),
        }
    }

    fn missing(field: &str) -> Self {
        Self::invalid(Some(field), "missing required field")
    }

    fn retryable(mut self) -> Self {
        self.retryable = true;
        self.next = Some("retry the call".into());
        self
    }

    fn from_argument(error: ToolArgumentError) -> Self {
        Self {
            code: error.code().into(),
            message: error.to_string(),
            field: match &error {
                ToolArgumentError::InvalidArguments { field, .. } => field.clone(),
                _ => None,
            },
            retryable: error.retryable(),
            next: Some("fix the call and retry".into()),
        }
    }

    fn body(self) -> ToolErrorBody {
        ToolErrorBody {
            code: self.code,
            message: self.message,
            field: self.field,
            retryable: self.retryable,
            next: self.next,
        }
    }
}

impl From<MetamindToolError> for ToolFailure {
    fn from(error: MetamindToolError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            field: error.field,
            retryable: error.retryable,
            next: error.next,
        }
    }
}

impl From<DataToolError> for ToolFailure {
    fn from(error: DataToolError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            field: error.field,
            retryable: error.retryable,
            next: error.next,
        }
    }
}

impl From<StateServiceError> for ToolFailure {
    fn from(error: StateServiceError) -> Self {
        match error {
            StateServiceError::Conversation(error) => from_conversation(error),
            StateServiceError::Frame(error) => from_frame(error),
            StateServiceError::Session(error) => from_session(error),
            StateServiceError::Missing(session) => {
                Self::new("SESSION_NOT_FOUND", format!("session not found: {session}"))
            }
            StateServiceError::Deleted(session) => {
                Self::new("SESSION_DELETED", format!("session deleted: {session}"))
            }
            StateServiceError::NotOpen(session) => {
                Self::new("SESSION_ENDED", format!("session is not active: {session}"))
            }
            other => Self::new("INTERNAL", other.to_string()),
        }
    }
}

impl From<ConversationError> for ToolFailure {
    fn from(error: ConversationError) -> Self {
        from_conversation(error)
    }
}

impl From<ToolFailure> for ToolErrorBody {
    fn from(failure: ToolFailure) -> Self {
        failure.body()
    }
}

fn next_label(status: AgentRunStatus, next: Option<&str>) -> String {
    match status {
        AgentRunStatus::Running | AgentRunStatus::Waiting => next.unwrap_or("continue").to_owned(),
        AgentRunStatus::WaitingInput => "wait_for_user".into(),
        AgentRunStatus::Done => "done".into(),
        AgentRunStatus::Failed => "failed".into(),
        AgentRunStatus::Cancelled => "cancelled".into(),
        AgentRunStatus::Queued => "continue".into(),
    }
}

fn agent_failure(error: AgentError) -> ToolFailure {
    match error {
        AgentError::Unavailable(reason) => ToolFailure {
            code: "UNAVAILABLE".into(),
            message: format!("agent backend unavailable: {reason}"),
            field: None,
            retryable: true,
            next: Some("start or check the configured Hermes service and retry".into()),
        },
        AgentError::Rejected(reason) => {
            ToolFailure::invalid(None, format!("agent request rejected: {reason}"))
        }
        AgentError::Timeout(ms) => ToolFailure {
            code: "TIMEOUT".into(),
            message: format!("agent turn timed out after {ms} ms"),
            field: None,
            retryable: true,
            next: Some(
                "call agent.run again with the same run_id to check the turn; it will not start a duplicate"
                    .into(),
            ),
        },
        AgentError::NotFound { run_id, active } => ToolFailure {
            code: "NOT_FOUND".into(),
            message: format!("unknown agent run {run_id}"),
            field: Some("run_id".into()),
            retryable: false,
            next: Some(if active.is_empty() {
                "omit run_id to start a new run".into()
            } else {
                format!("active runs: {}", active.join(", "))
            }),
        },
        AgentError::RunTerminal(run) => ToolFailure {
            code: "RUN_TERMINAL".into(),
            message: format!("agent run {} is already terminal", run.run_id),
            field: Some("run_id".into()),
            retryable: false,
            next: Some("omit run_id to start a new run".into()),
        },
        AgentError::Budget(reason) => ToolFailure::new("BUDGET", reason),
        AgentError::WorkspaceEscape { path, root } => ToolFailure::new(
            "FORBIDDEN",
            format!(
                "workspace {} is outside the allowed root {}",
                path.display(),
                root.display()
            ),
        ),
        AgentError::Prompt(reason) => ToolFailure::new("PROMPT_UNAVAILABLE", reason),
        AgentError::Other(reason) => ToolFailure::new("INTERNAL", reason),
    }
}

fn from_conversation(error: ConversationError) -> ToolFailure {
    match error {
        ConversationError::Conflict { expected, current } => ToolFailure {
            code: "CONFLICT".into(),
            message: format!("expected revision {expected}, current revision {current}"),
            field: Some("expected_revision".into()),
            retryable: true,
            next: Some("re-read the state block and retry once".into()),
        },
        ConversationError::StateTooLarge { bytes, keys } => {
            ToolFailure::new("STATE_TOO_LARGE", format!("{keys} keys / {bytes} bytes"))
        }
        ConversationError::InvalidArguments(message) => ToolFailure::invalid(None, message),
        ConversationError::FrameLimit => {
            ToolFailure::new("FRAME_LIMIT", "frame stack limit reached")
        }
        ConversationError::FrameInvalid(message) => ToolFailure::new("FRAME_INVALID", message),
        ConversationError::SessionEnded => ToolFailure::new("SESSION_ENDED", "session ended"),
        other => ToolFailure::new("INTERNAL", other.to_string()),
    }
}

fn from_frame(error: FrameError) -> ToolFailure {
    match error {
        FrameError::UnknownTemplate(template) => ToolFailure::new(
            "FRAME_INVALID",
            format!("unknown frame template {template}"),
        ),
        FrameError::UnknownTransition {
            template,
            transition,
        } => ToolFailure::new(
            "FRAME_INVALID",
            format!("template {template} has no transition {transition}"),
        ),
        FrameError::UnknownKind(kind) => {
            ToolFailure::new("FRAME_INVALID", format!("unknown frame kind {kind}"))
        }
        FrameError::AttemptsExhausted { template, attempts } => ToolFailure::new(
            "FRAME_LIMIT",
            format!("template {template} exhausted its {attempts} attempts"),
        ),
        FrameError::Session(error) => from_session(error),
        FrameError::Prompt(error) => ToolFailure::new("PROMPT_UNAVAILABLE", error.to_string()),
        FrameError::Invalid(message) => ToolFailure::new("FRAME_INVALID", message),
    }
}

fn from_session(error: SessionError) -> ToolFailure {
    match error {
        SessionError::Conflict { expected, current } => ToolFailure {
            code: "CONFLICT".into(),
            message: format!("expected revision {expected}, current revision {current}"),
            field: Some("expected_revision".into()),
            retryable: true,
            next: Some("re-read the state block and retry once".into()),
        },
        SessionError::FrameLimit => ToolFailure::new("FRAME_LIMIT", "frame stack limit reached"),
        SessionError::FrameInvalid(message) => ToolFailure::new("FRAME_INVALID", message),
        SessionError::NotFound(session) => {
            ToolFailure::new("SESSION_NOT_FOUND", format!("session not found: {session}"))
        }
        SessionError::NotOpen(session) => {
            ToolFailure::new("SESSION_ENDED", format!("session is not active: {session}"))
        }
        SessionError::Invalid(message) => ToolFailure::invalid(None, message),
        other => ToolFailure::new("INTERNAL", other.to_string()),
    }
}

/// The full-state envelope is a type alias in the design; keep the name
/// available from this module too.
pub type CmToolEnvelope = ToolResultEnvelope;
