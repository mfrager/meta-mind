//! The turn executor: drives one Turn from planning through completion.
//!
//! The executor owns the ordered work a turn needs — plan, route, pre-warm, call,
//! stream into chunks, run tools, finalize — while the state machines own the
//! legality of each step. With no LLM client configured it runs the deterministic
//! `Echo:` path used by tests and local sessions; with a client it uses the
//! provider-neutral `mm-llm` substrate.

use std::sync::Arc;

use async_trait::async_trait;
use mm_llm::{LlmClient, LlmRequest, Message, ProviderToolCall, Purpose, ToolSchema};
use mm_prompt::{PromptInstance, PromptMessage, PromptRequest, PromptStore};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::budget::{BudgetClass, BudgetLedger};
use crate::chunk::{Chunk, ChunkEvent, ChunkKind, ChunkState};
use crate::error::{CmeError, Result};
use crate::observability::{ObserverHandle, SmObserver};
use crate::planner::{ResponseModality, ToolPolicy, TurnContext, TurnPlanner};
use crate::prompt::PromptVariables;
use crate::routing::{ConversationRouter, ConversationRoutingRequest};
use crate::session::SessionContext;
use crate::turn::{ToolCallEmission, Turn, TurnEvent, TurnOutcome, TurnState};
use crate::voice::VoiceTurnConfig;

/// Bounds a model-tool exchange. A model that keeps asking for tools cannot
/// hold a turn open indefinitely.
const MAX_TOOL_ROUNDS: usize = 32;
/// Tool steps allowed in one turn.
const MAX_TOOL_STEPS: usize = 32;

/// How the executor treats a model that keeps requesting tools.
///
/// The conversation manager uses [`TurnLoopPolicy::ContinueUntilDone`]: a
/// tool result is appended to the transcript and the model gets another round
/// until it answers, subject to the hard round/step caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnLoopPolicy {
    /// Continue model/tool rounds until the model returns a final answer.
    #[default]
    ContinueUntilDone,
}

impl TurnLoopPolicy {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContinueUntilDone => "continue_until_done",
        }
    }
}

/// Runs a tool call and advertises the catalog to the model.
#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// The tools the model may call. The default advertises none, preserving
    /// the deterministic tool-less executor for callers that only need the seam.
    fn schemas(&self) -> Vec<ToolSchema> {
        Vec::new()
    }

    /// Run `name` with provider-supplied JSON arguments.
    async fn call(&self, name: &str, arguments: &str) -> Result<String>;

    /// Run a tool in the context of a specific session. Stateless handlers use
    /// the default; session-bound handlers (the CM conversation tools) override
    /// this so the executor does not need a second dispatch path.
    async fn call_for_session(
        &self,
        _session_id: &str,
        name: &str,
        arguments: &str,
    ) -> Result<String> {
        self.call(name, arguments).await
    }
}

/// A handler that refuses every call.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableTools;

#[async_trait]
impl ToolHandler for UnavailableTools {
    async fn call(&self, name: &str, _arguments: &str) -> Result<String> {
        Err(CmeError::Tool(format!("tool `{name}` is not available")))
    }
}

/// Asks the user before a confirmation-gated tool runs.
#[async_trait]
pub trait ConfirmationHandler: Send + Sync {
    /// Whether `name` may run with `arguments`.
    async fn confirm(&self, name: &str, arguments: &str) -> Result<bool>;
}

/// A handler that refuses every confirmation.
///
/// This is the safe default: a turn planned with `ToolPolicy::Confirm` cannot
/// run a tool until the caller wires a handler that can actually ask.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyUnconfirmed;

#[async_trait]
impl ConfirmationHandler for DenyUnconfirmed {
    async fn confirm(&self, _name: &str, _arguments: &str) -> Result<bool> {
        Ok(false)
    }
}

/// Reloads the bounded prompt variables from authoritative stores before a
/// model call. Implementations return an error when the store is closed; the
/// executor renders an honest `[STATE unavailable]` block rather than stale
/// state.
#[async_trait]
pub trait PromptContextProvider: Send + Sync {
    /// Build this session's current prompt variables.
    async fn prompt_variables(
        &self,
        session_id: &str,
    ) -> std::result::Result<PromptVariables, String>;
}

/// Executes turns.
pub struct TurnExecutor {
    planner: TurnPlanner,
    router: Option<ConversationRouter>,
    client: Option<Arc<dyn LlmClient>>,
    tools: Arc<dyn ToolHandler>,
    confirm: Arc<dyn ConfirmationHandler>,
    observer: Option<ObserverHandle>,
    loop_policy: TurnLoopPolicy,
    prompts: Option<Arc<PromptStore>>,
    prompt_variables: Option<PromptVariables>,
    prompt_provider: Option<Arc<dyn PromptContextProvider>>,
    prompt_variant: Option<String>,
    budget_class: BudgetClass,
}

impl std::fmt::Debug for TurnExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnExecutor")
            .field("router", &self.router)
            .field("has_client", &self.client.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for TurnExecutor {
    fn default() -> Self {
        Self::deterministic()
    }
}

impl TurnExecutor {
    /// The deterministic executor: no model, no tools, `Echo:` output.
    pub fn deterministic() -> Self {
        Self {
            planner: TurnPlanner,
            router: None,
            client: None,
            tools: Arc::new(UnavailableTools),
            confirm: Arc::new(DenyUnconfirmed),
            observer: None,
            loop_policy: TurnLoopPolicy::default(),
            prompts: None,
            prompt_variables: None,
            prompt_provider: None,
            prompt_variant: None,
            budget_class: BudgetClass::Standard,
        }
    }

    /// An executor backed by a model client and conversation router.
    pub fn with_llm(client: Arc<dyn LlmClient>, router: ConversationRouter) -> Self {
        Self {
            planner: TurnPlanner,
            router: Some(router),
            client: Some(client),
            tools: Arc::new(UnavailableTools),
            confirm: Arc::new(DenyUnconfirmed),
            observer: None,
            loop_policy: TurnLoopPolicy::default(),
            prompts: default_prompt_store(),
            prompt_variables: None,
            prompt_provider: None,
            prompt_variant: None,
            budget_class: BudgetClass::Standard,
        }
    }

    /// Attach an observer for turn and chunk transitions.
    pub fn set_observer(&mut self, observer: Arc<dyn SmObserver>) {
        self.observer = Some(observer);
    }

    /// Replace the tool handler.
    pub fn with_tools(mut self, tools: Arc<dyn ToolHandler>) -> Self {
        self.tools = tools;
        self
    }

    /// Replace the confirmation handler used by `ToolPolicy::Confirm` turns.
    pub fn with_confirmation_handler(mut self, confirm: Arc<dyn ConfirmationHandler>) -> Self {
        self.confirm = confirm;
        self
    }

    /// Select the model/tool continuation policy.
    pub fn with_loop_policy(mut self, loop_policy: TurnLoopPolicy) -> Self {
        self.loop_policy = loop_policy;
        self
    }

    /// The continuation policy in force.
    pub fn loop_policy(&self) -> TurnLoopPolicy {
        self.loop_policy
    }

    /// Attach the reviewed prompt tree. Without one, a model call fails closed
    /// with `PROMPT_UNAVAILABLE` before reaching the provider.
    pub fn with_prompts(mut self, prompts: Arc<PromptStore>) -> Self {
        self.prompts = Some(prompts);
        self
    }

    /// Remove the prompt tree, for a fail-closed test or a caller that must
    /// prove no provider is reached without a PromptInstance.
    pub fn without_prompts(mut self) -> Self {
        self.prompts = None;
        self
    }

    /// Supply the prebuilt variables for this turn's prompt instance.
    pub fn with_prompt_variables(mut self, variables: PromptVariables) -> Self {
        self.prompt_variables = Some(variables);
        self
    }

    /// Reload prompt variables from stores before every model call.
    pub fn with_prompt_provider(mut self, provider: Arc<dyn PromptContextProvider>) -> Self {
        self.prompt_provider = Some(provider);
        self
    }

    /// Select a prompt variant (`pending`, `resume`, or `repair`).
    pub fn with_prompt_variant(mut self, variant: impl Into<String>) -> Self {
        self.prompt_variant = Some(variant.into());
        self
    }

    /// Select the initial budget class.
    pub fn with_budget_class(mut self, class: BudgetClass) -> Self {
        self.budget_class = class;
        self
    }

    /// Run a turn to completion. A model or tool failure marks the turn failed and
    /// is reported in the outcome; a state-machine violation is an error.
    pub async fn execute(
        &mut self,
        turn: &mut Turn,
        session: &SessionContext,
        voice: Option<VoiceTurnConfig>,
    ) -> Result<TurnOutcome> {
        turn.observer = self.observer.clone();
        let query = turn
            .plan
            .as_ref()
            .map(|plan| plan.user_query.clone())
            .unwrap_or_default();
        let mut ctx = TurnContext::user_input(turn.turn_id.clone(), turn.session_id.clone(), query);
        ctx.voice_config = voice;

        // 1. Planning. The planner is deterministic, so the plan is testable and
        //    the turn machine never sees an ambiguous shape.
        let plan = self.planner.build_plan(&ctx, session)?;
        turn.set_plan(plan.clone());
        turn.plan_ready()?;

        // 2. Routing.
        let speculative = plan
            .voice_config
            .map(|voice| voice.speculative)
            .unwrap_or(false);
        let model = match &self.router {
            Some(router) => {
                let mut request =
                    ConversationRoutingRequest::text(plan.user_query.clone(), plan.modality)
                        .with_cost_budget(plan.budget.cost_micros);
                if let Some(voice) = plan.voice_config {
                    request = request.with_voice_tier(voice.tier);
                }
                request.is_proactive = plan.metadata.is_proactive;
                request.preference_profile = session.preferences.clone();
                match router.choose(&request) {
                    Ok(decision) => decision.model,
                    Err(error) => return self.fail(turn, error.to_string()),
                }
            }
            None => "stub-model".to_owned(),
        };
        turn.route_selected(speculative, model.clone())?;

        // 3. Speculative pre-warm. The substrate has no pre-warm endpoint yet; the
        //    state is still entered and exited so the turn matches the design.
        if speculative {
            debug!(turn = %turn.turn_id, "speculative pre-warm requested");
            turn.dispatch(TurnEvent::SpeculativeReady)?;
        }

        // 4. Executing and streaming. Tool-capable turns alternate model calls
        //    with tool results until the model answers or the caps are reached.
        turn.dispatch(TurnEvent::LlmStreamStart)?;
        turn.spawn_chunks()?;

        let reply = match &self.client {
            Some(client) => {
                let mut budget = BudgetLedger::new(self.budget_class);
                let mut variables = self
                    .prompt_variables
                    .clone()
                    .unwrap_or_else(|| PromptVariables::for_turn(&plan, session));
                let prompt = match self.render_prompt(&plan, &variables, budget.snapshot()) {
                    Ok(prompt) => prompt,
                    Err(error) => {
                        return self.fail(turn, format!("PROMPT_UNAVAILABLE: {error}"));
                    }
                };
                turn.set_prompt(
                    prompt.instance_id.clone(),
                    prompt.hash.clone(),
                    prompt.variant.clone(),
                );
                let tools = if plan.tool_policy == ToolPolicy::None {
                    Vec::new()
                } else {
                    self.tools.schemas()
                };
                let mut messages = messages_from_instance(&prompt);
                let mut rounds = 0usize;
                let mut tool_steps = 0usize;
                loop {
                    rounds += 1;
                    if let Err(error) = budget.reserve_round() {
                        return self.fail(turn, error.to_string());
                    }
                    // Rebuild before every model call, including every tool
                    // round, and after any tool-side state/data/frame write.
                    variables = self.refresh_variables(&plan, &mut variables).await;
                    let prompt = match self.render_prompt(&plan, &variables, budget.snapshot()) {
                        Ok(prompt) => prompt,
                        Err(error) => {
                            return self.fail(turn, format!("PROMPT_UNAVAILABLE: {error}"));
                        }
                    };
                    turn.set_prompt(
                        prompt.instance_id.clone(),
                        prompt.hash.clone(),
                        prompt.variant.clone(),
                    );
                    if let Some(first) = messages.first_mut() {
                        *first = Message::system(prompt.text.clone());
                    }
                    let request =
                        self.llm_request(&plan, messages.clone(), tools.clone(), &model);
                    if let Err(error) = budget.reserve_tokens(u64::from(request.max_tokens)) {
                        return self.fail(turn, error.to_string());
                    }
                    let response = match client.complete(&request).await {
                        Ok(response) => response,
                        Err(error) => return self.fail(turn, error.to_string()),
                    };
                    let used = u64::from(response.usage.tokens_in)
                        .saturating_add(u64::from(response.usage.tokens_out));
                    if let Err(error) = budget.settle_tokens(Some(used)) {
                        return self.fail(turn, error.to_string());
                    }
                    if response.tool_calls.is_empty() {
                        break response.text;
                    }
                    if plan.tool_policy == ToolPolicy::None {
                        return self.fail(
                            turn,
                            "model requested a tool but the turn's policy forbids tools".to_owned(),
                        );
                    }
                    if rounds >= MAX_TOOL_ROUNDS {
                        return self.fail(turn, "tool round cap".to_owned());
                    }
                    if tool_steps >= MAX_TOOL_STEPS {
                        return self.fail(turn, "tool step cap".to_owned());
                    }
                    let remaining = MAX_TOOL_STEPS - tool_steps;
                    let calls = response
                        .tool_calls
                        .iter()
                        .take(remaining)
                        .cloned()
                        .collect::<Vec<_>>();
                    if let Err(error) = budget.reserve_steps(calls.len() as u32) {
                        return self.fail(turn, error.to_string());
                    }
                    messages.push(Message::assistant_calls(
                        response.text.clone(),
                        calls.clone(),
                    ));
                    let results = self.run_tool_calls(turn, &calls).await?;
                    tool_steps += calls.len();
                    messages.extend(results);
                }
            }
            None => format!("Echo: {}", plan.user_query),
        };

        if !reply.is_empty() {
            self.stream_text(turn, &reply)?;
        }
        self.flush_structured(turn, &reply)?;
        self.flush_audio(turn)?;

        // 5. Finalize.
        let outcome = turn.finalize()?;
        debug!(
            turn = %outcome.turn_id,
            state = %outcome.state,
            chunks = outcome.chunks_completed,
            model = outcome.model.as_deref().unwrap_or("none"),
            "turn finished"
        );
        Ok(outcome)
    }

    /// Refresh bounded state from the provider. A closed store leaves an honest
    /// `[STATE unavailable]` block instead of silently reusing stale state.
    async fn refresh_variables(
        &self,
        plan: &crate::turn::TurnPlan,
        current: &mut PromptVariables,
    ) -> PromptVariables {
        let Some(provider) = &self.prompt_provider else {
            return current.clone();
        };
        match provider.prompt_variables(plan.session_id.as_ref()).await {
            Ok(variables) => variables,
            Err(error) => {
                let mut stale = current.clone();
                let block = crate::state_block::StateBlock::unavailable(error);
                stale.set("state_block", block.text);
                stale.set("state_json", "");
                stale
            }
        }
    }

    /// Render the reviewed strategy before the first model call. A missing or
    /// invalid tree is a `PROMPT_UNAVAILABLE` refusal, not an ad hoc prompt.
    fn render_prompt(
        &self,
        plan: &crate::turn::TurnPlan,
        variables: &PromptVariables,
        budget: crate::budget::BudgetSnapshot,
    ) -> std::result::Result<PromptInstance, String> {
        let prompts = self
            .prompts
            .as_ref()
            .ok_or_else(|| "prompt tree is unavailable; no model call was made".to_owned())?;
        let mut variables = variables.clone();
        variables.set("budget_class", budget.class);
        variables.set("budget_rounds", budget.rounds.to_string());
        variables.set("budget_tokens", budget.tokens.to_string());
        variables.set("budget_wall_ms", budget.wall_ms.to_string());
        let state_block = variables.get("state_block").map(str::to_owned);
        // A pending variant is only valid while pending work remains. After a
        // tool round clears the last item, the rebuild returns to the standard
        // strategy instead of claiming work that no longer exists.
        let variant = match self.prompt_variant.as_deref() {
            Some("pending")
                if variables
                    .get("pending_summary")
                    .is_some_and(|pending| pending != "none") =>
            {
                Some("pending".into())
            }
            Some("pending") => None,
            Some(other) => Some(other.to_owned()),
            None => None,
        };
        let request = PromptRequest {
            application: "cm.turn".into(),
            variant,
            variables: variables.as_map().clone(),
            state_block,
            transcript: Vec::new(),
            user_input: Some(plan.user_query.clone()),
        };
        prompts
            .render_request(&request)
            .map_err(|error| error.to_string())
    }

    /// Build the provider-neutral request for a plan and its current transcript.
    fn llm_request(
        &self,
        plan: &crate::turn::TurnPlan,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        model: &str,
    ) -> LlmRequest {
        let mut request = LlmRequest::new(Purpose::Converse, messages)
            .with_cache(mm_llm::CacheMode::Use)
            .with_model(model);
        if !tools.is_empty() {
            request = request.with_tools(tools);
        }
        request.max_tokens = u32::try_from(plan.budget.tokens)
            .unwrap_or(u32::MAX)
            .min(4_096);
        request.temperature = 0.2;
        request
    }

    /// Run the model's tool calls under the turn's policy and return the tool
    /// messages to append to the model transcript.
    ///
    /// Refuses calls when the policy forbids tools. Under `ToolPolicy::Confirm`
    /// the turn pauses in `waiting_confirm` and the chunk in
    /// `tool_waiting_confirm` until the handler answers; a refusal records the
    /// denial and lets the rest of the turn finish instead of failing it.
    async fn run_tool_calls(
        &self,
        turn: &mut Turn,
        calls: &[ProviderToolCall],
    ) -> Result<Vec<Message>> {
        let policy = turn
            .plan
            .as_ref()
            .map(|plan| plan.tool_policy)
            .unwrap_or(ToolPolicy::None);
        if policy == ToolPolicy::None {
            return Err(CmeError::Tool(
                "model requested a tool but the turn's policy forbids tools".to_owned(),
            ));
        }
        let mut messages = Vec::with_capacity(calls.len());
        for call in calls {
            let approved = if policy == ToolPolicy::Confirm {
                turn.dispatch(TurnEvent::ConfirmationRequired)?;
                if let Some(chunk) = tool_chunk(turn) {
                    chunk.dispatch(ChunkEvent::ToolCall)?;
                    chunk.dispatch(ChunkEvent::ConfirmationRequired)?;
                }
                let approved = self.confirm.confirm(&call.name, &call.arguments).await?;
                if let Some(chunk) = tool_chunk(turn) {
                    chunk.dispatch(ChunkEvent::ConfirmationAnswer)?;
                }
                turn.dispatch(TurnEvent::ConfirmationAnswer)?;
                approved
            } else {
                true
            };

            let (result, ok) = if approved {
                turn.dispatch(TurnEvent::ToolCall)?;
                if let Some(chunk) = tool_chunk(turn) {
                    if chunk.state() == ChunkState::Streaming.as_str() {
                        chunk.dispatch(ChunkEvent::ToolCall)?;
                    }
                }
                let result = match self
                    .tools
                    .call_for_session(turn.session_id.as_str(), &call.name, &call.arguments)
                    .await
                {
                    Ok(result) => {
                        let ok = tool_result_ok(&result);
                        (result, ok)
                    }
                    Err(error) => (format!("tool `{}` failed: {error}", call.name), false),
                };
                if let Some(chunk) = tool_chunk(turn) {
                    chunk.dispatch(ChunkEvent::ToolResult)?;
                }
                turn.dispatch(TurnEvent::ToolResult)?;
                result
            } else {
                if let Some(chunk) = tool_chunk(turn) {
                    chunk.dispatch(ChunkEvent::ToolResult)?;
                }
                (format!("tool `{}` denied by the user", call.name), false)
            };

            turn.tool_results.push(result.clone());
            turn.tool_calls.push(ToolCallEmission {
                call_id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                result: result.clone(),
                ok,
            });
            messages.push(Message::tool(call.id.clone(), call.name.clone(), result));
        }
        Ok(messages)
    }

    /// Split the reply across streaming text chunks in order, so a multi-section
    /// plan keeps the text readable instead of duplicating it.
    fn stream_text(&mut self, turn: &mut Turn, reply: &str) -> Result<()> {
        let ids: Vec<String> = turn
            .chunks
            .iter()
            .filter(|chunk| {
                matches!(chunk.directive.kind, ChunkKind::Text) && chunk.state() == "streaming"
            })
            .map(|chunk| chunk.chunk_id.to_string())
            .collect();
        for (id, segment) in ids.iter().zip(split_text(reply, ids.len())) {
            for token in segment.split_inclusive(' ') {
                turn.token_to(id, token)?;
            }
        }
        Ok(())
    }

    /// Give buffered structured chunks a JSON payload derived from the answer.
    fn flush_structured(&mut self, turn: &mut Turn, reply: &str) -> Result<()> {
        for chunk in &mut turn.chunks {
            if matches!(chunk.directive.kind, ChunkKind::Structured) && chunk.state() == "buffering"
            {
                let payload = serde_json::json!({
                    "label": chunk.directive.label,
                    "text": reply,
                });
                chunk.flush(&payload.to_string())?;
            }
        }
        Ok(())
    }

    /// Audio chunks are marked complete by the voice pipeline in Phase 5's
    /// integration; in a text turn they carry no audio.
    fn flush_audio(&mut self, turn: &mut Turn) -> Result<()> {
        for chunk in &mut turn.chunks {
            if matches!(chunk.directive.kind, ChunkKind::Audio) && chunk.state() == "buffering" {
                chunk.flush("")?;
            }
        }
        Ok(())
    }

    /// Fail the turn and return its outcome.
    fn fail(&self, turn: &mut Turn, detail: String) -> Result<TurnOutcome> {
        turn.dispatch(TurnEvent::Error(detail))?;
        Ok(turn.outcome())
    }
}

/// Prefer a structured `{ "ok": false }` result over the generic success case.
fn tool_result_ok(result: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(result)
        .ok()
        .and_then(|value| value.get("ok").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

/// The turn's tool-call chunk, when the plan has one.
fn tool_chunk(turn: &mut Turn) -> Option<&mut Chunk> {
    turn.chunks
        .iter_mut()
        .find(|chunk| matches!(chunk.directive.kind, ChunkKind::ToolCall))
}

/// Split text into `parts` contiguous word groups; joins preserve the words.
fn split_text(text: &str, parts: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if parts == 0 {
        return Vec::new();
    }
    if words.is_empty() {
        return vec![String::new(); parts];
    }
    let mut result = Vec::with_capacity(parts);
    let base = words.len() / parts;
    let remainder = words.len() % parts;
    let mut index = 0;
    for part in 0..parts {
        let take = base + usize::from(part < remainder);
        result.push(words[index..index + take].join(" "));
        index += take;
    }
    result
}

fn messages_from_instance(instance: &PromptInstance) -> Vec<Message> {
    instance
        .messages
        .iter()
        .map(|message: &PromptMessage| match message.role.as_str() {
            "user" => Message::user(message.content.clone()),
            "assistant" => Message::assistant(message.content.clone()),
            _ => Message::system(message.content.clone()),
        })
        .collect()
}

fn default_prompt_store() -> Option<Arc<PromptStore>> {
    let root = std::env::var_os("MM_PROMPT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts")
        });
    PromptStore::load(root).ok().map(Arc::new)
}

/// Whether the outcome completed successfully.
pub fn completed(outcome: &TurnOutcome) -> bool {
    outcome.state == TurnState::Completed.as_str()
}

/// A convenience alias for a modality's display name.
pub fn modality_name(modality: ResponseModality) -> &'static str {
    modality.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_splits_preserve_every_word() {
        let parts = split_text("one two three four five", 2);
        assert_eq!(parts, vec!["one two three", "four five"]);
        assert_eq!(split_text("solo", 2), vec!["solo", ""]);
        assert_eq!(split_text("", 2), vec!["", ""]);
    }
}
