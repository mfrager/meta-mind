//! Live chat frames for a three-pass turn.
//!
//! The chat route must return its SSE response before the model answers, so the
//! turn runs in a spawned task and reports through [`ChatStream`]. The stream is
//! both a [`mm_cme::PassObserver`] — it turns pass, round, tool, check and retry
//! hooks into frames — and the writer that appends those frames to the durable
//! run and hands them to the open response.
//!
//! Ordering is the store's: `append_frame_live` returns the run's frame number,
//! and the same number is the SSE `id`, so a client that reconnects with
//! `?from=<seq>` sees exactly the frames it missed and no duplicates.

use std::sync::Arc;

use mm_cme::{PassKind, PassObserver, PassSummary, RoundSummary, VerificationResult};
use mm_llm::ProviderToolCall;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::dto::chat::{ChatFrame, Phase};
use crate::state::AppState;

/// A live run's frame writer.
pub struct ChatStream {
    state: Arc<AppState>,
    /// Frames are attributed to the run, not to the turn's internal id: the turn
    /// id does not exist until the turn is spawned, and every consumer of these
    /// frames correlates by request anyway.
    request_id: String,
    sender: mpsc::UnboundedSender<(u64, ChatFrame)>,
}

impl ChatStream {
    /// Create a stream and the receiver the SSE response reads.
    pub fn new(
        state: Arc<AppState>,
        request_id: String,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<(u64, ChatFrame)>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                state,
                request_id,
                sender,
            }),
            receiver,
        )
    }

    /// Append one frame to the run and, if a response is still open, send it.
    pub fn emit(&self, frame: ChatFrame) {
        let Ok(seq) = self
            .state
            .chat
            .append_frame_live(&self.request_id, frame.clone())
        else {
            return;
        };
        // A disconnected browser is not an error: the frame is in the run and a
        // reconnect replays it.
        let _ = self.sender.send((seq, frame));
    }

    fn turn_id(&self) -> &str {
        &self.request_id
    }
}

fn phase_of(pass: PassKind) -> Phase {
    match pass {
        PassKind::Interpret | PassKind::Think => Phase::Plan,
        PassKind::Process => Phase::Act,
        PassKind::Verify => Phase::Verify,
    }
}

fn json_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!(raw))
}

/// Control calls are the pass protocol, not work the operator asked for. The
/// pass card already shows their outcome, so they must not become tool cards.
fn is_control(name: &str) -> bool {
    matches!(name, "metamind.approve" | "metamind.verify")
}

impl PassObserver for ChatStream {
    fn pass_started(&self, pass: PassKind, attempt: u32, model: Option<&str>) {
        self.emit(ChatFrame::PassStart {
            pass: pass.as_str().to_owned(),
            turn_id: self.turn_id().to_owned(),
            attempt,
            model: model.map(str::to_owned),
            parent_tool_id: None,
        });
    }

    fn pass_finished(&self, summary: &PassSummary) {
        let Some(pass) = summary.pass else {
            return;
        };
        self.emit(ChatFrame::PassComplete {
            pass: pass.as_str().to_owned(),
            finished: summary.finished.clone(),
            budget: serde_json::to_value(summary.budget).unwrap_or(Value::Null),
            prompt_hash: summary.prompt_hash.clone(),
            attempt: Some(summary.attempt),
            parent_tool_id: None,
        });
    }

    fn round_started(
        &self,
        pass: PassKind,
        attempt: u32,
        round: u32,
        streaming: bool,
        model: Option<&str>,
    ) {
        self.emit(ChatFrame::PassRound {
            pass: pass.as_str().to_owned(),
            turn_id: self.turn_id().to_owned(),
            attempt,
            round,
            model: model.map(str::to_owned),
            streaming,
            parent_tool_id: None,
        });
    }

    fn delta(&self, pass: PassKind, attempt: u32, round: u32, text: &str) {
        self.emit(ChatFrame::PassToken {
            pass: pass.as_str().to_owned(),
            token: text.to_owned(),
            is_final: false,
            attempt: Some(attempt),
            round: Some(round),
            channel: Some("prose".into()),
            call_id: None,
            parent_tool_id: None,
        });
    }

    fn round_finished(&self, pass: PassKind, attempt: u32, round: u32, summary: &RoundSummary) {
        self.emit(ChatFrame::PassRoundEnd {
            pass: pass.as_str().to_owned(),
            turn_id: self.turn_id().to_owned(),
            attempt,
            round,
            outcome: summary.outcome.clone(),
            tokens_in: summary.tokens_in,
            tokens_out: summary.tokens_out,
            latency_ms: summary.latency_ms,
            error: summary.error.clone(),
            parent_tool_id: None,
        });
    }

    fn tool_call(&self, pass: PassKind, attempt: u32, round: u32, call: &ProviderToolCall) {
        if is_control(&call.name) {
            return;
        }
        let input = json_value(&call.arguments);
        self.emit(ChatFrame::ToolCall {
            id: call.id.clone(),
            turn_id: self.turn_id().to_owned(),
            phase: phase_of(pass),
            name: call.name.clone(),
            action: input
                .get("action")
                .and_then(Value::as_str)
                .map(str::to_owned),
            input,
            pass: Some(pass.as_str().to_owned()),
            attempt: Some(attempt),
            round: Some(round),
            parent_tool_id: None,
        });
    }

    fn tool_result(
        &self,
        pass: PassKind,
        attempt: u32,
        round: u32,
        call: &ProviderToolCall,
        result: &str,
        ok: bool,
    ) {
        if is_control(&call.name) {
            return;
        }
        self.emit(ChatFrame::ToolResult {
            id: call.id.clone(),
            turn_id: self.turn_id().to_owned(),
            name: call.name.clone(),
            success: ok,
            output: json_value(result),
            duration_ms: 0,
            pass: Some(pass.as_str().to_owned()),
            attempt: Some(attempt),
            round: Some(round),
            parent_tool_id: None,
        });
    }

    fn check(&self, _pass: PassKind, attempt: u32, result: &VerificationResult) {
        let reasoning = if result.pass {
            result.reasoning.clone()
        } else {
            result
                .error
                .clone()
                .unwrap_or_else(|| result.reasoning.clone())
        };
        self.emit(ChatFrame::Verification {
            label: result.check_label.clone(),
            pass: result.pass,
            confidence: result.confidence,
            attempt: Some(attempt),
            backend: Some(result.backend.clone()),
            duration_ms: Some(result.elapsed_ms),
            reasoning: Some(reasoning),
        });
    }

    fn retry_scheduled(&self, pass: PassKind, attempt: u32, next_attempt: u32, reason: &str) {
        self.emit(ChatFrame::PassRetry {
            turn_id: self.turn_id().to_owned(),
            pass: pass.as_str().to_owned(),
            attempt,
            next_attempt,
            reason: reason.to_owned(),
            parent_tool_id: None,
        });
    }
}
