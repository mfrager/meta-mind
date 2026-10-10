//! Terms (`full_system_design.md` §7.2).

use crate::formula::Formula;

/// A variable-binding name/value pair (used by `Let` in terms and formulas).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binding {
    pub name: String,
    pub value: Term,
}

impl Binding {
    pub fn new(name: impl Into<String>, value: Term) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

/// A term: a value-denoting expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Term {
    Variable(String),
    /// A numeric literal, kept symbolic until the Numerical IR interprets it
    /// (this preserves exactness and keeps the IR `Eq`/`Hash`-friendly).
    Constant(String),
    Application {
        function: String,
        arguments: Vec<Term>,
    },
    /// Higher-order terms are optional (they change solver requirements).
    Lambda {
        parameters: Vec<String>,
        body: Box<Term>,
    },
    If {
        condition: Box<Formula>,
        then_branch: Box<Term>,
        else_branch: Box<Term>,
    },
    Let {
        bindings: Vec<Binding>,
        body: Box<Term>,
    },
    /// Arithmetic expressions delegate to the math subsystem's full `Expr`
    /// via this leaf (`full_system_design.md` §7.2, §15).
    Arithmetic(ArithmeticExpr),
}

/// The arithmetic leaf of `Term` (defined in `arithmetic.rs`).
pub use crate::arithmetic::ArithmeticExpr;

impl Term {
    pub fn variable(name: impl Into<String>) -> Self {
        Term::Variable(name.into())
    }

    pub fn arithmetic(e: ArithmeticExpr) -> Self {
        Term::Arithmetic(e)
    }
}
