//! The one turn pipeline: Interpret → Process → Verify, with explicit retries.
//!
//! The pipeline is a sequential function. It owns the event sequencing, the
//! staged state patch, the retry loop, and the terminal events; the three pass
//! modules own one stage each.

use std::sync::Arc;
use std::time::Instant;

use mm_agentstream::{
    agent_message, internal_action, internal_error, pass_failed, pass_retry, prompt_rendered,
    state_updated, turn_completed, turn_failed, turn_started, user_message, CleanLog, EventStore,
    SessionState, StatePatch,
};
use mm_core::{SessionId, TurnId};
use mm_llm::{
    CacheMode, CallContext, LlmClient, LlmRequest, LlmResponse, Message, Purpose, ToolSchema, Usage,
};
use mm_prompt::{PromptInstance, PromptStore};
use serde_json::{json, Value};

use crate::budget::{BudgetSnapshot, PassBudget, TurnEnvelope};
use crate::config::ThreePassConfig;
use crate::error::{CmeError, Result};
use crate::pass_scope::PassScope;
use crate::prompt::{CmePromptManager, Persona, PromptVariables};
use crate::state_block;
use crate::tools;
use crate::verification::{
    BackendRegistry, LlmJudgeBackend, ProcessResult, VerificationResult, VerifyCheck, VerifyContext,
};

/// Hard ceiling on model rounds inside one pass.
pub const MAX_PASS_ROUNDS: u32 = 12;

/// One turn request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRequest {
    /// Owning session.
    pub session_id: SessionId,
    /// This turn.
    pub turn_id: TurnId,
    /// The operator's text.
    pub user_input: String,
}

/// One completed turn.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnResult {
    /// This turn.
    pub turn_id: TurnId,
    /// The approved reply.
    pub reply: String,
    /// The model that answered.
    pub model: String,
    /// Process attempts used.
    pub attempts: u32,
    /// Whether verification approved the reply.
    pub verified: bool,
    /// Total usage.
    pub usage: Usage,
    /// Wall time.
    pub duration_ms: u64,
}

/// What Interpret decided.
#[derive(Debug, Clone)]
pub(crate) enum InterpretOutcome {
    /// Answer directly.
    FastPath { reply: String },
    /// Hand off to Process.
    Action {
        checks: Vec<VerifyCheck>,
        process: Option<crate::verification::ProcessContext>,
    },
}

/// One Process attempt.
#[derive(Debug, Clone)]
pub(crate) struct ProcessOutcome {
    /// The reply.
    pub reply: String,
    /// Staged conversation state.
    pub patch: StatePatch,
    /// Tool calls made.
    pub calls: Vec<Value>,
}

/// The reviewer's decision.
#[derive(Debug, Clone)]
pub(crate) enum Verdict {
    /// Accept the reply.
    Approve,
    /// Reject it and retry Process.
    Reject { reason: String },
}

/// One Verify attempt.
#[derive(Debug, Clone)]
pub(crate) struct VerifyOutcome {
    /// The decision.
    pub verdict: Verdict,
    /// Phase A results.
    pub results: Vec<VerificationResult>,
    /// State staged by Verify.
    pub patch: StatePatch,
}

/// Context carried into a retry.
#[derive(Debug, Clone, Default)]
pub(crate) struct RetryContext {
    /// The attempt that was rejected.
    pub attempt: u32,
    /// Maximum attempts.
    pub max_attempts: u32,
    /// The rejection reason.
    pub reason: String,
    /// Failed checks.
    pub failures: Vec<String>,
}

/// The turn pipeline.
pub struct TurnPipeline {
    pub(crate) store: Arc<EventStore>,
    pub(crate) client: Arc<dyn LlmClient>,
    pub(crate) prompts: CmePromptManager,
    pub(crate) config: ThreePassConfig,
    pub(crate) backends: BackendRegistry,
    pub(crate) model: String,
    pub(crate) persona: Persona,
}

impl std::fmt::Debug for TurnPipeline {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TurnPipeline")
            .field("model", &self.model)
            .field("strategies", &self.config.process.strategy)
            .finish_non_exhaustive()
    }
}

impl TurnPipeline {
    /// Build a pipeline over one store, one gateway-wrapped client, and one tree.
    pub fn new(
        store: Arc<EventStore>,
        client: Arc<dyn LlmClient>,
        prompts: Arc<PromptStore>,
        config: ThreePassConfig,
        model: impl Into<String>,
        persona: Persona,
    ) -> Result<Self> {
        let model = model.into();
        if model.trim().is_empty() {
            return Err(CmeError::Config("a turn model is required".into()));
        }
        for strategy in [
            &config.interpret.strategy,
            &config.process.strategy,
            &config.verify.strategy,
        ] {
            if prompts.strategy(strategy).is_none() {
                return Err(CmeError::Config(format!(
                    "prompt strategy `{strategy}` is missing"
                )));
            }
        }
        let mut backends = BackendRegistry::new();
        let judge = config.verification.llm_judge();
        if judge.enabled {
            let judge_model = judge.model.clone().or_else(|| Some(model.clone()));
            backends.register(Arc::new(
                LlmJudgeBackend::new(Some(Arc::clone(&client)), judge_model)
                    .with_timeout(std::time::Duration::from_millis(
                        config.verify.check_timeout_ms,
                    ))
                    .with_sampling(judge.temperature, judge.max_output_tokens),
            ))?;
        }
        Ok(Self {
            store,
            client,
            prompts: CmePromptManager::new(prompts),
            config,
            backends,
            model,
            persona,
        })
    }

    /// The configured model.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Record one best-effort internal action.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_internal(
        &self,
        request: &TurnRequest,
        component: &str,
        action: &str,
        ok: bool,
        duration_ms: u64,
        input: Value,
        output: Value,
        error: Option<&str>,
    ) {
        let _ = self.store.append(internal_action(
            request.session_id.as_str(),
            Some(request.turn_id.as_str().to_owned()),
            component,
            action,
            ok,
            duration_ms,
            input,
            output,
            error,
        ));
    }

    /// Record a recovered internal failure.
    pub(crate) fn record_internal_error(
        &self,
        request: &TurnRequest,
        component: &str,
        action: &str,
        message: &str,
        detail: Option<&str>,
    ) {
        let _ = self.store.append(internal_error(
            request.session_id.as_str(),
            Some(request.turn_id.as_str().to_owned()),
            component,
            action,
            message,
            detail,
        ));
    }

    /// Record a pass budget reservation.
    pub(crate) fn record_budget(&self, request: &TurnRequest, pass: &str, budget: &PassBudget) {
        let snapshot = serde_json::to_value(budget.snapshot()).unwrap_or(Value::Null);
        self.record_internal(
            request,
            &format!("pass.{pass}"),
            "budget.reserve",
            true,
            0,
            json!({ "class": snapshot.get("class").cloned().unwrap_or(Value::Null) }),
            snapshot,
            None,
        );
    }

    /// Record a staged conversation-state patch.
    pub(crate) fn record_state_stage(
        &self,
        request: &TurnRequest,
        pass: &str,
        tool: &str,
        patch: &StatePatch,
    ) {
        self.record_internal(
            request,
            &format!("pass.{pass}"),
            "state.stage",
            true,
            0,
            json!({ "tool": tool }),
            json!({ "stage": patch }),
            None,
        );
    }

    /// Run one turn to completion.
    pub async fn run(&self, request: TurnRequest) -> Result<TurnResult> {
        let started = Instant::now();
        let session = request.session_id.as_str();
        let turn = request.turn_id.as_str();
        self.store.ensure_session(session, "operator", None)?;
        self.store
            .append(user_message(session, turn, request.user_input.clone()))?;
        self.store
            .append(turn_started(session, turn, request.user_input.clone()))?;

        let load_started = Instant::now();
        let events = self.store.events(session, 1, None)?;
        self.record_internal(
            &request,
            "turn.pipeline",
            "state.load",
            true,
            load_started.elapsed().as_millis() as u64,
            json!({ "sessionId": session, "turnId": turn }),
            json!({ "events": events.len() }),
            None,
        );
        let prior = events
            .iter()
            .filter(|event| event.turn_id.as_deref() != Some(turn))
            .cloned()
            .collect::<Vec<_>>();
        let clean_log =
            CleanLog::from_events(&prior, mm_agentstream::projection::CLEAN_LOG_MAX_CHARS);
        let state = mm_agentstream::projection::session_state(&events);
        let state_block = state_block::render("operator", &state);
        let base_budget =
            BudgetSnapshot::for_class(self.config.process.budget_class, self.config.budget.enabled);
        let base = PromptVariables::base(
            &self.persona,
            session,
            &state,
            &base_budget,
            &request.user_input,
            &tools::tools_summary(PassScope::Process),
        );
        let mut usage = Usage::default();

        let interpret = match self
            .run_interpret(&request, &clean_log, &state_block, &base, &mut usage)
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                self.fail(&request, "interpret", 1, &error);
                return Err(error);
            }
        };

        match interpret {
            InterpretOutcome::FastPath { reply } => {
                let reply = if reply.trim().is_empty() {
                    "I don't have an answer for that.".to_owned()
                } else {
                    reply
                };
                self.store
                    .append(agent_message(session, turn, reply.clone(), true))?;
                self.store.append(turn_completed(
                    session,
                    turn,
                    reply.clone(),
                    Some(self.model.clone()),
                    1,
                    true,
                    usage.tokens_in,
                    usage.tokens_out,
                    started.elapsed().as_millis() as u64,
                ))?;
                Ok(TurnResult {
                    turn_id: request.turn_id,
                    reply,
                    model: self.model.clone(),
                    attempts: 1,
                    verified: true,
                    usage,
                    duration_ms: started.elapsed().as_millis() as u64,
                })
            }
            InterpretOutcome::Action { checks, process } => {
                let mut checks = checks;
                if checks.is_empty() {
                    let prompt = self
                        .prompts
                        .store()
                        .fragment("default_action_check")
                        .map(|fragment| fragment.text.clone())
                        .ok_or_else(|| {
                            CmeError::Config(
                                "prompt fragment `default_action_check` is missing".into(),
                            )
                        })?;
                    checks.push(tools::default_action_check(&prompt));
                }
                self.backends
                    .validate_checks(&checks)
                    .map_err(CmeError::from)?;
                let check_json = serde_json::to_value(&checks)?;
                self.store
                    .append(mm_agentstream::event::verification_registered(
                        session, turn, check_json,
                    ))?;

                let max_attempts = self.config.verify.max_retries + 1;
                let mut retry: Option<RetryContext> = None;
                let mut last_reasons = Vec::new();
                let mut envelope = TurnEnvelope::new(
                    max_attempts,
                    self.config.budget.enabled,
                    self.config.envelope.max_tokens,
                );

                for attempt in 1..=max_attempts {
                    let processed = match self
                        .run_process(
                            &request,
                            &clean_log,
                            &state_block,
                            &base,
                            &process,
                            attempt,
                            retry.as_ref(),
                            &mut envelope,
                            &mut usage,
                        )
                        .await
                    {
                        Ok(outcome) => outcome,
                        Err(error) => {
                            self.fail(&request, "process", attempt, &error);
                            return Err(error);
                        }
                    };

                    let verified = match self
                        .run_verify(
                            &request,
                            &clean_log,
                            &state_block,
                            &base,
                            &checks,
                            &processed,
                            attempt,
                            retry.as_ref(),
                            &mut envelope,
                            &mut usage,
                        )
                        .await
                    {
                        Ok(outcome) => outcome,
                        Err(error) => {
                            self.fail(&request, "verify", attempt, &error);
                            return Err(error);
                        }
                    };

                    match verified.verdict {
                        Verdict::Approve => {
                            let mut patch = processed.patch.clone();
                            patch.merge(verified.patch.clone());
                            self.commit_state(&request, &state, &patch)?;
                            self.store.append(agent_message(
                                session,
                                turn,
                                processed.reply.clone(),
                                true,
                            ))?;
                            self.store.append(turn_completed(
                                session,
                                turn,
                                processed.reply.clone(),
                                Some(self.model.clone()),
                                attempt,
                                true,
                                usage.tokens_in,
                                usage.tokens_out,
                                started.elapsed().as_millis() as u64,
                            ))?;
                            return Ok(TurnResult {
                                turn_id: request.turn_id,
                                reply: processed.reply,
                                model: self.model.clone(),
                                attempts: attempt,
                                verified: true,
                                usage,
                                duration_ms: started.elapsed().as_millis() as u64,
                            });
                        }
                        Verdict::Reject { reason } => {
                            last_reasons = verified
                                .results
                                .iter()
                                .filter(|result| !result.pass)
                                .map(|result| {
                                    format!(
                                        "{}: {}",
                                        result.check_label,
                                        result
                                            .error
                                            .clone()
                                            .unwrap_or_else(|| result.reasoning.clone())
                                    )
                                })
                                .collect();
                            let results = serde_json::to_value(&verified.results)?;
                            let _ = self.store.append(pass_retry(
                                session,
                                turn,
                                "verify",
                                attempt,
                                attempt + 1,
                                &reason,
                                results,
                            ));
                            let failures = verified
                                .results
                                .iter()
                                .filter(|result| !result.pass)
                                .map(|result| {
                                    format!(
                                        "{}: {}",
                                        result.check_label,
                                        result
                                            .error
                                            .clone()
                                            .unwrap_or_else(|| result.reasoning.clone())
                                    )
                                })
                                .collect();
                            retry = Some(RetryContext {
                                attempt,
                                max_attempts,
                                reason,
                                failures,
                            });
                        }
                    }
                }

                self.store.append(turn_failed(
                    session,
                    turn,
                    "the reply could not be verified after the allowed attempts",
                    "VERIFY_EXHAUSTED",
                    max_attempts,
                ))?;
                Err(CmeError::VerifyExhausted {
                    reasons: last_reasons,
                })
            }
        }
    }

    fn commit_state(
        &self,
        request: &TurnRequest,
        before: &SessionState,
        patch: &StatePatch,
    ) -> Result<()> {
        let mut after = before.clone();
        if patch.apply(&mut after) {
            self.record_internal(
                request,
                "turn.pipeline",
                "state.commit",
                true,
                0,
                json!({ "fromRevision": before.revision }),
                json!({ "toRevision": after.revision, "changed": true }),
                None,
            );
            self.store.append(state_updated(
                request.session_id.as_str(),
                request.turn_id.as_str(),
                after.revision,
                serde_json::to_value(before)?,
                serde_json::to_value(&after)?,
            ))?;
        } else {
            self.record_internal(
                request,
                "turn.pipeline",
                "state.discard",
                true,
                0,
                json!({ "revision": before.revision, "patch": patch }),
                json!({ "changed": false }),
                None,
            );
        }
        Ok(())
    }

    fn fail(&self, request: &TurnRequest, pass: &str, attempt: u32, error: &CmeError) {
        let _ = self.store.append(pass_failed(
            request.session_id.as_str(),
            request.turn_id.as_str(),
            pass,
            attempt,
            error.to_string(),
            0,
        ));
        let _ = self.store.append(turn_failed(
            request.session_id.as_str(),
            request.turn_id.as_str(),
            error.to_string(),
            error.code(),
            attempt,
        ));
    }

    /// Render a pass prompt and build the full message array.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_messages(
        &self,
        request: &TurnRequest,
        pass: &str,
        attempt: u32,
        round: u32,
        strategy: &str,
        variables: &PromptVariables,
        clean_log: &CleanLog,
        state_block: &str,
        extra: &[Message],
    ) -> Result<(PromptInstance, Vec<Message>)> {
        let instance = self.prompts.render(strategy, variables)?;
        self.store.append(prompt_rendered(
            request.session_id.as_str(),
            request.turn_id.as_str(),
            pass,
            strategy,
            &instance.instance_id,
            &instance.hash,
            serde_json::to_value(instance.variables.clone())?,
            instance.messages.len(),
        ))?;
        let mut messages = instance
            .messages
            .iter()
            .map(|message| match message.role.as_str() {
                "assistant" => Message::assistant(message.content.clone()),
                "user" => Message::user(message.content.clone()),
                _ => Message::system(message.content.clone()),
            })
            .collect::<Vec<_>>();
        messages.push(Message::system(state_block.to_owned()));
        messages.extend(clean_log.messages());
        messages.push(Message::user(request.user_input.clone()));
        messages.extend(extra.iter().cloned());
        let _ = (attempt, round);
        Ok((instance, messages))
    }

    /// Call the gateway-wrapped client with pass attribution.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn call_model(
        &self,
        request: &TurnRequest,
        pass: &str,
        attempt: u32,
        round: u32,
        strategy: &str,
        prompt_instance_id: &str,
        prompt_hash: &str,
        purpose: Purpose,
        messages: Vec<Message>,
        tool_schemas: Vec<ToolSchema>,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<LlmResponse> {
        let mut llm_request = LlmRequest::new(purpose, messages)
            .with_model(self.model.clone())
            .with_tools(tool_schemas)
            .with_cache(CacheMode::Bypass);
        llm_request.max_tokens = max_tokens;
        llm_request.temperature = temperature;
        let context = CallContext::new("turn.pipeline")
            .with_session(request.session_id.as_str())
            .with_run(request.turn_id.as_str())
            .with_pass(pass, Some(round))
            .with_step(format!("attempt={attempt}"))
            .with_origin(strategy)
            .with_prompt(prompt_instance_id, prompt_hash);
        Ok(mm_llm::scope(context, self.client.complete(&llm_request)).await?)
    }

    /// A pass budget for one pass.
    pub(crate) fn pass_budget(&self, class: crate::budget::BudgetClass) -> PassBudget {
        PassBudget::new(class, self.config.budget.enabled)
    }

    /// The process result view a check sees.
    pub(crate) fn process_result(&self, processed: &ProcessOutcome) -> ProcessResult {
        ProcessResult {
            reply_text: processed.reply.clone(),
            tool_calls: processed.calls.clone(),
        }
    }

    /// The verify context a check sees.
    pub(crate) fn verify_context(
        &self,
        request: &TurnRequest,
        clean_log: &CleanLog,
        state_block: &str,
        checks: &[VerifyCheck],
    ) -> VerifyContext {
        VerifyContext {
            clean_log: clean_log.clone(),
            state_summary: state_block.to_owned(),
            user_request: request.user_input.clone(),
            interpret_checks: checks.to_vec(),
        }
    }
}
