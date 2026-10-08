//! The kernel's three store boundaries and the records that cross them.
//!
//! There are exactly three authorities and no fact is authoritative in two of
//! them: the **event log** is the source of truth, while SQLite (tabular) and
//! Oxigraph (RDF) hold projections that are replayable from it. These traits are
//! what the rest of the kernel programs against.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::error::MmError;
use crate::hash::hash_fields;
use crate::time::Timestamp;

/// A bound SQL parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    /// SQL `NULL`.
    Null,
    /// A 64-bit signed integer.
    Int(i64),
    /// A 64-bit float.
    Real(f64),
    /// UTF-8 text.
    Text(String),
    /// A byte string.
    Blob(Vec<u8>),
}

impl From<&str> for Param {
    fn from(v: &str) -> Self {
        Param::Text(v.to_string())
    }
}

impl From<String> for Param {
    fn from(v: String) -> Self {
        Param::Text(v)
    }
}

impl From<i64> for Param {
    fn from(v: i64) -> Self {
        Param::Int(v)
    }
}

impl From<i32> for Param {
    fn from(v: i32) -> Self {
        Param::Int(i64::from(v))
    }
}

impl From<u64> for Param {
    fn from(v: u64) -> Self {
        // Values above i64::MAX would silently wrap through SQLite's integer type.
        match i64::try_from(v) {
            Ok(v) => Param::Int(v),
            Err(_) => Param::Text(v.to_string()),
        }
    }
}

impl From<f64> for Param {
    fn from(v: f64) -> Self {
        Param::Real(v)
    }
}

impl From<Option<String>> for Param {
    fn from(v: Option<String>) -> Self {
        Param::opt_text(v)
    }
}

impl Param {
    /// An optional text value: `None` binds SQL `NULL`.
    pub fn opt_text(v: Option<String>) -> Self {
        v.map_or(Param::Null, Param::Text)
    }
}

/// A positional parameter list.
pub type Params = Vec<Param>;

/// The tabular (SQLite) store boundary.
#[async_trait]
pub trait Tabular: Send + Sync {
    /// Execute a statement, returning the number of affected rows.
    async fn execute(&self, sql: &str, args: Params) -> Result<u64, MmError>;

    /// Run a query and map each row to a JSON object keyed by column name.
    async fn query_json(&self, sql: &str, args: Params) -> Result<Vec<serde_json::Value>, MmError>;
}

/// One SHACL violation, flattened so `mm-core` need not depend on the validator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaclViolation {
    /// The shape path that was violated.
    pub path: String,
    /// A human-readable description.
    pub message: String,
    /// The declared severity.
    pub severity: String,
}

/// The outcome of validating a graph against a shapes graph.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaclReport {
    /// True when the graph satisfies every shape.
    pub conforms: bool,
    /// Every violation found, deterministically ordered.
    pub violations: Vec<ShaclViolation>,
}

/// The RDF store boundary.
#[async_trait]
pub trait Graph: Send + Sync {
    /// Insert one quad into a named graph.
    async fn insert(&self, graph: &str, quad: oxrdf::Quad) -> Result<(), MmError>;

    /// Run a SPARQL query scoped to a named graph, returning rows as JSON objects.
    async fn sparql(&self, graph: &str, query: &str) -> Result<Vec<serde_json::Value>, MmError>;

    /// Validate a named graph against the kernel shapes.
    async fn validate(&self, graph: &str) -> Result<ShaclReport, MmError>;
}

/// The kind of event appended to the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The kernel came up and verified its stores.
    KernelBoot,
    /// A projection (tabular or graph) was mutated.
    StoreMutation,
    /// An ontology file was loaded.
    OntologyLoad,
    /// A replay checkpoint was written.
    Checkpoint,
    /// Any event a later phase defines.
    Custom,
}

impl EventKind {
    /// The stable wire name, which doubles as the log event code.
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::KernelBoot => "kernel.boot",
            EventKind::StoreMutation => "store.mutation",
            EventKind::OntologyLoad => "ontology.load",
            EventKind::Checkpoint => "checkpoint",
            EventKind::Custom => "custom",
        }
    }

    /// Parse a wire name. Unknown kinds are `Custom`, never an error: the log
    /// must accept events written by a newer phase.
    pub fn from_wire(s: &str) -> Self {
        match s {
            "kernel.boot" => EventKind::KernelBoot,
            "store.mutation" => EventKind::StoreMutation,
            "ontology.load" => EventKind::OntologyLoad,
            "checkpoint" => EventKind::Checkpoint,
            _ => EventKind::Custom,
        }
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An event proposed for the log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewEvent {
    /// What kind of event this is.
    pub kind: EventKind,
    /// The canonical JSON body.
    pub payload: serde_json::Value,
    /// The correlation `trace_id` shared by the logs of the same operation.
    pub correlation: Option<Ulid>,
}

impl NewEvent {
    /// Build an event from a serializable payload.
    pub fn new<T: Serialize>(
        kind: EventKind,
        payload: &T,
        correlation: Option<Ulid>,
    ) -> Result<Self, MmError> {
        Ok(NewEvent {
            kind,
            payload: serde_json::to_value(payload)?,
            correlation,
        })
    }

    /// The canonical JSON encoding used for hashing and storage.
    pub fn canonical_payload(&self) -> Result<String, MmError> {
        Ok(serde_json::to_string(&self.payload)?)
    }
}

/// The append/commit half of the event log, as seen by other layers.
#[async_trait]
pub trait EventSink: Send + Sync {
    /// Append an event `provisional`. Idempotent by content hash.
    async fn append(&self, event: NewEvent) -> Result<Ulid, MmError>;

    /// Mark a previously appended event `committed`.
    async fn commit(&self, id: &Ulid) -> Result<(), MmError>;
}

/// An immutable, append-only audit record.
///
/// Audit records are never updated or deleted (SQLite triggers enforce that) and
/// each one carries a hash chained to its predecessor, so rewriting history is
/// detectable rather than merely forbidden.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// The record's own identifier.
    pub record_id: Ulid,
    /// The log event code this record attests to.
    pub event_code: String,
    /// The severity level.
    pub level: String,
    /// The emitting subsystem.
    pub target: String,
    /// The correlation identifier, if any.
    pub trace_id: Option<Ulid>,
    /// The structured body.
    pub payload: serde_json::Value,
    /// When the audit was taken.
    pub at: Timestamp,
}

impl AuditRecord {
    /// Build an audit record for an event code.
    pub fn new(
        event_code: impl Into<String>,
        level: impl Into<String>,
        target: impl Into<String>,
        trace_id: Option<Ulid>,
        payload: serde_json::Value,
    ) -> Self {
        AuditRecord {
            record_id: Ulid::nil(),
            event_code: event_code.into(),
            level: level.into(),
            target: target.into(),
            trace_id,
            payload,
            at: Timestamp::now(),
        }
    }

    /// The canonical JSON of the record's body. Key order is deterministic
    /// because `serde_json::Map` is ordered.
    pub fn canonical_payload(&self) -> String {
        serde_json::to_string(&self.payload).unwrap_or_else(|_| "null".to_string())
    }

    /// The chain hash of this record, given its predecessor's hash.
    ///
    /// The writer and the verifier share this one definition, so a chain that
    /// verifies is a chain that was written by this code.
    pub fn chain_hash(&self, prev_hash: Option<&str>) -> String {
        let payload = self.canonical_payload();
        let trace = self
            .trace_id
            .map_or(String::new(), |t| crate::ulid_string(&t));
        hash_fields(&[
            prev_hash.unwrap_or(""),
            &crate::ulid_string(&self.record_id),
            &self.event_code,
            &self.level,
            &self.target,
            &trace,
            &payload,
            &self.at.to_rfc3339(),
        ])
    }
}

/// A store that can persist audit records.
#[async_trait]
pub trait AuditWriter: Send + Sync {
    /// Append the record, extending the hash chain atomically.
    async fn write_audit(&self, record: &AuditRecord) -> Result<(), MmError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_wire_names_round_trip_and_unknown_is_custom() {
        for kind in [
            EventKind::KernelBoot,
            EventKind::StoreMutation,
            EventKind::OntologyLoad,
            EventKind::Checkpoint,
            EventKind::Custom,
        ] {
            assert_eq!(EventKind::from_wire(kind.as_str()), kind);
        }
        assert_eq!(
            EventKind::from_wire("from.a.future.phase"),
            EventKind::Custom
        );
    }

    #[test]
    fn audit_chain_hash_is_stable_and_prev_sensitive() {
        let mut rec = AuditRecord::new(
            "kernel.boot",
            "info",
            "mm.cli",
            None,
            serde_json::json!({"version": "0.1.0", "stores_ready": true}),
        );
        rec.record_id = Ulid::from_parts(1, 2);
        let a = rec.chain_hash(None);
        assert_eq!(a, rec.chain_hash(None), "hashing must be deterministic");
        assert_ne!(a, rec.chain_hash(Some("deadbeef")));
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn payload_key_order_does_not_change_the_hash() {
        let at = crate::Timestamp::from_rfc3339("2024-01-01T00:00:00.000000000Z").unwrap();
        let mut one = AuditRecord::new("e", "info", "t", None, serde_json::json!({"b": 1, "a": 2}));
        let mut two = AuditRecord::new("e", "info", "t", None, serde_json::json!({"a": 2, "b": 1}));
        // Pin the instants: the chain hash covers `at`, so two records built at
        // different moments legitimately differ.
        one.at = at;
        two.at = at;
        assert_eq!(one.chain_hash(None), two.chain_hash(None));

        let mut reordered = one.clone();
        reordered.payload = serde_json::json!({"a": 2, "b": 1});
        assert_eq!(one.canonical_payload(), reordered.canonical_payload());
    }

    #[test]
    fn a_different_instant_changes_the_hash() {
        let one = AuditRecord::new("e", "info", "t", None, serde_json::json!({}));
        let mut two = one.clone();
        two.at = crate::Timestamp::from_rfc3339("2025-01-01T00:00:00.000000000Z").unwrap();
        assert_ne!(one.chain_hash(None), two.chain_hash(None));
    }

    #[test]
    fn params_convert_from_common_types() {
        assert_eq!(Param::from("x"), Param::Text("x".into()));
        assert_eq!(Param::from(7i64), Param::Int(7));
        assert_eq!(Param::from(7i32), Param::Int(7));
        assert_eq!(Param::from(None::<String>), Param::Null);
        assert_eq!(Param::from(1.5f64), Param::Real(1.5));
    }
}
