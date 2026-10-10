//! The semantic effect system (`full_system_design.md` §5.4).
//!
//! Every expression/statement declares which semantic domains it touches, so
//! the compiler can reject dangerous combinations early (e.g. a probabilistic
//! statement compiling into classical truth, or a causal edge becoming an
//! ordinary implication).

use crate::ty::Type;

/// A semantic domain touched by a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SemanticEffect {
    ExactArithmetic,
    Propositional,
    FirstOrder,
    HigherOrder,
    Probability,
    FuzzyTruth,
    CausalIntervention,
    Counterfactual,
    Temporal,
    Modal,
    Deontic,
    Default,
    Planning,
    Optimization,
    Game,
    Learned,
    Approximate,
    Sampled,
}

/// Anything that participates in the effect system declares the semantic
/// domains it touches.
pub trait HasEffects {
    fn effects(&self) -> Vec<SemanticEffect>;
}

impl HasEffects for Type {
    fn effects(&self) -> Vec<SemanticEffect> {
        match self {
            Type::Prop | Type::Bool => vec![SemanticEffect::Propositional],
            Type::Probability => vec![SemanticEffect::Probability],
            Type::TruthDegree => vec![SemanticEffect::FuzzyTruth],
            Type::Int
            | Type::Nat
            | Type::Rational
            | Type::Real
            | Type::Decimal
            | Type::Float
            | Type::BitVec
            | Type::Complex => vec![SemanticEffect::ExactArithmetic],
            _ => vec![],
        }
    }
}
