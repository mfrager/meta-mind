//! The three-pass runner: the deterministic shell around the model.
//!
//! Phase 2 implements the Interpret pass and the fast path. Process still runs
//! through [`TurnExecutor`]'s existing loop, and Verify is stubbed to approve
//! the completed process reply. The runner owns prompt rendering, pass scope,
//! budget accounting, transcript accumulation, and the control-tool protocol;
//! the model only chooses text and tool calls.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mm_data::{CleanLog, CleanRole, CleanTurn, SessionId, TurnId};
use mm_llm::{CacheMode, LlmClient, LlmRequest, Message, ProviderToolCall, Purpose};
use mm_prompt::{PromptInstance, PromptStore};
use serde::{Deserialize, Serialize};

use crate::budget::{BudgetLedger, BudgetSnapshot, TurnEnvelope};
use crate::config::ThreePassConfig;
use crate::error::{CmeError, Result};
use crate::executor::{DenyUnconfirmed, PromptContextProvider, ToolHandler};
use crate::pass_scope::PassScope;
use crate::process::{render_retry_context, ProcessPass, ProcessRequest};
use crate::prompt::PromptVariables;
use crate::session::SessionContext;
use crate::subagent::{parse_think_result, ThinkResult};
use crate::turn::{ToolCallEmission, Turn, TurnPlan};
use crate::verification::{
    BackendRegistry, LlmJudgeBackend, MutationJournal, ProcessContext, ProcessMutation,
    ProcessResult, VerificationBackend, VerificationResult, VerifyCheck, VerifyContext,
};

/// Maximum reply accepted from a fast-path `metamind.approve`.
pub const MAX_FAST_PATH_CHARS: usize = 8_000;
/// Maximum task restatement accepted from `metamind.verify`.
pub const MAX_TASK_CHARS: usize = 8_000;
/// Maximum constraints accepted from `metamind.verify`.
pub const MAX_CONSTRAINTS: usize = 8;
/// Maximum constraint length accepted from `metamind.verify`.
pub const MAX_CONSTRAINT_CHARS: usize = 300;

/// The pass being run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassKind {
    /// Classify the request, register checks, or fast-path a reply.
    Interpret,
    /// Do the work.
    Process,
    /// Check the work.
    Verify,
    /// A sub-agent think pass.
    Think,
}

impl PassKind {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interpret => "interpret",
            Self::Process => "process",
            Self::Verify => "verify",
            Self::Think => "think",
        }
    }

    /// The scope this pass runs under.
    pub fn scope(self) -> PassScope {
        match self {
            Self::Interpret => PassScope::Interpret,
            Self::Process => PassScope::Process,
            Self::Verify => PassScope::Verify,
            Self::Think => PassScope::Think,
        }
    }
}

/// The model's structured Interpret output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct InterpretResult {
    /// Registered checks; a zero-check action turn receives the implicit check.
    #[serde(default)]
    pub checks: Vec<VerifyCheck>,
    /// The Process brief carried by `metamind.verify`.
    #[serde(default)]
    pub process: Option<ProcessContext>,
    /// Whether the model chose the pure-conversation fast path.
    #[serde(default)]
    pub fast_path: bool,
}

/// Everything a pass may see. Fields absent here are absent from the rendered
/// prompt, which is how context isolation is enforced by construction.
#[derive(Debug, Clone)]
pub struct PassContext {
    /// Which pass is running.
    pub kind: PassKind,
    /// The pass's tool scope.
    pub scope: PassScope,
    /// User messages and approved final replies only.
    pub clean_log: CleanLog,
    /// The canonical `[STATE]` block.
    pub state_block: String,
    /// The operator's request.
    pub user_request: String,
    /// The selected model, when routing pinned one.
    pub model: Option<String>,
    /// Reviewed prompt slots.
    pub variables: PromptVariables,
    /// Interpret output, supplied only to Process and Verify.
    pub interpret: Option<InterpretResult>,
    /// Process output, supplied only to Verify.
    pub process: Option<crate::verification::ProcessResult>,
    /// Verification results, supplied only to a retry or the final status.
    pub verification: Vec<crate::verification::VerificationResult>,
    /// Retry context, supplied only to a retry.
    pub retry: Option<RetryContext>,
    /// The turn's total allowance.
    pub envelope: TurnEnvelope,
    /// Durable effects from the previous Process attempt.
    pub prior_effects: Vec<ProcessMutation>,
    /// Number of retries already used.
    pub retry_count: u32,
}

impl PassContext {
    /// A clean Interpret context. It carries no Process, Verify, or retry data.
    pub fn interpret(
        clean_log: CleanLog,
        state_block: impl Into<String>,
        user_request: impl Into<String>,
        variables: PromptVariables,
        model: Option<String>,
        envelope: TurnEnvelope,
    ) -> Self {
        Self {
            kind: PassKind::Interpret,
            scope: PassScope::Interpret,
            clean_log,
            state_block: state_block.into(),
            user_request: user_request.into(),
            model,
            variables,
            interpret: None,
            process: None,
            verification: Vec::new(),
            retry: None,
            envelope,
            prior_effects: Vec::new(),
            retry_count: 0,
        }
    }

    /// Override the context's scope. A recursive think reviews its own reply
    /// under the Think scope, so its reviewer cannot touch user state.
    #[must_use]
    pub fn with_scope(mut self, scope: PassScope) -> Self {
        self.scope = scope;
        self
    }

    /// A Verify context. It sees the Process reply, current Phase A results,
    /// the retry context, and the staged intents already proposed.
    #[allow(clippy::too_many_arguments)]
    pub fn verify(
        clean_log: CleanLog,
        state_block: impl Into<String>,
        user_request: impl Into<String>,
        variables: PromptVariables,
        model: Option<String>,
        interpret: InterpretResult,
        process: ProcessResult,
        verification: Vec<VerificationResult>,
        retry: Option<RetryContext>,
        prior_effects: Vec<ProcessMutation>,
        retry_count: u32,
        envelope: TurnEnvelope,
    ) -> Self {
        Self {
            kind: PassKind::Verify,
            scope: PassScope::Verify,
            clean_log,
            state_block: state_block.into(),
            user_request: user_request.into(),
            model,
            variables,
            interpret: Some(interpret),
            process: Some(process),
            verification,
            retry,
            envelope,
            prior_effects,
            retry_count,
        }
    }
}

/// How the pass ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassFinish {
    /// A pure-conversation turn was answered in one call.
    FastPath,
    /// The pass registered an action and handed off to Process.
    Action,
    /// The work was approved.
    Approved,
    /// The work was rejected.
    Rejected,
    /// The model did not follow the control-tool protocol.
    ProtocolError,
    /// A budget was exhausted.
    Exhausted,
    /// A transport or prompt failure ended the pass.
    Error,
}

impl PassFinish {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FastPath => "fast_path",
            Self::Action => "action",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::ProtocolError => "protocol_error",
            Self::Exhausted => "exhausted",
            Self::Error => "error",
        }
    }
}

/// A rejected attempt's context. It never contains an internal verification
/// prompt or a secret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RetryContext {
    /// One-based attempt number.
    pub attempt: u32,
    /// Maximum attempts.
    pub max_attempts: u32,
    /// Digest of the prior reply.
    pub prior_reply_digest: String,
    /// Failed checks with the model-authored reasoning.
    pub failures: Vec<RetryFailure>,
    /// The exact error string.
    pub error: String,
    /// Durable effects of the previous Process attempt, for reconciliation.
    #[serde(default)]
    pub prior_effects: Vec<crate::verification::ProcessMutation>,
}

/// One failed check in a retry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryFailure {
    /// Check label.
    pub label: String,
    /// The judge's reasoning.
    pub reasoning: String,
    /// Judge confidence.
    pub confidence: f64,
}

/// The result of one pass.
#[derive(Debug, Clone)]
pub struct PassOutcome {
    /// The pass's text. For a fast path this is the user-facing reply.
    pub text: String,
    /// Immutable prompt instances used, in call order.
    pub prompt_instance_ids: Vec<String>,
    /// Prompt hashes, parallel to `prompt_instance_ids`.
    pub prompt_hashes: Vec<String>,
    /// Tool calls the pass made.
    pub tool_emissions: Vec<ToolCallEmission>,
    /// The final ledger view.
    pub budget: BudgetSnapshot,
    /// How the pass ended.
    pub finished: PassFinish,
    /// Registered checks (Interpret) or checks evaluated (Verify).
    pub checks: Vec<VerifyCheck>,
    /// The Process brief from Interpret.
    pub process: Option<ProcessContext>,
    /// Durable effects of this Process attempt.
    pub mutations: Vec<ProcessMutation>,
    /// The turn envelope after this pass's usage was absorbed.
    pub envelope: TurnEnvelope,
    /// A protocol or transport error, when one ended the pass.
    pub error: Option<String>,
}

impl PassOutcome {
    /// Whether this was a pure-conversation fast path.
    pub fn is_fast_path(&self) -> bool {
        self.finished == PassFinish::FastPath
    }

    /// Whether the pass handed off to Process.
    pub fn is_action(&self) -> bool {
        self.finished == PassFinish::Action
    }
}

/// The result of one Verify pass.
#[derive(Debug, Clone)]
pub struct VerifyOutcome {
    /// Prompt instances used, in call order.
    pub prompt_instance_ids: Vec<String>,
    /// Prompt hashes, parallel to `prompt_instance_ids`.
    pub prompt_hashes: Vec<String>,
    /// Tool calls made by the reviewer.
    pub tool_emissions: Vec<ToolCallEmission>,
    /// The Verify ledger view.
    pub budget: BudgetSnapshot,
    /// How the pass ended.
    pub finished: PassFinish,
    /// Phase A results.
    pub verification: Vec<VerificationResult>,
    /// Writes proposed but not committed.
    pub journal: MutationJournal,
    /// The envelope after this pass's usage was absorbed.
    pub envelope: TurnEnvelope,
    /// A protocol, transport, or override error.
    pub error: Option<String>,
    /// The reviewer's error for the next Process attempt.
    pub rejection: Option<String>,
    /// Whether a failed check was overridden by the reviewer.
    pub overrode: bool,
}

impl VerifyOutcome {
    /// Whether the reply may be committed and presented.
    pub fn is_approved(&self) -> bool {
        self.finished == PassFinish::Approved
    }

    /// Whether the reviewer asked for another Process attempt.
    pub fn is_rejected(&self) -> bool {
        self.finished == PassFinish::Rejected
    }

    /// The pass record outcome name.
    pub fn outcome_name(&self) -> &'static str {
        if self.overrode {
            "approved_override"
        } else {
            self.finished.as_str()
        }
    }
}

/// One recursive think request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThinkPassRequest {
    /// The sub-agent session id (`think/<parent_turn>/<ulid>`).
    pub think_id: String,
    /// The question the parent turn asked.
    pub question: String,
    /// Conversation context supplied by the parent, if any.
    pub context: Option<String>,
    /// Focus entity IRIs.
    #[serde(default)]
    pub focus: Vec<String>,
    /// The model to route the thought through.
    pub model: Option<String>,
}

/// A thought whose work has been checked by its own Verify pass.
#[derive(Debug, Clone)]
pub struct ThinkRun {
    /// The parsed thought.
    pub result: ThinkResult,
    /// Phase A results from the Think scope.
    pub verification: Vec<VerificationResult>,
    /// Whether the Think reviewer approved the thought.
    pub approved: bool,
    /// Prompt instances used by the thought.
    pub prompt_instance_ids: Vec<String>,
}

/// The three-pass runner.
pub struct ThreePassRunner {
    client: Arc<dyn LlmClient>,
    prompts: Arc<PromptStore>,
    tools: Arc<dyn ToolHandler>,
    config: ThreePassConfig,
    backends: BackendRegistry,
    repair_text: String,
    verify_repair_text: String,
    context_provider: Option<Arc<dyn PromptContextProvider>>,
}

impl std::fmt::Debug for ThreePassRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreePassRunner")
            .field("config", &self.config.interpret.strategy)
            .field("backends", &self.backends.ids())
            .finish_non_exhaustive()
    }
}

impl ThreePassRunner {
    /// Build a runner and the Phase 1 judge registry from the same client.
    pub fn new(
        client: Arc<dyn LlmClient>,
        prompts: Arc<PromptStore>,
        tools: Arc<dyn ToolHandler>,
        config: ThreePassConfig,
    ) -> Result<Self> {
        if prompts.strategy(&config.process.strategy).is_none() {
            return Err(CmeError::Driver(format!(
                "prompt strategy `{}` is missing",
                config.process.strategy
            )));
        }
        let judge = config.verification.llm_judge();
        let mut backends = BackendRegistry::new();
        if judge.enabled {
            backends.register(Arc::new(
                LlmJudgeBackend::new(Some(Arc::clone(&client)), judge.model.clone())
                    .with_timeout(Duration::from_millis(config.verify.check_timeout_ms))
                    .with_sampling(judge.temperature, judge.max_output_tokens),
            ))?;
        }
        let repair_text = prompts
            .fragment("interpret_repair")
            .ok_or_else(|| {
                CmeError::Driver("prompt fragment `interpret_repair` is missing".into())
            })?
            .text
            .clone();
        let verify_repair_text = prompts
            .fragment("verify_repair")
            .ok_or_else(|| CmeError::Driver("prompt fragment `verify_repair` is missing".into()))?
            .text
            .clone();
        Ok(Self {
            client,
            prompts,
            tools,
            config,
            backends,
            repair_text,
            verify_repair_text,
            context_provider: None,
        })
    }

    /// Reload prompt variables and `[STATE]` before every Interpret round.
    #[must_use]
    pub fn with_context_provider(mut self, provider: Arc<dyn PromptContextProvider>) -> Self {
        self.context_provider = Some(provider);
        self
    }

    /// Register an additional verification backend.
    pub fn with_backend(mut self, backend: Arc<dyn VerificationBackend>) -> Result<Self> {
        self.backends.register(backend)?;
        Ok(self)
    }

    /// The configuration in force.
    pub fn config(&self) -> &ThreePassConfig {
        &self.config
    }

    /// The registered verification backends.
    pub fn backends(&self) -> &BackendRegistry {
        &self.backends
    }

    /// The token ceiling a retry reserves before starting. It covers the
    /// Process completion, the Verify completion, and every judge verdict.
    pub fn retry_reservation(&self) -> u64 {
        let judges = u64::from(
            self.config
                .verification
                .llm_judge()
                .max_output_tokens
                .max(1),
        )
        .saturating_mul(self.config.interpret.max_checks as u64);
        4_096u64.saturating_add(2_048).saturating_add(judges)
    }

    /// Replay a staged journal through the real handlers after approval.
    pub async fn commit(
        &self,
        session_id: &str,
        journal: &MutationJournal,
    ) -> Result<Vec<crate::verification::CommittedMutation>> {
        self.tools
            .commit_staged_for_session(session_id, journal)
            .await
    }

    /// Turn envelope for one turn, lowered by the configured token ceiling.
    pub fn envelope(&self) -> TurnEnvelope {
        let mut envelope = TurnEnvelope::standard(self.config.verify.max_retries);
        if self.config.envelope.max_tokens < envelope.limits().tokens {
            let mut limits = envelope.limits();
            limits.tokens = self.config.envelope.max_tokens;
            envelope = TurnEnvelope::new(limits, self.config.verify.max_retries);
        }
        envelope
    }

    /// Run the Interpret pass.
    ///
    /// The loop accepts exactly three outcomes: `metamind.approve` (fast path),
    /// `metamind.verify` (action), or a `conversation.*` call, which is executed
    /// immediately and fed back to the model. Text without a control call is a
    /// protocol violation; the second one ends the pass with
    /// `INTERPRET_PROTOCOL`.
    pub async fn run_interpret(&self, ctx: PassContext, session_id: &str) -> Result<PassOutcome> {
        if ctx.kind != PassKind::Interpret {
            return Err(CmeError::Driver(
                "run_interpret requires an Interpret context".into(),
            ));
        }
        let scope = ctx.scope;
        if scope != PassScope::Interpret {
            return Err(CmeError::Driver(
                "Interpret requires the Interpret tool scope".into(),
            ));
        }
        let mut envelope = ctx.envelope.clone();
        let mut ledger = envelope
            .ledger(self.config.interpret.budget_class)
            .map_err(CmeError::from)?;
        let mut prompt_ids = Vec::new();
        let mut prompt_hashes = Vec::new();
        let mut emissions = Vec::new();
        let mut history: Vec<Message> = Vec::new();
        let mut checks: Vec<VerifyCheck> = Vec::new();
        let mut process: Option<ProcessContext> = None;
        let mut repaired = false;

        for _round in 0..ledger.limits().rounds {
            ledger.reserve_round().map_err(CmeError::from)?;
            let (round_variables, round_state) = self.refresh_context(session_id, &ctx).await;
            let instance = match self.render_interpret(&ctx, &ledger, round_variables, round_state)
            {
                Ok(instance) => instance,
                Err(error) => {
                    return Ok(self.finish(
                        PassFinish::Error,
                        String::new(),
                        prompt_ids,
                        prompt_hashes,
                        emissions,
                        checks,
                        process,
                        &mut envelope,
                        &ledger,
                        Some(format!("PROMPT_UNAVAILABLE: {error}")),
                    ));
                }
            };
            prompt_ids.push(instance.instance_id.clone());
            prompt_hashes.push(instance.hash.clone());

            let mut messages = messages_from_instance(&instance);
            messages.extend(history.iter().cloned());
            let mut request = LlmRequest::new(Purpose::Interpret, messages)
                .with_cache(CacheMode::Use)
                .with_tools(self.tools.schemas_for(scope));
            request.model = ctx.model.clone();
            request.temperature = 0.2;
            request.max_tokens = 2_048;
            ledger
                .reserve_tokens(u64::from(request.max_tokens))
                .map_err(CmeError::from)?;
            let response = match self.client.complete(&request).await {
                Ok(response) => response,
                Err(error) => {
                    return Ok(self.finish(
                        PassFinish::Error,
                        String::new(),
                        prompt_ids,
                        prompt_hashes,
                        emissions,
                        checks,
                        process,
                        &mut envelope,
                        &ledger,
                        Some(error.to_string()),
                    ));
                }
            };
            let used = u64::from(response.usage.tokens_in)
                .saturating_add(u64::from(response.usage.tokens_out));
            ledger.settle_tokens(Some(used)).map_err(CmeError::from)?;

            if response.tool_calls.is_empty() {
                if repaired {
                    return Ok(self.finish(
                        PassFinish::ProtocolError,
                        String::new(),
                        prompt_ids,
                        prompt_hashes,
                        emissions,
                        checks,
                        process,
                        &mut envelope,
                        &ledger,
                        Some("INTERPRET_PROTOCOL".into()),
                    ));
                }
                repaired = true;
                if !response.text.is_empty() {
                    history.push(Message::assistant(response.text));
                }
                history.push(Message::user(self.repair_text.clone()));
                continue;
            }

            // A fast-path approval must be the only call in the round.
            if response
                .tool_calls
                .iter()
                .any(|call| call.name == "metamind.approve")
            {
                if response.tool_calls.len() != 1 {
                    return Ok(self.finish(
                        PassFinish::ProtocolError,
                        String::new(),
                        prompt_ids,
                        prompt_hashes,
                        emissions,
                        checks,
                        process,
                        &mut envelope,
                        &ledger,
                        Some("INTERPRET_PROTOCOL: approve must be the only call".into()),
                    ));
                }
                let call = &response.tool_calls[0];
                match self.parse_approval(call) {
                    Ok(reply) => {
                        emissions.push(emission(call, json_ok(&reply), true));
                        return Ok(self.finish(
                            PassFinish::FastPath,
                            reply,
                            prompt_ids,
                            prompt_hashes,
                            emissions,
                            checks,
                            process,
                            &mut envelope,
                            &ledger,
                            None,
                        ));
                    }
                    Err(error) => {
                        emissions.push(emission(call, json_error(&error), false));
                        if repaired {
                            return Ok(self.finish(
                                PassFinish::ProtocolError,
                                String::new(),
                                prompt_ids,
                                prompt_hashes,
                                emissions,
                                checks,
                                process,
                                &mut envelope,
                                &ledger,
                                Some(format!("INTERPRET_PROTOCOL: {error}")),
                            ));
                        }
                        repaired = true;
                        history.push(Message::assistant_calls(
                            response.text.clone(),
                            response.tool_calls.clone(),
                        ));
                        history.push(Message::tool(
                            call.id.clone(),
                            call.name.clone(),
                            json_error(&error),
                        ));
                        history.push(Message::user(self.repair_text.clone()));
                        continue;
                    }
                }
            }

            // Work: register checks and stop. The Process pass is next.
            if let Some(call) = response
                .tool_calls
                .iter()
                .find(|call| call.name == "metamind.verify")
            {
                if response.tool_calls.len() != 1 {
                    return Ok(self.finish(
                        PassFinish::ProtocolError,
                        String::new(),
                        prompt_ids,
                        prompt_hashes,
                        emissions,
                        checks,
                        process,
                        &mut envelope,
                        &ledger,
                        Some("INTERPRET_PROTOCOL: verify must be the only call".into()),
                    ));
                }
                match self.parse_verify(call, &ctx) {
                    Ok((mut registered, brief)) => {
                        if registered.len() > self.config.interpret.max_checks {
                            return Ok(self.finish(
                                PassFinish::ProtocolError,
                                String::new(),
                                prompt_ids,
                                prompt_hashes,
                                emissions,
                                checks,
                                process,
                                &mut envelope,
                                &ledger,
                                Some(format!(
                                    "INTERPRET_PROTOCOL: {} checks registered; the maximum is {}",
                                    registered.len(),
                                    self.config.interpret.max_checks
                                )),
                            ));
                        }
                        if let Err(error) = self.backends.validate_checks(&registered) {
                            return Ok(self.finish(
                                PassFinish::ProtocolError,
                                String::new(),
                                prompt_ids,
                                prompt_hashes,
                                emissions,
                                checks,
                                process,
                                &mut envelope,
                                &ledger,
                                Some(format!("INTERPRET_PROTOCOL: {error}")),
                            ));
                        }
                        if registered.is_empty() {
                            match self.default_check() {
                                Ok(check) => registered.push(check),
                                Err(error) => {
                                    return Ok(self.finish(
                                        PassFinish::Error,
                                        String::new(),
                                        prompt_ids,
                                        prompt_hashes,
                                        emissions,
                                        checks,
                                        process,
                                        &mut envelope,
                                        &ledger,
                                        Some(error.to_string()),
                                    ));
                                }
                            }
                        }
                        if let Err(error) = self.backends.validate_checks(&registered) {
                            return Ok(self.finish(
                                PassFinish::Error,
                                String::new(),
                                prompt_ids,
                                prompt_hashes,
                                emissions,
                                checks,
                                process,
                                &mut envelope,
                                &ledger,
                                Some(error.to_string()),
                            ));
                        }
                        checks = registered;
                        process = Some(brief);
                        emissions.push(emission(call, json_ok("checks registered"), true));
                        return Ok(self.finish(
                            PassFinish::Action,
                            String::new(),
                            prompt_ids,
                            prompt_hashes,
                            emissions,
                            checks,
                            process,
                            &mut envelope,
                            &ledger,
                            None,
                        ));
                    }
                    Err(error) => {
                        emissions.push(emission(call, json_error(&error), false));
                        if repaired {
                            return Ok(self.finish(
                                PassFinish::ProtocolError,
                                String::new(),
                                prompt_ids,
                                prompt_hashes,
                                emissions,
                                checks,
                                process,
                                &mut envelope,
                                &ledger,
                                Some(format!("INTERPRET_PROTOCOL: {error}")),
                            ));
                        }
                        repaired = true;
                        history.push(Message::assistant_calls(
                            response.text.clone(),
                            response.tool_calls.clone(),
                        ));
                        history.push(Message::tool(
                            call.id.clone(),
                            call.name.clone(),
                            json_error(&error),
                        ));
                        history.push(Message::user(self.repair_text.clone()));
                        continue;
                    }
                }
            }

            // Conversation-state calls are the only work Interpret may do
            // directly. Everything else is denied before it reaches a handler.
            let mut round_results = Vec::new();
            for call in &response.tool_calls {
                let action = action_of(&call.arguments);
                if !scope.allows(&call.name, action.as_deref()) {
                    let message = format!(
                        "{} is not available in the {} pass",
                        call.name,
                        scope.as_str()
                    );
                    let result = json_error_code("TOOL_NOT_AVAILABLE", &message);
                    emissions.push(emission(call, result.clone(), false));
                    round_results.push(Message::tool(call.id.clone(), call.name.clone(), result));
                    continue;
                }
                if !call.name.starts_with("conversation.") {
                    let message = format!("{} is not a conversation tool", call.name);
                    let result = json_error_code("TOOL_NOT_AVAILABLE", &message);
                    emissions.push(emission(call, result.clone(), false));
                    round_results.push(Message::tool(call.id.clone(), call.name.clone(), result));
                    continue;
                }
                let result = match self
                    .tools
                    .call_for_session(session_id, &call.name, &call.arguments)
                    .await
                {
                    Ok(result) => {
                        let ok = result_ok(&result);
                        emissions.push(emission(call, result.clone(), ok));
                        result
                    }
                    Err(error) => {
                        let result = format!("tool `{}` failed: {error}", call.name);
                        emissions.push(emission(call, result.clone(), false));
                        result
                    }
                };
                round_results.push(Message::tool(call.id.clone(), call.name.clone(), result));
            }
            history.push(Message::assistant_calls(
                response.text.clone(),
                response.tool_calls.clone(),
            ));
            history.extend(round_results);
        }

        // The quick ledger ran out of rounds.
        Ok(self.finish(
            PassFinish::Exhausted,
            String::new(),
            prompt_ids,
            prompt_hashes,
            emissions,
            checks,
            process,
            &mut envelope,
            &ledger,
            Some("BUDGET: interpret round exhausted".into()),
        ))
    }

    /// Run Phase A checks concurrently while preserving declaration order.
    ///
    /// A timeout, backend error, or missing backend becomes a failed result;
    /// it never aborts the other checks and never becomes a pass.
    pub async fn run_checks(
        &self,
        checks: &[VerifyCheck],
        process_output: &ProcessResult,
        context: &VerifyContext,
    ) -> Vec<VerificationResult> {
        let mut slots: Vec<Option<VerificationResult>> = vec![None; checks.len()];
        let mut handles = Vec::new();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(
            self.config.verify.max_parallel_checks.max(1),
        ));
        let timeout = Duration::from_millis(self.config.verify.check_timeout_ms);
        for (index, check) in checks.iter().enumerate() {
            let Some(backend) = self.backends.get(&check.backend) else {
                slots[index] = Some(VerificationResult::failed(
                    check,
                    format!("unknown verification backend `{}`", check.backend),
                    0,
                ));
                continue;
            };
            let check = check.clone();
            let process_output = process_output.clone();
            let context = context.clone();
            let semaphore = Arc::clone(&semaphore);
            handles.push((
                index,
                tokio::spawn(async move {
                    let _permit = semaphore.acquire_owned().await;
                    let started = Instant::now();
                    match tokio::time::timeout(
                        timeout,
                        backend.verify(&check, &process_output, &context),
                    )
                    .await
                    {
                        Ok(Ok(result)) => result,
                        Ok(Err(error)) => VerificationResult::failed(
                            &check,
                            error.to_string(),
                            started.elapsed().as_millis() as u64,
                        ),
                        Err(_) => VerificationResult::failed(
                            &check,
                            format!("verification timed out after {} ms", timeout.as_millis()),
                            started.elapsed().as_millis() as u64,
                        ),
                    }
                }),
            ));
        }
        for (index, handle) in handles {
            match handle.await {
                Ok(result) => slots[index] = Some(result),
                Err(error) => {
                    slots[index] = Some(VerificationResult::failed(
                        &checks[index],
                        format!("verification task failed: {error}"),
                        0,
                    ));
                }
            }
        }
        slots.into_iter().flatten().collect()
    }

    /// Run the Verify pass: Phase A, the review model, and a staged journal.
    pub async fn run_verify(&self, ctx: PassContext, session_id: &str) -> Result<VerifyOutcome> {
        if ctx.kind != PassKind::Verify {
            return Err(CmeError::Driver(
                "run_verify requires a Verify context".into(),
            ));
        }
        if !matches!(ctx.scope, PassScope::Verify | PassScope::Think) {
            return Err(CmeError::Driver(
                "Verify requires the Verify or Think tool scope".into(),
            ));
        }
        let mut envelope = ctx.envelope.clone();
        let mut ledger = envelope
            .ledger(self.config.verify.budget_class)
            .map_err(CmeError::from)?;
        let registered = ctx
            .interpret
            .as_ref()
            .map(|interpret| interpret.checks.clone())
            .unwrap_or_default();
        let process_output = ctx.process.clone().unwrap_or_default();
        let verify_context = VerifyContext {
            clean_log: ctx.clean_log.clone(),
            state_summary: ctx.state_block.clone(),
            user_request: ctx.user_request.clone(),
            interpret_checks: registered.clone(),
        };

        // A judge verdict is bounded; reserve its completion cap conservatively
        // so a turn cannot run an unbounded number of checks for free.
        let per_check = u64::from(
            self.config
                .verification
                .llm_judge()
                .max_output_tokens
                .max(1),
        );
        let estimate = per_check.saturating_mul(registered.len() as u64);
        if estimate > 0 {
            ledger.reserve_tokens(estimate).map_err(CmeError::from)?;
            ledger.settle_tokens(None).map_err(CmeError::from)?;
        }
        let verification = self
            .run_checks(&registered, &process_output, &verify_context)
            .await;
        let checks_failed = verification.iter().any(|result| !result.pass);
        let mut journal = MutationJournal::new(session_id);
        let mut prompt_ids = Vec::new();
        let mut prompt_hashes = Vec::new();
        let mut emissions = Vec::new();
        let mut history: Vec<Message> = Vec::new();
        let mut repaired = false;

        for _round in 0..ledger.limits().rounds {
            ledger.reserve_round().map_err(CmeError::from)?;
            let (round_variables, round_state) = self.refresh_context(session_id, &ctx).await;
            let instance = match self.render_verify(
                &ctx,
                &ledger,
                round_variables,
                round_state,
                &registered,
                &verification,
                &journal,
            ) {
                Ok(instance) => instance,
                Err(error) => {
                    return Ok(self.finish_verify(
                        PassFinish::Error,
                        prompt_ids,
                        prompt_hashes,
                        emissions,
                        verification,
                        journal,
                        &mut envelope,
                        &ledger,
                        Some(format!("PROMPT_UNAVAILABLE: {error}")),
                        None,
                        false,
                    ));
                }
            };
            prompt_ids.push(instance.instance_id.clone());
            prompt_hashes.push(instance.hash.clone());
            let mut messages = messages_from_instance(&instance);
            messages.extend(history.iter().cloned());
            let mut request = LlmRequest::new(Purpose::Critique, messages)
                .with_cache(CacheMode::Use)
                .with_tools(self.tools.schemas_for(ctx.scope));
            request.model = ctx.model.clone();
            request.temperature = 0.2;
            request.max_tokens = 2_048;
            ledger
                .reserve_tokens(u64::from(request.max_tokens))
                .map_err(CmeError::from)?;
            let response = match self.client.complete(&request).await {
                Ok(response) => response,
                Err(error) => {
                    return Ok(self.finish_verify(
                        PassFinish::Error,
                        prompt_ids,
                        prompt_hashes,
                        emissions,
                        verification,
                        journal,
                        &mut envelope,
                        &ledger,
                        Some(error.to_string()),
                        None,
                        false,
                    ));
                }
            };
            let used = u64::from(response.usage.tokens_in)
                .saturating_add(u64::from(response.usage.tokens_out));
            ledger.settle_tokens(Some(used)).map_err(CmeError::from)?;

            if response.tool_calls.is_empty() {
                if repaired {
                    return Ok(self.finish_verify(
                        PassFinish::ProtocolError,
                        prompt_ids,
                        prompt_hashes,
                        emissions,
                        verification,
                        journal,
                        &mut envelope,
                        &ledger,
                        Some("VERIFY_PROTOCOL: the reviewer did not approve".into()),
                        None,
                        false,
                    ));
                }
                repaired = true;
                if !response.text.is_empty() {
                    history.push(Message::assistant(response.text));
                }
                history.push(Message::user(self.verify_repair_text.clone()));
                continue;
            }

            if response
                .tool_calls
                .iter()
                .any(|call| call.name == "metamind.approve")
            {
                if response.tool_calls.len() != 1 {
                    return Ok(self.finish_verify(
                        PassFinish::ProtocolError,
                        prompt_ids,
                        prompt_hashes,
                        emissions,
                        verification,
                        journal,
                        &mut envelope,
                        &ledger,
                        Some("VERIFY_PROTOCOL: approve must be the only call".into()),
                        None,
                        false,
                    ));
                }
                let call = &response.tool_calls[0];
                match parse_verify_approval(call) {
                    Ok(Approval::Approve) => {
                        if checks_failed && !self.config.verify.allow_override {
                            let message = "VERIFICATION_OVERRIDE_DENIED: at least one check failed and override is disabled";
                            emissions.push(emission(
                                call,
                                json_error_code("VERIFICATION_OVERRIDE_DENIED", message),
                                false,
                            ));
                            if repaired {
                                return Ok(self.finish_verify(
                                    PassFinish::ProtocolError,
                                    prompt_ids,
                                    prompt_hashes,
                                    emissions,
                                    verification,
                                    journal,
                                    &mut envelope,
                                    &ledger,
                                    Some(message.into()),
                                    None,
                                    false,
                                ));
                            }
                            repaired = true;
                            history.push(Message::assistant_calls(
                                response.text.clone(),
                                response.tool_calls.clone(),
                            ));
                            history.push(Message::tool(
                                call.id.clone(),
                                call.name.clone(),
                                json_error_code("VERIFICATION_OVERRIDE_DENIED", message),
                            ));
                            history.push(Message::user(self.verify_repair_text.clone()));
                            continue;
                        }
                        emissions.push(emission(call, json_ok("approved"), true));
                        return Ok(self.finish_verify(
                            PassFinish::Approved,
                            prompt_ids,
                            prompt_hashes,
                            emissions,
                            verification,
                            journal,
                            &mut envelope,
                            &ledger,
                            None,
                            None,
                            checks_failed,
                        ));
                    }
                    Ok(Approval::Reject(error)) => {
                        emissions.push(emission(
                            call,
                            json_ok("rejected; another Process attempt is scheduled"),
                            true,
                        ));
                        return Ok(self.finish_verify(
                            PassFinish::Rejected,
                            prompt_ids,
                            prompt_hashes,
                            emissions,
                            verification,
                            journal,
                            &mut envelope,
                            &ledger,
                            None,
                            Some(error),
                            false,
                        ));
                    }
                    Err(error) => {
                        emissions.push(emission(call, json_error(&error), false));
                        if repaired {
                            return Ok(self.finish_verify(
                                PassFinish::ProtocolError,
                                prompt_ids,
                                prompt_hashes,
                                emissions,
                                verification,
                                journal,
                                &mut envelope,
                                &ledger,
                                Some(format!("VERIFY_PROTOCOL: {error}")),
                                None,
                                false,
                            ));
                        }
                        repaired = true;
                        history.push(Message::assistant_calls(
                            response.text.clone(),
                            response.tool_calls.clone(),
                        ));
                        history.push(Message::tool(
                            call.id.clone(),
                            call.name.clone(),
                            json_error(&error),
                        ));
                        history.push(Message::user(self.verify_repair_text.clone()));
                        continue;
                    }
                }
            }

            let mut round_results = Vec::new();
            for call in &response.tool_calls {
                let action = action_of(&call.arguments);
                if !ctx.scope.allows(&call.name, action.as_deref()) {
                    let message = format!(
                        "{} is not available in the {} pass",
                        call.name,
                        ctx.scope.as_str()
                    );
                    let result = json_error_code("TOOL_NOT_AVAILABLE", &message);
                    emissions.push(emission(call, result.clone(), false));
                    round_results.push(Message::tool(call.id.clone(), call.name.clone(), result));
                    continue;
                }
                let result = match self
                    .tools
                    .call_staged_for_session(session_id, &call.name, &call.arguments, &mut journal)
                    .await
                {
                    Ok(result) => {
                        let ok = result_ok(&result);
                        emissions.push(emission(call, result.clone(), ok));
                        result
                    }
                    Err(error) => {
                        let result = format!("tool `{}` failed: {error}", call.name);
                        emissions.push(emission(call, result.clone(), false));
                        result
                    }
                };
                round_results.push(Message::tool(call.id.clone(), call.name.clone(), result));
            }
            history.push(Message::assistant_calls(
                response.text.clone(),
                response.tool_calls.clone(),
            ));
            history.extend(round_results);
        }

        Ok(self.finish_verify(
            PassFinish::Exhausted,
            prompt_ids,
            prompt_hashes,
            emissions,
            verification,
            journal,
            &mut envelope,
            &ledger,
            Some("BUDGET: verify round exhausted".into()),
            None,
            false,
        ))
    }

    /// Run a thought through the same verified execution path.
    ///
    /// Think skips a separate Interpret model call: a thought's task is the
    /// question itself, so the runner injects the step-style think check. The
    /// work runs under the Think scope (read-only data, `agent.run`, no writes)
    /// and its own session, and the reviewer runs under the same scope, so a
    /// thought can never mutate user state. The journal is never committed.
    pub async fn run_think(&self, request: ThinkPassRequest) -> Result<ThinkRun> {
        let started = Instant::now();
        let Some(model) = request.model.as_deref() else {
            return Err(CmeError::Driver(
                "think requires a routed model to run its three-pass cycle".into(),
            ));
        };
        let check = self.think_check()?;
        let mut plan = TurnPlan::simple(
            SessionId::new(request.think_id.clone()),
            TurnId::new(request.think_id.clone()),
            request.question.clone(),
        );
        plan.tool_policy = crate::planner::ToolPolicy::Allowed;
        let thought_question = if request.focus.is_empty() {
            request.question.clone()
        } else {
            format!(
                "{}\n\nFocus entities: {}",
                request.question,
                request.focus.join(", ")
            )
        };
        let mut variables = PromptVariables::for_turn(&plan, &SessionContext::default());
        variables.set("thought_question", thought_question);
        let clean_log = match request
            .context
            .as_deref()
            .map(str::trim)
            .filter(|context| !context.is_empty())
        {
            Some(context) => CleanLog {
                turns: vec![CleanTurn {
                    role: CleanRole::User,
                    text: context.to_owned(),
                    turn_id: request.think_id.clone(),
                }],
                truncated: false,
            },
            None => CleanLog::default(),
        };
        let mut turn = Turn::new(plan.clone())?;
        let pass = ProcessPass::new(
            Arc::clone(&self.client),
            Some(Arc::clone(&self.prompts)),
            Arc::clone(&self.tools),
            Arc::new(DenyUnconfirmed),
            // A thought has no conversation bundle to reload; a provider would
            // replace its question with the parent session's variables.
            None,
            self.config.think.budget_class,
        );
        let run = pass
            .run(
                &mut turn,
                &plan,
                model,
                variables.clone(),
                ProcessRequest {
                    application: self.config.think.strategy.clone(),
                    scope: Some(PassScope::Think),
                    process: None,
                    retry: None,
                    clean_log: clean_log.clone(),
                    variant: None,
                    require_reply: true,
                    child_budget: true,
                },
            )
            .await?;
        let state_block = variables.get("state_block").unwrap_or_default().to_owned();
        let process_result = ProcessResult {
            reply_text: run.reply.clone(),
            tool_emissions: run.tool_emissions.clone(),
        };
        let interpret = InterpretResult {
            checks: vec![check],
            process: None,
            fast_path: false,
        };
        let ctx = PassContext::verify(
            clean_log,
            state_block,
            request.question.clone(),
            variables,
            Some(model.to_owned()),
            interpret,
            process_result,
            Vec::new(),
            None,
            Vec::new(),
            0,
            self.envelope(),
        )
        .with_scope(PassScope::Think);
        let verify = self.run_verify(ctx, &request.think_id).await?;
        let approved = verify.is_approved();
        let failures = verify
            .verification
            .iter()
            .filter(|result| !result.pass)
            .map(|result| {
                format!(
                    "{} — {}",
                    result.check_label,
                    result.error.as_deref().unwrap_or(&result.reasoning)
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        if !approved {
            let reason = if failures.is_empty() {
                verify
                    .rejection
                    .clone()
                    .or_else(|| verify.error.clone())
                    .unwrap_or_else(|| "the thought was not approved".into())
            } else {
                failures
            };
            return Err(CmeError::Driver(format!(
                "think verification failed: {reason}"
            )));
        }
        let mut result = parse_think_result(&run.reply);
        result.think_id = request.think_id;
        result.prompt_instance_id = turn.prompt_instance_id.clone();
        result.prompt_hash = turn.prompt_hash.clone();
        result.prompt_tokens = run.tokens_in;
        result.completion_tokens = run.tokens_out;
        result.elapsed_ms = started.elapsed().as_millis() as u64;
        let mut prompt_instance_ids = verify.prompt_instance_ids.clone();
        if let Some(instance) = &turn.prompt_instance_id {
            prompt_instance_ids.insert(0, instance.clone());
        }
        Ok(ThinkRun {
            result,
            verification: verify.verification,
            approved,
            prompt_instance_ids,
        })
    }

    fn think_check(&self) -> Result<VerifyCheck> {
        let prompt = self
            .prompts
            .fragment("think_check")
            .ok_or_else(|| CmeError::Driver("prompt fragment `think_check` is missing".into()))?
            .text
            .clone();
        Ok(VerifyCheck {
            label: "answer".into(),
            description: Some(
                "The thought answers the question with evidence and honest confidence.".into(),
            ),
            details: Some("A thought that omits evidence or overstates certainty fails.".into()),
            backend: "llm_judge".into(),
            backend_params: serde_json::json!({
                "evaluation_prompt": prompt,
                "expected_output_schema": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["pass", "reasoning", "confidence"],
                    "properties": {
                        "pass": { "type": "boolean" },
                        "reasoning": { "type": "string", "maxLength": 2000 },
                        "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
                    }
                }
            }),
            confidence_threshold: 0.5,
        })
    }

    fn render_verify(
        &self,
        ctx: &PassContext,
        ledger: &BudgetLedger,
        mut variables: PromptVariables,
        state_block: String,
        checks: &[VerifyCheck],
        verification: &[VerificationResult],
        journal: &MutationJournal,
    ) -> std::result::Result<PromptInstance, String> {
        variables.set("state_block", state_block.clone());
        variables.set("user_input", ctx.user_request.clone());
        variables.set("transcript_compact", ctx.clean_log.render());
        variables.set("tools_summary", ctx.scope.visible_tools().join(", "));
        let snapshot = ledger.snapshot();
        variables.set("budget_class", snapshot.class);
        variables.set("budget_rounds", snapshot.rounds.to_string());
        variables.set("budget_tokens", snapshot.tokens.to_string());
        variables.set("budget_wall_ms", snapshot.wall_ms.to_string());
        variables.set(
            "checks_registered",
            if checks.is_empty() {
                "none".to_owned()
            } else {
                checks
                    .iter()
                    .map(|check| {
                        format!(
                            "- {} ({}) threshold {:.2}",
                            check.label, check.backend, check.confidence_threshold
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        );
        variables.set(
            "verification_results",
            render_verification_results(verification),
        );
        variables.set(
            "retry_context",
            ctx.retry
                .as_ref()
                .map(render_retry_context)
                .unwrap_or_else(|| "none".to_owned()),
        );
        variables.set("retry_count", ctx.retry_count.to_string());
        variables.set(
            "process_reply",
            ctx.process
                .as_ref()
                .map(|process| process.reply_text.clone())
                .unwrap_or_default(),
        );
        variables.set(
            "staged_intents",
            if journal.is_empty() {
                "none".to_owned()
            } else {
                serde_json::to_string_pretty(&journal.prompt_view())
                    .unwrap_or_else(|_| "unavailable".into())
            },
        );
        let mut values = variables.into_map();
        if let Some(strategy) = self.prompts.strategy(&self.config.verify.strategy) {
            for slot in &strategy.slots {
                values.entry(slot.clone()).or_default();
            }
        }
        let transcript = ctx
            .clean_log
            .turns
            .iter()
            .map(|turn| mm_prompt::PromptMessage {
                role: match turn.role {
                    mm_data::CleanRole::User => "user".into(),
                    mm_data::CleanRole::Assistant => "assistant".into(),
                },
                content: turn.text.clone(),
            })
            .collect();
        let request = mm_prompt::PromptRequest {
            application: self.config.verify.strategy.clone(),
            variant: None,
            variables: values,
            state_block: Some(state_block),
            transcript,
            user_input: Some(ctx.user_request.clone()),
        };
        self.prompts
            .render_request(&request)
            .map_err(|error| error.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_verify(
        &self,
        finished: PassFinish,
        prompt_instance_ids: Vec<String>,
        prompt_hashes: Vec<String>,
        tool_emissions: Vec<ToolCallEmission>,
        verification: Vec<VerificationResult>,
        journal: MutationJournal,
        envelope: &mut TurnEnvelope,
        ledger: &BudgetLedger,
        error: Option<String>,
        rejection: Option<String>,
        overrode: bool,
    ) -> VerifyOutcome {
        let budget = ledger.snapshot();
        let mut error = error;
        if let Err(absorb) = envelope.absorb(ledger) {
            error = Some(absorb.to_string());
        }
        VerifyOutcome {
            prompt_instance_ids,
            prompt_hashes,
            tool_emissions,
            budget,
            finished,
            verification,
            journal,
            envelope: envelope.clone(),
            error,
            rejection,
            overrode,
        }
    }

    async fn refresh_context(
        &self,
        session_id: &str,
        ctx: &PassContext,
    ) -> (PromptVariables, String) {
        let Some(provider) = &self.context_provider else {
            return (ctx.variables.clone(), ctx.state_block.clone());
        };
        match provider.prompt_variables(session_id).await {
            Ok(variables) => {
                let state = variables
                    .get("state_block")
                    .map(str::to_owned)
                    .unwrap_or_else(|| ctx.state_block.clone());
                (variables, state)
            }
            Err(error) => {
                let mut variables = ctx.variables.clone();
                let block = crate::state_block::StateBlock::unavailable(error);
                variables.set("state_block", block.text.clone());
                variables.set("state_json", "");
                (variables, block.text)
            }
        }
    }

    fn render_interpret(
        &self,
        ctx: &PassContext,
        ledger: &BudgetLedger,
        mut variables: PromptVariables,
        state_block: String,
    ) -> std::result::Result<PromptInstance, String> {
        variables.set("state_block", state_block.clone());
        variables.set("user_input", ctx.user_request.clone());
        variables.set("transcript_compact", ctx.clean_log.render());
        variables.set("tools_summary", ctx.scope.visible_tools().join(", "));
        let snapshot = ledger.snapshot();
        variables.set("budget_class", snapshot.class);
        variables.set("budget_rounds", snapshot.rounds.to_string());
        variables.set("budget_tokens", snapshot.tokens.to_string());
        variables.set("budget_wall_ms", snapshot.wall_ms.to_string());
        let transcript = ctx
            .clean_log
            .turns
            .iter()
            .map(|turn| mm_prompt::PromptMessage {
                role: match turn.role {
                    mm_data::CleanRole::User => "user".into(),
                    mm_data::CleanRole::Assistant => "assistant".into(),
                },
                content: turn.text.clone(),
            })
            .collect();
        let request = mm_prompt::PromptRequest {
            application: self.config.interpret.strategy.clone(),
            variant: None,
            variables: variables.as_map().clone(),
            state_block: Some(state_block),
            transcript,
            user_input: Some(ctx.user_request.clone()),
        };
        self.prompts
            .render_request(&request)
            .map_err(|error| error.to_string())
    }

    fn parse_approval(&self, call: &ProviderToolCall) -> std::result::Result<String, String> {
        let value: serde_json::Value = serde_json::from_str(&call.arguments)
            .map_err(|error| format!("approve arguments are not JSON: {error}"))?;
        let success = value
            .get("success")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| "approve requires a boolean `success`".to_owned())?;
        if !success {
            return Err("approve(success=false) cannot end an Interpret pass".into());
        }
        let reply = value
            .get("reply")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|reply| !reply.is_empty())
            .ok_or_else(|| "approve(true) requires a non-empty `reply`".to_owned())?;
        if reply.chars().count() > MAX_FAST_PATH_CHARS {
            return Err(format!(
                "approve reply is longer than {MAX_FAST_PATH_CHARS} characters"
            ));
        }
        Ok(reply.to_owned())
    }

    fn parse_verify(
        &self,
        call: &ProviderToolCall,
        ctx: &PassContext,
    ) -> std::result::Result<(Vec<VerifyCheck>, ProcessContext), String> {
        let value: serde_json::Value = serde_json::from_str(&call.arguments)
            .map_err(|error| format!("verify arguments are not JSON: {error}"))?;
        let checks = value
            .get("checks")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .cloned()
                    .map(|item| {
                        serde_json::from_value::<VerifyCheck>(item)
                            .map_err(|error| format!("invalid check: {error}"))
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let brief = value.get("process");
        let task = brief
            .and_then(|process| process.get("task"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|task| !task.is_empty())
            .map(|task| task.chars().take(MAX_TASK_CHARS).collect())
            .unwrap_or_else(|| ctx.user_request.chars().take(MAX_TASK_CHARS).collect());
        let constraints = brief
            .and_then(|process| process.get("constraints"))
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .filter(|item| !item.trim().is_empty())
                    .take(MAX_CONSTRAINTS)
                    .map(|item| item.chars().take(MAX_CONSTRAINT_CHARS).collect())
                    .collect()
            })
            .unwrap_or_default();
        let suggested_tools = brief
            .and_then(|process| process.get("suggested_tools"))
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .filter(|name| PassScope::Process.allows(name, None))
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|tools: &Vec<String>| !tools.is_empty());
        let process = ProcessContext {
            task,
            constraints,
            suggested_tools,
            budget_hint: self.config.process.budget_class,
        };
        Ok((checks, process))
    }

    fn default_check(&self) -> Result<VerifyCheck> {
        let prompt = self
            .prompts
            .fragment("default_action_check")
            .ok_or_else(|| {
                CmeError::Driver("prompt fragment `default_action_check` is missing".into())
            })?
            .text
            .clone();
        Ok(VerifyCheck {
            label: "action".into(),
            description: Some("The reply addresses the request with the work it claims.".into()),
            details: Some("The request needed work; check the reply against the task.".into()),
            backend: "llm_judge".into(),
            backend_params: serde_json::json!({
                "evaluation_prompt": prompt,
                "expected_output_schema": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["pass", "reasoning", "confidence"],
                    "properties": {
                        "pass": { "type": "boolean" },
                        "reasoning": { "type": "string", "maxLength": 2000 },
                        "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
                    }
                }
            }),
            confidence_threshold: 0.5,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        finished: PassFinish,
        text: String,
        prompt_instance_ids: Vec<String>,
        prompt_hashes: Vec<String>,
        tool_emissions: Vec<ToolCallEmission>,
        checks: Vec<VerifyCheck>,
        process: Option<ProcessContext>,
        envelope: &mut TurnEnvelope,
        ledger: &BudgetLedger,
        error: Option<String>,
    ) -> PassOutcome {
        let budget = ledger.snapshot();
        let mut error = error;
        if let Err(absorb) = envelope.absorb(ledger) {
            error = Some(absorb.to_string());
        }
        PassOutcome {
            text,
            prompt_instance_ids,
            prompt_hashes,
            tool_emissions,
            budget,
            finished,
            checks,
            process,
            mutations: Vec::new(),
            envelope: envelope.clone(),
            error,
        }
    }
}

enum Approval {
    Approve,
    Reject(String),
}

fn parse_verify_approval(call: &ProviderToolCall) -> std::result::Result<Approval, String> {
    let value: serde_json::Value = serde_json::from_str(&call.arguments)
        .map_err(|error| format!("approve arguments are not JSON: {error}"))?;
    let success = value
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| "approve requires a boolean `success`".to_owned())?;
    if success {
        return Ok(Approval::Approve);
    }
    let error = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|error| !error.is_empty())
        .ok_or_else(|| "approve(false) requires a non-empty `error`".to_owned())?;
    if error.chars().count() > 2_000 {
        return Err("approve(false) error is longer than 2000 characters".into());
    }
    Ok(Approval::Reject(error.to_owned()))
}

fn render_verification_results(results: &[VerificationResult]) -> String {
    if results.is_empty() {
        return "none".into();
    }
    results
        .iter()
        .map(|result| {
            let status = if result.pass { "pass" } else { "fail" };
            match &result.error {
                Some(error) => format!(
                    "- {}: {status} ({:.2}) error: {}",
                    result.check_label,
                    result.confidence,
                    truncate(error, 1_000)
                ),
                None => format!(
                    "- {}: {status} ({:.2}) {}",
                    result.check_label,
                    result.confidence,
                    truncate(&result.reasoning, 1_000)
                ),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    text.chars().take(max).collect::<String>() + "…"
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

fn result_ok(result: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(result)
        .ok()
        .and_then(|value| value.get("ok").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

fn emission(call: &ProviderToolCall, result: String, ok: bool) -> ToolCallEmission {
    ToolCallEmission {
        call_id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
        result,
        ok,
    }
}

fn json_ok(message: &str) -> String {
    serde_json::json!({ "ok": true, "message": message }).to_string()
}

fn json_error(message: &str) -> String {
    json_error_code("INVALID_ARGUMENT", message)
}

fn json_error_code(code: &str, message: &str) -> String {
    serde_json::json!({ "ok": false, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_kinds_map_to_their_scopes() {
        assert_eq!(PassKind::Interpret.scope(), PassScope::Interpret);
        assert_eq!(PassKind::Process.scope(), PassScope::Process);
        assert_eq!(PassKind::Interpret.as_str(), "interpret");
    }

    #[test]
    fn finish_names_are_stable() {
        assert_eq!(PassFinish::FastPath.as_str(), "fast_path");
        assert_eq!(PassFinish::ProtocolError.as_str(), "protocol_error");
    }
}
