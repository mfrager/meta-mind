//! The Process pass.

use std::time::Instant;

use mm_agentstream::StatePatch;
use mm_llm::{Message, Purpose};
use serde_json::{json, Value};

use crate::budget::TurnEnvelope;
use crate::error::{CmeError, Result};
use crate::pipeline::{ProcessOutcome, RetryContext, TurnPipeline, TurnRequest, MAX_PASS_ROUNDS};
use crate::tools;

impl TurnPipeline {
    /// Run Process until the model returns a final reply.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_process(
        &self,
        request: &TurnRequest,
        clean_log: &mm_agentstream::CleanLog,
        state_block: &str,
        base: &crate::prompt::PromptVariables,
        brief: &Option<crate::verification::ProcessContext>,
        attempt: u32,
        retry: Option<&RetryContext>,
        envelope: &mut TurnEnvelope,
        usage: &mut mm_llm::Usage,
    ) -> Result<ProcessOutcome> {
        let session = request.session_id.as_str();
        let turn = request.turn_id.as_str();
        let pass = "process";
        let started = Instant::now();
        self.store.append(mm_agentstream::pass_started(
            session,
            turn,
            pass,
            attempt,
            Some(self.model.clone()),
        ))?;

        let strategy = self.config.process.strategy.clone();
        let scope = crate::pass_scope::PassScope::Process;
        let schemas = tools::schemas(scope);
        let mut budget = self.pass_budget(self.config.process.budget_class);
        self.record_budget(request, pass, &budget);
        let mut history: Vec<Message> = Vec::new();
        let mut patch = StatePatch::default();
        let mut calls: Vec<Value> = Vec::new();
        let mut tokens_in = 0u32;
        let mut tokens_out = 0u32;

        for round in 1..=MAX_PASS_ROUNDS {
            if !budget.reserve_round() || envelope.exhausted() {
                return Err(CmeError::Protocol("Process ran out of budget".into()));
            }
            let mut variables = base.clone();
            variables.set("tools_summary", tools::tools_summary(scope));
            variables.set(
                "process_task",
                brief
                    .as_ref()
                    .map(|brief| brief.task.clone())
                    .unwrap_or_else(|| request.user_input.clone()),
            );
            variables.set(
                "process_constraints",
                brief
                    .as_ref()
                    .map(|brief| brief.constraints.join("\n"))
                    .unwrap_or_default(),
            );
            variables.set(
                "process_suggested_tools",
                brief
                    .as_ref()
                    .and_then(|brief| brief.suggested_tools.clone())
                    .map(|tools| tools.join(", "))
                    .unwrap_or_else(|| tools::tools_summary(scope)),
            );
            variables.set("retry_context", render_retry(retry));
            variables.set(
                "verification_results",
                retry
                    .map(|retry| retry.failures.join("\n"))
                    .unwrap_or_default(),
            );

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
                    Purpose::Converse,
                    messages,
                    schemas.clone(),
                    self.config.process.max_tokens,
                    self.config.process.temperature,
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

            if response.tool_calls.is_empty() {
                let reply = response.text.trim().to_owned();
                if reply.is_empty() {
                    return Err(CmeError::Protocol(
                        "Process returned neither a reply nor a tool call".into(),
                    ));
                }
                self.store.append(mm_agentstream::pass_completed(
                    session,
                    turn,
                    pass,
                    attempt,
                    "completed",
                    Some(reply.clone()),
                    Vec::new(),
                    tokens_in,
                    tokens_out,
                    started.elapsed().as_millis() as u64,
                ))?;
                return Ok(ProcessOutcome {
                    reply,
                    patch,
                    calls,
                });
            }

            history.push(Message::assistant_calls(
                response.text.clone(),
                response.tool_calls.clone(),
            ));
            for call in &response.tool_calls {
                let tool_started = Instant::now();
                let (ok, output) = if !scope.allows(&call.name) {
                    (
                        false,
                        json!({ "ok": false, "error": format!("tool `{}` is not visible here", call.name) })
                            .to_string(),
                    )
                } else {
                    match tools::parse_conversation(&call.name, &call.arguments) {
                        Ok(next) => {
                            patch.merge(next.clone());
                            self.record_state_stage(request, pass, &call.name, &next);
                            (true, tools::staged_output(&call.name, &next))
                        }
                        Err(error) => (
                            false,
                            json!({ "ok": false, "error": error.to_string() }).to_string(),
                        ),
                    }
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
                history.push(Message::tool(
                    call.id.clone(),
                    call.name.clone(),
                    output.clone(),
                ));
                calls.push(json!({
                    "name": call.name,
                    "arguments": call.arguments,
                    "ok": ok,
                    "output": output,
                }));
            }
        }

        Err(CmeError::Protocol(
            "Process did not produce a final reply".into(),
        ))
    }
}

pub(crate) fn add_usage(usage: &mut mm_llm::Usage, next: &mm_llm::Usage) {
    usage.tokens_in = usage.tokens_in.saturating_add(next.tokens_in);
    usage.tokens_out = usage.tokens_out.saturating_add(next.tokens_out);
    usage.cost_micros = usage.cost_micros.saturating_add(next.cost_micros);
    usage.latency_ms = usage.latency_ms.saturating_add(next.latency_ms);
}

fn render_retry(retry: Option<&RetryContext>) -> String {
    let Some(retry) = retry else {
        return String::new();
    };
    let failures = if retry.failures.is_empty() {
        "none".to_owned()
    } else {
        retry.failures.join("; ")
    };
    format!(
        "Attempt {} of {} was rejected: {}. Failed checks: {}",
        retry.attempt, retry.max_attempts, retry.reason, failures
    )
}
