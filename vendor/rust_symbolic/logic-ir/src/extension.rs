//! Typed semantic extensions (`full_system_design.md` §7.5).
//!
//! Keeping these as `Extension` variants (rather than `Formula` variants)
//! keeps the core formula AST small; the semantic checker verifies that the
//! active profile permits each extension.

use crate::arithmetic::ArithmeticExpr;
use crate::atom::CompareOp;
use crate::formula::{DeonticOperator, Formula};
use crate::term::Term;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CausalExpr {
    Cause {
        cause: Term,
        effect: Term,
    },
    Do {
        assignment: Term,
        consequence: Box<Formula>,
    },
    Counterfactual {
        intervention: Term,
        outcome: Box<Formula>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlanningExpr {
    Goal(Box<Formula>),
    Action {
        name: String,
        precondition: Box<Formula>,
        effect: Box<Formula>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OptimizationExpr {
    Minimize(Term),
    Maximize(Term),
    SubjectTo {
        objective: Term,
        constraints: Vec<Formula>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameExpr {
    pub players: Vec<String>,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SpatialExpr {
    Contains { container: Term, contained: Term },
    DistanceLt { a: Term, b: Term, distance: Term },
}

/// A metric bound `[lo, hi]` for the bounded temporal-logic operators
/// (`Eventually_[lo,hi]`, `Always_[lo,hi]`, temporal_uncertainty1.md §37).
/// `Eq`/`Hash` compare the IEEE bit patterns, so the bound can live inside the
/// `Eq`+`Hash` `TemporalExpr` (an `f64` is not `Eq`/`Hash`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricBound {
    pub lo: f64,
    pub hi: f64,
}

impl MetricBound {
    pub fn new(lo: f64, hi: f64) -> Self {
        Self { lo, hi }
    }
}

impl Eq for MetricBound {}

impl std::hash::Hash for MetricBound {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.lo.to_bits().hash(state);
        self.hi.to_bits().hash(state);
    }
}

/// Modal strength of a temporal statement (temporal_uncertainty1.md §13):
/// `Possibly` ("may have arrived before noon") vs. `Necessarily` ("definitely
/// arrived before noon").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TemporalModality {
    Possibly,
    Necessarily,
}

/// Temporal statements (SUBSYSTEM_INTEGRATION_DESIGN.md §1.1): Allen interval
/// relations, occurrence-within-window constraints, and the bounded
/// temporal-logic operators (`next/previous/eventually/always/until/since`,
/// temporal_uncertainty1.md §36–38).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TemporalExpr {
    /// An Allen relation between two labelled intervals (the relation name is
    /// one of the 13 Allen relations, e.g. "Before", "During").
    Allen { a: Term, relation: String, b: Term },
    /// An event/interval that must fall within a window.
    Within { event: Term, window: Term },
    /// `Next(from, to)`: `from` is immediately followed by `to` (§36).
    Next { from: Term, to: Term },
    /// `Previous(from, to)`: `to` is immediately preceded by `from` (§36).
    Previous { from: Term, to: Term },
    /// `Eventually(event)` or metric `Eventually_[lo,hi](event)` (§37).
    Eventually {
        event: Term,
        bound: Option<MetricBound>,
    },
    /// `Always(event)` or metric `Always_[lo,hi](event)` (§37).
    Always {
        event: Term,
        bound: Option<MetricBound>,
    },
    /// `Until(a, b)`: `a` holds until `b` (§36).
    Until {
        a: Term,
        b: Term,
        bound: Option<MetricBound>,
    },
    /// `Since(a, b)`: `a` has held since `b` (§36).
    Since {
        a: Term,
        b: Term,
        bound: Option<MetricBound>,
    },
    /// A modal wrapper `Possibly`/`Necessarily` over another temporal
    /// statement (§13).
    Modality {
        mode: TemporalModality,
        inner: Box<TemporalExpr>,
    },
}

/// Event-calculus statements (SUBSYSTEM_INTEGRATION_DESIGN.md §1.2):
/// initiates/terminates rules and occurrence constraints.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventExpr {
    /// `Happens(event, time)` — an event occurrence at a time point.
    Happens {
        event: Term,
        time: Term,
    },
    /// `HoldsAt(fluent, time)` — a boolean fluent query.
    HoldsAt {
        fluent: Term,
        time: Term,
    },
    /// `HoldsDuring(fluent, start, end)` — a frame-preserving interval query.
    HoldsDuring {
        fluent: Term,
        start: Term,
        end: Term,
    },
    /// `ValueAt(fluent, time, value)` — a typed/numeric state query.
    ValueAt {
        fluent: Term,
        time: Term,
        value: Term,
    },
    /// `StateAt(time, fluent)` — request the complete value of a fluent at a time.
    StateAt {
        time: Term,
        fluent: Term,
    },
    Initiates {
        event: Term,
        fluent: Term,
    },
    Terminates {
        event: Term,
        fluent: Term,
    },
    Releases {
        event: Term,
        fluent: Term,
    },
    /// A dynamic event guard evaluated immediately before occurrence.
    Precondition {
        event: Term,
        fluent: Term,
    },
    /// The dimensions explicitly modified by an event; used to check frame preservation.
    ModifiedDimensions {
        event: Term,
        fluents: Vec<Term>,
    },
    /// Compatibility spelling retained for existing lowering clients.
    OccursAt {
        event: Term,
        time: Term,
    },
}

/// Abstraction statements (SUBSYSTEM_INTEGRATION_DESIGN.md §1.3): symbol
/// mappings, invariant-preservation requirements, space membership, and cost
/// facets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AbstractionExpr {
    /// `source` maps onto `target` through the listed symbols.
    Maps {
        source: Term,
        target: Term,
        via: Vec<Term>,
    },
    /// The named invariant must be preserved by the representation.
    Preserves {
        representation: Term,
        invariant: Term,
    },
    /// The representation is a point of the named representation space.
    InSpace { representation: Term, space: Term },
    /// The representation measures the named §9 cost facet at the given value.
    CostFacet {
        representation: Term,
        facet: Term,
        value: Term,
    },
    /// The representation loses the named property under the abstraction.
    Loses {
        representation: Term,
        property: Term,
    },
    /// The representation approximately preserves the property within a bound.
    Approximates {
        representation: Term,
        property: Term,
        bound: Term,
    },
    /// A concept with its intension and representation views.
    Concept {
        name: Term,
        intension: Term,
        views: Vec<Term>,
    },
    /// A latent variable with its role and evidence.
    LatentVariable {
        name: Term,
        role: Term,
        evidence: Vec<Term>,
    },
    /// The representation is suitable for the named task (§8).
    Suitable { representation: Term, task: Term },
    /// The representation has the named cost facet at the given value (§9).
    RepresentationCost {
        representation: Term,
        cost_type: Term,
        value: Term,
    },
}

/// Information-theoretic statements (SUBSYSTEM_INTEGRATION_DESIGN.md §1.4).
///
/// The structured variants preserve operands in Semantic IR. `Measure` is
/// retained as a compatibility form for legacy RDF/query lowerings; new
/// lowerings should prefer the typed variants.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InformationExpr {
    EntropyGt {
        distribution: Term,
        bound: Term,
    },
    MutualInformationGt {
        x: Term,
        y: Term,
        bound: Term,
    },
    Entropy {
        variable: Term,
        distribution: Term,
    },
    JointEntropy {
        variables: Vec<Term>,
        joint: Term,
    },
    ConditionalEntropy {
        target: Term,
        condition: Term,
        joint: Term,
    },
    MutualInformation {
        x: Term,
        y: Term,
        joint: Term,
    },
    ConditionalMutualInformation {
        x: Term,
        y: Term,
        given: Term,
        joint: Term,
    },
    Divergence {
        kind: Term,
        p: Term,
        q: Term,
    },
    InformationGain {
        target: Term,
        observation: Term,
        prior: Term,
        posterior: Term,
    },
    ExpectedInformationGain {
        target: Term,
        action: Term,
        outcomes: Term,
    },
    ValueOfInformation {
        decision_problem: Term,
        action: Term,
        cost: Term,
    },
    Channel {
        input: Term,
        output: Term,
        transition: Term,
    },
    ChannelCapacity {
        channel: Term,
    },
    RateDistortion {
        source: Term,
        distortion: Term,
        rate: Term,
    },
    DescriptionLength {
        object: Term,
    },
    InformationConstraint {
        quantity: Term,
        relation: CompareOp,
        bound: Term,
    },
    /// A requested information measure (`H(rain)`, `I(X;Y)`, …), carried by
    /// its canonical name plus its measure kind. Legacy compatibility form.
    Measure {
        name: Term,
        kind: Term,
    },
}

/// The resource dimension of a declared complexity model. These dimensions
/// are deliberately distinct: communication, samples, and queries are not
/// interchangeable with runtime or memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComplexityKind {
    Time,
    Space,
    Communication,
    Energy,
    Io,
    Sample,
    Query,
    Parallel,
}

impl ComplexityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Space => "space",
            Self::Communication => "communication",
            Self::Energy => "energy",
            Self::Io => "io",
            Self::Sample => "sample",
            Self::Query => "query",
            Self::Parallel => "parallel",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "time" => Some(Self::Time),
            "space" => Some(Self::Space),
            "communication" => Some(Self::Communication),
            "energy" => Some(Self::Energy),
            "io" => Some(Self::Io),
            "sample" => Some(Self::Sample),
            "query" => Some(Self::Query),
            "parallel" => Some(Self::Parallel),
            _ => None,
        }
    }
}

/// A structured, parameterized complexity expression. `variables` declares
/// the names the expression may use; the expression itself remains a typed
/// arithmetic AST rather than a formatted formula string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComplexityModel {
    /// The declared algorithm/problem subject of this model.
    pub subject: String,
    pub kind: ComplexityKind,
    pub variables: Vec<String>,
    pub expression: ArithmeticExpr,
    pub asymptotic: bool,
}

impl ComplexityModel {
    pub fn new(
        kind: ComplexityKind,
        variables: Vec<String>,
        expression: ArithmeticExpr,
        asymptotic: bool,
    ) -> Self {
        Self {
            subject: String::new(),
            kind,
            variables,
            expression,
            asymptotic,
        }
    }

    /// Set the algorithm/problem subject while retaining the compact constructor.
    pub fn for_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = subject.into();
        self
    }
}

/// Complexity statements (SUBSYSTEM_INTEGRATION_DESIGN.md §1.5): legacy
/// membership/feasibility plus dimension-specific parameterized models.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ComplexityExpr {
    InClass {
        algorithm: Term,
        class: Term,
    },
    Feasible {
        algorithm: Term,
        size: Term,
        budget: Term,
    },
    TimeComplexity {
        subject: Term,
        model: ComplexityModel,
    },
    SpaceComplexity {
        subject: Term,
        model: ComplexityModel,
    },
    CommunicationComplexity {
        subject: Term,
        model: ComplexityModel,
    },
    EnergyComplexity {
        subject: Term,
        model: ComplexityModel,
    },
}

/// Decision-theoretic statements (SUBSYSTEM_INTEGRATION_DESIGN.md §2.2):
/// pairwise preferences and utility-maximization requirements.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DecisionExpr {
    /// `preferred` is (weakly) preferred over `over`.
    Prefers { preferred: Term, over: Term },
    /// The named action must maximize expected utility.
    MaximizeUtility { action: Term },
}

/// Epistemic statements (SUBSYSTEM_INTEGRATION_DESIGN.md §2.4): knowledge and
/// belief about propositions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EpistemicExpr {
    Knows { agent: Term, proposition: Term },
    Believes { agent: Term, proposition: Term },
}

/// Theory-of-mind statements (SUBSYSTEM_INTEGRATION_DESIGN.md §2.5): nested
/// beliefs over a chain of agents terminating in a proposition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TomExpr {
    /// `believer` believes `chain[0]` believes … believes `proposition`.
    NestedBelief {
        believer: Term,
        chain: Vec<Term>,
        proposition: Term,
    },
}

/// Mechanism-design statements (SUBSYSTEM_INTEGRATION_DESIGN.md §2.6):
/// equilibrium and incentive-compatibility requirements.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MechanismExpr {
    /// The named strategy profile is a Nash equilibrium.
    Nash { profile: Vec<Term> },
    /// The named truthful profile is incentive compatible.
    IncentiveCompatible { truthful: Vec<Term> },
}

/// Epistemic-market statements (SUBSYSTEM_INTEGRATION_DESIGN.md §2.7):
/// market-implied prices and forecaster reputation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EpistemicMarketExpr {
    /// The market-implied probability of the proposition is `value`.
    Price { proposition: Term, value: Term },
    /// The forecaster's reputation is at least `value`.
    ReputationAtLeast { forecaster: Term, value: Term },
    /// A forecaster's stated probability for a proposition.
    Prediction {
        forecaster: Term,
        proposition: Term,
        probability: Term,
    },
    /// The resolved outcome of a proposition.
    Resolution { proposition: Term, outcome: Term },
}

/// Analogical & case-based statements (SUBSYSTEM_INTEGRATION_DESIGN.md §3.3):
/// cases as typed element sets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AnalogyExpr {
    /// `case` contains the named element of the given kind.
    CaseElement {
        case: Term,
        element: Term,
        kind: Term,
    },
}

/// Evolutionary & population-dynamics statements (§3.2): trait frequencies and
/// the pairwise payoff structure.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EvolutionExpr {
    Trait {
        name: Term,
        frequency: Term,
    },
    Payoff {
        row: Term,
        column: Term,
        value: Term,
    },
}

/// Learning & adaptive-agent statements (§3.1): experiences and the learner
/// configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LearningExpr {
    Experience {
        state: Term,
        action: Term,
        reward: Term,
    },
    Learner {
        algorithm: Term,
        discount: Term,
    },
}

/// Memory statements (§4.1): typed memory traces and reasoning trajectories.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MemoryExpr {
    Trace { id: Term, kind: Term },
    Trajectory { id: Term },
}

/// Symbolic-regression statements (§3.4): the regression variables and the
/// scoring objectives.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SymRegExpr {
    Regression { variable: Term },
    Objective { name: Term },
}

/// Program-synthesis statements (§3.5): pre/postconditions and the optional
/// formal target.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SynthesisExpr {
    Precondition { condition: Term },
    Postcondition { condition: Term },
    Target { program: Term },
}

/// Argumentation & debate statements (§20): claims, arguments, and attack
/// edges as first-class objects.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArgumentationExpr {
    Claim { id: Term },
    Argument { id: Term },
    Attack { from: Term, to: Term },
}

/// Knowledge provenance & trust statements (§21): sources and knowledge
/// claims as first-class provenance-bearing objects.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProvenanceExpr {
    Source { id: Term },
    Claim { id: Term },
}

/// A non-classical semantic extension.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Extension {
    Probabilistic {
        probability: Term,
        proposition: Box<Formula>,
    },
    Fuzzy {
        degree: Term,
        proposition: Box<Formula>,
    },
    Causal(CausalExpr),
    Deontic {
        operator: DeonticOperator,
        body: Box<Formula>,
    },
    Planning(PlanningExpr),
    Optimization(OptimizationExpr),
    Game(GameExpr),
    Spatial(SpatialExpr),
    Temporal(TemporalExpr),
    Event(EventExpr),
    Abstraction(AbstractionExpr),
    Information(InformationExpr),
    Complexity(ComplexityExpr),
    Decision(DecisionExpr),
    Epistemic(EpistemicExpr),
    Tom(TomExpr),
    Mechanism(MechanismExpr),
    EpistemicMarket(EpistemicMarketExpr),
    Analogy(AnalogyExpr),
    Evolution(EvolutionExpr),
    Learning(LearningExpr),
    Memory(MemoryExpr),
    SymReg(SymRegExpr),
    Synthesis(SynthesisExpr),
    Argumentation(ArgumentationExpr),
    Provenance(ProvenanceExpr),
}
