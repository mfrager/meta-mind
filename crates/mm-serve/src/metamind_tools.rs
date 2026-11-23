//! `metamind.plan`, `think`, `memory`, `log`, and `act` handlers.
//!
//! The handlers are deliberately thin: storage lives in `mm-data`, the bounded
//! sub-sessions live in `mm-cme`, and this module maps strict JSON arguments to
//! typed service calls. No handler accepts raw SPARQL, SQL, or shell.

use std::sync::Arc;

use mm_agent::AgentRuntime;
use mm_cme::{control_request, IntrospectionService, PlanService, SubagentError, ThinkService};
use mm_data::{
    contains_secret, ActionError, ActionReceipt, ActionStore, AgentstreamEvent, AgentstreamStore,
    DataService, EventProvenance, EventType, FormValue, FormValues, InternalActionRegistry,
    LogEntry, LogFilter, LogLevel, MemoryKind, MemoryScope, MemoryService, PlanControl, PlanError,
    PlanFormat, PlanStore, RdfLogService, RdfMemoryService, RdfPlanStore, Recurrence, ScheduleSpec,
    ScheduledAction, ScheduledActionSpec, ScheduledActionStatus, SessionMetadata, SessionTrace,
    TraceKind,
};
use mm_llm::LlmClient;
use mm_prompt::PromptStore;
use serde_json::{json, Value};

use crate::cme::CONSOLE_USER;
use crate::state_service::StateService;

/// The five metamind tool names.
pub const METAMIND_TOOLS: &[&str] = &[
    "metamind.plan",
    "metamind.think",
    "metamind.memory",
    "metamind.log",
    "metamind.act",
];

/// Whether `name` belongs to the metamind family.
pub fn is_metamind_tool(name: &str) -> bool {
    METAMIND_TOOLS.contains(&name)
}

/// The metamind tools and their services.
pub struct MetamindTools {
    data: Arc<DataService>,
    state: Arc<StateService>,
    plans: Arc<RdfPlanStore>,
    plan_service: PlanService,
    think_service: ThinkService,
    introspection: IntrospectionService,
    memory: Arc<dyn MemoryService>,
    actions: Arc<ActionStore>,
    logs: Arc<RdfLogService>,
    agentstream: Arc<AgentstreamStore>,
    agent: Option<Arc<AgentRuntime>>,
}

impl std::fmt::Debug for MetamindTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetamindTools").finish_non_exhaustive()
    }
}

impl MetamindTools {
    /// Build the services over the initialized data layer.
    pub fn new(
        engine: Arc<mm_data::RdfEngine>,
        data: Arc<DataService>,
        state: Arc<StateService>,
        prompts: Arc<PromptStore>,
        client: Option<Arc<dyn LlmClient>>,
        model: Option<String>,
    ) -> Self {
        let plans = Arc::new(RdfPlanStore::new(Arc::clone(&engine)));
        let memory: Arc<dyn MemoryService> = Arc::new(RdfMemoryService::new(Arc::clone(&engine)));
        let actions = Arc::new(ActionStore::new(
            Arc::clone(&engine),
            Arc::new(InternalActionRegistry::builtin()),
        ));
        let logs = Arc::new(RdfLogService::new(Arc::clone(&engine)));
        let agentstream = Arc::clone(state.agentstream());
        let plan_service = PlanService::new(
            Arc::clone(&data),
            Arc::clone(&plans),
            Arc::clone(&prompts),
            client.clone(),
            model.clone(),
        );
        let think_service = ThinkService::new(
            Arc::clone(&data),
            Arc::clone(&memory),
            prompts,
            client,
            model,
        );
        let introspection =
            IntrospectionService::new(Arc::clone(&data), Arc::clone(&plans), Arc::clone(&memory));
        Self {
            data,
            state,
            plans,
            plan_service,
            think_service,
            introspection,
            memory,
            actions,
            logs,
            agentstream,
            agent: None,
        }
    }

    /// Attach the agent runtime so `plan_stop` can cancel plan-owned runs.
    pub fn set_agent(&mut self, agent: Arc<AgentRuntime>) {
        self.agent = Some(agent);
    }

    /// Route `metamind.think` through the verified three-pass cycle.
    pub fn set_think_runner(&self, runner: Arc<mm_cme::ThreePassRunner>) {
        self.think_service.install_runner(runner);
    }

    /// Dispatch one tool call.
    pub async fn dispatch(
        &self,
        session_id: &str,
        name: &str,
        arguments: &Value,
    ) -> Result<Value, MetamindToolError> {
        match name {
            "metamind.plan" => self.plan(session_id, arguments).await,
            "metamind.think" => self.think(session_id, arguments).await,
            "metamind.memory" => self.memory(arguments).await,
            "metamind.log" => self.log(session_id, arguments).await,
            "metamind.act" => self.act(session_id, arguments).await,
            other => Err(MetamindToolError::new(
                "INVALID_ARGUMENTS",
                format!("unknown metamind tool {other}"),
            )),
        }
    }

    async fn plan(&self, session_id: &str, arguments: &Value) -> Result<Value, MetamindToolError> {
        let action = string_arg(arguments, "action")?;
        match action {
            "create" => {
                let goal = opt_str(arguments, "goal").unwrap_or_default();
                let content = arguments.get("content");
                if goal.trim().is_empty() && content.is_none() {
                    return Err(MetamindToolError::missing("goal"));
                }
                let format = plan_format(arguments)?;
                let result = self
                    .plan_service
                    .create(
                        session_id,
                        if goal.trim().is_empty() {
                            "Plan from supplied content"
                        } else {
                            goal
                        },
                        format,
                        content,
                        opt_str(arguments, "instructions"),
                    )
                    .await
                    .map_err(map_subagent)?;
                let mut value = result.to_json();
                if let Some(object) = value.as_object_mut() {
                    object.insert("versions".into(), json!([result.version]));
                }
                self.register_trace(
                    session_id,
                    TraceKind::Plan,
                    &format!("plan/{}", short_id(&result.plan.id)),
                    result.plan.created_at,
                )
                .await;
                Ok(value)
            }
            "update" => {
                let plan_id = string_arg(arguments, "plan_id")?;
                let expected = opt_u64(arguments, "expected_revision")?
                    .ok_or_else(|| MetamindToolError::missing("expected_revision"))?;
                let format = match arguments.get("format").and_then(Value::as_str) {
                    Some(value) => Some(PlanFormat::parse(value).ok_or_else(|| {
                        MetamindToolError::invalid("format", "invalid plan format")
                    })?),
                    None => None,
                };
                let result = self
                    .plan_service
                    .update(
                        plan_id,
                        expected,
                        opt_str(arguments, "goal"),
                        format,
                        arguments.get("content"),
                        opt_str(arguments, "instructions"),
                    )
                    .await
                    .map_err(map_subagent)?;
                let mut value = result.to_json();
                if let Some(object) = value.as_object_mut() {
                    object.insert("versions".into(), json!([result.version]));
                }
                self.register_trace(
                    session_id,
                    TraceKind::Plan,
                    &format!("plan/{}/{}", short_id(plan_id), mm_data::rdf::new_id()),
                    mm_data::rdf::now_ms(),
                )
                .await;
                Ok(value)
            }
            "get" => {
                let plan_id = string_arg(arguments, "plan_id")?;
                let plan = self
                    .plan_service
                    .get(plan_id)
                    .await
                    .map_err(map_subagent)?
                    .ok_or_else(|| MetamindToolError::new("NOT_FOUND", "plan not found"))?;
                let versions = self
                    .plans
                    .versions(plan_id)
                    .await
                    .map_err(|error| MetamindToolError::new("INTERNAL", error.to_string()))?;
                Ok(json!({ "plan": plan, "versions": versions }))
            }
            "list" => {
                let plans = self
                    .plan_service
                    .list(session_id)
                    .await
                    .map_err(map_subagent)?;
                Ok(json!({ "plans": plans, "count": plans.len() }))
            }
            other => Err(MetamindToolError::invalid(
                "action",
                format!("unsupported plan action {other:?}"),
            )),
        }
    }

    async fn think(&self, session_id: &str, arguments: &Value) -> Result<Value, MetamindToolError> {
        let question = string_arg(arguments, "question")?;
        let focus = string_array(arguments, "focus");
        let think_id = format!("think/{session_id}/{}", mm_data::rdf::new_id());
        let mut result = self
            .think_service
            .think(question, opt_str(arguments, "context"), &focus, &think_id)
            .await
            .map_err(map_subagent)?;
        result.think_id = think_id.clone();
        if arguments
            .get("persist")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let payload = serde_json::to_value(&result)
                .map_err(|error| MetamindToolError::new("INTERNAL", error.to_string()))?;
            mm_data::rdf::upsert(
                self.data.engine(),
                &mm_data::graphs::resolve_graph("memory"),
                &format!("https://metamind.dev/data/thought/{}", result.think_id),
                "https://metamind.dev/ontology#Thought",
                Some(question),
                &payload,
            )
            .await
            .map_err(|error| MetamindToolError::new("INTERNAL", error.to_string()))?;
        }
        self.register_trace(
            session_id,
            TraceKind::Think,
            &think_id,
            mm_data::rdf::now_ms(),
        )
        .await;
        serde_json::to_value(&result)
            .map_err(|error| MetamindToolError::new("INTERNAL", error.to_string()))
    }

    async fn memory(&self, arguments: &Value) -> Result<Value, MetamindToolError> {
        let action = string_arg(arguments, "action")?;
        match action {
            "recall" => {
                let query = string_arg(arguments, "query")?;
                let kinds = string_array(arguments, "kinds");
                let (scopes, memory_kinds) = memory_scope(&kinds);
                let result = self
                    .memory
                    .recall(mm_data::RecallQuery {
                        query: query.to_owned(),
                        scopes,
                        kinds: memory_kinds,
                        limit: opt_u64(arguments, "limit")?.unwrap_or(6) as usize,
                    })
                    .await
                    .map_err(map_memory)?;
                Ok(json!({
                    "memories": result.records,
                    "count": result.records.len(),
                    "truncated": result.truncated,
                }))
            }
            "remember" => {
                let content = string_arg(arguments, "content")?;
                let kinds = string_array(arguments, "kinds");
                let (scope, kind) = remember_target(&kinds);
                let record = self
                    .memory
                    .remember(mm_data::RememberRequest {
                        scope,
                        kind,
                        key: content.chars().take(120).collect(),
                        value: content.to_owned(),
                        source: "cm".into(),
                        confidence: arguments
                            .get("importance")
                            .and_then(Value::as_f64)
                            .map(|value| value as f32),
                        expires_at: None,
                    })
                    .await
                    .map_err(map_memory)?;
                Ok(json!({ "memory": record }))
            }
            "forget" => {
                let memory_id = string_arg(arguments, "memory_id")?;
                let removed = self
                    .memory
                    .forget(mm_data::ForgetRequest {
                        selector: mm_data::ForgetSelector::Id {
                            id: memory_id.to_owned(),
                        },
                        reason: "cm forget".into(),
                    })
                    .await
                    .map_err(map_memory)?;
                Ok(json!({ "forgotten": removed }))
            }
            "status" => {
                let status = self.memory.status().await.map_err(map_memory)?;
                Ok(serde_json::to_value(status)
                    .map_err(|error| MetamindToolError::new("INTERNAL", error.to_string()))?)
            }
            other => Err(MetamindToolError::invalid(
                "action",
                format!("unsupported memory action {other:?}"),
            )),
        }
    }

    async fn log(&self, session_id: &str, arguments: &Value) -> Result<Value, MetamindToolError> {
        let action = string_arg(arguments, "action")?;
        if let Some(requested) = opt_str(arguments, "session_id") {
            if requested != session_id {
                return Err(MetamindToolError::new(
                    "FORBIDDEN",
                    "other sessions require operator authority",
                ));
            }
        }
        let source = opt_str(arguments, "source").unwrap_or("all");
        if !matches!(source, "all" | "agentstream" | "rdf") {
            return Err(MetamindToolError::invalid("source", "invalid log source"));
        }
        let filter = log_filter(arguments)?;
        let limit = opt_u64(arguments, "limit")?.unwrap_or(20).clamp(1, 100) as usize;
        match action {
            "overview" => {
                let rdf = if source == "agentstream" {
                    None
                } else {
                    Some(self.logs.overview(session_id).await)
                };
                let stream = if source == "rdf" {
                    Vec::new()
                } else {
                    self.agentstream_entries(session_id, arguments).await?
                };
                let mut sources = Vec::new();
                let mut entries = stream.len();
                let mut warnings = stream
                    .iter()
                    .filter(|entry| entry.level == LogLevel::Warn)
                    .count();
                let mut errors = stream
                    .iter()
                    .filter(|entry| entry.level == LogLevel::Error)
                    .count();
                let mut oldest = stream.iter().map(|entry| entry.at).min();
                let mut latest = stream.iter().map(|entry| entry.at).max();
                if let Some(rdf) = rdf {
                    let rdf = rdf.map_err(|error| {
                        MetamindToolError::new("LOG_UNAVAILABLE", error.to_string())
                    })?;
                    entries += rdf.entries;
                    warnings += rdf.warnings;
                    errors += rdf.errors;
                    sources.extend(
                        rdf.sources
                            .into_iter()
                            .map(|(name, count)| json!({"source": name, "count": count})),
                    );
                    oldest = match (oldest, rdf.oldest) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    latest = match (latest, rdf.latest) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        (a, b) => a.or(b),
                    };
                }
                if source != "rdf" {
                    sources.push(json!({"source": "agentstream", "count": stream.len()}));
                }
                Ok(json!({
                    "entries": entries,
                    "oldest": oldest,
                    "latest": latest,
                    "warnings": warnings,
                    "errors": errors,
                    "sources": sources,
                    "retentionFloor": mm_data::rdf::now_ms() - mm_data::log::LOG_RETENTION_MS,
                }))
            }
            "latest" => {
                let mut entries = if source == "rdf" {
                    Vec::new()
                } else {
                    self.agentstream_entries(session_id, arguments).await?
                };
                if source != "agentstream" {
                    entries.extend(self.rdf_entries(session_id, &filter).await?);
                }
                entries.retain(|entry| filter.matches(entry));
                entries.sort_by_key(|entry| {
                    (
                        std::cmp::Reverse(entry.at),
                        std::cmp::Reverse(entry.sequence),
                    )
                });
                entries.truncate(limit);
                Ok(json!({ "entries": entries, "count": entries.len() }))
            }
            "segment" => {
                let mut entries = if source == "rdf" {
                    Vec::new()
                } else {
                    self.agentstream_entries(session_id, arguments).await?
                };
                if source != "agentstream" {
                    entries.extend(self.rdf_entries(session_id, &filter).await?);
                }
                entries.retain(|entry| filter.matches(entry));
                entries.sort_by_key(|entry| (entry.at, entry.sequence, entry.id.clone()));
                let cursor = opt_str(arguments, "cursor");
                if let Some(cursor) = cursor {
                    let after = parse_log_cursor(cursor)?;
                    entries.retain(|entry| {
                        (entry.at, entry.sequence, entry.id.as_str())
                            > (after.0, after.1, after.2.as_str())
                    });
                }
                let has_more = entries.len() > limit;
                entries.truncate(limit);
                let next_cursor = has_more.then(|| entries.last().map(log_cursor)).flatten();
                Ok(json!({
                    "entries": entries,
                    "nextCursor": next_cursor,
                    "hasMore": has_more,
                }))
            }
            "append" => {
                let message = string_arg(arguments, "message")?;
                let entry = self
                    .logs
                    .append(
                        session_id,
                        opt_str(arguments, "trace_id"),
                        opt_str(arguments, "level")
                            .and_then(LogLevel::parse)
                            .unwrap_or(LogLevel::Info),
                        "cm",
                        message,
                        string_array(arguments, "tags"),
                        string_array(arguments, "refs"),
                        "cm",
                    )
                    .await
                    .map_err(|error| match error {
                        mm_data::LogError::Invalid(message) => {
                            MetamindToolError::new("INVALID_ARGUMENTS", message)
                        }
                        other => MetamindToolError::new("LOG_UNAVAILABLE", other.to_string()),
                    })?;
                Ok(json!({ "entry": entry }))
            }
            other => Err(MetamindToolError::invalid(
                "action",
                format!("unsupported log action {other:?}"),
            )),
        }
    }

    async fn agentstream_entries(
        &self,
        session_id: &str,
        arguments: &Value,
    ) -> Result<Vec<LogEntry>, MetamindToolError> {
        let trace_filter = opt_str(arguments, "trace_id");
        let mut ids = vec![session_id.to_owned()];
        if let Ok(page) = self.state.list_traces(session_id, None, 100).await {
            ids = page
                .items
                .iter()
                .filter(|trace| {
                    trace_filter
                        .map(|trace_id| trace.trace_id == trace_id)
                        .unwrap_or(trace.kind == TraceKind::Main)
                })
                .map(|trace| trace.agentstream_id.clone())
                .collect();
            if ids.is_empty() && trace_filter.is_none() {
                ids.push(session_id.to_owned());
            }
        }
        let mut entries = Vec::new();
        for id in ids {
            let events =
                self.agentstream.events(&id).await.map_err(|error| {
                    MetamindToolError::new("LOG_UNAVAILABLE", error.to_string())
                })?;
            for (index, event) in events.into_iter().enumerate() {
                entries.push(stream_entry(event, index as u64));
            }
        }
        Ok(entries)
    }

    async fn rdf_entries(
        &self,
        session_id: &str,
        filter: &LogFilter,
    ) -> Result<Vec<LogEntry>, MetamindToolError> {
        let mut entries = self
            .logs
            .entries(session_id)
            .await
            .map_err(|error| MetamindToolError::new("LOG_UNAVAILABLE", error.to_string()))?;
        entries.retain(|entry| filter.matches(entry));
        Ok(entries)
    }

    async fn act(&self, session_id: &str, arguments: &Value) -> Result<Value, MetamindToolError> {
        let action = string_arg(arguments, "action")?;
        if action == "internal" {
            return self.internal_action(session_id, arguments).await;
        }
        if action == "cancel" {
            let action_id = string_arg(arguments, "action_id")?;
            let cancelled = self.actions.cancel(action_id).await.map_err(map_action)?;
            if cancelled {
                self.logs
                    .append(
                        session_id,
                        None,
                        LogLevel::Info,
                        "action",
                        &format!("cancelled {action_id}"),
                        Vec::new(),
                        vec![action_id.to_owned()],
                        "action",
                    )
                    .await
                    .ok();
            }
            return Ok(json!({ "actionId": action_id, "cancelled": cancelled }));
        }
        if action == "introspection" {
            let report = self
                .introspection
                .introspect(opt_str(arguments, "input"))
                .await
                .map_err(map_subagent)?;
            self.register_trace(
                session_id,
                TraceKind::Introspection,
                &format!("introspect/{}", report.report_id),
                mm_data::rdf::now_ms(),
            )
            .await;
            return serde_json::to_value(report)
                .map_err(|error| MetamindToolError::new("INTERNAL", error.to_string()));
        }
        self.plan_control(session_id, arguments, action).await
    }

    async fn internal_action(
        &self,
        session_id: &str,
        arguments: &Value,
    ) -> Result<Value, MetamindToolError> {
        let kind = string_arg(arguments, "kind")?;
        let input = string_arg(arguments, "input")?;
        if contains_secret(input) {
            return Err(MetamindToolError::new(
                "INVALID_ARGUMENTS",
                "action input looks like a secret",
            ));
        }
        let spec = self
            .actions
            .registry()
            .require(kind)
            .map_err(map_action)?
            .clone();
        let parameters = form_values(arguments.get("parameters"))?;
        spec.validate_values(&parameters).map_err(map_action)?;
        let mode = opt_str(arguments, "mode").unwrap_or("immediate");
        if mode == "scheduled" {
            let schedule = schedule_from(arguments)?;
            let recurrence = arguments
                .get("schedule")
                .and_then(|value| value.get("recurrence"))
                .and_then(Value::as_str)
                .and_then(Recurrence::parse)
                .unwrap_or(if matches!(schedule, ScheduleSpec::Cron { .. }) {
                    Recurrence::Cron
                } else {
                    Recurrence::Once
                });
            let max_runs = arguments
                .get("schedule")
                .and_then(|value| value.get("max_runs"))
                .and_then(Value::as_u64)
                .map(|value| value as u32);
            let scheduled = self
                .actions
                .schedule(
                    ScheduledActionSpec {
                        session_id: session_id.to_owned(),
                        kind: kind.to_owned(),
                        input: Some(input.to_owned()),
                        parameters,
                        schedule,
                        idempotency_key: opt_str(arguments, "idempotency_key")
                            .map(str::to_owned)
                            .unwrap_or_else(mm_data::rdf::new_id),
                        status: ScheduledActionStatus::Scheduled,
                        created_at: mm_data::rdf::now_ms(),
                        last_run_at: None,
                        runs: 0,
                    },
                    recurrence,
                    max_runs,
                    mm_data::rdf::now_ms(),
                )
                .await
                .map_err(map_action)?;
            return Ok(scheduled_json(&scheduled));
        }
        if mode != "immediate" {
            return Err(MetamindToolError::invalid("mode", "invalid action mode"));
        }
        let log_entry = self
            .logs
            .append(
                session_id,
                None,
                LogLevel::Info,
                kind,
                &format!("{}: {}", spec.title, input),
                Vec::new(),
                Vec::new(),
                "action",
            )
            .await
            .map_err(|error| MetamindToolError::new("LOG_UNAVAILABLE", error.to_string()))?;
        let receipt = ActionReceipt {
            action_id: None,
            kind: kind.to_owned(),
            status: ScheduledActionStatus::Done,
            message: format!("{}: {}", spec.title, input),
            log_entry_id: Some(log_entry.id),
        };
        self.register_trace(
            session_id,
            TraceKind::Action,
            &format!("action/{}", mm_data::rdf::new_id()),
            mm_data::rdf::now_ms(),
        )
        .await;
        serde_json::to_value(receipt)
            .map_err(|error| MetamindToolError::new("INTERNAL", error.to_string()))
    }

    async fn plan_control(
        &self,
        session_id: &str,
        arguments: &Value,
        action: &str,
    ) -> Result<Value, MetamindToolError> {
        let plan_id = string_arg(arguments, "plan_id")?;
        let expected = match opt_u64(arguments, "expected_revision")? {
            Some(revision) => revision,
            None => self
                .plan_service
                .get(plan_id)
                .await
                .map_err(map_subagent)?
                .map(|plan| plan.revision)
                .ok_or_else(|| MetamindToolError::new("NOT_FOUND", "plan not found"))?,
        };
        let control = match action {
            "plan_start" => PlanControl::Start,
            "plan_stop" => PlanControl::Stop,
            "plan_pause" => PlanControl::Pause,
            "plan_resume" => PlanControl::Resume,
            "plan_retry_step" => PlanControl::Retry {
                step_id: string_arg(arguments, "step_id")?.to_owned(),
            },
            "plan_advance_step" => PlanControl::Advance,
            "plan_complete" => PlanControl::Complete,
            "plan_abandon" => PlanControl::Abandon,
            other => {
                return Err(MetamindToolError::invalid(
                    "action",
                    format!("unsupported act action {other:?}"),
                ))
            }
        };
        let is_stop = matches!(&control, PlanControl::Stop);
        let receipt = self
            .plan_service
            .control(control_request(
                plan_id,
                control,
                expected,
                opt_str(arguments, "reason").map(str::to_owned),
            ))
            .await
            .map_err(map_subagent)?;
        if is_stop {
            if let Some(agent) = &self.agent {
                if let Ok(Some(plan)) = self.plan_service.get(plan_id).await {
                    for run_id in plan
                        .steps
                        .iter()
                        .filter_map(|step| step.agent_run_id.clone())
                    {
                        let _ = agent.cancel(&run_id).await;
                    }
                }
            }
        }
        self.logs
            .append(
                session_id,
                None,
                LogLevel::Info,
                "plan",
                &format!("{action} {plan_id}: {}", receipt.message),
                Vec::new(),
                vec![plan_id.to_owned()],
                "cm",
            )
            .await
            .ok();
        serde_json::to_value(receipt)
            .map_err(|error| MetamindToolError::new("INTERNAL", error.to_string()))
    }

    async fn register_trace(
        &self,
        session_id: &str,
        kind: TraceKind,
        agentstream_id: &str,
        started_at: i64,
    ) {
        if self
            .agentstream
            .start_session(
                agentstream_id,
                SessionMetadata {
                    user_id: CONSOLE_USER.into(),
                    project_id: None,
                    title: Some(format!("{kind:?}")),
                },
            )
            .await
            .is_err()
        {
            return;
        }
        let _ = self
            .agentstream
            .log_event(
                AgentstreamEvent::new(
                    agentstream_id,
                    EventType::Lifecycle,
                    json!({ "phase": "started", "kind": format!("{kind:?}").to_lowercase() }),
                    EventProvenance {
                        source: "metamind".into(),
                        model: None,
                    },
                )
                .with_turn(agentstream_id),
            )
            .await;
        let trace = SessionTrace {
            trace_id: mm_data::rdf::new_id(),
            session_id: session_id.to_owned(),
            kind,
            agentstream_id: agentstream_id.to_owned(),
            first_seq: 0,
            last_seq: 0,
            event_count: 1,
            started_at,
            ended_at: None,
            size_bytes: 0,
        };
        let _ = self.state.register_trace(session_id, &trace).await;
    }
}

fn short_id(value: &str) -> &str {
    value.rsplit('/').next().unwrap_or(value)
}

fn memory_scope(kinds: &[String]) -> (Vec<MemoryScope>, Vec<MemoryKind>) {
    let mut scopes = vec![MemoryScope::Project];
    let mut memory_kinds = Vec::new();
    if kinds.is_empty() || kinds.iter().any(|kind| kind == "episodic") {
        scopes.push(MemoryScope::Session);
        memory_kinds.push(MemoryKind::Episode);
    }
    if kinds.is_empty() || kinds.iter().any(|kind| kind == "semantic") {
        memory_kinds.extend([
            MemoryKind::Fact,
            MemoryKind::Preference,
            MemoryKind::Decision,
            MemoryKind::Commitment,
        ]);
    }
    if kinds.iter().any(|kind| kind == "project") {
        scopes.push(MemoryScope::Project);
    }
    if kinds.iter().any(|kind| kind == "user") {
        scopes.push(MemoryScope::User);
    }
    scopes.sort();
    scopes.dedup();
    memory_kinds.sort_by_key(|kind| format!("{kind:?}"));
    memory_kinds.dedup();
    (scopes, memory_kinds)
}

fn remember_target(kinds: &[String]) -> (MemoryScope, MemoryKind) {
    if kinds.iter().any(|kind| kind == "user") {
        (MemoryScope::User, MemoryKind::Preference)
    } else if kinds.iter().any(|kind| kind == "project") {
        (MemoryScope::Project, MemoryKind::Fact)
    } else if kinds.iter().any(|kind| kind == "episodic") {
        (MemoryScope::Session, MemoryKind::Episode)
    } else {
        (MemoryScope::Project, MemoryKind::Fact)
    }
}

fn log_filter(arguments: &Value) -> Result<LogFilter, MetamindToolError> {
    let level = match opt_str(arguments, "level") {
        Some(level) => Some(
            LogLevel::parse(level)
                .ok_or_else(|| MetamindToolError::invalid("level", "invalid log level"))?,
        ),
        None => None,
    };
    Ok(LogFilter {
        trace_id: opt_str(arguments, "trace_id").map(str::to_owned),
        kinds: string_array(arguments, "kinds"),
        level,
        query: opt_str(arguments, "query").map(str::to_owned),
        since: opt_str(arguments, "since").map(parse_time).transpose()?,
        until: opt_str(arguments, "until").map(parse_time).transpose()?,
    })
}

fn parse_time(value: &str) -> Result<i64, MetamindToolError> {
    mm_core::Timestamp::from_rfc3339(value)
        .map(|timestamp| (timestamp.as_nanos() / 1_000_000) as i64)
        .map_err(|error| MetamindToolError::invalid("since/until", error.to_string()))
}

fn stream_entry(event: AgentstreamEvent, sequence: u64) -> LogEntry {
    let message = event
        .payload
        .get("query")
        .and_then(Value::as_str)
        .or_else(|| event.payload.get("frame").map(|_| "conversation frame"))
        .or_else(|| event.payload.get("phase").and_then(Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let text = event.payload.to_string();
            text.chars().take(500).collect()
        });
    LogEntry {
        id: event.event_id,
        session_id: event.session_id,
        trace_id: event.turn_id,
        level: LogLevel::Info,
        kind: event.event_type.as_str().into(),
        message,
        tags: Vec::new(),
        refs: Vec::new(),
        at: event.timestamp,
        source: "agentstream".into(),
        sequence,
    }
}

fn log_cursor(entry: &LogEntry) -> String {
    format!("{}:{}:{}", entry.at, entry.sequence, entry.id)
}

fn parse_log_cursor(cursor: &str) -> Result<(i64, u64, String), MetamindToolError> {
    let mut parts = cursor.splitn(3, ':');
    let at = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| MetamindToolError::invalid("cursor", "invalid cursor"))?;
    let sequence = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| MetamindToolError::invalid("cursor", "invalid cursor"))?;
    let id = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MetamindToolError::invalid("cursor", "invalid cursor"))?;
    Ok((at, sequence, id.to_owned()))
}

fn schedule_from(arguments: &Value) -> Result<ScheduleSpec, MetamindToolError> {
    let schedule = arguments
        .get("schedule")
        .ok_or_else(|| MetamindToolError::missing("schedule"))?;
    let at = schedule.get("at").and_then(Value::as_str);
    let delay = schedule.get("delay_seconds").and_then(Value::as_u64);
    let cron = schedule.get("cron").and_then(Value::as_str);
    let count = [at.is_some(), delay.is_some(), cron.is_some()]
        .into_iter()
        .filter(|value| *value)
        .count();
    if count != 1 {
        return Err(MetamindToolError::invalid(
            "schedule",
            "schedule needs exactly one of at, delay_seconds, or cron",
        ));
    }
    if let Some(at) = at {
        let at = parse_time(at)?;
        if at <= mm_data::rdf::now_ms() {
            return Err(MetamindToolError::invalid(
                "schedule.at",
                "scheduled instant must be in the future",
            ));
        }
        Ok(ScheduleSpec::At { at })
    } else if let Some(delay) = delay {
        Ok(ScheduleSpec::Delay {
            delay_ms: delay.saturating_mul(1_000),
        })
    } else {
        Ok(ScheduleSpec::Cron {
            expression: cron.unwrap_or_default().to_owned(),
            timezone: schedule
                .get("timezone")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

fn scheduled_json(action: &ScheduledAction) -> Value {
    json!({
        "actionId": action.id,
        "kind": action.spec.kind,
        "status": action.spec.status,
        "recurrence": action.recurrence,
        "dueAt": action.next_run_at.map(rfc3339),
        "nextRunAt": action.next_run_at.map(rfc3339),
        "runs": action.spec.runs,
    })
}

fn rfc3339(ms: i64) -> String {
    let seconds = ms.div_euclid(1_000).max(0) as u64;
    let nanos = (ms.rem_euclid(1_000) * 1_000_000) as u32;
    mm_core::Timestamp { seconds, nanos }.to_rfc3339()
}

fn form_values(value: Option<&Value>) -> Result<FormValues, MetamindToolError> {
    let Some(value) = value else {
        return Ok(FormValues::new());
    };
    let Some(map) = value.as_object() else {
        return Err(MetamindToolError::invalid(
            "parameters",
            "parameters must be an object",
        ));
    };
    let mut values = FormValues::new();
    for (key, value) in map {
        let value = match value {
            Value::String(text) => FormValue::Text(text.clone()),
            Value::Number(number) => FormValue::Number(
                number
                    .as_f64()
                    .ok_or_else(|| MetamindToolError::invalid("parameters", "invalid number"))?,
            ),
            Value::Bool(flag) => FormValue::Bool(*flag),
            _ => {
                return Err(MetamindToolError::invalid(
                    "parameters",
                    "parameter values must be string, number, or boolean",
                ))
            }
        };
        values.insert(key.clone(), value);
    }
    Ok(values)
}

fn plan_format(arguments: &Value) -> Result<PlanFormat, MetamindToolError> {
    match opt_str(arguments, "format").unwrap_or("markdown") {
        "markdown" => Ok(PlanFormat::Markdown),
        "structured" => Ok(PlanFormat::Structured),
        _ => Err(MetamindToolError::invalid("format", "invalid plan format")),
    }
}

fn map_subagent(error: SubagentError) -> MetamindToolError {
    match error {
        SubagentError::Plan(PlanError::Conflict { expected, current }) => MetamindToolError {
            code: "PLAN_CONFLICT".into(),
            message: format!("expected revision {expected}, current revision {current}"),
            field: Some("expected_revision".into()),
            retryable: true,
            next: Some("re-read the plan and retry".into()),
        },
        SubagentError::Plan(PlanError::NotFound(plan_id)) => {
            MetamindToolError::new("NOT_FOUND", format!("unknown plan {plan_id}"))
        }
        SubagentError::Plan(error) => {
            MetamindToolError::new("INVALID_ARGUMENTS", error.to_string())
        }
        SubagentError::Data(error) => MetamindToolError::new("DATA_UNAVAILABLE", error.to_string()),
        SubagentError::Memory(error) => MetamindToolError::new("MEMORY_UNAVAILABLE", error),
        SubagentError::Prompt(error) => MetamindToolError::new("PROMPT_UNAVAILABLE", error),
        SubagentError::Model(error) => MetamindToolError::new("UNAVAILABLE", error),
        SubagentError::Invalid(error) => MetamindToolError::new("INVALID_ARGUMENTS", error),
    }
}

fn map_memory(error: mm_data::MemoryContractError) -> MetamindToolError {
    match error {
        mm_data::MemoryContractError::Invalid(message) => {
            MetamindToolError::new("INVALID_ARGUMENTS", message)
        }
        mm_data::MemoryContractError::NotFound(message) => {
            MetamindToolError::new("NOT_FOUND", message)
        }
        mm_data::MemoryContractError::Unavailable(message) => {
            MetamindToolError::new("MEMORY_UNAVAILABLE", message)
        }
    }
}

fn map_action(error: ActionError) -> MetamindToolError {
    match error {
        ActionError::UnknownKind(kind) => MetamindToolError::new(
            "ACTION_UNKNOWN",
            format!("unknown internal action kind {kind:?}"),
        ),
        ActionError::Invalid(message) => MetamindToolError::new("INVALID_ARGUMENTS", message),
        ActionError::Limit => MetamindToolError::new("BUDGET", "scheduled action limit reached"),
        ActionError::DuplicateKind(kind) => {
            MetamindToolError::new("INVALID_ARGUMENTS", format!("duplicate action kind {kind}"))
        }
        ActionError::Unavailable(message) => MetamindToolError::new("UNAVAILABLE", message),
    }
}

fn string_arg<'a>(arguments: &'a Value, field: &str) -> Result<&'a str, MetamindToolError> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| MetamindToolError::missing(field))
}

fn opt_str<'a>(arguments: &'a Value, field: &str) -> Option<&'a str> {
    arguments.get(field).and_then(Value::as_str)
}

fn opt_u64(arguments: &Value, field: &str) -> Result<Option<u64>, MetamindToolError> {
    match arguments.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            MetamindToolError::invalid(field, format!("{field} must be a non-negative integer"))
        }),
    }
}

fn string_array(arguments: &Value, field: &str) -> Vec<String> {
    arguments
        .get(field)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// A typed metamind-tool failure.
#[derive(Debug, Clone)]
pub struct MetamindToolError {
    /// Stable code.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Offending field.
    pub field: Option<String>,
    /// Whether a retry can help.
    pub retryable: bool,
    /// Suggested next action.
    pub next: Option<String>,
}

impl MetamindToolError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            field: None,
            retryable: false,
            next: None,
        }
    }

    fn invalid(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_ARGUMENTS".into(),
            message: message.into(),
            field: Some(field.into()),
            retryable: false,
            next: Some("fix the call and retry".into()),
        }
    }

    fn missing(field: &str) -> Self {
        Self::invalid(field, "missing required field")
    }
}
