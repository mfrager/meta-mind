//! What `mm-decision` refuses, and why.
//!
//! The refusals are typed because each one is a different thing an operator acts
//! on: a *validation* refusal names the field, *unavailable* says the core cannot
//! answer at all (never a fabricated answer), *schema* names the structured-output
//! violation, *calibration* names the class with too little data, and
//! *comparable* is the comparison-integrity refusal — two things that must not be
//! compared, which is a refusal rather than an error in the caller.

use std::fmt;

/// Everything this crate can refuse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DecisionError {
    /// A field failed its own validation.
    #[error("{field}: {detail}")]
    Validation {
        /// The field that failed.
        field: &'static str,
        /// What was wrong with it.
        detail: String,
    },
    /// A core cannot answer here: no head is loaded, no provider is reachable, or
    /// no rule matched and the question allows no default. Never a guess.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// A structured answer violated its schema, or a field was out of range.
    #[error("schema: {0}")]
    Schema(String),
    /// The calibration data is insufficient for the class.
    #[error("calibration: {0}")]
    Calibration(String),
    /// The comparison is not admissible.
    #[error("comparison: {0}")]
    Comparison(String),
    /// The store refused a write or a read.
    #[error("store: {0}")]
    Store(String),
    /// A codec refused a document.
    #[error("codec: {0}")]
    Codec(String),
    /// An identifier is not one this system can have produced.
    #[error("id: {0}")]
    Id(String),
    /// A bug in this crate.
    #[error("internal: {0}")]
    Internal(String),
}

impl DecisionError {
    /// A validation refusal, with the field named.
    pub fn validation(field: &'static str, detail: impl Into<String>) -> Self {
        DecisionError::Validation {
            field,
            detail: detail.into(),
        }
    }

    /// The stable code an operator filters on.
    pub fn code(&self) -> &'static str {
        match self {
            DecisionError::Validation { .. } => "validation",
            DecisionError::Unavailable(_) => "unavailable",
            DecisionError::Schema(_) => "schema",
            DecisionError::Calibration(_) => "calibration",
            DecisionError::Comparison(_) => "comparison",
            DecisionError::Store(_) => "store",
            DecisionError::Codec(_) => "codec",
            DecisionError::Id(_) => "id",
            DecisionError::Internal(_) => "internal",
        }
    }

    /// True when the core could not answer rather than answering badly.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, DecisionError::Unavailable(_))
    }
}

impl From<serde_json::Error> for DecisionError {
    fn from(e: serde_json::Error) -> Self {
        DecisionError::Codec(e.to_string())
    }
}

impl From<mm_core::MmError> for DecisionError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(message) => DecisionError::Store(message),
            mm_core::MmError::Codec(message) => DecisionError::Codec(message),
            other => DecisionError::Internal(other.to_string()),
        }
    }
}

/// The kernel's error type, so a command can `?` through this crate.
impl From<DecisionError> for mm_core::MmError {
    fn from(e: DecisionError) -> Self {
        match e {
            DecisionError::Store(message) => mm_core::MmError::Store(message),
            DecisionError::Codec(message) => mm_core::MmError::Codec(message),
            other => mm_core::MmError::Internal(format!("{} ({})", other, other.code())),
        }
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, DecisionError>;

/// A refusal rendered for a human, without the variant name.
impl fmt::Display for UnavailableCore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} cannot answer: {}", self.core, self.reason)
    }
}

/// A core that could not answer, kept as a value so a conformance report can carry
/// it rather than losing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableCore {
    /// The core's wire name.
    pub core: String,
    /// Why it could not answer.
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_has_a_distinct_code() {
        let cases = [
            DecisionError::validation("field", "detail"),
            DecisionError::Unavailable("no provider".into()),
            DecisionError::Schema("bad json".into()),
            DecisionError::Calibration("n = 0".into()),
            DecisionError::Comparison("different units".into()),
            DecisionError::Store("closed".into()),
            DecisionError::Codec("bad toml".into()),
            DecisionError::Id("nil".into()),
            DecisionError::Internal("bug".into()),
        ];
        let codes: std::collections::BTreeSet<&str> = cases.iter().map(|c| c.code()).collect();
        assert_eq!(codes.len(), cases.len(), "codes must distinguish refusals");
        assert!(DecisionError::Unavailable("x".into()).is_unavailable());
        assert!(!DecisionError::Schema("x".into()).is_unavailable());
    }
}
