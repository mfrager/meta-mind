//! The unified exact/approximate execution contract
//! (`full_system_design.md` §12.3, requirement 17).

/// The guarantee class of a backend or result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Exactness {
    /// Exact by construction (symbolic algebra, exact arithmetic).
    Exact,
    /// Backed by an independently checkable certificate.
    Certified,
    /// Numerically stable/accurate under a declared tolerance.
    Approximate,
    /// A probability distribution / Monte Carlo estimate.
    Probabilistic,
    /// Produced by sampling (no exact/certified guarantee).
    Sampled,
    /// Produced by a learned model.
    Learned,
    /// Produced by a heuristic search.
    Heuristic,
    /// No established guarantee.
    Unverified,
}
