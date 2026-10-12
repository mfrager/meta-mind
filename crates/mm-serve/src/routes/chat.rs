//! `/api/chat` runs and the input-answer lifecycle (plan `09` §2–3).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::chat::{self, validate_attachments};
use crate::docs::RUN_ID;
use crate::dto::chat::ChatFrame;
use crate::error::AppError;
use crate::sse::{self, SseFrame};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct OpenChat {
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    pub query: String,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<Value>,
}

#[derive(Deserialize)]
pub struct AnswerBody {
    pub id: String,
    pub answer: Value,
}

#[derive(Deserialize)]
pub struct FromQuery {
    pub from: Option<u64>,
}

fn focus_for(state: &AppState, op: &str) -> Option<Value> {
    if op != "meta analyze" {
        return None;
    }
    state
        .focus
        .get(RUN_ID)
        .and_then(|doc| serde_json::to_value(doc).ok())
}

pub async fn list_sessions(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "sessions": state.chat.list_sessions()? })))
}

pub async fn create_session(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let id = state.ids.next_string();
    let (id, title) = state.chat.create_session(id)?;
    Ok(Json(json!({ "id": id, "title": title })))
}

pub async fn open(
    State(state): State<Arc<AppState>>,
    Json(body): Json<OpenChat>,
) -> Result<Response, AppError> {
    if body.query.trim().is_empty() {
        return Err(AppError::BadRequest("a query is required".into()));
    }
    validate_attachments(&body.attachments)?;
    let session_id = match body.session_id {
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
    let op = chat::classify(&body.query);
    let (success, output) = chat::execute(&state, op, &body.query).await;
    let cancelled = state.chat.is_cancelled(&request_id)?;
    let frames = chat::plan_frames(
        &request_id,
        &session_id,
        &body.query,
        op,
        success,
        output,
        cancelled,
        body.attachments.len(),
        &body.refs,
        focus_for(&state, op),
    );
    let stored = state.chat.finish(
        &request_id,
        frames,
        if cancelled { "cancelled" } else { "completed" },
    )?;
    Ok(sse_response(&stored)?.into_response())
}

pub async fn stream(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    Query(query): Query<FromQuery>,
) -> Result<Response, AppError> {
    let from = sse::resume_point(query.from);
    let frames = state.chat.frames_from(&request_id, from)?;
    Ok(sse_response(&frames)?.into_response())
}

pub async fn stop(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(state.chat.stop(&request_id)?))
}

pub async fn regenerate(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
) -> Result<Response, AppError> {
    let (session_id, query) = state.chat.query_of(&request_id)?;
    open(
        State(state),
        Json(OpenChat {
            session_id: Some(session_id),
            query,
            refs: Vec::new(),
            attachments: Vec::new(),
        }),
    )
    .await
}

pub async fn answer(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(body): Json<AnswerBody>,
) -> Result<Json<Value>, AppError> {
    let frames = state.chat.answer(&session_id, &body.id, &body.answer)?;
    let new_frames: Vec<Value> = frames
        .into_iter()
        .rev()
        .take(2)
        .map(|(seq, frame)| json!({ "seq": seq, "frame": frame }))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Ok(Json(json!({ "frames": new_frames })))
}

pub async fn branch(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let new_session = state.ids.next_string();
    let new_request = state.ids.next_string();
    state
        .chat
        .branch(&session_id, new_session.clone(), new_request.clone())?;
    Ok(Json(json!({
        "sessionId": new_session,
        "requestId": new_request
    })))
}

fn sse_response(frames: &[(u64, ChatFrame)]) -> Result<Response, AppError> {
    let mut body = String::new();
    for (seq, frame) in frames {
        let value =
            serde_json::to_value(frame).map_err(|err| AppError::Internal(err.to_string()))?;
        let event = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message");
        let encoded = SseFrame::json(*seq, event, &value)
            .map_err(|err| AppError::Internal(err.to_string()))?;
        body.push_str(&encoded.encode());
    }
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(axum::body::Body::from(body))
        .map_err(|err| AppError::Internal(err.to_string()))?)
}
