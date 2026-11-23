//! Bounded planning and inner-thought sub-sessions.
//!
//! These services own the model loop; `mm-serve` owns the tool handlers and
//! trace registration. The planner can inspect data read-only. The thinker has
//! an even smaller allowlist and can never write, plan, act, delegate, or
//! recurse.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mm_data::data::DataError;
use mm_data::{
    parse_structured_plan, DataBroker, DataQuery, DataSearchRequest, DataService, MemoryKind,
    MemoryScope, MemoryService, Plan, PlanControl, PlanControlReceipt, PlanControlRequest,
    PlanError, PlanFormat, PlanOwner, PlanStore, PlanVersion, PlanWrite, RdfPlanStore, RecallQuery,
    SchemaInclude, ValidationTarget,
};
use mm_llm::{LlmClient, LlmRequest, Message, ProviderToolCall, Purpose, ToolSchema};
use mm_prompt::PromptStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

/// Planning is capped at 8 model rounds and 90 seconds.
pub const PLAN_ROUNDS: usize = 8;
/// Planning wall clock.
pub const PLAN_TIMEOUT: Duration = Duration::from_secs(90);
/// Thinking is capped at 4 tool rounds and 90 seconds.
pub const THINK_ROUNDS: usize = 4;
/// Thinking wall clock.
pub const THINK_TIMEOUT: Duration = Duration::from_secs(90);
/// Introspection wall clock.
pub const INTROSPECT_TIMEOUT: Duration = Duration::from_secs(20);

/// A sub-agent failure.
#[derive(Debug, Error)]
pub enum SubagentError {
    /// Planning failed.
    #[error(transparent)]
    Plan(#[from] PlanError),
    /// Data lookup failed.
    #[error(transparent)]
    Data(#[from] DataError),
    /// Memory lookup failed.
    #[error("memory unavailable: {0}")]
    Memory(String),
    /// Prompt rendering failed.
    #[error("prompt unavailable: {0}")]
    Prompt(String),
    /// Model call failed or timed out.
    #[error("model unavailable: {0}")]
    Model(String),
    /// Invalid request.
    #[error("invalid request: {0}")]
    Invalid(String),
}

/// Result alias.
pub type Result<T, E = SubagentError> = std::result::Result<T, E>;

/// A compact plan result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanResult {
    /// The plan header.
    pub plan: Plan,
    /// The version just written, when the call wrote one.
    pub version: Option<PlanVersion>,
    /// Whether the planner believes the plan is complete.
    pub complete: bool,
    /// Explicit gaps.
    pub gaps: Vec<String>,
    /// The prompt instance that produced this plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_instance_id: Option<String>,
    /// Stable hash of that prompt instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hash: Option<String>,
}

impl PlanResult {
    /// The tool-facing compact projection.
    pub fn to_json(&self) -> Value {
        json!({
            "planId": self.plan.id,
            "promptInstanceId": self.prompt_instance_id,
            "goal": self.plan.goal,
            "format": self.plan.format,
            "status": self.plan.status,
            "currentVersion": self.plan.current_version,
            "revision": self.plan.revision,
            "steps": self.plan.steps,
            "body": self.plan.body,
            "complete": self.complete,
            "gaps": self.gaps,
        })
    }
}

/// The planning sub-session.
pub struct PlanService {
    _data: Arc<DataService>,
    plans: Arc<RdfPlanStore>,
    prompts: Arc<PromptStore>,
    client: Option<Arc<dyn LlmClient>>,
    model: Option<String>,
}

impl std::fmt::Debug for PlanService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanService")
            .field("has_client", &self.client.is_some())
            .finish_non_exhaustive()
    }
}

impl PlanService {
    /// Build the planner.
    pub fn new(
        data: Arc<DataService>,
        plans: Arc<RdfPlanStore>,
        prompts: Arc<PromptStore>,
        client: Option<Arc<dyn LlmClient>>,
        model: Option<String>,
    ) -> Self {
        Self {
            _data: data,
            plans,
            prompts,
            client,
            model,
        }
    }

    /// Create a plan. Supplied content is validated rather than regenerated.
    pub async fn create(
        &self,
        session_id: &str,
        goal: &str,
        format: PlanFormat,
        content: Option<&Value>,
        instructions: Option<&str>,
    ) -> Result<PlanResult> {
        let plan_id = mm_data::rdf::new_id();
        let mut gaps = Vec::new();
        let (format, body, steps, prompt_meta) = match content {
            Some(Value::String(text)) => {
                if text.trim().is_empty() {
                    return Err(SubagentError::Invalid(
                        "plan content must not be empty".into(),
                    ));
                }
                if format == PlanFormat::Structured {
                    let parsed = parse_structured_plan(text, &plan_id, 1)?;
                    (format, text.clone(), parsed.steps, None)
                } else {
                    (PlanFormat::Markdown, text.clone(), Vec::new(), None)
                }
            }
            Some(Value::Object(_)) => {
                let text = content
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "{}".into());
                let parsed = parse_structured_plan(&text, &plan_id, 1)?;
                (PlanFormat::Structured, text, parsed.steps, None)
            }
            Some(_) => {
                return Err(SubagentError::Invalid(
                    "content must be text or an object".into(),
                ))
            }
            None => self
                .generate(goal, format, instructions)
                .await
                .map(|(format, body, steps, id, hash)| (format, body, steps, Some((id, hash))))
                .unwrap_or_else(|error| {
                    gaps.push(error.to_string());
                    let (format, body, steps) = fallback_plan(goal, format);
                    (format, body, steps, None)
                }),
        };
        let mut plan = Plan::new(&plan_id, session_id, goal, mm_data::rdf::now_ms());
        plan.format = format;
        plan.body = body.clone();
        plan.steps = steps;
        plan.validate()?;
        let version = PlanVersion {
            plan_id: plan.id.clone(),
            version: 1,
            format,
            body,
            steps: plan.steps.clone(),
            created_at: plan.created_at,
            created_by: "cm".into(),
        };
        self.plans
            .save_version(&PlanWrite {
                plan: plan.clone(),
                version: version.clone(),
                expected_revision: 0,
            })
            .await?;
        Ok(PlanResult {
            plan,
            version: Some(version),
            complete: gaps.is_empty(),
            gaps,
            prompt_instance_id: prompt_meta.as_ref().map(|(id, _)| id.clone()),
            prompt_hash: prompt_meta.map(|(_, hash)| hash),
        })
    }

    /// Update a plan into a new immutable version.
    pub async fn update(
        &self,
        plan_id: &str,
        expected_revision: u64,
        goal: Option<&str>,
        format: Option<PlanFormat>,
        content: Option<&Value>,
        instructions: Option<&str>,
    ) -> Result<PlanResult> {
        let mut plan = self
            .plans
            .load(plan_id)
            .await?
            .ok_or_else(|| PlanError::NotFound(plan_id.to_owned()))?;
        if plan.revision != expected_revision {
            return Err(PlanError::Conflict {
                expected: expected_revision,
                current: plan.revision,
            }
            .into());
        }
        let format = format.unwrap_or(plan.format);
        let mut gaps = Vec::new();
        let (body, steps, prompt_meta) = match content {
            Some(Value::String(text)) => {
                if text.trim().is_empty() {
                    return Err(SubagentError::Invalid(
                        "plan content must not be empty".into(),
                    ));
                }
                if format == PlanFormat::Structured {
                    let parsed = parse_structured_plan(text, plan_id, plan.revision + 1)?;
                    (text.clone(), parsed.steps, None)
                } else {
                    (text.clone(), Vec::new(), None)
                }
            }
            Some(Value::Object(_)) => {
                let text = content
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "{}".into());
                let parsed = parse_structured_plan(&text, plan_id, plan.revision + 1)?;
                (text, parsed.steps, None)
            }
            Some(_) => {
                return Err(SubagentError::Invalid(
                    "content must be text or an object".into(),
                ))
            }
            None => self
                .generate(
                    goal.unwrap_or(&plan.goal),
                    format,
                    instructions.or(Some("Refine the existing plan.")),
                )
                .await
                .map(|(_, body, steps, id, hash)| (body, steps, Some((id, hash))))
                .unwrap_or_else(|error| {
                    gaps.push(error.to_string());
                    let (_, body, steps) = fallback_plan(goal.unwrap_or(&plan.goal), format);
                    (body, steps, None)
                }),
        };
        plan.format = format;
        plan.body = body.clone();
        plan.steps = steps;
        if let Some(goal) = goal {
            plan.goal = goal.to_owned();
        }
        plan.current_version = plan.current_version.saturating_add(1);
        plan.revision = plan.revision.saturating_add(1);
        plan.updated_at = mm_data::rdf::now_ms();
        plan.validate()?;
        let version = PlanVersion {
            plan_id: plan.id.clone(),
            version: plan.current_version,
            format,
            body,
            steps: plan.steps.clone(),
            created_at: plan.updated_at,
            created_by: "cm".into(),
        };
        self.plans
            .save_version(&PlanWrite {
                plan: plan.clone(),
                version: version.clone(),
                expected_revision,
            })
            .await?;
        Ok(PlanResult {
            plan,
            version: Some(version),
            complete: gaps.is_empty(),
            gaps,
            prompt_instance_id: prompt_meta.as_ref().map(|(id, _)| id.clone()),
            prompt_hash: prompt_meta.map(|(_, hash)| hash),
        })
    }

    /// Load one plan.
    pub async fn get(&self, plan_id: &str) -> Result<Option<Plan>> {
        Ok(self.plans.load(plan_id).await?)
    }

    /// List plans for a session.
    pub async fn list(&self, session_id: &str) -> Result<Vec<Plan>> {
        Ok(self.plans.list(session_id).await?)
    }

    /// Apply one lifecycle control.
    pub async fn control(&self, request: PlanControlRequest) -> Result<PlanControlReceipt> {
        Ok(self.plans.control(&request).await?)
    }

    async fn generate(
        &self,
        goal: &str,
        format: PlanFormat,
        instructions: Option<&str>,
    ) -> Result<(PlanFormat, String, Vec<mm_data::PlanStep>, String, String)> {
        let Some(client) = &self.client else {
            return Err(SubagentError::Model(
                "planning model is not configured; wrote a partial plan".into(),
            ));
        };
        let prompt = self.render_prompt(
            "metamind.plan.v1",
            BTreeMap::from([
                ("plan_goal".to_string(), goal.to_string()),
                (
                    "plan_state".to_string(),
                    instructions.unwrap_or("").to_string(),
                ),
            ]),
        )?;
        let mut request = LlmRequest::new(
            Purpose::Plan,
            vec![
                Message::system(prompt.text),
                Message::user(format!(
                    "Produce a {} plan for: {goal}",
                    match format {
                        PlanFormat::Markdown => "markdown",
                        PlanFormat::Structured => "mm.plan.v1 structured",
                    }
                )),
            ],
        );
        if let Some(model) = &self.model {
            request = request.with_model(model.clone());
        }
        let response = tokio::time::timeout(PLAN_TIMEOUT, client.complete(&request))
            .await
            .map_err(|_| SubagentError::Model("planning timed out".into()))?
            .map_err(|error| SubagentError::Model(error.to_string()))?;
        if format == PlanFormat::Structured {
            let parsed = parse_structured_plan(&response.text, "pending", 1).map_err(|error| {
                SubagentError::Invalid(format!("planner returned an invalid plan: {error}"))
            })?;
            Ok((
                format,
                response.text,
                parsed.steps,
                prompt.instance_id.clone(),
                prompt.hash.clone(),
            ))
        } else {
            Ok((
                format,
                response.text,
                Vec::new(),
                prompt.instance_id,
                prompt.hash,
            ))
        }
    }

    fn render_prompt(
        &self,
        strategy: &str,
        values: BTreeMap<String, String>,
    ) -> Result<mm_prompt::PromptInstance> {
        let Some(spec) = self.prompts.strategy(strategy) else {
            return Err(SubagentError::Prompt(format!(
                "missing prompt strategy {strategy}"
            )));
        };
        let mut variables = BTreeMap::new();
        for slot in &spec.slots {
            variables.insert(slot.clone(), String::new());
        }
        for (key, value) in values {
            variables.insert(key, value);
        }
        self.prompts
            .render(strategy, &variables)
            .map_err(|error| SubagentError::Prompt(error.to_string()))
    }
}

fn fallback_plan(goal: &str, format: PlanFormat) -> (PlanFormat, String, Vec<mm_data::PlanStep>) {
    let mut step = mm_data::PlanStep::new("s1", "Clarify and execute the goal");
    step.detail = Some(goal.to_owned());
    step.owner = PlanOwner::Cm;
    match format {
        PlanFormat::Markdown => (
            PlanFormat::Markdown,
            format!("# {goal}\n\n1. Clarify and execute the goal.\n"),
            Vec::new(),
        ),
        PlanFormat::Structured => {
            let body = json!({
                "schema": "mm.plan.v1",
                "goal": goal,
                "assumptions": [],
                "steps": [{
                    "id": "s1",
                    "title": "Clarify and execute the goal",
                    "detail": goal,
                    "dependsOn": [],
                    "owner": "cm"
                }],
                "risks": [],
                "openQuestions": [],
                "successCriteria": []
            })
            .to_string();
            (PlanFormat::Structured, body, vec![step])
        }
    }
}

/// The inner-thought result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ThinkResult {
    /// Thought id.
    pub think_id: String,
    /// The prompt instance that produced this thought.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_instance_id: Option<String>,
    /// Stable hash of that prompt instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hash: Option<String>,
    /// One-paragraph answer.
    pub summary: String,
    /// Supported claims.
    #[serde(default)]
    pub findings: Vec<ThinkFinding>,
    /// Explicit gaps.
    #[serde(default)]
    pub gaps: Vec<String>,
    /// Suggested focus IRIs.
    #[serde(default)]
    pub suggested_focus: Vec<String>,
    /// Prompt tokens.
    #[serde(default)]
    pub prompt_tokens: u32,
    /// Completion tokens.
    #[serde(default)]
    pub completion_tokens: u32,
    /// Wall time.
    #[serde(default)]
    pub elapsed_ms: u64,
}

/// One claim and its evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThinkFinding {
    /// Claim text.
    pub claim: String,
    /// Confidence 0..1.
    pub confidence: f32,
    /// Evidence references.
    #[serde(default)]
    pub evidence: Vec<ThinkEvidence>,
}

/// One evidence reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThinkEvidence {
    /// `entity`, `memory`, or `file`.
    pub kind: String,
    /// IRI/id.
    pub id: String,
}

/// The private thinker.
pub struct ThinkService {
    data: Arc<DataService>,
    memory: Arc<dyn MemoryService>,
    prompts: Arc<PromptStore>,
    client: Option<Arc<dyn LlmClient>>,
    model: Option<String>,
}

impl std::fmt::Debug for ThinkService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThinkService")
            .field("has_client", &self.client.is_some())
            .finish_non_exhaustive()
    }
}

impl ThinkService {
    /// Build the thinker.
    pub fn new(
        data: Arc<DataService>,
        memory: Arc<dyn MemoryService>,
        prompts: Arc<PromptStore>,
        client: Option<Arc<dyn LlmClient>>,
        model: Option<String>,
    ) -> Self {
        Self {
            data,
            memory,
            prompts,
            client,
            model,
        }
    }

    /// Run one bounded inner thought.
    pub async fn think(
        &self,
        question: &str,
        context: Option<&str>,
        focus: &[String],
        session_id: &str,
    ) -> Result<ThinkResult> {
        let started = Instant::now();
        let Some(client) = &self.client else {
            return self.fallback(question, focus, started).await;
        };
        let prompt = self.render_prompt(question)?;
        let user = match context {
            Some(context) if !context.is_empty() => {
                format!("Question: {question}\n\nConversation context:\n{context}")
            }
            _ => format!("Question: {question}"),
        };
        let mut messages = vec![Message::system(prompt.text), Message::user(user)];
        for reference in focus {
            messages.push(Message::user(format!("Focus entity: {reference}")));
        }
        let tools = read_only_schemas();
        let mut prompt_tokens = 0u32;
        let mut completion_tokens = 0u32;
        for _ in 0..THINK_ROUNDS {
            let mut request =
                LlmRequest::new(Purpose::Diagnose, messages.clone()).with_tools(tools.clone());
            if let Some(model) = &self.model {
                request = request.with_model(model.clone());
            }
            let response = tokio::time::timeout(THINK_TIMEOUT, client.complete(&request))
                .await
                .map_err(|_| SubagentError::Model("thinking timed out".into()))?
                .map_err(|error| SubagentError::Model(error.to_string()))?;
            prompt_tokens = prompt_tokens.saturating_add(response.usage.tokens_in);
            completion_tokens = completion_tokens.saturating_add(response.usage.tokens_out);
            if response.tool_calls.is_empty() {
                let mut result = parse_think_result(&response.text);
                result.think_id = session_id.to_owned();
                result.prompt_instance_id = Some(prompt.instance_id.clone());
                result.prompt_hash = Some(prompt.hash.clone());
                result.prompt_tokens = prompt_tokens;
                result.completion_tokens = completion_tokens;
                result.elapsed_ms = started.elapsed().as_millis() as u64;
                return Ok(result);
            }
            messages.push(Message::assistant_calls(
                response.text.clone(),
                response.tool_calls.clone(),
            ));
            for call in &response.tool_calls {
                let content = self
                    .run_read_only(call)
                    .await
                    .unwrap_or_else(|error| format!("{{\"error\":\"{error}\"}}"));
                messages.push(Message::tool(call.id.clone(), call.name.clone(), content));
            }
        }
        Err(SubagentError::Model(
            "thinker exceeded its tool-round cap".into(),
        ))
    }

    fn render_prompt(&self, question: &str) -> Result<mm_prompt::PromptInstance> {
        let Some(spec) = self.prompts.strategy("metamind.think.v1") else {
            return Err(SubagentError::Prompt(
                "missing prompt strategy metamind.think.v1".into(),
            ));
        };
        let mut variables: BTreeMap<String, String> = spec
            .slots
            .iter()
            .map(|slot| (slot.clone(), String::new()))
            .collect();
        variables.insert("thought_question".into(), question.to_owned());
        self.prompts
            .render("metamind.think.v1", &variables)
            .map_err(|error| SubagentError::Prompt(error.to_string()))
    }

    async fn run_read_only(&self, call: &ProviderToolCall) -> Result<String> {
        let args: Value = serde_json::from_str(&call.arguments)
            .map_err(|error| SubagentError::Invalid(error.to_string()))?;
        match call.name.as_str() {
            "data.query" => {
                let mut query = DataQuery {
                    class: args.get("class").and_then(Value::as_str).map(str::to_owned),
                    subject: args
                        .get("subject")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    graph: args.get("graph").and_then(Value::as_str).map(str::to_owned),
                    namespace: args
                        .get("namespace")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    include_deleted: false,
                    limit: args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize,
                    offset: args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
                    filters: Vec::new(),
                };
                query.limit = query.limit.clamp(1, 100);
                query.validate()?;
                let result = self.data.query(query).await?;
                Ok(json!({
                    "entities": result.entities.iter().map(compact_entity).collect::<Vec<_>>(),
                    "count": result.entities.len(),
                })
                .to_string())
            }
            "data.search" => {
                let request = DataSearchRequest {
                    query: args
                        .get("query")
                        .and_then(Value::as_str)
                        .ok_or_else(|| SubagentError::Invalid("search requires query".into()))?
                        .to_owned(),
                    class: args.get("class").and_then(Value::as_str).map(str::to_owned),
                    graph: args.get("graph").and_then(Value::as_str).map(str::to_owned),
                    namespace: args
                        .get("namespace")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    limit: args.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize,
                    snippet: true,
                    include_deleted: false,
                };
                request.validate()?;
                let result = self.data.search(request).await?;
                Ok(json!({
                    "hits": result.hits.iter().map(|hit| json!({
                        "iri": hit.iri, "label": hit.label, "type": hit.r#type,
                        "snippet": hit.snippet, "score": hit.score_micros as f64 / 1_000_000.0
                    })).collect::<Vec<_>>(),
                    "count": result.hits.len(),
                })
                .to_string())
            }
            "data.schema" => {
                let action = args.get("action").and_then(Value::as_str).unwrap_or("list");
                match action {
                    "list" => {
                        let schemas = self.data.list_schemas().await?;
                        Ok(json!({ "schemas": schemas, "count": schemas.len() }).to_string())
                    }
                    "get" => {
                        let id = args
                            .get("schema")
                            .and_then(Value::as_str)
                            .ok_or_else(|| SubagentError::Invalid("get requires schema".into()))?;
                        let detail = self.data.get_schema(id, SchemaInclude::Summary).await?;
                        Ok(serde_json::to_string(&detail).unwrap_or_default())
                    }
                    "validate" => {
                        let subject = args.get("subject").and_then(Value::as_str);
                        let class = args.get("class").and_then(Value::as_str);
                        let graph = args.get("graph").and_then(Value::as_str);
                        let target = if let Some(subject) = subject {
                            ValidationTarget::Subject(subject.into())
                        } else if let Some(class) = class {
                            ValidationTarget::Class(class.into())
                        } else if let Some(graph) = graph {
                            ValidationTarget::Graph(graph.into())
                        } else {
                            return Err(SubagentError::Invalid(
                                "validate requires subject, class, or graph".into(),
                            ));
                        };
                        let violations = self.data.validate(target).await?;
                        Ok(
                            json!({ "conforms": violations.is_empty(), "violations": violations })
                                .to_string(),
                        )
                    }
                    _ => Err(SubagentError::Invalid(
                        "think may only list/get/validate schemas".into(),
                    )),
                }
            }
            "data.namespace" => {
                let action = args.get("action").and_then(Value::as_str).unwrap_or("list");
                match action {
                    "list" => {
                        let namespaces = self.data.list_namespaces().await?;
                        Ok(serde_json::to_string(&namespaces).unwrap_or_default())
                    }
                    "expand" | "resolve" => {
                        let iri = args
                            .get("iri")
                            .and_then(Value::as_str)
                            .ok_or_else(|| SubagentError::Invalid("iri required".into()))?;
                        let value = if action == "expand" {
                            self.data.expand(iri).await?
                        } else {
                            self.data.resolve(iri).await?
                        };
                        Ok(json!({ "value": value }).to_string())
                    }
                    _ => Err(SubagentError::Invalid(
                        "think may only list/expand/resolve namespaces".into(),
                    )),
                }
            }
            "metamind.memory" => {
                let action = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("status");
                match action {
                    "recall" => {
                        let query = args.get("query").and_then(Value::as_str).ok_or_else(|| {
                            SubagentError::Invalid("recall requires query".into())
                        })?;
                        let result = self
                            .memory
                            .recall(RecallQuery {
                                query: query.to_owned(),
                                scopes: vec![
                                    MemoryScope::Session,
                                    MemoryScope::Project,
                                    MemoryScope::User,
                                ],
                                kinds: Vec::new(),
                                limit: args
                                    .get("limit")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(6)
                                    .clamp(1, 20) as usize,
                            })
                            .await
                            .map_err(|error| SubagentError::Memory(error.to_string()))?;
                        Ok(serde_json::to_string(&result).unwrap_or_default())
                    }
                    "status" => {
                        let status = self
                            .memory
                            .status()
                            .await
                            .map_err(|error| SubagentError::Memory(error.to_string()))?;
                        Ok(serde_json::to_string(&status).unwrap_or_default())
                    }
                    _ => Err(SubagentError::Invalid(
                        "think may only recall or inspect memory status".into(),
                    )),
                }
            }
            other => Err(SubagentError::Invalid(format!(
                "{other} is not a read-only think tool"
            ))),
        }
    }

    async fn fallback(
        &self,
        question: &str,
        focus: &[String],
        started: Instant,
    ) -> Result<ThinkResult> {
        let mut entities = Vec::new();
        for reference in focus.iter().take(8) {
            let query = DataQuery {
                subject: Some(reference.clone()),
                limit: 5,
                ..DataQuery::default()
            };
            if let Ok(result) = self.data.query(query).await {
                entities.extend(result.entities);
            }
        }
        if entities.is_empty() {
            let request = DataSearchRequest {
                query: question.chars().take(200).collect(),
                limit: 5,
                snippet: true,
                include_deleted: false,
                class: None,
                graph: None,
                namespace: None,
            };
            if request.validate().is_ok() {
                if let Ok(result) = self.data.search(request).await {
                    entities.extend(result.hits.into_iter().map(|hit| mm_data::DataEntity {
                        iri: hit.iri,
                        types: vec![hit.r#type],
                        label: hit.label,
                        properties: BTreeMap::new(),
                        graph: hit.graph,
                        revision: hit.revision,
                        status: mm_data::EntityStatus::Active,
                        created_at: hit.updated_at,
                        updated_at: hit.updated_at,
                        recorded_by: "search".into(),
                    }));
                }
            }
        }
        let recalled = self
            .memory
            .recall(RecallQuery {
                query: question.chars().take(500).collect(),
                scopes: vec![
                    MemoryScope::Session,
                    MemoryScope::Project,
                    MemoryScope::User,
                ],
                kinds: Vec::<MemoryKind>::new(),
                limit: 5,
            })
            .await
            .unwrap_or(mm_data::RecallResult {
                records: Vec::new(),
                truncated: false,
            });
        let mut findings = Vec::new();
        for entity in entities.iter().take(5) {
            findings.push(ThinkFinding {
                claim: entity.label.clone().unwrap_or_else(|| entity.iri.clone()),
                confidence: 0.5,
                evidence: vec![ThinkEvidence {
                    kind: "entity".into(),
                    id: entity.iri.clone(),
                }],
            });
        }
        for record in recalled.records.iter().take(3) {
            findings.push(ThinkFinding {
                claim: record.value.clone(),
                confidence: record.confidence,
                evidence: vec![ThinkEvidence {
                    kind: "memory".into(),
                    id: record.id.clone(),
                }],
            });
        }
        let gaps = if findings.is_empty() {
            vec!["no data or memory supported the question".to_string()]
        } else {
            Vec::new()
        };
        Ok(ThinkResult {
            think_id: mm_data::rdf::new_id(),
            prompt_instance_id: None,
            prompt_hash: None,
            summary: if findings.is_empty() {
                "No supporting evidence was found.".into()
            } else {
                format!("{} supported findings were found.", findings.len())
            },
            findings,
            gaps,
            suggested_focus: entities.iter().take(5).map(|e| e.iri.clone()).collect(),
            prompt_tokens: 0,
            completion_tokens: 0,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

fn parse_think_result(text: &str) -> ThinkResult {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        let summary = value
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or(text)
            .to_owned();
        let findings = value
            .get("findings")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let gaps = value
            .get("gaps")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let suggested_focus = value
            .get("suggestedFocus")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        ThinkResult {
            summary,
            findings,
            gaps,
            suggested_focus,
            ..ThinkResult::default()
        }
    } else {
        ThinkResult {
            summary: text.trim().to_owned(),
            gaps: if text.trim().is_empty() {
                vec!["thinker returned no content".into()]
            } else {
                Vec::new()
            },
            ..ThinkResult::default()
        }
    }
}

/// One introspection finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntrospectionFinding {
    /// Area.
    pub area: String,
    /// Observation.
    pub finding: String,
    /// Suggested action.
    pub suggestion: Option<String>,
}

/// A bounded read-only introspection report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntrospectionReport {
    /// Report id.
    pub report_id: String,
    /// One-paragraph summary.
    #[serde(default)]
    pub summary: String,
    /// Findings.
    pub findings: Vec<IntrospectionFinding>,
    /// Risks worth operator attention.
    #[serde(default)]
    pub risks: Vec<String>,
    /// Areas with no evidence.
    pub gaps: Vec<String>,
    /// Proposed plan controls; never executed by introspection.
    #[serde(default)]
    pub recommended_plan_actions: Vec<String>,
    /// Proposed external work; the CM or PlanMonitor must start it through agent.run.
    #[serde(default)]
    pub recommended_external_work: Vec<String>,
    /// Elapsed time.
    pub elapsed_ms: u64,
}

/// Read-only introspection over plans, data health, and memory status.
pub struct IntrospectionService {
    data: Arc<DataService>,
    plans: Arc<RdfPlanStore>,
    memory: Arc<dyn MemoryService>,
}

impl IntrospectionService {
    /// Build the service.
    pub fn new(
        data: Arc<DataService>,
        plans: Arc<RdfPlanStore>,
        memory: Arc<dyn MemoryService>,
    ) -> Self {
        Self {
            data,
            plans,
            memory,
        }
    }

    /// Run one bounded report.
    pub async fn introspect(&self, question: Option<&str>) -> Result<IntrospectionReport> {
        let started = Instant::now();
        let mut findings = Vec::new();
        let mut risks = Vec::new();
        let mut gaps = Vec::new();
        let mut recommended_plan_actions = Vec::new();
        let mut recommended_external_work = Vec::new();
        let registry = self.plans.registry().await?;
        let mut failed_steps = 0usize;
        let mut blocked_steps = 0usize;
        for plan_id in &registry.plans {
            let Some(plan) = self.plans.load(plan_id).await? else {
                gaps.push(format!("plan {plan_id} disappeared from the graph"));
                continue;
            };
            for step in &plan.steps {
                match step.status {
                    mm_data::PlanStepStatus::Failed => {
                        failed_steps += 1;
                        risks.push(format!(
                            "plan {} step {} failed: {}",
                            plan.id,
                            step.id,
                            step.last_error.as_deref().unwrap_or("no reason")
                        ));
                        recommended_plan_actions.push(format!(
                            "metamind.act(action=\"plan_retry_step\", plan_id=\"{}\", step_id=\"{}\")",
                            plan.id, step.id
                        ));
                    }
                    mm_data::PlanStepStatus::Blocked => {
                        blocked_steps += 1;
                        recommended_plan_actions.push(format!(
                            "unblock {}/{}: {}",
                            plan.id,
                            step.id,
                            step.last_error.as_deref().unwrap_or("waiting")
                        ));
                    }
                    mm_data::PlanStepStatus::Pending if step.owner == mm_data::PlanOwner::Agent => {
                        recommended_external_work.push(format!(
                            "agent.run for {}/{}: {}",
                            plan.id, step.id, step.title
                        ));
                    }
                    _ => {}
                }
            }
        }
        findings.push(IntrospectionFinding {
            area: "plans".into(),
            finding: format!(
                "{} non-terminal plans, {} failed steps, {} blocked steps",
                registry.plans.len(),
                failed_steps,
                blocked_steps
            ),
            suggestion: None,
        });
        let health = self.data.status().await?;
        if health.index_pending > 0 {
            risks.push(format!(
                "{} search documents are waiting on the index",
                health.index_pending
            ));
        }
        findings.push(IntrospectionFinding {
            area: "data".into(),
            finding: format!(
                "entities={} documents={} pending={} ready={}",
                health.entities, health.search_documents, health.index_pending, health.index_ready
            ),
            suggestion: None,
        });
        match self.memory.status().await {
            Ok(status) => findings.push(IntrospectionFinding {
                area: "memory".into(),
                finding: format!("{} explicit memory records", status.records),
                suggestion: None,
            }),
            Err(error) => gaps.push(format!("memory: {error}")),
        }
        if let Some(question) = question {
            findings.push(IntrospectionFinding {
                area: "question".into(),
                finding: format!("inspected: {question}"),
                suggestion: Some("delegate external work through agent.run".into()),
            });
        }
        let summary = if risks.is_empty() && gaps.is_empty() {
            "No immediate risks found in plans, data, or memory.".to_owned()
        } else {
            format!(
                "{} risks and {} gaps need attention.",
                risks.len(),
                gaps.len()
            )
        };
        Ok(IntrospectionReport {
            report_id: mm_data::rdf::new_id(),
            summary,
            findings,
            risks,
            gaps,
            recommended_plan_actions,
            recommended_external_work,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

fn compact_entity(entity: &mm_data::DataEntity) -> Value {
    json!({
        "iri": entity.iri,
        "types": entity.types,
        "label": entity.label,
        "graph": entity.graph,
        "revision": entity.revision,
        "properties": entity.properties,
    })
}

/// The read-only tool catalog advertised to sub-agents.
pub fn read_only_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "data.query".into(),
            description: "Read ontology entities by class, subject, or graph.".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "class": {"type": "string"},
                    "subject": {"type": "string"},
                    "graph": {"type": "string"},
                    "namespace": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                    "offset": {"type": "integer", "minimum": 0}
                }
            }),
        },
        ToolSchema {
            name: "data.search".into(),
            description: "Search ontology entities.".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string"},
                    "class": {"type": "string"},
                    "graph": {"type": "string"},
                    "namespace": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50}
                }
            }),
        },
        ToolSchema {
            name: "data.schema".into(),
            description: "List, get, or validate schemas.".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["action"],
                "properties": {
                    "action": {"type": "string", "enum": ["list", "get", "validate"]},
                    "schema": {"type": "string"},
                    "subject": {"type": "string"},
                    "class": {"type": "string"},
                    "graph": {"type": "string"}
                }
            }),
        },
        ToolSchema {
            name: "data.namespace".into(),
            description: "List, expand, or resolve namespaces.".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["action"],
                "properties": {
                    "action": {"type": "string", "enum": ["list", "expand", "resolve"]},
                    "iri": {"type": "string"}
                }
            }),
        },
        ToolSchema {
            name: "metamind.memory".into(),
            description: "Recall memories or inspect memory health. Read-only.".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["action"],
                "properties": {
                    "action": {"type": "string", "enum": ["recall", "status"]},
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 20}
                }
            }),
        },
    ]
}

/// A `PlanService`-compatible control helper used by `metamind.act`.
pub fn control_request(
    plan_id: &str,
    control: PlanControl,
    expected_revision: u64,
    note: Option<String>,
) -> PlanControlRequest {
    PlanControlRequest {
        plan_id: plan_id.into(),
        control,
        expected_revision,
        note,
    }
}
