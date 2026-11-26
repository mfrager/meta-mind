//! Resources and budgets: deterministic arithmetic, and nothing else.
//!
//! The LLM proposes a spend; it never decides one. Every debit is arithmetic over
//! an account balance, refused by a policy that is a plain `bool` rather than a
//! judgement, and the ledger row records the balance *after* the debit so the
//! account is provably a fold of its ledger (`sum(delta) == balance`).
//!
//! The library works on a caller-owned [`ResourceState`] rather than reaching for
//! a store, so a test can exercise the arithmetic with no database at all; the
//! facade is what persists the receipt.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{BeingError, BudgetExceeded, Result};

/// A budgeted quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// Model tokens.
    Tokens,
    /// Wall-clock milliseconds.
    Time,
    /// Money, in micros.
    Money,
    /// Attentional capacity.
    Attention,
    /// Discrete actions, e.g. tool calls.
    Actions,
}

impl ResourceKind {
    /// The stable wire name, which is the `resource_accounts.kind` primary key.
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceKind::Tokens => "tokens",
            ResourceKind::Time => "time",
            ResourceKind::Money => "money",
            ResourceKind::Attention => "attention",
            ResourceKind::Actions => "actions",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        ResourceKind::all().into_iter().find(|k| k.as_str() == s)
    }

    /// The unit a balance is counted in.
    pub fn unit(self) -> &'static str {
        match self {
            ResourceKind::Tokens => "tokens",
            ResourceKind::Time => "ms",
            ResourceKind::Money => "micros",
            ResourceKind::Attention => "units",
            ResourceKind::Actions => "count",
        }
    }

    /// Every kind, in a stable order.
    pub fn all() -> [ResourceKind; 5] {
        [
            ResourceKind::Tokens,
            ResourceKind::Time,
            ResourceKind::Money,
            ResourceKind::Attention,
            ResourceKind::Actions,
        ]
    }
}

impl std::fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What is left of one kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// The remaining balance.
    pub balance: f64,
    /// The unit the balance is counted in.
    pub unit: String,
}

/// The rule that decides whether a debit is allowed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetPolicy {
    /// The period the limit applies to, e.g. `session` or `day`.
    pub period: String,
    /// How far the balance may go below zero before a debit is refused.
    pub limit_amount: f64,
    /// A hard policy refuses any overdraft; a soft one allows a bounded one.
    pub hard: bool,
}

/// Every account, keyed by kind.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ResourceState {
    /// The accounts.
    pub accounts: BTreeMap<ResourceKind, Account>,
}

impl ResourceState {
    /// An empty set of accounts.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open (or reset) an account.
    pub fn open(&mut self, kind: ResourceKind, balance: f64) -> &mut Account {
        let account = self.entry(kind);
        account.balance = balance;
        account
    }

    /// The balance of a kind: `0.0` when no account is open.
    pub fn balance(&self, kind: ResourceKind) -> f64 {
        self.accounts.get(&kind).map_or(0.0, |a| a.balance)
    }

    fn entry(&mut self, kind: ResourceKind) -> &mut Account {
        self.accounts.entry(kind).or_insert_with(|| Account {
            balance: 0.0,
            unit: kind.unit().to_string(),
        })
    }
}

/// What a successful debit did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DebitReceipt {
    /// The kind debited.
    pub kind: ResourceKind,
    /// How much was taken.
    pub amount: f64,
    /// The balance afterwards, as written to the ledger.
    pub balance_after: f64,
    /// Why the spend happened.
    pub purpose: String,
}

/// Take `amount` from `kind`, or refuse.
///
/// Refusals are arithmetic and are recorded with the numbers that produced them,
/// so a rejected spend can be explained without guessing:
///
/// * a non-finite or non-positive amount is a caller error;
/// * a hard policy (or no policy) refuses to go below zero;
/// * a soft policy refuses to go below `-limit_amount`.
pub fn debit(
    state: &mut ResourceState,
    kind: ResourceKind,
    amount: f64,
    purpose: &str,
    policy: Option<&BudgetPolicy>,
) -> Result<DebitReceipt> {
    // The account is opened (at zero) before the amount is judged, so a refused
    // debit still leaves a kind that a caller asked about addressable.
    let account = state.entry(kind);
    let balance = account.balance;

    if !amount.is_finite() || amount <= 0.0 {
        return Err(BeingError::Config(format!(
            "a debit must be a finite, positive amount, got {amount}"
        )));
    }
    let label = match policy {
        Some(p) if p.hard => format!("hard:{}", p.period),
        Some(p) => format!("soft:{}", p.period),
        None => "default".to_string(),
    };
    let floor = match policy {
        Some(p) if !p.hard => -p.limit_amount,
        _ => 0.0,
    };

    if balance - amount < floor {
        return Err(BeingError::Budget(BudgetExceeded {
            kind: kind.as_str().to_string(),
            requested: amount,
            balance,
            policy: label,
        }));
    }
    account.balance = balance - amount;
    let balance_after = account.balance;
    Ok(DebitReceipt {
        kind,
        amount,
        balance_after,
        purpose: purpose.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hard_overspend_is_refused_and_leaves_the_balance_untouched() {
        let mut state = ResourceState::new();
        state.open(ResourceKind::Tokens, 100.0);
        let err = debit(&mut state, ResourceKind::Tokens, 101.0, "plan", None).unwrap_err();
        assert_eq!(err.kind(), "budget_exceeded");
        assert_eq!(err.code(), "mm.being.budget_exceeded");
        assert!(err.to_string().contains("100"), "{err}");
        assert!((state.balance(ResourceKind::Tokens) - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_debit_that_exactly_empties_an_account_succeeds() {
        let mut state = ResourceState::new();
        state.open(ResourceKind::Actions, 3.0);
        let receipt = debit(&mut state, ResourceKind::Actions, 3.0, "tool call", None).unwrap();
        assert!(receipt.balance_after.abs() < f64::EPSILON);
        assert!((state.balance(ResourceKind::Actions)).abs() < f64::EPSILON);
        // Exactly one more is refused.
        assert!(debit(&mut state, ResourceKind::Actions, 1.0, "tool call", None).is_err());
    }

    #[test]
    fn the_ledger_reconciles_exactly_with_the_account() {
        let mut state = ResourceState::new();
        state.open(ResourceKind::Money, 1.0);
        // Every balance change is a ledger delta — including the funding that
        // opened the account — so the account is exactly the fold of its ledger.
        let mut ledger: Vec<f64> = vec![1.0];
        let mut last_balance = 1.0;
        for step in 0..4 {
            let receipt = debit(
                &mut state,
                ResourceKind::Money,
                0.25,
                &format!("step {step}"),
                None,
            )
            .unwrap();
            ledger.push(-receipt.amount);
            // The recorded balance_after is the running total, exactly.
            last_balance -= receipt.amount;
            assert!((receipt.balance_after - last_balance).abs() < f64::EPSILON);
        }
        let delta_sum: f64 = ledger.iter().sum();
        let balance = state.balance(ResourceKind::Money);
        assert!(
            (delta_sum - balance).abs() < f64::EPSILON,
            "sum(delta) = {delta_sum} but balance = {balance}"
        );
        assert!(balance.abs() < f64::EPSILON);
    }

    #[test]
    fn a_soft_policy_tolerates_a_bounded_overdraft_and_no_more() {
        let mut state = ResourceState::new();
        state.open(ResourceKind::Attention, 1.0);
        let soft = BudgetPolicy {
            period: "session".into(),
            limit_amount: 0.5,
            hard: false,
        };
        let receipt = debit(
            &mut state,
            ResourceKind::Attention,
            1.4,
            "deep work",
            Some(&soft),
        )
        .unwrap();
        assert!((receipt.balance_after + 0.4).abs() < f64::EPSILON);

        let err = debit(
            &mut state,
            ResourceKind::Attention,
            0.2,
            "deep work",
            Some(&soft),
        )
        .unwrap_err();
        assert_eq!(err.kind(), "budget_exceeded");
        assert!(err.to_string().contains("soft:session"), "{err}");
        // A hard policy on the same state refuses immediately.
        let hard = BudgetPolicy {
            period: "day".into(),
            limit_amount: 5.0,
            hard: true,
        };
        let err = debit(
            &mut state,
            ResourceKind::Attention,
            0.1,
            "deep work",
            Some(&hard),
        )
        .unwrap_err();
        assert!(err.to_string().contains("hard:day"), "{err}");
    }

    #[test]
    fn a_non_positive_amount_is_a_caller_error_not_a_budget_event() {
        let mut state = ResourceState::new();
        for amount in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let err = debit(&mut state, ResourceKind::Time, amount, "x", None).unwrap_err();
            assert_eq!(err.kind(), "config", "{amount}");
        }
    }

    #[test]
    fn a_kind_with_no_account_opens_at_zero_and_its_unit_is_recorded() {
        let mut state = ResourceState::new();
        let receipt = debit(&mut state, ResourceKind::Actions, 0.0, "x", None);
        assert!(receipt.is_err());
        assert_eq!(state.balance(ResourceKind::Actions), 0.0);
        // The account exists now (opening happens before the amount is judged).
        assert_eq!(
            state
                .accounts
                .get(&ResourceKind::Actions)
                .map(|a| a.unit.as_str()),
            Some("count")
        );
        for kind in ResourceKind::all() {
            assert_eq!(ResourceKind::parse(kind.as_str()), Some(kind));
            assert!(!kind.unit().is_empty());
        }
        assert_eq!(ResourceKind::parse("vibes"), None);
    }
}
