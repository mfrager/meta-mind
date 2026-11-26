//! Agentstream snapshot and SSE routes.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::sse::{KeepAlive, Sse};
use axum::Json;
use futures::StreamExt;
use mm_agentstream::{Event, EventType};
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;

use crate::dto::{EventPage, SessionList};
use crate::error::AppError;
use crate::sse;
use crate::state::AppState;

/// The event page query.
#[derive(Debug, Clone, Deserialize)]
pub struct EventsQuery {
    /// First sequence to return (inclusive, default 1).
    #[serde(default)]
    pub from: Option<u64>,
    /// Maximum events (default 500, max 5000).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `GET /api/agentstream/sessions`.
pub async fn sessions(State(state): State<Arc<AppState>>) -> Result<Json<SessionList>, AppError> {
    let sessions = state.store.sessions()?;
    if sessions.is_empty() {
        // A brand-new server still has a valid, empty index.
        return Ok(Json(SessionList { sessions }));
    }
    Ok(Json(SessionList { sessions }))
}

/// `GET /api/agentstream/sessions/{id}/events`.
pub async fn events(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventPage>, AppError> {
    let from = query.from.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(500).clamp(1, 5000);
    if !state.store.session_exists(&session_id)? {
        return Err(AppError::NotFound(format!(
            "session not found: {session_id}"
        )));
    }
    let events = state.store.events(&session_id, from, Some(limit))?;
    let next = events
        .last()
        .map(|event| event.seq.saturating_add(1))
        .unwrap_or(from);
    Ok(Json(EventPage {
        session_id,
        from,
        next,
        events,
    }))
}

/// `GET /api/agentstream/sessions/{id}/stream?from=<seq>`.
///
/// Replays the persisted file from the cursor, then forwards live events. A live
/// event at or below the replay cursor is dropped.
pub async fn stream(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<
    Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, Infallible>>>,
    AppError,
> {
    let from = query.from.unwrap_or(1).max(1);
    if !state.store.session_exists(&session_id)? {
        return Err(AppError::NotFound(format!(
            "session not found: {session_id}"
        )));
    }
    // Subscribe before reading the replay so an append between the snapshot
    // read and the live tail is buffered, not dropped.
    let receiver = state.store.subscribe();
    let retained_start = state.store.retained_start(&session_id)?;
    let resync = from < retained_start;
    let replay_from = if resync { retained_start } else { from };
    let replay = state.store.events(&session_id, replay_from, None)?;
    let last = replay
        .last()
        .map(|event| event.seq)
        .unwrap_or_else(|| replay_from.saturating_sub(1));

    let mut frames: Vec<axum::response::sse::Event> = Vec::new();
    if resync {
        frames.push(sse::resync(&EventPage {
            session_id: session_id.clone(),
            from: retained_start,
            next: last.saturating_add(1),
            events: replay.clone(),
        }));
    } else {
        frames.extend(replay.iter().map(sse::frame));
    }

    let replay_stream = futures::stream::iter(frames.into_iter().map(Ok::<_, Infallible>));
    let live = BroadcastStream::new(receiver).filter_map(move |item| {
        let session_id = session_id.clone();
        async move {
            let event = item.ok()?;
            let forward = event.event_type == EventType::LlmDelta || event.seq > last;
            (event.session_id == session_id && forward).then_some(Ok::<Event, Infallible>(event))
        }
    });
    let stream = replay_stream.chain(live.map(|event| event.map(|event| sse::frame(&event))));
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
