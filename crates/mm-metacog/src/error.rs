//! What `mm-metacog` refuses, and why.
//!
//! Every refusal here is *typed*, because each one is a different thing an
//! operator can act on: a validation refusal names the field, a budget refusal
//! names the resource the episode ran out of, a scan refusal names the schema
//! violation, and a stop is not a refusal at all. A single `Error(String)` would
//! make `episode run`, `episode budget-audit` and `episode verify` print the same
//! thing.
//!
//! A [`StopCause`] is deliberately *not* an error: stopping early is the
//! controller succeeding at spending the least cognition that can change the
//! decision. Only a controller that cannot continue at all refuses.

use std::fmt;

use crate::budget::BudgetResource;

/// Why a budget dimension ran out.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BudgetExceeded {
    /// The dimension that ran out.
    pub resource: BudgetResource,
    /// The limit the caller set.
    pub limit: f64,
    /// What had already been spent.
    pub spent: f64,
    /// What this debit asked for.
    pub requested: f64,
}

impl std::error::Error for BudgetExceeded {}

impl fmt::Display for BudgetExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "budget exceeded on {}: requested {} with {} spent of {}",
            self.resource, self.requested, self.spent, self.limit
        )
    }
}

/// Everything this crate can refuse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MetacogError {
    /// A field failed its own validation.
    #[error("{field}: {detail}")]
    Validation {
        /// The field that failed.
        field: &'static str,
        /// What was wrong with it.
        detail: String,
    },
    /// A budget dimension ran out. Never clamped: the debit is refused whole.
    #[error("{0}")]
    Budget(#[from] BudgetExceeded),
    /// The episode could not be turned into a program.
    #[error("compile: {0}")]
    Compile(String),
    /// The scan was refused, and its schema violation is named.
    #[error("scan: {0}")]
    Scan(String),
    /// The program could not be lowered to an execution document.
    #[error("lower: {0}")]
    Lower(String),
    /// A program graph is not a DAG, or references a node that is not in it.
    #[error("graph: {0}")]
    Graph(String),
    /// The store refused a write or a read.
    #[error("store: {0}")]
    Store(String),
    /// The graph store refused a write.
    #[error("graph store: {0}")]
    GraphStore(String),
    /// A codec refused a document.
    #[error("codec: {0}")]
    Codec(String),
    /// An identifier is not one this system can have produced.
    #[error("id: {0}")]
    Id(String),
    /// An operation was requested that has no node.
    #[error("no such node: {0}")]
    NoSuchNode(u16),
    /// An internal invariant did not hold.
    #[error("internal: {0}")]
    Internal(String),
}

impl MetacogError {
    /// A validation refusal.
    pub fn validation(field: &'static str, detail: impl Into<String>) -> Self {
        MetacogError::Validation {
            field,
            detail: detail.into(),
        }
    }

    /// The stable code the logger records for this refusal.
    pub fn code(&self) -> &'static str {
        match self {
            MetacogError::Validation { .. } => "validation",
            MetacogError::Budget(_) => "budget",
            MetacogError::Compile(_) => "compile",
            MetacogError::Scan(_) => "scan",
            MetacogError::Lower(_) => "lower",
            MetacogError::Graph(_) => "graph",
            MetacogError::Store(_) => "store",
            MetacogError::GraphStore(_) => "graph_store",
            MetacogError::Codec(_) => "codec",
            MetacogError::Id(_) => "id",
            MetacogError::NoSuchNode(_) => "no_such_node",
            MetacogError::Internal(_) => "internal",
        }
    }

    /// True when the refusal is a budget dimension running out, which the loop
    /// turns into a stop rather than an abort.
    pub fn is_budget(&self) -> bool {
        matches!(self, MetacogError::Budget(_))
    }
}

impl From<serde_json::Error> for MetacogError {
    fn from(e: serde_json::Error) -> Self {
        MetacogError::Internal(format!("serialization: {e}"))
    }
}

impl From<mm_core::MmError> for MetacogError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(m) => MetacogError::Store(m),
            mm_core::MmError::Graph(m) => MetacogError::GraphStore(m),
            mm_core::MmError::Codec(m) => MetacogError::Codec(m),
            other => MetacogError::Internal(other.to_string()),
        }
    }
}

impl From<MetacogError> for mm_core::MmError {
    fn from(e: MetacogError) -> Self {
        match e {
            MetacogError::Store(m) => mm_core::MmError::Store(m),
            MetacogError::GraphStore(m) => mm_core::MmError::Graph(m),
            MetacogError::Codec(m) => mm_core::MmError::Codec(m),
            other => mm_core::MmError::Internal(format!("{} ({})", other, other.code())),
        }
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, MetacogError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_has_a_distinct_code() {
        let cases = [
            MetacogError::validation("tier", "out of range"),
            MetacogError::Budget(BudgetExceeded {
                resource: BudgetResource::Ops,
                limit: 1.0,
                spent: 1.0,
                requested: 1.0,
            }),
            MetacogError::Compile("no ops".into()),
            MetacogError::Scan("bad json".into()),
            MetacogError::Lower("cycle".into()),
            MetacogError::Graph("cycle".into()),
            MetacogError::Store("closed".into()),
            MetacogError::GraphStore("closed".into()),
            MetacogError::Codec("bad turtle".into()),
            MetacogError::Id("nil".into()),
            MetacogError::NoSuchNode(7),
            MetacogError::Internal("bug".into()),
        ];
        let codes: std::collections::BTreeSet<&str> = cases.iter().map(|c| c.code()).collect();
        assert_eq!(codes.len(), cases.len(), "codes must distinguish refusals");
    }

    #[test]
    fn a_budget_refusal_names_the_resource() {
        let e = MetacogError::Budget(BudgetExceeded {
            resource: BudgetResource::Cost,
            limit: 0.5,
            spent: 0.5,
            requested: 0.25,
        });
        let rendered = e.to_string();
        assert!(rendered.contains("cost"), "{rendered}");
        assert!(rendered.contains("0.5"), "{rendered}");
        assert!(e.is_budget());
        assert!(!MetacogError::Compile("x".into()).is_budget());
    }
}
