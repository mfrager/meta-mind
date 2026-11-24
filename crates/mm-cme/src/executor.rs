//! The turn executor: drives one Turn from planning through completion.
//!
//! The executor owns the ordered work a turn needs — plan, route, pre-warm, call,
//! stream into chunks, run tools, finalize — while the state machines own the
//! legality of each step. With no LLM client configured it runs the deterministic
//! `Echo:` path used by tests and local sessions; with a client it uses the
//! provider-neutral `mm-llm` substrate.

use std::sync::Arc;

use async_trait::async_trait;
use mm_data::CleanLog;
use mm_llm::{LlmClient, ToolSchema};
use mm_prompt::PromptStore;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::budget::BudgetClass;
use crate::chunk::ChunkKind;
use crate::error::{CmeError, Result};
use crate::observability::{ObserverHandle, SmObserver};
use crate::observer::PassObserver;
use crate::pass_scope::PassScope;
use crate::planner::{ResponseModality, TurnContext, TurnPlanner};
use crate::process::{ProcessPass, ProcessRequest};
use crate::prompt::PromptVariables;
use crate::routing::{ConversationRouter, ConversationRoutingRequest};
use crate::runner::{
    InterpretResult, PassContext, RetryContext, RetryFailure, ThreePassRunner, VerifyOutcome,
};
use crate::session::SessionContext;
use crate::turn::{Turn, TurnEvent, TurnOutcome, TurnState};
use crate::verification::{
    CommittedMutation, MutationJournal, ProcessMutation, ProcessResult, VerificationResult,
};
use crate::voice::VoiceTurnConfig;

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

    /// The tools one pass may see. The default filters [`schemas`](Self::schemas)
    /// through the scope; handlers with a reviewed catalog override this so the
    /// control tools are included where the scope allows them.
    fn schemas_for(&self, scope: PassScope) -> Vec<ToolSchema> {
        self.schemas()
            .into_iter()
            .filter(|schema| scope.allows(&schema.name, None))
            .collect()
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

    /// Run a tool from the Verify pass. A handler that supports staged writes
    /// validates mutating calls and appends them to `journal` instead of running
    /// them. The default is immediate because a handler without staging has no
    /// durable write surface to protect.
    async fn call_staged_for_session(
        &self,
        session_id: &str,
        name: &str,
        arguments: &str,
        _journal: &mut MutationJournal,
    ) -> Result<String> {
        self.call_for_session(session_id, name, arguments).await
    }

    /// Replay a journal after approval. Handlers without staging cannot commit.
    async fn commit_staged_for_session(
        &self,
        _session_id: &str,
        _journal: &MutationJournal,
    ) -> Result<Vec<CommittedMutation>> {
        Err(CmeError::Tool(
            "this tool handler does not support staged writes".into(),
        ))
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

/// Supplies the clean log for a session. The executor rebuilds it from the
/// agentstream before Interpret, so a resumed session sees the same log it had
/// in memory.
#[async_trait]
pub trait CleanLogProvider: Send + Sync {
    /// The session's user messages and approved final replies.
    async fn clean_log(&self, session_id: &str) -> std::result::Result<CleanLog, String>;
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
    budgeted: bool,
    pass_observer: Option<Arc<dyn PassObserver>>,
    three_pass: Option<ThreePassRunner>,
    clean_log_provider: Option<Arc<dyn CleanLogProvider>>,
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
            budgeted: true,
            pass_observer: None,
            three_pass: None,
            clean_log_provider: None,
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
            budgeted: true,
            pass_observer: None,
            three_pass: None,
            clean_log_provider: None,
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

    /// Enable or disable per-turn resource budgeting.
    pub fn with_budgeting(mut self, budgeted: bool) -> Self {
        self.budgeted = budgeted;
        self
    }

    /// Attach the Phase 2 three-pass runner. When absent, the executor uses the
    /// legacy single-loop path unchanged.
    pub fn with_three_pass(mut self, runner: ThreePassRunner) -> Self {
        self.three_pass = Some(runner);
        self
    }

    /// Attach a display observer to the pass runner. It covers both the
    /// three-pass path and the legacy `ProcessPass` tool loop.
    pub fn with_pass_observer(mut self, observer: Arc<dyn PassObserver>) -> Self {
        self.pass_observer = Some(Arc::clone(&observer));
        if let Some(runner) = self.three_pass.take() {
            self.three_pass = Some(runner.with_observer(observer));
        }
        self
    }

    /// Attach the provider that rebuilds the clean log before Interpret.
    pub fn with_clean_log_provider(mut self, provider: Arc<dyn CleanLogProvider>) -> Self {
        self.clean_log_provider = Some(provider);
        self
    }

    /// Whether the three-pass path is attached.
    pub fn three_pass(&self) -> bool {
        self.three_pass.is_some()
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
        // The executor outlives one turn inside a session driver, so a per-turn
        // display observer must not stay attached after this turn finishes.
        let pass_observer = self.pass_observer.take();
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
                if self.three_pass.is_some() {
                    match self
                        .execute_three_pass(turn, &plan, session, &model, Arc::clone(client))
                        .await?
                    {
                        ThreePassFlow::FastPath(reply) => {
                            if !reply.is_empty() {
                                self.stream_text(turn, &reply)?;
                            }
                            self.flush_structured(turn, &reply)?;
                            self.flush_audio(turn)?;
                            return turn.finalize();
                        }
                        ThreePassFlow::Approved(reply) => reply,
                        ThreePassFlow::Failed(detail) => return self.fail(turn, detail),
                    }
                } else {
                    let variables = self
                        .prompt_variables
                        .clone()
                        .unwrap_or_else(|| PromptVariables::for_turn(&plan, session));
                    let mut pass = ProcessPass::new(
                        Arc::clone(client),
                        self.prompts.clone(),
                        Arc::clone(&self.tools),
                        Arc::clone(&self.confirm),
                        self.prompt_provider.clone(),
                        self.budget_class,
                    )
                    .with_budgeting(self.budgeted);
                    if let Some(observer) = &pass_observer {
                        pass = pass.with_observer(Arc::clone(observer), 1);
                    }
                    let run = match pass
                        .run(
                            turn,
                            &plan,
                            &model,
                            variables,
                            ProcessRequest::legacy("cm.turn", self.prompt_variant.clone()),
                        )
                        .await
                    {
                        Ok(run) => run,
                        Err(error) => return self.fail(turn, error.to_string()),
                    };
                    run.reply
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

    /// Run Interpret → Process → Phase A → Verify, retrying Process and Verify
    /// until the reviewer approves or the retry allowance is exhausted.
    async fn execute_three_pass(
        &self,
        turn: &mut Turn,
        plan: &crate::turn::TurnPlan,
        session: &SessionContext,
        model: &str,
        client: Arc<dyn LlmClient>,
    ) -> Result<ThreePassFlow> {
        let Some(runner) = self.three_pass.as_ref() else {
            return Ok(ThreePassFlow::Failed(
                "three-pass runner is not configured".into(),
            ));
        };
        let clean_log = self.load_clean_log(plan.session_id.as_ref()).await;
        let variables = self
            .prompt_variables
            .clone()
            .unwrap_or_else(|| PromptVariables::for_turn(plan, session));
        let state_block = variables.get("state_block").unwrap_or_default().to_owned();
        let mut interpret_variables = variables.clone();
        interpret_variables.set("user_input", plan.user_query.clone());
        let ctx = PassContext::interpret(
            clean_log.clone(),
            state_block.clone(),
            plan.user_query.clone(),
            interpret_variables,
            Some(model.to_owned()),
            runner.envelope(),
        );
        let pass = match runner.run_interpret(ctx, plan.session_id.as_ref()).await {
            Ok(pass) => pass,
            Err(error) => return Ok(ThreePassFlow::Failed(error.to_string())),
        };
        if let (Some(instance), Some(hash)) =
            (pass.prompt_instance_ids.last(), pass.prompt_hashes.last())
        {
            turn.set_prompt(instance.clone(), hash.clone(), None);
        }
        turn.record_pass(crate::turn::PassRecord {
            kind: crate::runner::PassKind::Interpret,
            attempt: 1,
            prompt_hash: pass.prompt_hashes.last().cloned(),
            budget: pass.budget.into(),
            outcome: pass.finished.as_str().to_owned(),
            verification: Vec::new(),
            staged: 0,
            committed: 0,
            checks: pass
                .checks
                .iter()
                .map(|check| check.label.clone())
                .collect(),
        });
        // Control calls are protocol, not user-facing work; the pass frames
        // carry their outcome without rendering a tool card.
        let visible: Vec<_> = pass
            .tool_emissions
            .iter()
            .filter(|emission| {
                !matches!(
                    emission.name.as_str(),
                    "metamind.verify" | "metamind.approve"
                )
            })
            .cloned()
            .collect();
        turn.tool_calls.extend(visible.iter().cloned());
        turn.tool_results
            .extend(visible.iter().map(|emission| emission.result.clone()));
        if let Some(error) = &pass.error {
            return Ok(ThreePassFlow::Failed(error.clone()));
        }
        if pass.is_fast_path() {
            return Ok(ThreePassFlow::FastPath(pass.text.clone()));
        }
        if !pass.is_action() {
            return Ok(ThreePassFlow::Failed(
                pass.error
                    .clone()
                    .unwrap_or_else(|| "interpret pass failed".into()),
            ));
        }

        let process_class = runner.config().process.budget_class;
        let observer = runner.observer();
        let checks = pass.checks.clone();
        let mut envelope = pass.envelope.clone();
        let mut retry: Option<RetryContext> = None;
        let mut prior_effects: Vec<ProcessMutation> = Vec::new();
        let mut verification: Vec<VerificationResult> = Vec::new();
        let mut staged_writes = Vec::new();
        let mut committed_writes = Vec::new();
        let mut discarded_writes = 0u32;
        let max_retries = runner.config().verify.max_retries;
        let mut exhaustion: Option<String> = None;

        for attempt in 0..=max_retries {
            let attempt_number = attempt + 1;
            let mut reservation = 0u64;
            if attempt > 0 {
                reservation = runner.retry_reservation();
                if let Err(error) = envelope.reserve_retry(reservation) {
                    exhaustion = Some(error.to_string());
                    break;
                }
            }
            let request = ProcessRequest {
                application: runner.config().process.strategy.clone(),
                scope: Some(PassScope::Process),
                process: pass.process.clone(),
                retry: retry.clone(),
                clean_log: clean_log.clone(),
                variant: None,
                require_reply: true,
                child_budget: true,
            };
            let process_pass = ProcessPass::new(
                Arc::clone(&client),
                self.prompts.clone(),
                Arc::clone(&self.tools),
                Arc::clone(&self.confirm),
                self.prompt_provider.clone(),
                process_class,
            )
            .with_budgeting(runner.config().budget.enabled)
            .with_observer(Arc::clone(&observer), attempt_number);
            observer.pass_started(
                crate::runner::PassKind::Process,
                attempt_number,
                Some(model),
            );
            let run = match process_pass
                .run_accounted(
                    turn,
                    plan,
                    model,
                    variables.clone(),
                    request,
                    Some(&mut envelope),
                )
                .await
            {
                Ok(run) => {
                    observer.pass_finished(&crate::observer::PassSummary {
                        pass: Some(crate::runner::PassKind::Process),
                        attempt: attempt_number,
                        finished: "completed".into(),
                        budget: run.budget,
                        prompt_hash: turn.prompt_hash.clone(),
                        tokens_in: run.tokens_in,
                        tokens_out: run.tokens_out,
                        error: None,
                    });
                    run
                }
                Err(error) => {
                    observer.pass_finished(&crate::observer::PassSummary {
                        pass: Some(crate::runner::PassKind::Process),
                        attempt: attempt_number,
                        finished: "error".into(),
                        error: Some(error.to_string()),
                        ..crate::observer::PassSummary::default()
                    });
                    if reservation > 0 {
                        envelope.release_reservation(reservation);
                    }
                    return Ok(ThreePassFlow::Failed(error.to_string()));
                }
            };
            turn.record_pass(crate::turn::PassRecord {
                kind: crate::runner::PassKind::Process,
                attempt: attempt_number,
                prompt_hash: turn.prompt_hash.clone(),
                budget: run.budget.into(),
                outcome: "completed".into(),
                verification: Vec::new(),
                staged: 0,
                committed: 0,
                checks: Vec::new(),
            });
            prior_effects.extend(run.mutations.iter().cloned());
            let interpret = InterpretResult {
                checks: checks.clone(),
                process: pass.process.clone(),
                fast_path: false,
            };
            let process_result = ProcessResult {
                reply_text: run.reply.clone(),
                tool_emissions: run.tool_emissions.clone(),
            };
            let verify_ctx = PassContext::verify(
                clean_log.clone(),
                state_block.clone(),
                plan.user_query.clone(),
                variables.clone(),
                Some(model.to_owned()),
                interpret,
                process_result,
                verification.clone(),
                retry.clone(),
                prior_effects.clone(),
                attempt,
                envelope.clone(),
            );
            let verify = match runner
                .run_verify(verify_ctx, plan.session_id.as_ref())
                .await
            {
                Ok(verify) => verify,
                Err(error) => {
                    if reservation > 0 {
                        envelope.release_reservation(reservation);
                    }
                    return Ok(ThreePassFlow::Failed(error.to_string()));
                }
            };
            envelope = verify.envelope.clone();
            if reservation > 0 {
                envelope.release_reservation(reservation);
            }
            verification = verify.verification.clone();
            staged_writes.extend(verify.journal.intents.iter().cloned());
            if let Some(error) = &verify.error {
                turn.verification = verification.clone();
                turn.staged_writes = staged_writes.clone();
                turn.discarded_writes = discarded_writes;
                record_verify(turn, attempt_number, &verify, 0);
                return Ok(ThreePassFlow::Failed(error.clone()));
            }
            if verify.is_approved() {
                // An approval with nothing staged is a no-op, not a commit the
                // handler must support.
                let commit = if verify.journal.is_empty() {
                    Ok(Vec::new())
                } else {
                    runner
                        .commit(plan.session_id.as_ref(), &verify.journal)
                        .await
                };
                let committed = match commit {
                    Ok(committed) => committed,
                    Err(error) => {
                        turn.verification = verification.clone();
                        turn.staged_writes = staged_writes.clone();
                        turn.discarded_writes = discarded_writes;
                        record_verify(turn, attempt_number, &verify, 0);
                        return Ok(ThreePassFlow::Failed(format!(
                            "VERIFY_COMMIT_FAILED: {error}"
                        )));
                    }
                };
                committed_writes.extend(committed.iter().cloned());
                turn.verification = verification.clone();
                turn.staged_writes = staged_writes.clone();
                turn.committed_writes = committed_writes.clone();
                turn.discarded_writes = discarded_writes;
                record_verify(turn, attempt_number, &verify, committed.len());
                return Ok(ThreePassFlow::Approved(run.reply));
            }
            if verify.is_rejected() {
                record_verify(turn, attempt_number, &verify, 0);
                discarded_writes = discarded_writes.saturating_add(verify.journal.len() as u32);
                let failures = verify
                    .verification
                    .iter()
                    .filter(|result| !result.pass)
                    .map(|result| RetryFailure {
                        label: result.check_label.clone(),
                        reasoning: if let Some(error) = &result.error {
                            error.clone()
                        } else {
                            result.reasoning.clone()
                        },
                        confidence: result.confidence,
                    })
                    .collect();
                retry = Some(RetryContext {
                    attempt: attempt_number + 1,
                    max_attempts: max_retries + 1,
                    prior_reply_digest: digest(&run.reply),
                    failures,
                    error: verify
                        .rejection
                        .clone()
                        .unwrap_or_else(|| "the reviewer rejected the reply".into()),
                    prior_effects: prior_effects.clone(),
                });
                observer.retry_scheduled(
                    crate::runner::PassKind::Process,
                    attempt_number,
                    attempt_number + 1,
                    retry
                        .as_ref()
                        .map(|retry| retry.error.as_str())
                        .unwrap_or("the reviewer rejected the reply"),
                );
                continue;
            }
            turn.verification = verification.clone();
            turn.staged_writes = staged_writes.clone();
            turn.discarded_writes = discarded_writes;
            record_verify(turn, attempt_number, &verify, 0);
            return Ok(ThreePassFlow::Failed(verify.error.clone().unwrap_or_else(
                || format!("verify finished {}", verify.outcome_name()),
            )));
        }

        turn.verification = verification.clone();
        turn.staged_writes = staged_writes;
        turn.discarded_writes = discarded_writes;
        Ok(ThreePassFlow::Failed(exhaustion.unwrap_or_else(|| {
            verify_exhausted_message(max_retries + 1, &verification, &prior_effects)
        })))
    }

    /// Rebuild the clean log from the agentstream. A closed store yields an
    /// empty log rather than stale internal text; the prompt still carries the
    /// current request and state.
    async fn load_clean_log(&self, session_id: &str) -> CleanLog {
        let Some(provider) = &self.clean_log_provider else {
            return CleanLog::default();
        };
        match provider.clean_log(session_id).await {
            Ok(log) => log,
            Err(error) => {
                debug!(%error, "clean log unavailable; using an empty log");
                CleanLog::default()
            }
        }
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

enum ThreePassFlow {
    FastPath(String),
    Approved(String),
    Failed(String),
}

fn record_verify(turn: &mut Turn, attempt: u32, verify: &VerifyOutcome, committed: usize) {
    turn.record_pass(crate::turn::PassRecord {
        kind: crate::runner::PassKind::Verify,
        attempt,
        prompt_hash: verify.prompt_hashes.last().cloned(),
        budget: verify.budget.into(),
        outcome: verify.outcome_name().to_owned(),
        verification: verify.verification.clone(),
        staged: verify.journal.len() as u32,
        committed: committed as u32,
        checks: verify
            .verification
            .iter()
            .map(|result| result.check_label.clone())
            .collect(),
    });
}

fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn verify_exhausted_message(
    attempts: u32,
    verification: &[VerificationResult],
    effects: &[ProcessMutation],
) -> String {
    let failed = verification
        .iter()
        .filter(|result| !result.pass)
        .map(|result| {
            let reason = result.error.as_deref().unwrap_or(result.reasoning.as_str());
            format!("{} — {}", result.check_label, reason)
        })
        .collect::<Vec<_>>();
    let reason = if failed.is_empty() {
        "the reviewer did not approve".to_owned()
    } else {
        failed.join("; ")
    };
    let effects = effects
        .iter()
        .map(|effect| {
            format!(
                "{} {}",
                effect.tool,
                effect.target.as_deref().unwrap_or("(unnamed)")
            )
        })
        .collect::<Vec<_>>();
    let effects = if effects.is_empty() {
        "none".to_owned()
    } else {
        effects.join(", ")
    };
    format!(
        "VERIFY_EXHAUSTED: I couldn't verify this answer after {attempts} attempts: {reason}. Durable changes already made: {effects}."
    )
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
