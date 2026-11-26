//! Formulas (`full_system_design.md` §7.3).
//!
//! `Modal` and `Temporal` are core `Formula` variants; the remaining
//! non-classical operators (probabilistic, fuzzy, causal, deontic, planning,
//! optimization, game, spatial) live in `Extension` to keep `Formula` small
//! (`full_system_design.md` §7.5).

use logic_types::Type;

use crate::atom::Atom;
use crate::extension::Extension;
use crate::probabilistic::{StateSelector, TrialSelector};
use crate::term::{Binding, Term};

/// A bound variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variable(pub String);

impl Variable {
    pub fn new(name: impl Into<String>) -> Self {
        Variable(name.into())
    }
}

/// Generalized quantifiers — extensible beyond `forall`/`exists`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GenQuant {
    Count,
    AtLeast(usize),
    AtMost(usize),
    Exactly(usize),
    Sum,
    Min,
    Max,
    ArgMax,
    ArgMin,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModalOperator {
    Necessary,
    Possible,
    /// Agent-indexed epistemic operator.
    Knows(String),
    Believes(String),
    /// Accessibility-relation-indexed box/diamond.
    Box(String),
    Diamond(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TemporalOperator {
    Next,
    Previous,
    Eventually,
    Always,
    Until,
    Since,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DeonticOperator {
    Obligatory,
    Permitted,
    Forbidden,
}

/// A logical formula.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Formula {
    True,
    False,
    Atom(Atom),
    Not(Box<Formula>),
    And(Vec<Formula>),
    Or(Vec<Formula>),
    Xor(Box<Formula>, Box<Formula>),
    Implies(Box<Formula>, Box<Formula>),
    Iff(Box<Formula>, Box<Formula>),
    ForAll {
        variable: Variable,
        domain: Option<Type>,
        body: Box<Formula>,
    },
    Exists {
        variable: Variable,
        domain: Option<Type>,
        body: Box<Formula>,
    },
    Generalized {
        quantifier: GenQuant,
        variables: Vec<Variable>,
        body: Box<Formula>,
    },
    Modal {
        operator: ModalOperator,
        body: Box<Formula>,
    },
    Temporal {
        operator: TemporalOperator,
        body: Box<Formula>,
    },
    /// Any non-classical extension (probabilistic, fuzzy, causal, deontic,
    /// planning, optimization, game, spatial).
    Extension(Extension),
    Let {
        bindings: Vec<Binding>,
        body: Box<Formula>,
    },
    /// A typed categorical-state selector `state(variable=..., value=...)` in
    /// formula position (`categorical_sem_design.md` §5). Never a dynamic
    /// predicate.
    State(StateSelector),
    /// A typed trial selector `trial(family=..., index=..., value=...)` in
    /// formula position (`categorical_sem_design.md` §5). Never a dynamic
    /// predicate.
    Trial(TrialSelector),
}

impl Formula {
    pub fn predicate(name: impl Into<String>, arguments: Vec<Term>) -> Self {
        Formula::Atom(Atom::predicate(name, arguments))
    }
}
