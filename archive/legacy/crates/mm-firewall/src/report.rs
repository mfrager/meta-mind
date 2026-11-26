//! The firewall's input and its report.
//!
//! The input is deliberately a *flat* record of what the caller already knows —
//! the assumptions in play, the contradictions found, the comparisons made, the
//! risk profile computed, the reversibility, the authorization, the precedent, the
//! stakes — rather than a handle onto the stores. Two reasons, and both matter:
//!
//! * **The firewall must not be able to go looking for a better input.** A
//!   component that can read the assumption ledger for itself can also read it
//!   *after* the assumptions changed, and then the run is no longer reproducible
//!   from the record it was given. Everything the firewall consulted is in the
//!   input, and the input's [`FirewallInput::digest`] is the identity of the run.
//! * **A digest is what makes a cached decision safe.** Phase 8 established the
//!   same rule for its semantic cache; the firewall reuses it. A decision cached
//!   against one input cannot be replayed against another, because the digest is
//!   part of the key.
//!
//! List order never changes the digest. Two callers that hand over the same
//! assumptions in a different order describe the same situation, and a record
//! whose identity depended on arrival order would make every replay a diff.
//!
//! The input is also serde-serializable in both directions on purpose: the `mm-cli
//! firewall eval` corpus is a JSONL file of inputs and expected outcomes, and the
//! adversarial corpus is only meaningful if the exact input that produced a
//! `REJECT` can be re-read from the file that claims it.

use std::collections::BTreeMap;

use mm_core::Ulid;
use mm_decision::risk::RiskProfile;
use serde::{Deserialize, Serialize};

use crate::error::{FirewallError, Result};
use crate::outcome::{sort_signals, FirewallOutcome, ReasonCode};

/// One assumption the action depends on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssumptionRef {
    /// The assumption's ULID in the epistemic ledger.
    pub id: Ulid,
    /// The assumption in words, so a report is readable without a second lookup.
    pub claim: String,
    /// True when the decision changes if the assumption is false.
    pub decision_critical: bool,
    /// True when the assumption has been checked against the world.
    pub verified: bool,
}

/// One contradiction involving a claim the action depends on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContradictionRef {
    /// The contradiction's ULID.
    pub id: Ulid,
    /// The claim the contradiction is about.
    pub claim: String,
    /// True when the decision changes if the contradiction is real.
    pub decision_critical: bool,
    /// True when the contradiction has been resolved.
    pub resolved: bool,
}

/// One constraint the action has to respect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintRef {
    /// The constraint's identifier.
    pub id: String,
    /// True when the constraint is a hard feasibility limit rather than a
    /// preference. A hard constraint that is unsatisfied is not a signal here — it
    /// is a feasibility failure the caller reports as
    /// [`FirewallInput::action_schema_valid`] being false — because a soft
    /// preference that is unmet and a limit that cannot be met call for different
    /// outcomes (`VERIFY_FIRST` versus `REJECT`).
    pub hard: bool,
    /// True when the constraint is currently satisfied.
    pub satisfied: bool,
}

/// A step the plan is missing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingStep {
    /// The step, in words.
    pub step: String,
    /// True when the plan does not work without it.
    pub decision_critical: bool,
}

/// What the caller will accept on the action's behalf.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationRef {
    /// True when a grant covers this action.
    pub granted: bool,
    /// What the grant is for, in words.
    pub scope: String,
    /// The approval record's ULID, when an approval exists.
    pub approval_id: Option<Ulid>,
}

/// How much is at stake.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stakes {
    /// How much the outcome matters, in `[0,1]`.
    pub level: f32,
    /// How hard the action is to undo, in `[0,1]`.
    pub irreversibility: f32,
}

impl Stakes {
    /// A stable rendering.
    pub fn canonical(&self) -> String {
        format!("{:.9},{:.9}", self.level, self.irreversibility)
    }
}

/// How hard an action is to undo. A three-valued judgment rather than a number,
/// because the prohibitions key on it: `Irreversible` is the value that requires an
/// approval, and a number with a threshold would make "is this reversible" a
/// question about the threshold rather than about the action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reversibility {
    /// The action can be undone completely.
    Reversible,
    /// The action can be undone at a cost.
    PartiallyReversible,
    /// The action cannot be undone.
    Irreversible,
}

impl Reversibility {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Reversibility::Reversible => "reversible",
            Reversibility::PartiallyReversible => "partially_reversible",
            Reversibility::Irreversible => "irreversible",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        let lower = text.trim().to_ascii_lowercase();
        [
            Reversibility::Reversible,
            Reversibility::PartiallyReversible,
            Reversibility::Irreversible,
        ]
        .into_iter()
        .find(|value| value.as_str() == lower)
    }
}

impl std::fmt::Display for Reversibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A comparison the caller made, with the verdict `mm-decision` gave it.
///
/// The verdict arrives as a string rather than as `mm_decision::compare::ComparisonVerdict`
/// because the *comparison* was checked before the firewall ran and what the
/// firewall needs is the answer, not the ability to re-check it. Carrying the
/// string also keeps a persisted firewall input readable after the compare types
/// change shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComparisonOutcomeRef {
    /// The comparison's ULID.
    pub id: Ulid,
    /// The verdict's wire name — `"valid"`, `"non_comparable"` or `"rejected"`.
    pub verdict: String,
    /// What was being compared.
    pub objective: String,
}

impl ComparisonOutcomeRef {
    /// True when the comparison was admitted.
    pub fn is_valid(&self) -> bool {
        self.verdict.eq_ignore_ascii_case("valid")
    }
}

/// A tool the action would use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolTarget {
    /// The tool's name.
    pub name: String,
    /// True when a permission grant covers this tool.
    pub permission_granted: bool,
}

/// Projected spend against the phase's budget.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spend {
    /// What the action is projected to cost.
    pub projected: f64,
    /// What is left to spend.
    pub budget: f64,
}

/// One Phase 4 identity invariant the action would violate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityInvariantHit {
    /// The invariant's name.
    pub invariant: String,
    /// How the action would violate it.
    pub detail: String,
}

/// Everything the firewall evaluates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FirewallInput {
    /// The episode the action belongs to.
    pub episode: Ulid,
    /// The assumptions in play.
    pub assumptions: Vec<AssumptionRef>,
    /// The contradictions found.
    pub contradictions: Vec<ContradictionRef>,
    /// The comparisons already checked.
    pub comparisons: Vec<ComparisonOutcomeRef>,
    /// The constraints in play.
    pub constraints: Vec<ConstraintRef>,
    /// The steps the plan is missing.
    pub missing_steps: Vec<MissingStep>,
    /// The deterministic risk profile of the action's outcomes.
    pub risks: RiskProfile,
    /// How hard the action is to undo.
    pub reversibility: Reversibility,
    /// What the caller will accept on the action's behalf.
    pub authorization: AuthorizationRef,
    /// A precedent the action follows, if there is one.
    pub precedent: Option<Ulid>,
    /// A simpler alternative that would do, if there is one.
    pub simpler_alternative: Option<Ulid>,
    /// How much is at stake.
    pub stakes: Stakes,
    /// Calibrated confidences or nonconformity pressures, each in `[0,1]`.
    pub uncertainty: Vec<f32>,
    /// Phase 4 identity invariants the action would violate.
    pub identity_invariant_hits: Vec<IdentityInvariantHit>,
    /// The tool the action would use, if any.
    pub tool: Option<ToolTarget>,
    /// The projected spend, when there is a budget to check it against.
    pub spend: Option<Spend>,
    /// True when the action satisfies its own declared schema.
    pub action_schema_valid: bool,
    /// The factuality check's support score, when one was run.
    pub factuality_support: Option<f32>,
}

impl FirewallInput {
    /// A blank input for one episode: nothing in play, reversible, unauthorized,
    /// nothing at stake, and every boolean field at its most permissive.
    ///
    /// A starting point for a fixture rather than a safe default: the caller is
    /// expected to overwrite every field it has an opinion about, and the fields it
    /// leaves are the ones it is asserting it has no evidence about.
    pub fn new(episode: Ulid, risks: RiskProfile) -> Self {
        FirewallInput {
            episode,
            assumptions: Vec::new(),
            contradictions: Vec::new(),
            comparisons: Vec::new(),
            constraints: Vec::new(),
            missing_steps: Vec::new(),
            risks,
            reversibility: Reversibility::Reversible,
            authorization: AuthorizationRef {
                granted: true,
                scope: "unspecified".into(),
                approval_id: None,
            },
            precedent: None,
            simpler_alternative: None,
            stakes: Stakes {
                level: 0.0,
                irreversibility: 0.0,
            },
            uncertainty: Vec::new(),
            identity_invariant_hits: Vec::new(),
            tool: None,
            spend: None,
            action_schema_valid: true,
            factuality_support: None,
        }
    }

    /// The canonical rendering the digest is taken over.
    ///
    /// Every list is sorted first. The situation a caller describes does not depend
    /// on the order the descriptions arrived in, so a digest that did would make
    /// every replay a false diff and every decision-cache hit conditional on a
    /// detail nobody controls.
    pub fn canonical(&self) -> String {
        let mut uncertainty: Vec<f32> = self.uncertainty.clone();
        uncertainty.sort_by(|a, b| a.total_cmp(b));
        let uncertainty: Vec<String> = uncertainty.iter().map(|v| format!("{v:.9}")).collect();
        let parts = [
            format!("episode={}", mm_core::ulid_string(&self.episode)),
            format!("assumptions={}", sorted_lines(&self.assumptions)),
            format!("contradictions={}", sorted_lines(&self.contradictions)),
            format!("comparisons={}", sorted_lines(&self.comparisons)),
            format!("constraints={}", sorted_lines(&self.constraints)),
            format!("missing_steps={}", sorted_lines(&self.missing_steps)),
            format!("risks={}", self.risks.canonical()),
            format!("reversibility={}", self.reversibility.as_str()),
            format!("authorization={}", line(&self.authorization)),
            format!(
                "precedent={}",
                self.precedent
                    .map_or_else(|| "-".to_string(), |id| mm_core::ulid_string(&id))
            ),
            format!(
                "simpler_alternative={}",
                self.simpler_alternative
                    .map_or_else(|| "-".to_string(), |id| mm_core::ulid_string(&id))
            ),
            format!("stakes={}", self.stakes.canonical()),
            format!("uncertainty={}", uncertainty.join(",")),
            format!(
                "identity_invariant_hits={}",
                sorted_lines(&self.identity_invariant_hits)
            ),
            format!(
                "tool={}",
                self.tool.as_ref().map_or_else(|| "-".to_string(), line)
            ),
            format!(
                "spend={}",
                self.spend.as_ref().map_or_else(|| "-".to_string(), line)
            ),
            format!("action_schema_valid={}", self.action_schema_valid),
            format!(
                "factuality_support={}",
                self.factuality_support
                    .map_or_else(|| "-".to_string(), |v| format!("{v:.9}"))
            ),
        ];
        parts.join("\n")
    }

    /// A digest of the input, stable across runs and independent of list order.
    ///
    /// This is the plan's `input_digest`, and it is also the decision cache key's
    /// state half: a decision made about one situation must not be replayed against
    /// another.
    pub fn digest(&self) -> String {
        mm_core::content_hash(self.canonical().as_bytes())
    }

    /// Refuse an input the firewall cannot evaluate honestly.
    pub fn validate(&self) -> Result<()> {
        self.risks.validate()?;
        if !self.stakes.level.is_finite() || !(0.0..=1.0).contains(&self.stakes.level) {
            return Err(FirewallError::validation(
                "stakes.level",
                format!("must be in [0,1], got {}", self.stakes.level),
            ));
        }
        if !self.stakes.irreversibility.is_finite()
            || !(0.0..=1.0).contains(&self.stakes.irreversibility)
        {
            return Err(FirewallError::validation(
                "stakes.irreversibility",
                format!("must be in [0,1], got {}", self.stakes.irreversibility),
            ));
        }
        for (index, value) in self.uncertainty.iter().enumerate() {
            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                return Err(FirewallError::validation(
                    "uncertainty",
                    format!("uncertainty[{index}] must be in [0,1], got {value}"),
                ));
            }
        }
        if let Some(support) = self.factuality_support {
            if !support.is_finite() || !(0.0..=1.0).contains(&support) {
                return Err(FirewallError::validation(
                    "factuality_support",
                    format!("must be in [0,1], got {support}"),
                ));
            }
        }
        if let Some(spend) = &self.spend {
            if !spend.projected.is_finite() || !spend.budget.is_finite() {
                return Err(FirewallError::validation(
                    "spend",
                    "projected and budget must be finite",
                ));
            }
            if spend.budget < 0.0 {
                return Err(FirewallError::validation(
                    "spend.budget",
                    format!("must not be negative, got {}", spend.budget),
                ));
            }
        }
        if !self.authorization.granted && self.authorization.approval_id.is_some() {
            return Err(FirewallError::validation(
                "authorization",
                "an approval record exists but the grant is marked not granted",
            ));
        }
        Ok(())
    }

    /// True when the authorization covers the tool the action would use.
    pub fn tool_authorized(&self) -> bool {
        self.tool
            .as_ref()
            .is_none_or(|tool| tool.permission_granted)
    }
}

/// The hard prohibition that fired.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardProhibition {
    /// The prohibition id, one of [`crate::prohibitions::PROHIBITION_IDS`].
    pub id: String,
    /// Why it fired, in words.
    pub reason: String,
    /// What the firewall looked at, one line per piece of evidence.
    pub evidence: Vec<String>,
}

impl HardProhibition {
    /// A prohibition.
    pub fn new(id: impl Into<String>, reason: impl Into<String>, evidence: Vec<String>) -> Self {
        HardProhibition {
            id: id.into(),
            reason: reason.into(),
            evidence,
        }
    }

    /// The reason code this prohibition is reported under.
    pub fn reason_code(&self) -> String {
        crate::outcome::prohibition_reason_code(&self.id)
    }

    /// A stable rendering.
    pub fn canonical(&self) -> String {
        format!("{}|{}|{}", self.id, self.reason, self.evidence.join(","))
    }
}

/// What one firewall run concluded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FirewallReport {
    /// The run's own identifier.
    pub id: Ulid,
    /// The episode the action belongs to.
    pub episode: Ulid,
    /// The outcome.
    pub outcome: FirewallOutcome,
    /// Why, sorted and deduplicated by code.
    pub reason_codes: Vec<ReasonCode>,
    /// The decision questions consulted, in the order they were asked.
    pub decisions: Vec<Ulid>,
    /// The hard prohibition that fired, if one did. Present exactly when the
    /// outcome is `REJECT` because of one.
    pub hard_prohibition: Option<HardProhibition>,
    /// The numeric magnitudes behind the reasons, keyed by reason code.
    pub signals: BTreeMap<String, f64>,
    /// The digest of the input this run evaluated.
    pub input_digest: String,
}

impl FirewallReport {
    /// Assemble a report from an evaluated input.
    ///
    /// The episode and the input digest come from the input rather than from
    /// parameters: a report that named a different episode than the input it
    /// evaluated would be a record of nothing.
    pub fn new(
        id: Ulid,
        input: &FirewallInput,
        outcome: FirewallOutcome,
        reason_codes: Vec<ReasonCode>,
        decisions: Vec<Ulid>,
        hard_prohibition: Option<HardProhibition>,
        signals: BTreeMap<String, f64>,
    ) -> Self {
        let mut reason_codes = reason_codes;
        sort_signals(&mut reason_codes);
        FirewallReport {
            id,
            episode: input.episode,
            outcome,
            reason_codes,
            decisions,
            hard_prohibition,
            signals,
            input_digest: input.digest(),
        }
    }

    /// True when a hard prohibition fired.
    pub fn has_prohibition(&self) -> bool {
        self.hard_prohibition.is_some()
    }

    /// The reason codes alone, in report order — the `reason_codes_json` payload.
    ///
    /// Codes rather than whole reasons because the SQLite column is an index and a
    /// later phase aggregates over it; the readable detail is in
    /// [`FirewallReport::canonical`] and in the audit record.
    pub fn reason_code_strings(&self) -> Vec<String> {
        self.reason_codes.iter().map(|r| r.code.clone()).collect()
    }

    /// The outcome's wire name — the `firewall_runs.outcome` value.
    pub fn outcome_str(&self) -> &'static str {
        self.outcome.as_str()
    }

    /// The prohibition's id, when one fired.
    pub fn hard_prohibition_id(&self) -> Option<&str> {
        self.hard_prohibition.as_ref().map(|h| h.id.as_str())
    }

    /// A stable rendering, so two runs on the same input compare as strings.
    pub fn canonical(&self) -> String {
        let reasons: Vec<String> = self.reason_codes.iter().map(|r| r.canonical()).collect();
        let decisions: Vec<String> = self.decisions.iter().map(mm_core::ulid_string).collect();
        let signals: Vec<String> = self
            .signals
            .iter()
            .map(|(code, value)| format!("{code}={value:.9}"))
            .collect();
        format!(
            "id={}\nepisode={}\noutcome={}\ninput_digest={}\nreasons=[{}]\ndecisions=[{}]\nsignals=[{}]\nprohibition={}",
            mm_core::ulid_string(&self.id),
            mm_core::ulid_string(&self.episode),
            self.outcome.as_str(),
            self.input_digest,
            reasons.join(";"),
            decisions.join(","),
            signals.join(","),
            self.hard_prohibition
                .as_ref()
                .map_or_else(|| "-".to_string(), |h| h.canonical())
        )
    }

    /// The content hash of the report.
    pub fn content_hash(&self) -> String {
        mm_core::content_hash(self.canonical().as_bytes())
    }
}

/// Serialize one value to a single deterministic line.
fn line<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

/// Serialize a list to sorted, joined lines so its order cannot change a digest.
fn sorted_lines<T: Serialize>(items: &[T]) -> String {
    let mut lines: Vec<String> = items.iter().map(line).collect();
    lines.sort();
    lines.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_decision::risk::{analyze_risk_with, LossOutcome, RiskMeasure, RiskOptions};

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    /// A risk profile that is not catastrophic, for fixtures that are not about
    /// risk. The capital is large enough that a loss of this size cannot deplete it,
    /// which is how a fixture stays out of the way of the rail that reads it.
    fn benign_risks() -> RiskProfile {
        analyze_risk_with(
            &[
                LossOutcome {
                    name: "small".into(),
                    probability: 0.9,
                    loss: 1.0,
                },
                LossOutcome {
                    name: "large".into(),
                    probability: 0.1,
                    loss: 100.0,
                },
            ],
            RiskMeasure::Var { alpha: 0.9 },
            &RiskOptions {
                capital: 1.0e9,
                reversibility: 0.5,
                optionality: 0.5,
            },
        )
    }

    fn clean() -> FirewallInput {
        FirewallInput::new(ulid(1), benign_risks())
    }

    #[test]
    fn a_digest_ignores_list_order() {
        let mut a = clean();
        a.assumptions = vec![
            AssumptionRef {
                id: ulid(2),
                claim: "the cache is warm".into(),
                decision_critical: false,
                verified: true,
            },
            AssumptionRef {
                id: ulid(3),
                claim: "the index is current".into(),
                decision_critical: true,
                verified: false,
            },
        ];
        a.uncertainty = vec![0.2, 0.7];
        let mut b = a.clone();
        b.assumptions.reverse();
        b.uncertainty.reverse();
        assert_eq!(a.digest(), b.digest());
        assert_eq!(a.digest().len(), 64);
    }

    #[test]
    fn a_changed_scalar_changes_the_digest() {
        let a = clean();
        let mut b = clean();
        b.reversibility = Reversibility::Irreversible;
        assert_ne!(a.digest(), b.digest());
    }

    #[test]
    fn an_out_of_range_input_is_refused() {
        let mut input = clean();
        input.stakes.level = 1.5;
        assert_eq!(input.validate().unwrap_err().code(), "validation");

        let mut input = clean();
        input.uncertainty = vec![0.5, 2.0];
        assert!(input.validate().is_err());

        let mut input = clean();
        input.spend = Some(Spend {
            projected: 1.0,
            budget: -1.0,
        });
        assert!(input.validate().is_err());

        let mut input = clean();
        input.authorization.granted = false;
        input.authorization.approval_id = Some(ulid(9));
        assert!(input.validate().is_err());
    }

    #[test]
    fn a_report_sorts_and_indexes_its_reasons() {
        let input = clean();
        let report = FirewallReport::new(
            ulid(10),
            &input,
            FirewallOutcome::VerifyFirst,
            vec![
                ReasonCode::new(
                    "signal.nominal",
                    "signal.nominal",
                    "nothing objected",
                    FirewallOutcome::Proceed,
                ),
                ReasonCode::new(
                    "signal.high_uncertainty",
                    "safety",
                    "below the verify cutoff",
                    FirewallOutcome::VerifyFirst,
                ),
            ],
            vec![ulid(11)],
            None,
            BTreeMap::new(),
        );
        assert_eq!(
            report.reason_code_strings(),
            vec![
                "signal.high_uncertainty".to_string(),
                "signal.nominal".to_string()
            ],
            "the more severe reason sorts first"
        );
        assert_eq!(report.outcome_str(), "VERIFY_FIRST");
        assert!(!report.has_prohibition());
        assert_eq!(report.input_digest, input.digest());
        assert_eq!(report.content_hash().len(), 64);
        assert_eq!(report.content_hash(), report.content_hash());
    }

    #[test]
    fn a_prohibition_reason_code_is_derived_from_its_id() {
        let prohibition = HardProhibition::new(
            "irreversible_without_approval",
            "the action cannot be undone and no approval covers it",
            vec!["reversibility=irreversible".into(), "granted=false".into()],
        );
        assert_eq!(
            prohibition.reason_code(),
            "prohibition.irreversible_without_approval"
        );
        assert!(prohibition.canonical().contains("granted=false"));
    }

    #[test]
    fn tool_authorization_is_absent_when_there_is_no_tool() {
        let mut input = clean();
        assert!(input.tool_authorized());
        input.tool = Some(ToolTarget {
            name: "wipe".into(),
            permission_granted: false,
        });
        assert!(!input.tool_authorized());
    }

    #[test]
    fn reversibility_round_trips_through_its_wire_name() {
        for value in [
            Reversibility::Reversible,
            Reversibility::PartiallyReversible,
            Reversibility::Irreversible,
        ] {
            assert_eq!(Reversibility::parse(value.as_str()), Some(value));
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(json, format!("\"{}\"", value.as_str()));
        }
        assert_eq!(Reversibility::parse("sometimes"), None);
    }
}
