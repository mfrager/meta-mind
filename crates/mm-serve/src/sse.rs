//! SSE framing for the agentstream.

use axum::response::sse::Event as SseEvent;
use mm_agentstream::Event;

use crate::dto::EventPage;

/// One SSE frame. `id:` is the per-session cursor, `event:` the event type, and
/// `data:` the persisted record unchanged.
pub fn frame(event: &Event) -> SseEvent {
    let mut frame = SseEvent::default().event(event.event_type.as_str());
    // A delta is live-only and has no cursor; an `id: 0` would rewind a client.
    if event.event_type.is_persisted() {
        frame = frame.id(event.seq.to_string());
    }
    frame
        .json_data(event)
        .unwrap_or_else(|_| SseEvent::default().data("{}"))
}

/// A snapshot sent when the cursor is below the retained segment start.
pub fn resync(page: &EventPage) -> SseEvent {
    SseEvent::default()
        .event("resync")
        .json_data(page)
        .unwrap_or_else(|_| SseEvent::default().data("{}"))
}
