//! What `mm-metaanalysis` refuses, and why.
//!
//! Every refusal here is *typed*, because each one is a different thing an operator
//! can act on: a validation refusal names the field, a resolution refusal says the
//! prediction was already resolved (the append-only ledger's whole point), a
//! diagnosis refusal names the schema or the provider that failed. A single
//! `Error(String)` would make `calibrate`, `meta analyze` and `meta lessons` print
//! the same thing.

/// Everything this crate can refuse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MetaError {
    /// A field failed its own validation.
    #[error("{field}: {detail}")]
    Validation {
        /// The field that failed.
        field: &'static str,
        /// What was wrong with it.
        detail: String,
    },
    /// The prediction was already resolved. A resolution is written once: the ledger
    /// scores a claim staked before the outcome, and re-resolving until the score
    /// looks good is the failure mode the schema exists to prevent.
    #[error("prediction {0} was already resolved")]
    AlreadyResolved(String),
    /// No prediction with that identifier exists.
    #[error("no prediction {0} is in the ledger")]
    UnknownPrediction(String),
    /// A stored row is not the shape this crate writes.
    #[error("malformed row in {table}: {detail}")]
    Malformed {
        /// The table the row came from.
        table: &'static str,
        /// What was wrong with it.
        detail: String,
    },
    /// The bounded diagnosis pass failed, and its schema violation is named.
    #[error("diagnosis: {0}")]
    Diagnosis(String),
    /// The heuristic classifier could classify nothing, which is a bug rather than a
    /// refusal: its cue table always falls back to a class.
    #[error("no error class could be assigned to episode {0}")]
    Unclassifiable(String),
    /// A record could not be read, or is not the document this crate expects.
    #[error("record: {0}")]
    Record(String),
    /// The calibrator cannot answer: no labeled point of that class meets the
    /// requested precision.
    #[error("calibration: {0}")]
    Calibration(String),
    /// The store refused a read or a write.
    #[error("store: {0}")]
    Store(String),
    /// The graph store refused a write.
    #[error("graph store: {0}")]
    GraphStore(String),
    /// The library refused the lesson.
    #[error("library: {0}")]
    Library(String),
    /// An internal invariant did not hold.
    #[error("internal: {0}")]
    Internal(String),
}

impl MetaError {
    /// A validation refusal.
    pub fn validation(field: &'static str, detail: impl Into<String>) -> Self {
        MetaError::Validation {
            field,
            detail: detail.into(),
        }
    }

    /// The stable code the logger records for this refusal.
    pub fn code(&self) -> &'static str {
        match self {
            MetaError::Validation { .. } => "validation",
            MetaError::AlreadyResolved(_) => "already_resolved",
            MetaError::UnknownPrediction(_) => "unknown_prediction",
            MetaError::Malformed { .. } => "malformed",
            MetaError::Diagnosis(_) => "diagnosis",
            MetaError::Unclassifiable(_) => "unclassifiable",
            MetaError::Record(_) => "record",
            MetaError::Calibration(_) => "calibration",
            MetaError::Store(_) => "store",
            MetaError::GraphStore(_) => "graph_store",
            MetaError::Library(_) => "library",
            MetaError::Internal(_) => "internal",
        }
    }
}

impl From<std::io::Error> for MetaError {
    fn from(e: std::io::Error) -> Self {
        MetaError::Record(format!("io: {e}"))
    }
}

impl From<serde_json::Error> for MetaError {
    fn from(e: serde_json::Error) -> Self {
        MetaError::Record(format!("serialization: {e}"))
    }
}

impl From<mm_core::MmError> for MetaError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(m) => MetaError::Store(m),
            mm_core::MmError::Graph(m) => MetaError::GraphStore(m),
            other => MetaError::Internal(other.to_string()),
        }
    }
}

impl From<MetaError> for mm_core::MmError {
    fn from(e: MetaError) -> Self {
        match e {
            MetaError::Store(m) => mm_core::MmError::Store(m),
            MetaError::GraphStore(m) => mm_core::MmError::Graph(m),
            other => mm_core::MmError::Internal(format!("{} ({})", other, other.code())),
        }
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, MetaError>;

/// A refusal that names the table a malformed row came from.
pub(crate) fn malformed(table: &'static str, detail: impl Into<String>) -> MetaError {
    MetaError::Malformed {
        table,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_has_a_distinct_code() {
        let cases = [
            MetaError::validation("class", "empty"),
            MetaError::AlreadyResolved("01h".into()),
            MetaError::UnknownPrediction("01h".into()),
            malformed("prediction_ledger", "no probability"),
            MetaError::Diagnosis("bad json".into()),
            MetaError::Unclassifiable("01h".into()),
            MetaError::Record("missing".into()),
            MetaError::Calibration("no threshold".into()),
            MetaError::Store("closed".into()),
            MetaError::GraphStore("closed".into()),
            MetaError::Library("shape".into()),
            MetaError::Internal("bug".into()),
        ];
        let codes: std::collections::BTreeSet<&str> = cases.iter().map(|c| c.code()).collect();
        assert_eq!(codes.len(), cases.len(), "codes must distinguish refusals");
    }

    #[test]
    fn a_validation_refusal_names_the_field() {
        let e = MetaError::validation("probability", "must be in [0,1], got 1.5");
        assert!(e.to_string().starts_with("probability:"));
    }

    #[test]
    fn an_already_resolved_prediction_says_which_one() {
        let e = MetaError::AlreadyResolved("01h00000000000000000000000".into());
        assert!(e.to_string().contains("01h00000000000000000000000"));
        assert_eq!(e.code(), "already_resolved");
    }
}
