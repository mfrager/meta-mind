//! Phase 7 CME routes.
//!
//! `POST /api/cme/chat` is the conversation-engine counterpart of
//! `/api/chat`; `POST /api/chat` delegates here when `MM_CME_ENABLED` is set.
//! The snapshot, metrics and event streams are per session because each CME
//! session owns its own observability hub.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use futures::stream::Stream;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use mm_cme::{ChunkKind, SessionSnapshot};

use crate::chat::validate_attachments;
use crate::dto::chat::{ChatFrame, Phase};
use crate::error::AppError;
use crate::routes::chat::{sse_response, OpenChat};
use crate::state::AppState;

/// Run one query through the conversation engine and stream its SSE frames.
pub async fn open(
    State(state): State<Arc<AppState>>,
    Json(body): Json<OpenChat>,
) -> Result<Response, AppError> {
    if body.query.trim().is_empty() {
        return Err(AppError::BadRequest("a query is required".into()));
    }
    validate_attachments(&body.attachments)?;
    let session_id = match body.session_id.clone() {
        Some(id) if state.chat.session_exists(&id)? => id,
        Some(id) => return Err(AppError::NotFound(format!("no session {id}"))),
        None => {
            let id = state.ids.next_string();
            state.chat.create_session(id.clone())?;
            id
        }
    };
    let request_id = state.ids.next_string();
    state
        .chat
        .begin(request_id.clone(), session_id.clone(), body.query.clone())?;

    let executor = state.cme_executor();
    let outcome = match state
        .cme
        .run_turn(&session_id, body.query.clone(), executor)
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            let frames = vec![ChatFrame::Failed {
                error: "the conversation engine stopped".into(),
                reason: Some(error.to_string()),
            }];
            let _ = state.chat.finish(&request_id, frames, "failed");
            return Err(error);
        }
    };
    let frames = frames_for(&state, &request_id, &session_id, &body.query, &outcome).await;
    let state_name = if outcome.is_success() {
        "completed"
    } else {
        "failed"
    };
    let stored = state.chat.finish(&request_id, frames, state_name)?;
    state
        .cme
        .log_turn_frames(&session_id, &request_id, &body.query, &outcome, &stored)
        .await;
    Ok(sse_response(&stored)?.into_response())
}

async fn frames_for(
    state: &AppState,
    request_id: &str,
    session_id: &str,
    query: &str,
    outcome: &mm_cme::TurnOutcome,
) -> Vec<ChatFrame> {
    let turn_id = outcome.turn_id.to_string();
    let mut frames = vec![
        ChatFrame::Started {
            query: query.to_owned(),
        },
        ChatFrame::Run {
            request_id: request_id.to_owned(),
            session_id: session_id.to_owned(),
            query: query.to_owned(),
        },
    ];
    if outcome.is_success() {
        for chunk in &outcome.chunks {
            if chunk.kind == ChunkKind::Text {
                continue;
            }
            frames.push(ChatFrame::Block {
                turn_id: turn_id.clone(),
                block: json!({
                    "chunkId": chunk.chunk_id,
                    "kind": chunk.kind.as_str(),
                    "state": chunk.state,
                    "text": chunk.text,
                }),
            });
        }
        for call in &outcome.tool_calls {
            frames.push(ChatFrame::ToolCall {
                id: call.call_id.clone(),
                turn_id: turn_id.clone(),
                phase: Phase::Act,
                name: call.name.clone(),
                input: json_value(&call.arguments),
            });
            let output = json_value(&call.result);
            frames.push(ChatFrame::ToolResult {
                id: call.call_id.clone(),
                turn_id: turn_id.clone(),
                name: call.name.clone(),
                success: call.ok,
                output: output.clone(),
                duration_ms: 0,
            });
            if let Some(focus) = output.get("focus") {
                push_focus(state, &mut frames, focus).await;
            }
        }
        frames.push(ChatFrame::LlmResponse {
            turn_id: turn_id.clone(),
            phase: Phase::Final,
            text: outcome.text.clone(),
            visible: true,
            is_final: true,
        });
        frames.push(ChatFrame::FinalResponse {
            response_id: turn_id,
            text: outcome.text.clone(),
            blocks: None,
            suggestions: None,
            unbacked: None,
        });
        let mut subsystems = vec!["cme".into(), "memory".into()];
        if !outcome.tool_calls.is_empty() {
            subsystems.push("tools".into());
        }
        frames.push(ChatFrame::Completed { subsystems });
    } else {
        frames.push(ChatFrame::Failed {
            error: "the conversation turn failed".into(),
            reason: outcome.error.clone(),
        });
    }
    frames
}

/// `GET /api/cme/sessions/{id}/snapshot`.
pub async fn snapshot(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionSnapshot>, AppError> {
    state
        .cme
        .snapshot(&session_id)
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("no CME session {session_id}")))
}

/// `GET /api/cme/sessions/{id}/metrics`.
pub async fn metrics(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Response, AppError> {
    let text = state
        .cme
        .metrics(&session_id)
        .ok_or_else(|| AppError::NotFound(format!("no CME session {session_id}")))?;
    Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], text).into_response())
}

/// `GET /api/cme/sessions/{id}/events` — one SSE event per state transition.
pub async fn events(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let receiver = state
        .cme
        .subscribe(&session_id)
        .ok_or_else(|| AppError::NotFound(format!("no CME session {session_id}")))?;
    let stream = futures::stream::unfold(receiver, |receiver| async move {
        let mut receiver = receiver;
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
                    return Some((
                        Ok(Event::default().event("sm_transition").data(data)),
                        receiver,
                    ));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// `GET /debug/cme` — a server-rendered operator view of CME sessions.
pub async fn debug(State(state): State<Arc<AppState>>) -> Html<String> {
    let mut body = String::from(
        "<!doctype html><html><head><title>CME debug</title></head><body>\
         <h1>Conversation engine</h1>",
    );
    let ids = state.cme.session_ids();
    if ids.is_empty() {
        body.push_str("<p>No CME sessions yet.</p>");
    } else {
        body.push_str("<ul>");
        for id in ids {
            body.push_str("<li><code>");
            body.push_str(&html(&id));
            body.push_str("</code>");
            if let Some(snapshot) = state.cme.snapshot(&id) {
                body.push_str(" &mdash; session=");
                body.push_str(&html(&snapshot.session.state.to_string()));
                body.push_str(", proactive=");
                body.push_str(&html(&snapshot.proactive.state));
                body.push_str(", voice=");
                body.push_str(&html(&snapshot.voice.state));
            }
            body.push_str("</li>");
        }
        body.push_str("</ul>");
    }
    body.push_str(
        "<p>Snapshot: <code>/api/cme/sessions/{id}/snapshot</code>; \
         events: <code>/api/cme/sessions/{id}/events</code>; \
         metrics: <code>/api/cme/sessions/{id}/metrics</code></p>\
         </body></html>",
    );
    Html(body)
}

fn json_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!(raw))
}

async fn push_focus(state: &AppState, frames: &mut Vec<ChatFrame>, focus: &Value) {
    let Some(id) = focus.get("id").and_then(Value::as_str) else {
        return;
    };
    if id == "state" {
        frames.push(ChatFrame::Focus {
            doc: json!({ "id": "state" }),
        });
        return;
    }
    if let Ok(doc) = crate::routes::focus::resolve(state, id).await {
        if let Ok(value) = serde_json::to_value(doc) {
            frames.push(ChatFrame::Focus { doc: value });
        }
    }
}

fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A tiny JSON view used by future debug widgets; kept out of the routes so the
/// page itself stays dependency-free.
#[allow(dead_code)]
fn summary(snapshot: &SessionSnapshot) -> Value {
    serde_json::json!({
        "sessionId": snapshot.session_id,
        "state": snapshot.session.state,
        "turnsCompleted": snapshot.turns_completed,
        "proactiveQueue": snapshot.proactive_queue_length,
        "regions": {
            "proactive": snapshot.proactive,
            "voice": snapshot.voice,
            "memory": snapshot.memory,
            "preferences": snapshot.preferences,
            "plugins": snapshot.plugins,
            "improvement": snapshot.improvement,
        }
    })
}
