//! The epistemic error type and the refusals it carries.
//!
//! Every refusal is *typed* for the same reason Phase 4's and Phase 5's are: a
//! refusal that is only a string cannot be told apart from a bug, and the gate has
//! to be able to say *which* guarantee held. Three refusals are worth naming:
//! a promotion the table forbids, a write the `/world` barrier declines, and a
//! justification cycle.

use mm_core::Ulid;
use serde::{Deserialize, Serialize};

use crate::status::EpistemicStatus;

/// A promotion the transition table forbids.
///
/// `reason` is `&'static str` because it comes from the table, not from a caller:
/// the guard takes no model output, so there is nothing dynamic to interpolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionDenied {
    /// The status being left.
    pub from: EpistemicStatus,
    /// The status wanted.
    pub to: EpistemicStatus,
    /// Why the table refuses it.
    pub reason: &'static str,
}

impl std::fmt::Display for PromotionDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} -> {} refused: {}", self.from, self.to, self.reason)
    }
}

/// A write the `/world` barrier declined.
///
/// Only `OBSERVED`/`VERIFIED` claims belong in `/world`, and the barrier is the
/// only writer. A refusal here is the mechanism, not an accident.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldRefusal {
    /// The claim that may not enter.
    pub claim: String,
    /// The status it carries.
    pub status: EpistemicStatus,
    /// Why it may not enter.
    pub reason: String,
}

impl std::fmt::Display for WorldRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "claim {} ({}) may not enter /world: {}",
            self.claim, self.status, self.reason
        )
    }
}

/// A failure in the epistemic layer.
#[derive(Debug, thiserror::Error)]
pub enum EpistemicError {
    /// A promotion the deterministic table forbids.
    #[error("{0}")]
    Promotion(PromotionDenied),
    /// A value failed validation.
    #[error("epistemic validation failed for {field}: {reason}")]
    Validation {
        /// The field that failed.
        field: String,
        /// Why it failed.
        reason: String,
    },
    /// A write into `/world` was refused.
    #[error("{0}")]
    World(WorldRefusal),
    /// A justification would have introduced a cycle.
    #[error("justification cycle through {0}")]
    Cycle(String),
    /// The `ValidationBarrier` found a defect and refused the commit.
    #[error("validation barrier refused the write: {0}")]
    Barrier(String),
    /// A named record does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// A tabular store operation failed.
    #[error("epistemic database error: {0}")]
    Db(String),
    /// An RDF store operation failed.
    #[error("epistemic graph error: {0}")]
    Graph(String),
    /// A document could not be encoded or decoded.
    #[error("epistemic codec error: {0}")]
    Codec(String),
    /// The configuration or an argument is invalid.
    #[error("epistemic config error: {0}")]
    Config(String),
    /// A component this crate does not have failed.
    #[error("epistemic internal error: {0}")]
    Internal(String),
}

impl EpistemicError {
    /// The stable short code, mirroring `MmError::code`.
    pub fn code(&self) -> &'static str {
        match self {
            EpistemicError::Promotion(_) => "epistemic.promotion",
            EpistemicError::Validation { .. } => "epistemic.validation",
            EpistemicError::World(_) => "epistemic.world",
            EpistemicError::Cycle(_) => "epistemic.cycle",
            EpistemicError::Barrier(_) => "epistemic.barrier",
            EpistemicError::NotFound(_) => "epistemic.not_found",
            EpistemicError::Db(_) => "epistemic.store",
            EpistemicError::Graph(_) => "epistemic.graph",
            EpistemicError::Codec(_) => "epistemic.codec",
            EpistemicError::Config(_) => "epistemic.config",
            EpistemicError::Internal(_) => "epistemic.internal",
        }
    }

    /// A validation failure for one field.
    pub fn validation(field: impl Into<String>, reason: impl Into<String>) -> Self {
        EpistemicError::Validation {
            field: field.into(),
            reason: reason.into(),
        }
    }

    /// True when this refusal is the promotion guard declining.
    pub fn is_promotion(&self) -> bool {
        matches!(self, EpistemicError::Promotion(_))
    }
}

/// An epistemic result.
pub type Result<T> = std::result::Result<T, EpistemicError>;

impl From<mm_core::MmError> for EpistemicError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(m) => EpistemicError::Db(m),
            mm_core::MmError::Graph(m) => EpistemicError::Graph(m),
            mm_core::MmError::Config(m) => EpistemicError::Config(m),
            mm_core::MmError::Codec(m) => EpistemicError::Codec(m),
            other => EpistemicError::Internal(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for EpistemicError {
    fn from(e: serde_json::Error) -> Self {
        EpistemicError::Codec(e.to_string())
    }
}

impl From<std::io::Error> for EpistemicError {
    fn from(e: std::io::Error) -> Self {
        EpistemicError::Config(e.to_string())
    }
}

/// A ULID rendered for an error payload.
pub fn id_text(id: &Ulid) -> String {
    mm_core::ulid_string(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_namespaced() {
        let cases: Vec<EpistemicError> = vec![
            EpistemicError::Promotion(PromotionDenied {
                from: EpistemicStatus::Assumed,
                to: EpistemicStatus::Observed,
                reason: "an assumption is never an observation",
            }),
            EpistemicError::validation("confidence", "out of range"),
            EpistemicError::World(WorldRefusal {
                claim: "01h".into(),
                status: EpistemicStatus::Assumed,
                reason: "only OBSERVED/VERIFIED".into(),
            }),
            EpistemicError::Cycle("01h -> 01h".into()),
            EpistemicError::Barrier("shape violation".into()),
        ];
        for error in cases {
            assert!(error.code().starts_with("epistemic."), "{}", error.code());
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn a_kernel_error_maps_onto_its_own_variant() {
        let mapped: EpistemicError = mm_core::MmError::Store("boom".into()).into();
        assert_eq!(mapped.code(), "epistemic.store");
        let mapped: EpistemicError = mm_core::MmError::Graph("boom".into()).into();
        assert_eq!(mapped.code(), "epistemic.graph");
    }
}
