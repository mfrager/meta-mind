//! The in-process LLM activity feed.
//!
//! The feed is the only owner of activity sequencing. It subscribes to the
//! gateway's live events; [`LlmActivityFeed::publish_live`] maps the gateway's
//! request/response shapes into the wire events the right-pane dock renders,
//! applies redaction and bounds, and publishes.
//!
//! Publishing is deliberately cheap: a short ring lock, a `VecDeque` push, and a
//! non-awaiting `broadcast::send`. There is no logger call and no disk write
//! here. The feed is a display projection that may drop events under load; the
//! audit log and `llm_calls` remain authoritative.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use mm_llm::redact::Redactor;
use mm_llm::{CallContext, LlmRequest, LlmResponse, Role};
use mm_llm_agentstream::LiveLlmEvent;
use tokio::sync::broadcast;

use crate::dto::llm_activity::{LlmActivityEvent, LlmActivitySnapshot};

/// Default number of retained events (`MM_LLM_ACTIVITY_MAX_EVENTS`).
pub const DEFAULT_CAPACITY: usize = 2_000;
/// Default reply preview length (`MM_LLM_ACTIVITY_PREVIEW_CHARS`).
pub const DEFAULT_REPLY_PREVIEW_CHARS: usize = 4_000;
/// Request previews show the last user message, bounded to this many chars.
pub const REQUEST_PREVIEW_CHARS: usize = 240;
/// Frames buffered for live subscribers before a slow client is told to resync.
const BROADCAST_CAPACITY: usize = 256;

/// Selection policy for the text previews on the wire.
#[derive(Debug, Clone)]
struct PreviewPolicy {
    request_chars: usize,
    reply_chars: usize,
    full_prompts: bool,
    redactor: Redactor,
}

impl PreviewPolicy {
    fn from_env() -> Self {
        Self {
            request_chars: REQUEST_PREVIEW_CHARS,
            reply_chars: env_usize("MM_LLM_ACTIVITY_PREVIEW_CHARS", DEFAULT_REPLY_PREVIEW_CHARS)
                .max(1),
            full_prompts: std::env::var("MM_LLM_ACTIVITY_PROMPTS")
                .map(|value| value.trim().eq_ignore_ascii_case("full"))
                .unwrap_or(false),
            redactor: Redactor::new(),
        }
    }

    /// The last user message, or the whole transcript when full prompts are on.
    fn request(&self, request: &LlmRequest) -> String {
        let text = if self.full_prompts {
            request
                .messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == Role::User)
                .map(|message| message.content.as_str())
                .unwrap_or("")
                .to_string()
        };
        let limit = if self.full_prompts {
            self.reply_chars
        } else {
            self.request_chars
        };
        self.redactor.excerpt(&text, limit)
    }

    fn reply(&self, text: &str) -> String {
        self.redactor.excerpt(text, self.reply_chars)
    }

    fn error(&self, message: &str) -> String {
        self.redactor.excerpt(message, self.request_chars.max(512))
    }
}

/// One retained-and-broadcast activity event.
#[derive(Debug, Clone)]
struct FeedEntry {
    seq: u64,
    event: LlmActivityEvent,
}

/// The live activity feed: a bounded ring plus a broadcast channel.
pub struct LlmActivityFeed {
    capacity: usize,
    next_seq: AtomicU64,
    ring: Mutex<VecDeque<FeedEntry>>,
    tx: broadcast::Sender<(u64, LlmActivityEvent)>,
    preview: PreviewPolicy,
}

impl std::fmt::Debug for LlmActivityFeed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (len, oldest, newest) = {
            let ring = self.ring.lock().unwrap_or_else(|error| error.into_inner());
            (
                ring.len(),
                ring.front().map(|entry| entry.seq),
                ring.back().map(|entry| entry.seq),
            )
        };
        formatter
            .debug_struct("LlmActivityFeed")
            .field("capacity", &self.capacity)
            .field("retained", &len)
            .field("oldest_seq", &oldest)
            .field("newest_seq", &newest)
            .field("next_seq", &self.next_seq.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl LlmActivityFeed {
    /// Build the feed with the configured ring and preview bounds.
    pub fn from_env() -> Self {
        Self::with_capacity(env_usize("MM_LLM_ACTIVITY_MAX_EVENTS", DEFAULT_CAPACITY))
    }

    /// Build the feed with an explicit ring capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            capacity: capacity.max(1),
            next_seq: AtomicU64::new(1),
            ring: Mutex::new(VecDeque::with_capacity(capacity.max(1))),
            tx,
            preview: PreviewPolicy::from_env(),
        }
    }

    /// The configured ring capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Assign a cursor, retain the event, and broadcast it.
    ///
    /// A broadcast with no subscribers is not an error: activity is optional.
    pub fn publish(&self, mut event: LlmActivityEvent) {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        event.set_seq(seq);
        {
            let mut ring = self.ring.lock().unwrap_or_else(|error| error.into_inner());
            ring.push_back(FeedEntry {
                seq,
                event: event.clone(),
            });
            while ring.len() > self.capacity {
                ring.pop_front();
            }
        }
        let _ = self.tx.send((seq, event));
    }

    /// A snapshot of retained events at or after `from`, newest `limit`.
    ///
    /// `oldest_seq` is the oldest retained cursor (or `next_seq` when empty), so
    /// a caller can tell a cursor gap from a complete replay.
    pub fn snapshot(&self, from: Option<u64>, limit: usize) -> LlmActivitySnapshot {
        let ring = self.ring.lock().unwrap_or_else(|error| error.into_inner());
        let next_seq = self.next_seq.load(Ordering::SeqCst);
        let oldest_seq = ring.front().map(|entry| entry.seq).unwrap_or(next_seq);
        let mut events: Vec<LlmActivityEvent> = ring
            .iter()
            .filter(|entry| from.is_none_or(|cursor| entry.seq >= cursor))
            .map(|entry| entry.event.clone())
            .collect();
        if limit == 0 {
            events.clear();
        } else if events.len() > limit {
            let first = events.len() - limit;
            events.drain(..first);
        }
        LlmActivitySnapshot {
            oldest_seq,
            next_seq,
            events,
        }
    }

    /// Subscribe to live events. The receiver sees `(seq, event)` pairs.
    pub fn subscribe(&self) -> broadcast::Receiver<(u64, LlmActivityEvent)> {
        self.tx.subscribe()
    }

    /// Project one gateway live event onto the existing activity wire shape.
    pub fn publish_live(&self, event: LiveLlmEvent) {
        match event {
            LiveLlmEvent::Request {
                call_id,
                context,
                provider,
                streamed,
                request,
            } => self.request(call_id, &context, &provider, streamed, &request),
            LiveLlmEvent::Delta {
                call_id,
                text,
                total_chars,
                truncated,
            } => self.delta(call_id, &text, total_chars, truncated),
            LiveLlmEvent::Response {
                call_id,
                latency_ms,
                response,
            } => self.response(call_id, latency_ms, &response),
            LiveLlmEvent::Error {
                call_id,
                latency_ms,
                partial_chars,
                kind,
                message,
            } => self.error_parts(call_id, latency_ms, partial_chars, &kind, &message),
        }
    }
}

impl LlmActivityFeed {
    fn request(
        &self,
        call_id: ulid::Ulid,
        context: &CallContext,
        provider: &str,
        streamed: bool,
        request: &LlmRequest,
    ) {
        self.publish(LlmActivityEvent::Request {
            seq: 0,
            at: now_ms(),
            call_id: mm_core::ulid_string(&call_id),
            trace_id: request.trace_id.map(|trace| mm_core::ulid_string(&trace)),
            source: context.source.clone(),
            session_id: context.session_id.clone(),
            run_id: context.run_id.clone(),
            pass: context.pass.clone(),
            round: context.round,
            purpose: request.purpose.as_str().to_string(),
            provider: provider.to_string(),
            model: request.model.clone(),
            streamed,
            requested: true,
            message_count: request.messages.len() as u32,
            tool_count: request.tools.len() as u32,
            preview: self.preview.request(request),
        });
    }

    fn delta(&self, call_id: ulid::Ulid, text: &str, total_chars: u32, truncated: bool) {
        let (text, _) = self.preview.redactor.apply(text);
        self.publish(LlmActivityEvent::Delta {
            seq: 0,
            at: now_ms(),
            call_id: mm_core::ulid_string(&call_id),
            text,
            chars: total_chars,
            truncated,
        });
    }

    fn response(&self, call_id: ulid::Ulid, latency_ms: u64, response: &LlmResponse) {
        let status = if response.cached {
            "cached"
        } else if !response.schema_valid {
            "schema_rejected"
        } else {
            "ok"
        };
        self.publish(LlmActivityEvent::Response {
            seq: 0,
            at: now_ms(),
            call_id: mm_core::ulid_string(&call_id),
            model: response.model.clone(),
            status: status.to_string(),
            cached: response.cached,
            tokens_in: response.usage.tokens_in,
            tokens_out: response.usage.tokens_out,
            latency_ms,
            text: self.preview.reply(&response.text),
            tool_calls: response.tool_calls.len() as u32,
        });
    }
}

impl LlmActivityFeed {
    fn error_parts(
        &self,
        call_id: ulid::Ulid,
        latency_ms: u64,
        partial_chars: u32,
        kind: &str,
        message: &str,
    ) {
        self.publish(LlmActivityEvent::Error {
            seq: 0,
            at: now_ms(),
            call_id: mm_core::ulid_string(&call_id),
            status: kind.to_string(),
            error: self.preview.error(message),
            latency_ms,
            partial_chars,
        });
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_llm::{DecoderKind, Message, ProviderToolCall, Purpose, Usage};
    use serde_json::json;

    fn context(source: &str) -> CallContext {
        CallContext::new(source)
            .with_session("session-1")
            .with_run("run-1")
            .with_pass("process", Some(2))
    }

    fn request(text: &str) -> LlmRequest {
        LlmRequest::new(
            Purpose::Plan,
            vec![
                Message::system("system prompt that must not leak"),
                Message::user(text),
            ],
        )
        .with_tools(vec![mm_llm::ToolSchema {
            name: "memory.add".into(),
            description: "store a fact".into(),
            parameters: json!({ "type": "object" }),
        }])
    }

    fn response(text: &str) -> LlmResponse {
        LlmResponse {
            call_id: mm_llm::accounting::new_call_id(),
            text: text.into(),
            model: "response-model".into(),
            usage: Usage {
                tokens_in: 11,
                tokens_out: 22,
                cost_micros: 0,
                latency_ms: 5,
            },
            cached: false,
            schema_valid: true,
            decoder: DecoderKind::None,
            tool_calls: vec![ProviderToolCall {
                id: "t1".into(),
                name: "memory.add".into(),
                arguments: "{}".into(),
            }],
            finish_reason: None,
            provider_metadata: Default::default(),
            streamed: false,
        }
    }

    fn call_id() -> ulid::Ulid {
        mm_llm::accounting::new_call_id()
    }

    fn gateway_request(feed: &LlmActivityFeed, id: ulid::Ulid, request: LlmRequest) {
        feed.publish_live(LiveLlmEvent::Request {
            call_id: id,
            context: context("chat"),
            provider: "mock-provider".into(),
            streamed: true,
            request,
        });
    }

    fn gateway_delta(feed: &LlmActivityFeed, id: ulid::Ulid, text: &str, chars: u32) {
        feed.publish_live(LiveLlmEvent::Delta {
            call_id: id,
            text: text.into(),
            total_chars: chars,
            truncated: false,
        });
    }

    #[test]
    fn sequence_starts_at_one_and_increases() {
        let feed = LlmActivityFeed::with_capacity(8);
        let id = call_id();
        gateway_request(&feed, id, request("hello"));
        feed.publish_live(LiveLlmEvent::Response {
            call_id: id,
            latency_ms: 10,
            response: response("hi"),
        });

        let snapshot = feed.snapshot(None, 8);
        assert_eq!(snapshot.oldest_seq, 1);
        assert_eq!(snapshot.next_seq, 3);
        assert_eq!(
            snapshot
                .events
                .iter()
                .map(LlmActivityEvent::seq)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(snapshot.events[0].kind(), "request");
        assert_eq!(snapshot.events[1].kind(), "response");
    }

    #[test]
    fn the_ring_evicts_the_oldest_and_reports_the_gap() {
        let feed = LlmActivityFeed::with_capacity(2);
        for index in 0..4 {
            gateway_delta(&feed, call_id(), &format!("fragment {index}"), index);
        }
        let snapshot = feed.snapshot(Some(1), 10);
        assert_eq!(snapshot.oldest_seq, 3, "the first two events were evicted");
        assert_eq!(snapshot.next_seq, 5);
        assert_eq!(
            snapshot
                .events
                .iter()
                .map(LlmActivityEvent::seq)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
    }

    #[test]
    fn a_snapshot_returns_the_newest_limit() {
        let feed = LlmActivityFeed::with_capacity(10);
        for index in 0..5 {
            gateway_delta(&feed, call_id(), &format!("{index}"), index);
        }
        let snapshot = feed.snapshot(None, 2);
        assert_eq!(
            snapshot
                .events
                .iter()
                .map(LlmActivityEvent::seq)
                .collect::<Vec<_>>(),
            vec![4, 5]
        );
        assert_eq!(snapshot.oldest_seq, 1);
    }

    #[test]
    fn a_request_carries_context_counts_and_a_bounded_preview() {
        let feed = LlmActivityFeed::with_capacity(8);
        let id = call_id();
        gateway_request(&feed, id, request(&"x".repeat(1_000)));

        match &feed.snapshot(None, 1).events[0] {
            LlmActivityEvent::Request {
                call_id: seen,
                source,
                session_id,
                run_id,
                pass,
                round,
                purpose,
                provider,
                streamed,
                requested,
                message_count,
                tool_count,
                preview,
                ..
            } => {
                assert_eq!(seen, &mm_core::ulid_string(&id));
                assert_eq!(source, "chat");
                assert_eq!(session_id.as_deref(), Some("session-1"));
                assert_eq!(run_id.as_deref(), Some("run-1"));
                assert_eq!(pass.as_deref(), Some("process"));
                assert_eq!(*round, Some(2));
                assert_eq!(purpose, "plan");
                assert_eq!(provider, "mock-provider");
                assert!(streamed);
                assert!(requested);
                assert_eq!(*message_count, 2);
                assert_eq!(*tool_count, 1);
                assert_eq!(preview.chars().count(), REQUEST_PREVIEW_CHARS);
                assert!(!preview.contains('\n'), "previews are one line");
                assert!(
                    !preview.contains("system prompt"),
                    "only the user message is shown: {preview}"
                );
            }
            other => panic!("expected a request, got {other:?}"),
        }
    }

    #[test]
    fn deltas_and_outcomes_map_onto_the_wire() {
        let feed = LlmActivityFeed::with_capacity(8);
        let id = call_id();
        feed.publish_live(LiveLlmEvent::Delta {
            call_id: id,
            text: "partial".into(),
            total_chars: 7,
            truncated: true,
        });
        feed.publish_live(LiveLlmEvent::Response {
            call_id: id,
            latency_ms: 42,
            response: response(&"reply ".repeat(2_000)),
        });
        feed.publish_live(LiveLlmEvent::Error {
            call_id: id,
            latency_ms: 9,
            partial_chars: 3,
            kind: "transport".into(),
            message: "connection reset".into(),
        });

        let events = feed.snapshot(None, 8).events;
        assert_eq!(events.len(), 3);

        match &events[0] {
            LlmActivityEvent::Delta {
                text,
                chars,
                truncated,
                ..
            } => {
                assert_eq!(text, "partial");
                assert_eq!(*chars, 7);
                assert!(truncated);
            }
            other => panic!("expected a delta, got {other:?}"),
        }
        match &events[1] {
            LlmActivityEvent::Response {
                model,
                status,
                tokens_in,
                tokens_out,
                latency_ms,
                text,
                tool_calls,
                ..
            } => {
                assert_eq!(model, "response-model");
                assert_eq!(status, "ok");
                assert_eq!(*tokens_in, 11);
                assert_eq!(*tokens_out, 22);
                assert_eq!(*latency_ms, 42);
                assert!(text.chars().count() <= DEFAULT_REPLY_PREVIEW_CHARS);
                assert!(!text.contains('\n'), "reply previews are one line");
                assert_eq!(*tool_calls, 1);
            }
            other => panic!("expected a response, got {other:?}"),
        }
        match &events[2] {
            LlmActivityEvent::Error {
                status,
                error,
                partial_chars,
                ..
            } => {
                assert_eq!(status, "transport");
                assert!(error.contains("connection reset"));
                assert_eq!(*partial_chars, 3);
            }
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn a_cached_or_invalid_response_gets_an_honest_status() {
        let feed = LlmActivityFeed::with_capacity(8);
        let id = call_id();

        let mut cached = response("cached");
        cached.cached = true;
        feed.publish_live(LiveLlmEvent::Response {
            call_id: id,
            latency_ms: 1,
            response: cached,
        });
        let mut invalid = response("invalid");
        invalid.schema_valid = false;
        feed.publish_live(LiveLlmEvent::Response {
            call_id: id,
            latency_ms: 1,
            response: invalid,
        });

        let events = feed.snapshot(None, 8).events;
        assert!(matches!(
            &events[0],
            LlmActivityEvent::Response { status, cached, .. } if status == "cached" && *cached
        ));
        assert!(matches!(
            &events[1],
            LlmActivityEvent::Response { status, .. } if status == "schema_rejected"
        ));
    }

    #[test]
    fn subscribers_receive_published_events_in_order() {
        let feed = LlmActivityFeed::with_capacity(8);
        let mut receiver = feed.subscribe();
        gateway_delta(&feed, call_id(), "one", 3);
        gateway_delta(&feed, call_id(), "two", 6);

        let first = receiver.try_recv().expect("first event");
        let second = receiver.try_recv().expect("second event");
        assert_eq!(first.0, 1);
        assert_eq!(second.0, 2);
        assert_eq!(first.1.kind(), "delta");
    }

    #[test]
    fn an_empty_feed_reports_the_next_cursor_as_oldest() {
        let feed = LlmActivityFeed::with_capacity(4);
        let snapshot = feed.snapshot(None, 10);
        assert_eq!(snapshot.oldest_seq, 1);
        assert_eq!(snapshot.next_seq, 1);
        assert!(snapshot.events.is_empty());
    }
}
