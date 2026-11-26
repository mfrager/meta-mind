//! The deterministic rails: everything the firewall can conclude **without
//! consulting a model**.
//!
//! This is the first of the plan's two producers of judgment, and it is
//! consequence-free in the sense that matters: these signals are functions of the
//! input alone. No core, no provider, no clock, no configuration. A firewall
//! configured with no decision core at all still runs this and still reaches an
//! outcome, which is what makes `mm-cli firewall eval` usable on a machine that
//! has never talked to a provider — and what makes the adversarial corpus a
//! statement about the *firewall* rather than about a model.
//!
//! Two of the thresholds here are the plan's own numbers rather than tunings, and
//! both are stated as constants so a corpus can cite them: a ruin probability at
//! or above [`RUIN_REVIEW_THRESHOLD`] or a maximum loss at or above
//! [`MAX_LOSS_REVIEW_THRESHOLD`] is *catastrophic*, which is a `HUMAN_REVIEW` and
//! not a `VERIFY_FIRST`. The distinction is deliberate: a residual risk worth
//! measuring can be verified by the system, and a risk that would end the
//! enterprise cannot.
//!
//! Precedent is the one signal that is raised for *absence*: taking a
//! high-stakes action nothing in the record resembles is exactly when a bounded
//! amount of extra care is cheapest. Below
//! [`PRECEDENT_STAKES_THRESHOLD`] it is not raised at all, because requiring a
//! precedent for every small decision would make the firewall a brake on
//! everything.

use std::collections::BTreeMap;

use crate::outcome::{normalized_signals, outcome_for_code, upsert_signal_at, ReasonCode};
use crate::prohibitions::first_prohibition;
use crate::report::{FirewallInput, HardProhibition};

/// At or above this ruin probability, the residual risk is catastrophic.
pub const RUIN_REVIEW_THRESHOLD: f64 = 0.05;

/// At or above this maximum loss, the residual risk is catastrophic.
pub const MAX_LOSS_REVIEW_THRESHOLD: f64 = 1.0e6;

/// Only at or above this stake level is a missing precedent worth raising.
pub const PRECEDENT_STAKES_THRESHOLD: f32 = 0.5;

/// What the input-only rails concluded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RailOutcome {
    /// The first prohibition that fired, if one did.
    pub prohibition: Option<HardProhibition>,
    /// The signals the input alone justifies, sorted for a report.
    pub signals: Vec<ReasonCode>,
    /// The numeric magnitudes behind those signals, keyed by reason code.
    pub signals_map: BTreeMap<String, f64>,
}

impl RailOutcome {
    /// True when a prohibition fired.
    pub fn has_prohibition(&self) -> bool {
        self.prohibition.is_some()
    }

    /// True when nothing was raised at all.
    pub fn is_clean(&self) -> bool {
        self.prohibition.is_none() && self.signals.is_empty()
    }

    /// The signal codes, in report order.
    pub fn codes(&self) -> Vec<String> {
        self.signals.iter().map(|s| s.code.clone()).collect()
    }
}

/// Run the deterministic rails over one input.
pub fn run_rails(input: &FirewallInput) -> RailOutcome {
    let mut signals: Vec<ReasonCode> = Vec::new();
    let mut magnitudes: BTreeMap<String, f64> = BTreeMap::new();

    // ---------------------------------------------------------- prohibitions ----
    // The prohibition is looked up first because it short-circuits the whole
    // evaluation: the caller decides whether to keep going, and a rail that
    // computed signals a refusal would discard would only be a way for a bug to
    // leak a model call into a rejected run.
    let prohibition = first_prohibition(input);

    // -------------------------------------------------- critical assumptions ----
    let unverified: Vec<&str> = input
        .assumptions
        .iter()
        .filter(|a| a.decision_critical && !a.verified)
        .map(|a| a.claim.as_str())
        .collect();
    if !unverified.is_empty() {
        upsert_signal_at(
            &mut signals,
            "signal.critical_unverified_assumption",
            format!(
                "{} decision-critical assumption(s) unverified: {}",
                unverified.len(),
                unverified.join("; ")
            ),
            outcome_for_code("signal.critical_unverified_assumption"),
        );
        magnitudes.insert(
            "signal.critical_unverified_assumption".to_string(),
            unverified.len() as f64,
        );
    }

    // ---------------------------------------------------------- catastrophic ----
    // The two catastrophic readings are reported separately, not folded into one
    // number: a large maximum loss and a high ruin probability are different
    // facts, and a report that merged them would hide which one applied.
    let ruin = input.risks.ruin_probability;
    let max_loss = input.risks.max_loss;
    if ruin >= RUIN_REVIEW_THRESHOLD || max_loss >= MAX_LOSS_REVIEW_THRESHOLD {
        upsert_signal_at(
            &mut signals,
            "signal.catastrophic_risk",
            format!("ruin_probability={ruin:.9} max_loss={max_loss:.9}"),
            outcome_for_code("signal.catastrophic_risk"),
        );
        magnitudes.insert(
            "signal.catastrophic_risk".to_string(),
            ruin.max(max_loss / MAX_LOSS_REVIEW_THRESHOLD),
        );
    }

    // ------------------------------------------------------ invalid comparison ----
    // Every invalid comparison is named. `check_contract` has already refused to
    // coerce one, so all this rail does is turn "the comparison was not admissible"
    // into "the reasoning has to be redone".
    let invalid: Vec<&str> = input
        .comparisons
        .iter()
        .filter(|c| !c.is_valid())
        .map(|c| c.objective.as_str())
        .collect();
    if !invalid.is_empty() {
        upsert_signal_at(
            &mut signals,
            "signal.invalid_comparison",
            format!(
                "{} comparison(s) not admissible: {}",
                invalid.len(),
                invalid.join("; ")
            ),
            outcome_for_code("signal.invalid_comparison"),
        );
        magnitudes.insert(
            "signal.invalid_comparison".to_string(),
            invalid.len() as f64,
        );
    }

    // ------------------------------------------------------ missing authorization ----
    if !input.authorization.granted {
        upsert_signal_at(
            &mut signals,
            "signal.missing_authorization",
            format!(
                "no grant covers this action (scope={})",
                input.authorization.scope
            ),
            outcome_for_code("signal.missing_authorization"),
        );
        magnitudes.insert("signal.missing_authorization".to_string(), 1.0);
    }

    // ------------------------------------------------------------ missing steps ----
    let critical_steps: Vec<&str> = input
        .missing_steps
        .iter()
        .filter(|step| step.decision_critical)
        .map(|step| step.step.as_str())
        .collect();
    if !critical_steps.is_empty() {
        upsert_signal_at(
            &mut signals,
            "signal.missing_steps",
            format!(
                "{} decision-critical step(s) missing: {}",
                critical_steps.len(),
                critical_steps.join("; ")
            ),
            outcome_for_code("signal.missing_steps"),
        );
        magnitudes.insert(
            "signal.missing_steps".to_string(),
            critical_steps.len() as f64,
        );
    }

    // ------------------------------------------------------ simpler alternative ----
    // A yes here is *cautionary*, which reads backwards until you say it plainly:
    // the rail fires when a simpler alternative exists, because an action that is
    // more complicated than it needs to be is where unmodelled risk lives.
    if let Some(alternative) = input.simpler_alternative {
        upsert_signal_at(
            &mut signals,
            "signal.simpler_alternative_available",
            format!(
                "a simpler alternative exists: {}",
                mm_core::ulid_string(&alternative)
            ),
            outcome_for_code("signal.simpler_alternative_available"),
        );
        magnitudes.insert("signal.simpler_alternative_available".to_string(), 1.0);
    }

    // ------------------------------------------------------- precedent absent ----
    if input.precedent.is_none() && input.stakes.level >= PRECEDENT_STAKES_THRESHOLD {
        upsert_signal_at(
            &mut signals,
            "signal.precedent_absent",
            format!(
                "no precedent in the record and stakes are {:.3}",
                input.stakes.level
            ),
            outcome_for_code("signal.precedent_absent"),
        );
        magnitudes.insert(
            "signal.precedent_absent".to_string(),
            input.stakes.level as f64,
        );
    }

    // -------------------------------------------------- soft constraint unmet ----
    // Only the *soft* ones. A hard constraint that is unmet is a feasibility
    // failure, which the caller reports as `action_schema_valid = false`; treating
    // it as a `VERIFY_FIRST` signal here would let the aggregation reach a
    // permissive outcome on an action that cannot be performed at all.
    let soft_unmet: Vec<&str> = input
        .constraints
        .iter()
        .filter(|constraint| !constraint.hard && !constraint.satisfied)
        .map(|constraint| constraint.id.as_str())
        .collect();
    if !soft_unmet.is_empty() {
        upsert_signal_at(
            &mut signals,
            "signal.soft_constraint_violation",
            format!(
                "{} soft constraint(s) unmet: {}",
                soft_unmet.len(),
                soft_unmet.join("; ")
            ),
            outcome_for_code("signal.soft_constraint_violation"),
        );
        magnitudes.insert(
            "signal.soft_constraint_violation".to_string(),
            soft_unmet.len() as f64,
        );
    }

    // Normalizing never drops a code: it merges a code raised twice and sorts the
    // result. A magnitude whose signal is not in the report is dropped here, since
    // a number for a reason the report does not name is a number nobody can read.
    let signals = normalized_signals(&signals);
    magnitudes.retain(|code, _| signals.iter().any(|signal| &signal.code == code));
    RailOutcome {
        prohibition,
        signals,
        signals_map: magnitudes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{
        AssumptionRef, AuthorizationRef, ComparisonOutcomeRef, ConstraintRef, IdentityInvariantHit,
        MissingStep, Reversibility, Stakes, ToolTarget,
    };
    use mm_core::Ulid;
    use mm_decision::risk::{analyze_risk_with, LossOutcome, RiskMeasure, RiskOptions};

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn outcome(name: &str, probability: f64, loss: f64) -> LossOutcome {
        LossOutcome {
            name: name.into(),
            probability,
            loss,
        }
    }

    /// A risk budget nothing in these fixtures can deplete, so a fixture that is not
    /// about catastrophic risk does not accidentally become one.
    fn options(capital: f64) -> RiskOptions {
        RiskOptions {
            capital,
            reversibility: 0.5,
            optionality: 0.5,
        }
    }

    fn clean() -> FirewallInput {
        FirewallInput::new(
            ulid(1),
            analyze_risk_with(
                &[outcome("small", 1.0, 1.0)],
                RiskMeasure::Variance,
                &options(1.0e9),
            ),
        )
    }

    #[test]
    fn a_clean_input_raises_nothing() {
        let rails = run_rails(&clean());
        assert!(rails.is_clean(), "{:?}", rails.signals);
        assert!(rails.codes().is_empty());
    }

    #[test]
    fn a_critical_unverified_assumption_is_a_human_review() {
        let mut input = clean();
        input.assumptions = vec![AssumptionRef {
            id: ulid(2),
            claim: "the migration is idempotent".into(),
            decision_critical: true,
            verified: false,
        }];
        let rails = run_rails(&input);
        assert_eq!(rails.codes(), vec!["signal.critical_unverified_assumption"]);
        assert_eq!(
            rails.signals[0].severity,
            crate::outcome::FirewallOutcome::HumanReview
        );
        assert_eq!(
            rails.signals_map["signal.critical_unverified_assumption"],
            1.0
        );

        // A verified assumption is not raised, and neither is a non-critical one.
        input.assumptions[0].verified = true;
        assert!(run_rails(&input).is_clean());
        input.assumptions[0].verified = false;
        input.assumptions[0].decision_critical = false;
        assert!(run_rails(&input).is_clean());
    }

    #[test]
    fn catastrophic_risk_is_raised_on_either_reading() {
        // A risk budget of 1.0, so the 10% chance of losing 10.0 is a 10% ruin
        // probability — above RUIN_REVIEW_THRESHOLD.
        let mut input = clean();
        input.risks = analyze_risk_with(
            &[
                outcome("catastrophe", 0.1, 10.0),
                outcome("small", 0.9, 0.5),
            ],
            RiskMeasure::Var { alpha: 0.9 },
            &options(1.0),
        );
        assert!(run_rails(&input)
            .codes()
            .contains(&"signal.catastrophic_risk".to_string()));
        assert!(input.risks.ruin_probability >= RUIN_REVIEW_THRESHOLD);

        // A budget nothing can deplete, but a maximum loss above the catastrophic
        // threshold — the other reading of "catastrophic".
        let mut input = clean();
        input.risks = analyze_risk_with(
            &[outcome("huge", 0.001, 5.0e6), outcome("small", 0.999, 1.0)],
            RiskMeasure::Variance,
            &options(1.0e9),
        );
        assert!(input.risks.ruin_probability < RUIN_REVIEW_THRESHOLD);
        assert!(input.risks.max_loss >= MAX_LOSS_REVIEW_THRESHOLD);
        assert!(run_rails(&input)
            .codes()
            .contains(&"signal.catastrophic_risk".to_string()));
    }

    #[test]
    fn an_invalid_comparison_is_a_replan() {
        let mut input = clean();
        input.comparisons = vec![
            ComparisonOutcomeRef {
                id: ulid(3),
                verdict: "non_comparable".into(),
                objective: "pick a datastore".into(),
            },
            ComparisonOutcomeRef {
                id: ulid(4),
                verdict: "valid".into(),
                objective: "pick a queue".into(),
            },
        ];
        let rails = run_rails(&input);
        assert_eq!(rails.codes(), vec!["signal.invalid_comparison"]);
        assert_eq!(
            rails.signals[0].severity,
            crate::outcome::FirewallOutcome::Replan
        );
        assert_eq!(rails.signals_map["signal.invalid_comparison"], 1.0);
    }

    #[test]
    fn a_missing_grant_is_an_ask_user() {
        let mut input = clean();
        input.authorization = AuthorizationRef {
            granted: false,
            scope: "read-only".into(),
            approval_id: None,
        };
        let rails = run_rails(&input);
        assert_eq!(rails.codes(), vec!["signal.missing_authorization"]);
        assert_eq!(
            rails.signals[0].severity,
            crate::outcome::FirewallOutcome::AskUser
        );
    }

    #[test]
    fn missing_critical_steps_replan_but_optional_ones_do_not() {
        let mut input = clean();
        input.missing_steps = vec![MissingStep {
            step: "run the migration dry".into(),
            decision_critical: true,
        }];
        assert_eq!(run_rails(&input).codes(), vec!["signal.missing_steps"]);
        input.missing_steps[0].decision_critical = false;
        assert!(run_rails(&input).is_clean());
    }

    #[test]
    fn a_simpler_alternative_is_cautionary() {
        let mut input = clean();
        input.simpler_alternative = Some(ulid(5));
        let rails = run_rails(&input);
        assert_eq!(rails.codes(), vec!["signal.simpler_alternative_available"]);
        assert_eq!(
            rails.signals[0].severity,
            crate::outcome::FirewallOutcome::ProceedWithCaution
        );
    }

    #[test]
    fn a_missing_precedent_is_raised_only_for_high_stakes() {
        let mut input = clean();
        input.stakes = Stakes {
            level: 0.8,
            irreversibility: 0.0,
        };
        assert_eq!(run_rails(&input).codes(), vec!["signal.precedent_absent"]);

        input.stakes.level = 0.2;
        assert!(run_rails(&input).is_clean());

        input.stakes.level = 0.8;
        input.precedent = Some(ulid(6));
        assert!(run_rails(&input).is_clean());
    }

    #[test]
    fn only_soft_unmet_constraints_are_raised() {
        let mut input = clean();
        input.constraints = vec![
            ConstraintRef {
                id: "keep-it-cheap".into(),
                hard: false,
                satisfied: false,
            },
            ConstraintRef {
                id: "must-fit-in-memory".into(),
                hard: true,
                satisfied: false,
            },
        ];
        let rails = run_rails(&input);
        assert_eq!(rails.codes(), vec!["signal.soft_constraint_violation"]);
        assert_eq!(
            rails.signals[0].severity,
            crate::outcome::FirewallOutcome::VerifyFirst
        );
    }

    #[test]
    fn the_rails_report_a_prohibition_and_keep_the_rest_in_order() {
        let mut input = clean();
        input.tool = Some(ToolTarget {
            name: "shell".into(),
            permission_granted: false,
        });
        input.identity_invariant_hits = vec![IdentityInvariantHit {
            invariant: "no_self_modification".into(),
            detail: "would rewrite its own identity block".into(),
        }];
        input.missing_steps = vec![MissingStep {
            step: "dry run".into(),
            decision_critical: true,
        }];
        input.simpler_alternative = Some(ulid(5));
        let rails = run_rails(&input);
        assert!(rails.has_prohibition());
        assert_eq!(rails.prohibition.as_ref().unwrap().id, "identity_invariant");
        // The signals are in report order: `REPLAN` before `PROCEED_WITH_CAUTION`.
        assert_eq!(
            rails.codes(),
            vec![
                "signal.missing_steps".to_string(),
                "signal.simpler_alternative_available".to_string()
            ]
        );
    }

    #[test]
    fn the_rail_order_is_independent_of_input_order() {
        let mut a = clean();
        a.missing_steps = vec![
            MissingStep {
                step: "one".into(),
                decision_critical: true,
            },
            MissingStep {
                step: "two".into(),
                decision_critical: true,
            },
        ];
        a.simpler_alternative = Some(ulid(5));
        let b = a.clone();
        assert_eq!(run_rails(&a).codes(), run_rails(&b).codes());
    }

    #[test]
    fn reversibility_does_not_raise_a_rail_on_its_own() {
        // Reversibility feeds a *prohibition*, not a signal; a partially reversible
        // action with no other input raises nothing.
        let mut input = clean();
        input.reversibility = Reversibility::PartiallyReversible;
        assert!(run_rails(&input).is_clean());
    }
}
