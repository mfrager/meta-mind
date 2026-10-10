//! What `mm-selfeng` refuses, and why.
//!
//! Every refusal here is a different thing an operator can act on: a validation
//! refusal names the field, an escape names the path, a build refusal carries the
//! exit code and the tail of the output, a budget refusal names the limit and what
//! was already spent, and a gate refusal names the rule. A single `Error(String)`
//! would make `changeset new`, `sandbox run`, `promote` and `budget show` print the
//! same thing, and the phase's gate asks for a rejection *with a written reason* —
//! which starts with the reason being a typed thing rather than a sentence.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::budget::BudgetKind;

/// A debit that would have crossed a limit.
///
/// The numbers are all reported because each answers a different question: the limit
/// is the policy, the spent side is what happened, and the requested amount is what
/// the caller was trying to do when it was refused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetDenied {
    /// The dimension that ran out.
    pub kind: BudgetKind,
    /// The period the row belongs to.
    pub period: String,
    /// The limit the row carries.
    pub limit: f64,
    /// What was already spent.
    pub spent: f64,
    /// What this debit asked for.
    pub requested: f64,
}

impl fmt::Display for BudgetDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "budget {} for period {} denies {}: {} spent of {}",
            self.kind.as_str(),
            self.period,
            self.requested,
            self.spent,
            self.limit
        )
    }
}

// A refusal is an error the caller may hold on its own, so it carries the trait the
// `#[error("{0}")]` transclusion in `SelfEngError::BudgetDenied` needs.
impl std::error::Error for BudgetDenied {}

/// Everything this crate can refuse.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SelfEngError {
    /// A field failed its own validation.
    #[error("{field}: {detail}")]
    Validation {
        /// The field that failed.
        field: &'static str,
        /// What was wrong with it.
        detail: String,
    },
    /// The change set is not a usable proposal.
    #[error("changeset: {0}")]
    Changeset(String),
    /// The sandbox could not be prepared, applied to, or read.
    #[error("sandbox: {0}")]
    Sandbox(String),
    /// A path tried to leave the sandbox root.
    #[error("sandbox escape: {path} resolves outside the sandbox root")]
    Escape {
        /// The path that was refused.
        path: String,
    },
    /// A build or test run failed, with what the command said.
    #[error("{dir}: {command} exited {exit_code}")]
    Build {
        /// The sandbox directory.
        dir: String,
        /// The command that ran.
        command: String,
        /// Its exit code.
        exit_code: i32,
    },
    /// A benchmark could not be read.
    #[error("bench: {0}")]
    Bench(String),
    /// The promotion gate refused, or the evidence bundle is not usable.
    #[error("gate: {0}")]
    Gate(String),
    /// A budget dimension denied a debit. Never clamped: the row is left unchanged.
    #[error("{0}")]
    BudgetDenied(#[from] BudgetDenied),
    /// The budget ledger could not be read or written.
    #[error("budget: {0}")]
    Budget(String),
    /// The store refused a read or a write.
    #[error("store: {0}")]
    Store(String),
    /// The graph store refused a write.
    #[error("graph: {0}")]
    Graph(String),
    /// A git command failed. The sandbox is a worktree, so this is a first-class
    /// refusal rather than a fallback.
    #[error("git {command} exited {status}: {stderr}")]
    Git {
        /// The git subcommand that ran.
        command: String,
        /// Its exit status.
        status: i32,
        /// Its standard error, trimmed.
        stderr: String,
    },
    /// A rollback could not be applied.
    #[error("rollback: {0}")]
    Rollback(String),
    /// The filesystem refused.
    #[error("io: {0}")]
    Io(String),
    /// An internal invariant did not hold.
    #[error("internal: {0}")]
    Internal(String),
}

impl SelfEngError {
    /// A validation refusal.
    pub fn validation(field: &'static str, detail: impl Into<String>) -> Self {
        SelfEngError::Validation {
            field,
            detail: detail.into(),
        }
    }

    /// The stable code the logger records for this refusal.
    pub fn code(&self) -> &'static str {
        match self {
            SelfEngError::Validation { .. } => "validation",
            SelfEngError::Changeset(_) => "changeset",
            SelfEngError::Sandbox(_) => "sandbox",
            SelfEngError::Escape { .. } => "sandbox.escape",
            SelfEngError::Build { .. } => "sandbox.build",
            SelfEngError::Bench(_) => "bench",
            SelfEngError::Gate(_) => "gate",
            SelfEngError::BudgetDenied(_) => "budget.denied",
            SelfEngError::Budget(_) => "budget",
            SelfEngError::Store(_) => "store",
            SelfEngError::Graph(_) => "graph",
            SelfEngError::Git { .. } => "git",
            SelfEngError::Rollback(_) => "rollback",
            SelfEngError::Io(_) => "io",
            SelfEngError::Internal(_) => "internal",
        }
    }

    /// True when the refusal is a budget dimension denying a debit, which the caller
    /// turns into "the change is refused" rather than "the command broke".
    pub fn is_budget_denied(&self) -> bool {
        matches!(self, SelfEngError::BudgetDenied(_))
    }
}

impl From<std::io::Error> for SelfEngError {
    fn from(e: std::io::Error) -> Self {
        SelfEngError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for SelfEngError {
    fn from(e: serde_json::Error) -> Self {
        SelfEngError::Internal(format!("serialization: {e}"))
    }
}

impl From<mm_core::MmError> for SelfEngError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(m) => SelfEngError::Store(m),
            mm_core::MmError::Graph(m) => SelfEngError::Graph(m),
            other => SelfEngError::Internal(other.to_string()),
        }
    }
}

impl From<SelfEngError> for mm_core::MmError {
    fn from(e: SelfEngError) -> Self {
        match e {
            SelfEngError::Store(m) => mm_core::MmError::Store(m),
            SelfEngError::Graph(m) => mm_core::MmError::Graph(m),
            other => mm_core::MmError::Internal(format!("{} ({})", other, other.code())),
        }
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, SelfEngError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_has_a_distinct_code() {
        let cases = [
            SelfEngError::validation("reason", "must not be empty"),
            SelfEngError::Changeset("no artifacts".into()),
            SelfEngError::Sandbox("no such worktree".into()),
            SelfEngError::Escape {
                path: "etc/shadow".into(),
            },
            SelfEngError::Build {
                dir: "data/sandbox/x".into(),
                command: "cargo build".into(),
                exit_code: 101,
            },
            SelfEngError::Bench("no float".into()),
            SelfEngError::Gate("no evidence".into()),
            SelfEngError::BudgetDenied(BudgetDenied {
                kind: BudgetKind::Evolution,
                period: DEFAULT_PERIOD_FOR_TEST.into(),
                limit: 20.0,
                spent: 20.0,
                requested: 1.0,
            }),
            SelfEngError::Budget("no row".into()),
            SelfEngError::Store("closed".into()),
            SelfEngError::Graph("closed".into()),
            SelfEngError::Git {
                command: "worktree add".into(),
                status: 128,
                stderr: "not a git repository".into(),
            },
            SelfEngError::Rollback("hash mismatch".into()),
            SelfEngError::Io("permission denied".into()),
            SelfEngError::Internal("bug".into()),
        ];
        let codes: std::collections::BTreeSet<&str> = cases.iter().map(|c| c.code()).collect();
        assert_eq!(codes.len(), cases.len(), "codes must distinguish refusals");
    }

    const DEFAULT_PERIOD_FOR_TEST: &str = "cycle-1";

    #[test]
    fn a_budget_refusal_names_the_dimension_and_the_numbers() {
        let error = SelfEngError::BudgetDenied(BudgetDenied {
            kind: BudgetKind::Improvement,
            period: "cycle-1".into(),
            limit: 50.0,
            spent: 49.5,
            requested: 1.0,
        });
        let rendered = error.to_string();
        assert!(rendered.contains("improvement"), "{rendered}");
        assert!(rendered.contains("49.5"), "{rendered}");
        assert!(rendered.contains("50"), "{rendered}");
        assert!(error.is_budget_denied());
        assert!(!SelfEngError::Gate("x".into()).is_budget_denied());
    }
}
