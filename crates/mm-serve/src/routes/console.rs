//! Phase 10 inspector routes.
//!
//! Every route here is read-only except session create/resume/presence/end/delete,
//! action cancel, and agent cancel. The status view comes from the coalescing
//! [`crate::status_service::StatusService`]; the list/tree/log endpoints are
//! cursor-paginated and never scan an unbounded history into one response.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use mm_data::{
    AgentstreamStore, ConversationSessionFilter, DataBroker, DataQuery, DataSearchRequest,
    DeleteMode, LogFilter, PlanStore, SessionLifecycle,
};
use mm_prompt::PromptStore;

use crate::cme::CONSOLE_USER;
use crate::error::AppError;
use crate::state::AppState;
use crate::state_service::StateServiceError;
use crate::status::StatusView;
use crate::status_service::{frame_json, needs_resync, StatusSection};

#[derive(Debug, Deserialize, Default)]
pub struct CreateSessionBody {
    #[serde(default)]
    pub new: bool,
    #[serde(rename = "resumeSessionId", alias = "resume_session_id", default)]
    pub resume_session_id: Option<String>,
    #[serde(rename = "previousSessionId", alias = "previous_session_id", default)]
    pub previous_session_id: Option<String>,
    #[serde(rename = "userId", alias = "user_id", default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct SessionListQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub status: Option<String>,
    pub q: Option<String>,
    #[serde(default, rename = "includeDeleted")]
    pub include_deleted: bool,
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PageQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LogsQuery {
    pub source: Option<String>,
    pub mode: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub level: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct FileTreeQuery {
    pub path: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
pub struct EntityQuery {
    pub class: Option<String>,
    pub q: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    #[serde(default, rename = "includeDeleted")]
    pub include_deleted: bool,
}

#[derive(Debug, Deserialize, Default)]
pub struct FromQuery {
    pub from: Option<u64>,
    #[serde(rename = "types")]
    pub types: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AgentListQuery {
    #[serde(rename = "session_id")]
    pub session_id: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id_camel: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ActionListQuery {
    #[serde(rename = "session_id")]
    pub session_id: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id_camel: Option<String>,
}

/// Recursively expose struct field names as the camel-case wire vocabulary.
pub(crate) fn camelize(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (camel_key(&key), camelize(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(camelize).collect()),
        other => other,
    }
}

fn camel_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut upper = false;
    for ch in key.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

async fn session_view(state: &Arc<AppState>, session_id: &str) -> Result<Value, AppError> {
    state
        .status_service
        .mark_dirty(session_id, StatusSection::Session);
    let bundle = state
        .state_service
        .load(session_id)
        .await
        .map_err(state_error)?;
    let summary = state
        .state_service
        .summary(session_id)
        .await
        .map_err(state_error)?;
    Ok(camelize(json!({
        "sessionId": bundle.session.session_id,
        "session": bundle.session,
        "conversation": bundle.conversation,
        "frames": bundle.frames,
        "summary": summary,
    })))
}

/// `POST /api/console/sessions`.
pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionBody>,
) -> Result<Json<Value>, AppError> {
    let _ = body.new;
    let now = mm_data::rdf::now_ms();
    let user = body.user_id.as_deref().unwrap_or(CONSOLE_USER);
    let bundle = if let Some(session_id) = body.resume_session_id.as_deref() {
        let bundle = state
            .state_service
            .resume_session(session_id, now)
            .await
            .map_err(state_error)?;
        if let Some(automation) = &state.automation {
            automation.ensure(session_id).await;
        }
        bundle
    } else {
        let bundle = state
            .state_service
            .open_session(user, body.previous_session_id.as_deref(), now)
            .await
            .map_err(state_error)?;
        if let Some(automation) = &state.automation {
            automation.ensure(&bundle.session.session_id).await;
        }
        bundle
    };
    let session_id = bundle.session.session_id.clone();
    state.status_service.mark_dirty_many(
        &session_id,
        [
            StatusSection::State,
            StatusSection::Session,
            StatusSection::Frames,
        ],
    );
    state.status_service.flush(&session_id).await;
    let status = state.status_service.snapshot(&session_id).await;
    Ok(Json(camelize(json!({
        "sessionId": bundle.session.session_id,
        "session": bundle.session,
        "conversation": bundle.conversation,
        "frames": bundle.frames,
        "status": status,
    }))))
}

/// `GET /api/console/sessions`.
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SessionListQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let status = match query.status.as_deref() {
        None | Some("") | Some("all") => None,
        Some("active") => Some(SessionLifecycle::Active),
        Some("ending") => Some(SessionLifecycle::Ending),
        Some("ended") => Some(SessionLifecycle::Ended),
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "unknown session status {other:?}"
            )))
        }
    };
    let user = query.user_id.as_deref().unwrap_or(CONSOLE_USER);
    let page = state
        .state_service
        .list_sessions(
            user,
            query.cursor.as_deref(),
            limit,
            ConversationSessionFilter {
                status,
                query: query.q.clone(),
                include_deleted: query.include_deleted,
            },
        )
        .await
        .map_err(state_error)?;
    Ok(Json(camelize(json!(page))))
}

/// `GET /api/console/sessions/{id}`.
pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(session_view(&state, &session_id).await?))
}

/// `GET /api/console/sessions/{id}/traces`.
pub async fn traces(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Value>, AppError> {
    let page = state
        .state_service
        .list_traces(
            &session_id,
            query.cursor.as_deref(),
            query.limit.unwrap_or(20),
        )
        .await
        .map_err(state_error)?;
    Ok(Json(camelize(json!(page))))
}

/// `DELETE /api/console/sessions/{id}?mode=soft|purge`.
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<DeleteQuery>,
) -> Result<Json<Value>, AppError> {
    let bundle = state
        .state_service
        .load(&session_id)
        .await
        .map_err(state_error)?;
    if bundle.session.lifecycle == SessionLifecycle::Active {
        return Err(AppError::Conflict(
            "end the session before deleting it".into(),
        ));
    }
    let mode = match query.mode.as_deref() {
        None | Some("soft") => DeleteMode::Soft,
        Some("purge") => DeleteMode::Purge,
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "unknown delete mode {other:?}"
            )))
        }
    };
    if let Some(automation) = &state.automation {
        automation.stop(&session_id);
    }
    state
        .state_service
        .delete(&session_id, mode, CONSOLE_USER)
        .await
        .map_err(state_error)?;
    state
        .status_service
        .mark_dirty(&session_id, StatusSection::Session);
    Ok(Json(json!({
        "sessionId": session_id,
        "mode": if mode == DeleteMode::Purge { "purge" } else { "soft" },
        "deleted": true,
    })))
}

/// `POST /api/console/sessions/{id}/presence`.
pub async fn presence(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let now = mm_data::rdf::now_ms();
    let presence = state
        .state_service
        .heartbeat(&session_id, now)
        .await
        .map_err(state_error)?;
    state
        .status_service
        .mark_dirty(&session_id, StatusSection::Session);
    // Presence changes the operator's view of who is here; publish it before
    // the reply so a client does not have to wait for the coalescing timer.
    state.status_service.flush(&session_id).await;
    Ok(Json(json!({
        "sessionId": session_id,
        "presence": serde_json::to_value(presence).unwrap_or(Value::Null),
        "at": now,
    })))
}

/// `POST /api/console/sessions/{id}/resume`.
pub async fn resume_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let now = mm_data::rdf::now_ms();
    state
        .state_service
        .resume_session(&session_id, now)
        .await
        .map_err(state_error)?;
    if let Some(automation) = &state.automation {
        automation.ensure(&session_id).await;
    }
    state.status_service.mark_dirty_many(
        &session_id,
        [
            StatusSection::Session,
            StatusSection::Frames,
            StatusSection::State,
        ],
    );
    Ok(Json(session_view(&state, &session_id).await?))
}

/// `POST /api/console/sessions/{id}/end`.
pub async fn end_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let now = mm_data::rdf::now_ms();
    let session = state
        .state_service
        .end_session(&session_id, "user_closed", now)
        .await
        .map_err(state_error)?;
    if let Some(automation) = &state.automation {
        automation.stop(&session_id);
    }
    state
        .status_service
        .mark_dirty(&session_id, StatusSection::Session);
    Ok(Json(json!(session)))
}

/// `GET /api/console/sessions/{id}/state`.
pub async fn state(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let conversation = state
        .state_service
        .conversation(&session_id)
        .await
        .map_err(state_error)?;
    Ok(Json(camelize(
        serde_json::to_value(conversation).unwrap_or(Value::Null),
    )))
}

/// `GET /api/console/sessions/{id}/status`.
pub async fn status(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<StatusView>, AppError> {
    let view = state
        .status_service
        .snapshot(&session_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no session {session_id}")))?;
    Ok(Json(view))
}

/// `GET /api/console/sessions/{id}/status/stream?from=<revision>`.
pub async fn status_stream(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<FromQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let from = query.from.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
    });
    let initial = state
        .status_service
        .snapshot(&session_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no session {session_id}")))?;
    let history = state.status_service.history(&session_id);
    let reason = if needs_resync(&history, from) {
        "resync"
    } else {
        "full"
    };
    let receiver = state.status_service.subscribe();
    let first = Some((initial, reason.to_owned()));
    let stream_session = session_id.clone();
    let stream = stream::unfold((receiver, first), move |(mut receiver, first)| {
        let session_id = stream_session.clone();
        async move {
            if let Some((view, reason)) = first {
                let payload = frame_json(&view, &reason);
                let frame = Event::default()
                    .event("status")
                    .id(view.revision.to_string())
                    .data(payload.to_string());
                return Some((Ok(frame), (receiver, None)));
            }
            loop {
                match receiver.recv().await {
                    Ok(event) if event.session_id == session_id => {
                        let payload = frame_json(&event.view, &event.reason);
                        let frame = Event::default()
                            .event("status")
                            .id(event.view.revision.to_string())
                            .data(payload.to_string());
                        return Some((Ok(frame), (receiver, None)));
                    }
                    // The broadcast channel is process-wide; never leak another
                    // session's inspector view into this stream.
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // The client fell behind the 64-frame broadcast buffer.
                        // Close the stream; its reconnect starts with a full
                        // snapshot, which is the only safe recovery.
                        return None;
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// `GET /api/console/sessions/{id}/operations`.
pub async fn operations(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let events = state
        .state_service
        .agentstream()
        .events(&session_id)
        .await
        .map_err(|error| AppError::Internal(format!("agentstream: {error}")))?;
    let start = query
        .cursor
        .as_deref()
        .and_then(|cursor| cursor.parse::<usize>().ok())
        .unwrap_or(0)
        .min(events.len());
    let rows = events
        .iter()
        .skip(start)
        .take(limit)
        .map(|event| {
            json!({
                "id": event.event_id,
                "kind": event.event_type.as_str(),
                "source": event.provenance.source,
                "turnId": event.turn_id,
                "at": event.timestamp,
                "payload": event.payload,
            })
        })
        .collect::<Vec<_>>();
    let next = start.saturating_add(rows.len());
    Ok(Json(json!({
        "items": rows,
        "nextCursor": (next < events.len()).then(|| next.to_string()),
        "hasMore": next < events.len(),
    })))
}

/// `GET /api/console/sessions/{id}/logs`.
pub async fn logs(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(50);
    let filter = LogFilter {
        level: query.level.as_deref().and_then(mm_data::LogLevel::parse),
        query: query.q.clone(),
        ..LogFilter::default()
    };
    let logs = mm_data::RdfLogService::new(Arc::clone(&state.rdf_engine));
    let source = query.source.as_deref().unwrap_or("log");
    if source == "agentstream" || source == "all" {
        let events = state
            .state_service
            .agentstream()
            .events(&session_id)
            .await
            .map_err(|error| AppError::Internal(format!("agentstream: {error}")))?;
        let rows = events
            .iter()
            .rev()
            .take(limit.clamp(1, 100))
            .map(|event| {
                json!({
                    "id": event.event_id,
                    "sessionId": event.session_id,
                    "traceId": event.turn_id,
                    "level": "info",
                    "kind": event.event_type.as_str(),
                    "message": event.payload.to_string(),
                    "tags": [],
                    "refs": [],
                    "at": event.timestamp,
                    "source": event.provenance.source,
                })
            })
            .collect::<Vec<_>>();
        return Ok(Json(json!({
            "entries": rows,
            "nextCursor": Value::Null,
            "hasMore": false,
            "source": source,
        })));
    }
    if query.mode.as_deref() == Some("latest") {
        let entries = logs
            .latest(&session_id, &filter, limit)
            .await
            .map_err(|error| AppError::Internal(format!("log: {error}")))?;
        return Ok(Json(json!({
            "entries": entries,
            "nextCursor": Value::Null,
            "hasMore": false,
            "source": "log",
        })));
    }
    let page = logs
        .segment(&session_id, &filter, query.cursor.as_deref(), limit)
        .await
        .map_err(|error| AppError::Internal(format!("log: {error}")))?;
    Ok(Json(camelize(json!(page))))
}

/// `GET /api/console/files/tree`.
pub async fn file_tree(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FileTreeQuery>,
) -> Result<Json<Value>, AppError> {
    let path = query.path.as_deref().unwrap_or("/");
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    if path == "/" || path.is_empty() {
        return Ok(Json(json!({
            "path": "/",
            "entries": [
                {"name":"sessions","kind":"dir","path":"/sessions","size":Value::Null},
                {"name":"traces","kind":"dir","path":"/traces","size":Value::Null},
                {"name":"logs","kind":"dir","path":"/logs","size":Value::Null},
            ],
            "nextCursor": Value::Null,
            "hasMore": false,
        })));
    }
    if path == "/logs" || path == "/traces" {
        return Ok(Json(json!({
            "path": path,
            "entries": [],
            "nextCursor": Value::Null,
            "hasMore": false,
        })));
    }
    if path == "/sessions" {
        let page = state
            .state_service
            .list_sessions(
                "",
                query.cursor.as_deref(),
                limit,
                ConversationSessionFilter {
                    status: None,
                    query: None,
                    include_deleted: false,
                },
            )
            .await
            .map_err(state_error)?;
        let entries = page
            .items
            .iter()
            .map(|session| {
                json!({
                    "name": session.session_id,
                    "kind": "dir",
                    "path": format!("/sessions/{}", session.session_id),
                    "size": Value::Null,
                })
            })
            .collect::<Vec<_>>();
        return Ok(Json(json!({
            "path": "/sessions",
            "entries": entries,
            "nextCursor": page.next_cursor,
            "hasMore": page.has_more,
        })));
    }
    let Some((session_id, rest)) = session_path(path) else {
        return Err(AppError::NotFound(format!("no tree path {path}")));
    };
    state
        .state_service
        .load(&session_id)
        .await
        .map_err(state_error)?;
    if rest.is_empty() {
        return Ok(Json(json!({
            "path": path,
            "entries": [
                {"name":"traces","kind":"dir","path":format!("/sessions/{session_id}/traces"),"size":Value::Null},
                {"name":"logs","kind":"dir","path":format!("/sessions/{session_id}/logs"),"size":Value::Null},
                {"name":"files","kind":"dir","path":format!("/sessions/{session_id}/files"),"size":Value::Null},
            ],
            "nextCursor": Value::Null,
            "hasMore": false,
        })));
    }
    if rest == "traces" {
        let page = state
            .state_service
            .list_traces(&session_id, query.cursor.as_deref(), limit)
            .await
            .map_err(state_error)?;
        let entries = page
            .items
            .iter()
            .map(|trace| {
                json!({
                    "name": trace.trace_id,
                    "kind": "trace",
                    "path": format!("/sessions/{session_id}/traces/{}", trace.trace_id),
                    "size": trace.size_bytes,
                    "agentstreamId": trace.agentstream_id,
                    "eventCount": trace.event_count,
                })
            })
            .collect::<Vec<_>>();
        return Ok(Json(json!({
            "path": path,
            "entries": entries,
            "nextCursor": page.next_cursor,
            "hasMore": page.has_more,
        })));
    }
    if rest == "logs" {
        let logs = mm_data::RdfLogService::new(Arc::clone(&state.rdf_engine));
        let page = logs
            .segment(
                &session_id,
                &LogFilter::default(),
                query.cursor.as_deref(),
                limit,
            )
            .await
            .map_err(|error| AppError::Internal(format!("log: {error}")))?;
        let entries = page
            .entries
            .iter()
            .map(|entry| {
                json!({
                    "name": format!("{:08}-{}.log", entry.sequence, entry.kind),
                    "kind": "log",
                    "path": format!("/sessions/{session_id}/logs/{}", entry.id),
                    "size": entry.message.len(),
                    "at": entry.at,
                })
            })
            .collect::<Vec<_>>();
        return Ok(Json(json!({
            "path": path,
            "entries": entries,
            "nextCursor": page.next_cursor,
            "hasMore": page.has_more,
        })));
    }
    list_session_files(&state, &session_id, &rest, query.cursor.as_deref(), limit)
}

fn session_path(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix("/sessions/")?;
    let (session, tail) = rest.split_once('/').unwrap_or((rest, ""));
    if session.is_empty()
        || session
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
    {
        return None;
    }
    Some((session.to_owned(), tail.trim_matches('/').to_owned()))
}

fn list_session_files(
    state: &Arc<AppState>,
    session_id: &str,
    rest: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Json<Value>, AppError> {
    let Some(relative) = rest.strip_prefix("files") else {
        return Err(AppError::NotFound(format!("no tree path {rest}")));
    };
    let relative = relative.trim_matches('/');
    if relative
        .split('/')
        .filter(|part| !part.is_empty())
        .any(|part| part == ".." || part == ".")
    {
        return Err(AppError::BadRequest("tree path escapes the session".into()));
    }
    let root = PathBuf::from(&state.cfg.store.data_dir)
        .join("sessions")
        .join(session_id)
        .join("files");
    let dir = if relative.is_empty() {
        root.clone()
    } else {
        root.join(relative)
    };
    let mut entries = Vec::new();
    if dir.exists() {
        let canonical_root = std::fs::canonicalize(&root)
            .map_err(|error| AppError::Internal(format!("session files: {error}")))?;
        let canonical_dir = std::fs::canonicalize(&dir)
            .map_err(|error| AppError::Internal(format!("session files: {error}")))?;
        if !canonical_dir.starts_with(&canonical_root) {
            return Err(AppError::BadRequest("tree path escapes the session".into()));
        }
    }
    if dir.is_dir() {
        let mut rows = std::fs::read_dir(&dir)
            .map_err(|error| AppError::Internal(format!("session files: {error}")))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        rows.sort_by_key(|entry| entry.file_name());
        let start = cursor
            .and_then(|cursor| cursor.parse::<usize>().ok())
            .unwrap_or(0)
            .min(rows.len());
        for entry in rows.iter().skip(start).take(limit) {
            let metadata = entry.metadata().ok();
            let name = entry.file_name().to_string_lossy().to_string();
            let child = if relative.is_empty() {
                name.clone()
            } else {
                format!("{}/{name}", relative.trim_end_matches('/'))
            };
            entries.push(json!({
                "name": name,
                "kind": if entry.path().is_dir() { "dir" } else { "file" },
                "path": format!("/sessions/{session_id}/files/{child}"),
                "size": metadata.as_ref().map(|value| value.len()),
            }));
        }
        let next = start.saturating_add(entries.len());
        return Ok(Json(json!({
            "path": format!("/sessions/{session_id}/files/{relative}"),
            "entries": entries,
            "nextCursor": (next < rows.len()).then(|| next.to_string()),
            "hasMore": next < rows.len(),
        })));
    }
    Ok(Json(json!({
        "path": format!("/sessions/{session_id}/files/{relative}"),
        "entries": [],
        "nextCursor": Value::Null,
        "hasMore": false,
    })))
}

/// `GET /api/console/data/health`.
pub async fn data_health(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let status = state
        .data
        .data_service
        .status()
        .await
        .map_err(|error| AppError::Internal(format!("data: {error}")))?;
    let schemas = state
        .data
        .data_service
        .list_schemas()
        .await
        .map_err(|error| AppError::Internal(format!("schemas: {error}")))?;
    let namespaces = state
        .data
        .data_service
        .list_namespaces()
        .await
        .map_err(|error| AppError::Internal(format!("namespaces: {error}")))?;
    Ok(Json(json!({
        "status": status,
        "health": state.data.health,
        "schemas": schemas.len(),
        "namespaces": namespaces.len(),
    })))
}

/// `GET /api/console/data/entities`.
pub async fn data_entities(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EntityQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    if let Some(needle) = query.q.as_deref().filter(|value| !value.trim().is_empty()) {
        let result = state
            .data
            .data_service
            .search(DataSearchRequest {
                query: needle.to_owned(),
                class: query.class.clone(),
                graph: None,
                namespace: None,
                limit,
                snippet: true,
                include_deleted: query.include_deleted,
            })
            .await
            .map_err(|error| AppError::Internal(format!("search: {error}")))?;
        return Ok(Json(json!({
            "items": result.hits,
            "index": result.index,
            "hasMore": result.hits.len() >= limit,
            "nextOffset": Value::Null,
        })));
    }
    let result = state
        .data
        .data_service
        .query(DataQuery {
            class: query.class.clone(),
            limit,
            offset: query.offset.unwrap_or(0),
            include_deleted: query.include_deleted,
            ..DataQuery::default()
        })
        .await
        .map_err(|error| AppError::Internal(format!("data query: {error}")))?;
    Ok(Json(json!({
        "items": result.entities,
        "hasMore": result.has_more,
        "nextOffset": result.next_offset,
    })))
}

/// `GET /api/console/data/schemas`.
pub async fn data_schemas(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let schemas = state
        .data
        .data_service
        .list_schemas()
        .await
        .map_err(|error| AppError::Internal(format!("schemas: {error}")))?;
    Ok(Json(json!({ "items": schemas })))
}

/// `GET /api/console/data/namespaces`.
pub async fn data_namespaces(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let namespaces = state
        .data
        .data_service
        .list_namespaces()
        .await
        .map_err(|error| AppError::Internal(format!("namespaces: {error}")))?;
    Ok(Json(json!({ "items": namespaces })))
}

/// `GET /api/console/plans`.
pub async fn plans(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let registry = state
        .plans
        .registry()
        .await
        .map_err(|error| AppError::Internal(format!("plan registry: {error}")))?;
    let mut items = Vec::new();
    for plan_id in &registry.plans {
        if let Some(plan) = state
            .plans
            .load(plan_id)
            .await
            .map_err(|error| AppError::Internal(format!("plan {plan_id}: {error}")))?
        {
            items.push(plan);
        }
    }
    Ok(Json(camelize(json!({
        "revision": registry.revision,
        "items": items,
        "monitor": state.plan_monitor.is_some(),
    }))))
}

/// `GET /api/console/plans/{id}`.
/// Wildcard form for plan IRIs that contain `/`.
pub async fn plan_detail(
    State(state): State<Arc<AppState>>,
    Path(plan_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    plan_detail_id(&state, &plan_id).await
}

async fn plan_detail_id(state: &Arc<AppState>, plan_id: &str) -> Result<Json<Value>, AppError> {
    let plan = state
        .plans
        .load(plan_id)
        .await
        .map_err(|error| AppError::Internal(format!("plan: {error}")))?
        .ok_or_else(|| AppError::NotFound(format!("no plan {plan_id}")))?;
    let versions = state
        .plans
        .versions(plan_id)
        .await
        .map_err(|error| AppError::Internal(format!("plan versions: {error}")))?;
    Ok(Json(camelize(json!({
        "plan": plan,
        "versions": versions,
        "monitor": state.plan_monitor.is_some(),
    }))))
}

/// `GET /api/console/actions`.
pub async fn actions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ActionListQuery>,
) -> Result<Json<Value>, AppError> {
    let requested = query.session_id.or(query.session_id_camel);
    let session_ids = if let Some(session_id) = requested.clone() {
        vec![session_id]
    } else {
        state
            .state_service
            .list_sessions(
                "",
                None,
                100,
                ConversationSessionFilter {
                    status: None,
                    query: None,
                    include_deleted: false,
                },
            )
            .await
            .map_err(state_error)?
            .items
            .into_iter()
            .map(|session| session.session_id)
            .collect()
    };
    let mut actions = Vec::new();
    for session_id in &session_ids {
        actions.extend(
            state
                .actions
                .list(session_id)
                .await
                .map_err(|error| AppError::Internal(format!("actions: {error}")))?,
        );
    }
    let mut scheduled = Vec::new();
    let mut recent = Vec::new();
    for action in actions {
        let row = json!({
            "actionId": action.id,
            "kind": action.spec.kind,
            "status": format!("{:?}", action.spec.status).to_lowercase(),
            "dueAt": action.next_run_at,
            "lastRunAt": action.spec.last_run_at,
            "runs": action.spec.runs,
            "schedule": action.spec.schedule,
        });
        if matches!(
            action.spec.status,
            mm_data::ScheduledActionStatus::Scheduled
                | mm_data::ScheduledActionStatus::Claimed
                | mm_data::ScheduledActionStatus::Running
        ) {
            scheduled.push(row);
        } else {
            recent.push(row);
        }
    }
    let next_run_at = scheduled
        .iter()
        .filter_map(|row| row.get("dueAt").and_then(Value::as_i64))
        .min();
    Ok(Json(json!({
        "sessionId": requested,
        "sessions": session_ids,
        "scheduled": scheduled,
        "recent": recent,
        "nextRunAt": next_run_at,
    })))
}

/// `POST /api/console/actions/{id}/cancel`.
/// Wildcard form for action IRIs that contain `/` (for example
/// `mmd:action/<ulid>`).
pub async fn cancel_action(
    State(state): State<Arc<AppState>>,
    Path(rest): Path<String>,
) -> Result<Json<Value>, AppError> {
    let action_id = rest.strip_suffix("/cancel").unwrap_or(&rest);
    cancel_action_id(&state, action_id).await
}

async fn cancel_action_id(state: &Arc<AppState>, action_id: &str) -> Result<Json<Value>, AppError> {
    let cancelled = state
        .actions
        .cancel(action_id)
        .await
        .map_err(|error| AppError::Internal(format!("actions: {error}")))?;
    if !cancelled {
        return Err(AppError::NotFound(format!("no action {action_id}")));
    }
    Ok(Json(json!({
        "actionId": action_id,
        "status": "cancelled",
        "cancelled": true,
    })))
}

/// `GET /api/console/automation`.
pub async fn automation(State(state): State<Arc<AppState>>) -> Json<Value> {
    let sessions = state
        .automation
        .as_ref()
        .map_or_else(Vec::new, |manager| manager.session_ids());
    let mut frame_timers = 0usize;
    let mut next_action_at: Option<i64> = None;
    for session_id in &sessions {
        if let Ok(bundle) = state.state_service.load(session_id).await {
            if bundle.session.pending_utterance.is_some() {
                frame_timers += 1;
            }
        }
        if let Ok(actions) = state.actions.list(session_id).await {
            let next = actions.iter().filter_map(|action| action.next_run_at).min();
            next_action_at = match (next_action_at, next) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (None, Some(right)) => Some(right),
                (left, None) => left,
            };
        }
    }
    let config = state.automation.as_ref().map(|manager| {
        json!({
            "enabled": manager.config().supervisor.enabled,
            "tickSecs": manager.config().supervisor.tick_secs,
            "maxSessions": manager.config().supervisor.max_sessions,
            "actionsEnabled": manager.config().actions.enabled,
            "framesEnabled": manager.config().frames.enabled,
        })
    });
    Json(json!({
        "enabled": state.automation.is_some(),
        "supervisor": if sessions.is_empty() { "idle" } else { "running" },
        "sessions": sessions,
        "frameTimers": frame_timers,
        "nextActionAt": next_action_at,
        "config": config,
        "planMonitor": state.plan_monitor.is_some(),
        "internalActions": {
            "disabled": std::env::var("MM_INTERNAL_ACTIONS_DISABLED").unwrap_or_default(),
            "maxCronJobs": std::env::var("MM_AUTOMATION_MAX_CRON_JOBS").unwrap_or_else(|_| "20".into()),
        },
    }))
}

/// `GET /api/console/sessions/{id}/frames`.
pub async fn frames(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let bundle = state
        .state_service
        .load(&session_id)
        .await
        .map_err(state_error)?;
    Ok(Json(camelize(json!({
        "frames": bundle.frames.frames,
        "revision": bundle.frames.revision,
        "top": bundle.frames.top(),
        "pendingUtterance": bundle.session.pending_utterance,
    }))))
}

/// `GET /api/console/sessions/{id}/interjections`.
pub async fn session_interjections(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let view = state
        .status_service
        .snapshot(&session_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no session {session_id}")))?;
    Ok(Json(json!(view.interjections)))
}

/// `GET /api/console/interjections`.
pub async fn interjections() -> Json<Value> {
    Json(json!({
        "policyId": "global.default",
        "policies": [
            "global.default", "internal.default", "scheduled.default",
            "agent.result", "frame.utterance", "plan.monitor", "introspection.default"
        ],
    }))
}

/// `GET /api/console/prompts`.
pub async fn prompts(State(state): State<Arc<AppState>>) -> Json<Value> {
    let prompts: &Arc<PromptStore> = state.state_service.prompts();
    let strategies = prompts
        .strategies()
        .map(|strategy| {
            json!({
                "id": strategy.id,
                "version": strategy.version,
                "application": strategy.application,
                "budget": strategy.budget,
                "variants": strategy.variants,
                "fragments": strategy.fragments,
            })
        })
        .collect::<Vec<_>>();
    let frames = prompts
        .frames()
        .map(|frame| {
            json!({
                "id": frame.id,
                "version": frame.version,
                "kind": frame.kind,
                "step": frame.step,
                "strategy": frame.strategy,
            })
        })
        .collect::<Vec<_>>();
    Json(json!({
        "manifest": prompts.manifest(),
        "frameManifest": prompts.frame_manifest(),
        "strategies": strategies,
        "frames": frames,
    }))
}

/// `GET /api/agents`.
pub async fn agents(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AgentListQuery>,
) -> Json<Value> {
    let session = query.session_id.or(query.session_id_camel);
    let runs = if let Some(session) = session {
        state.agent.runs_for_session(&session).await
    } else {
        state.agent.runs().await
    };
    Json(json!({
        "items": runs,
        "active": runs.iter().filter(|run| !run.status.is_terminal()).count(),
    }))
}

/// `GET /api/agents/{run_id}`.
pub async fn agent_detail(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let run = state
        .agent
        .status(&run_id)
        .await
        .map_err(|error| AppError::NotFound(error.to_string()))?;
    Ok(Json(json!(run)))
}

/// `GET /api/agents/{run_id}/events`.
pub async fn agent_events(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Query(query): Query<FromQuery>,
) -> Result<Json<Value>, AppError> {
    let run = state
        .agent
        .status(&run_id)
        .await
        .map_err(|error| AppError::NotFound(error.to_string()))?;
    let events = state
        .agentstream
        .events(&run.agentstream_id)
        .await
        .map_err(|error| AppError::Internal(format!("agentstream: {error}")))?;
    let types = query.types.as_deref().map(|types| {
        types
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
    });
    let start = query.from.unwrap_or(1).saturating_sub(1) as usize;
    let rows = events
        .iter()
        .filter(|event| {
            types
                .as_ref()
                .is_none_or(|types| types.iter().any(|kind| *kind == event.event_type.as_str()))
        })
        .skip(start)
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "runId": run_id,
        "items": rows,
        "next": start.saturating_add(rows.len()).saturating_add(1),
    })))
}

/// `GET /api/agents/{run_id}/stream`.
pub async fn agent_stream(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Query(query): Query<FromQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let run = state
        .agent
        .status(&run_id)
        .await
        .map_err(|error| AppError::NotFound(error.to_string()))?;
    let store = Arc::clone(&state.agentstream);
    let start = query.from.unwrap_or(1).saturating_sub(1) as usize;
    let stream = stream::unfold(
        (store, run.agentstream_id, start),
        |(store, agentstream_id, next)| async move {
            loop {
                match store.events(&agentstream_id).await {
                    Ok(events) if events.len() > next => {
                        let event = &events[next];
                        let data = serde_json::to_string(event).unwrap_or_else(|_| "{}".into());
                        let frame = Event::default()
                            .event(event.event_type.as_str())
                            .id((next + 1).to_string())
                            .data(data);
                        return Some((Ok(frame), (store, agentstream_id, next + 1)));
                    }
                    Ok(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
                    Err(_) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
                }
            }
        },
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// `POST /api/agents/{run_id}/cancel`.
pub async fn agent_cancel(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let run = state
        .agent
        .cancel(&run_id)
        .await
        .map_err(|error| AppError::NotFound(error.to_string()))?;
    state
        .status_service
        .mark_dirty(&run.session_id, StatusSection::Agents);
    Ok(Json(json!(run)))
}

/// `GET /api/agentstream/sessions`.
pub async fn agentstream_sessions(State(state): State<Arc<AppState>>) -> Json<Value> {
    match state
        .agentstream
        .sessions(mm_data::SessionFilter {
            limit: Some(100),
            ..Default::default()
        })
        .await
    {
        Ok(items) => Json(json!({ "items": items })),
        Err(error) => Json(json!({
            "items": [],
            "unavailable": error.to_string(),
        })),
    }
}

/// `GET /api/agentstream/sessions/{id}/events`.
pub async fn agentstream_events(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<FromQuery>,
) -> Result<Json<Value>, AppError> {
    let events = state
        .agentstream
        .events(&session_id)
        .await
        .map_err(|error| AppError::Internal(format!("agentstream: {error}")))?;
    let start = query.from.unwrap_or(1).saturating_sub(1) as usize;
    let rows = events.iter().skip(start).collect::<Vec<_>>();
    Ok(Json(json!({
        "sessionId": session_id,
        "items": rows,
        "next": start.saturating_add(rows.len()).saturating_add(1),
    })))
}

/// `GET /api/agentstream/sessions/{id}/stream`.
pub async fn agentstream_stream(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<FromQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let store: Arc<AgentstreamStore> = Arc::clone(&state.agentstream);
    let start = query.from.unwrap_or(1).saturating_sub(1) as usize;
    let stream = stream::unfold(
        (store, session_id, start),
        |(store, session_id, next)| async move {
            loop {
                match store.events(&session_id).await {
                    Ok(events) if events.len() > next => {
                        let event = &events[next];
                        let data = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_owned());
                        let frame = Event::default()
                            .event(event.event_type.as_str())
                            .id((next + 1).to_string())
                            .data(data);
                        return Some((Ok(frame), (store, session_id, next + 1)));
                    }
                    Ok(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
                    Err(_) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
                }
            }
        },
    );
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// `GET /api/metrics`.
pub async fn metrics(State(state): State<Arc<AppState>>) -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.metrics.prometheus(),
    )
        .into_response()
}

/// `GET /api/status` — the first cached session, a convenience for operators.
pub async fn latest_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<StatusView>, AppError> {
    let sessions = state.status_service.cached_sessions();
    let Some(session_id) = sessions.first() else {
        return Err(AppError::NotFound("no status snapshot yet".into()));
    };
    let view = state
        .status_service
        .snapshot(session_id)
        .await
        .ok_or_else(|| AppError::NotFound("no status snapshot yet".into()))?;
    Ok(Json(view))
}

fn state_error(error: StateServiceError) -> AppError {
    match error {
        StateServiceError::Missing(id) => AppError::NotFound(format!("no session {id}")),
        StateServiceError::Deleted(id) => AppError::Conflict(format!("session {id} is deleted")),
        StateServiceError::NotOpen(lifecycle) => {
            AppError::Conflict(format!("session is {lifecycle}"))
        }
        other => AppError::Internal(other.to_string()),
    }
}
