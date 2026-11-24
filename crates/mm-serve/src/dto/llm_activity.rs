//! LLM activity wire types (plan `llm_activity_log_plan.md` §4).
//!
//! These are the rows the right-pane activity dock renders. They are a live,
//! redacted projection of model traffic — not the audit ledger and not the chat
//! transcript: `seq` is the feed's own monotonic cursor, and every event carries
//! the feed's `call_id` so a request, its streamed fragments, and its outcome can
//! be merged into one row.
//!
//! Event fields are camelCase on the wire, matching the existing chat frames
//! (`requestId`, `sessionId`, `tokensIn`). The snapshot envelope is camelCase
//! for the same reason.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One observable moment in a model call.
///
/// `kind` is the discriminator. A call emits exactly one `request`, any number
/// of `delta` fragments for a streaming answer, and exactly one terminal
/// `response` or `error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LlmActivityEvent {
    /// A call reached the observed client.
    Request {
        /// Monotonic feed cursor.
        seq: u64,
        /// When the call started, epoch milliseconds.
        at: i64,
        /// ULID shared by every event of this call.
        call_id: String,
        /// The request's trace id, when it has one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
        /// Subsystem that made the call (`chat`, `agent`, `automation`, …).
        source: String,
        /// Conversation session, when the call belongs to one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        /// Chat run, when the call belongs to one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        /// Three-pass phase (`interpret`, `process`, `verify`, `think`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pass: Option<String>,
        /// Model round inside the phase.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        round: Option<u32>,
        /// Why the call is being made (`Purpose::as_str`).
        purpose: String,
        /// The provider adapter's name.
        provider: String,
        /// The requested model. The answering model is on `response`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// Whether the caller asked for deltas.
        streamed: bool,
        /// Whether this call was asked of a provider. P1 always reports `true`;
        /// `response.cached` says whether a cache answered instead.
        requested: bool,
        /// How many messages the request carried.
        message_count: u32,
        /// How many tools the request offered.
        tool_count: u32,
        /// Redacted, bounded user excerpt.
        preview: String,
    },
    /// A coalesced fragment of streaming text.
    Delta {
        /// Monotonic feed cursor.
        seq: u64,
        /// When the fragment was reported, epoch milliseconds.
        at: i64,
        /// The call this fragment belongs to.
        call_id: String,
        /// Redacted fragment text.
        text: String,
        /// Characters reported for the call so far.
        chars: u32,
        /// True once the per-call preview cap was reached.
        truncated: bool,
    },
    /// A call answered.
    Response {
        /// Monotonic feed cursor.
        seq: u64,
        /// When the answer arrived, epoch milliseconds.
        at: i64,
        /// The call this answer belongs to.
        call_id: String,
        /// The model that actually answered.
        model: String,
        /// `ok`, `cached`, or `schema_rejected`.
        status: String,
        /// Whether the answer came from the exact cache.
        cached: bool,
        /// Prompt tokens billed.
        tokens_in: u32,
        /// Completion tokens billed.
        tokens_out: u32,
        /// Wall-clock latency.
        latency_ms: u64,
        /// Redacted, bounded reply excerpt.
        text: String,
        /// How many tool calls the answer proposed.
        tool_calls: u32,
    },
    /// A call failed.
    Error {
        /// Monotonic feed cursor.
        seq: u64,
        /// When the failure arrived, epoch milliseconds.
        at: i64,
        /// The call that failed.
        call_id: String,
        /// The `LlmError::kind()` value.
        status: String,
        /// Redacted, bounded error message.
        error: String,
        /// Wall-clock latency.
        latency_ms: u64,
        /// Streaming characters reported before the failure.
        partial_chars: u32,
    },
}

impl LlmActivityEvent {
    /// The event's feed cursor.
    pub fn seq(&self) -> u64 {
        match self {
            LlmActivityEvent::Request { seq, .. }
            | LlmActivityEvent::Delta { seq, .. }
            | LlmActivityEvent::Response { seq, .. }
            | LlmActivityEvent::Error { seq, .. } => *seq,
        }
    }

    /// The wire discriminator (`request`, `delta`, `response`, `error`).
    pub fn kind(&self) -> &'static str {
        match self {
            LlmActivityEvent::Request { .. } => "request",
            LlmActivityEvent::Delta { .. } => "delta",
            LlmActivityEvent::Response { .. } => "response",
            LlmActivityEvent::Error { .. } => "error",
        }
    }

    /// The conversation session, when the event belongs to one.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            LlmActivityEvent::Request { session_id, .. } => session_id.as_deref(),
            _ => None,
        }
    }

    /// The call this event belongs to.
    pub fn call_id(&self) -> &str {
        match self {
            LlmActivityEvent::Request { call_id, .. }
            | LlmActivityEvent::Delta { call_id, .. }
            | LlmActivityEvent::Response { call_id, .. }
            | LlmActivityEvent::Error { call_id, .. } => call_id,
        }
    }

    /// Assign the feed cursor. Only the feed calls this.
    pub(crate) fn set_seq(&mut self, next: u64) {
        match self {
            LlmActivityEvent::Request { seq, .. }
            | LlmActivityEvent::Delta { seq, .. }
            | LlmActivityEvent::Response { seq, .. }
            | LlmActivityEvent::Error { seq, .. } => *seq = next,
        }
    }
}

/// The initial snapshot the dock hydrates from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LlmActivitySnapshot {
    /// Oldest sequence still retained. A `from` below this has lost events.
    pub oldest_seq: u64,
    /// The next sequence the feed will assign.
    pub next_seq: u64,
    /// Events matching the request, oldest first.
    pub events: Vec<LlmActivityEvent>,
}

/// The payload of an SSE `resync` frame: the retained ring after a cursor gap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LlmActivityResync {
    /// Oldest sequence still retained.
    pub oldest_seq: u64,
    /// The retained events, oldest first.
    pub events: Vec<LlmActivityEvent>,
}
