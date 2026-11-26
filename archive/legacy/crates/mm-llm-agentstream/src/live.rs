//! Best-effort live publication for the console activity dock.
//!
//! The bus is bounded and never blocks the model call. Token deltas are display
//! only: the durable terminal event and the caller's response remain authoritative.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mm_llm::DeltaSink;
use tokio::sync::broadcast;
use ulid::Ulid;

use crate::LiveLlmEvent;

/// Characters buffered before a streaming fragment is reported.
pub(crate) const DELTA_FLUSH_CHARS: usize = 256;
/// Longest a streaming fragment waits before it is reported.
pub(crate) const DELTA_FLUSH_MS: u64 = 50;
/// Most text reported per call. Display only; the response keeps the full text.
pub(crate) const DELTA_CAP_CHARS: usize = 2_000;

/// Bounds on how much streamed text is published for one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeltaPolicy {
    pub(crate) flush_chars: usize,
    pub(crate) flush_ms: u64,
    pub(crate) cap_chars: usize,
}

impl Default for DeltaPolicy {
    fn default() -> Self {
        Self {
            flush_chars: DELTA_FLUSH_CHARS,
            flush_ms: DELTA_FLUSH_MS,
            cap_chars: DELTA_CAP_CHARS,
        }
    }
}

impl DeltaPolicy {
    /// Use `MM_LLM_ACTIVITY_DELTA_MS` to widen or narrow the display window.
    fn from_env() -> Self {
        Self {
            flush_ms: std::env::var("MM_LLM_ACTIVITY_DELTA_MS")
                .ok()
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(DELTA_FLUSH_MS),
            ..Self::default()
        }
    }
}

/// A bounded broadcast bus. Publishing with no subscribers is a no-op.
pub(crate) struct LiveBus {
    sender: broadcast::Sender<LiveLlmEvent>,
    policy: DeltaPolicy,
}

impl LiveBus {
    pub(crate) fn new(capacity: usize) -> Self {
        Self::with_policy(capacity, DeltaPolicy::from_env())
    }

    fn with_policy(capacity: usize, policy: DeltaPolicy) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity.max(1));
        Self { sender, policy }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<LiveLlmEvent> {
        self.sender.subscribe()
    }

    pub(crate) fn publish(&self, event: LiveLlmEvent) {
        // A slow or absent subscriber must never delay or fail a model call.
        let _ = self.sender.send(event);
    }
}

/// A delta sink that forwards to the caller and publishes a coalesced copy.
pub(crate) struct LiveDeltaSink<'a> {
    inner: &'a mut dyn DeltaSink,
    bus: Arc<LiveBus>,
    call_id: Ulid,
    policy: DeltaPolicy,
    buffer: String,
    total_chars: u32,
    truncated: bool,
    last_flush: Instant,
}

impl<'a> LiveDeltaSink<'a> {
    pub(crate) fn new(inner: &'a mut dyn DeltaSink, bus: Arc<LiveBus>, call_id: Ulid) -> Self {
        Self {
            policy: bus.policy,
            inner,
            bus,
            call_id,
            buffer: String::new(),
            total_chars: 0,
            truncated: false,
            last_flush: Instant::now(),
        }
    }

    pub(crate) fn total_chars(&self) -> u32 {
        self.total_chars
    }

    /// Flush whatever is buffered, including after a failed call.
    pub(crate) fn finish(&mut self) {
        self.flush();
    }

    fn flush(&mut self) {
        if self.buffer.is_empty() {
            self.last_flush = Instant::now();
            return;
        }
        self.bus.publish(LiveLlmEvent::Delta {
            call_id: self.call_id,
            text: std::mem::take(&mut self.buffer),
            total_chars: self.total_chars,
            truncated: self.truncated,
        });
        self.last_flush = Instant::now();
    }
}

impl DeltaSink for LiveDeltaSink<'_> {
    fn delta(&mut self, delta: &str) {
        self.inner.delta(delta);
        if self.truncated || delta.is_empty() {
            return;
        }

        let remaining = self
            .policy
            .cap_chars
            .saturating_sub(self.total_chars as usize);
        let chars = delta.chars().count();
        if chars >= remaining {
            self.buffer.extend(delta.chars().take(remaining));
            self.total_chars = self.policy.cap_chars as u32;
            self.truncated = true;
            self.flush();
            return;
        }

        self.buffer.push_str(delta);
        self.total_chars += chars as u32;
        if self.buffer.chars().count() >= self.policy.flush_chars
            || self.last_flush.elapsed() >= Duration::from_millis(self.policy.flush_ms)
        {
            self.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_llm::CollectSink;

    fn bus(policy: DeltaPolicy) -> Arc<LiveBus> {
        Arc::new(LiveBus::with_policy(8, policy))
    }

    #[test]
    fn deltas_are_forwarded_and_published_in_coalesced_fragments() {
        let bus = bus(DeltaPolicy {
            flush_chars: 4,
            flush_ms: 50,
            cap_chars: 10,
        });
        let mut live = bus.subscribe();
        let mut collect = CollectSink::default();
        let mut sink = LiveDeltaSink::new(
            &mut collect,
            Arc::clone(&bus),
            mm_llm::accounting::new_call_id(),
        );

        sink.delta("abcdef");
        sink.delta("ghijkl");
        sink.finish();
        assert_eq!(collect.0, "abcdefghijkl", "the caller sees every fragment");

        let first = match live.try_recv().expect("first fragment") {
            LiveLlmEvent::Delta {
                text,
                total_chars,
                truncated,
                ..
            } => (text, total_chars, truncated),
            other => panic!("expected a delta, got {other:?}"),
        };
        assert_eq!(first, ("abcdef".into(), 6, false));

        let second = match live.try_recv().expect("second fragment") {
            LiveLlmEvent::Delta {
                text,
                total_chars,
                truncated,
                ..
            } => (text, total_chars, truncated),
            other => panic!("expected a delta, got {other:?}"),
        };
        assert_eq!(second, ("ghij".into(), 10, true));
        assert!(live.try_recv().is_err(), "the cap stops publication");
    }

    #[test]
    fn the_display_cap_bounds_publication_but_not_the_caller() {
        let bus = bus(DeltaPolicy {
            flush_chars: 2,
            flush_ms: 50,
            cap_chars: 5,
        });
        let mut live = bus.subscribe();
        let mut collect = CollectSink::default();
        let mut sink = LiveDeltaSink::new(&mut collect, bus, mm_llm::accounting::new_call_id());

        sink.delta("abcdefghij");
        sink.finish();
        assert_eq!(collect.0, "abcdefghij", "the caller still gets everything");

        match live.try_recv().expect("one fragment") {
            LiveLlmEvent::Delta {
                text,
                total_chars,
                truncated,
                ..
            } => {
                assert_eq!(text, "abcde");
                assert_eq!(total_chars, 5);
                assert!(truncated);
            }
            other => panic!("expected a delta, got {other:?}"),
        }
        assert!(live.try_recv().is_err());
    }

    #[test]
    fn default_policy_matches_the_activity_contract() {
        let policy = DeltaPolicy::default();
        assert_eq!(policy.flush_chars, 256);
        assert_eq!(policy.flush_ms, 50);
        assert_eq!(policy.cap_chars, 2_000);
    }
}
