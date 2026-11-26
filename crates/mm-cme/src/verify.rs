//! The Verify pass.

use std::time::Instant;

use mm_llm::{Message, Purpose};
use serde_json::json;

use crate::budget::TurnEnvelope;
use crate::error::{CmeError, Result};
use crate::pass_scope::PassScope;
use crate::pipeline::{
    ProcessOutcome, RetryContext, TurnPipeline, TurnRequest, Verdict, VerifyOutcome,
};
use crate::process::add_usage;
use crate::tools;
use crate::verification::{VerificationResult, VerifyCheck};

impl TurnPipeline {
    /// Run Verify Phase A checks, then the reviewer pass.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_verify(
        &self,
        request: &TurnRequest,
        clean_log: &mm_agentstream::CleanLog,
        state_block: &str,
        base: &crate::prompt::PromptVariables,
        checks: &[VerifyCheck],
        processed: &ProcessOutcome,
        attempt: u32,
        retry: Option<&RetryContext>,
        envelope: &mut TurnEnvelope,
        usage: &mut mm_llm::Usage,
    ) -> Result<VerifyOutcome> {
        let session = request.session_id.as_str();
        let turn = request.turn_id.as_str();
        let pass = "verify";
        let started = Instant::now();
        self.store.append(mm_agentstream::pass_started(
            session,
            turn,
            pass,
            attempt,
            Some(self.model.clone()),
        ))?;

        let results = self
            .run_checks(request, clean_log, state_block, checks, processed, attempt)
            .await?;

        let strategy = self.config.verify.strategy.clone();
        let scope = PassScope::Verify;
        let schemas = tools::schemas(scope);
        let repair_text = self
            .prompts
            .store()
            .fragment("verify_repair")
            .map(|fragment| fragment.text.clone())
            .unwrap_or_else(|| "Finish with exactly one metamind.approve call.".into());
        let mut budget = self.pass_budget(self.config.verify.budget_class);
        self.record_budget(request, pass, &budget);
        let mut history: Vec<Message> = Vec::new();
        let mut patch = mm_agentstream::StatePatch::default();
        let mut tokens_in = 0u32;
        let mut tokens_out = 0u32;

        for round in 1..=3u32 {
            if !budget.reserve_round() || envelope.exhausted() {
                return Err(CmeError::Protocol("Verify ran out of budget".into()));
            }
            let mut variables = base.clone();
            variables.set("tools_summary", tools::tools_summary(scope));
            variables.set("process_reply", &processed.reply);
            variables.set("checks_registered", render_checks(checks));
            variables.set("verification_results", render_results(&results));
            variables.set("staged_intents", serde_json::to_string(&patch)?);
            variables.set("retry_count", attempt.saturating_sub(1).to_string());
            variables.set("retry_context", render_retry(retry));

            let (prompt, messages) = self.prepare_messages(
                request,
                pass,
                attempt,
                round,
                &strategy,
                &variables,
                clean_log,
                state_block,
                &history,
            )?;
            let response = self
                .call_model(
                    request,
                    pass,
                    attempt,
                    round,
                    &strategy,
                    &prompt.instance_id,
                    &prompt.hash,
                    Purpose::Critique,
                    messages,
                    schemas.clone(),
                    self.config.verify.max_tokens,
                    self.config.verify.temperature,
                )
                .await?;
            add_usage(usage, &response.usage);
            tokens_in += response.usage.tokens_in;
            tokens_out += response.usage.tokens_out;
            budget.add_tokens(u64::from(
                response.usage.tokens_in + response.usage.tokens_out,
            ));
            envelope.add_tokens(u64::from(
                response.usage.tokens_in + response.usage.tokens_out,
            ));

            if let Some(call) = response
                .tool_calls
                .iter()
                .find(|call| call.name == "metamind.approve")
            {
                let approval = tools::parse_approve(&call.arguments)?;
                self.store.append(mm_agentstream::event::tool_call(
                    session,
                    turn,
                    &call.id,
                    &call.name,
                    &call.arguments,
                    pass,
                    attempt,
                    round,
                ))?;
                self.store.append(mm_agentstream::event::tool_result(
                    session,
                    turn,
                    &call.id,
                    &call.name,
                    true,
                    &json!({ "ok": true, "success": approval.success }).to_string(),
                    0,
                    false,
                ))?;

                if approval.success {
                    let failed = results.iter().filter(|result| !result.pass).count();
                    if failed > 0 && !self.config.verify.allow_override {
                        let reason = format!(
                            "{} verification check(s) failed: {}",
                            failed,
                            render_failures(&results)
                        );
                        self.store.append(mm_agentstream::pass_completed(
                            session,
                            turn,
                            pass,
                            attempt,
                            "rejected",
                            None,
                            checks.iter().map(|check| check.label.clone()).collect(),
                            tokens_in,
                            tokens_out,
                            started.elapsed().as_millis() as u64,
                        ))?;
                        return Ok(VerifyOutcome {
                            verdict: Verdict::Reject { reason },
                            results,
                            patch,
                        });
                    }
                    self.store.append(mm_agentstream::pass_completed(
                        session,
                        turn,
                        pass,
                        attempt,
                        "approved",
                        None,
                        checks.iter().map(|check| check.label.clone()).collect(),
                        tokens_in,
                        tokens_out,
                        started.elapsed().as_millis() as u64,
                    ))?;
                    return Ok(VerifyOutcome {
                        verdict: Verdict::Approve,
                        results,
                        patch,
                    });
                }

                let reason = approval
                    .error
                    .filter(|error| !error.trim().is_empty())
                    .unwrap_or_else(|| "the reviewer rejected the reply".into());
                self.store.append(mm_agentstream::pass_completed(
                    session,
                    turn,
                    pass,
                    attempt,
                    "rejected",
                    None,
                    checks.iter().map(|check| check.label.clone()).collect(),
                    tokens_in,
                    tokens_out,
                    started.elapsed().as_millis() as u64,
                ))?;
                return Ok(VerifyOutcome {
                    verdict: Verdict::Reject { reason },
                    results,
                    patch,
                });
            }

            let conversation_calls = response
                .tool_calls
                .iter()
                .filter(|call| scope.allows(&call.name))
                .cloned()
                .collect::<Vec<_>>();
            if !conversation_calls.is_empty() {
                history.push(Message::assistant_calls(
                    response.text.clone(),
                    response.tool_calls.clone(),
                ));
                for call in &conversation_calls {
                    let tool_started = Instant::now();
                    let (ok, output) = match tools::parse_conversation(&call.name, &call.arguments)
                    {
                        Ok(next) => {
                            patch.merge(next.clone());
                            self.record_state_stage(request, pass, &call.name, &next);
                            (true, tools::staged_output(&call.name, &next))
                        }
                        Err(error) => (
                            false,
                            json!({ "ok": false, "error": error.to_string() }).to_string(),
                        ),
                    };
                    self.store.append(mm_agentstream::event::tool_call(
                        session,
                        turn,
                        &call.id,
                        &call.name,
                        &call.arguments,
                        pass,
                        attempt,
                        round,
                    ))?;
                    self.store.append(mm_agentstream::event::tool_result(
                        session,
                        turn,
                        &call.id,
                        &call.name,
                        ok,
                        &output,
                        tool_started.elapsed().as_millis() as u64,
                        ok,
                    ))?;
                    history.push(Message::tool(call.id.clone(), call.name.clone(), output));
                }
                continue;
            }

            if round == 1 {
                history.push(Message::user(repair_text.clone()));
                continue;
            }
            let reason = "the reviewer did not call metamind.approve".to_owned();
            self.store.append(mm_agentstream::pass_completed(
                session,
                turn,
                pass,
                attempt,
                "protocol_error",
                None,
                checks.iter().map(|check| check.label.clone()).collect(),
                tokens_in,
                tokens_out,
                started.elapsed().as_millis() as u64,
            ))?;
            return Ok(VerifyOutcome {
                verdict: Verdict::Reject { reason },
                results,
                patch,
            });
        }

        Err(CmeError::Protocol(
            "Verify did not produce an approval".into(),
        ))
    }

    async fn run_checks(
        &self,
        request: &TurnRequest,
        clean_log: &mm_agentstream::CleanLog,
        state_block: &str,
        checks: &[VerifyCheck],
        processed: &ProcessOutcome,
        attempt: u32,
    ) -> Result<Vec<VerificationResult>> {
        let process_result = self.process_result(processed);
        let context = self.verify_context(request, clean_log, state_block, checks);
        let call_context = mm_llm::CallContext::new("turn.pipeline")
            .with_session(request.session_id.as_str())
            .with_run(request.turn_id.as_str())
            .with_pass("verify", Some(1))
            .with_step(format!("attempt={attempt}"))
            .with_origin(&self.config.verify.strategy);
        let mut results = Vec::with_capacity(checks.len());
        for check in checks {
            let dispatch_started = Instant::now();
            self.record_internal(
                request,
                "verification",
                "dispatch",
                true,
                0,
                json!({ "label": check.label, "backend": check.backend, "attempt": attempt }),
                json!({ "dispatched": true }),
                None,
            );
            let result = match self.backends.get(&check.backend) {
                Some(backend) => match mm_llm::scope(
                    call_context.clone(),
                    backend.verify(check, &process_result, &context),
                )
                .await
                {
                    Ok(result) => result,
                    Err(error) => {
                        self.record_internal_error(
                            request,
                            "verification",
                            "check",
                            &error.to_string(),
                            Some(&check.label),
                        );
                        VerificationResult::failed(
                            check,
                            error.to_string(),
                            dispatch_started.elapsed().as_millis() as u64,
                        )
                    }
                },
                None => {
                    let message = format!("unknown verification backend `{}`", check.backend);
                    self.record_internal_error(
                        request,
                        "verification",
                        "check",
                        &message,
                        Some(&check.label),
                    );
                    VerificationResult::failed(
                        check,
                        message,
                        dispatch_started.elapsed().as_millis() as u64,
                    )
                }
            };
            self.store
                .append(mm_agentstream::event::verification_result(
                    request.session_id.as_str(),
                    request.turn_id.as_str(),
                    &result.check_label,
                    &result.backend,
                    result.pass,
                    result.confidence,
                    &result.reasoning,
                    result.elapsed_ms,
                    attempt,
                    result.error.as_deref(),
                ))?;
            results.push(result);
        }
        Ok(results)
    }
}

fn render_checks(checks: &[VerifyCheck]) -> String {
    checks
        .iter()
        .map(|check| {
            format!(
                "- {}: {}",
                check.label,
                check.description.as_deref().unwrap_or("").trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_results(results: &[VerificationResult]) -> String {
    if results.is_empty() {
        return "no checks ran".into();
    }
    results
        .iter()
        .map(|result| {
            format!(
                "- {}: {} ({:.2}) — {}{}",
                result.check_label,
                if result.pass { "PASS" } else { "FAIL" },
                result.confidence,
                result.reasoning,
                result
                    .error
                    .as_deref()
                    .map(|error| format!(" [error: {error}]"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_failures(results: &[VerificationResult]) -> String {
    results
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
        .collect::<Vec<_>>()
        .join("; ")
}

fn render_retry(retry: Option<&RetryContext>) -> String {
    let Some(retry) = retry else {
        return String::new();
    };
    format!(
        "Attempt {} of {} was rejected: {}",
        retry.attempt, retry.max_attempts, retry.reason
    )
}
