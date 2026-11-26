//! Goals and commitments: an append-only lifecycle with frozen terminal rows
//! (plan §2, §7, §8).
//!
//! The claim under test is that a terminal status is *history*. It is enforced in
//! three places that must agree — the lifecycle table in `goals.rs`, the guard in
//! `invariants.rs`, and the SQL trigger in `0004_being.sql` — so this file drives
//! all three: the fixtures exercise the table, the facade drives the guard, and a
//! direct `UPDATE` drives the trigger.

mod common;

use common::{fixture, jsonl, user, Harness};
use mm_being::{
    transition_commitment, transition_goal, BeingError, BeingOp, CommitmentStatus, GoalOrigin,
    GoalStatus, InvariantCode,
};
use mm_log::codes;

/// Every line of the lifecycle fixture lands where it says, with a no-op
/// re-assertion treated as legal and every move out of a terminal status denied.
#[test]
fn the_lifecycle_matches_every_fixture_line() {
    let entries = jsonl(&fixture("bench/being/commitment_lifecycle.jsonl"));
    assert!(
        !entries.is_empty(),
        "the lifecycle fixtures must not be empty"
    );

    for entry in &entries {
        let name = entry["name"].as_str().unwrap();
        let subject = entry["subject"].as_str().unwrap();
        let from = entry["from"].as_str().unwrap();
        let to = entry["to"].as_str().unwrap();
        let allowed = entry["expect"].as_str().unwrap() == "allowed";

        let ok = match subject {
            "goal" => {
                let from = GoalStatus::parse(from).unwrap();
                let to = GoalStatus::parse(to).unwrap();
                transition_goal(from, to).is_ok()
            }
            "commitment" => {
                let from = CommitmentStatus::parse(from).unwrap();
                let to = CommitmentStatus::parse(to).unwrap();
                transition_commitment(from, to).is_ok()
            }
            other => panic!("{name}: unknown subject {other:?}"),
        };
        assert_eq!(ok, allowed, "{name}: {subject} {from} -> {to}");
    }
}

/// A goal reaches `fulfilled`, its transition history is append-only, and the
/// terminal row cannot be reopened through the facade.
#[tokio::test]
async fn a_fulfilled_goal_is_a_terminal_row_and_its_history_is_appended() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;

    facade
        .apply(BeingOp::AddGoal {
            description: "ship Phase 4".into(),
            priority: 0.9,
            origin: GoalOrigin::User,
        })
        .await
        .unwrap();
    let goal = *facade.goals().keys().next().unwrap();

    facade
        .apply(BeingOp::TransitionGoal {
            id: goal,
            from: None,
            to: GoalStatus::Fulfilled,
            reason: "gate is green".into(),
        })
        .await
        .unwrap();

    assert_eq!(
        h.scalar("SELECT count(*) FROM goal_transitions").await,
        2,
        "creation plus one transition"
    );

    // Reopening is refused by the facade's terminal-row check.
    let err = facade
        .apply(BeingOp::TransitionGoal {
            id: goal,
            from: None,
            to: GoalStatus::Active,
            reason: "changed my mind".into(),
        })
        .await
        .unwrap_err();
    match err {
        BeingError::Invariant(v) => assert_eq!(v.code, InvariantCode::NoHistoryRewrite),
        other => panic!("expected a history-rewrite refusal, got {other:?}"),
    }
    assert_eq!(
        h.scalar("SELECT count(*) FROM goal_transitions").await,
        2,
        "a refused transition appends nothing"
    );

    // The corpus form, carrying the terminal `from`, is refused by the guard
    // before the facade ever sees it.
    let err = facade
        .apply(BeingOp::TransitionGoal {
            id: goal,
            from: Some(GoalStatus::Fulfilled),
            to: GoalStatus::Active,
            reason: "and again".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invariant_violation");
    assert!(!h.records_of(codes::BEING_INVARIANT_VIOLATION).is_empty());

    h.shutdown().await;
}

/// The migration's trigger aborts a direct rewrite of a terminal row, so a caller
/// that bypasses the facade still cannot erase history.
#[tokio::test]
async fn the_terminal_row_trigger_aborts_a_raw_update() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;

    facade
        .apply(BeingOp::AddGoal {
            description: "write the plan".into(),
            priority: 0.5,
            origin: GoalOrigin::Being,
        })
        .await
        .unwrap();
    let goal = *facade.goals().keys().next().unwrap();
    let id = mm_core::ulid_string(&goal);

    facade
        .apply(BeingOp::TransitionGoal {
            id: goal,
            from: None,
            to: GoalStatus::Abandoned,
            reason: "superseded by a better plan".into(),
        })
        .await
        .unwrap();

    let raw = mm_core::Tabular::execute(
        &h.store,
        "UPDATE goals SET status = 'active' WHERE id = ?",
        vec![id.into()],
    )
    .await;
    assert!(raw.is_err(), "the trigger must abort a terminal rewrite");

    assert_eq!(
        h.query("SELECT status FROM goals")
            .await
            .first()
            .and_then(|row| row["status"].as_str())
            .unwrap(),
        "abandoned"
    );
    h.shutdown().await;
}

/// A commitment has its own lifecycle: it is discharged or withdrawn, never
/// reopened, and reaching a terminal status records the instant it did.
#[tokio::test]
async fn a_commitment_is_fulfilled_once_and_never_revoked_afterwards() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;

    facade
        .apply(BeingOp::AddGoal {
            description: "land the being substrate".into(),
            priority: 0.8,
            origin: GoalOrigin::User,
        })
        .await
        .unwrap();
    let goal = *facade.goals().keys().next().unwrap();

    facade
        .apply(BeingOp::AddCommitment {
            goal_id: Some(goal),
            made_to: user(1),
            description: "report by Friday".into(),
        })
        .await
        .unwrap();
    let commitment = *facade.commitments().keys().next().unwrap();

    facade
        .apply(BeingOp::TransitionCommitment {
            id: commitment,
            from: None,
            to: CommitmentStatus::Fulfilled,
            reason: "reported".into(),
        })
        .await
        .unwrap();

    let terminal = h
        .query("SELECT status, terminal_ulid FROM commitments")
        .await;
    let row = terminal.first().unwrap();
    assert_eq!(row["status"].as_str().unwrap(), "fulfilled");
    assert!(
        !row["terminal_ulid"].is_null(),
        "reaching a terminal status records when it happened"
    );

    let err = facade
        .apply(BeingOp::TransitionCommitment {
            id: commitment,
            from: None,
            to: CommitmentStatus::Revoked,
            reason: "withdrawn".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invariant_violation");
    assert_eq!(
        h.scalar("SELECT count(*) FROM commitment_transitions")
            .await,
        2,
        "creation plus one transition"
    );
    h.shutdown().await;
}

/// A transition that names a goal the being does not hold is a lookup failure, not
/// a silent no-op.
#[tokio::test]
async fn transitioning_an_unknown_goal_is_an_error() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;
    let err = facade
        .apply(BeingOp::TransitionGoal {
            id: common::ulid(999),
            from: None,
            to: GoalStatus::Fulfilled,
            reason: "no such goal".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "not_found");
    h.shutdown().await;
}
