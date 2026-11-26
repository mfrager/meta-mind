//! Assumptions: the propositions the being holds without support, and what it
//! costs to stop holding them that way.
//!
//! An assumption is the only kind of claim the being is *allowed* to hold with no
//! evidence, which makes it the dangerous kind. Two consequences shape this
//! module:
//!
//! * **An assumption carries its own risk.** `consequence_if_false` is required —
//!   there is no "it depends" — because the whole point of the ledger is to
//!   answer "what am I relying on, and what happens if I am wrong".
//! * **Priority is arithmetic, not judgement.** [`verification_priority`] is a
//!   fixed `f64` expression over the assumption's fields and one caller-supplied
//!   probability. The plan's invariant 6 says probabilities enter from the caller
//!   and are never computed here, so `p_false` is a parameter, not a prediction.

use mm_core::Ulid;
use serde::{Deserialize, Serialize};

use crate::error::{EpistemicError, Result};
use crate::proposition::{Proposition, RiskLevel};
use crate::status::EpistemicStatus;

/// A proposition held without support, with its cost of being wrong.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assumption {
    /// The assumption's ULID.
    pub id: Ulid,
    /// What is being assumed.
    pub proposition: Proposition,
    /// Where it stands. An assumption starts `ASSUMED`; it may rise only through
    /// the guard, and it may never rise straight to an observation.
    pub status: EpistemicStatus,
    /// How confident the caller is, in `[0,1]`.
    pub confidence: f32,
    /// What it costs if it is false. Required: there is no "unknown" risk.
    pub consequence_if_false: RiskLevel,
    /// What it would cost to check, in arbitrary consistent units.
    pub verification_cost: f64,
    /// How much a decision depends on it, in `[0,1]`.
    pub decision_dependence: f64,
    /// The other records it depends on.
    pub dependencies: Vec<Ulid>,
    /// When it starts being relied on, if it says.
    pub valid_from: Option<mm_core::Timestamp>,
    /// When it stops, if it does.
    pub valid_until: Option<mm_core::Timestamp>,
}

impl Assumption {
    /// Build an assumption. It starts `ASSUMED`; nothing else is a starting point.
    pub fn new(
        id: Ulid,
        proposition: Proposition,
        confidence: f32,
        consequence_if_false: RiskLevel,
        verification_cost: f64,
        decision_dependence: f64,
    ) -> Result<Self> {
        let assumption = Assumption {
            id,
            proposition,
            status: EpistemicStatus::Assumed,
            confidence,
            consequence_if_false,
            verification_cost,
            decision_dependence,
            dependencies: Vec::new(),
            valid_from: None,
            valid_until: None,
        };
        assumption.validate()?;
        Ok(assumption)
    }

    /// The records it depends on.
    pub fn with_dependencies(mut self, dependencies: Vec<Ulid>) -> Self {
        self.dependencies = dependencies;
        self
    }

    /// The interval it is relied on over.
    pub fn with_validity(
        mut self,
        from: Option<mm_core::Timestamp>,
        until: Option<mm_core::Timestamp>,
    ) -> Self {
        self.valid_from = from;
        self.valid_until = until;
        self
    }

    /// Refuse an assumption that cannot be reasoned about.
    pub fn validate(&self) -> Result<()> {
        if self.id.is_nil() {
            return Err(EpistemicError::validation("id", "must not be the nil ULID"));
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(EpistemicError::validation(
                "confidence",
                format!("must be in [0,1], got {}", self.confidence),
            ));
        }
        if !(0.0..=1.0).contains(&self.decision_dependence) {
            return Err(EpistemicError::validation(
                "decision_dependence",
                format!("must be in [0,1], got {}", self.decision_dependence),
            ));
        }
        if !self.verification_cost.is_finite() || self.verification_cost < 0.0 {
            return Err(EpistemicError::validation(
                "verification_cost",
                format!(
                    "must be finite and non-negative, got {}",
                    self.verification_cost
                ),
            ));
        }
        if let (Some(from), Some(until)) = (self.valid_from, self.valid_until) {
            if until < from {
                return Err(EpistemicError::validation(
                    "validity",
                    "valid_until precedes valid_from",
                ));
            }
        }
        Ok(())
    }

    /// True while the being is still relying on it.
    pub fn is_open(&self) -> bool {
        self.valid_until.is_none()
    }
}

/// How much attention an assumption deserves, given how likely it is to be wrong.
///
/// The expression is fixed on purpose. Expected loss — probability times the risk
/// weight, raised when a decision leans on it — divided by what it costs to check
/// it. A caller that wants a different ordering supplies different
/// `verification_cost`/`decision_dependence`, not a different formula, so a
/// golden file can pin the result.
///
/// `p_false` is clamped into `[0,1]` rather than refused: it is a caller-supplied
/// belief, and a caller that says `1.2` means "certain".
pub fn verification_priority(assumption: &Assumption, p_false: f64) -> f64 {
    let risk = assumption.consequence_if_false.weight();
    let p_false = clamp_unit(p_false);
    let dependence = clamp_unit(assumption.decision_dependence);
    let expected_loss = p_false * risk * (1.0 + dependence);
    expected_loss / (1.0 + assumption.verification_cost.max(0.0))
}

/// Clamp a caller-supplied probability into `[0,1]`.
///
/// A `NaN` becomes `0` rather than propagating: a non-finite belief is not "very
/// likely", it is an unusable input, and letting it through would make a caller
/// bug visible only as a `NaN` in a golden file.
fn clamp_unit(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// How much verification an assumption is worth, in the same units as its cost.
///
/// Positive means the expected cost of being wrong outweighs the cost of
/// checking; non-positive means leaving it assumed is the better trade. This is a
/// *budget*, not a probability, so nothing here needs a model.
pub fn epistemic_budget(
    p_false: f64,
    error_cost: f64,
    uncertainty: f64,
    verification_cost: f64,
) -> f64 {
    let p_false = clamp_unit(p_false);
    let error_cost = if error_cost.is_finite() {
        error_cost.max(0.0)
    } else {
        0.0
    };
    let uncertainty = if uncertainty.is_finite() {
        uncertainty.max(0.0)
    } else {
        0.0
    };
    let verification_cost = if verification_cost.is_finite() {
        verification_cost.max(0.0)
    } else {
        0.0
    };
    p_false * error_cost * (1.0 + uncertainty) - verification_cost
}

/// The being's open assumptions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AssumptionLedger {
    /// The assumptions, in insertion order.
    pub assumptions: Vec<Assumption>,
}

impl AssumptionLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        AssumptionLedger::default()
    }

    /// Add an assumption, refusing a duplicate id.
    pub fn add(&mut self, assumption: Assumption) -> Result<()> {
        assumption.validate()?;
        if self.assumptions.iter().any(|a| a.id == assumption.id) {
            return Err(EpistemicError::validation(
                "id",
                format!(
                    "assumption {} is already in the ledger",
                    crate::error::id_text(&assumption.id)
                ),
            ));
        }
        self.assumptions.push(assumption);
        Ok(())
    }

    /// How many assumptions the ledger holds.
    pub fn len(&self) -> usize {
        self.assumptions.len()
    }

    /// True when nothing is assumed.
    pub fn is_empty(&self) -> bool {
        self.assumptions.is_empty()
    }

    /// The assumptions still being relied on.
    pub fn open(&self) -> Vec<&Assumption> {
        self.assumptions.iter().filter(|a| a.is_open()).collect()
    }

    /// Every assumption, ordered by verification priority: highest first, ties
    /// broken by ULID so the order is a function of the ledger alone.
    pub fn by_priority(&self, p_false: f64) -> Vec<(Ulid, f64)> {
        let mut ranked: Vec<(Ulid, f64)> = self
            .assumptions
            .iter()
            .map(|a| (a.id, verification_priority(a, p_false)))
            .collect();
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        ranked
    }

    /// The assumption with this id.
    pub fn get(&self, id: &Ulid) -> Option<&Assumption> {
        self.assumptions.iter().find(|a| a.id == *id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::id_text;
    use ulid::Ulid as UlidType;

    fn id(n: u128) -> Ulid {
        UlidType::from_parts(1_700_000_000_000, n)
    }

    fn proposition() -> Proposition {
        Proposition::literal("https://x/s", "https://x/p", "v").unwrap()
    }

    fn assumption(n: u128, risk: RiskLevel, cost: f64, dependence: f64) -> Assumption {
        Assumption::new(id(n), proposition(), 0.5, risk, cost, dependence).unwrap()
    }

    #[test]
    fn an_assumption_starts_assumed_and_refuses_bad_numbers() {
        let a = assumption(1, RiskLevel::Low, 0.0, 0.0);
        assert_eq!(a.status, EpistemicStatus::Assumed);
        assert!(Assumption::new(id(1), proposition(), 1.5, RiskLevel::Low, 0.0, 0.0).is_err());
        assert!(Assumption::new(id(1), proposition(), 0.5, RiskLevel::Low, -1.0, 0.0).is_err());
        assert!(Assumption::new(id(1), proposition(), 0.5, RiskLevel::Low, 0.0, 2.0).is_err());
    }

    #[test]
    fn priority_rises_with_risk_dependence_and_likelihood_and_falls_with_cost() {
        let cheap = assumption(1, RiskLevel::Low, 1.0, 0.5);
        let expensive = assumption(1, RiskLevel::Low, 4.0, 0.5);
        assert!(verification_priority(&cheap, 0.5) > verification_priority(&expensive, 0.5));

        let low = assumption(1, RiskLevel::Low, 0.0, 0.0);
        let high = assumption(1, RiskLevel::Catastrophic, 0.0, 0.0);
        assert!(verification_priority(&high, 0.5) > verification_priority(&low, 0.5));

        let unattached = assumption(1, RiskLevel::Medium, 0.0, 0.0);
        let load_bearing = assumption(1, RiskLevel::Medium, 0.0, 1.0);
        assert!(
            verification_priority(&load_bearing, 0.5) > verification_priority(&unattached, 0.5)
        );

        assert!(
            verification_priority(&unattached, 0.75) > verification_priority(&unattached, 0.25)
        );
        // A caller that says "more than certain" means "certain".
        assert_eq!(
            verification_priority(&unattached, 1.0),
            verification_priority(&unattached, 1.5)
        );
        assert_eq!(
            verification_priority(&unattached, 0.0),
            verification_priority(&unattached, -1.0)
        );
    }

    #[test]
    fn the_budget_says_whether_checking_is_worth_it() {
        // 0.5 * 10 * (1 + 0.2) = 6 > 2, so checking pays.
        assert!((epistemic_budget(0.5, 10.0, 0.2, 2.0) - 4.0).abs() < 1e-12);
        // A cheap check on a costly error is always worth it.
        assert!(epistemic_budget(1.0, 10.0, 0.0, 1.0) > 0.0);
        // An expensive check on a harmless error is not.
        assert!(epistemic_budget(0.1, 1.0, 0.0, 100.0) < 0.0);
        // A non-finite input cannot turn into a huge number.
        assert!(epistemic_budget(f64::NAN, f64::INFINITY, f64::NAN, 0.0).is_finite());
    }

    #[test]
    fn the_ledger_orders_by_priority_and_refuses_duplicates() {
        let mut ledger = AssumptionLedger::new();
        assert!(ledger.is_empty());
        ledger.add(assumption(2, RiskLevel::Low, 0.0, 0.0)).unwrap();
        ledger
            .add(assumption(1, RiskLevel::Catastrophic, 0.0, 0.0))
            .unwrap();
        assert_eq!(ledger.len(), 2);
        assert!(ledger.add(assumption(1, RiskLevel::Low, 0.0, 0.0)).is_err());
        let ranked = ledger.by_priority(0.5);
        assert_eq!(ranked[0].0, id(1), "the catastrophic assumption leads");
        assert_eq!(ranked[1].0, id(2));
        assert_eq!(ledger.open().len(), 2);
        assert!(ledger.get(&id(1)).is_some());
        // The error names the offending id.
        let error = ledger
            .add(assumption(1, RiskLevel::Low, 0.0, 0.0))
            .unwrap_err();
        assert!(error.to_string().contains(&id_text(&id(1))), "{error}");
    }
}
