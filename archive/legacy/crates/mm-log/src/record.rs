//! The log record: one JSON object per line, and the only shape a Metamind
//! record has in any sink.

use mm_core::{Timestamp, Ulid};
use serde::{Deserialize, Serialize};

/// Severity, ordered `Error > Warn > Info > Debug > Trace`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// A failure that needs attention.
    Error,
    /// Something is wrong but the kernel continued.
    Warn,
    /// A normal, notable state change.
    Info,
    /// Detail useful when diagnosing.
    Debug,
    /// Full detail; rarely enabled.
    Trace,
}

impl Level {
    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
            Level::Trace => "trace",
        }
    }

    /// Parse a wire name. Unknown names are an error: a typo in the config must
    /// not silently change what is logged.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "error" => Some(Level::Error),
            "warn" => Some(Level::Warn),
            "info" => Some(Level::Info),
            "debug" => Some(Level::Debug),
            "trace" => Some(Level::Trace),
            _ => None,
        }
    }

    /// Numeric rank, higher is more severe.
    pub fn rank(self) -> u8 {
        match self {
            Level::Error => 4,
            Level::Warn => 3,
            Level::Info => 2,
            Level::Debug => 1,
            Level::Trace => 0,
        }
    }
}

impl PartialOrd for Level {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Level {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One structured record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogRecord {
    /// Lowercase ULID of this record.
    pub record_id: String,
    /// When the record was taken.
    pub at: Timestamp,
    /// Severity.
    pub level: Level,
    /// The stable event code (see [`crate::codes`]).
    pub event_code: String,
    /// The emitting subsystem.
    pub target: String,
    /// Correlation identifier shared with the event log and RDF provenance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// The enclosing span, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<String>,
    /// The structured body. Always a JSON object.
    pub fields: serde_json::Value,
    /// Whether this record must also be written to the immutable audit chain.
    pub audit: bool,
    /// How many redactions were applied to this record.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub redactions: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl LogRecord {
    /// A record with an empty body.
    pub fn new(level: Level, event_code: impl Into<String>, target: impl Into<String>) -> Self {
        LogRecord {
            record_id: String::new(),
            at: Timestamp::now(),
            level,
            event_code: event_code.into(),
            target: target.into(),
            trace_id: None,
            span: None,
            fields: serde_json::Value::Object(serde_json::Map::new()),
            audit: false,
            redactions: 0,
        }
    }

    /// Correlate with an operation.
    pub fn with_trace(mut self, trace: Ulid) -> Self {
        self.trace_id = Some(mm_core::ulid_string(&trace));
        self
    }

    /// Correlate with an operation given its string form.
    pub fn with_trace_str(mut self, trace: impl Into<String>) -> Self {
        self.trace_id = Some(trace.into());
        self
    }

    /// Name the enclosing span.
    pub fn with_span(mut self, span: impl Into<String>) -> Self {
        self.span = Some(span.into());
        self
    }

    /// Add one field.
    pub fn with_field(mut self, key: &str, value: impl Serialize) -> Self {
        let value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
        if let serde_json::Value::Object(map) = &mut self.fields {
            map.insert(key.to_string(), value);
        }
        self
    }

    /// Mark the record as an audit record.
    pub fn audited(mut self) -> Self {
        self.audit = true;
        self
    }

    /// The single-line JSON form that every sink writes.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|e| {
            format!(r#"{{"level":"error","event_code":"log.serialize.failed","message":"{e}"}}"#)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ranks_order_error_highest() {
        assert!(Level::Error > Level::Warn);
        assert!(Level::Warn > Level::Info);
        assert!(Level::Info > Level::Debug);
        assert!(Level::Debug > Level::Trace);
    }

    #[test]
    fn level_wire_names_round_trip_and_typos_are_rejected() {
        for l in [
            Level::Error,
            Level::Warn,
            Level::Info,
            Level::Debug,
            Level::Trace,
        ] {
            assert_eq!(Level::parse(l.as_str()), Some(l));
        }
        assert_eq!(Level::parse("verbose"), None);
    }

    #[test]
    fn record_serializes_without_absent_optionals() {
        let rec = LogRecord::new(Level::Info, "kernel.boot", "mm.cli")
            .with_field("version", "0.1.0")
            .with_field("stores_ready", true);
        let json = rec.to_json();
        assert!(json.contains(r#""event_code":"kernel.boot""#));
        assert!(!json.contains("trace_id"));
        assert!(!json.contains("span"));
        assert!(!json.contains("redactions"));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["fields"]["stores_ready"], serde_json::json!(true));
    }
}
