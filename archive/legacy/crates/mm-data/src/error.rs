//! Errors raised by the data layer.

use std::path::PathBuf;

/// A data-layer failure. Graph failures keep the kernel's own `MmError` so a
/// caller can distinguish a store problem from a schema problem.
#[derive(Debug, thiserror::Error)]
pub enum DataError {
    /// Filesystem failure while creating a home or writing an artifact.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON payload failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// TOML configuration failure.
    #[error("toml: {0}")]
    TomlDeserialize(#[from] toml::de::Error),
    /// TOML serialization failure.
    #[error("toml: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    /// Graph store or SPARQL failure.
    #[error("graph: {0}")]
    Graph(#[from] mm_core::MmError),
    /// SQLite failure.
    #[error("sqlite: {0}")]
    Sqlite(#[from] sqlx::Error),
    /// A configuration value is invalid or incomplete.
    #[error("config: {0}")]
    Config(String),
    /// The requested record does not exist.
    #[error("{kind} not found: {id}")]
    NotFound {
        /// Record kind, for example `user` or `segment`.
        kind: &'static str,
        /// Identifier that was looked up.
        id: String,
    },
    /// A record is malformed or violates a data-layer invariant.
    #[error("invalid {kind}: {detail}")]
    Invalid {
        /// Record kind.
        kind: &'static str,
        /// What is wrong with it.
        detail: String,
    },
    /// An RDF payload could not be interpreted.
    #[error("rdf: {0}")]
    Rdf(String),
    /// A path is relative but no base directory is available.
    #[error("path {0} is not under the data root")]
    OutsideRoot(PathBuf),
}

/// A result carrying [`DataError`].
pub type Result<T> = std::result::Result<T, DataError>;
