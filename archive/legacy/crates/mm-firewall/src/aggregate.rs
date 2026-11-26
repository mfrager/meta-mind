//! Aggregation: several judgments, one outcome.
//!
//! The plan's precedence, most cautious first:
//!
//! ```text
//! prohib > review > verify > ask > replan > caution > proceed
//! ```
//!
//! This module is where "model output can only ever make the firewall more
//! cautious" stops being a slogan. Everything the judgment layers can contribute
//! is a [`ReasonCode`] carrying a severity, and the outcome is the **maximum** of
//! those severities. A core cannot add a signal that lowers the outcome, because
//! there is no operation here that lowers anything: a signal can only raise.
//!
//! Two functions rather than one, because only the caller knows whether a
//! prohibition was evaluated. [`aggregate_with_prohibition`] takes the prohibition
//! explicitly and returns `Reject` unconditionally when one is present — the
//! parameter exists so that the invariant is stated in the signature and asserted
//! in a test, rather than being a fact about some field that a refactor could
//! quietly stop passing.
//!
//! An empty signal list is not an error and not a `Reject`: it means every layer
//! was satisfied, and the outcome is `Proceed` with one `signal.nominal` reason.
//! A report with no reasons at all would be a report nothing can explain, and
//! Phase 11's completeness check counts reason codes.

use crate::outcome::{
    normalized_signals, outcome_for_code, upsert_signal_at, FirewallOutcome, ReasonCode,
};
use crate::report::HardProhibition;

/// Aggregate a signal list into one outcome and the reasons behind it.
pub fn aggregate(signals: &[ReasonCode]) -> (FirewallOutcome, Vec<ReasonCode>) {
    let (outcome, reasons, _) = aggregate_with_prohibition(None, signals);
    (outcome, reasons)
}

/// Aggregate with an optional hard prohibition, which is `Reject` if present.
///
/// The `Option` is taken by value rather than by reference so the prohibition
/// travels with the result: a caller that has a prohibition must hand it over to
/// get a report, and cannot end up with a `Reject`-less report and a prohibition it
/// forgot to mention.
pub fn aggregate_with_prohibition(
    prohibition: Option<HardProhibition>,
    signals: &[ReasonCode],
) -> (FirewallOutcome, Vec<ReasonCode>, Option<HardProhibition>) {
    let mut reasons = normalized_signals(signals);

    if let Some(prohibition) = &prohibition {
        upsert_signal_at(
            &mut reasons,
            &prohibition.reason_code(),
            prohibition.reason.clone(),
            FirewallOutcome::Reject,
        );
    }

    let outcome = if prohibition.is_some() {
        // Stated as its own branch rather than left to the maximum below, so the
        // rule is visible where a reader looks for it. The two agree by
        // construction — a prohibition's reason code carries `Reject` — and both
        // being written down is the point.
        FirewallOutcome::Reject
    } else if reasons.is_empty() {
        upsert_signal_at(
            &mut reasons,
            "signal.nominal",
            "every deterministic rail and every bounded judgment was satisfied",
            outcome_for_code("signal.nominal"),
        );
        FirewallOutcome::Proceed
    } else {
        reasons
            .iter()
            .map(|reason| reason.severity)
            .max()
            .unwrap_or(FirewallOutcome::Proceed)
    };

    let reasons = normalized_signals(&reasons);
    // The outcome and the reason list cannot disagree: this says so out loud, so a
    // future branch that computes an outcome another way fails a test rather than
    // shipping a report that contradicts itself.
    debug_assert_eq!(
        outcome,
        reasons
            .iter()
            .map(|reason| reason.severity)
            .max()
            .unwrap_or(FirewallOutcome::Proceed),
        "the outcome must be the most cautious reason"
    );
    (outcome, reasons, prohibition)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(code: &str, severity: FirewallOutcome) -> ReasonCode {
        ReasonCode::new(code, code, "raised by a test", severity)
    }

    #[test]
    fn no_signals_proceeds_with_a_nominal_reason() {
        let (outcome, reasons, prohibition) = aggregate_with_prohibition(None, &[]);
        assert_eq!(outcome, FirewallOutcome::Proceed);
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0].code, "signal.nominal");
        assert!(prohibition.is_none());
    }

    #[test]
    fn the_outcome_is_the_most_cautious_signal() {
        let signals = vec![
            signal(
                "signal.simpler_alternative_available",
                FirewallOutcome::ProceedWithCaution,
            ),
            signal("signal.missing_steps", FirewallOutcome::Replan),
            signal("signal.high_uncertainty", FirewallOutcome::VerifyFirst),
        ];
        let (outcome, _) = aggregate(&signals);
        assert_eq!(outcome, FirewallOutcome::VerifyFirst);
    }

    #[test]
    fn a_prohibition_rejects_even_when_everything_else_proceeds() {
        // The whole point of the phase, as a test: a model said everything is fine.
        let signals = vec![
            signal("signal.nominal", FirewallOutcome::Proceed),
            signal("signal.residual_risk", FirewallOutcome::ProceedWithCaution),
        ];
        let prohibition = HardProhibition::new(
            "unauthorized_tool",
            "the action uses a tool with no permission grant",
            vec!["tool=shell".into()],
        );
        let (outcome, reasons, returned) = aggregate_with_prohibition(Some(prohibition), &signals);
        assert_eq!(outcome, FirewallOutcome::Reject);
        assert_eq!(returned.unwrap().id, "unauthorized_tool");
        assert!(reasons
            .iter()
            .any(|reason| reason.code == "prohibition.unauthorized_tool"));
        // The permissive reasons are still reported: the report says what a
        // model thought as well as what the prohibition decided.
        assert!(reasons
            .iter()
            .any(|reason| reason.code == "signal.residual_risk"));
    }

    #[test]
    fn a_prohibition_with_no_signals_still_rejects() {
        let prohibition = HardProhibition::new("schema_invalid_action", "bad shape", vec![]);
        let (outcome, reasons, _) = aggregate_with_prohibition(Some(prohibition), &[]);
        assert_eq!(outcome, FirewallOutcome::Reject);
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0].code, "prohibition.schema_invalid_action");
        assert_eq!(reasons[0].severity, FirewallOutcome::Reject);
    }

    #[test]
    fn aggregation_is_order_independent() {
        let a = vec![
            signal("signal.high_uncertainty", FirewallOutcome::VerifyFirst),
            signal("signal.missing_steps", FirewallOutcome::Replan),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(aggregate(&a), aggregate(&b));
        assert_eq!(aggregate(&a).1, aggregate(&b).1);
    }

    #[test]
    fn a_duplicate_code_is_one_reason() {
        let signals = vec![
            signal("signal.residual_risk", FirewallOutcome::ProceedWithCaution),
            signal("signal.residual_risk", FirewallOutcome::VerifyFirst),
        ];
        let (outcome, reasons) = aggregate(&signals);
        assert_eq!(reasons.len(), 1);
        assert_eq!(outcome, FirewallOutcome::VerifyFirst);
    }
}
