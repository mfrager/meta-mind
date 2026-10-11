//! The SSE frame codec every `mm-serve` stream is written with (plan `01` §6,
//! `09` §3.1).
//!
//! One frame is `id: <seq>\nevent: <type>\ndata: <json>\n\n`:
//!
//! * `id` is the monotonic sequence. Clients resume with `?from=<lastSeq+1>`
//!   (or `Last-Event-ID`) and dedup by it, so replay is idempotent — the same
//!   discipline the rust_symbolic reference transcript uses.
//! * `event` names the frame type (`run`, `tool_call`, `focus`, …).
//! * `data` is one line of JSON. Multi-line payloads are split across several
//!   `data:` lines per the SSE spec; serde's compact output never needs it, but
//!   the codec handles it so no caller has to.
//!
//! Heartbeats are SSE comment lines (`: ping`), which clients ignore but which
//! keep intermediaries from buffering or dropping an idle stream.

use serde::Serialize;

/// One SSE frame. `data` holds the already-serialized JSON body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    /// The monotonic sequence this frame is numbered with.
    pub id: u64,
    /// The event name (`started`, `run`, `tool_call`, `focus`, …).
    pub event: String,
    /// The serialized JSON payload (single line, per `encode`'s contract).
    pub data: String,
}

impl SseFrame {
    /// Build a frame from any serializable payload.
    pub fn json<T: Serialize>(
        id: u64,
        event: &str,
        payload: &T,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            id,
            event: event.to_string(),
            data: serde_json::to_string(payload)?,
        })
    }

    /// Encode as an SSE wire chunk, including the trailing blank line.
    ///
    /// `data` is split on newlines into one `data:` line each, per the spec.
    pub fn encode(&self) -> String {
        let mut out = String::with_capacity(self.data.len() + 32);
        out.push_str("id: ");
        out.push_str(&self.id.to_string());
        out.push('\n');
        out.push_str("event: ");
        out.push_str(&self.event);
        out.push('\n');
        for line in self.data.split('\n') {
            out.push_str("data: ");
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
        out
    }
}

/// An SSE comment line that keeps the connection warm (plan `09` §3.1:
/// every 15s, ignored by clients).
pub fn heartbeat() -> &'static str {
    ": ping\n\n"
}

/// The first sequence a stream should send given the client's `?from=` cursor.
///
/// Sequences are 1-based; `None` (a fresh tail) or `0` starts at 1.
pub fn resume_point(from: Option<u64>) -> u64 {
    match from {
        Some(n) if n >= 1 => n,
        _ => 1,
    }
}

/// The `text/event-stream` headers every stream response must carry (plan
/// `09` §3.1): no caching, no proxy buffering.
pub const SSE_CONTENT_TYPE: &str = "text/event-stream; charset=utf-8";
pub const SSE_CACHE_CONTROL: &str = "no-cache";
pub const SSE_NO_BUFFERING: &str = "no";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_frame_encodes_id_event_and_data() {
        let frame = SseFrame::json(7, "run", &json!({"requestId": "01h8", "query": "hi"})).unwrap();
        assert_eq!(
            frame.encode(),
            "id: 7\nevent: run\ndata: {\"query\":\"hi\",\"requestId\":\"01h8\"}\n\n"
        );
    }

    #[test]
    fn multi_line_data_is_split_per_spec() {
        let frame = SseFrame {
            id: 3,
            event: "note".to_string(),
            data: "line one\nline two".to_string(),
        };
        assert_eq!(
            frame.encode(),
            "id: 3\nevent: note\ndata: line one\ndata: line two\n\n"
        );
    }

    #[test]
    fn json_payloads_are_always_single_line() {
        let frame = SseFrame::json(1, "x", &json!({"text": "a\nb"})).unwrap();
        assert!(
            !frame.data.contains('\n'),
            "serde must escape newlines: {}",
            frame.data
        );
    }

    #[test]
    fn heartbeats_are_spec_comments() {
        assert_eq!(heartbeat(), ": ping\n\n");
    }

    #[test]
    fn resume_points_are_one_based() {
        assert_eq!(resume_point(None), 1);
        assert_eq!(resume_point(Some(0)), 1);
        assert_eq!(resume_point(Some(1)), 1);
        assert_eq!(resume_point(Some(42)), 42);
    }
}
