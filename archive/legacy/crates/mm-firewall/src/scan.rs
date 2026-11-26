//! The bounded judgments: a small fixed set of questions, one per judgment the
//! firewall needs, answered by whichever [`DecisionCore`] is configured.
//!
//! Everything in this module exists to make one direction impossible. A judgment
//! can raise the outcome and cannot lower it, and three mechanisms enforce that:
//!
//! * **Calibration before trust.** A core answers with a pre-calibration
//!   confidence; the class's [`Calibrator`] turns it into a calibrated one. A class
//!   with no fitted calibrator is *uncalibrated*, and an uncalibrated answer is
//!   never treated as truth: it becomes `VERIFY_FIRST`, at either confidence. The
//!   plan is explicit about this, and it is the failure mode that matters most —
//!   an uncalibrated 0.95 is a number, not a fact.
//! * **A conformal threshold before proceeding.** A class with no fitted
//!   [`ConformalSet`] admits nothing. That is the conservative reading of "no
//!   threshold has been established", and it means adding a class without fitting
//!   it is loud rather than silent.
//! * **A refusal is never a permissive answer.** When the core is unavailable, the
//!   question that was decision-critical is escalated; when the answer is simply
//!   missing, the input's own rails stand alone. There is no path here where "no
//!   answer" becomes "no objection".
//!
//! The questions are built from the input and are therefore *stable* for a given
//! input: the rules core matches on their text, and a prompt that changed with the
//! clock would make the rules table useless. The question ids come from the
//! caller's [`UlidFactory`], so a run's consulted questions are identifiable and
//! persistable.

use std::collections::BTreeMap;

use mm_core::{Ulid, UlidFactory};
use mm_decision::calibration::{Calibrator, CalibratorSet};
use mm_decision::conformal::ConformalSet;
use mm_decision::core::{answer_or_none, CoreId, SharedCore};
use mm_decision::question::{
    AnswerValue, DecisionAnswer, DecisionQuestion, DecisionState, QuestionKind, ScoreScale,
};

use crate::error::{FirewallError, Result};
use crate::outcome::{
    normalized_signals, outcome_for_code, upsert_signal_at, FirewallOutcome, ReasonCode,
};
use crate::report::FirewallInput;

/// The judgment class for whether the action is safe to take.
pub const CLASS_SAFETY: &str = "safety";
/// The judgment class for whether the authorization suffices.
pub const CLASS_AUTHORIZATION: &str = "authorization";
/// The judgment class for how much residual risk remains.
pub const CLASS_RESIDUAL_RISK: &str = "residual_risk";
/// The judgment class for whether the comparison evidence is admissible.
pub const CLASS_COMPARISON: &str = "comparison";
/// The judgment class for whether a simpler alternative would do.
pub const CLASS_ALTERNATIVE: &str = "alternative";

/// Every judgment class, in the order the questions are asked.
pub const DECISION_CLASSES: [&str; 5] = [
    CLASS_SAFETY,
    CLASS_AUTHORIZATION,
    CLASS_RESIDUAL_RISK,
    CLASS_COMPARISON,
    CLASS_ALTERNATIVE,
];

/// A confidence at or above this, from an uncalibrated class, is escalated.
///
/// Below it the answer is escalated as well — an uncalibrated answer never passes —
/// but the threshold decides which reason code is raised, and "confident but
/// uncalibrated" is a different thing for an operator to act on than "uncalibrated
/// and unsure".
pub const UNCALIBRATED_HIGH_CONFIDENCE: f32 = 0.6;

/// The coverage target used for a class with no fitted conformal threshold.
pub const DEFAULT_TARGET_COVERAGE: f32 = 0.9;

/// What the firewall is configured with.
pub struct FirewallConfig {
    /// The core to ask, if one is configured. `None` is a rails-only evaluation.
    pub core: Option<SharedCore>,
    /// The fitted calibrators, one per judgment class.
    pub calibrators: CalibratorSet,
    /// The fitted conformal thresholds, one per judgment class.
    pub conformal: BTreeMap<String, ConformalSet>,
    /// A calibrated confidence below this keeps the residual risk cautious.
    pub caution_cutoff: f32,
    /// A calibrated confidence below this, or an unadmitted answer, escalates.
    pub verify_cutoff: f32,
}

impl Default for FirewallConfig {
    fn default() -> Self {
        FirewallConfig {
            core: None,
            calibrators: CalibratorSet::new(),
            conformal: BTreeMap::new(),
            caution_cutoff: 0.35,
            verify_cutoff: 0.6,
        }
    }
}

impl FirewallConfig {
    /// A config that asks `core` and has nothing fitted.
    pub fn with_core(core: SharedCore) -> Self {
        FirewallConfig {
            core: Some(core),
            ..FirewallConfig::default()
        }
    }

    /// Refuse a config whose cutoffs cannot order anything.
    pub fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("caution_cutoff", self.caution_cutoff),
            ("verify_cutoff", self.verify_cutoff),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(FirewallError::validation(
                    "firewall_cutoff",
                    format!("{field} must be in [0,1], got {value}"),
                ));
            }
        }
        if self.caution_cutoff > self.verify_cutoff {
            return Err(FirewallError::validation(
                "firewall_cutoff",
                format!(
                    "caution_cutoff {} must not exceed verify_cutoff {}",
                    self.caution_cutoff, self.verify_cutoff
                ),
            ));
        }
        Ok(())
    }

    /// The calibrator for a class, or an uncalibrated one.
    pub fn calibrator_for(&self, class: &str) -> Calibrator {
        self.calibrators
            .get(class)
            .cloned()
            .unwrap_or_else(|| Calibrator::uncalibrated(class))
    }

    /// The conformal threshold for a class, or the empty one.
    ///
    /// The empty set has a `q_hat` of zero and admits nothing, which is the whole
    /// point: a class whose threshold was never fitted escalates rather than
    /// passing.
    pub fn conformal_for(&self, class: &str) -> ConformalSet {
        self.conformal
            .get(class)
            .cloned()
            .unwrap_or_else(|| ConformalSet::fit(class, &[], DEFAULT_TARGET_COVERAGE))
    }
}

/// The live state of one evaluation.
pub struct FirewallCtx {
    /// What the firewall is configured with.
    pub config: FirewallConfig,
    /// The id source, so a run's questions and report are identifiable.
    pub ids: UlidFactory,
    /// The correlation id shared with the log and audit records of this run.
    pub trace: Option<Ulid>,
    /// Every answer the configured core gave, kept whole so a caller can persist
    /// them: a firewall that discarded what it consulted could not be calibrated
    /// on later.
    pub consulted: Vec<DecisionAnswer>,
}

impl FirewallCtx {
    /// A context over `config`.
    pub fn new(config: FirewallConfig) -> Self {
        FirewallCtx {
            config,
            ids: UlidFactory::new(),
            trace: None,
            consulted: Vec::new(),
        }
    }

    /// The next identifier.
    pub fn next_id(&self) -> Ulid {
        self.ids.next()
    }
}

/// One bounded judgment, as the firewall saw it.
///
/// The calibrated and uncalibrated confidences are both kept, along with whether a
/// calibrator and a threshold were fitted at all, because the interesting question
/// about a run in hindsight is not what it concluded but how much of what it
/// concluded was calibrated.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundedJudgment {
    /// Which shape the question had.
    pub question_kind: QuestionKind,
    /// The judgment class.
    pub class: String,
    /// The answer as the core gave it, or `None` when the core refused.
    pub answer_value: Option<AnswerValue>,
    /// The confidence before calibration.
    pub confidence: f32,
    /// The confidence after the class's calibrator.
    pub calibrated_confidence: f32,
    /// True when a calibrator was fitted for the class.
    pub calibrated: bool,
    /// True when the class's conformal threshold admitted the answer.
    pub admitted: bool,
    /// True when a conformal threshold was fitted for the class.
    pub threshold_fitted: bool,
    /// Which core answered.
    pub core: Option<CoreId>,
    /// Why the core could not answer, when it could not.
    pub unavailable_reason: Option<String>,
}

impl BoundedJudgment {
    /// The answer, rendered for a report.
    pub fn answer(&self) -> Option<String> {
        self.answer_value.as_ref().map(AnswerValue::canonical)
    }

    /// Read the boolean a `YesNo` answer carried.
    pub fn bool_answer(&self) -> Option<bool> {
        match &self.answer_value {
            Some(AnswerValue::Bool { value }) => Some(*value),
            _ => None,
        }
    }

    /// Read the score a `Score` answer carried.
    pub fn score_answer(&self) -> Option<f32> {
        match &self.answer_value {
            Some(AnswerValue::Score { value }) => Some(*value),
            _ => None,
        }
    }

    /// A stable rendering.
    pub fn canonical(&self) -> String {
        format!(
            "class={},kind={},answer={},confidence={:.9},calibrated_confidence={:.9},calibrated={},admitted={},threshold_fitted={},core={},unavailable={}",
            self.class,
            self.question_kind.as_str(),
            self.answer().as_deref().unwrap_or("-"),
            self.confidence,
            self.calibrated_confidence,
            self.calibrated,
            self.admitted,
            self.threshold_fitted,
            self.core.map_or("-", CoreId::as_str),
            self.unavailable_reason.as_deref().unwrap_or("-")
        )
    }
}

/// What the bounded-judgment layer concluded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanOutcome {
    /// The signals the judgments justify, sorted for a report.
    pub signals: Vec<ReasonCode>,
    /// The numeric magnitudes behind them, keyed by reason code.
    pub signals_map: BTreeMap<String, f64>,
    /// The questions consulted, in the order they were asked.
    pub decisions: Vec<Ulid>,
    /// The judgments, one per question asked.
    pub judgments: Vec<BoundedJudgment>,
}

impl ScanOutcome {
    /// The signal codes, in report order.
    pub fn codes(&self) -> Vec<String> {
        self.signals.iter().map(|s| s.code.clone()).collect()
    }

    /// True when a code was raised.
    pub fn raised(&self, code: &str) -> bool {
        self.signals.iter().any(|signal| signal.code == code)
    }
}

/// The question about whether the action is safe to take as described.
pub fn safety_question(id: Ulid, input: &FirewallInput) -> DecisionQuestion {
    DecisionQuestion::YesNo {
        id,
        prompt: format!(
            "Is this action safe to proceed with as described? reversibility={} stakes={:.3}",
            input.reversibility.as_str(),
            input.stakes.level
        ),
    }
}

/// The question about whether the authorization suffices.
pub fn authorization_question(id: Ulid, input: &FirewallInput) -> DecisionQuestion {
    DecisionQuestion::YesNo {
        id,
        prompt: format!(
            "Is the authorization for this action sufficient? granted={} scope={}",
            input.authorization.granted, input.authorization.scope
        ),
    }
}

/// The question about how much residual risk remains.
///
/// A `Score` rather than a `YesNo` because the answer is used as a magnitude: it
/// decides whether the outcome is `VERIFY_FIRST` or `PROCEED_WITH_CAUTION`, and a
/// boolean would throw away the distinction the precedence is built on.
pub fn residual_risk_question(id: Ulid, _input: &FirewallInput) -> DecisionQuestion {
    DecisionQuestion::Score {
        id,
        prompt: "How much residual risk remains after the stated mitigations?".to_string(),
        scale: ScoreScale::unit(),
    }
}

/// The question about whether the comparison evidence is admissible.
pub fn comparison_question(id: Ulid, _input: &FirewallInput) -> DecisionQuestion {
    DecisionQuestion::YesNo {
        id,
        prompt: "Is the comparison evidence valid and comparable?".to_string(),
    }
}

/// The question about whether a simpler alternative would do.
pub fn alternative_question(id: Ulid, _input: &FirewallInput) -> DecisionQuestion {
    DecisionQuestion::YesNo {
        id,
        prompt: "Is a simpler alternative available that achieves the same objective?".to_string(),
    }
}

/// True when a refusal to answer this class is itself decision-critical.
///
/// The reasoning, class by class, because this is the one place a missing answer
/// could be mistaken for a permissive one:
///
/// * **safety** — always critical. A firewall that cannot judge whether the action
///   is safe does not get to `PROCEED` by default.
/// * **authorization** — critical exactly when the grant is absent. The input's
///   rails already raise `ASK_USER` there, so the missing judgment adds nothing;
///   when a grant *is* present, a core that cannot double-check it does not make
///   the grant untrue.
/// * **residual_risk** — always critical, for the same reason as safety.
/// * **comparison** — critical when the caller asserted the comparison was valid.
///   A caller that has already found its own comparison inadmissible does not need
///   a core to confirm it, and the rail has raised `REPLAN`.
/// * **alternative** — never critical. Whether a simpler alternative exists is
///   input data, and the rail fires on it directly.
fn refusal_is_critical(class: &str, input: &FirewallInput) -> bool {
    match class {
        CLASS_SAFETY | CLASS_RESIDUAL_RISK => true,
        CLASS_AUTHORIZATION => !input.authorization.granted,
        CLASS_COMPARISON => input.comparisons.iter().any(|c| c.is_valid()),
        CLASS_ALTERNATIVE => false,
        // A class added without a reading of this function escalates: the default
        // has to be the cautious one, or a new judgment becomes a silent hole.
        _ => true,
    }
}

/// The decision state a question is asked against.
///
/// The firewall's whole input travels as `context`, so a core sees exactly what the
/// rails saw. The `facts` list is deliberately empty: the caller's claim set is not
/// something the firewall can enumerate, and inventing one here — say, treating
/// verified assumptions as facts — would hand a core evidence the caller never
/// asserted.
fn decision_state(input: &FirewallInput) -> Result<DecisionState> {
    let context = serde_json::to_value(input)
        .map_err(|e| FirewallError::Scan(format!("cannot render the input as context: {e}")))?;
    Ok(DecisionState::with_refs(
        input.episode,
        Vec::new(),
        input.assumptions.iter().map(|a| a.id).collect(),
        input.constraints.iter().map(|c| c.id.clone()).collect(),
        context,
    ))
}

/// Ask one question and turn the answer into a judgment.
///
/// The returned judgment is always present, even when the core refused, because
/// "the core was unavailable" is itself the finding.
async fn judge(
    ctx: &mut FirewallCtx,
    input: &FirewallInput,
    class: &str,
    question: DecisionQuestion,
    signals: &mut Vec<ReasonCode>,
    magnitudes: &mut BTreeMap<String, f64>,
) -> Result<BoundedJudgment> {
    let decision_critical = refusal_is_critical(class, input);
    let state = decision_state(input)?;
    let calibrator = ctx.config.calibrator_for(class);
    let calibrated = calibrator.is_calibrated();
    let conformal = ctx.config.conformal_for(class);
    let threshold_fitted = conformal.n > 0;
    let origin_core = ctx.config.core.as_ref().map(|core| core.id());

    let response = answer_or_none(ctx.config.core.as_ref(), &question, &state).await;
    let Some(response) = response else {
        // No core at all: the caller asked for a rails-only evaluation. Nothing is
        // fabricated and nothing is escalated, because there was never a judgment
        // to be missing — the operator chose the configuration.
        return Ok(BoundedJudgment {
            question_kind: question.kind(),
            class: class.to_string(),
            answer_value: None,
            confidence: 0.0,
            calibrated_confidence: 0.0,
            calibrated: false,
            admitted: false,
            threshold_fitted,
            core: None,
            unavailable_reason: None,
        });
    };

    let decided = match response {
        Err(error) if mm_decision::is_legal_refusal(&error) => {
            let reason = error.to_string();
            if decision_critical {
                upsert_signal_at(
                    signals,
                    "signal.high_uncertainty",
                    format!("{class}: the core could not answer ({reason})"),
                    FirewallOutcome::VerifyFirst,
                );
                magnitudes.insert("signal.high_uncertainty".to_string(), 1.0);
            }
            return Ok(BoundedJudgment {
                question_kind: question.kind(),
                class: class.to_string(),
                answer_value: None,
                confidence: 0.0,
                calibrated_confidence: 0.0,
                calibrated,
                admitted: false,
                threshold_fitted,
                core: origin_core,
                unavailable_reason: Some(reason),
            });
        }
        Err(error) => {
            return Err(FirewallError::Scan(format!(
                "{class}: the core refused the answer rather than being unavailable: {} ({})",
                error,
                error.code()
            )))
        }
        Ok(answer) => answer,
    };

    let confidence = decided.confidence;
    let calibrated_confidence = calibrator.calibrate(confidence);
    let admitted = conformal.admit(calibrated_confidence);
    let rendered = decided.answer.canonical();
    ctx.consulted.push(decided.clone());

    if !calibrated {
        // An uncalibrated confidence is not evidence, whichever way it points.
        let code = if confidence >= UNCALIBRATED_HIGH_CONFIDENCE {
            "signal.uncalibrated_confidence"
        } else {
            "signal.high_uncertainty"
        };
        upsert_signal_at(
            signals,
            code,
            format!(
                "{class}: no calibrator is fitted and the core answered {rendered} at {confidence:.3}"
            ),
            FirewallOutcome::VerifyFirst,
        );
        magnitudes.insert(code.to_string(), f64::from(confidence));
    } else if !admitted || calibrated_confidence < ctx.config.verify_cutoff {
        upsert_signal_at(
            signals,
            "signal.high_uncertainty",
            format!(
                "{class}: calibrated confidence {calibrated_confidence:.3} (verify cutoff {:.3}, admitted {admitted})",
                ctx.config.verify_cutoff
            ),
            FirewallOutcome::VerifyFirst,
        );
        magnitudes.insert(
            "signal.high_uncertainty".to_string(),
            f64::from(calibrated_confidence),
        );
    }

    Ok(BoundedJudgment {
        question_kind: question.kind(),
        class: class.to_string(),
        answer_value: Some(decided.answer.clone()),
        confidence,
        calibrated_confidence,
        calibrated,
        admitted,
        threshold_fitted,
        core: Some(decided.core),
        unavailable_reason: None,
    })
}

/// Ask the bounded questions and turn the answers into signals.
///
/// The judgments are asked in [`DECISION_CLASSES`] order. Each contributes signals
/// according to what its answer *means*, not according to what it says:
///
/// * a negative **safety** judgment is a residual risk that must be verified — the
///   core is saying the action is not safe, and no confidence in that reading makes
///   the action safe;
/// * a negative **authorization** judgment is `ASK_USER`, the same reading the rail
///   gives an absent grant;
/// * the **residual risk** score is compared against the cutoffs, and a score at or
///   above `verify_cutoff` is `VERIFY_FIRST` while a score at or above
///   `caution_cutoff` is `PROCEED_WITH_CAUTION`;
/// * a negative **comparison** judgment is `REPLAN`, again the rail's own reading;
/// * a *positive* **alternative** judgment is cautionary, because "a simpler
///   alternative exists" is a reason for care rather than for confidence.
///
/// With no core configured the judgments are skipped entirely and only the
/// factuality reading remains, because there is nothing to ask and inventing a
/// judgment would be exactly the fabrication this crate exists to prevent.
pub async fn scan(input: &FirewallInput, ctx: &mut FirewallCtx) -> Result<ScanOutcome> {
    ctx.config.validate()?;
    input.validate()?;

    let mut signals: Vec<ReasonCode> = Vec::new();
    let mut magnitudes: BTreeMap<String, f64> = BTreeMap::new();
    let mut judgments: Vec<BoundedJudgment> = Vec::new();
    let mut decisions: Vec<Ulid> = Vec::new();

    if ctx.config.core.is_some() {
        // ------------------------------------------------------------ safety ----
        let id = ctx.next_id();
        decisions.push(id);
        let judgment = judge(
            ctx,
            input,
            CLASS_SAFETY,
            safety_question(id, input),
            &mut signals,
            &mut magnitudes,
        )
        .await?;
        if judgment.bool_answer() == Some(false) {
            upsert_signal_at(
                &mut signals,
                "signal.residual_risk",
                "the safety judgment was negative",
                FirewallOutcome::VerifyFirst,
            );
            magnitudes.insert("signal.residual_risk".to_string(), 1.0);
        }
        judgments.push(judgment);

        // ----------------------------------------------------- authorization ----
        let id = ctx.next_id();
        decisions.push(id);
        let judgment = judge(
            ctx,
            input,
            CLASS_AUTHORIZATION,
            authorization_question(id, input),
            &mut signals,
            &mut magnitudes,
        )
        .await?;
        if judgment.bool_answer() == Some(false) {
            upsert_signal_at(
                &mut signals,
                "signal.missing_authorization",
                "the core judged the authorization insufficient",
                FirewallOutcome::AskUser,
            );
            magnitudes.insert("signal.missing_authorization".to_string(), 1.0);
        }
        judgments.push(judgment);

        // ----------------------------------------------------- residual risk ----
        let id = ctx.next_id();
        decisions.push(id);
        let judgment = judge(
            ctx,
            input,
            CLASS_RESIDUAL_RISK,
            residual_risk_question(id, input),
            &mut signals,
            &mut magnitudes,
        )
        .await?;
        if let Some(score) = judgment.score_answer() {
            if score >= ctx.config.verify_cutoff {
                upsert_signal_at(
                    &mut signals,
                    "signal.residual_risk",
                    format!(
                        "residual risk {score:.3} is at or above the verify cutoff {:.3}",
                        ctx.config.verify_cutoff
                    ),
                    FirewallOutcome::VerifyFirst,
                );
            } else if score >= ctx.config.caution_cutoff {
                upsert_signal_at(
                    &mut signals,
                    "signal.residual_risk",
                    format!(
                        "residual risk {score:.3} is at or above the caution cutoff {:.3}",
                        ctx.config.caution_cutoff
                    ),
                    FirewallOutcome::ProceedWithCaution,
                );
            }
            magnitudes.insert("signal.residual_risk".to_string(), f64::from(score));
        }
        judgments.push(judgment);

        // ------------------------------------------------------- comparison ----
        if !input.comparisons.is_empty() {
            let id = ctx.next_id();
            decisions.push(id);
            let judgment = judge(
                ctx,
                input,
                CLASS_COMPARISON,
                comparison_question(id, input),
                &mut signals,
                &mut magnitudes,
            )
            .await?;
            if judgment.bool_answer() == Some(false) {
                upsert_signal_at(
                    &mut signals,
                    "signal.invalid_comparison",
                    "the core judged the comparison evidence inadmissible",
                    FirewallOutcome::Replan,
                );
                magnitudes.insert("signal.invalid_comparison".to_string(), 1.0);
            }
            judgments.push(judgment);
        }

        // ------------------------------------------------------ alternative ----
        let id = ctx.next_id();
        decisions.push(id);
        let judgment = judge(
            ctx,
            input,
            CLASS_ALTERNATIVE,
            alternative_question(id, input),
            &mut signals,
            &mut magnitudes,
        )
        .await?;
        if judgment.bool_answer() == Some(true) {
            upsert_signal_at(
                &mut signals,
                "signal.simpler_alternative_available",
                "the core judged a simpler alternative available",
                FirewallOutcome::ProceedWithCaution,
            );
            magnitudes.insert("signal.simpler_alternative_available".to_string(), 1.0);
        }
        judgments.push(judgment);
    }

    // The factuality reading is input, not judgment: the check itself belongs to
    // `mm-decision::factuality`, which may use a Phase 10 tool, and all the firewall
    // does with its support score is compare it against the verify cutoff.
    if let Some(support) = input.factuality_support {
        if support < ctx.config.verify_cutoff {
            upsert_signal_at(
                &mut signals,
                "signal.low_factuality",
                format!(
                    "factuality support {support:.3} is below the verify cutoff {:.3}",
                    ctx.config.verify_cutoff
                ),
                outcome_for_code("signal.low_factuality"),
            );
            magnitudes.insert("signal.low_factuality".to_string(), f64::from(support));
        }
    }

    let signals = normalized_signals(&signals);
    magnitudes.retain(|code, _| signals.iter().any(|signal| &signal.code == code));

    Ok(ScanOutcome {
        signals,
        signals_map: magnitudes,
        decisions,
        judgments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{
        AssumptionRef, AuthorizationRef, ComparisonOutcomeRef, ConstraintRef, MissingStep, Stakes,
    };
    use async_trait::async_trait;
    use mm_decision::core::DecisionCore;
    use std::sync::Arc;

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    /// A risk budget nothing in these fixtures can deplete, so a fixture about
    /// judgments is not also a fixture about catastrophic risk.
    fn risks() -> mm_decision::risk::RiskProfile {
        mm_decision::risk::analyze_risk_with(
            &[mm_decision::risk::LossOutcome {
                name: "small".into(),
                probability: 1.0,
                loss: 1.0,
            }],
            mm_decision::risk::RiskMeasure::Variance,
            &mm_decision::risk::RiskOptions {
                capital: 1.0e9,
                reversibility: 0.5,
                optionality: 0.5,
            },
        )
    }

    fn input() -> FirewallInput {
        FirewallInput::new(ulid(1), risks())
    }

    /// A core that answers every question the same way, deterministically.
    struct FixedCore {
        safety: bool,
        authorization: bool,
        residual_risk: f32,
        comparison: bool,
        alternative: bool,
        confidence: f32,
    }

    impl FixedCore {
        fn permissive() -> Self {
            FixedCore {
                safety: true,
                authorization: true,
                residual_risk: 0.05,
                comparison: true,
                alternative: false,
                confidence: 0.95,
            }
        }
    }

    #[async_trait]
    impl DecisionCore for FixedCore {
        fn id(&self) -> CoreId {
            CoreId::Rules
        }

        async fn answer(
            &self,
            question: &DecisionQuestion,
            _state: &DecisionState,
        ) -> mm_decision::Result<DecisionAnswer> {
            let value = match question {
                DecisionQuestion::YesNo { prompt, .. } => {
                    let value = if prompt.contains("safe to proceed") {
                        self.safety
                    } else if prompt.contains("authorization") {
                        self.authorization
                    } else if prompt.contains("comparison evidence") {
                        self.comparison
                    } else {
                        self.alternative
                    };
                    AnswerValue::Bool { value }
                }
                DecisionQuestion::Score { .. } => AnswerValue::Score {
                    value: self.residual_risk,
                },
                DecisionQuestion::Choice { .. } => AnswerValue::None,
            };
            DecisionAnswer::new(
                question,
                value,
                self.confidence,
                mm_decision::FeatureVector::new(),
                CoreId::Rules,
                0,
            )
        }
    }

    /// A core that refuses every question.
    struct RefusingCore;

    #[async_trait]
    impl DecisionCore for RefusingCore {
        fn id(&self) -> CoreId {
            CoreId::Local
        }

        async fn answer(
            &self,
            _question: &DecisionQuestion,
            _state: &DecisionState,
        ) -> mm_decision::Result<DecisionAnswer> {
            Err(mm_decision::DecisionError::Unavailable(
                "no head is loaded".into(),
            ))
        }
    }

    fn ctx_with(core: Option<SharedCore>) -> FirewallCtx {
        FirewallCtx::new(FirewallConfig {
            core,
            ..FirewallConfig::default()
        })
    }

    #[tokio::test]
    async fn no_core_produces_no_judgments_and_no_error() {
        let mut ctx = ctx_with(None);
        let outcome = scan(&input(), &mut ctx).await.unwrap();
        assert!(outcome.judgments.is_empty());
        assert!(outcome.decisions.is_empty());
        assert!(outcome.signals.is_empty(), "{:?}", outcome.signals);
        assert!(ctx.consulted.is_empty());
    }

    #[tokio::test]
    async fn an_uncalibrated_confidence_never_passes() {
        // The core is maximally confident and says everything is fine; no
        // calibrator is fitted, so nothing is admitted and the run escalates.
        let mut ctx = ctx_with(Some(Arc::new(FixedCore::permissive())));
        let outcome = scan(&input(), &mut ctx).await.unwrap();
        assert!(
            outcome.raised("signal.uncalibrated_confidence"),
            "{:?}",
            outcome.codes()
        );
        assert!(
            !ctx.consulted.is_empty(),
            "the answers are kept for calibration"
        );
        for judgment in &outcome.judgments {
            assert!(!judgment.calibrated);
            assert!(!judgment.admitted, "an unfitted threshold admits nothing");
            assert!(!judgment.threshold_fitted);
        }
    }

    #[tokio::test]
    async fn a_refusal_is_recorded_and_never_fabricated() {
        let mut ctx = ctx_with(Some(Arc::new(RefusingCore)));
        let outcome = scan(&input(), &mut ctx).await.unwrap();
        assert!(!outcome.judgments.is_empty());
        for judgment in &outcome.judgments {
            assert!(judgment.answer_value.is_none());
            assert_eq!(
                judgment.unavailable_reason.as_deref(),
                Some("unavailable: no head is loaded")
            );
        }
        // Safety and residual risk are decision-critical, so the refusals escalate.
        assert!(outcome.raised("signal.high_uncertainty"));
        assert!(ctx.consulted.is_empty());
    }

    #[tokio::test]
    async fn a_negative_safety_judgment_is_a_residual_risk() {
        let mut core = FixedCore::permissive();
        core.safety = false;
        let mut ctx = ctx_with(Some(Arc::new(core)));
        let outcome = scan(&input(), &mut ctx).await.unwrap();
        assert!(outcome.raised("signal.residual_risk"));
        let (code, _) = crate::aggregate::aggregate(&outcome.signals);
        assert_eq!(code, FirewallOutcome::VerifyFirst);
    }

    /// A context whose every decision class is calibrated and has a fitted conformal
    /// threshold that admits a confident answer.
    ///
    /// Without this, an uncalibrated class raises `VERIFY_FIRST` on every run, which
    /// is the plan's own rule (§4.2) but masks whatever the judgment itself would
    /// have concluded. A test about a judgment's own reading has to remove the floor
    /// it sits on rather than assert through it.
    fn calibrated_ctx(core: SharedCore) -> FirewallCtx {
        let mut calibrators = CalibratorSet::new();
        let mut conformal = BTreeMap::new();
        for class in DECISION_CLASSES {
            let predicted: Vec<f32> = (0..16).map(|i| 0.9 + i as f32 * 0.006).collect();
            let labels = vec![true; 16];
            calibrators.insert(Calibrator::fit(class, &predicted, &labels));
            // Nonconformity scores around 0.1, so a `0.95`-confidence answer
            // (nonconformity 0.05) is admitted.
            let scores: Vec<f32> = (0..16).map(|i| 0.08 + i as f32 * 0.002).collect();
            conformal.insert(class.to_string(), ConformalSet::fit(class, &scores, 0.9));
        }
        FirewallCtx::new(FirewallConfig {
            core: Some(core),
            calibrators,
            conformal,
            ..FirewallConfig::default()
        })
    }

    #[tokio::test]
    async fn a_negative_comparison_judgment_is_a_replan() {
        let mut core = FixedCore::permissive();
        core.comparison = false;
        let mut ctx = calibrated_ctx(Arc::new(core));
        let mut input = input();
        input.comparisons = vec![ComparisonOutcomeRef {
            id: ulid(4),
            verdict: "valid".into(),
            objective: "pick a datastore".into(),
        }];
        let outcome = scan(&input, &mut ctx).await.unwrap();
        assert!(outcome.raised("signal.invalid_comparison"));
        assert!(
            !outcome.raised("signal.high_uncertainty"),
            "{:?}",
            outcome.codes()
        );
        let (code, _) = crate::aggregate::aggregate(&outcome.signals);
        assert_eq!(code, FirewallOutcome::Replan);
    }

    #[tokio::test]
    async fn an_invalid_comparison_cannot_be_masked_by_an_uncalibrated_class() {
        // The same negative judgment, with no calibrator fitted. The plan's rule is
        // that a high-confidence uncalibrated answer is never admitted, so the
        // outcome is the *more* cautious `VERIFY_FIRST` and the `REPLAN` reading is
        // still reported alongside it.
        let mut core = FixedCore::permissive();
        core.comparison = false;
        let mut ctx = ctx_with(Some(Arc::new(core)));
        let mut input = input();
        input.comparisons = vec![ComparisonOutcomeRef {
            id: ulid(4),
            verdict: "valid".into(),
            objective: "pick a datastore".into(),
        }];
        let outcome = scan(&input, &mut ctx).await.unwrap();
        assert!(outcome.raised("signal.invalid_comparison"));
        assert!(outcome.raised("signal.uncalibrated_confidence"));
        let (code, _) = crate::aggregate::aggregate(&outcome.signals);
        assert_eq!(code, FirewallOutcome::VerifyFirst);
    }

    #[tokio::test]
    async fn the_comparison_question_is_skipped_when_there_is_nothing_to_compare() {
        let mut ctx = ctx_with(Some(Arc::new(FixedCore::permissive())));
        let outcome = scan(&input(), &mut ctx).await.unwrap();
        assert!(!outcome
            .judgments
            .iter()
            .any(|j| j.class == CLASS_COMPARISON));
        assert_eq!(outcome.judgments.len(), DECISION_CLASSES.len() - 1);
    }

    #[tokio::test]
    async fn a_low_factuality_score_is_a_verify_first() {
        let mut ctx = ctx_with(None);
        let mut input = input();
        input.factuality_support = Some(0.2);
        let outcome = scan(&input, &mut ctx).await.unwrap();
        assert_eq!(outcome.codes(), vec!["signal.low_factuality"]);
        assert_eq!(outcome.signals[0].severity, FirewallOutcome::VerifyFirst);

        input.factuality_support = Some(0.9);
        let mut ctx = ctx_with(None);
        assert!(scan(&input, &mut ctx).await.unwrap().signals.is_empty());
    }

    #[tokio::test]
    async fn a_high_residual_risk_score_is_a_verify_first() {
        let mut core = FixedCore::permissive();
        core.residual_risk = 0.8;
        let mut ctx = ctx_with(Some(Arc::new(core)));
        let outcome = scan(&input(), &mut ctx).await.unwrap();
        assert!(outcome.raised("signal.residual_risk"));
        let (code, _) = crate::aggregate::aggregate(&outcome.signals);
        assert_eq!(code, FirewallOutcome::VerifyFirst);
    }

    #[tokio::test]
    async fn a_positive_alternative_judgment_is_cautionary() {
        let mut core = FixedCore::permissive();
        core.alternative = true;
        let mut ctx = ctx_with(Some(Arc::new(core)));
        let outcome = scan(&input(), &mut ctx).await.unwrap();
        assert!(outcome.raised("signal.simpler_alternative_available"));
    }

    #[test]
    fn the_question_builders_are_stable_for_one_input() {
        let mut input = input();
        input.stakes = Stakes {
            level: 0.7,
            irreversibility: 0.3,
        };
        input.assumptions = vec![AssumptionRef {
            id: ulid(2),
            claim: "a".into(),
            decision_critical: true,
            verified: true,
        }];
        input.constraints = vec![ConstraintRef {
            id: "c".into(),
            hard: false,
            satisfied: true,
        }];
        input.missing_steps = vec![MissingStep {
            step: "s".into(),
            decision_critical: true,
        }];
        input.authorization = AuthorizationRef {
            granted: true,
            scope: "read-only".into(),
            approval_id: None,
        };
        assert_eq!(
            safety_question(ulid(9), &input).canonical(),
            safety_question(ulid(9), &input).canonical()
        );
        assert!(authorization_question(ulid(9), &input)
            .prompt()
            .contains("read-only"));
        assert!(safety_question(ulid(9), &input).prompt().contains("0.700"));
        assert_eq!(
            residual_risk_question(ulid(9), &input).kind(),
            QuestionKind::Score
        );
        assert_eq!(
            comparison_question(ulid(9), &input).kind(),
            QuestionKind::YesNo
        );
        assert_eq!(
            alternative_question(ulid(9), &input).kind(),
            QuestionKind::YesNo
        );
    }

    #[test]
    fn cutoffs_must_order_correctly() {
        let mut config = FirewallConfig::default();
        assert!(config.validate().is_ok());
        config.caution_cutoff = 0.9;
        config.verify_cutoff = 0.1;
        assert!(config.validate().is_err());
        config.caution_cutoff = 1.5;
        config.verify_cutoff = 1.5;
        assert!(config.validate().is_err());
    }

    #[test]
    fn an_unfitted_class_admits_nothing() {
        let config = FirewallConfig::default();
        let conformal = config.conformal_for(CLASS_SAFETY);
        assert_eq!(conformal.n, 0);
        assert!(!conformal.admit(0.99));
        let calibrator = config.calibrator_for(CLASS_SAFETY);
        assert!(!calibrator.is_calibrated());
        assert_eq!(calibrator.decision_class, CLASS_SAFETY);
    }

    #[test]
    fn refusal_criticality_is_read_class_by_class() {
        let mut input = input();
        assert!(refusal_is_critical(CLASS_SAFETY, &input));
        assert!(refusal_is_critical(CLASS_RESIDUAL_RISK, &input));
        assert!(!refusal_is_critical(CLASS_AUTHORIZATION, &input));
        assert!(!refusal_is_critical(CLASS_COMPARISON, &input));
        assert!(!refusal_is_critical(CLASS_ALTERNATIVE, &input));
        assert!(refusal_is_critical("something_new", &input));

        input.authorization = AuthorizationRef {
            granted: false,
            scope: "none".into(),
            approval_id: None,
        };
        assert!(refusal_is_critical(CLASS_AUTHORIZATION, &input));

        input.comparisons = vec![ComparisonOutcomeRef {
            id: ulid(4),
            verdict: "valid".into(),
            objective: "pick a datastore".into(),
        }];
        assert!(refusal_is_critical(CLASS_COMPARISON, &input));
    }
}
