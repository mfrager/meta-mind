//! The agentstream event envelope and constructors.
//!
//! Every line of a session JSONL file is one [`Event`]. The envelope is stable;
//! `data` is type-specific. `seq` is assigned by the store, never by a caller.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ts_rs::TS;
use ulid::Ulid;

/// The schema version this crate writes.
pub const SCHEMA_VERSION: u32 = 2;

/// A persisted or live agentstream event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agentstream.ts")]
pub struct Event {
    /// Schema version.
    pub v: u32,
    /// Per-session cursor, starting at 1.
    pub seq: u64,
    /// Event ULID.
    pub id: String,
    /// Owning session.
    pub session_id: String,
    /// Owning turn, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Epoch milliseconds.
    pub at: u64,
    /// Dot-separated event type.
    #[serde(rename = "type")]
    pub event_type: EventType,
    /// Emitting component.
    pub source: String,
    /// Model involved, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Full payload.
    #[ts(type = "unknown")]
    pub data: Value,
}

/// Every event the retained core writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "agentstream.ts")]
pub enum EventType {
    /// A session was created.
    #[serde(rename = "session.started")]
    SessionStarted,
    /// A session ended.
    #[serde(rename = "session.ended")]
    SessionEnded,
    /// A turn started.
    #[serde(rename = "turn.started")]
    TurnStarted,
    /// A turn completed successfully.
    #[serde(rename = "turn.completed")]
    TurnCompleted,
    /// A turn failed.
    #[serde(rename = "turn.failed")]
    TurnFailed,
    /// A pass started.
    #[serde(rename = "pass.started")]
    PassStarted,
    /// A pass completed.
    #[serde(rename = "pass.completed")]
    PassCompleted,
    /// A pass failed.
    #[serde(rename = "pass.failed")]
    PassFailed,
    /// A rejected attempt will be retried.
    #[serde(rename = "pass.retry")]
    PassRetry,
    /// A prompt was rendered.
    #[serde(rename = "prompt.rendered")]
    PromptRendered,
    /// A model request, written before the provider is touched.
    #[serde(rename = "llm.request")]
    LlmRequest,
    /// One provider attempt.
    #[serde(rename = "llm.attempt")]
    LlmAttempt,
    /// A live-only text delta.
    #[serde(rename = "llm.delta")]
    LlmDelta,
    /// A model response.
    #[serde(rename = "llm.response")]
    LlmResponse,
    /// A model failure.
    #[serde(rename = "llm.error")]
    LlmError,
    /// A tool was called.
    #[serde(rename = "tool.call")]
    ToolCall,
    /// A tool returned.
    #[serde(rename = "tool.result")]
    ToolResult,
    /// Conversation state was committed.
    #[serde(rename = "state.updated")]
    StateUpdated,
    /// Verification checks were registered.
    #[serde(rename = "verification.registered")]
    VerificationRegistered,
    /// One verification check returned.
    #[serde(rename = "verification.result")]
    VerificationResult,
    /// An internal action.
    #[serde(rename = "internal.action")]
    InternalAction,
    /// A recovered internal error.
    #[serde(rename = "internal.error")]
    InternalError,
    /// The assistant's final approved message.
    #[serde(rename = "agent.message")]
    AgentMessage,
    /// The operator's message.
    #[serde(rename = "user.message")]
    UserMessage,
}

impl EventType {
    /// Every event name, in a stable order. The contract test asserts this list.
    pub const ALL: [EventType; 24] = [
        EventType::SessionStarted,
        EventType::SessionEnded,
        EventType::TurnStarted,
        EventType::TurnCompleted,
        EventType::TurnFailed,
        EventType::PassStarted,
        EventType::PassCompleted,
        EventType::PassFailed,
        EventType::PassRetry,
        EventType::PromptRendered,
        EventType::LlmRequest,
        EventType::LlmAttempt,
        EventType::LlmDelta,
        EventType::LlmResponse,
        EventType::LlmError,
        EventType::ToolCall,
        EventType::ToolResult,
        EventType::StateUpdated,
        EventType::VerificationRegistered,
        EventType::VerificationResult,
        EventType::InternalAction,
        EventType::InternalError,
        EventType::AgentMessage,
        EventType::UserMessage,
    ];

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::SessionStarted => "session.started",
            EventType::SessionEnded => "session.ended",
            EventType::TurnStarted => "turn.started",
            EventType::TurnCompleted => "turn.completed",
            EventType::TurnFailed => "turn.failed",
            EventType::PassStarted => "pass.started",
            EventType::PassCompleted => "pass.completed",
            EventType::PassFailed => "pass.failed",
            EventType::PassRetry => "pass.retry",
            EventType::PromptRendered => "prompt.rendered",
            EventType::LlmRequest => "llm.request",
            EventType::LlmAttempt => "llm.attempt",
            EventType::LlmDelta => "llm.delta",
            EventType::LlmResponse => "llm.response",
            EventType::LlmError => "llm.error",
            EventType::ToolCall => "tool.call",
            EventType::ToolResult => "tool.result",
            EventType::StateUpdated => "state.updated",
            EventType::VerificationRegistered => "verification.registered",
            EventType::VerificationResult => "verification.result",
            EventType::InternalAction => "internal.action",
            EventType::InternalError => "internal.error",
            EventType::AgentMessage => "agent.message",
            EventType::UserMessage => "user.message",
        }
    }

    /// Parse a wire name.
    pub fn parse(name: &str) -> Option<Self> {
        EventType::ALL
            .into_iter()
            .find(|event_type| event_type.as_str() == name)
    }

    /// Whether the event is persisted. `llm.delta` is live-only.
    pub fn is_persisted(self) -> bool {
        self != EventType::LlmDelta
    }
}

/// Epoch milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// An unsaved event. The store assigns `seq` and the final `at`.
#[derive(Debug, Clone)]
pub struct NewEvent {
    /// Owning session.
    pub session_id: String,
    /// Owning turn.
    pub turn_id: Option<String>,
    /// Event type.
    pub event_type: EventType,
    /// Emitting component.
    pub source: String,
    /// Model, when applicable.
    pub model: Option<String>,
    /// Payload.
    pub data: Value,
}

impl NewEvent {
    /// Build a new event.
    pub fn new(
        session_id: impl Into<String>,
        turn_id: Option<String>,
        event_type: EventType,
        source: impl Into<String>,
        model: Option<String>,
        data: Value,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id,
            event_type,
            source: source.into(),
            model,
            data,
        }
    }

    /// Materialize with a sequence number.
    pub fn into_event(self, seq: u64, at: u64) -> Event {
        Event {
            v: SCHEMA_VERSION,
            seq,
            id: crate::ulid_string(&Ulid::from_datetime(std::time::SystemTime::now())),
            session_id: self.session_id,
            turn_id: self.turn_id,
            at,
            event_type: self.event_type,
            source: self.source,
            model: self.model,
            data: self.data,
        }
    }
}

/// A live-only delta event.
pub fn llm_delta(
    session_id: impl Into<String>,
    turn_id: Option<String>,
    call_id: &str,
    index: u32,
    text: &str,
) -> NewEvent {
    NewEvent::new(
        session_id,
        turn_id,
        EventType::LlmDelta,
        "llm.gateway",
        None,
        json!({ "callId": call_id, "index": index, "text": text }),
    )
}

/// A user message.
pub fn user_message(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    text: impl Into<String>,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::UserMessage,
        "turn.pipeline",
        None,
        json!({ "text": text.into() }),
    )
}

/// A turn start.
pub fn turn_started(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    user_input: impl Into<String>,
) -> NewEvent {
    let input = user_input.into();
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::TurnStarted,
        "turn.pipeline",
        None,
        json!({ "userInput": input, "text": input }),
    )
}

/// A successful turn completion.
#[allow(clippy::too_many_arguments)]
pub fn turn_completed(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    reply: impl Into<String>,
    model: Option<String>,
    attempts: u32,
    verified: bool,
    tokens_in: u32,
    tokens_out: u32,
    duration_ms: u64,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::TurnCompleted,
        "turn.pipeline",
        model.clone(),
        json!({
            "reply": reply.into(),
            "model": model.clone(),
            "attempts": attempts,
            "verified": verified,
            "usage": { "tokensIn": tokens_in, "tokensOut": tokens_out },
            "durationMs": duration_ms,
        }),
    )
}

/// A failed turn.
pub fn turn_failed(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    error: impl Into<String>,
    kind: impl Into<String>,
    attempts: u32,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::TurnFailed,
        "turn.pipeline",
        None,
        json!({ "error": error.into(), "kind": kind.into(), "attempts": attempts }),
    )
}

/// A pass start.
pub fn pass_started(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    pass: &str,
    attempt: u32,
    model: Option<String>,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::PassStarted,
        format!("pass.{pass}"),
        model.clone(),
        json!({ "pass": pass, "attempt": attempt, "model": model }),
    )
}

/// A pass completion.
#[allow(clippy::too_many_arguments)]
pub fn pass_completed(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    pass: &str,
    attempt: u32,
    finished: &str,
    text: Option<String>,
    checks: Vec<String>,
    tokens_in: u32,
    tokens_out: u32,
    duration_ms: u64,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::PassCompleted,
        format!("pass.{pass}"),
        None,
        json!({
            "pass": pass,
            "attempt": attempt,
            "finished": finished,
            "text": text,
            "checks": checks,
            "usage": { "tokensIn": tokens_in, "tokensOut": tokens_out },
            "durationMs": duration_ms,
        }),
    )
}

/// A pass failure.
pub fn pass_failed(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    pass: &str,
    attempt: u32,
    error: impl Into<String>,
    duration_ms: u64,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::PassFailed,
        format!("pass.{pass}"),
        None,
        json!({
            "pass": pass,
            "attempt": attempt,
            "error": error.into(),
            "durationMs": duration_ms,
        }),
    )
}

/// A retry after a rejection.
pub fn pass_retry(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    pass: &str,
    attempt: u32,
    next_attempt: u32,
    reason: impl Into<String>,
    verification: Value,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::PassRetry,
        format!("pass.{pass}"),
        None,
        json!({
            "pass": pass,
            "attempt": attempt,
            "nextAttempt": next_attempt,
            "reason": reason.into(),
            "verification": verification,
        }),
    )
}

/// A rendered prompt record.
pub fn prompt_rendered(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    pass: &str,
    strategy: &str,
    instance_id: &str,
    hash: &str,
    variables: Value,
    message_count: usize,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::PromptRendered,
        format!("pass.{pass}"),
        None,
        json!({
            "pass": pass,
            "strategy": strategy,
            "instanceId": instance_id,
            "hash": hash,
            "variables": variables,
            "messageCount": message_count,
        }),
    )
}

/// A model request. Full messages and tools are part of the record.
#[allow(clippy::too_many_arguments)]
pub fn llm_request(
    session_id: impl Into<String>,
    turn_id: Option<String>,
    call_id: &str,
    pass: Option<&str>,
    attempt: u32,
    round: u32,
    provider: &str,
    purpose: &str,
    strategy: Option<&str>,
    prompt_instance_id: Option<&str>,
    prompt_hash: Option<&str>,
    params: Value,
    messages: Value,
    tools: Value,
) -> NewEvent {
    NewEvent::new(
        session_id,
        turn_id,
        EventType::LlmRequest,
        "llm.gateway",
        None,
        json!({
            "callId": call_id,
            "pass": pass,
            "attempt": attempt,
            "round": round,
            "provider": provider,
            "purpose": purpose,
            "strategy": strategy,
            "promptInstanceId": prompt_instance_id,
            "promptHash": prompt_hash,
            "params": params,
            "messageCount": messages.as_array().map(Vec::len).unwrap_or(0),
            "toolCount": tools.as_array().map(Vec::len).unwrap_or(0),
            "messages": messages,
            "tools": tools,
        }),
    )
}

/// One provider attempt.
#[allow(clippy::too_many_arguments)]
pub fn llm_attempt(
    session_id: impl Into<String>,
    turn_id: Option<String>,
    call_id: &str,
    attempt_no: u32,
    status: &str,
    model: Option<&str>,
    latency_ms: u64,
    tokens_in: u32,
    tokens_out: u32,
) -> NewEvent {
    NewEvent::new(
        session_id,
        turn_id,
        EventType::LlmAttempt,
        "llm.gateway",
        model.map(str::to_owned),
        json!({
            "callId": call_id,
            "attemptNo": attempt_no,
            "status": status,
            "model": model,
            "latencyMs": latency_ms,
            "usage": { "tokensIn": tokens_in, "tokensOut": tokens_out },
        }),
    )
}

/// A model response.
#[allow(clippy::too_many_arguments)]
pub fn llm_response(
    session_id: impl Into<String>,
    turn_id: Option<String>,
    call_id: &str,
    model: &str,
    text: &str,
    tool_calls: Value,
    tokens_in: u32,
    tokens_out: u32,
    latency_ms: u64,
    finish_reason: Option<&str>,
    reasoning_chars: u32,
    cached: bool,
    streamed: bool,
) -> NewEvent {
    NewEvent::new(
        session_id,
        turn_id,
        EventType::LlmResponse,
        "llm.gateway",
        Some(model.to_owned()),
        json!({
            "callId": call_id,
            "status": "ok",
            "model": model,
            "text": text,
            "toolCalls": tool_calls,
            "usage": { "tokensIn": tokens_in, "tokensOut": tokens_out },
            "latencyMs": latency_ms,
            "finishReason": finish_reason,
            "reasoningChars": reasoning_chars,
            "cached": cached,
            "streamed": streamed,
        }),
    )
}

/// A model failure.
pub fn llm_error(
    session_id: impl Into<String>,
    turn_id: Option<String>,
    call_id: &str,
    kind: &str,
    message: &str,
    latency_ms: u64,
    partial_chars: usize,
) -> NewEvent {
    NewEvent::new(
        session_id,
        turn_id,
        EventType::LlmError,
        "llm.gateway",
        None,
        json!({
            "callId": call_id,
            "kind": kind,
            "message": message,
            "latencyMs": latency_ms,
            "partialChars": partial_chars,
        }),
    )
}

/// A tool call.
pub fn tool_call(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    call_id: &str,
    name: &str,
    arguments: &str,
    pass: &str,
    attempt: u32,
    round: u32,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::ToolCall,
        format!("tool.{name}"),
        None,
        json!({
            "callId": call_id,
            "name": name,
            "arguments": arguments,
            "pass": pass,
            "attempt": attempt,
            "round": round,
        }),
    )
}

/// A tool result.
pub fn tool_result(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    call_id: &str,
    name: &str,
    ok: bool,
    output: &str,
    duration_ms: u64,
    staged: bool,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::ToolResult,
        format!("tool.{name}"),
        None,
        json!({
            "callId": call_id,
            "name": name,
            "ok": ok,
            "output": output,
            "durationMs": duration_ms,
            "staged": staged,
        }),
    )
}

/// A committed state revision.
pub fn state_updated(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    revision: u64,
    before: Value,
    after: Value,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::StateUpdated,
        "turn.pipeline",
        None,
        json!({ "revision": revision, "before": before, "after": after }),
    )
}

/// Registered checks.
pub fn verification_registered(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    checks: Value,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::VerificationRegistered,
        "pass.interpret",
        None,
        json!({ "checks": checks }),
    )
}

/// One verification result.
#[allow(clippy::too_many_arguments)]
pub fn verification_result(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    label: &str,
    backend: &str,
    pass: bool,
    confidence: f64,
    reasoning: &str,
    duration_ms: u64,
    attempt: u32,
    error: Option<&str>,
) -> NewEvent {
    NewEvent::new(
        session_id,
        Some(turn_id.into()),
        EventType::VerificationResult,
        "verification",
        None,
        json!({
            "label": label,
            "backend": backend,
            "pass": pass,
            "confidence": confidence,
            "reasoning": reasoning,
            "durationMs": duration_ms,
            "attempt": attempt,
            "error": error,
        }),
    )
}

/// An internal action.
#[allow(clippy::too_many_arguments)]
pub fn internal_action(
    session_id: impl Into<String>,
    turn_id: Option<String>,
    component: &str,
    action: &str,
    ok: bool,
    duration_ms: u64,
    input: Value,
    output: Value,
    error: Option<&str>,
) -> NewEvent {
    NewEvent::new(
        session_id,
        turn_id,
        EventType::InternalAction,
        component.to_owned(),
        None,
        json!({
            "component": component,
            "action": action,
            "ok": ok,
            "durationMs": duration_ms,
            "input": input,
            "output": output,
            "error": error,
        }),
    )
}

/// A recovered internal error.
pub fn internal_error(
    session_id: impl Into<String>,
    turn_id: Option<String>,
    component: &str,
    action: &str,
    message: &str,
    detail: Option<&str>,
) -> NewEvent {
    NewEvent::new(
        session_id,
        turn_id,
        EventType::InternalError,
        component.to_owned(),
        None,
        json!({
            "component": component,
            "action": action,
            "message": message,
            "detail": detail,
        }),
    )
}

/// The assistant's approved final message.
pub fn agent_message(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    text: impl Into<String>,
    verified: bool,
) -> NewEvent {
    let turn_id = turn_id.into();
    NewEvent::new(
        session_id,
        Some(turn_id.clone()),
        EventType::AgentMessage,
        "turn.pipeline",
        None,
        json!({ "text": text.into(), "final": true, "verified": verified, "replyTo": turn_id }),
    )
}

/// A session start.
pub fn session_started(
    session_id: impl Into<String>,
    user_id: impl Into<String>,
    title: Option<String>,
) -> NewEvent {
    NewEvent::new(
        session_id,
        None,
        EventType::SessionStarted,
        "server",
        None,
        json!({ "userId": user_id.into(), "title": title }),
    )
}

/// A session end.
pub fn session_ended(session_id: impl Into<String>, reason: impl Into<String>) -> NewEvent {
    NewEvent::new(
        session_id,
        None,
        EventType::SessionEnded,
        "server",
        None,
        json!({ "reason": reason.into() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_name_round_trips() {
        for event_type in EventType::ALL {
            assert_eq!(EventType::parse(event_type.as_str()), Some(event_type));
        }
        assert_eq!(EventType::parse("nope"), None);
    }

    #[test]
    fn envelope_uses_camel_case_and_version_two() {
        let event = user_message("s", "t", "hello").into_event(7, 100);
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["v"], 2);
        assert_eq!(value["seq"], 7);
        assert_eq!(value["sessionId"], "s");
        assert_eq!(value["turnId"], "t");
        assert_eq!(value["type"], "user.message");
        assert_eq!(value["data"]["text"], "hello");
        assert_eq!(event.id.len(), 26);
    }

    #[test]
    fn only_deltas_are_live_only() {
        for event_type in EventType::ALL {
            assert_eq!(event_type.is_persisted(), event_type != EventType::LlmDelta);
        }
    }
}
