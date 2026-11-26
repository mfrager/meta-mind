//! Agentstream mapping for downstream agent runs.
//!
//! Every run gets an isolated `agent/<run_id>` session. The conversation manager
//! never reads this trace directly: it receives a compact turn result, while the
//! inspector and replay path read the raw events.

use mm_data::agentstream::{
    AgentstreamEvent, AgentstreamStore, EventProvenance, EventType, SessionMetadata,
};
use mm_data::Result as DataResult;
use serde_json::{json, Value};

use crate::{AgentKind, AgentRunStatus, AgentTurnResult, AgentTurnSummary, Result};

/// Payload schema for one downstream turn.
pub const AGENT_TURN_SCHEMA: &str = "mm.agent.turn.v1";

/// One message reconstructed from the agentstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayMessage {
    /// `user` or `assistant`.
    pub role: &'static str,
    /// Full text retained in the trace.
    pub content: String,
}

/// The isolated agentstream session id for a run.
pub fn session_id(run_id: &str) -> String {
    format!("agent/{run_id}")
}

/// Start the run's isolated trace session.
pub async fn start_session(
    store: &AgentstreamStore,
    run_id: &str,
    user_id: &str,
    parent_session: &str,
    task: &str,
    kind: AgentKind,
) -> Result<String> {
    let session = session_id(run_id);
    let title = format!(
        "{}: {}",
        kind_name(kind),
        task.chars().take(60).collect::<String>()
    );
    store
        .start_session(
            &session,
            SessionMetadata {
                user_id: user_id.to_owned(),
                project_id: None,
                title: Some(title),
            },
        )
        .await?;
    log(
        store,
        &session,
        EventType::Lifecycle,
        json!({
            "schema": AGENT_TURN_SCHEMA,
            "op": "opened",
            "runId": run_id,
            "parentSessionId": parent_session,
            "kind": kind_name(kind),
        }),
        None,
    )
    .await?;
    Ok(session)
}

/// Append the user's task for a turn.
pub async fn log_user(
    store: &AgentstreamStore,
    session: &str,
    turn: u32,
    input: &str,
) -> Result<AgentstreamEvent> {
    log(
        store,
        session,
        EventType::UserMessage,
        json!({
            "schema": AGENT_TURN_SCHEMA,
            "turn": turn,
            "message": input,
            "raw": input,
        }),
        Some(format!("t{turn}")),
    )
    .await
}

/// Append one completed backend turn, returning the event id used as `raw_ref`.
pub async fn log_turn(
    store: &AgentstreamStore,
    session: &str,
    turn: u32,
    result: &AgentTurnResult,
    raw: &str,
) -> Result<String> {
    let event = log(
        store,
        session,
        EventType::AgentTurn,
        json!({
            "schema": AGENT_TURN_SCHEMA,
            "turn": turn,
            "message": result.message,
            "raw": raw,
            "summary": result.summary,
            "artifacts": result.artifacts,
            "toolCalls": result.tool_calls,
            "usage": result.usage,
            "streamed": result.streamed,
            "model": result.model,
            "status": result.status,
        }),
        Some(format!("t{turn}")),
    )
    .await?;
    Ok(event.event_id)
}

/// Append the retained structured summary for a long turn.
pub async fn log_summary(
    store: &AgentstreamStore,
    session: &str,
    turn: u32,
    summary: &AgentTurnSummary,
) -> Result<()> {
    log(
        store,
        session,
        EventType::AgentTurnSummary,
        json!({
            "schema": AGENT_TURN_SCHEMA,
            "turn": turn,
            "summary": summary,
        }),
        Some(format!("t{turn}")),
    )
    .await?;
    Ok(())
}

/// Append a run status change.
pub async fn log_status(
    store: &AgentstreamStore,
    session: &str,
    run_id: &str,
    status: AgentRunStatus,
    turn: u32,
    next: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    log(
        store,
        session,
        EventType::AgentStatus,
        json!({
            "schema": AGENT_TURN_SCHEMA,
            "runId": run_id,
            "status": status,
            "turn": turn,
            "next": next,
            "error": error,
        }),
        Some(format!("t{turn}")),
    )
    .await?;
    Ok(())
}

/// Rebuild the full backend conversation from retained raw events.
pub async fn replay(store: &AgentstreamStore, session: &str) -> DataResult<Vec<ReplayMessage>> {
    let events = store.events(session).await?;
    let mut messages = Vec::new();
    for event in events {
        let role = match event.event_type {
            EventType::UserMessage => "user",
            EventType::AgentMessage | EventType::AgentTurn => "assistant",
            _ => continue,
        };
        let Some(content) = event
            .payload
            .get("raw")
            .and_then(Value::as_str)
            .or_else(|| event.payload.get("message").and_then(Value::as_str))
        else {
            continue;
        };
        if content.is_empty() {
            continue;
        }
        messages.push(ReplayMessage {
            role,
            content: content.to_owned(),
        });
    }
    Ok(messages)
}

/// Truncate the replayed history to an approximate token budget.
pub fn bound_messages(messages: &mut Vec<ReplayMessage>, max_tokens: u32) {
    let max_chars = (max_tokens as usize).saturating_mul(4).max(4_000);
    let mut total = messages
        .iter()
        .map(|message| message.content.chars().count())
        .sum::<usize>();
    while messages.len() > 1 && total > max_chars {
        let removed = messages.remove(0);
        total = total.saturating_sub(removed.content.chars().count());
    }
}

async fn log(
    store: &AgentstreamStore,
    session: &str,
    event_type: EventType,
    payload: Value,
    turn: Option<String>,
) -> Result<AgentstreamEvent> {
    let mut event = AgentstreamEvent::new(
        session,
        event_type,
        payload,
        EventProvenance {
            source: "agent".into(),
            model: None,
        },
    );
    if let Some(turn) = turn {
        event = event.with_turn(turn);
    }
    let stored = store.log_event(event).await?;
    Ok(stored)
}

/// A stable wire name for a backend.
pub fn kind_name(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Hermes => "hermes",
        AgentKind::Pi => "pi",
    }
}
