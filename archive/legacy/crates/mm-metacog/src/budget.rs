//! The budget, and the forcer that decides whether spending more is warranted.
//!
//! The phase's invariant 5 is "budget is enforced, never clamped": a debit that
//! would cross a limit is refused whole, and the loop turns that refusal into a
//! stop cause rather than quietly spending the remainder. That is what makes
//! `budget-audit` a meaningful gate — no episode can be over budget, because it
//! could not have been written.
//!
//! The s1-style budget forcer is the other half: [`BudgetForcer`] holds a fraction
//! of the budget in reserve and only lets a *high-value* operation spend it. It
//! never debits anything itself, so it cannot be the thing that overspends; it
//! only says whether the next operation is worth attempting.

use serde::{Deserialize, Serialize};

use crate::error::{BudgetExceeded, MetacogError, Result};
use crate::tier::ComputePolicy;

/// One dimension of the budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetResource {
    /// Operations executed.
    Ops,
    /// Model calls made.
    LlmCalls,
    /// Cost, in the same units as the operation values.
    Cost,
    /// Wall-clock milliseconds.
    WallMs,
}

/// Every dimension, in the order the audit reports them.
pub const BUDGET_RESOURCES: [BudgetResource; 4] = [
    BudgetResource::Ops,
    BudgetResource::LlmCalls,
    BudgetResource::Cost,
    BudgetResource::WallMs,
];

impl BudgetResource {
    /// Every dimension.
    pub const ALL: [BudgetResource; 4] = BUDGET_RESOURCES;

    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetResource::Ops => "ops",
            BudgetResource::LlmCalls => "llm_calls",
            BudgetResource::Cost => "cost",
            BudgetResource::WallMs => "wall_ms",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        BUDGET_RESOURCES.into_iter().find(|r| r.as_str() == text)
    }
}

impl std::fmt::Display for BudgetResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A request to spend part of the budget.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetDebit {
    /// The dimension being spent.
    pub resource: BudgetResource,
    /// How much. Non-negative and finite.
    pub amount: f64,
}

impl BudgetDebit {
    /// Debit `amount` operations.
    pub fn ops(amount: u32) -> Self {
        BudgetDebit {
            resource: BudgetResource::Ops,
            amount: f64::from(amount),
        }
    }

    /// Debit `amount` model calls.
    pub fn llm_calls(amount: u32) -> Self {
        BudgetDebit {
            resource: BudgetResource::LlmCalls,
            amount: f64::from(amount),
        }
    }

    /// Debit `amount` cost.
    pub fn cost(amount: f64) -> Self {
        BudgetDebit {
            resource: BudgetResource::Cost,
            amount,
        }
    }

    /// Debit `amount` milliseconds.
    pub fn wall_ms(amount: u64) -> Self {
        BudgetDebit {
            resource: BudgetResource::WallMs,
            amount: amount as f64,
        }
    }
}

/// Why the loop stopped. A stop is a success, not a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopCause {
    /// A budget dimension ran out.
    BudgetExhausted,
    /// Remaining uncertainty fell below the program's threshold.
    RemainingUncertaintyBelow,
    /// No ready operation's expected value cleared the threshold.
    ExpectedValueBelow,
    /// The required confidence was reached.
    RequiredConfidenceMet,
    /// An irreversible action was reached and not taken.
    IrreversibleActionReached,
    /// Nothing left to run.
    NoReadyOperation,
}

impl StopCause {
    /// The stable wire name, matching the `program_traces.stopping_reason` column.
    pub fn as_str(self) -> &'static str {
        match self {
            StopCause::BudgetExhausted => "budget_exhausted",
            StopCause::RemainingUncertaintyBelow => "remaining_uncertainty_below",
            StopCause::ExpectedValueBelow => "expected_value_below",
            StopCause::RequiredConfidenceMet => "required_confidence_met",
            StopCause::IrreversibleActionReached => "irreversible_action_reached",
            StopCause::NoReadyOperation => "no_ready_operation",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        [
            StopCause::BudgetExhausted,
            StopCause::RemainingUncertaintyBelow,
            StopCause::ExpectedValueBelow,
            StopCause::RequiredConfidenceMet,
            StopCause::IrreversibleActionReached,
            StopCause::NoReadyOperation,
        ]
        .into_iter()
        .find(|c| c.as_str() == text)
    }
}

impl std::fmt::Display for StopCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a deliberation may spend, and what it has.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CognitiveBudget {
    /// The operation ceiling.
    pub max_ops: u32,
    /// The model-call ceiling.
    pub max_llm_calls: u32,
    /// The cost ceiling.
    pub max_cost: f64,
    /// The wall-clock ceiling in milliseconds.
    pub max_wall_ms: u64,
    /// Operations spent.
    pub spent_ops: u32,
    /// Model calls spent.
    pub spent_llm_calls: u32,
    /// Cost spent.
    pub spent_cost: f64,
    /// Milliseconds spent.
    pub spent_wall_ms: u64,
}

impl CognitiveBudget {
    /// A budget with the given ceilings and nothing spent.
    pub fn from_spec(
        max_ops: u32,
        max_llm_calls: u32,
        max_cost: f64,
        max_wall_ms: u64,
    ) -> Result<Self> {
        let budget = CognitiveBudget {
            max_ops,
            max_llm_calls,
            max_cost,
            max_wall_ms,
            spent_ops: 0,
            spent_llm_calls: 0,
            spent_cost: 0.0,
            spent_wall_ms: 0,
        };
        budget.validate()?;
        Ok(budget)
    }

    /// Refuse a budget that cannot bound anything.
    pub fn validate(&self) -> Result<()> {
        if !self.max_cost.is_finite() || self.max_cost < 0.0 {
            return Err(MetacogError::validation(
                "budget.max_cost",
                format!("must be finite and non-negative, got {}", self.max_cost),
            ));
        }
        if !self.spent_cost.is_finite() || self.spent_cost < 0.0 {
            return Err(MetacogError::validation(
                "budget.spent_cost",
                format!("must be finite and non-negative, got {}", self.spent_cost),
            ));
        }
        if self.spent_cost > self.max_cost {
            return Err(MetacogError::validation(
                "budget.spent_cost",
                format!(
                    "{} spent already exceeds the limit {}",
                    self.spent_cost, self.max_cost
                ),
            ));
        }
        if self.spent_ops > self.max_ops {
            return Err(MetacogError::validation(
                "budget.spent_ops",
                format!(
                    "{} spent already exceeds the limit {}",
                    self.spent_ops, self.max_ops
                ),
            ));
        }
        if self.spent_llm_calls > self.max_llm_calls {
            return Err(MetacogError::validation(
                "budget.spent_llm_calls",
                format!(
                    "{} spent already exceeds the limit {}",
                    self.spent_llm_calls, self.max_llm_calls
                ),
            ));
        }
        if self.spent_wall_ms > self.max_wall_ms {
            return Err(MetacogError::validation(
                "budget.spent_wall_ms",
                format!(
                    "{} spent already exceeds the limit {}",
                    self.spent_wall_ms, self.max_wall_ms
                ),
            ));
        }
        Ok(())
    }

    /// How much of a dimension is left.
    pub fn remaining(&self, resource: BudgetResource) -> f64 {
        match resource {
            BudgetResource::Ops => f64::from(self.max_ops.saturating_sub(self.spent_ops)),
            BudgetResource::LlmCalls => {
                f64::from(self.max_llm_calls.saturating_sub(self.spent_llm_calls))
            }
            BudgetResource::Cost => (self.max_cost - self.spent_cost).max(0.0),
            BudgetResource::WallMs => (self.max_wall_ms.saturating_sub(self.spent_wall_ms)) as f64,
        }
    }

    /// How much of a dimension was granted.
    pub fn limit(&self, resource: BudgetResource) -> f64 {
        match resource {
            BudgetResource::Ops => f64::from(self.max_ops),
            BudgetResource::LlmCalls => f64::from(self.max_llm_calls),
            BudgetResource::Cost => self.max_cost,
            BudgetResource::WallMs => self.max_wall_ms as f64,
        }
    }

    /// What has been spent on a dimension.
    pub fn spent(&self, resource: BudgetResource) -> f64 {
        match resource {
            BudgetResource::Ops => f64::from(self.spent_ops),
            BudgetResource::LlmCalls => f64::from(self.spent_llm_calls),
            BudgetResource::Cost => self.spent_cost,
            BudgetResource::WallMs => self.spent_wall_ms as f64,
        }
    }

    /// Spend, or refuse whole.
    ///
    /// A refused debit leaves the budget *unchanged*: not clamped to the limit, not
    /// partially applied. The loop treats the refusal as a stop cause, so the
    /// recorded spend is always something the caller asked for and got.
    pub fn debit(&mut self, d: BudgetDebit) -> std::result::Result<(), BudgetExceeded> {
        if !d.amount.is_finite() || d.amount < 0.0 {
            // A non-finite or negative amount cannot be a legitimate spend; refuse
            // it as exceeding, because that is the only refusal this signature has.
            return Err(BudgetExceeded {
                resource: d.resource,
                limit: self.limit(d.resource),
                spent: self.spent(d.resource),
                requested: d.amount,
            });
        }
        let limit = self.limit(d.resource);
        let spent = self.spent(d.resource);
        if spent + d.amount > limit {
            return Err(BudgetExceeded {
                resource: d.resource,
                limit,
                spent,
                requested: d.amount,
            });
        }
        match d.resource {
            BudgetResource::Ops => {
                self.spent_ops = self.spent_ops.saturating_add(d.amount as u32);
            }
            BudgetResource::LlmCalls => {
                self.spent_llm_calls = self.spent_llm_calls.saturating_add(d.amount as u32);
            }
            BudgetResource::Cost => {
                self.spent_cost += d.amount;
            }
            BudgetResource::WallMs => {
                self.spent_wall_ms = self.spent_wall_ms.saturating_add(d.amount as u64);
            }
        }
        Ok(())
    }

    /// The dimension that is exhausted, if any, so the loop can name the stop.
    pub fn exhausted(&self) -> Option<StopCause> {
        BUDGET_RESOURCES
            .into_iter()
            .find(|resource| self.remaining(*resource) <= 0.0)
            .map(|_| StopCause::BudgetExhausted)
    }

    /// True when nothing is left in any dimension.
    pub fn is_empty(&self) -> bool {
        self.exhausted().is_some()
    }

    /// The fraction of the *operation* ceiling still available, in `[0,1]`.
    ///
    /// Operations rather than cost because it is the dimension the forcer can
    /// reason about without knowing a price: a forcer that divided by a cost limit
    /// of zero would be deciding on a number nobody set.
    pub fn remaining_fraction(&self) -> f64 {
        let limit = f64::from(self.max_ops);
        if limit <= 0.0 {
            return 0.0;
        }
        (self.remaining(BudgetResource::Ops) / limit).clamp(0.0, 1.0)
    }
}

/// What the forcer decided about the next operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Spend normally.
    Permit,
    /// Spend, but out of the held-back reserve — only for high-value operations.
    Reserve,
    /// Do not spend.
    Forbid,
}

impl Decision {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Permit => "permit",
            Decision::Reserve => "reserve",
            Decision::Forbid => "forbid",
        }
    }

    /// True when the operation may be attempted.
    pub fn allows(self) -> bool {
        !matches!(self, Decision::Forbid)
    }
}

/// Holds a fraction of the budget back for operations that could change the
/// decision. Deterministic: no sampling, no randomness.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetForcer {
    /// The fraction of the operation ceiling held in reserve, in `[0,1]`.
    pub headroom: f64,
}

impl Default for BudgetForcer {
    fn default() -> Self {
        // A quarter held back is enough to stop a long tail of cheap, useless
        // operations from spending the budget before the one that matters.
        BudgetForcer { headroom: 0.25 }
    }
}

impl BudgetForcer {
    /// A forcer holding `headroom` of the operation ceiling back.
    pub fn new(headroom: f64) -> Result<Self> {
        if !headroom.is_finite() || !(0.0..=1.0).contains(&headroom) {
            return Err(MetacogError::validation(
                "forcer.headroom",
                format!("must be in [0,1], got {headroom}"),
            ));
        }
        Ok(BudgetForcer { headroom })
    }

    /// The score an operation must reach to be allowed to dip into the reserve at
    /// this tier.
    ///
    /// Higher tiers hold back more and demand more: the tiers that exist for hard
    /// problems are exactly the ones where a cheap operation is least likely to
    /// settle anything.
    pub fn reserve_threshold(policy: ComputePolicy) -> f64 {
        match policy.tier {
            crate::tier::Tier::T0 | crate::tier::Tier::T1 => 0.0,
            crate::tier::Tier::T2 => 0.5,
            crate::tier::Tier::T3 => 1.0,
            crate::tier::Tier::T4 => 2.0,
            crate::tier::Tier::T5 => 4.0,
        }
    }

    /// Decide whether an operation scoring `next` may run.
    ///
    /// * Budget already exhausted, or nothing scored: `Forbid`.
    /// * More than `headroom` of the ceiling left: `Permit`.
    /// * Inside the reserve: `Reserve` when the score reaches the tier's
    ///   threshold, `Forbid` otherwise.
    pub fn permit(&self, next: f64, budget: &CognitiveBudget, policy: ComputePolicy) -> Decision {
        if budget.exhausted().is_some() {
            return Decision::Forbid;
        }
        if !next.is_finite() || next <= 0.0 {
            return Decision::Forbid;
        }
        if budget.remaining_fraction() > self.headroom {
            return Decision::Permit;
        }
        if next >= Self::reserve_threshold(policy) {
            Decision::Reserve
        } else {
            Decision::Forbid
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tier::{ComputePolicy, Tier};

    fn budget() -> CognitiveBudget {
        CognitiveBudget::from_spec(10, 5, 1.0, 1000).unwrap()
    }

    fn policy(tier: Tier) -> ComputePolicy {
        tier.policy()
    }

    #[test]
    fn resources_round_trip_and_count_four() {
        assert_eq!(BUDGET_RESOURCES.len(), 4);
        for resource in BUDGET_RESOURCES {
            assert_eq!(BudgetResource::parse(resource.as_str()), Some(resource));
        }
        assert_eq!(BudgetResource::parse("nope"), None);
    }

    #[test]
    fn a_debit_within_the_limit_is_applied() {
        let mut b = budget();
        b.debit(BudgetDebit::ops(3)).unwrap();
        assert_eq!(b.spent_ops, 3);
        b.debit(BudgetDebit::cost(0.25)).unwrap();
        assert!((b.spent_cost - 0.25).abs() < 1e-12);
        assert_eq!(b.remaining(BudgetResource::Ops), 7.0);
    }

    #[test]
    fn an_over_debit_is_refused_and_changes_nothing() {
        let mut b = budget();
        b.debit(BudgetDebit::ops(8)).unwrap();
        let err = b.debit(BudgetDebit::ops(3)).unwrap_err();
        assert_eq!(err.resource, BudgetResource::Ops);
        assert_eq!(
            b.spent_ops, 8,
            "a refused debit must not be partially applied"
        );
        // Exactly the remainder is still allowed.
        b.debit(BudgetDebit::ops(2)).unwrap();
        assert_eq!(b.spent_ops, 10);
        assert_eq!(b.exhausted(), Some(StopCause::BudgetExhausted));
    }

    #[test]
    fn a_negative_or_non_finite_debit_is_refused() {
        let mut b = budget();
        assert!(b.debit(BudgetDebit::cost(-1.0)).is_err());
        assert!(b.debit(BudgetDebit::cost(f64::NAN)).is_err());
        assert_eq!(b.spent_cost, 0.0);
    }

    #[test]
    fn the_forcer_permits_early_and_reserves_late() {
        let forcer = BudgetForcer::new(0.25).unwrap();
        let tier = policy(Tier::T3);

        let mut early = budget();
        assert_eq!(forcer.permit(0.1, &early, tier), Decision::Permit);
        assert_eq!(forcer.permit(0.0, &early, tier), Decision::Forbid);

        // Spend down into the reserve.
        early.debit(BudgetDebit::ops(9)).unwrap();
        assert_eq!(early.remaining_fraction(), 0.1);
        // T3 demands a score of 1.0 to spend the reserve.
        assert_eq!(forcer.permit(0.5, &early, tier), Decision::Forbid);
        assert_eq!(forcer.permit(1.5, &early, tier), Decision::Reserve);

        let exhausted = {
            let mut b = budget();
            b.debit(BudgetDebit::ops(10)).unwrap();
            b
        };
        assert_eq!(forcer.permit(100.0, &exhausted, tier), Decision::Forbid);
        assert!(!Decision::Forbid.allows());
        assert!(Decision::Reserve.allows());
    }

    #[test]
    fn the_forcer_can_never_overspend_because_it_never_debits() {
        let forcer = BudgetForcer::new(0.5).unwrap();
        let mut b = budget();
        let tier = policy(Tier::T5);
        while forcer.permit(10.0, &b, tier).allows() {
            if b.debit(BudgetDebit::ops(1)).is_err() {
                break;
            }
        }
        assert_eq!(b.spent_ops, b.max_ops);
        assert!(b.exhausted().is_some());
    }

    #[test]
    fn a_bad_headroom_is_refused() {
        assert!(BudgetForcer::new(-0.1).is_err());
        assert!(BudgetForcer::new(1.5).is_err());
        assert!(BudgetForcer::new(f64::NAN).is_err());
        assert_eq!(BudgetForcer::default().headroom, 0.25);
    }

    #[test]
    fn a_budget_cannot_be_built_already_over_spent() {
        let mut raw = budget();
        raw.spent_cost = 2.0;
        assert!(raw.validate().is_err());
        assert!(CognitiveBudget::from_spec(1, 1, -1.0, 1).is_err());
    }

    #[test]
    fn stop_causes_round_trip() {
        for cause in [
            StopCause::BudgetExhausted,
            StopCause::RemainingUncertaintyBelow,
            StopCause::ExpectedValueBelow,
            StopCause::RequiredConfidenceMet,
            StopCause::IrreversibleActionReached,
            StopCause::NoReadyOperation,
        ] {
            assert_eq!(StopCause::parse(cause.as_str()), Some(cause));
        }
        assert_eq!(StopCause::parse("nope"), None);
    }
}
