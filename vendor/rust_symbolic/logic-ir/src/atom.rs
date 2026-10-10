//! Atomic formulas (`full_system_design.md` §7.3).

use crate::term::Term;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

/// An atomic formula — the smallest unit that can be true or false.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Atom {
    Predicate {
        predicate: String,
        arguments: Vec<Term>,
    },
    Equality(Term, Term),
    Comparison {
        op: CompareOp,
        left: Term,
        right: Term,
    },
    Membership {
        element: Term,
        set: Term,
    },
}

impl Atom {
    pub fn predicate(name: impl Into<String>, arguments: Vec<Term>) -> Self {
        Atom::Predicate {
            predicate: name.into(),
            arguments,
        }
    }
}
