//! `logic-types` — the semantic type system and shared cross-crate vocabulary
//! for the full-scale heterogeneous reasoning system
//! (`full_system_design.md` §5, §12, §13).

pub mod action;
pub mod economy;
pub mod effect;
pub mod equivalence;
pub mod exactness;
pub mod information;
pub mod intern;
pub mod resource;
pub mod status;
pub mod ty;
pub mod unit;

pub use action::LlmOperation;
pub use economy::{
    decide_economy, pareto_front, score_candidate, CostProjection, EconomyActionKind,
    EconomyCandidate, EconomyDecision, EconomyDecisionRecord, EconomyError, EconomyObjective,
};
pub use effect::{HasEffects, SemanticEffect};
pub use equivalence::EquivalenceStatus;
pub use exactness::Exactness;
pub use information::{
    ChannelCapacity, CompressionRatio, ConditionalEntropy, ConditionalMutualInformation,
    CrossEntropy, DescriptionLength, Entropy, ExpectedInformationGain, ExpectedValueOfInformation,
    InformationCost, InformationKind, InformationLoss, InformationQuantity, InformationUnit,
    InformationValue, JSDivergence, JointEntropy, KLDivergence, MDLScore, MutualInformation,
    RateDistortion, Surprisal, ValueOfInformation,
};
pub use intern::{Interner, SymbolInterner};
pub use resource::{
    BudgetCheck, BudgetSet, BudgetSetAdapter, CapabilityRequirements,
    CapabilityRequirementsAdapter, ConstraintKind, ConversionReport, ResourceConstraint,
    ResourceDimension, ResourceError, ResourceReservation, ResourceReservationAdapter,
    ResourceUnit, ResourceValue, ResourceVector, ResourceVectorAdapter, ResourceViolation,
};
pub use status::ResultStatus;
pub use ty::{Quantity, Type, TypeError};
pub use unit::Unit;

// Re-used from the math subsystem so there is a *single* dimensional/index
// algebra and a *single* capability taxonomy (`full_system_design.md` §12.2,
// §5.3) — no parallel vocabulary can drift out of sync.
pub use math_types::{
    BaseDimension, CapabilityPath, Dimension, IndexColumn, IndexDim, IndexDimId, IndexFrame,
    IndexSignature, JoinPolicy, MissingPolicy,
};
