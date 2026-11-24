//! `GET /api/llm-activity` and its live SSE tail.
//!
//! The snapshot endpoint hydrates the dock; the stream endpoint tails new
//! activity and replays the retained ring after a reconnect. Both use the
//! feed's own sequence, so a client dedupes and resumes exactly as it does for
//! chat and status. Nothing here persists: the feed is a bounded in-memory
//! projection.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::stream::{self, Stream};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::dto::llm_activity::{LlmActivityEvent, LlmActivityResync, LlmActivitySnapshot};
use crate::error::AppError;
use crate::llm_feed::LlmActivityFeed;
use crate::state::AppState;

/// Query for the snapshot endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct ActivityQuery {
    /// Return events at or after this cursor.
    pub from: Option<u64>,
    /// Maximum events to return (clamped to 1..=500).
    pub limit: Option<usize>,
    /// `all` (default), `request`, `delta`, `response`, or `error`.
    pub kind: Option<String>,
    /// Only events from this conversation session.
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
}

/// A `Last-Event-ID` header supplies the cursor when `?from=` is absent.
fn cursor(headers: &HeaderMap, from: Option<u64>) -> Option<u64> {
    from.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
    })
}

fn feed(state: &AppState) -> Result<&Arc<LlmActivityFeed>, AppError> {
    state
        .llm_activity
        .as_ref()
        .ok_or_else(|| AppError::Unavailable("LLM activity is disabled by MM_LLM_ACTIVITY".into()))
}

fn kind_allowed(kind: &str) -> bool {
    matches!(kind, "all" | "request" | "delta" | "response" | "error")
}

/// `GET /api/llm-activity?from=&limit=&kind=&sessionId=`.
pub async fn snapshot(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<LlmActivitySnapshot>, AppError> {
    let feed = feed(&state)?;
    let limit = query.limit.unwrap_or(200).clamp(1, 500);
    let kind = query.kind.as_deref().unwrap_or("all");
    if !kind_allowed(kind) {
        return Err(AppError::BadRequest(format!(
            "unknown activity kind {kind:?}; expected all, request, delta, response, or error"
        )));
    }

    // Filter the whole retained ring first so `limit` counts matching rows.
    let mut snapshot = feed.snapshot(query.from, feed.capacity());
    if let Some(session_id) = query.session_id.as_deref() {
        // Only the request carries the context; the rest of a call's events are
        // selected by call id so deltas and outcomes survive the filter. This
        // runs before the kind filter, which would otherwise drop the request
        // that names the session.
        let calls: std::collections::HashSet<String> = snapshot
            .events
            .iter()
            .filter(|event| event.session_id() == Some(session_id))
            .map(|event| event.call_id().to_string())
            .collect();
        snapshot
            .events
            .retain(|event| calls.contains(event.call_id()));
    }
    if kind != "all" {
        snapshot.events.retain(|event| event.kind() == kind);
    }
    if snapshot.events.len() > limit {
        let first = snapshot.events.len() - limit;
        snapshot.events.drain(..first);
    }
    Ok(Json(snapshot))
}

/// The initial replay a stream owes its cursor.
///
/// `None` means live-only. A cursor below `oldest_seq` lost rows while the
/// client was away, so the stream must start with a `resync` payload.
fn replay(feed: &LlmActivityFeed, cursor: Option<u64>) -> (LlmActivitySnapshot, bool) {
    match cursor {
        None => (feed.snapshot(None, 0), false),
        Some(from) => {
            let snapshot = feed.snapshot(Some(from), usize::MAX);
            let resync = from < snapshot.oldest_seq;
            (snapshot, resync)
        }
    }
}

/// `GET /api/llm-activity/stream?from=<seq>`.
pub async fn stream(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ActivityQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let feed = Arc::clone(feed(&state)?);
    // Subscribe before taking the cursor so nothing published from here on can
    // be missed. A live-only tail starts at zero; the snapshot endpoint is what
    // supplies history to a fresh client.
    let receiver = feed.subscribe();
    let live_cursor = cursor(&headers, query.from);
    // A browser that survived a backend restart can carry a cursor ahead of the
    // feed. Clamp to the next sequence so live rows are not discarded forever.
    let next_seq = feed.snapshot(None, 0).next_seq;
    let live_cursor = live_cursor.map(|from| from.min(next_seq));

    let (initial, resync) = replay(&feed, live_cursor);
    let oldest_seq = initial.oldest_seq;
    let last_sent = live_cursor.map(|from| from.saturating_sub(1)).unwrap_or(0);
    let pending: VecDeque<(u64, LlmActivityEvent)> = initial
        .events
        .into_iter()
        .map(|event| (event.seq(), event))
        .collect();

    let state = LiveState {
        feed,
        receiver,
        pending,
        resync,
        oldest_seq,
        last_sent,
    };
    let stream = stream::unfold(state, |mut state| async move {
        loop {
            if state.resync {
                state.resync = false;
                let events: Vec<LlmActivityEvent> =
                    state.pending.drain(..).map(|(_, event)| event).collect();
                if let Some(last) = events.last() {
                    state.last_sent = last.seq();
                }
                let payload = LlmActivityResync {
                    oldest_seq: state.oldest_seq,
                    events,
                };
                let frame = Event::default()
                    .event("resync")
                    .id(state.last_sent.to_string())
                    .data(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into()));
                return Some((Ok(frame), state));
            }
            if let Some((seq, event)) = state.pending.pop_front() {
                if seq <= state.last_sent {
                    continue;
                }
                state.last_sent = seq;
                return Some((Ok(activity_frame(seq, &event)), state));
            }
            match state.receiver.recv().await {
                Ok((seq, event)) => {
                    if seq <= state.last_sent {
                        continue;
                    }
                    state.last_sent = seq;
                    return Some((Ok(activity_frame(seq, &event)), state));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // The client fell behind the broadcast buffer. Send the whole
                    // retained ring as one resync, then continue from the tail.
                    let snapshot = state.feed.snapshot(None, state.feed.capacity());
                    state.oldest_seq = snapshot.oldest_seq;
                    state.pending.extend(
                        snapshot
                            .events
                            .into_iter()
                            .map(|event| (event.seq(), event)),
                    );
                    state.resync = true;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

struct LiveState {
    feed: Arc<LlmActivityFeed>,
    receiver: broadcast::Receiver<(u64, LlmActivityEvent)>,
    pending: VecDeque<(u64, LlmActivityEvent)>,
    resync: bool,
    oldest_seq: u64,
    last_sent: u64,
}

fn activity_frame(seq: u64, event: &LlmActivityEvent) -> Event {
    Event::default()
        .event("activity")
        .id(seq.to_string())
        .data(serde_json::to_string(event).unwrap_or_else(|_| "{}".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use futures::StreamExt;
    use http_body_util::BodyExt;
    use mm_llm::{
        ActivitySink, CallContext, LlmRequest, Message, MockClient, Purpose, ScriptedResponse,
    };
    use tokio::time::{timeout, Duration};
    use tower::ServiceExt;

    use crate::feature_flags::FeatureFlags;
    use crate::routes::router;
    use crate::state::ConsoleLock;

    async fn booted(flags: FeatureFlags) -> (tempfile::TempDir, Arc<AppState>, axum::Router) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = mm_core::Config::for_data_dir(dir.path());
        let lock = ConsoleLock::acquire(dir.path()).unwrap();
        let state = Arc::new(
            AppState::boot_with_flags(cfg, lock, flags)
                .await
                .expect("boot with activity flags"),
        );
        let app = router(Arc::clone(&state));
        (dir, state, app)
    }

    async fn snapshot_of(app: axum::Router, query: &str) -> serde_json::Value {
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/llm-activity{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }

    #[tokio::test]
    async fn the_snapshot_serves_retained_events_and_filters() {
        let (_dir, state, app) = booted(FeatureFlags::default()).await;
        let feed = state.llm_activity.as_ref().expect("feed is enabled");
        let id = mm_llm::accounting::new_call_id();
        feed.delta(id, "one", 3, false);
        feed.delta(id, "two", 6, false);

        let body = snapshot_of(app.clone(), "").await;
        assert_eq!(body["oldestSeq"], 1);
        assert_eq!(body["nextSeq"], 3);
        assert_eq!(body["events"].as_array().unwrap().len(), 2);
        assert_eq!(body["events"][0]["kind"], "delta");

        let filtered = snapshot_of(app, "?kind=request").await;
        assert!(filtered["events"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_session_filter_keeps_every_event_of_the_call() {
        let (_dir, state, app) = booted(FeatureFlags::default()).await;
        let feed = state.llm_activity.as_ref().expect("feed is enabled");
        let id = mm_llm::accounting::new_call_id();
        let context = CallContext::new("chat").with_session("session-1");
        feed.request(
            id,
            &context,
            "mock",
            true,
            &LlmRequest::new(Purpose::Converse, vec![Message::user("hello")]),
        );
        feed.delta(id, "fragment", 8, false);

        let body = snapshot_of(app.clone(), "?sessionId=session-1").await;
        assert_eq!(
            body["events"].as_array().unwrap().len(),
            2,
            "the delta belongs to the request's session: {body}"
        );

        let deltas = snapshot_of(app, "?sessionId=session-1&kind=delta").await;
        assert_eq!(
            deltas["events"].as_array().unwrap().len(),
            1,
            "the kind filter keeps the session's delta: {deltas}"
        );
    }

    #[tokio::test]
    async fn an_unknown_kind_is_a_bad_request() {
        let (_dir, _state, app) = booted(FeatureFlags::default()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/llm-activity?kind=nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_disabled_feed_answers_unavailable() {
        let (_dir, _state, app) = booted(FeatureFlags {
            llm_activity: false,
            ..FeatureFlags::default()
        })
        .await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/llm-activity")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["code"], "unavailable");
        assert!(body["reason"].as_str().unwrap().contains("MM_LLM_ACTIVITY"));
    }

    #[tokio::test]
    async fn a_scripted_chat_turn_is_observed_with_its_run_context() {
        let (_dir, state, app) = booted(FeatureFlags {
            cme_enabled: false,
            three_pass_v2: false,
            ..FeatureFlags::default()
        })
        .await;
        let client: Arc<dyn mm_llm::LlmClient> =
            Arc::new(MockClient::constant(ScriptedResponse::text("Stored.")));
        *state.controller_model.lock().unwrap() =
            crate::controller::ControllerModel::Scripted(client);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/chat")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"query":"remember this"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = snapshot_of(app, "?kind=request").await;
        let events = body["events"].as_array().unwrap();
        let event = events
            .iter()
            .find(|event| event["source"] == "chat")
            .unwrap_or_else(|| panic!("no chat-scoped request in {events:?}"));
        assert!(event["sessionId"].as_str().is_some(), "{event}");
        assert!(event["runId"].as_str().is_some(), "{event}");
        assert_eq!(event["provider"], "mock");
        assert!(!event["callId"].as_str().unwrap().is_empty());
    }

    async fn first_frame(app: axum::Router, uri: &str) -> String {
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut chunks = response.into_body().into_data_stream();
        let chunk = timeout(Duration::from_secs(2), chunks.next())
            .await
            .expect("the stream must send its replay promptly")
            .expect("the stream must have a first chunk")
            .expect("the first chunk must be readable");
        String::from_utf8_lossy(&chunk).into_owned()
    }

    #[tokio::test]
    async fn the_stream_replays_from_the_cursor() {
        let (_dir, state, app) = booted(FeatureFlags::default()).await;
        let feed = state.llm_activity.as_ref().expect("feed is enabled");
        let id = mm_llm::accounting::new_call_id();
        feed.delta(id, "one", 3, false);
        feed.delta(id, "two", 6, false);

        let frame = first_frame(app, "/api/llm-activity/stream?from=1").await;
        assert!(frame.contains("event: activity"), "{frame}");
        assert!(frame.contains("id: 1"), "{frame}");
        assert!(frame.contains("\"text\":\"one\""), "{frame}");
    }

    #[test]
    fn replay_resyncs_only_when_the_cursor_fell_out_of_the_ring() {
        let feed = LlmActivityFeed::with_capacity(2);
        let id = mm_llm::accounting::new_call_id();
        for index in 0..4 {
            feed.delta(id, &format!("{index}"), index, false);
        }

        let (snapshot, resync) = replay(&feed, Some(1));
        assert!(resync);
        assert_eq!(snapshot.oldest_seq, 3);
        assert_eq!(
            snapshot
                .events
                .iter()
                .map(LlmActivityEvent::seq)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );

        let (snapshot, resync) = replay(&feed, Some(3));
        assert!(!resync);
        assert_eq!(snapshot.events.len(), 2);

        let (snapshot, resync) = replay(&feed, None);
        assert!(!resync);
        assert!(snapshot.events.is_empty());
        assert_eq!(snapshot.next_seq, 5);
    }
}
