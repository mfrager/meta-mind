//! Belief promotion: the evidence gate (plan §4.1, §7, §8).
//!
//! The failure this file exists to prevent is *silent certainty*: a guess about
//! the user drifting into something the being will act on as though it were
//! observed. So the claim is measured twice — once on the status lattice, and once
//! end to end through the guarded facade — and a denied promotion must always
//! leave the belief exactly where it was.

mod common;

use common::{fixture, jsonl, ulid, user, Harness};
use mm_being::{
    BeingError, BeingOp, Belief, BlockKind, CoreBlock, EpistemicStatus, InvariantCode, UserModel,
};
use mm_log::codes;
use proptest::prelude::*;

/// A `human` block to hang a user model on.
fn human_block() -> CoreBlock {
    CoreBlock::new(BlockKind::Human, "human", "the primary user", 1500, ulid(1)).unwrap()
}

/// The statuses, in the order the lattice declares them.
const STATUSES: [EpistemicStatus; 10] = [
    EpistemicStatus::Observed,
    EpistemicStatus::Verified,
    EpistemicStatus::Reported,
    EpistemicStatus::Inferred,
    EpistemicStatus::Assumed,
    EpistemicStatus::Hypothetical,
    EpistemicStatus::Predicted,
    EpistemicStatus::Simulated,
    EpistemicStatus::Fictional,
    EpistemicStatus::Unknown,
];

/// The fixture's `evidence` array, as ids.
fn evidence_of(fx: &serde_json::Value) -> Vec<String> {
    fx.get("evidence")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Every fixture line lands where it says: allowed with evidence, denied without —
/// and a denial never moves the status.
#[test]
fn the_status_lattice_matches_every_fixture_line() {
    let fixtures = jsonl(&fixture("bench/being/belief_promotion.jsonl"));
    assert!(
        !fixtures.is_empty(),
        "the promotion fixtures must not be empty"
    );

    for (index, fx) in fixtures.iter().enumerate() {
        let name = fx["name"].as_str().unwrap();
        let from = EpistemicStatus::parse(fx["from"].as_str().unwrap()).unwrap();
        let to = EpistemicStatus::parse(fx["to"].as_str().unwrap()).unwrap();
        let evidence = evidence_of(fx);
        let expect = fx["expect"].as_str().unwrap();
        let proposition = format!("prop-{index}");

        let mut model = UserModel::new(user(1), human_block());
        model.insert(
            proposition.clone(),
            Belief::new(proposition.clone(), from, 0.4),
        );

        // Copying the status out of the result ends the borrow `promote` holds on
        // `model`, so the "it did not move" assertion below is legal.
        let outcome = model
            .promote(&proposition, to, &evidence)
            .map(|belief| belief.epistemic_status);

        match expect {
            "allowed" => {
                assert_eq!(
                    outcome.unwrap_or_else(|e| panic!("{name}: expected allowed: {e}")),
                    to
                );
                assert_eq!(
                    model.get(&proposition).unwrap().epistemic_status,
                    to,
                    "{name}: an allowed promotion must take effect"
                );
            }
            "denied" => {
                let err = outcome.err().unwrap_or_else(|| {
                    panic!("{name}: expected a denial, but the promotion succeeded")
                });
                assert_eq!(err.kind(), "promotion_denied", "{name}");
                assert_eq!(
                    model.get(&proposition).unwrap().epistemic_status,
                    from,
                    "{name}: a denied promotion must never move the status"
                );
            }
            other => panic!("{name}: unknown `expect` value {other:?}"),
        }
    }
}

/// The same gate, end to end: the facade refuses `ASSUMED -> OBSERVED` without an
/// observation, writes nothing, and audits the refusal; with evidence it commits.
#[tokio::test]
async fn the_facade_never_promotes_an_assumption_without_an_observation() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;

    facade
        .apply(BeingOp::SetBelief {
            user_id: user(1),
            proposition: "the user prefers Rust".into(),
            status: EpistemicStatus::Assumed,
            confidence: 0.4,
            evidence: vec![],
        })
        .await
        .unwrap();

    let err = facade
        .apply(BeingOp::PromoteBelief {
            user_id: user(1),
            proposition: "the user prefers Rust".into(),
            to: EpistemicStatus::Observed,
            evidence: vec![],
        })
        .await
        .unwrap_err();
    // `OBSERVED` with no observation is refused by the guard, before the belief
    // gate is reached — the strongest place for it to be refused.
    match err {
        BeingError::Invariant(v) => assert_eq!(v.code, InvariantCode::NoAssumptionToObservation),
        other => panic!("expected the guard to refuse it, got {other:?}"),
    }
    assert!(
        h.audit_codes()
            .await
            .contains(&codes::BEING_INVARIANT_VIOLATION.to_string()),
        "the guard's refusal must be audited"
    );

    // The current (open) row still holds the assumption.
    let current = h
        .query("SELECT epistemic_status FROM user_beliefs WHERE valid_until_ulid IS NULL")
        .await;
    assert_eq!(current.len(), 1, "exactly one open belief row");
    assert_eq!(current[0]["epistemic_status"].as_str(), Some("ASSUMED"));

    // A status the guard allows but the evidence does not support — `VERIFIED`
    // needs two independent records — is refused by the belief gate itself, and
    // *that* refusal is the one `being.belief.promotion_denied` records.
    let err = facade
        .apply(BeingOp::PromoteBelief {
            user_id: user(1),
            proposition: "the user prefers Rust".into(),
            to: EpistemicStatus::Verified,
            evidence: vec!["ev-1".into()],
        })
        .await
        .unwrap_err();
    match err {
        BeingError::Promotion(denied) => {
            assert_eq!(denied.from, "ASSUMED");
            assert_eq!(denied.to, "VERIFIED");
        }
        other => panic!("expected a promotion denial, got {other:?}"),
    }
    assert!(
        h.audit_codes()
            .await
            .contains(&codes::BEING_BELIEF_PROMOTION_DENIED.to_string()),
        "a denied promotion must be audited"
    );

    // With an observation record the same promotion is allowed.
    facade
        .apply(BeingOp::PromoteBelief {
            user_id: user(1),
            proposition: "the user prefers Rust".into(),
            to: EpistemicStatus::Observed,
            evidence: vec!["ev-1".into()],
        })
        .await
        .unwrap();

    let current = h
        .query("SELECT epistemic_status, evidence_json FROM user_beliefs WHERE valid_until_ulid IS NULL")
        .await;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["epistemic_status"].as_str(), Some("OBSERVED"));
    assert_eq!(current[0]["evidence_json"].as_str(), Some(r#"["ev-1"]"#));
    assert!(
        h.audit_codes()
            .await
            .contains(&codes::BEING_BELIEF_UPDATE.to_string()),
        "a committed promotion must be audited"
    );

    h.shutdown().await;
}

/// An `OBSERVED` belief with no evidence record is exactly what `being verify`
/// looks for; the gate must refuse to create one.
#[tokio::test]
async fn the_database_never_holds_an_unbacked_observation() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;

    facade
        .apply(BeingOp::SetBelief {
            user_id: user(2),
            proposition: "the user lives in Berlin".into(),
            status: EpistemicStatus::Reported,
            confidence: 0.6,
            evidence: vec![],
        })
        .await
        .unwrap();

    for evidence in [vec![], vec!["ev-1".into()]] {
        let _ = facade
            .apply(BeingOp::PromoteBelief {
                user_id: user(2),
                proposition: "the user lives in Berlin".into(),
                to: EpistemicStatus::Observed,
                evidence: evidence.clone(),
            })
            .await;
    }

    assert_eq!(
        h.scalar(
            "SELECT count(*) FROM user_beliefs WHERE epistemic_status = 'OBSERVED' \
             AND (evidence_json = '[]' OR evidence_json IS NULL)"
        )
        .await,
        0,
        "an OBSERVED belief must always carry evidence"
    );
    h.shutdown().await;
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `promote` never coerces: it either returns the new status or leaves the old
    /// one in place, for any pair of statuses and any amount of evidence.
    #[test]
    fn promotion_never_coerces_a_status(
        from_ix in 0usize..STATUSES.len(),
        to_ix in 0usize..STATUSES.len(),
        evidence_len in 0usize..4,
    ) {
        let from = STATUSES[from_ix];
        let to = STATUSES[to_ix];
        let evidence: Vec<String> = (0..evidence_len).map(|i| format!("ev-{i}")).collect();

        let mut model = UserModel::new(user(9), human_block());
        model.insert("p".into(), Belief::new("p", from, 0.5));

        let outcome = model.promote("p", to, &evidence).map(|b| b.epistemic_status);
        let settled = model.get("p").unwrap().epistemic_status;

        match &outcome {
            Ok(status) => prop_assert_eq!(*status, to),
            Err(err) => {
                prop_assert_eq!(err.kind(), "promotion_denied");
                prop_assert_eq!(settled, from, "a refused promotion must not move the status");
            }
        }
        prop_assert_eq!(settled, outcome.unwrap_or(from));
    }
}
