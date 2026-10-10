//! `logic-ir` — the Semantic IR (LogicIR): terms, formulas, statements, and
//! typed extensions (`full_system_design.md` §7).

pub mod arithmetic;
pub mod atom;
pub mod extension;
pub mod formula;
pub mod model;
pub mod probabilistic;
pub mod rdf;
pub mod statement;
pub mod term;

pub use atom::{Atom, CompareOp};
pub use extension::{
    AbstractionExpr, AnalogyExpr, ArgumentationExpr, CausalExpr, ComplexityExpr, ComplexityKind,
    ComplexityModel, DecisionExpr, EpistemicExpr, EpistemicMarketExpr, EventExpr, EvolutionExpr,
    Extension, GameExpr, InformationExpr, LearningExpr, MechanismExpr, MemoryExpr, MetricBound,
    OptimizationExpr, PlanningExpr, ProvenanceExpr, SpatialExpr, SymRegExpr, SynthesisExpr,
    TemporalExpr, TemporalModality, TomExpr,
};
pub use formula::{DeonticOperator, Formula, GenQuant, ModalOperator, TemporalOperator, Variable};
pub use model::{
    BaseLogic, ExtensionKind, LogicModel, LogicProgram, NegationSemantics, Query, SemanticProfile,
    Signature, Theory, VarDecl,
};
pub use probabilistic::{
    BernoulliDeclaration, BernoulliTrialsDeclaration, CategoricalDeclaration, EventExpression,
    OutcomeConstraint, OutcomeMeasure, OutcomeQuery, OutcomeSpace, OutcomeStep, OutcomeUpdate,
    OutcomeUpdateOperation, OutcomeValue, StateSelector, TrialSelector,
};
pub use statement::{
    Action, Equation, EquationKind, Fact, Objective, Rule, RuleSemantics, Statement,
};
pub use term::{ArithmeticExpr, Binding, Term};
