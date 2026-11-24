//! Phase 7 CME routes.
//!
//! `POST /api/cme/chat` is the conversation-engine counterpart of
//! `/api/chat`; `POST /api/chat` delegates here when `MM_CME_ENABLED` is set.
//! The snapshot, metrics and event streams are per session because each CME
//! session owns its own observability hub.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use mm_cme::{
    BudgetClass, BudgetLedger, ChunkKind, PromptVariables, SessionSnapshot, UserProjection,
};
use mm_llm::CallContext;

use crate::chat::validate_attachments;
use crate::cme::CONSOLE_USER;
use crate::cme_stream::ChatStream;
use crate::dto::chat::{ChatFrame, Phase};
use crate::error::AppError;
use crate::routes::chat::{sse_response, OpenChat};
use crate::routes::console::camelize;
use crate::state::AppState;

/// Process inspector query.
#[derive(Debug, Deserialize)]
pub struct ProcessQuery {
    /// Limit to one conversation session.
    pub session_id: Option<String>,
}

/// Process control body.
#[derive(Debug, Default, Deserialize)]
pub struct ProcessControlBody {
    /// Resume input.
    pub input: Option<String>,
    /// Step for `retry`.
    pub step_id: Option<String>,
}

/// List process templates and instances for the inspector.
pub async fn processes(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProcessQuery>,
) -> Result<Json<Value>, AppError> {
    if !state.features.process_v1 {
        return Ok(Json(json!({
            "enabled": false,
            "processes": [],
            "templates": [],
        })));
    }
    let engine = state
        .process_engine()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let processes = match query.session_id.as_deref() {
        Some(session) => engine.session_instances(session).await,
        None => engine.store().list_instances().await,
    }
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let templates = engine
        .templates(false)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let analytics = engine
        .analytics(query.session_id.as_deref())
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(Json(camelize(json!({
        "enabled": true,
        "processes": processes,
        "templates": templates,
        "analytics": analytics,
    }))))
}

/// One process with its pinned template.
pub async fn process_detail(
    State(state): State<Arc<AppState>>,
    Path(process_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let engine = state
        .process_engine()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let process = engine
        .load_instance(&process_id)
        .await
        .map_err(|error| AppError::NotFound(error.to_string()))?;
    let template = engine
        .template(&process.template_uri)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(Json(camelize(
        json!({ "process": process, "template": template }),
    )))
}

/// Pause, resume, cancel or retry one process.
pub async fn process_control(
    State(state): State<Arc<AppState>>,
    Path((process_id, action)): Path<(String, String)>,
    Json(body): Json<ProcessControlBody>,
) -> Result<Json<Value>, AppError> {
    let engine = state
        .process_engine()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let process = match action.as_str() {
        "pause" => engine.pause(&process_id).await,
        "resume" => engine.resume(&process_id, body.input.as_deref()).await,
        "cancel" => engine.cancel(&process_id).await,
        "retry" => {
            let step = body
                .step_id
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("step_id is required for retry".into()))?;
            engine.retry_step(&process_id, step).await
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown process control {other:?}"
            )))
        }
    }
    .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(Json(camelize(json!({ "process": process }))))
}

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
    state
        .state_service
        .ensure_session(&session_id, CONSOLE_USER, mm_data::rdf::now_ms())
        .await
        .map_err(|error| AppError::Internal(format!("conversation state: {error}")))?;

    if let Some(automation) = &state.automation {
        automation.ensure(&session_id).await;
    }
    state.status_service.mark_dirty_many(
        &session_id,
        [
            crate::status_service::StatusSection::State,
            crate::status_service::StatusSection::Session,
            crate::status_service::StatusSection::Frames,
            crate::status_service::StatusSection::Prompt,
            crate::status_service::StatusSection::Operations,
            crate::status_service::StatusSection::Data,
            crate::status_service::StatusSection::Plans,
            crate::status_service::StatusSection::Actions,
            crate::status_service::StatusSection::Agents,
        ],
    );
    let bundle = state
        .state_service
        .load(&session_id)
        .await
        .map_err(|error| AppError::Internal(format!("conversation bundle: {error}")))?;
    let budget = BudgetLedger::new(BudgetClass::Standard).snapshot();
    let variables = PromptVariables::from_bundle(
        &bundle,
        UserProjection {
            iri: "mmd:current-user".into(),
            entity: bundle.session.user_id.clone(),
            name: CONSOLE_USER.into(),
            workspace: format!("workspace/{session_id}"),
        },
        budget,
    )
    .map_err(|error| AppError::Internal(format!("prompt variables: {error}")))?;
    let variant = if bundle.conversation.status.contains_key("next")
        || bundle.conversation.status.contains_key("blockers")
    {
        Some("pending")
    } else {
        None
    };
    let mut executor = state.cme_executor().with_prompt_variables(variables);
    if let Some(variant) = variant {
        executor = executor.with_prompt_variant(variant);
    }

    if state.three_pass.turn.stream {
        let (chat_stream, receiver) = ChatStream::new(Arc::clone(&state), request_id.clone());
        chat_stream.emit(ChatFrame::Started {
            query: body.query.clone(),
        });
        chat_stream.emit(ChatFrame::Run {
            request_id: request_id.clone(),
            session_id: session_id.clone(),
            query: body.query.clone(),
        });
        let executor = executor.with_pass_observer(chat_stream.clone());
        let task_state = Arc::clone(&state);
        let query = body.query.clone();
        let turn_session = session_id.clone();
        let activity = CallContext::new("chat")
            .with_session(turn_session.clone())
            .with_run(request_id.clone());
        tokio::spawn(mm_llm::scope(activity, async move {
            let result = task_state
                .cme
                .run_turn(&turn_session, query.clone(), executor)
                .await;
            let outcome = match result {
                Ok(outcome) => outcome,
                Err(error) => {
                    chat_stream.emit(ChatFrame::Failed {
                        error: "the conversation engine stopped".into(),
                        reason: Some(error.to_string()),
                    });
                    let _ = task_state.chat.finish_stream(&request_id, "failed");
                    return;
                }
            };
            settle_after_turn(&task_state, &turn_session).await;
            for frame in terminal_frames(&task_state, &request_id, &query, &outcome).await {
                chat_stream.emit(frame);
            }
            let state_name = if outcome.is_success() {
                "completed"
            } else {
                "failed"
            };
            let _ = task_state.chat.finish_stream(&request_id, state_name);
            if let Ok(stored) = task_state.chat.frames_from(&request_id, 1) {
                task_state
                    .cme
                    .log_turn_frames(&turn_session, &request_id, &query, &outcome, &stored)
                    .await;
            }
        }));
        let stream = futures::stream::unfold(receiver, |mut receiver| async move {
            receiver
                .recv()
                .await
                .map(|(seq, frame)| (Ok::<Event, Infallible>(frame_event(seq, frame)), receiver))
        });
        return Ok(Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let activity = CallContext::new("chat")
        .with_session(session_id.clone())
        .with_run(request_id.clone());
    let outcome = match mm_llm::scope(activity, async {
        state
            .cme
            .run_turn(&session_id, body.query.clone(), executor)
            .await
    })
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
    settle_after_turn(&state, &session_id).await;
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

/// The post-turn refreshes every path performs, so a streamed and a buffered
/// turn leave the same state behind.
async fn settle_after_turn(state: &AppState, session_id: &str) {
    if state.automation.is_some() {
        let _ = crate::automation::resume_frame_timer(
            &state.state_service,
            session_id,
            mm_data::rdf::now_ms(),
        )
        .await;
    }
    state.status_service.mark_dirty_many(
        session_id,
        [
            crate::status_service::StatusSection::State,
            crate::status_service::StatusSection::Session,
            crate::status_service::StatusSection::Frames,
            crate::status_service::StatusSection::Prompt,
            crate::status_service::StatusSection::Operations,
            crate::status_service::StatusSection::Automation,
            crate::status_service::StatusSection::Data,
            crate::status_service::StatusSection::Plans,
            crate::status_service::StatusSection::Actions,
            crate::status_service::StatusSection::Agents,
        ],
    );
}

/// The event type taken from the frame's own tag, so the wire keeps one source
/// of truth for what a frame is.
fn frame_event(seq: u64, frame: ChatFrame) -> Event {
    let value = serde_json::to_value(&frame).unwrap_or(Value::Null);
    let event = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message")
        .to_owned();
    Event::default()
        .id(seq.to_string())
        .event(event)
        .data(value.to_string())
}

/// The frames a streamed turn adds after its passes and tools have already been
/// delivered live: non-text chunks, focus, the final reply, and the terminal
/// frame. Including pass or tool frames here would render every card twice.
async fn terminal_frames(
    state: &AppState,
    _request_id: &str,
    _query: &str,
    outcome: &mm_cme::TurnOutcome,
) -> Vec<ChatFrame> {
    let turn_id = outcome.turn_id.to_string();
    let mut frames = Vec::new();
    let verified = outcome
        .pass_history
        .iter()
        .rev()
        .find(|record| record.kind == mm_cme::PassKind::Verify)
        .map(|record| record.verification.iter().all(|result| result.pass));
    if !outcome.is_success() {
        frames.push(ChatFrame::Failed {
            error: "the conversation turn failed".into(),
            reason: outcome.error.clone(),
        });
        return frames;
    }
    for chunk in &outcome.chunks {
        if matches!(chunk.kind, ChunkKind::Text | ChunkKind::ToolCall) {
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
        let output = json_value(&call.result);
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
        verified,
    });
    let mut subsystems = vec!["cme".into(), "memory".into()];
    if !outcome.tool_calls.is_empty() {
        subsystems.push("tools".into());
    }
    frames.push(ChatFrame::Completed { subsystems });
    frames
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
    frames.extend(pass_frames(&turn_id, &outcome.text, &outcome.pass_history));
    let verified = outcome
        .pass_history
        .iter()
        .rev()
        .find(|record| record.kind == mm_cme::PassKind::Verify)
        .map(|record| record.verification.iter().all(|result| result.pass));
    if outcome.is_success() {
        for chunk in &outcome.chunks {
            if matches!(chunk.kind, ChunkKind::Text | ChunkKind::ToolCall) {
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
                pass: None,
                attempt: None,
                round: None,
                action: json_value(&call.arguments)
                    .get("action")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                parent_tool_id: None,
            });
            let output = json_value(&call.result);
            frames.push(ChatFrame::ToolResult {
                id: call.call_id.clone(),
                turn_id: turn_id.clone(),
                name: call.name.clone(),
                success: call.ok,
                output: output.clone(),
                duration_ms: 0,
                pass: None,
                attempt: None,
                round: None,
                parent_tool_id: None,
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
            verified,
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

/// The pass timeline frames for one turn. Every field comes from a recorded
/// pass, so the UI never has to infer a pass that did not run.
fn pass_frames(turn_id: &str, text: &str, pass_history: &[mm_cme::PassRecord]) -> Vec<ChatFrame> {
    let mut frames = Vec::new();
    for record in pass_history {
        frames.push(ChatFrame::PassStart {
            pass: record.kind.as_str().to_owned(),
            turn_id: turn_id.to_owned(),
            attempt: record.attempt,
            model: None,
            parent_tool_id: None,
        });
        if record.outcome == "fast_path" {
            frames.push(ChatFrame::PassToken {
                pass: record.kind.as_str().to_owned(),
                token: text.to_owned(),
                is_final: true,
                attempt: Some(record.attempt),
                round: None,
                channel: None,
                call_id: None,
                parent_tool_id: None,
            });
        }
        frames.push(ChatFrame::PassComplete {
            pass: record.kind.as_str().to_owned(),
            finished: record.outcome.clone(),
            budget: serde_json::to_value(&record.budget).unwrap_or(Value::Null),
            prompt_hash: record.prompt_hash.clone(),
            attempt: Some(record.attempt),
            parent_tool_id: None,
        });
        for result in &record.verification {
            frames.push(ChatFrame::Verification {
                label: result.check_label.clone(),
                pass: result.pass,
                confidence: result.confidence,
                attempt: Some(record.attempt),
                backend: Some(result.backend.clone()),
                duration_ms: Some(result.elapsed_ms),
                reasoning: Some(result.reasoning.clone()),
            });
        }
    }
    frames
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

#[cfg(test)]
mod tests {
    use super::*;
    use mm_cme::{PassBudget, PassKind, PassRecord, VerificationResult};

    fn budget() -> PassBudget {
        PassBudget {
            class: "quick".into(),
            rounds: 7,
            steps: 24,
            tokens: 31_000,
            wall_ms: 90_000,
            estimated: false,
        }
    }

    fn record(kind: PassKind, outcome: &str, hash: &str) -> PassRecord {
        PassRecord {
            kind,
            attempt: 1,
            prompt_hash: Some(hash.into()),
            budget: budget(),
            outcome: outcome.into(),
            verification: Vec::new(),
            staged: 0,
            committed: 0,
            checks: Vec::new(),
        }
    }

    #[test]
    fn the_pass_timeline_comes_only_from_recorded_passes() {
        let mut verify = record(PassKind::Verify, "approved", "ef01");
        verify.verification = vec![VerificationResult {
            check_label: "cites".into(),
            pass: true,
            reasoning: "a source is named".into(),
            confidence: 0.9,
            backend: "llm_judge".into(),
            elapsed_ms: 12,
            error: None,
        }];
        let frames = pass_frames(
            "t1",
            "reply",
            &[
                record(PassKind::Interpret, "action", "abcd"),
                record(PassKind::Process, "completed", "1234"),
                verify,
            ],
        );

        let starts = frames
            .iter()
            .filter(|frame| matches!(frame, ChatFrame::PassStart { .. }))
            .count();
        let completes = frames
            .iter()
            .filter(|frame| matches!(frame, ChatFrame::PassComplete { .. }))
            .count();
        let checks = frames
            .iter()
            .filter(|frame| matches!(frame, ChatFrame::Verification { .. }))
            .count();
        assert_eq!((starts, completes, checks), (3, 3, 1));
        assert!(!frames
            .iter()
            .any(|frame| matches!(frame, ChatFrame::PassToken { .. })));
        assert!(matches!(
            &frames[1],
            ChatFrame::PassComplete {
                pass,
                finished,
                prompt_hash,
                ..
            } if pass == "interpret"
                && finished == "action"
                && prompt_hash.as_deref() == Some("abcd")
        ));
        assert!(matches!(
            &frames[6],
            ChatFrame::Verification {
                label,
                pass: true,
                confidence,
                ..
            } if label == "cites" && (*confidence - 0.9).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn a_fast_path_emits_one_final_token() {
        let frames = pass_frames(
            "t1",
            "hello",
            &[record(PassKind::Interpret, "fast_path", "abcd")],
        );
        assert!(matches!(
            &frames[1],
            ChatFrame::PassToken {
                pass,
                token,
                is_final: true,
                ..
            } if pass == "interpret" && token == "hello"
        ));
        assert_eq!(
            frames
                .iter()
                .filter(|frame| matches!(frame, ChatFrame::PassStart { .. }))
                .count(),
            1
        );
    }
}
