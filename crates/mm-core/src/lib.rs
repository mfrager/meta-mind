//! `mm-core` — the Metamind kernel core.
//!
//! It holds everything every other crate needs and nothing that depends on a
//! store, a runtime, or a model: identity, time, configuration, hashing, the
//! IRI grammar, and the three store traits. Nothing here is probabilistic.
#![forbid(unsafe_code)]

pub mod activation;
pub mod codex;
pub mod config;
pub mod error;
pub mod hash;
pub mod id;
pub mod iri;
pub mod store;
pub mod time;

pub use activation::ActivationCondition;
pub use config::Config;
pub use error::{MmError, Result};
pub use hash::{content_hash, hash_fields};
pub use id::{serde_ulid, UlidFactory, ULID_LEN};
pub use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Quad, Term, Triple};
pub use store::{
    AuditRecord, AuditWriter, EventKind, EventSink, Graph, NewEvent, Param, Params, ShaclReport,
    ShaclViolation, Tabular,
};
pub use time::Timestamp;
pub use ulid::Ulid;

/// The lowercase Crockford rendering used for every identifier in the system.
///
/// `ulid`'s `Display` emits uppercase Crockford; Metamind stores and compares
/// the lowercase form so that IRIs and primary keys agree byte-for-byte.
pub fn ulid_string(id: &ulid::Ulid) -> String {
    id.to_string().to_ascii_lowercase()
}
