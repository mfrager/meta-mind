//! `mm-metaanalysis` — the system observing itself.
//!
//! Design §60 and Phase 11 put four organs between "something went wrong" and "here
//! is the change that fixes it". This crate is the first two halves: the **prediction
//! ledger with calibration** (did the confidence mean anything?) and the
//! **event-triggered meta-analysis** (what kind of mistake was it, and what is the
//! lesson?). `mm-mistakes` turns the lesson into a test and `mm-selfeng` turns the gap
//! into a promoted change; neither can start without an analysis that names a class.
//!
//! The modules, in the order data flows through them:
//!
//! * [`ledger`] — the append-only prediction ledger and its one resolution per
//!   prediction. The database refuses UPDATE and DELETE, so this is where "the claim
//!   was staked before the outcome" stops being a convention.
//! * [`calibration`] — Brier, log loss, ECE, reliability bins, coverage and selective
//!   risk, and `threshold_for`. The arithmetic is Phase 9's; only the presentation and
//!   the threshold rule are new.
//! * [`taxonomy`] — the sixteen error classes, the eight triggers, and the
//!   five-component priority an analysis is ranked by.
//! * [`triggers`] — which triggers actually fired, read deterministically off the
//!   event log and the stores. Event-triggered, never scheduled: a trigger that has
//!   not happened is not a reason to analyse.
//! * [`diagnosis`] — one bounded pass (an LLM call when a provider is configured, a
//!   documented cue table when one is not) that writes one `meta_analyses` row.
//! * [`lessons`] — the extracted lesson, written into the cognitive library through
//!   that library's own gate rather than into a table of its own.
//! * [`rdf`] — the `/selfeng` mirror. Predictions, runs, analyses and lessons reach RDF
//!   through one named graph, asserted by the shapes in `ontology/shapes/selfeng.ttl`.
//!
//! Four invariants hold across the crate, and each is a test somewhere:
//!
//! 1. **A prediction is immutable once staked, and resolved once.** Enforced in SQL
//!    (triggers, primary key) *and* in the API (no update path, `AlreadyResolved`).
//! 2. **Nothing is scored that was not labeled.** `labeled` inner-joins the outcomes.
//! 3. **The metrics are Phase 9's.** `brier_score`, `log_loss`,
//!    `expected_calibration_error` and the temperature fit come from `mm-decision`, so
//!    two gates cannot disagree about what a Brier score is.
//! 4. **A diagnosis classifies at least one class.** An analysis that classified
//!    nothing cannot be acted on, so both diagnosers refuse to produce one.
#![forbid(unsafe_code)]

pub mod calibration;
pub mod diagnosis;
pub mod error;
pub mod ledger;
pub mod lessons;
pub mod rdf;
pub mod taxonomy;
pub mod triggers;

// Spelled `crate::calibration`, not the bare path: the short form reads to the
// code-metadata scanner exactly like a dependency on an external crate named
// `calibration`, and Phase 11's `modules/cognition/calibration` module carries that
// name — the scanner would then record a dependency this crate does not have.
pub use crate::calibration::{
    bins, classes_of, coverage_and_risk, fit_default, load_labeled, load_reference, record_run,
    CalibrationReference, CalibrationReferenceClass, CalibrationReport, Calibrator, Labeled,
    MetricsCalibrator, ReliabilityBin, SharedCalibrator, DEFAULT_BINS, DEFAULT_TARGET_PRECISION,
    MIN_COVERED,
};
pub use diagnosis::{
    analyze, analyze_fixture, Diagnoser, Diagnosis, HeuristicDiagnoser, LlmDiagnoser, MetaAnalysis,
    MetaContext, DIAGNOSIS_SCHEMA_ID,
};
pub use error::{MetaError, Result};
pub use ledger::{content_ulid, Prediction, PredictionLedger, PredictionOutcome};
pub use lessons::{extract, list, Lesson};
pub use taxonomy::{ErrorClass, MetaAnalysisPriority, Trigger, ERROR_CLASSES, TRIGGERS};
pub use triggers::{fire, load_episode, EpisodeContext, MetaTrigger};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.metaanalysis";

/// A logger that collects its records, for this crate's tests.
///
/// Built from the public constructor rather than a test-only hook in `mm-log`, so a
/// test uses the same code path the kernel does.
#[cfg(test)]
/// A logger over an in-memory sink, writing its audit records to `store`.
///
/// The audit writer is the point: `meta.diagnose` and `meta.lesson` are audited records,
/// and `Logger::emit` refuses an audited record when no writer is configured — which is
/// the behaviour that keeps "the row was written" and "the audit chain saw it" from
/// drifting apart. A test logger without one would fail for the wrong reason.
pub(crate) fn test_logger(store: &mm_store_sqlite::SqliteStore) -> std::sync::Arc<mm_log::Logger> {
    std::sync::Arc::new(mm_log::Logger::new(
        mm_log::Level::Trace,
        vec![Box::new(mm_log::CollectSink::new())],
        Some(std::sync::Arc::new(store.clone())),
        mm_log::RedactionPolicy::empty(),
    ))
}
