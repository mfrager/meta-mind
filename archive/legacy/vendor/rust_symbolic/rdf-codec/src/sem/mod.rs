//! The SEM lane (Phase 0 of `planning/s_expr/s_expr_design2.md`): an
//! independent processing lane that compiles S-expression documents straight
//! to final RDF graphs and renders graphs back to canonical SEM text — no
//! cRDF is produced or consumed anywhere in this lane.
//!
//! Modules:
//! - [`errors`] — the stable `sem:e1`…`sem:e10` error set
//! - [`lex`] — tokens, comments, trailing commas
//! - [`ast`] — the typed AST
//! - [`parse`] — EBNF §3.2 → typed AST
//! - [`grammar`] — module grammar tables + grammar card renderer
//! - [`compile`] — document → final RDF Graph (R1–R11)
//! - [`render`] — Graph → canonical SEM text
//! - [`indent`] — structural indenting of SEM text (LLM output + Web UI)
//! - [`detect`] — edge dispatch (first-char format selection)

pub mod ast;
pub mod compile;
pub mod detect;
pub mod errors;
pub mod grammar;
pub mod indent;
pub mod lex;
pub mod parse;
pub mod render;

// NOTE (vendored copy): the origin project's in-src test modules
// (`annotation_tests`, `report_tests`, `roundtrip_tests`, `subsystem_tests`,
// `tests`) depend on fixtures from the wider `rust_symbolic` workspace and were
// omitted when this crate was copied in. `mm-store-graph`'s conformance tests
// exercise the SEM lane's public surface that Metamind depends on.

pub use compile::{compile, compile_to_turtle, compile_to_turtle_seeded, compile_with, Compiled};
pub use detect::{detect, is_sem, PayloadFormat};
pub use errors::SemError;
pub use grammar::{
    card_budget, class_form, grammar, grammar_example, module_status, non_form_classes,
    render_grammar_card, ArgType, FormSpec, Grammar, Prop, SemPrefixes, SubjKind, ALL_MODULES,
};
pub use indent::indent;
pub use lex::{lex, Token};
pub use parse::parse;
pub use render::{
    find_report_root, find_root, render, render_logic_report, render_report, render_with_mode,
    AnnotationMode, REPORT_ROOTS,
};
