//! The Process pass: do the interpreted work and produce the reply.
//!
//! Phase 3 extracts the model/tool loop from [`TurnExecutor`](crate::TurnExecutor)
//! so the legacy `cm.turn.v1` path and the three-pass `cm.process.v1` path run the
//! same code. The pass owns prompt rendering, budget accounting, tool dispatch,
//! confirmation, and the final-reply contract; the executor remains the adapter
//! that turns the result into chunks and turn telemetry.

use std::sync::Arc;

use mm_data::CleanLog;
use mm_llm::{CacheMode, LlmClient, LlmRequest, Message, ProviderToolCall, Purpose};
use mm_prompt::{PromptInstance, PromptMessage, PromptRequest, PromptStore};

use crate::budget::{BudgetClass, BudgetLedger, BudgetSnapshot, TurnEnvelope};
use crate::chunk::{ChunkEvent, ChunkState};
use crate::error::{CmeError, Result};
use crate::executor::{ConfirmationHandler, PromptContextProvider, ToolHandler};
use crate::pass_scope::PassScope;
use crate::planner::ToolPolicy;
use crate::prompt::PromptVariables;
use crate::runner::RetryContext;
use crate::turn::{ToolCallEmission, Turn, TurnEvent, TurnPlan};
use crate::verification::{is_mutating_call, ProcessContext, ProcessMutation};

/// Bounds a model-tool exchange. A model that keeps asking for tools cannot
/// hold a turn open indefinitely.
pub const MAX_TOOL_ROUNDS: usize = 32;
/// Tool steps allowed in one turn.
pub const MAX_TOOL_STEPS: usize = 32;

/// Everything the Process pass needs from its turn and the Interpret result.
#[derive(Debug, Clone)]
pub struct ProcessRequest {
    /// Prompt strategy to render (`cm.turn.v1` legacy, `cm.process.v1` V2).
    pub application: String,
    /// The pass scope, or `None` for the legacy seventeen-tool surface.
    pub scope: Option<PassScope>,
    /// The Interpret brief, when running under three-pass.
    pub process: Option<ProcessContext>,
    /// Retry context, when this is a retry attempt.
    pub retry: Option<RetryContext>,
    /// The clean log used as transcript. Legacy leaves it empty.
    pub clean_log: CleanLog,
    /// Prompt variant for the legacy path.
    pub variant: Option<String>,
    /// Whether an empty final answer is a hard `PROCESS_INCOMPLETE` failure.
    pub require_reply: bool,
    /// Whether child operations reserve from this pass's ledger.
    pub child_budget: bool,
}

impl ProcessRequest {
    /// The legacy Process request: no scope, no brief, no clean log.
    pub fn legacy(application: impl Into<String>, variant: Option<String>) -> Self {
        Self {
            application: application.into(),
            scope: None,
            process: None,
            retry: None,
            clean_log: CleanLog::default(),
            variant,
            require_reply: false,
            child_budget: false,
        }
    }
}

/// The result of one Process pass.
#[derive(Debug, Clone)]
pub struct ProcessRun {
    /// The final assistant text. This is the reply, never a summary.
    pub reply: String,
    /// The Process ledger after the final round.
    pub budget: BudgetSnapshot,
    /// Model rounds used.
    pub rounds: u32,
    /// Tool steps used.
    pub tool_steps: u32,
    /// The ledger that produced the snapshot, so the turn can absorb it.
    pub ledger: BudgetLedger,
    /// Tool calls made by this Process attempt.
    pub tool_emissions: Vec<ToolCallEmission>,
    /// Durable effects observed in this attempt, for a later retry.
    pub mutations: Vec<ProcessMutation>,
    /// Prompt tokens the attempt consumed.
    pub tokens_in: u32,
    /// Completion tokens the attempt produced.
    pub tokens_out: u32,
}

/// Runs the model/tool loop for one Process pass.
pub struct ProcessPass {
    client: Arc<dyn LlmClient>,
    prompts: Option<Arc<PromptStore>>,
    tools: Arc<dyn ToolHandler>,
    confirm: Arc<dyn ConfirmationHandler>,
    provider: Option<Arc<dyn PromptContextProvider>>,
    budget_class: BudgetClass,
}

impl std::fmt::Debug for ProcessPass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessPass")
            .field("budget_class", &self.budget_class)
            .field("has_prompts", &self.prompts.is_some())
            .finish_non_exhaustive()
    }
}

impl ProcessPass {
    /// Build a pass over the executor's shared dependencies.
    pub fn new(
        client: Arc<dyn LlmClient>,
        prompts: Option<Arc<PromptStore>>,
        tools: Arc<dyn ToolHandler>,
        confirm: Arc<dyn ConfirmationHandler>,
        provider: Option<Arc<dyn PromptContextProvider>>,
        budget_class: BudgetClass,
    ) -> Self {
        Self {
            client,
            prompts,
            tools,
            confirm,
            provider,
            budget_class,
        }
    }

    /// Run until the model returns text, the loop cap is reached, or the budget
    /// is exhausted. Every model call re-renders the prompt and refreshes state.
    pub async fn run(
        &self,
        turn: &mut Turn,
        plan: &TurnPlan,
        model: &str,
        variables: PromptVariables,
        request: ProcessRequest,
    ) -> Result<ProcessRun> {
        self.run_accounted(turn, plan, model, variables, request, None)
            .await
    }

    /// Run the pass and charge its ledger to the turn envelope.
    pub async fn run_accounted(
        &self,
        turn: &mut Turn,
        plan: &TurnPlan,
        model: &str,
        mut variables: PromptVariables,
        request: ProcessRequest,
        account: Option<&mut TurnEnvelope>,
    ) -> Result<ProcessRun> {
        let prompts = self.prompts.as_ref().ok_or_else(|| {
            CmeError::Driver(
                "PROMPT_UNAVAILABLE: prompt tree is unavailable; no model call was made".into(),
            )
        })?;
        let mut budget = BudgetLedger::new(self.budget_class);
        let tools = self.visible_tools(plan, request.scope);
        let mut messages: Vec<Message> = Vec::new();
        let mut rounds = 0u32;
        let mut tool_steps = 0usize;
        let mut tool_emissions = Vec::new();
        let mut mutations = Vec::new();
        let mut tokens_in = 0u32;
        let mut tokens_out = 0u32;

        let reply = loop {
            rounds += 1;
            budget.reserve_round()?;
            variables = self.refresh_variables(plan, variables).await;
            let (prompt, variant) = self.render_prompt(
                prompts,
                plan,
                variables.clone(),
                budget.snapshot(),
                &request,
            )?;
            turn.set_prompt(prompt.instance_id.clone(), prompt.hash.clone(), variant);
            if messages.is_empty() {
                messages = messages_from_instance(&prompt);
                if let Some(retry) = &request.retry {
                    messages.push(Message::user(render_retry_context(retry)));
                }
            } else if let Some(first) = messages.first_mut() {
                *first = Message::system(prompt.text.clone());
            }

            let mut llm_request = LlmRequest::new(Purpose::Converse, messages.clone())
                .with_cache(CacheMode::Use)
                .with_model(model);
            if !tools.is_empty() {
                llm_request = llm_request.with_tools(tools.clone());
            }
            llm_request.max_tokens = u32::try_from(plan.budget.tokens)
                .unwrap_or(u32::MAX)
                .min(4_096);
            llm_request.temperature = 0.2;
            budget.reserve_tokens(u64::from(llm_request.max_tokens))?;
            let response = self.client.complete(&llm_request).await?;
            let used = u64::from(response.usage.tokens_in)
                .saturating_add(u64::from(response.usage.tokens_out));
            tokens_in = tokens_in.saturating_add(response.usage.tokens_in);
            tokens_out = tokens_out.saturating_add(response.usage.tokens_out);
            budget.settle_tokens(Some(used))?;

            if response.tool_calls.is_empty() {
                break response.text;
            }
            if plan.tool_policy == ToolPolicy::None {
                return Err(CmeError::Driver(
                    "model requested a tool but the turn's policy forbids tools".into(),
                ));
            }
            if rounds as usize >= MAX_TOOL_ROUNDS {
                return Err(CmeError::Driver("tool round cap".into()));
            }
            if tool_steps >= MAX_TOOL_STEPS {
                return Err(CmeError::Driver("tool step cap".into()));
            }
            let remaining = MAX_TOOL_STEPS - tool_steps;
            let calls = response
                .tool_calls
                .iter()
                .take(remaining)
                .cloned()
                .collect::<Vec<_>>();
            budget.reserve_steps(calls.len() as u32)?;
            messages.push(Message::assistant_calls(
                response.text.clone(),
                calls.clone(),
            ));
            let results = self
                .run_tool_calls(
                    turn,
                    &calls,
                    request.scope,
                    request.child_budget,
                    &mut budget,
                    &mut tool_emissions,
                    &mut mutations,
                )
                .await?;
            tool_steps += calls.len();
            messages.extend(results);
        };

        if request.require_reply && reply.trim().is_empty() {
            return Err(CmeError::Driver(
                "PROCESS_INCOMPLETE: process produced no final text".into(),
            ));
        }
        if let Some(envelope) = account {
            envelope.absorb(&budget)?;
        }
        Ok(ProcessRun {
            reply,
            budget: budget.snapshot(),
            rounds,
            tool_steps: tool_steps as u32,
            ledger: budget,
            tool_emissions,
            mutations,
            tokens_in,
            tokens_out,
        })
    }

    fn visible_tools(&self, plan: &TurnPlan, scope: Option<PassScope>) -> Vec<mm_llm::ToolSchema> {
        if plan.tool_policy == ToolPolicy::None {
            return Vec::new();
        }
        match scope {
            Some(scope) => self.tools.schemas_for(scope),
            None => self.tools.schemas(),
        }
    }

    async fn refresh_variables(
        &self,
        plan: &TurnPlan,
        current: PromptVariables,
    ) -> PromptVariables {
        let Some(provider) = &self.provider else {
            return current;
        };
        match provider.prompt_variables(plan.session_id.as_ref()).await {
            Ok(variables) => variables,
            Err(error) => {
                let mut stale = current;
                let block = crate::state_block::StateBlock::unavailable(error);
                stale.set("state_block", block.text);
                stale.set("state_json", "");
                stale
            }
        }
    }

    fn render_prompt(
        &self,
        prompts: &PromptStore,
        plan: &TurnPlan,
        variables: PromptVariables,
        budget: BudgetSnapshot,
        request: &ProcessRequest,
    ) -> Result<(PromptInstance, Option<String>)> {
        let mut variables = variables;
        let state_block = variables.get("state_block").unwrap_or_default().to_owned();
        variables.set("user_input", plan.user_query.clone());
        apply_budget(&mut variables, budget);
        if let Some(scope) = request.scope {
            variables.set("tools_summary", scope.visible_tools().join(", "));
        }
        apply_process_context(&mut variables, plan, request.process.as_ref());

        let variant = match request.variant.as_deref() {
            Some("pending")
                if variables
                    .get("pending_summary")
                    .is_some_and(|pending| pending != "none") =>
            {
                Some("pending".to_owned())
            }
            Some("pending") => None,
            Some(other) => Some(other.to_owned()),
            None => None,
        };
        let mut values = variables.into_map();
        if let Some(strategy) = prompts.strategy(&request.application) {
            for slot in &strategy.slots {
                values.entry(slot.clone()).or_default();
            }
        }
        let transcript = transcript_messages(&request.clean_log);
        let prompt_request = PromptRequest {
            application: request.application.clone(),
            variant,
            variables: values,
            state_block: Some(state_block),
            transcript,
            user_input: Some(plan.user_query.clone()),
        };
        let instance = prompts
            .render_request(&prompt_request)
            .map_err(|error| CmeError::Driver(format!("PROMPT_UNAVAILABLE: {error}")))?;
        Ok((instance, prompt_request.variant))
    }

    /// Run the model's tool calls under the turn's policy and return the tool
    /// messages to append to the model transcript.
    async fn run_tool_calls(
        &self,
        turn: &mut Turn,
        calls: &[ProviderToolCall],
        scope: Option<PassScope>,
        child_budget: bool,
        budget: &mut BudgetLedger,
        tool_emissions: &mut Vec<ToolCallEmission>,
        mutations: &mut Vec<ProcessMutation>,
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
            if let Some(scope) = scope {
                let action = action_of(&call.arguments);
                if !scope.allows(&call.name, action.as_deref()) {
                    let result = format!(
                        "tool `{}` is not available in the {} pass",
                        call.name,
                        scope.as_str()
                    );
                    let emission = ToolCallEmission {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                        result: result.clone(),
                        ok: false,
                    };
                    turn.tool_results.push(result.clone());
                    turn.tool_calls.push(emission.clone());
                    tool_emissions.push(emission);
                    messages.push(Message::tool(call.id.clone(), call.name.clone(), result));
                    continue;
                }
            }

            let child_ceiling = child_budget
                .then(|| child_token_ceiling(&call.name))
                .flatten();
            let mut reserved = None;
            if let Some(ceiling) = child_ceiling {
                match budget.reserve_tokens(ceiling) {
                    Ok(()) => reserved = Some(ceiling),
                    Err(error) => {
                        let result = json_error("BUDGET", &error.to_string());
                        let emission = ToolCallEmission {
                            call_id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                            result: result.clone(),
                            ok: false,
                        };
                        turn.tool_results.push(result.clone());
                        turn.tool_calls.push(emission.clone());
                        tool_emissions.push(emission);
                        messages.push(Message::tool(call.id.clone(), call.name.clone(), result));
                        continue;
                    }
                }
            }

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

            let (mut result, mut ok) = if approved {
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

            if let Some(ceiling) = reserved {
                let actual =
                    child_usage(&call.name, &result).unwrap_or(if ok { ceiling } else { 0 });
                let settled = budget.settle_tokens(Some(actual.min(ceiling)));
                if actual > ceiling {
                    result = json_error(
                        "BUDGET",
                        &format!(
                            "child operation used {actual} tokens, above its {ceiling}-token reservation"
                        ),
                    );
                    ok = false;
                } else if let Err(error) = settled {
                    result = json_error("BUDGET", &error.to_string());
                    ok = false;
                }
            }

            let emission = ToolCallEmission {
                call_id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                result: result.clone(),
                ok,
            };
            if ok {
                if let Some(effect) = process_effect(&call.name, &call.arguments, &result) {
                    mutations.push(effect);
                }
            }
            turn.tool_results.push(result.clone());
            turn.tool_calls.push(emission.clone());
            tool_emissions.push(emission);
            messages.push(Message::tool(call.id.clone(), call.name.clone(), result));
        }
        Ok(messages)
    }
}

/// Render the Interpret brief into the Process prompt slots.
pub fn apply_process_context(
    variables: &mut PromptVariables,
    plan: &TurnPlan,
    process: Option<&ProcessContext>,
) {
    let task = process
        .map(|context| context.task.trim())
        .filter(|task| !task.is_empty())
        .unwrap_or(plan.user_query.as_str());
    variables.set("process_task", task);
    let constraints = process
        .map(|context| context.constraints.as_slice())
        .unwrap_or_default();
    variables.set(
        "process_constraints",
        if constraints.is_empty() {
            "none".to_owned()
        } else {
            constraints
                .iter()
                .map(|constraint| format!("- {constraint}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    let suggested = process
        .and_then(|context| context.suggested_tools.as_ref())
        .filter(|tools| !tools.is_empty())
        .map(|tools| tools.join(", "))
        .unwrap_or_else(|| "none".to_owned());
    variables.set("process_suggested_tools", suggested);
}

/// Render the retry section. It carries labels, reasoning, confidence, and the
/// model-authored error, never an internal verification prompt or secret.
pub fn render_retry_context(retry: &RetryContext) -> String {
    let mut text = format!(
        "RETRY_CONTEXT\nattempt: {}/{}\nprior_reply_digest: {}",
        retry.attempt, retry.max_attempts, retry.prior_reply_digest
    );
    if retry.failures.is_empty() {
        text.push_str("\nfailed_checks: none");
    } else {
        text.push_str("\nfailed_checks:");
        for failure in &retry.failures {
            text.push_str(&format!(
                "\n- {}: {} (confidence {:.2})",
                failure.label,
                truncate(&failure.reasoning, 1_000),
                failure.confidence
            ));
        }
    }
    if retry.prior_effects.is_empty() {
        text.push_str("\nprior_effects: none");
    } else {
        text.push_str("\nprior_effects (already durable; reconcile, do not duplicate):");
        for effect in &retry.prior_effects {
            text.push_str(&format!(
                "\n- {} {}{}{}",
                effect.tool,
                effect.target.as_deref().unwrap_or("(unnamed)"),
                effect
                    .revision
                    .map(|revision| format!(" revision {revision}"))
                    .unwrap_or_default(),
                effect
                    .summary
                    .as_deref()
                    .map(|summary| format!(": {}", truncate(summary, 300)))
                    .unwrap_or_default()
            ));
        }
    }
    text.push_str(&format!("\nerror: {}", truncate(&retry.error, 2_000)));
    text
}

fn process_effect(name: &str, arguments: &str, result: &str) -> Option<ProcessMutation> {
    let arguments_value = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
    let action = arguments_value
        .get("action")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    if !is_mutating_call(name, action.as_deref()) {
        return None;
    }
    let result_value = serde_json::from_str::<serde_json::Value>(result).ok()?;
    let payload = result_value.get("result").unwrap_or(&result_value);
    let target =
        find_string(payload, &["iri", "id", "subject", "planId", "memory_id"]).or_else(|| {
            payload
                .get("entity")
                .and_then(|entity| entity.get("iri"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        });
    let revision = find_u64(payload, &["revision", "dataRevision"]);
    Some(ProcessMutation {
        tool: name.to_owned(),
        action,
        arguments_digest: digest(arguments),
        target,
        revision,
        summary: Some(truncate(result, 500)),
    })
}

fn find_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    })
}

fn find_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
}

fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn apply_budget(variables: &mut PromptVariables, budget: BudgetSnapshot) {
    variables.set("budget_class", budget.class);
    variables.set("budget_rounds", budget.rounds.to_string());
    variables.set("budget_tokens", budget.tokens.to_string());
    variables.set("budget_wall_ms", budget.wall_ms.to_string());
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn transcript_messages(log: &CleanLog) -> Vec<PromptMessage> {
    log.turns
        .iter()
        .map(|turn| PromptMessage {
            role: match turn.role {
                mm_data::CleanRole::User => "user".into(),
                mm_data::CleanRole::Assistant => "assistant".into(),
            },
            content: turn.text.clone(),
        })
        .collect()
}

fn messages_from_instance(instance: &PromptInstance) -> Vec<Message> {
    instance
        .messages
        .iter()
        .map(|message| match message.role.as_str() {
            "user" => Message::user(message.content.clone()),
            "assistant" => Message::assistant(message.content.clone()),
            _ => Message::system(message.content.clone()),
        })
        .collect()
}

fn action_of(arguments: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|value| {
            value
                .get("action")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
}

/// Whether a structured result reports failure. Plain text defaults to success.
fn tool_result_ok(result: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(result)
        .ok()
        .and_then(|value| value.get("ok").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

/// The token ceiling reserved for a child operation before it starts.
fn child_token_ceiling(name: &str) -> Option<u64> {
    match name {
        "metamind.think" => Some(BudgetClass::Think.limits().tokens),
        "metamind.plan" => Some(BudgetClass::Plan.limits().tokens),
        "agent.run" => Some(BudgetClass::Action.limits().tokens),
        _ => None,
    }
}

/// The tokens a child operation reports. Unknown counts keep the reservation.
fn child_usage(name: &str, result: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(result).ok()?;
    match name {
        "agent.run" => value
            .get("usage")
            .and_then(|usage| usage.get("promptTokens"))
            .and_then(serde_json::Value::as_u64)
            .zip(
                value
                    .get("usage")
                    .and_then(|usage| usage.get("completionTokens"))
                    .and_then(serde_json::Value::as_u64),
            )
            .map(|(input, output)| input.saturating_add(output)),
        _ => value
            .get("prompt_tokens")
            .and_then(serde_json::Value::as_u64)
            .zip(
                value
                    .get("completion_tokens")
                    .and_then(serde_json::Value::as_u64),
            )
            .map(|(input, output)| input.saturating_add(output)),
    }
}

fn json_error(code: &str, message: &str) -> String {
    serde_json::json!({ "ok": false, "error": { "code": code, "message": message } }).to_string()
}

/// The turn's tool-call chunk, when the plan has one.
fn tool_chunk(turn: &mut Turn) -> Option<&mut crate::chunk::Chunk> {
    turn.chunks
        .iter_mut()
        .find(|chunk| matches!(chunk.directive.kind, crate::chunk::ChunkKind::ToolCall))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::RetryFailure;
    use crate::turn::TurnPlan;
    use mm_data::{SessionId, TurnId};

    #[test]
    fn the_process_context_fills_task_constraints_and_tools() {
        let plan = TurnPlan::simple(SessionId::new("s1"), TurnId::new("t1"), "request");
        let context = ProcessContext {
            task: "Do the thing".into(),
            constraints: vec!["stay brief".into(), "cite sources".into()],
            suggested_tools: Some(vec!["data.search".into()]),
            budget_hint: BudgetClass::Standard,
        };
        let mut variables = PromptVariables::new();
        apply_process_context(&mut variables, &plan, Some(&context));
        assert_eq!(variables.get("process_task"), Some("Do the thing"));
        assert_eq!(
            variables.get("process_constraints"),
            Some("- stay brief\n- cite sources")
        );
        assert_eq!(
            variables.get("process_suggested_tools"),
            Some("data.search")
        );
    }

    #[test]
    fn a_missing_brief_falls_back_to_the_request() {
        let plan = TurnPlan::simple(SessionId::new("s1"), TurnId::new("t1"), "request");
        let mut variables = PromptVariables::new();
        apply_process_context(&mut variables, &plan, None);
        assert_eq!(variables.get("process_task"), Some("request"));
        assert_eq!(variables.get("process_constraints"), Some("none"));
        assert_eq!(variables.get("process_suggested_tools"), Some("none"));
    }

    #[test]
    fn the_retry_context_names_only_failed_checks() {
        let retry = RetryContext {
            attempt: 2,
            max_attempts: 5,
            prior_reply_digest: "sha256:abc".into(),
            failures: vec![RetryFailure {
                label: "cites".into(),
                reasoning: "no source".into(),
                confidence: 0.4,
            }],
            error: "cites failed".into(),
            prior_effects: Vec::new(),
        };
        let text = render_retry_context(&retry);
        assert!(text.starts_with("RETRY_CONTEXT"));
        assert!(text.contains("attempt: 2/5"));
        assert!(text.contains("- cites: no source (confidence 0.40)"));
        assert!(text.contains("error: cites failed"));
        assert!(!text.contains("internal"));
    }

    #[test]
    fn budget_slots_describe_the_ledger() {
        let snapshot = BudgetSnapshot {
            class: "standard",
            rounds: 31,
            steps: 96,
            tokens: 119_000,
            wall_ms: 599_000,
            estimated: false,
        };
        let mut variables = PromptVariables::new();
        apply_budget(&mut variables, snapshot);
        assert_eq!(variables.get("budget_class"), Some("standard"));
        assert_eq!(variables.get("budget_rounds"), Some("31"));
        assert_eq!(variables.get("budget_tokens"), Some("119000"));
    }
}
