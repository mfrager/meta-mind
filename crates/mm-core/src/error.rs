//! The kernel error type. Every fallible boundary in Metamind funnels through
//! `MmError`; no error is swallowed and no error is a stringly-typed panic.

use crate::time::TimestampError;

/// A Metamind kernel error, grouped by the layer that produced it.
#[derive(Debug, thiserror::Error)]
pub enum MmError {
    /// A tabular (SQLite) store operation failed.
    #[error("store error: {0}")]
    Store(String),
    /// An RDF graph operation failed.
    #[error("graph error: {0}")]
    Graph(String),
    /// An event-log operation failed.
    #[error("event error: {0}")]
    Event(String),
    /// Configuration could not be read or is invalid.
    #[error("config error: {0}")]
    Config(String),
    /// A serialization or canonicalization step failed.
    #[error("codec error: {0}")]
    Codec(String),
    /// A kernel invariant was violated.
    #[error("internal error: {0}")]
    Internal(String),
}

impl MmError {
    /// A stable machine-readable code, used in log records and CLI output.
    pub fn code(&self) -> &'static str {
        match self {
            MmError::Store(_) => "mm.store",
            MmError::Graph(_) => "mm.graph",
            MmError::Event(_) => "mm.event",
            MmError::Config(_) => "mm.config",
            MmError::Codec(_) => "mm.codec",
            MmError::Internal(_) => "mm.internal",
        }
    }
}

impl From<std::io::Error> for MmError {
    fn from(e: std::io::Error) -> Self {
        MmError::Store(e.to_string())
    }
}

impl From<serde_json::Error> for MmError {
    fn from(e: serde_json::Error) -> Self {
        MmError::Codec(e.to_string())
    }
}

impl From<TimestampError> for MmError {
    fn from(e: TimestampError) -> Self {
        MmError::Codec(e.to_string())
    }
}

/// The kernel result alias.
pub type Result<T> = std::result::Result<T, MmError>;
