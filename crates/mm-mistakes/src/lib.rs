//! `mm-mistakes` — the mistake→regression-test compiler.
//!
//! Design §60 / Phase 11 asks for one thing: a mistake the being made must become a test
//! that **fails before** the fix and **passes after** it, and it must stay in the suite.
//! Everything here serves that sentence.
//!
//! The two modules, in the order data flows through them:
//!
//! * [`reproduce`] — reduce a recorded mistake (a Phase 5 `memories` row of kind `mistake`
//!   plus its `mistakes` detail row) to a statement: what happened, what should have
//!   happened, and which signals were available and missed. Pure arithmetic over the
//!   record; nothing here reads a clock, a file, or a model.
//! * [`emit`] — write that statement as a *case directory* under `bench/regression/`, run
//!   it, and record the result only once the fail-before/pass-after pair has actually been
//!   observed.
//!
//! Three rules hold across the crate:
//!
//! 1. **A test that never failed is not evidence.** The compiler runs the generated case
//!    and refuses to record it unless `check.sh` exited non-zero on the buggy subject and
//!    zero after the fix. [`MistakeError::NotFailingBefore`] is that refusal, and it is
//!    named rather than reported as a boolean.
//! 2. **A case is a directory, not a snippet.** `bench/regression/README.md` documents the
//!    four files and why: a case has to be runnable by an operator with no toolchain,
//!    resettable to its buggy state, and reviewable as a diff. `mm-cli regression run`
//!    drives the same scripts the compiler wrote.
//! 3. **A generated subject says it is a re-enactment.** This phase cannot synthesize a
//!    patch for arbitrary code, so the generated `subject.sh` re-enacts the recorded
//!    incident. The contract — the expectation, the marker, the pair — is the durable half,
//!    and the scripts carry a comment saying so, so a later phase can replace the subject
//!    with the real code path and keep the case.
//!
//! What this crate deliberately does not do: it does not decide *which* mistakes deserve a
//! test. That is `mm-metaanalysis`'s trigger and priority logic; the compiler compiles what
//! it is handed, and reports when it cannot.
#![forbid(unsafe_code)]

pub mod emit;
pub mod error;
pub mod reproduce;

pub use emit::{
    case_path, cases_in_suite, compile_mistake, escape_for_double_quotes, read_case,
    render_case_json, render_check_script, render_fix_script, render_reset_script, run_case,
    run_suite, write_case, CaseExpectation, CaseFile, CaseResult, RegressionTest, SuiteReport,
    SCRIPT_TIMEOUT_SECS, SUITE_DIR,
};
pub use error::{MistakeError, Result};
pub use reproduce::{fetch_mistake, minimize, normalize, reproduce, slug_for, MinimizedCase};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.mistakes";
