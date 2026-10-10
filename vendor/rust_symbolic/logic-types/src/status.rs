//! The result status vocabulary (`full_system_design.md` §11.1, §13).
//!
//! The distinctions matter: `Unknown` ≠ `false`, `Timeout` ≠ `Disproved`,
//! and `x = 4.73` ≠ `x = 4.73 ± 0.12`.

/// How a reasoning operation concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResultStatus {
    /// Logically established.
    Proved,
    /// Logically refuted.
    Disproved,
    /// A model/satisfying assignment was found.
    Satisfied,
    /// No satisfying assignment exists.
    Unsatisfied,
    /// The model is consistent (no contradiction found).
    Consistent,
    /// A contradiction was found.
    Inconsistent,
    /// A probability distribution over outcomes.
    Probabilistic,
    /// A numerical/approximate answer under a declared tolerance.
    Approximate,
    /// The engine could not decide.
    Unknown,
    /// The answer is not determined by the model/assumptions.
    Underdetermined,
    /// The resource budget was exhausted.
    Timeout,
    /// The requested semantics/fragment are not supported.
    Unsupported,
}

impl ResultStatus {
    /// Whether this status is a *conclusive* logical answer (as opposed to a
    /// degraded, uncertain, or resource-limited one).
    pub fn is_definitive(self) -> bool {
        matches!(
            self,
            ResultStatus::Proved
                | ResultStatus::Disproved
                | ResultStatus::Satisfied
                | ResultStatus::Unsatisfied
        )
    }
}
