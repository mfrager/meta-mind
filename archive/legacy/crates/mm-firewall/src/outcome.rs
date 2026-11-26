//! The seven outcomes, the reason codes, and the precedence between them.
//!
//! The outcome enum's declaration order **is** the precedence order. That is not
//! tidiness: it means aggregation is a `max` over severities, and it means the
//! derived `Ord` cannot disagree with [`FirewallOutcome::severity`] — a type whose
//! `Ord` said one thing while the documented precedence said another is exactly
//! how a firewall silently becomes more permissive. The order is the plan's
//! precedence line read from the least to the most cautious end:
//!
//! ```text
//! prohib > review > verify > ask > replan > caution > proceed
//! ```
//!
//! Reason codes are constants in one array rather than string literals at the call
//! sites, for the same reason the log event codes are: a code is an API. Phase 11
//! calibrates on these strings and the gold corpus asserts on them, so a code that
//! changes must change in exactly one place, and a code that is *not* in
//! [`REASON_CODES`] cannot be ordered in a report.
//!
//! One judgment is worth stating because it is not visible in the shape of the
//! code: a signal is identified by its code, and [`upsert_signal`] keeps **one**
//! entry per code, at the highest severity it was raised at. Two copies of
//! `signal.high_uncertainty` — one from the safety judgment and one from the
//! residual-risk judgment — are one fact about the run, and a report that listed
//! them twice would make the count of reasons meaningless.

use serde::{Deserialize, Serialize};

/// The outcome of one firewall run.
///
/// Ordered by caution: `Proceed` is the least cautious and `Reject` the most. See
/// the module doc for why that ordering is load-bearing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FirewallOutcome {
    /// Nothing objected: the action may be taken as described.
    Proceed = 0,
    /// A bounded residual risk remains; proceed, but with the risk named.
    ProceedWithCaution = 1,
    /// The comparison, the plan, or a premise does not hold. Re-do the reasoning.
    Replan = 2,
    /// A judgment the caller has to supply is missing.
    AskUser = 3,
    /// Something must be checked against the world before acting.
    VerifyFirst = 4,
    /// A human must decide; the machine's judgment is not sufficient here.
    HumanReview = 5,
    /// A hard prohibition fired, or the residual risk is catastrophic.
    Reject = 6,
}

/// Every outcome, in precedence order (least cautious first).
pub const FIREWALL_OUTCOMES: [FirewallOutcome; 7] = [
    FirewallOutcome::Proceed,
    FirewallOutcome::ProceedWithCaution,
    FirewallOutcome::Replan,
    FirewallOutcome::AskUser,
    FirewallOutcome::VerifyFirst,
    FirewallOutcome::HumanReview,
    FirewallOutcome::Reject,
];

impl FirewallOutcome {
    /// The caution rank. Higher is more cautious; aggregation takes the maximum.
    pub fn severity(self) -> u8 {
        self as u8
    }

    /// The stable wire name, which is also the `firewall_runs.outcome` value and
    /// the `mm-cli firewall eval` report spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            FirewallOutcome::Proceed => "PROCEED",
            FirewallOutcome::ProceedWithCaution => "PROCEED_WITH_CAUTION",
            FirewallOutcome::Replan => "REPLAN",
            FirewallOutcome::AskUser => "ASK_USER",
            FirewallOutcome::VerifyFirst => "VERIFY_FIRST",
            FirewallOutcome::HumanReview => "HUMAN_REVIEW",
            FirewallOutcome::Reject => "REJECT",
        }
    }

    /// Parse a wire name, accepting either spelling the system writes.
    ///
    /// `serde` writes the screaming-snake-case spelling and the CLI report writes
    /// `as_str()`; both are the same name, so a corpus author who typed the
    /// lower-case form is not punished for it.
    pub fn parse(text: &str) -> Option<Self> {
        let upper = text.trim().to_ascii_uppercase();
        FIREWALL_OUTCOMES
            .into_iter()
            .find(|outcome| outcome.as_str() == upper)
    }

    /// True when the outcome does not require a human or a verification step.
    pub fn is_safe(self) -> bool {
        matches!(
            self,
            FirewallOutcome::Proceed | FirewallOutcome::ProceedWithCaution
        )
    }

    /// True when the outcome could only have been reached with a judgment from a
    /// decision core.
    pub fn requires_model_judgment(self) -> bool {
        matches!(
            self,
            FirewallOutcome::VerifyFirst | FirewallOutcome::HumanReview
        )
    }
}

impl std::fmt::Display for FirewallOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an outcome was chosen.
///
/// The code is a `String` rather than a `&'static str` because a report is
/// persisted, round-tripped through SQLite, and re-read by a later phase: a code
/// this build does not know about must survive the trip rather than fail to
/// deserialize. [`REASON_CODES`] is still the closed set the firewall itself
/// raises, and [`reason_code_rank`] orders whatever it is given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasonCode {
    /// The reason code, one of [`REASON_CODES`] for anything this crate raises.
    pub code: String,
    /// The signal that raised it, in the caller's words.
    pub signal: String,
    /// The evidence, rendered so a human can act on it.
    pub detail: String,
    /// How cautious this reason alone would make the outcome.
    pub severity: FirewallOutcome,
}

impl ReasonCode {
    /// A reason code.
    pub fn new(
        code: impl Into<String>,
        signal: impl Into<String>,
        detail: impl Into<String>,
        severity: FirewallOutcome,
    ) -> Self {
        ReasonCode {
            code: code.into(),
            signal: signal.into(),
            detail: detail.into(),
            severity,
        }
    }

    /// A stable rendering, so two runs' reasons compare as strings.
    pub fn canonical(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.code, self.signal, self.detail, self.severity
        )
    }
}

/// The reason codes this crate raises, in report order.
///
/// Prohibitions first — they are the hard, deterministic half of the firewall —
/// then the signals, most severe first, with `signal.nominal` last so that the
/// reason a clean run carries sorts after every reason a dirty one carries.
pub const REASON_CODES: [&str; 19] = [
    "prohibition.identity_invariant",
    "prohibition.unauthorized_tool",
    "prohibition.irreversible_without_approval",
    "prohibition.unresolved_hard_contradiction",
    "prohibition.spend_over_budget",
    "prohibition.schema_invalid_action",
    "signal.critical_unverified_assumption",
    "signal.catastrophic_risk",
    "signal.low_factuality",
    "signal.high_uncertainty",
    "signal.uncalibrated_confidence",
    "signal.invalid_comparison",
    "signal.missing_authorization",
    "signal.missing_steps",
    "signal.precedent_absent",
    "signal.simpler_alternative_available",
    "signal.soft_constraint_violation",
    "signal.residual_risk",
    "signal.nominal",
];

/// The position of a code in [`REASON_CODES`], or `usize::MAX` for a code this
/// build does not know.
///
/// An unknown code sorts last rather than first: a future phase's reason must not
/// jump the queue ahead of the reasons this build understands, and sorting must
/// remain total so a report is byte-stable.
pub fn reason_code_rank(code: &str) -> usize {
    REASON_CODES
        .iter()
        .position(|known| *known == code)
        .unwrap_or(usize::MAX)
}

/// The reason code a prohibition id maps to.
pub fn prohibition_reason_code(id: &str) -> String {
    format!("prohibition.{id}")
}

/// Add or raise a signal, keeping one entry per code.
///
/// A code already present is **raised**, never replaced with a weaker reason: the
/// entry keeps the highest severity seen and its detail names both readings, so
/// the report can still explain why the outcome is as cautious as it is.
pub fn upsert_signal(
    signals: &mut Vec<ReasonCode>,
    code: &str,
    detail: impl Into<String>,
) -> FirewallOutcome {
    upsert_signal_at(signals, code, detail, outcome_for_code(code))
}

/// Add or raise a signal at an explicit severity.
///
/// The severity is explicit wherever a signal's caution depends on a magnitude
/// rather than on its code — residual risk is `VERIFY_FIRST` above the verify
/// cutoff and `PROCEED_WITH_CAUTION` below it — so the mapping from code to a
/// default severity is only the fallback.
pub fn upsert_signal_at(
    signals: &mut Vec<ReasonCode>,
    code: &str,
    detail: impl Into<String>,
    severity: FirewallOutcome,
) -> FirewallOutcome {
    let detail = detail.into();
    if let Some(existing) = signals.iter_mut().find(|s| s.code == code) {
        if severity > existing.severity {
            existing.severity = severity;
        }
        if !existing.detail.contains(&detail) {
            existing.detail.push_str("; ");
            existing.detail.push_str(&detail);
        }
        return existing.severity;
    }
    signals.push(ReasonCode::new(code, code, detail, severity));
    severity
}

/// The severity a code implies when no magnitude overrides it.
///
/// Prohibitions are `Reject` because nothing else can be true once one fires.
/// Everything else is a judgment that something must be checked before acting,
/// except `signal.nominal`, which is the absence of a reason.
pub fn outcome_for_code(code: &str) -> FirewallOutcome {
    if code.starts_with("prohibition.") {
        return FirewallOutcome::Reject;
    }
    match code {
        // `nominal` is the code the aggregation writes when *nothing* objected, so
        // it is the one code whose severity is the least cautious outcome there is.
        // Mapping it to `ProceedWithCaution` — which an earlier version did, by
        // grouping it with `residual_risk` — would make a clean run report caution
        // for a reason that does not exist, and would make the aggregation's own
        // "outcome is the most cautious reason" assertion fire on every clean run.
        "signal.nominal" => FirewallOutcome::Proceed,
        "signal.residual_risk" => FirewallOutcome::ProceedWithCaution,
        "signal.simpler_alternative_available" | "signal.precedent_absent" => {
            FirewallOutcome::ProceedWithCaution
        }
        "signal.invalid_comparison" | "signal.missing_steps" => FirewallOutcome::Replan,
        "signal.missing_authorization" => FirewallOutcome::AskUser,
        "signal.critical_unverified_assumption" | "signal.catastrophic_risk" => {
            FirewallOutcome::HumanReview
        }
        _ => FirewallOutcome::VerifyFirst,
    }
}

/// Sort reasons for a report: by code rank, then code, then signal, then detail.
///
/// Total and deterministic, which is what makes two runs' reports comparable
/// byte-for-byte even when the signals arrived in a different order.
pub fn sort_signals(signals: &mut [ReasonCode]) {
    signals.sort_by(|a, b| {
        reason_code_rank(&a.code)
            .cmp(&reason_code_rank(&b.code))
            .then_with(|| a.code.cmp(&b.code))
            .then_with(|| a.signal.cmp(&b.signal))
            .then_with(|| a.detail.cmp(&b.detail))
    });
}

/// Every reason code, deduplicated and sorted for a report.
pub fn normalized_signals(signals: &[ReasonCode]) -> Vec<ReasonCode> {
    let mut out: Vec<ReasonCode> = Vec::new();
    for signal in signals {
        upsert_signal_at(
            &mut out,
            &signal.code,
            signal.detail.clone(),
            signal.severity,
        );
    }
    sort_signals(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_precedence_is_total_and_reject_wins() {
        let severities: Vec<u8> = FIREWALL_OUTCOMES.iter().map(|o| o.severity()).collect();
        let mut sorted = severities.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            severities.len(),
            "severities must be distinct"
        );
        assert_eq!(
            FIREWALL_OUTCOMES.iter().copied().max(),
            Some(FirewallOutcome::Reject),
            "the derived Ord must agree with the precedence"
        );
        assert!(FirewallOutcome::Reject.severity() > FirewallOutcome::HumanReview.severity());
        assert!(FirewallOutcome::HumanReview.severity() > FirewallOutcome::VerifyFirst.severity());
        assert!(FirewallOutcome::VerifyFirst.severity() > FirewallOutcome::AskUser.severity());
        assert!(FirewallOutcome::AskUser.severity() > FirewallOutcome::Replan.severity());
        assert!(
            FirewallOutcome::Replan.severity() > FirewallOutcome::ProceedWithCaution.severity()
        );
        assert!(
            FirewallOutcome::ProceedWithCaution.severity() > FirewallOutcome::Proceed.severity()
        );
    }

    #[test]
    fn every_outcome_round_trips_through_its_wire_name() {
        for outcome in FIREWALL_OUTCOMES {
            assert_eq!(FirewallOutcome::parse(outcome.as_str()), Some(outcome));
            assert_eq!(
                FirewallOutcome::parse(&outcome.as_str().to_ascii_lowercase()),
                Some(outcome)
            );
            let json = serde_json::to_string(&outcome).unwrap();
            assert_eq!(json, format!("\"{}\"", outcome.as_str()));
            let back: FirewallOutcome = serde_json::from_str(&json).unwrap();
            assert_eq!(back, outcome);
        }
        assert_eq!(FirewallOutcome::parse("maybe"), None);
    }

    #[test]
    fn reason_codes_are_unique_and_ordered() {
        let unique: std::collections::BTreeSet<&str> = REASON_CODES.into_iter().collect();
        assert_eq!(unique.len(), REASON_CODES.len());
        for (index, code) in REASON_CODES.iter().enumerate() {
            assert_eq!(reason_code_rank(code), index);
        }
        assert_eq!(reason_code_rank("signal.something.new"), usize::MAX);
        assert_eq!(reason_code_rank("signal.nominal"), REASON_CODES.len() - 1);
    }

    #[test]
    fn a_signal_is_raised_rather_than_duplicated() {
        let mut signals = Vec::new();
        upsert_signal_at(
            &mut signals,
            "signal.residual_risk",
            "bounded",
            FirewallOutcome::ProceedWithCaution,
        );
        let raised = upsert_signal_at(
            &mut signals,
            "signal.residual_risk",
            "above the verify cutoff",
            FirewallOutcome::VerifyFirst,
        );
        assert_eq!(signals.len(), 1, "one entry per code");
        assert_eq!(raised, FirewallOutcome::VerifyFirst);
        assert_eq!(signals[0].severity, FirewallOutcome::VerifyFirst);
        assert!(signals[0].detail.contains("bounded"));
        assert!(signals[0].detail.contains("above the verify cutoff"));
        // Raising again at a lower severity must not lower it.
        upsert_signal_at(
            &mut signals,
            "signal.residual_risk",
            "bounded",
            FirewallOutcome::Proceed,
        );
        assert_eq!(signals[0].severity, FirewallOutcome::VerifyFirst);
    }

    #[test]
    fn report_order_is_stable_across_arrival_order() {
        let a = vec![
            ReasonCode::new(
                "signal.high_uncertainty",
                "s",
                "d",
                FirewallOutcome::VerifyFirst,
            ),
            ReasonCode::new("signal.nominal", "s", "d", FirewallOutcome::Proceed),
        ];
        let b = vec![
            ReasonCode::new("signal.nominal", "s", "d", FirewallOutcome::Proceed),
            ReasonCode::new(
                "signal.high_uncertainty",
                "s",
                "d",
                FirewallOutcome::VerifyFirst,
            ),
        ];
        assert_eq!(normalized_signals(&a), normalized_signals(&b));
        assert_eq!(
            normalized_signals(&a)[0].code,
            "signal.high_uncertainty",
            "the more severe signal sorts by its rank, which precedes nominal"
        );
    }

    #[test]
    fn a_prohibition_code_implies_reject() {
        for code in REASON_CODES
            .iter()
            .filter(|c| c.starts_with("prohibition."))
        {
            assert_eq!(outcome_for_code(code), FirewallOutcome::Reject);
        }
        assert_eq!(
            prohibition_reason_code("identity_invariant"),
            "prohibition.identity_invariant"
        );
    }
}
