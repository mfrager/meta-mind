//! Typed probabilistic SEM forms (`planning/s_expr/categorical_sem_design.md`
//! §3–§4): categorical declarations, Bernoulli shorthand, repeated-trial
//! families, typed query targets, and typed `state`/`trial` selectors.
//!
//! These are the *typed* forms the structured SEM surface lowers into — never
//! synthetic predicates or arbitrary strings. Selectors keep their
//! `(variable, state)` / `(family, index, value)` identity all the way to the
//! ProbLog adapter.
//!
//! Probabilities are carried as literal text (like `Term::Constant`), so the
//! declarations stay `Eq + Hash` and the exact surface number survives until
//! IR lowering (which may keep it a literal or resolve it as a parameter).

use crate::{CompareOp, Formula, Term};

/// A generic mutable outcome value declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutcomeValue {
    pub name: String,
    pub initial: Term,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutcomeSpace {
    pub values: Vec<OutcomeValue>,
    pub constraints: Vec<Formula>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutcomeStep {
    pub index: u64,
    pub guard: Option<Formula>,
    pub updates: Vec<OutcomeUpdate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutcomeUpdateOperation {
    Set,
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutcomeUpdate {
    pub variable: String,
    pub operation: OutcomeUpdateOperation,
    pub value: Term,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutcomeConstraint {
    pub expression: Formula,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutcomeMeasure {
    pub name: String,
    pub expression: Term,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutcomeQuery {
    Probability {
        measure: String,
        relation: CompareOp,
        threshold: Term,
    },
    Distribution {
        measure: String,
    },
    Expected {
        measure: String,
    },
    Validity,
}

/// A categorical-state selector: `state(variable=..., value=...)`. Identifies
/// one state of a declared categorical distribution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StateSelector {
    pub variable: String,
    pub state: String,
}

/// An indexed Bernoulli-family outcome selector: `trial(family=..., index=...,
/// value=...)`. Identifies one outcome of one trial of a declared
/// `bernoulli_trials` family.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrialSelector {
    pub family: String,
    pub index: u64,
    pub value: String,
}

/// A compound event expression over typed selectors (design §4.4): the
/// supported event vocabulary is `and`, `or`, `not`, `trial`, and `state`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventExpression {
    And(Vec<EventExpression>),
    Or(Vec<EventExpression>),
    Not(Box<EventExpression>),
    Trial(TrialSelector),
    State(StateSelector),
}

/// A categorical distribution declaration: `categorical(name=..., states=[...])`.
/// Exactly one state is selected; state probabilities must sum to one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CategoricalDeclaration {
    pub name: String,
    pub states: Vec<(String, String)>,
}

/// Bernoulli shorthand: `bernoulli(name=..., success=..., failure=...,
/// probability=...)`. Expands to a two-state categorical variable; the
/// failure probability is the complement, never a supplied answer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BernoulliDeclaration {
    pub name: String,
    pub success: String,
    pub failure: String,
    pub probability: String,
}

/// Repeated independent Bernoulli trials: `bernoulli_trials(name=..., count=...,
/// success=..., failure=..., probability=..., independent=...)`. A named
/// indexed family — not one categorical variable with repeated states.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BernoulliTrialsDeclaration {
    pub name: String,
    pub count: u64,
    pub success: String,
    pub failure: String,
    pub probability: String,
    pub independent: bool,
}
