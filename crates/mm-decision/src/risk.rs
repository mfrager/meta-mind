//! Deterministic risk measures over an outcome distribution.
//!
//! Phase 9 asks for eight measures — expected loss, maximum loss, tail
//! probability, ruin probability, variance, downside asymmetry, reversibility and
//! optionality — and for the numbers to match hand computation within `1e-6`
//! (`bench/risk/reference_values.json`). Two rules follow from that and shape this
//! module:
//!
//! * **The numbers are arithmetic, never model output.** Every field of a
//!   [`RiskProfile`] is a closed-form function of the outcome list, so the same
//!   list always yields the same profile and a reference value is a *check*, not a
//!   tolerance around a guess.
//! * **Dispersion is reported separately from expectation.** A distribution whose
//!   mean loss is small can still be ruinous, so [`RiskProfile::max_loss`] and
//!   [`RiskProfile::ruin_probability`] stand on their own rather than being folded
//!   into one "risk score" a caller could average away.
//!
//! The requested measure's value comes from the vendored `decision-ir` engine
//! rather than being re-derived here, so the engine stays the single authority for
//! what `Var` and `Cvar` mean. No `decision-ir` type appears in this crate's public
//! surface: the mapping lives in [`engine_value`] and nothing else can see it.

use serde::{Deserialize, Serialize};

use crate::error::{DecisionError, Result};

/// The risk measure applied when a profile is summarized by one number.
///
/// This mirrors the vendored engine's measure set rather than wrapping it —
/// wrapping would put an `decision-ir` type in a public signature, which the phase
/// plan forbids.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskMeasure {
    /// The variance of the loss distribution.
    Variance,
    /// The value at risk at level `alpha` (0 < alpha <= 1): the loss value
    /// bounding the worst `alpha` probability mass.
    Var {
        /// The tail level.
        alpha: f64,
    },
    /// The conditional value at risk at level `alpha`: the expected loss within
    /// the worst `alpha` probability mass.
    Cvar {
        /// The tail level.
        alpha: f64,
    },
}

/// Every measure's wire name, in the order the CLI reports them.
pub const RISK_MEASURE_NAMES: [&str; 3] = ["variance", "var", "cvar"];

impl RiskMeasure {
    /// The stable wire name, which is also the `risk.analyze` log field.
    pub fn name(&self) -> &'static str {
        match self {
            RiskMeasure::Variance => "variance",
            RiskMeasure::Var { .. } => "var",
            RiskMeasure::Cvar { .. } => "cvar",
        }
    }

    /// Parse a wire name. `alpha` is used only by `var` and `cvar`; it is clamped
    /// where it is applied, not here, so a caller cannot lose the value it typed.
    pub fn parse(kind: &str, alpha: f64) -> Option<Self> {
        match kind.trim().to_ascii_lowercase().as_str() {
            "variance" => Some(RiskMeasure::Variance),
            "var" => Some(RiskMeasure::Var { alpha }),
            "cvar" => Some(RiskMeasure::Cvar { alpha }),
            _ => None,
        }
    }

    /// The tail level this measure reads, if it has one. Clamped to `[0,1]` so the
    /// arithmetic below cannot be handed a nonsensical tail.
    fn level(&self) -> Option<f64> {
        match self {
            RiskMeasure::Variance => None,
            RiskMeasure::Var { alpha } | RiskMeasure::Cvar { alpha } => Some(alpha.clamp(0.0, 1.0)),
        }
    }
}

impl std::fmt::Display for RiskMeasure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskMeasure::Variance => f.write_str("variance"),
            RiskMeasure::Var { alpha } => write!(f, "var@{alpha}"),
            RiskMeasure::Cvar { alpha } => write!(f, "cvar@{alpha}"),
        }
    }
}

/// One possible loss, realized with a probability.
///
/// `loss` is signed: a negative loss is a gain. Keeping the sign gives the same
/// module the ability to describe a bet that pays and a bet that costs, without a
/// second convention for which direction is bad.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LossOutcome {
    /// What realizes the loss — an outcome name, not an identifier, because this
    /// list is read in a reference-value file as often as by code.
    pub name: String,
    /// The probability of this outcome, in `[0,1]`.
    pub probability: f64,
    /// The loss realized. Negative is a gain.
    pub loss: f64,
}

/// The parts of a risk analysis that are not functions of the outcomes.
///
/// `capital` is the risk budget: ruin is the probability of losing the whole of
/// it. `reversibility` and `optionality` are judgments the *caller* makes about
/// the action (Phase 4/8 inputs) and are carried through rather than invented
/// here — a function of an outcome distribution cannot know whether an action can
/// be undone.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiskOptions {
    /// The risk budget. Ruin is `P(loss >= capital)`.
    pub capital: f64,
    /// How far the action can be undone, in `[0,1]` (0 = fully irreversible).
    pub reversibility: f32,
    /// How much the action preserves future options, in `[0,1]`.
    pub optionality: f32,
}

impl RiskOptions {
    /// Options whose ruin level is the worst outcome in `outcomes`.
    ///
    /// "Ruin" then reads as "the worst thing that can happen happens", which is the
    /// only non-arbitrary budget available when the caller has not declared one.
    pub fn worst_case(outcomes: &[LossOutcome]) -> Self {
        let capital = outcomes
            .iter()
            .map(|outcome| outcome.loss)
            .fold(f64::NEG_INFINITY, f64::max);
        RiskOptions {
            capital: if capital.is_finite() { capital } else { 0.0 },
            reversibility: 0.0,
            optionality: 0.0,
        }
    }

    /// Refuse options that cannot be acted on.
    pub fn validate(&self) -> Result<()> {
        if !self.capital.is_finite() {
            return Err(DecisionError::validation("risk.capital", "must be finite"));
        }
        for (field, value) in [
            ("risk.reversibility", self.reversibility),
            ("risk.optionality", self.optionality),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(DecisionError::validation(
                    field,
                    format!("must be in [0,1], got {value}"),
                ));
            }
        }
        Ok(())
    }
}

/// The eight measures a decision is graded on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiskProfile {
    /// `Σ p·loss`.
    pub expected_loss: f64,
    /// The largest loss any single outcome realizes.
    pub max_loss: f64,
    /// `P(loss >= var_alpha)`, where `var_alpha` is the loss-space alpha quantile.
    pub tail_probability: f64,
    /// `P(loss >= capital)`.
    pub ruin_probability: f64,
    /// `Σ p·(loss − expected_loss)²`.
    pub variance: f64,
    /// Downside semi-deviation minus upside semi-deviation, both from the
    /// expectation. Positive when the down tail dominates.
    pub downside_asymmetry: f64,
    /// The caller's reversibility judgment, carried through.
    pub reversibility: f32,
    /// The caller's optionality judgment, carried through.
    pub optionality: f32,
    /// The measure `measure_value` reports.
    pub measure: RiskMeasure,
    /// The requested measure's value in loss space, from the vendored engine.
    pub measure_value: f64,
}

impl RiskProfile {
    /// A zeroed profile for an empty or unusable outcome list.
    ///
    /// Returned rather than an error because the analysis functions are total:
    /// a caller that needs a hard refusal validates its inputs first with
    /// [`validate_outcomes`], and a caller that has already validated cannot reach
    /// this branch.
    fn empty(measure: RiskMeasure, options: &RiskOptions) -> Self {
        RiskProfile {
            expected_loss: 0.0,
            max_loss: 0.0,
            tail_probability: 0.0,
            ruin_probability: 0.0,
            variance: 0.0,
            downside_asymmetry: 0.0,
            reversibility: options.reversibility,
            optionality: options.optionality,
            measure,
            measure_value: 0.0,
        }
    }

    /// A fixed rendering, so two runs of the same analysis compare as strings.
    pub fn canonical(&self) -> String {
        format!(
            "measure={},measure_value={:.9},expected_loss={:.9},max_loss={:.9},\
             tail_probability={:.9},ruin_probability={:.9},variance={:.9},\
             downside_asymmetry={:.9},reversibility={:.9},optionality={:.9}",
            self.measure,
            self.measure_value,
            self.expected_loss,
            self.max_loss,
            self.tail_probability,
            self.ruin_probability,
            self.variance,
            self.downside_asymmetry,
            self.reversibility,
            self.optionality
        )
    }

    /// Refuse a profile whose numbers cannot describe a distribution.
    pub fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("risk.expected_loss", self.expected_loss),
            ("risk.max_loss", self.max_loss),
            ("risk.tail_probability", self.tail_probability),
            ("risk.ruin_probability", self.ruin_probability),
            ("risk.variance", self.variance),
            ("risk.downside_asymmetry", self.downside_asymmetry),
            ("risk.measure_value", self.measure_value),
        ] {
            if !value.is_finite() {
                return Err(DecisionError::validation(field, "must be finite"));
            }
        }
        for (field, value) in [
            ("risk.tail_probability", self.tail_probability),
            ("risk.ruin_probability", self.ruin_probability),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(DecisionError::validation(
                    field,
                    format!("must be in [0,1], got {value}"),
                ));
            }
        }
        if self.variance < 0.0 {
            return Err(DecisionError::validation(
                "risk.variance",
                format!("must be non-negative, got {}", self.variance),
            ));
        }
        Ok(())
    }
}

/// Refuse an outcome list that is not a probability distribution.
///
/// The tolerance on the total probability is `1e-9`, not `0`: a distribution
/// assembled from decimal literals in a reference-value file sums to 1 only up to
/// representation error, and refusing that would make the corpus unusable while
/// refusing nothing real.
pub fn validate_outcomes(outcomes: &[LossOutcome]) -> Result<()> {
    if outcomes.is_empty() {
        return Err(DecisionError::validation(
            "risk.outcomes",
            "a risk analysis needs at least one outcome",
        ));
    }
    let mut total = 0.0;
    for (index, outcome) in outcomes.iter().enumerate() {
        if outcome.name.trim().is_empty() {
            return Err(DecisionError::validation(
                "risk.outcomes",
                format!("outcomes[{index}].name is empty"),
            ));
        }
        if !outcome.probability.is_finite() || !(0.0..=1.0).contains(&outcome.probability) {
            return Err(DecisionError::validation(
                "risk.outcomes",
                format!(
                    "outcomes[{index}].probability must be in [0,1], got {}",
                    outcome.probability
                ),
            ));
        }
        if !outcome.loss.is_finite() {
            return Err(DecisionError::validation(
                "risk.outcomes",
                format!(
                    "outcomes[{index}].loss must be finite, got {}",
                    outcome.loss
                ),
            ));
        }
        total += outcome.probability;
    }
    if (total - 1.0).abs() > 1e-9 {
        return Err(DecisionError::validation(
            "risk.outcomes",
            format!("probabilities must sum to 1, they sum to {total}"),
        ));
    }
    Ok(())
}

/// Analyze an outcome distribution under the requested measure, with ruin read as
/// "the worst outcome happens" and no reversibility judgment.
pub fn analyze_risk(outcomes: &[LossOutcome], measure: RiskMeasure) -> RiskProfile {
    analyze_risk_with(outcomes, measure, &RiskOptions::worst_case(outcomes))
}

/// Analyze an outcome distribution under the requested measure and options.
///
/// The caller is expected to have called [`validate_outcomes`] and
/// [`RiskOptions::validate`] first; this function computes over whatever it is
/// given rather than filtering, because silently dropping a malformed outcome
/// would produce a plausible number for a distribution nobody declared.
pub fn analyze_risk_with(
    outcomes: &[LossOutcome],
    measure: RiskMeasure,
    options: &RiskOptions,
) -> RiskProfile {
    if outcomes.is_empty() {
        return RiskProfile::empty(measure, options);
    }

    let expected_loss: f64 = outcomes
        .iter()
        .map(|outcome| outcome.probability * outcome.loss)
        .sum();
    let max_loss = outcomes
        .iter()
        .map(|outcome| outcome.loss)
        .fold(f64::NEG_INFINITY, f64::max);
    let variance: f64 = outcomes
        .iter()
        .map(|outcome| outcome.probability * (outcome.loss - expected_loss).powi(2))
        .sum();

    // Downside minus upside, both as semi-deviations from the expectation.
    //
    // The phase plan writes this as `E[max(0,-r)] - E[max(0,r)]` over returns.
    // Taken *linearly* that expression is identically the expectation, because
    // `max(0,x) - max(0,-x) = x`: it would report a field the profile already has
    // and nothing about the shape. The squared form below is the same one-sided
    // comparison with the magnitudes kept, so it is non-zero exactly when one tail
    // is broader, which is the property a firewall escalates on.
    let downside = (outcomes
        .iter()
        .map(|outcome| outcome.probability * (outcome.loss - expected_loss).max(0.0).powi(2))
        .sum::<f64>())
    .sqrt();
    let upside = (outcomes
        .iter()
        .map(|outcome| outcome.probability * (expected_loss - outcome.loss).max(0.0).powi(2))
        .sum::<f64>())
    .sqrt();
    let downside_asymmetry = downside - upside;

    // The alpha quantile of the loss distribution, measured from the bottom: the
    // loss at which the cumulative probability from the best case first reaches
    // `alpha`. At `alpha = 1` that is the worst case, which is what makes it the
    // value-at-risk an operator expects to read. A measure with no tail level of
    // its own (`variance`) reads the field at level 1, so `tail_probability` then
    // means "the probability of the worst outcome" rather than being left at zero
    // for a distribution that plainly has a tail.
    let level = measure.level().unwrap_or(1.0);
    let mut ordered: Vec<(f64, f64)> = outcomes
        .iter()
        .map(|outcome| (outcome.loss, outcome.probability))
        .collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut cumulative = 0.0;
    let mut var_alpha = ordered.last().map(|entry| entry.0).unwrap_or(0.0);
    for (loss, probability) in &ordered {
        cumulative += probability;
        if cumulative >= level {
            var_alpha = *loss;
            break;
        }
    }
    let tail_probability = outcomes
        .iter()
        .filter(|outcome| outcome.loss.total_cmp(&var_alpha) != std::cmp::Ordering::Less)
        .map(|outcome| outcome.probability)
        .sum::<f64>();

    let ruin_probability = outcomes
        .iter()
        .filter(|outcome| outcome.loss.total_cmp(&options.capital) != std::cmp::Ordering::Less)
        .map(|outcome| outcome.probability)
        .sum::<f64>();

    RiskProfile {
        expected_loss,
        max_loss,
        tail_probability,
        ruin_probability,
        variance,
        downside_asymmetry,
        reversibility: options.reversibility,
        optionality: options.optionality,
        measure,
        measure_value: engine_value(outcomes, measure),
    }
}

/// The requested measure's loss-space value, computed by the vendored engine.
///
/// The engine accumulates probability **from the worst utility upward** and stops
/// when the requested mass is filled. A loss-space level `alpha` measured from the
/// bottom is the same point as the engine's `1 - alpha` measured from the top, so
/// that reflection is applied here and nowhere else. In loss space the sign also
/// flips: the engine reports a *utility*, and a low utility is a large loss.
///
/// The engine's tail expectation apportions the outcome that straddles the
/// boundary by its remaining mass, while [`RiskProfile::tail_probability`]
/// conditions on the discrete tail event. Both are standard estimators; they agree
/// exactly when the boundary falls between outcomes, which the reference corpus is
/// built so that it does.
fn engine_value(outcomes: &[LossOutcome], measure: RiskMeasure) -> f64 {
    use decision_ir::{Action, DecisionEngine, Outcome, RiskMeasure as EngineMeasure};

    let action = Action {
        name: "risk".to_string(),
        outcomes: outcomes
            .iter()
            .map(|outcome| Outcome {
                name: outcome.name.clone(),
                probability: outcome.probability,
                utility: -outcome.loss,
            })
            .collect(),
        cost: 0.0,
    };

    match measure {
        RiskMeasure::Variance => {
            // Utilities are negated losses, so the utility variance *is* the loss
            // variance: the twice-flipped sign cancels. Cross-checked against
            // `RiskProfile::variance` in this module's tests.
            DecisionEngine::risk(&action, EngineMeasure::Variance)
        }
        RiskMeasure::Var { alpha } => {
            let alpha = alpha.clamp(0.0, 1.0);
            -DecisionEngine::risk(&action, EngineMeasure::Var { alpha: 1.0 - alpha })
        }
        RiskMeasure::Cvar { alpha } => {
            let alpha = alpha.clamp(0.0, 1.0);
            -DecisionEngine::risk(&action, EngineMeasure::Cvar { alpha: 1.0 - alpha })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distribution() -> Vec<LossOutcome> {
        vec![
            LossOutcome {
                name: "total loss".into(),
                probability: 0.1,
                loss: 100.0,
            },
            LossOutcome {
                name: "partial loss".into(),
                probability: 0.3,
                loss: 20.0,
            },
            LossOutcome {
                name: "no loss".into(),
                probability: 0.6,
                loss: 0.0,
            },
        ]
    }

    #[test]
    fn every_measure_matches_hand_computation() {
        let options = RiskOptions {
            capital: 50.0,
            reversibility: 0.25,
            optionality: 0.75,
        };
        let profile = analyze_risk_with(&distribution(), RiskMeasure::Variance, &options);

        // 0.1*100 + 0.3*20 + 0.6*0
        assert!((profile.expected_loss - 16.0).abs() < 1e-9, "{profile:?}");
        assert!((profile.max_loss - 100.0).abs() < 1e-9);
        assert!((profile.ruin_probability - 0.1).abs() < 1e-9);
        assert!((profile.reversibility - 0.25).abs() < 1e-9);
        assert!((profile.optionality - 0.75).abs() < 1e-9);

        // 0.1*(100-16)^2 + 0.3*(20-16)^2 + 0.6*(0-16)^2
        let expected_variance =
            0.1 * 84.0_f64.powi(2) + 0.3 * 4.0_f64.powi(2) + 0.6 * 16.0_f64.powi(2);
        assert!(
            (profile.variance - expected_variance).abs() < 1e-9,
            "{} vs {expected_variance}",
            profile.variance
        );
        assert!(
            (profile.measure_value - expected_variance).abs() < 1e-6,
            "the engine's variance must agree with ours: {}",
            profile.measure_value
        );

        // Downside semi-deviation minus upside semi-deviation. Here the down tail
        // is the heavy one, so the sign is positive.
        let downside = (0.1 * 84.0_f64.powi(2) + 0.3 * 4.0_f64.powi(2)).sqrt();
        let upside = (0.6 * 16.0_f64.powi(2)).sqrt();
        assert!(
            (profile.downside_asymmetry - (downside - upside)).abs() < 1e-9,
            "{:?}",
            profile.downside_asymmetry
        );
        assert!(profile.downside_asymmetry > 0.0);
    }

    #[test]
    fn the_tail_probability_is_read_at_the_requested_level() {
        // Bottom cumulative: 0.6 at 0, 0.9 at 20, 1.0 at 100.
        let profile = analyze_risk(&distribution(), RiskMeasure::Var { alpha: 0.8 });
        // The level is reached at 20, and 0.3 + 0.1 of the mass is at or above it.
        assert!((profile.tail_probability - 0.4).abs() < 1e-9);
        assert!(
            (profile.measure_value - 20.0).abs() < 1e-9,
            "{}",
            profile.measure_value
        );

        // A level inside the first bucket reads at the lowest loss.
        let low = analyze_risk(&distribution(), RiskMeasure::Var { alpha: 0.5 });
        assert!((low.tail_probability - 1.0).abs() < 1e-9);
        assert!((low.measure_value - 0.0).abs() < 1e-9);

        // A measure with no level of its own reads the field at the worst case,
        // so `tail_probability` is never left at zero for a distribution with a
        // tail.
        let variance = analyze_risk(&distribution(), RiskMeasure::Variance);
        assert!((variance.tail_probability - 0.1).abs() < 1e-9);
    }

    #[test]
    fn alpha_one_is_the_worst_case_and_alpha_zero_the_best() {
        let worst = analyze_risk(&distribution(), RiskMeasure::Var { alpha: 1.0 });
        assert!(
            (worst.measure_value - 100.0).abs() < 1e-9,
            "{}",
            worst.measure_value
        );
        assert!((worst.tail_probability - 0.1).abs() < 1e-9);

        let best = analyze_risk(&distribution(), RiskMeasure::Var { alpha: 0.0 });
        assert!(
            (best.measure_value - 0.0).abs() < 1e-9,
            "{}",
            best.measure_value
        );
    }

    #[test]
    fn the_conditional_value_at_risk_is_at_least_the_value_at_risk() {
        for alpha in [0.2_f64, 0.5, 0.8, 0.95] {
            let var = analyze_risk(&distribution(), RiskMeasure::Var { alpha });
            let cvar = analyze_risk(&distribution(), RiskMeasure::Cvar { alpha });
            assert!(
                cvar.measure_value >= var.measure_value - 1e-9,
                "cvar {} < var {} at alpha {alpha}",
                cvar.measure_value,
                var.measure_value
            );
            assert!(cvar.measure_value <= 100.0 + 1e-9);
        }
    }

    #[test]
    fn the_asymmetry_sign_tracks_which_tail_is_heavier() {
        let downside_heavy = vec![
            LossOutcome {
                name: "catastrophe".into(),
                probability: 0.1,
                loss: 100.0,
            },
            LossOutcome {
                name: "calm".into(),
                probability: 0.9,
                loss: 0.0,
            },
        ];
        let upside_heavy = vec![
            LossOutcome {
                name: "calm".into(),
                probability: 0.9,
                loss: 100.0,
            },
            LossOutcome {
                name: "rescue".into(),
                probability: 0.1,
                loss: 0.0,
            },
        ];
        let down = analyze_risk(&downside_heavy, RiskMeasure::Variance);
        let up = analyze_risk(&upside_heavy, RiskMeasure::Variance);
        assert!(down.downside_asymmetry > 0.0, "{down:?}");
        assert!(up.downside_asymmetry < 0.0, "{up:?}");
        assert!(
            (down.downside_asymmetry + up.downside_asymmetry).abs() < 1e-9,
            "the two mirror distributions must mirror"
        );
    }

    #[test]
    fn a_lost_probability_mass_is_refused() {
        let mut broken = distribution();
        broken[0].probability = 0.1;
        broken[2].probability = 0.5;
        let err = validate_outcomes(&broken).unwrap_err();
        assert_eq!(err.code(), "validation");
        assert!(err.to_string().contains("sum to 1"), "{err}");

        assert!(validate_outcomes(&[]).is_err());
    }

    #[test]
    fn an_empty_distribution_analyzes_to_a_zeroed_profile() {
        let profile = analyze_risk(&[], RiskMeasure::Variance);
        assert!((profile.expected_loss).abs() < 1e-12);
        assert!((profile.max_loss).abs() < 1e-12);
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn a_measure_name_round_trips_and_unknown_names_are_refused() {
        assert_eq!(
            RiskMeasure::parse("variance", 0.9),
            Some(RiskMeasure::Variance)
        );
        assert_eq!(
            RiskMeasure::parse("VAR", 0.9),
            Some(RiskMeasure::Var { alpha: 0.9 })
        );
        assert_eq!(
            RiskMeasure::parse("cvar", 0.5),
            Some(RiskMeasure::Cvar { alpha: 0.5 })
        );
        assert_eq!(RiskMeasure::parse("ruin", 0.5), None);
        for name in RISK_MEASURE_NAMES {
            assert!(RiskMeasure::parse(name, 0.9).is_some());
        }
    }

    #[test]
    fn a_profile_reports_its_measure_and_renders_deterministically() {
        let profile = analyze_risk(&distribution(), RiskMeasure::Cvar { alpha: 0.9 });
        assert_eq!(profile.measure, RiskMeasure::Cvar { alpha: 0.9 });
        assert_eq!(profile.canonical(), profile.canonical());
        assert!(profile.validate().is_ok());
        assert!(profile.canonical().contains("measure=cvar@0.9"));
    }

    #[test]
    fn outcomes_that_are_not_a_distribution_are_refused_field_by_field() {
        let bad_probability = vec![LossOutcome {
            name: "only".into(),
            probability: 1.5,
            loss: 1.0,
        }];
        assert!(validate_outcomes(&bad_probability).is_err());

        let bad_loss = vec![LossOutcome {
            name: "only".into(),
            probability: 1.0,
            loss: f64::NAN,
        }];
        assert!(validate_outcomes(&bad_loss).is_err());

        let unnamed = vec![LossOutcome {
            name: "  ".into(),
            probability: 1.0,
            loss: 1.0,
        }];
        assert!(validate_outcomes(&unnamed).is_err());
    }

    #[test]
    fn options_are_validated_in_range() {
        assert!(RiskOptions {
            capital: 1.0,
            reversibility: 0.5,
            optionality: 0.5,
        }
        .validate()
        .is_ok());
        assert!(RiskOptions {
            capital: 1.0,
            reversibility: 1.5,
            optionality: 0.5,
        }
        .validate()
        .is_err());
        assert!(RiskOptions {
            capital: f64::INFINITY,
            reversibility: 0.0,
            optionality: 0.0,
        }
        .validate()
        .is_err());

        let options = RiskOptions::worst_case(&distribution());
        assert!((options.capital - 100.0).abs() < 1e-9);
    }
}
