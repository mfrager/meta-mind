//! Usage accounting for model calls.
//!
//! The durable `llm_calls` ledger retired with `mm-store-sqlite`. Agentstream is
//! the only call log now, and this module keeps the typed usage summary a caller
//! may compute from the events it already received.

use std::collections::BTreeMap;

use serde::Serialize;
use ulid::Ulid;

use crate::client::{DecoderKind, Purpose};

/// A fresh call identifier.
pub fn new_call_id() -> Ulid {
    Ulid::from_datetime(std::time::SystemTime::now())
}

/// How a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    /// The provider answered and the output was acceptable.
    Ok,
    /// The output did not satisfy its schema.
    SchemaRejected,
    /// The provider could not be reached, was not configured, or errored.
    ProviderError,
    /// The provider did not answer in time.
    Timeout,
    /// The answer came from a recorded session.
    Replay,
}

impl CallStatus {
    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            CallStatus::Ok => "ok",
            CallStatus::SchemaRejected => "schema_rejected",
            CallStatus::ProviderError => "provider_error",
            CallStatus::Timeout => "timeout",
            CallStatus::Replay => "replay",
        }
    }

    /// Parse the stored form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(CallStatus::Ok),
            "schema_rejected" => Some(CallStatus::SchemaRejected),
            "provider_error" => Some(CallStatus::ProviderError),
            "timeout" => Some(CallStatus::Timeout),
            "replay" => Some(CallStatus::Replay),
            _ => None,
        }
    }
}

/// One completed call.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallAccount {
    /// The call's identifier.
    pub call_id: Ulid,
    /// Why it was made.
    pub purpose: Purpose,
    /// The provider that answered.
    pub provider: String,
    /// The model used.
    pub model: String,
    /// Whether the output satisfied its schema.
    pub schema_ok: bool,
    /// Which decoder was used.
    pub decoder: DecoderKind,
    /// Whether the answer was cached.
    pub cached: bool,
    /// Prompt tokens.
    pub tokens_in: u32,
    /// Completion tokens.
    pub tokens_out: u32,
    /// Cost in micros.
    pub cost_micros: i64,
    /// Latency in milliseconds.
    pub latency_ms: u32,
    /// How the call ended.
    pub status: CallStatus,
}

/// Per-purpose totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct PurposeStats {
    /// Calls made.
    pub calls: u64,
    /// Prompt tokens.
    pub tokens_in: u64,
    /// Completion tokens.
    pub tokens_out: u64,
    /// Cost in micros.
    pub cost_micros: i64,
    /// Calls whose output was rejected.
    pub schema_rejects: u64,
    /// Calls that hit a cache.
    pub cache_hits: u64,
}

/// Usage totals over a set of calls.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LlmStats {
    /// Calls recorded.
    pub calls: u64,
    /// Prompt tokens across calls.
    pub tokens_in: u64,
    /// Completion tokens across calls.
    pub tokens_out: u64,
    /// Cost across calls.
    pub cost_micros: i64,
    /// Calls whose output was rejected.
    pub schema_rejects: u64,
    /// Calls that hit a cache.
    pub cache_hits: u64,
    /// Per-purpose breakdown.
    pub by_purpose: BTreeMap<String, PurposeStats>,
}

impl LlmStats {
    /// Fold one call into the totals.
    pub fn record(&mut self, call: &CallAccount) {
        let purpose = call.purpose.as_str().to_owned();
        let per = self.by_purpose.entry(purpose).or_default();
        per.calls += 1;
        per.tokens_in += u64::from(call.tokens_in);
        per.tokens_out += u64::from(call.tokens_out);
        per.cost_micros += call.cost_micros;
        per.schema_rejects +=
            u64::from(!call.schema_ok || call.status == CallStatus::SchemaRejected);
        per.cache_hits += u64::from(call.cached);

        self.calls += 1;
        self.tokens_in += u64::from(call.tokens_in);
        self.tokens_out += u64::from(call.tokens_out);
        self.cost_micros += call.cost_micros;
        self.schema_rejects +=
            u64::from(!call.schema_ok || call.status == CallStatus::SchemaRejected);
        self.cache_hits += u64::from(call.cached);
    }
}

/// Which calls a summary covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatsWindow {
    /// Every call.
    All,
    /// Everything created at or after this instant.
    Since(String),
    /// One purpose.
    Purpose(Purpose),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(purpose: Purpose, status: CallStatus) -> CallAccount {
        CallAccount {
            call_id: new_call_id(),
            purpose,
            provider: "mock".into(),
            model: "small".into(),
            schema_ok: true,
            decoder: DecoderKind::None,
            cached: false,
            tokens_in: 10,
            tokens_out: 5,
            cost_micros: 12,
            latency_ms: 7,
            status,
        }
    }

    #[test]
    fn statuses_round_trip() {
        for status in [
            CallStatus::Ok,
            CallStatus::SchemaRejected,
            CallStatus::ProviderError,
            CallStatus::Timeout,
            CallStatus::Replay,
        ] {
            assert_eq!(CallStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(CallStatus::parse("nope"), None);
    }

    #[test]
    fn stats_sum_calls_and_group_by_purpose() {
        let mut stats = LlmStats::default();
        stats.record(&account(Purpose::Plan, CallStatus::Ok));
        stats.record(&account(Purpose::Plan, CallStatus::SchemaRejected));
        stats.record(&account(Purpose::Summarize, CallStatus::Ok));

        assert_eq!(stats.calls, 3);
        assert_eq!(stats.tokens_in, 30);
        assert_eq!(stats.tokens_out, 15);
        assert_eq!(stats.cost_micros, 36);
        assert_eq!(stats.schema_rejects, 1);
        assert_eq!(stats.by_purpose["plan"].calls, 2);
    }
}
