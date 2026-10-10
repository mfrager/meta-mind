//! `mm-decision` — bounded decisions, risk, comparison integrity, and calibration.
//!
//! Design §47–49 and Phase 9 put the *bounded-decision primitive* between
//! reasoning and action: not "what should we do", but a typed question with a
//! closed answer set, asked against a frozen state, answered by one of three
//! interchangeable cores. This crate is that primitive, plus the deterministic
//! arithmetic a decision is graded with — risk measures, comparison validity,
//! uncertainty, calibration and conformal abstention.
//!
//! The modules, in the order a decision flows through them:
//!
//! * [`question`] — the bounded question, its answer, the state, and the feature
//!   vector. Confidence is bounded and the answer is typed **by construction**.
//! * [`core`] — the [`core::DecisionCore`] trait and the three cores'
//!   shared contract.
//! * [`rules`], [`local`], [`hosted`] — the three implementations.
//! * [`risk`] — eight deterministic measures over an outcome distribution, built
//!   on the copied-in `decision-ir` engine.
//! * [`compare`] — the comparison contract and its validity check. Most "wrong"
//!   decisions are really invalid comparisons, so this is a first-class refusal.
//! * [`uncertainty`], [`factuality`] — the signals a firewall escalates on.
//! * [`calibration`], [`conformal`] — turning a core's pre-calibration confidence
//!   into a calibrated one, and a calibrated one into an abstention threshold.
//! * [`rdf`] — the T-Box mirror every record carries.
//!
//! Four invariants hold across the crate, and each is a test somewhere:
//!
//! 1. **An answer is bounded by its question.** A core cannot return an option that
//!    was not offered, or a score outside the scale.
//! 2. **Confidence is in `[0,1]` by construction.** Nothing downstream clamps it.
//! 3. **Every core is deterministic.** Identical inputs produce identical output,
//!    including the tie-break.
//! 4. **No `decision-ir` type is public.** The engine is an implementation detail;
//!    callers see [`risk::RiskProfile`] and [`risk::RiskMeasure`].
#![forbid(unsafe_code)]

pub mod calibration;
pub mod compare;
pub mod conformal;
pub mod conformance;
pub mod core;
pub mod error;
pub mod factuality;
pub mod hosted;
pub mod local;
pub mod question;
pub mod rdf;
pub mod risk;
pub mod rules;
pub mod uncertainty;

// `crate::calibration` rather than the bare `calibration`: a bare first segment is
// also how a dependency on an external crate named `calibration` would be written,
// and the code-metadata scanner reads use-roots literally — with Phase 11's
// `modules/cognition/calibration` module in the tree, the short form would make the
// scanner record a self-invented dependency edge here.
pub use crate::calibration::{
    brier_score, expected_calibration_error, log_loss, logit, reliability_curve, sigmoid,
    Calibrator, CalibratorSet,
};
pub use compare::{
    check_contract, normalize, ComparisonCheck, ComparisonContract, ComparisonVerdict,
    ComparisonViolation, Condition, Dimension, NormalizedComparison, ObjRef, Timeframe, Unit,
    COMPARISON_VERDICTS, VIOLATION_CODES,
};
pub use conformal::{empirical_coverage, fit_held_out, split_conformal_quantile, ConformalSet};
pub use conformance::{
    findings_passed, run_conformance, summary as conformance_summary, Finding, CHECKS,
};
pub use core::{answer_or_none, is_legal_refusal, CoreId, DecisionCore, SharedCore, CORE_IDS};
pub use error::{DecisionError, Result, UnavailableCore};
pub use factuality::{
    atomic_precision, check_factuality, self_consistency_support, AtomExtractor, AtomicFact,
    FactualityCheck, FactualityReport, FactualityTool, LexicalAtomExtractor, LexicalSupportJudger,
    SearchAugmentedChecker, SupportJudger, FACTUALITY_CHECKS,
};
pub use hosted::{
    register as register_hosted_decision, HostedCore, HostedDecisionReply,
    HOSTED_DECISION_SCHEMA_ID,
};
pub use local::{DecisionHead, LocalCore};
pub use question::{
    AnswerValue, DecisionAnswer, DecisionQuestion, DecisionState, FeatureVector, OptionId,
    QuestionKind, ScoreAnchor, ScoreScale, QUESTION_KINDS,
};
pub use risk::{
    analyze_risk, analyze_risk_with, validate_outcomes, LossOutcome, RiskMeasure, RiskOptions,
    RiskProfile, RISK_MEASURE_NAMES,
};
pub use rules::{Rule, RuleAnswer, RuleTable, RulesCore, DEFAULT_RULES_TOML};
pub use uncertainty::{
    cluster_entropy, normalize_text, EquivalenceJudge, KernelEntropyScorer, LexicalJudge,
    SelfConsistencyScorer, SemanticEntropyScorer, UncertaintyKind, UncertaintyScorer,
    UNCERTAINTY_KINDS,
};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.decision";

/// Render JSON with object keys sorted, so two equal values render identically.
///
/// Every digest in this crate — the state digest a decision cache keys on, the
/// answer's content hash — is taken over this rendering, because "the same
/// features" has to mean "the same bytes" for a cache or a replay to be sound.
/// The implementation is the substrate's, so `mm-decision` and `mm-llm` cannot
/// disagree about what "the same JSON" means.
pub fn canonical_json(value: &serde_json::Value) -> String {
    mm_llm::schema::canonical_json(value)
}

/// A digest of an arbitrary JSON value under the crate's canonical rendering.
pub fn json_digest(value: &serde_json::Value) -> String {
    mm_core::content_hash(canonical_json(value).as_bytes())
}
