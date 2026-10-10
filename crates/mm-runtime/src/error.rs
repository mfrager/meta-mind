//! Everything the autonomy layer can refuse.
//!
//! The variants are separated by *who is being told*, because that is what a caller does
//! with an error: a [`LoopError::BudgetDenied`] is a run that stopped where it was told
//! to, a [`LoopError::GcRefused`] is the ledger guard doing its job, and
//! [`LoopError::Stage`] is a stage that failed for a reason the run has to record and
//! then stop. Collapsing them into one string would make \"the run hit its cap\" and \"the
//! run is broken\" the same answer.

use mm_core::MmError;

/// Anything the autonomy layer refuses.
#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    /// A field was missing, empty or malformed.
    #[error("validation: {field}: {detail}")]
    Validation {
        /// The field.
        field: &'static str,
        /// What was wrong with it.
        detail: String,
    },
    /// A stage's debit would cross the envelope.
    #[error("budget: {bucket} needs {needed} but holds {remaining}")]
    BudgetDenied {
        /// The bucket's name.
        bucket: String,
        /// What the stage asked for.
        needed: f64,
        /// What the bucket held.
        remaining: f64,
    },
    /// A stage failed.
    #[error("stage {idx} ({stage}) failed: {detail}")]
    Stage {
        /// Its position.
        idx: usize,
        /// Its name.
        stage: String,
        /// Why.
        detail: String,
    },
    /// A GC action was refused: a protected ledger subject, or a non-reversible action.
    #[error("gc refused {subject}: {reason}")]
    GcRefused {
        /// The subject the action was about.
        subject: String,
        /// Why it was refused.
        reason: String,
    },
    /// A design revision was attempted through something other than a promoted change set.
    #[error("design: {0}")]
    Design(String),
    /// The self-engineering pipeline (Phase 11) refused something.
    #[error("self-eng: {0}")]
    SelfEng(String),
    /// A module could not be loaded.
    #[error("load: {0}")]
    Load(String),
    /// A replay did not reproduce the run.
    #[error("replay: {0}")]
    Replay(String),
    /// The store refused a write.
    #[error("store: {0}")]
    Store(String),
}

impl LoopError {
    /// A validation refusal.
    pub fn validation(field: &'static str, detail: impl Into<String>) -> Self {
        LoopError::Validation {
            field,
            detail: detail.into(),
        }
    }

    /// A stage refusal.
    pub fn stage(idx: usize, stage: &str, detail: impl Into<String>) -> Self {
        LoopError::Stage {
            idx,
            stage: stage.to_string(),
            detail: detail.into(),
        }
    }

    /// A stable code, so a caller can branch without parsing a message.
    pub fn code(&self) -> &'static str {
        match self {
            LoopError::Validation { .. } => "validation",
            LoopError::BudgetDenied { .. } => "budget_denied",
            LoopError::Stage { .. } => "stage_failed",
            LoopError::GcRefused { .. } => "gc_refused",
            LoopError::Design(_) => "design",
            LoopError::SelfEng(_) => "self_eng",
            LoopError::Load(_) => "load",
            LoopError::Replay(_) => "replay",
            LoopError::Store(_) => "store",
        }
    }

    /// True when this is the hard-budget refusal.
    pub fn is_budget_denied(&self) -> bool {
        matches!(self, LoopError::BudgetDenied { .. })
    }

    /// True when the ledger guard refused a collection.
    pub fn is_gc_refused(&self) -> bool {
        matches!(self, LoopError::GcRefused { .. })
    }
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, LoopError>;

impl From<mm_selfeng::error::SelfEngError> for LoopError {
    fn from(e: mm_selfeng::error::SelfEngError) -> Self {
        // A validation refusal names its field, which a caller can act on; every other
        // refusal is carried verbatim, because Phase 11 already typed it.
        match e {
            mm_selfeng::error::SelfEngError::Validation { field, detail } => {
                LoopError::Validation { field, detail }
            }
            other => LoopError::SelfEng(other.to_string()),
        }
    }
}

impl From<LoopError> for MmError {
    /// The kernel error carries the refusal's message, and the refusal's own `code()`
    /// stays on the `LoopError` for a caller that still holds it. The autonomy layer's
    /// refusals are typed for the loop and rendered for the operator; the CLI reads the
    /// message and decides its exit status from the variant it caught.
    fn from(e: LoopError) -> Self {
        MmError::Internal(e.to_string())
    }
}

impl From<MmError> for LoopError {
    fn from(e: MmError) -> Self {
        match e {
            MmError::Config(detail) | MmError::Internal(detail) => LoopError::Store(detail),
            other => LoopError::Store(other.to_string()),
        }
    }
}
