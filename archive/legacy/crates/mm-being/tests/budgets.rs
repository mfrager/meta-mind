//! Deterministic budgets: arithmetic, reconciliation, and no concurrent overspend
//! (plan §2, §7, §8).
//!
//! Two properties are proved here. The account is exactly the fold of its ledger
//! (`sum(delta) == balance`, and every `balance_after` matches the running total),
//! and a spend is refused by arithmetic rather than by a judge — including when
//! two writers race for the same hard budget.

mod common;

use common::{fixture, jsonl, Harness};
use mm_being::{debit, BeingOp, BudgetPolicy, ResourceKind, ResourceState};
use mm_log::codes;

/// Every line of the overspend fixture lands where it says, and the ledger still
/// reconciles with the account afterwards.
#[test]
fn the_overdraft_fixtures_are_decided_by_arithmetic() {
    let entries = jsonl(&fixture("bench/being/budget_overspend.jsonl"));
    assert!(!entries.is_empty(), "the budget fixtures must not be empty");

    for entry in &entries {
        let name = entry["name"].as_str().unwrap();
        let kind = ResourceKind::parse(entry["kind"].as_str().unwrap()).unwrap();
        let balance = entry["balance"].as_f64().unwrap();
        let amount = entry["amount"].as_f64().unwrap();
        let limit = entry["limit"].as_f64().unwrap();
        let hard = entry["hard"].as_bool().unwrap();
        let expected = entry["expect"].as_str().unwrap();

        let policy = BudgetPolicy {
            period: entry["period"].as_str().unwrap().to_string(),
            limit_amount: limit,
            hard,
        };

        let mut state = ResourceState::new();
        state.open(kind, balance);
        // The funding that opened the account is a delta too, so the account is
        // exactly the fold of its ledger.
        let mut ledger: Vec<f64> = vec![balance];

        let outcome = debit(&mut state, kind, amount, name, Some(&policy));
        match expected {
            "allowed" => {
                let receipt = outcome
                    .unwrap_or_else(|e| panic!("{name}: expected an allowed spend, got {e}"));
                ledger.push(-amount);
                assert!(
                    (receipt.balance_after - (balance - amount)).abs() < f64::EPSILON,
                    "{name}: balance_after must be the running total"
                );
            }
            "refused" => {
                let err = match outcome {
                    Ok(_) => panic!("{name}: expected a refusal, was allowed"),
                    Err(e) => e,
                };
                assert_eq!(err.kind(), "budget_exceeded", "{name}: {err}");
                assert_eq!(err.code(), "mm.being.budget_exceeded");
                assert!(
                    (state.balance(kind) - balance).abs() < f64::EPSILON,
                    "{name}: a refusal must leave the balance untouched"
                );
            }
            other => panic!("{name}: unknown expectation {other:?}"),
        }

        let delta_sum: f64 = ledger.iter().sum();
        assert!(
            (delta_sum - state.balance(kind)).abs() < f64::EPSILON,
            "{name}: sum(delta) = {delta_sum} but balance = {}",
            state.balance(kind)
        );
    }
}

/// A spend through the facade persists the receipt, audits it, and keeps the
/// ledger reconciled; a refused spend audits a control event and writes nothing.
#[tokio::test]
async fn a_debit_reconciles_and_a_refusal_rolls_back() {
    let h = Harness::new().await;
    h.fund("actions", 10.0, "count").await;
    let mut facade = h.facade().await;

    facade
        .apply(BeingOp::DebitBudget {
            kind: ResourceKind::Actions,
            amount: 4.0,
            purpose: "one tool call".into(),
        })
        .await
        .unwrap();
    assert_eq!(h.scalar("SELECT count(*) FROM resource_ledger").await, 2);
    assert_eq!(h.records_of(codes::BEING_BUDGET_DEBIT).len(), 1);
    assert!(facade.budget_reconciles().await.unwrap());

    // A hard policy does not care that the balance could cover it later: going
    // below zero is refused, and the rollback leaves the ledger reconciling.
    h.policy("actions", "session", 0.0, true).await;
    let err = facade
        .apply(BeingOp::DebitBudget {
            kind: ResourceKind::Actions,
            amount: 7.0,
            purpose: "seven more calls".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "budget_exceeded");
    assert_eq!(h.scalar("SELECT count(*) FROM resource_ledger").await, 2);
    assert_eq!(h.records_of(codes::BEING_BUDGET_EXCEEDED).len(), 1);
    assert!(facade.budget_reconciles().await.unwrap());
    assert!(
        (h.query("SELECT balance FROM resource_accounts")
            .await
            .first()
            .and_then(|row| row["balance"].as_f64())
            .unwrap()
            - 6.0)
            .abs()
            < f64::EPSILON
    );

    h.shutdown().await;
}

/// Two writers cannot over-debit the same hard budget: the account row is read
/// under the write lock, so the loser sees the winner's balance and is refused.
#[tokio::test]
async fn two_concurrent_writers_cannot_overdraw_a_hard_budget() {
    let h = Harness::new().await;
    h.fund("actions", 10.0, "count").await;
    h.policy("actions", "session", 0.0, true).await;

    let mut first = h.facade().await;
    let mut second = h.facade().await;

    let (a, b) = tokio::join!(
        first.apply(BeingOp::DebitBudget {
            kind: ResourceKind::Actions,
            amount: 6.0,
            purpose: "a".into(),
        }),
        second.apply(BeingOp::DebitBudget {
            kind: ResourceKind::Actions,
            amount: 6.0,
            purpose: "b".into(),
        }),
    );

    let successes = usize::from(a.is_ok()) + usize::from(b.is_ok());
    assert_eq!(
        successes, 1,
        "exactly one 6-of-10 spend may succeed: {a:?} / {b:?}"
    );

    let balance = h
        .query("SELECT balance FROM resource_accounts")
        .await
        .first()
        .and_then(|row| row["balance"].as_f64())
        .unwrap();
    assert!(
        (balance - 4.0).abs() < f64::EPSILON,
        "the balance must be 4, got {balance}"
    );

    let ledger_sum: f64 = h
        .query("SELECT delta FROM resource_ledger")
        .await
        .iter()
        .filter_map(|row| row["delta"].as_f64())
        .sum();
    assert!(
        (ledger_sum - balance).abs() < f64::EPSILON,
        "sum(delta) = {ledger_sum} but balance = {balance}"
    );
    // The facade's own reconciliation check agrees.
    let facade = h.facade().await;
    assert!(facade.budget_reconciles().await.unwrap());
    h.shutdown().await;
}
