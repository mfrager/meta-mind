//! `mm-core` — the Metamind kernel core.
//!
//! It holds everything every kept crate needs and nothing that depends on a
//! store, a runtime, or a model: configuration, identity, time, and hashing.
#![forbid(unsafe_code)]

pub mod config;
pub mod error;
pub mod hash;
pub mod id;
pub mod time;

pub use config::{load_project_env, Config};
pub use error::{MmError, Result};
pub use hash::{content_hash, hash_fields};
pub use id::{parse_ulid, serde_ulid, SessionId, TurnId, UlidFactory, ULID_LEN};
pub use time::Timestamp;
pub use ulid::Ulid;

/// The lowercase Crockford rendering used for every identifier in the system.
///
/// `ulid`'s `Display` emits uppercase Crockford; Metamind stores and compares
/// the lowercase form so that identifiers agree byte-for-byte everywhere.
pub fn ulid_string(id: &ulid::Ulid) -> String {
    id.to_string().to_ascii_lowercase()
}
