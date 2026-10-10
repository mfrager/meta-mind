//! Statements (`full_system_design.md` §7.4).

use crate::atom::Atom;
use crate::extension::CausalExpr;
use crate::formula::Formula;
use crate::probabilistic::{
    BernoulliDeclaration, BernoulliTrialsDeclaration, CategoricalDeclaration, OutcomeConstraint,
    OutcomeMeasure, OutcomeSpace, OutcomeStep,
};
use crate::term::Term;

/// A rule's semantics is part of its identity, never inferred from syntax.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RuleSemantics {
    ClassicalImplication,
    Horn,
    Datalog,
    Probabilistic,
    Default,
    Defeasible,
    Fuzzy,
    Causal,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rule {
    pub head: Atom,
    pub body: Vec<Formula>,
    pub semantics: RuleSemantics,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fact {
    pub atom: Atom,
    pub source: Option<String>,
    pub confidence: Option<String>,
}

/// The kind of an equation (`full_system_design.md` §15; inherited from the
/// math subsystem's equation taxonomy).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EquationKind {
    Identity,
    Definition,
    Behavioral,
    Constraint,
    Objective,
    AccountingIdentity,
    Differential,
    Inequality,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Equation {
    pub kind: EquationKind,
    pub lhs: Term,
    pub rhs: Term,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Action {
    pub name: String,
    pub parameters: Vec<String>,
    pub precondition: Option<Formula>,
    pub effect: Vec<Formula>,
    pub cost: Option<Term>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Objective {
    Minimize(Term),
    Maximize(Term),
}

/// A statement: a fact, rule, formula, equation, constraint, definition,
/// observation, assumption, evidence, action, causal statement, objective, or
/// distribution declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Statement {
    Fact(Fact),
    Rule(Rule),
    Formula(Formula),
    Equation(Equation),
    Constraint(Formula),
    Definition {
        name: String,
        term: Term,
    },
    /// An *observation* is not a fact (`full_system_design.md` §5.2).
    Observation {
        atom: Atom,
        source: Option<String>,
    },
    /// An *assumption* is not a fact.
    Assumption {
        formula: Formula,
        source: Option<String>,
        scope: Option<String>,
    },
    Evidence {
        atom: Atom,
        observed: bool,
        source: Option<String>,
    },
    Action(Action),
    CausalStatement(CausalExpr),
    Objective(Objective),
    DistributionDecl {
        name: String,
        distribution: String,
    },
    /// A categorical distribution declaration
    /// `categorical(name=..., states=[state(name=..., probability=...), ...])`
    /// (`categorical_sem_design.md` §3.2). A prior, not evidence.
    Categorical(CategoricalDeclaration),
    /// Bernoulli shorthand `bernoulli(name=..., success=..., failure=...,
    /// probability=...)` (§3.3). Expands to a two-state categorical.
    Bernoulli(BernoulliDeclaration),
    /// Repeated independent Bernoulli trials `bernoulli_trials(...)` (§3.3).
    BernoulliTrials(BernoulliTrialsDeclaration),
    OutcomeSpace(OutcomeSpace),
    OutcomeStep(OutcomeStep),
    OutcomeConstraint(OutcomeConstraint),
    OutcomeMeasure(OutcomeMeasure),
}
