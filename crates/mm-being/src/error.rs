//! The being error type and the payloads it carries.
//!
//! Every refusal in this crate is *typed*: an invariant violation names the
//! invariant, a denied promotion names the status it wanted and what was missing,
//! and an overspend names the balance it would have broken. A stringly-typed
//! error would leave `being verify` unable to say which of the four invariants
//! held, which is the whole point of the guard.
//!
//! The payload types live here rather than beside their domains so that the
//! guard, the domain modules, and the audit writer can all name them without
//! depending on each other.

use serde::{Deserialize, Serialize};

/// One of the four core invariants (plan §2).
///
/// The set is closed on purpose. A fifth invariant is a design change, not a
/// configuration option, so it would arrive with a new `InvariantCode` and a
/// migration — never with a string a caller can invent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
// The `No…` prefix is part of the invariant names in the plan (§2) and in the
// persisted `invariants.code` values, so it stays even though every variant shares it.
#[allow(clippy::enum_variant_names)]
pub enum InvariantCode {
    /// The being never invents a memory of something that did not happen.
    NoFabricatedAutobiography,
    /// An assumption never becomes an observation without an observation.
    NoAssumptionToObservation,
    /// An action is never claimed without evidence.
    NoActionWithoutEvidence,
    /// A terminal row is never rewritten.
    NoHistoryRewrite,
}

/// The invariants every kernel enforces, in a stable order.
pub const CORE_INVARIANTS: [InvariantCode; 4] = [
    InvariantCode::NoFabricatedAutobiography,
    InvariantCode::NoAssumptionToObservation,
    InvariantCode::NoActionWithoutEvidence,
    InvariantCode::NoHistoryRewrite,
];

impl InvariantCode {
    /// The stable wire name, recorded in `invariants.code` and in every violation.
    pub fn as_str(self) -> &'static str {
        match self {
            InvariantCode::NoFabricatedAutobiography => "no_fabricated_autobiography",
            InvariantCode::NoAssumptionToObservation => "no_assumption_to_observation",
            InvariantCode::NoActionWithoutEvidence => "no_action_without_evidence",
            InvariantCode::NoHistoryRewrite => "no_history_rewrite",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        CORE_INVARIANTS.into_iter().find(|c| c.as_str() == s)
    }

    /// The assertion stored alongside the code in the `invariants` table.
    pub fn assertion(self) -> &'static str {
        match self {
            InvariantCode::NoFabricatedAutobiography => {
                "the being never invents a memory of something that did not happen"
            }
            InvariantCode::NoAssumptionToObservation => {
                "an assumption never becomes an observation without an observation"
            }
            InvariantCode::NoActionWithoutEvidence => "an action is never claimed without evidence",
            InvariantCode::NoHistoryRewrite => "a terminal row is never rewritten",
        }
    }
}

impl std::fmt::Display for InvariantCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An operation the guard refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantViolation {
    /// Which invariant was about to be broken.
    pub code: InvariantCode,
    /// The operation, in its wire form.
    pub op: String,
    /// Why it was refused.
    pub reason: String,
}

impl std::fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} refused `{}`: {}", self.code, self.op, self.reason)
    }
}

/// A belief promotion that the evidence does not support.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionDenied {
    /// The proposition that would have been promoted.
    pub proposition: String,
    /// The status it held.
    pub from: String,
    /// The status it wanted.
    pub to: String,
    /// Why it was refused.
    pub reason: String,
}

impl std::fmt::Display for PromotionDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "promotion denied for `{}`: {} -> {} ({})",
            self.proposition, self.from, self.to, self.reason
        )
    }
}

/// A debit a budget policy refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetExceeded {
    /// The resource kind.
    pub kind: String,
    /// What was asked for.
    pub requested: f64,
    /// The balance it would have broken.
    pub balance: f64,
    /// The policy that refused it, or `default`.
    pub policy: String,
}

impl std::fmt::Display for BudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "budget exceeded for {}: requested {} against a balance of {} ({})",
            self.kind, self.requested, self.balance, self.policy
        )
    }
}

/// A status change the lifecycle does not allow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionDenied {
    /// The object kind: `goal` or `commitment`.
    pub subject: String,
    /// The status it held.
    pub from: String,
    /// The status it wanted.
    pub to: String,
    /// Why it was refused.
    pub reason: String,
}

impl std::fmt::Display for TransitionDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} transition denied: {} -> {} ({})",
            self.subject, self.from, self.to, self.reason
        )
    }
}

/// A failure in the being substrate.
#[derive(Debug, thiserror::Error)]
pub enum BeingError {
    /// The guard refused an operation.
    #[error("invariant violated: {0}")]
    Invariant(InvariantViolation),
    /// A belief could not be promoted.
    #[error("{0}")]
    Promotion(PromotionDenied),
    /// A spend was refused.
    #[error("{0}")]
    Budget(BudgetExceeded),
    /// A lifecycle transition was refused.
    #[error("{0}")]
    Transition(TransitionDenied),
    /// A core block would exceed its character limit.
    #[error("block `{kind}` is {actual} chars, over its {limit}-char limit")]
    BlockTooLarge {
        /// The block kind.
        kind: String,
        /// The declared limit.
        limit: usize,
        /// What was offered.
        actual: usize,
    },
    /// A tabular store operation failed.
    #[error("being database error: {0}")]
    Db(String),
    /// An RDF store operation failed.
    #[error("being graph error: {0}")]
    Graph(String),
    /// The configuration or an argument is invalid.
    #[error("being config error: {0}")]
    Config(String),
    /// A document could not be encoded or decoded.
    #[error("being codec error: {0}")]
    Codec(String),
    /// A named entity does not exist.
    #[error("being entity not found: {0}")]
    NotFound(String),
}

impl BeingError {
    /// A stable machine-readable kind, used in logs and by `being verify`.
    pub fn kind(&self) -> &'static str {
        match self {
            BeingError::Invariant(_) => "invariant_violation",
            BeingError::Promotion(_) => "promotion_denied",
            BeingError::Budget(_) => "budget_exceeded",
            BeingError::Transition(_) => "transition_denied",
            BeingError::BlockTooLarge { .. } => "block_too_large",
            BeingError::Db(_) => "db",
            BeingError::Graph(_) => "graph",
            BeingError::Config(_) => "config",
            BeingError::Codec(_) => "codec",
            BeingError::NotFound(_) => "not_found",
        }
    }

    /// A dotted code for CLI output, mirroring `MmError::code`.
    pub fn code(&self) -> &'static str {
        match self {
            BeingError::Invariant(v) => match v.code {
                InvariantCode::NoFabricatedAutobiography => "mm.being.invariant.autobiography",
                InvariantCode::NoAssumptionToObservation => "mm.being.invariant.assumption",
                InvariantCode::NoActionWithoutEvidence => "mm.being.invariant.evidence",
                InvariantCode::NoHistoryRewrite => "mm.being.invariant.history",
            },
            BeingError::Promotion(_) => "mm.being.promotion_denied",
            BeingError::Budget(_) => "mm.being.budget_exceeded",
            BeingError::Transition(_) => "mm.being.transition_denied",
            BeingError::BlockTooLarge { .. } => "mm.being.block_too_large",
            BeingError::Db(_) => "mm.being.db",
            BeingError::Graph(_) => "mm.being.graph",
            BeingError::Config(_) => "mm.being.config",
            BeingError::Codec(_) => "mm.being.codec",
            BeingError::NotFound(_) => "mm.being.not_found",
        }
    }
}

impl From<mm_core::MmError> for BeingError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(m) => BeingError::Db(m),
            mm_core::MmError::Graph(m) => BeingError::Graph(m),
            mm_core::MmError::Codec(m) => BeingError::Codec(m),
            other => BeingError::Config(other.to_string()),
        }
    }
}

impl From<std::io::Error> for BeingError {
    fn from(e: std::io::Error) -> Self {
        BeingError::Config(e.to_string())
    }
}

impl From<serde_json::Error> for BeingError {
    fn from(e: serde_json::Error) -> Self {
        BeingError::Codec(e.to_string())
    }
}

/// The being result alias.
pub type Result<T> = std::result::Result<T, BeingError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_core_invariants_are_distinct_and_round_trip() {
        for code in CORE_INVARIANTS {
            assert_eq!(InvariantCode::parse(code.as_str()), Some(code));
            assert!(!code.assertion().is_empty());
        }
        let mut names: Vec<&str> = CORE_INVARIANTS.iter().map(|c| c.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CORE_INVARIANTS.len());
    }

    #[test]
    fn every_error_kind_is_distinct() {
        let errors = [
            BeingError::Invariant(InvariantViolation {
                code: InvariantCode::NoHistoryRewrite,
                op: "op".into(),
                reason: "r".into(),
            }),
            BeingError::Promotion(PromotionDenied {
                proposition: "p".into(),
                from: "ASSUMED".into(),
                to: "OBSERVED".into(),
                reason: "r".into(),
            }),
            BeingError::Budget(BudgetExceeded {
                kind: "time".into(),
                requested: 1.0,
                balance: 0.0,
                policy: "default".into(),
            }),
            BeingError::Transition(TransitionDenied {
                subject: "goal".into(),
                from: "fulfilled".into(),
                to: "active".into(),
                reason: "terminal".into(),
            }),
            BeingError::BlockTooLarge {
                kind: "self".into(),
                limit: 1,
                actual: 2,
            },
            BeingError::Db("x".into()),
            BeingError::Graph("x".into()),
            BeingError::Config("x".into()),
            BeingError::Codec("x".into()),
            BeingError::NotFound("x".into()),
        ];
        let mut kinds: Vec<&str> = errors.iter().map(BeingError::kind).collect();
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), errors.len(), "error kinds must be distinct");
    }

    #[test]
    fn an_invariant_violation_names_its_code_in_message_and_iri() {
        let e = BeingError::Invariant(InvariantViolation {
            code: InvariantCode::NoAssumptionToObservation,
            op: "promote_assumption".into(),
            reason: "no observation".into(),
        });
        assert!(
            e.to_string().contains("no_assumption_to_observation"),
            "{e}"
        );
        assert_eq!(e.code(), "mm.being.invariant.assumption");
    }
}
