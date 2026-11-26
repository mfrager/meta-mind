//! The memory error type and the payloads it carries.
//!
//! Every refusal in this crate is *typed*, for the same reason Phase 4's are: a
//! refusal that is only a string cannot be told apart from a bug, and
//! `memory verify` has to be able to say *which* guarantee held. The three
//! refusals worth naming are therefore their own payloads — a utility that cannot
//! be computed, an admission the store declines, and a protected record a forget
//! cycle may not touch.

use serde::{Deserialize, Serialize};

/// A utility score that could not be computed.
///
/// `storage_cost` is the divisor: a zero there is not a small utility, it is an
/// undefined one, and returning `0` or `f64::INFINITY` would let a record with no
/// cost look maximally valuable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UtilityDenied {
    /// The input that was rejected.
    pub field: String,
    /// Why it was rejected.
    pub reason: String,
}

impl std::fmt::Display for UtilityDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "utility({}) is undefined: {}", self.field, self.reason)
    }
}

/// A memory the store refused to admit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdmissionDenied {
    /// The admission channel: `duplicate`, `near_duplicate`, `contradiction`,
    /// `missing_provenance`, `orphan_entity`.
    pub flag: String,
    /// A human-readable reason.
    pub reason: String,
    /// The existing memory that already covers this one, if any.
    pub existing: Option<String>,
}

impl std::fmt::Display for AdmissionDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "memory not admitted ({}): {}", self.flag, self.reason)
    }
}

/// A record a forget cycle may not archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedRefusal {
    /// The memory that was spared.
    pub memory_id: String,
    /// Its kind.
    pub kind: String,
    /// What was attempted: `archive`.
    pub attempted_action: String,
    /// What protects it: `protected`, `developmental`, or a `commitment:<ulid>`.
    pub blocking_reference: String,
    /// Why it was below the threshold in the first place.
    pub retention: String,
}

impl std::fmt::Display for ProtectedRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "memory {} ({}) refused `{}`: held by {}",
            self.memory_id, self.kind, self.attempted_action, self.blocking_reference
        )
    }
}

/// A failure in the memory organ.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    /// A utility could not be computed.
    #[error("{0}")]
    Utility(UtilityDenied),
    /// A memory was refused admission.
    #[error("{0}")]
    Admission(AdmissionDenied),
    /// A record is protected and may not be archived.
    #[error("{0}")]
    Protected(ProtectedRefusal),
    /// A value failed validation.
    #[error("memory validation failed for {field}: {reason}")]
    Validation {
        /// The field that failed.
        field: String,
        /// Why it failed.
        reason: String,
    },
    /// A named record does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// A tabular store operation failed.
    #[error("memory database error: {0}")]
    Db(String),
    /// An RDF store operation failed.
    #[error("memory graph error: {0}")]
    Graph(String),
    /// A document could not be encoded or decoded.
    #[error("memory codec error: {0}")]
    Codec(String),
    /// The configuration or an argument is invalid.
    #[error("memory config error: {0}")]
    Config(String),
    /// A component this crate does not have failed.
    #[error("memory internal error: {0}")]
    Internal(String),
}

impl MemoryError {
    /// The stable short code, mirroring `MmError::code`.
    pub fn code(&self) -> &'static str {
        match self {
            MemoryError::Utility(_) => "memory.utility",
            MemoryError::Admission(_) => "memory.admission",
            MemoryError::Protected(_) => "memory.protected",
            MemoryError::Validation { .. } => "memory.validation",
            MemoryError::NotFound(_) => "memory.not_found",
            MemoryError::Db(_) => "memory.store",
            MemoryError::Graph(_) => "memory.graph",
            MemoryError::Codec(_) => "memory.codec",
            MemoryError::Config(_) => "memory.config",
            MemoryError::Internal(_) => "memory.internal",
        }
    }

    /// A validation failure for one field.
    pub fn validation(field: impl Into<String>, reason: impl Into<String>) -> Self {
        MemoryError::Validation {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

/// A memory result.
pub type Result<T> = std::result::Result<T, MemoryError>;

impl From<mm_core::MmError> for MemoryError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(m) => MemoryError::Db(m),
            mm_core::MmError::Graph(m) => MemoryError::Graph(m),
            mm_core::MmError::Config(m) => MemoryError::Config(m),
            mm_core::MmError::Codec(m) => MemoryError::Codec(m),
            other => MemoryError::Internal(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for MemoryError {
    fn from(e: serde_json::Error) -> Self {
        MemoryError::Codec(e.to_string())
    }
}

impl From<std::io::Error> for MemoryError {
    fn from(e: std::io::Error) -> Self {
        MemoryError::Config(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_prefixed() {
        let cases: Vec<MemoryError> = vec![
            MemoryError::Utility(UtilityDenied {
                field: "storage_cost".into(),
                reason: "must be > 0".into(),
            }),
            MemoryError::Admission(AdmissionDenied {
                flag: "duplicate".into(),
                reason: "same content hash".into(),
                existing: None,
            }),
            MemoryError::Protected(ProtectedRefusal {
                memory_id: "01h".into(),
                kind: "developmental".into(),
                attempted_action: "archive".into(),
                blocking_reference: "developmental".into(),
                retention: "0.01".into(),
            }),
            MemoryError::validation("confidence", "out of range"),
        ];
        for error in cases {
            assert!(
                error.code().starts_with("memory."),
                "{} should be namespaced",
                error.code()
            );
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn a_kernel_error_maps_onto_its_own_variant() {
        let mapped: MemoryError = mm_core::MmError::Store("boom".into()).into();
        assert_eq!(mapped.code(), "memory.store");
        let mapped: MemoryError = mm_core::MmError::Graph("boom".into()).into();
        assert_eq!(mapped.code(), "memory.graph");
    }
}
