//! The Interpret pass.

use std::time::Instant;

use mm_llm::{Message, Purpose};

use crate::error::{CmeError, Result};
use crate::pipeline::{InterpretOutcome, TurnPipeline, TurnRequest};
use crate::tools;

impl TurnPipeline {
    /// Run Interpret: register checks, approve a fast path, or hand off.
    pub(crate) async fn run_interpret(
        &self,
        request: &TurnRequest,
        clean_log: &mm_agentstream::CleanLog,
        state_block: &str,
        base: &crate::prompt::PromptVariables,
        usage: &mut mm_llm::Usage,
    ) -> Result<InterpretOutcome> {
        let session = request.session_id.as_str();
        let turn = request.turn_id.as_str();
        let pass = "interpret";
        let attempt = 1;
        let started = Instant::now();
        self.store.append(mm_agentstream::pass_started(
            session,
            turn,
            pass,
            attempt,
            Some(self.model.clone()),
        ))?;

        let strategy = self.config.interpret.strategy.clone();
        let scope = crate::pass_scope::PassScope::Interpret;
        let schemas = tools::schemas(scope);
        let repair_text = self
            .prompts
            .store()
            .fragment("interpret_repair")
            .map(|fragment| fragment.text.clone())
            .unwrap_or_else(|| {
                "Call exactly one control tool: metamind.approve or metamind.verify.".into()
            });
        let mut budget = self.pass_budget(self.config.interpret.budget_class);
        self.record_budget(request, pass, &budget);
        let mut history: Vec<Message> = Vec::new();
        let mut tokens_in = 0u32;
        let mut tokens_out = 0u32;

        for round in 1..=3u32 {
            if !budget.reserve_round() {
                return Err(CmeError::Protocol("Interpret ran out of rounds".into()));
            }
            let mut variables = base.clone();
            variables.set("tools_summary", tools::tools_summary(scope));
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
                    Purpose::Interpret,
                    messages,
                    schemas.clone(),
                    self.config.interpret.max_tokens,
                    self.config.interpret.temperature,
                )
                .await?;
            usage.tokens_in = usage.tokens_in.saturating_add(response.usage.tokens_in);
            usage.tokens_out = usage.tokens_out.saturating_add(response.usage.tokens_out);
            usage.cost_micros = usage.cost_micros.saturating_add(response.usage.cost_micros);
            usage.latency_ms = usage.latency_ms.saturating_add(response.usage.latency_ms);
            tokens_in += response.usage.tokens_in;
            tokens_out += response.usage.tokens_out;
            budget.add_tokens(u64::from(
                response.usage.tokens_in + response.usage.tokens_out,
            ));

            let approve = response
                .tool_calls
                .iter()
                .find(|call| call.name == "metamind.approve");
            let verify = response
                .tool_calls
                .iter()
                .find(|call| call.name == "metamind.verify");
            if approve.is_some() && verify.is_some() {
                return Err(CmeError::Protocol(
                    "Interpret called both metamind.approve and metamind.verify".into(),
                ));
            }

            if let Some(call) = approve {
                let approval = tools::parse_approve(&call.arguments)?;
                self.record_tool(request, pass, attempt, round, call, true)?;
                if !approval.success {
                    return Err(CmeError::Protocol(
                        approval
                            .error
                            .unwrap_or_else(|| "Interpret rejected the turn".into()),
                    ));
                }
                let reply = approval
                    .reply
                    .filter(|reply| !reply.trim().is_empty())
                    .or_else(|| {
                        let text = response.text.trim();
                        (!text.is_empty()).then(|| text.to_owned())
                    })
                    .ok_or_else(|| {
                        CmeError::Protocol(
                            "metamind.approve(true) needs a reply on the fast path".into(),
                        )
                    })?;
                self.store.append(mm_agentstream::pass_completed(
                    session,
                    turn,
                    pass,
                    attempt,
                    "fast_path",
                    Some(reply.clone()),
                    Vec::new(),
                    tokens_in,
                    tokens_out,
                    started.elapsed().as_millis() as u64,
                ))?;
                return Ok(InterpretOutcome::FastPath { reply });
            }

            if let Some(call) = verify {
                let result = tools::parse_verify(&call.arguments)?;
                self.record_tool(request, pass, attempt, round, call, true)?;
                let checks = result
                    .checks
                    .iter()
                    .map(|check| check.label.clone())
                    .collect::<Vec<_>>();
                self.store.append(mm_agentstream::pass_completed(
                    session,
                    turn,
                    pass,
                    attempt,
                    "action",
                    None,
                    checks,
                    tokens_in,
                    tokens_out,
                    started.elapsed().as_millis() as u64,
                ))?;
                return Ok(InterpretOutcome::Action {
                    checks: result.checks,
                    process: result.process,
                });
            }

            if round == 1 {
                history.push(Message::user(repair_text.clone()));
                continue;
            }
            return Err(CmeError::Protocol(
                "Interpret did not call a control tool".into(),
            ));
        }

        Err(CmeError::Protocol(
            "Interpret did not call a control tool".into(),
        ))
    }

    fn record_tool(
        &self,
        request: &TurnRequest,
        pass: &str,
        attempt: u32,
        round: u32,
        call: &mm_llm::ProviderToolCall,
        ok: bool,
    ) -> Result<()> {
        let output = serde_json::json!({ "ok": ok }).to_string();
        self.store.append(mm_agentstream::event::tool_call(
            request.session_id.as_str(),
            request.turn_id.as_str(),
            &call.id,
            &call.name,
            &call.arguments,
            pass,
            attempt,
            round,
        ))?;
        self.store.append(mm_agentstream::event::tool_result(
            request.session_id.as_str(),
            request.turn_id.as_str(),
            &call.id,
            &call.name,
            ok,
            &output,
            0,
            false,
        ))?;
        Ok(())
    }
}
