//! SEM typed AST (§3.2–3.3 of `planning/s_expr/s_expr_design2.md`): the
//! resolved, typed tree the compiler lowers to RDF. The AST is module-aware
//! and position-typed: a bare symbol in an *expression* position is a
//! variable/parameter reference (via [`Atom::Sym`]), while kinds are enum
//! values carried as [`Atom::Kind`].

use std::collections::BTreeMap;

/// A typed atomic value.
#[derive(Debug, Clone, PartialEq)]
pub enum Atom {
    /// A number literal (implicit constant, R5).
    Num(f64),
    /// An exact rational numeric literal written as `fraction(n, d)`.
    Fraction(i128, i128),
    /// A string literal (only where the module grammar expects `str`).
    Str(String),
    /// A boolean literal.
    Bool(bool),
    /// A bare identifier in *symbol* position — its meaning is decided by the
    /// module table at compile time (variable/param name, kind, or plain label).
    Sym(String),
    /// An explicit variable reference `?name` (`logic_sem_design.md` §4.1).
    /// In logic modules it lowers to a `logic:Variable` node so canonical
    /// SEM text round-trips the `?`; in other modules it behaves like `Sym`.
    Var(String),
    /// An enum kind value, e.g. `instant`, `maximize`, `le`.
    Kind(String),
}

/// A nested (unqualified) form inside a document, e.g. `sum(x, 2)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    /// The form name, validated against the module table.
    pub name: String,
    /// Positional arguments in canonical order.
    pub args: Vec<Value>,
    /// Named (keyword) arguments: `name=value`. Only the first `=` per name is
    /// preserved (the last wins).
    pub kwargs: BTreeMap<String, Value>,
}

/// One value in argument position.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Atom(Atom),
    Form(Form),
    /// A homogeneous list of values: `[ v1, v2, … ]`.
    List(Vec<Value>),
}

/// A fully typed, resolved SEM document with exactly one top form (R1).
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The module, taken from the top-level qname (R2).
    pub module: String,
    /// The entry form once resolved (may be synthesized when auto-rooted, R9).
    pub top: Form,
}
