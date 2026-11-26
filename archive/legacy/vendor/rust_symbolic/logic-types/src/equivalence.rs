//! The semantic-strength contract of a lowering/transformation
//! (`full_system_design.md` §12.5).

/// What a transformation preserved (or failed to preserve) between a source
/// and target representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EquivalenceStatus {
    /// Meaning is preserved exactly.
    Exact,
    /// Equivalent only under the recorded assumptions.
    EquivalentUnderAssumptions,
    /// A deliberate semantic relaxation (e.g. Boolean → real-valued).
    Relaxation,
    /// A numeric/sampling approximation with an error contract.
    Approximation,
    /// An embedding (symbols → vectors) — a computational representation,
    /// not a meaning-preserving rewrite.
    Embedding,
}
