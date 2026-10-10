//! Forgetting: retention decay archives, and three protections refuse.
//!
//! The invariant this phase exists to protect is that the being never loses what
//! it must keep. These tests build one record of each protected shape — an explicit
//! do-not-forget mark, the developmental ledger, and a memory serving an open
//! commitment — and prove that a forget cycle leaves all three alone while
//! archiving the one record it is allowed to.

mod common;

use mm_core::{Param, Tabular, Timestamp};
use mm_log::codes;
use mm_memory::forgetting::retention_of;
use mm_memory::WEEK_NS;
use mm_memory::{Memory, MemoryError, MemoryKind};

use common::Harness;

/// The instant the cycles run at.
fn now() -> Timestamp {
    common::ts(1_700_000_000)
}

/// Three weeks before `now`, which is three half-lives: a record this old and
/// this unimportant has a retention of exactly `0.5^3 * 0.5 = 0.0625`, below the
/// default floor of `0.15` and comfortably above zero.
fn stale_instant() -> Timestamp {
    common::ts(1_700_000_000 - 3 * 604_800)
}

/// A stale, unimportant, never-accessed memory under a known id.
fn stale(n: u128, content: &str) -> Memory {
    common::memory(n, content, stale_instant()).with_importance(0.0)
}

#[tokio::test]
async fn an_unprotected_developmental_memory_is_refused_before_it_is_stored() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let developmental = Memory::draft(
        common::ulid(30),
        MemoryKind::Developmental,
        "I exist",
        stale_instant(),
    )
    .unwrap();
    let error = engine.remember(developmental).await.unwrap_err();
    match error {
        MemoryError::Validation { field, .. } => assert_eq!(field, "protected"),
        other => panic!("expected a protected-field validation failure, got {other:?}"),
    }
    assert_eq!(
        harness.scalar("SELECT count(*) FROM memories").await,
        0,
        "a refused memory must not reach the store"
    );

    // With the mark it is stored, and it is unforgettable.
    let protected = Memory::draft(
        common::ulid(31),
        MemoryKind::Developmental,
        "I exist",
        stale_instant(),
    )
    .unwrap()
    .with_protected(true);
    engine.remember(protected).await.unwrap();
    assert_eq!(harness.scalar("SELECT count(*) FROM memories").await, 1);

    harness.shutdown().await;
}

#[tokio::test]
async fn a_forget_cycle_archives_only_what_it_may_and_refuses_the_rest() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let old_id = common::ulid(10);
    let protected_id = common::ulid(20);
    let developmental_id = common::ulid(30);
    let committed_id = common::ulid(40);

    engine
        .remember(stale(10, "an unimportant old detail"))
        .await
        .unwrap();
    engine
        .remember(stale(20, "a detail the being refuses to forget").with_protected(true))
        .await
        .unwrap();
    engine
        .remember(
            Memory::draft(
                common::ulid(30),
                MemoryKind::Developmental,
                "I was created and I know it",
                stale_instant(),
            )
            .unwrap()
            .with_protected(true)
            .with_importance(0.0),
        )
        .await
        .unwrap();
    engine
        .remember(stale(40, "a detail an open promise depends on"))
        .await
        .unwrap();

    // An open commitment, so the fourth memory is still needed.
    let commitment = common::ulid(909);
    Tabular::execute(
        &harness.store,
        "INSERT INTO commitments (id, goal_id, made_to, description, status, deadline_ulid, \
         created_ulid, terminal_ulid) VALUES (?, NULL, ?, 'finish phase 5', 'active', NULL, ?, NULL)",
        vec![
            Param::Text(mm_core::ulid_string(&commitment)),
            Param::Text(mm_core::ulid_string(&common::ulid(7))),
            Param::Text(mm_core::ulid_string(&common::ulid(8))),
        ],
    )
    .await
    .unwrap();
    engine
        .serves_commitment(committed_id, commitment)
        .await
        .unwrap();

    // --- dry run -----------------------------------------------------------
    let dry = engine.forget_cycle(now(), true).await.unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.considered, 4);
    assert_eq!(
        dry.archived,
        vec![old_id],
        "only the unprotected record is due"
    );
    assert_eq!(dry.refused.len(), 3);
    assert_eq!(
        harness
            .scalar("SELECT count(*) FROM memories WHERE status = 'archived'")
            .await,
        0,
        "a dry run must write nothing"
    );
    let dry_retention = &harness
        .query(&format!(
            "SELECT retention FROM memories WHERE id = '{}'",
            mm_core::ulid_string(&old_id)
        ))
        .await[0]["retention"];
    assert!(
        !dry_retention.is_f64(),
        "a dry run must not even record a score, found {dry_retention}"
    );

    let blocking: Vec<&str> = dry
        .refused
        .iter()
        .map(|refusal| refusal.blocking_reference.as_str())
        .collect();
    assert!(blocking.contains(&"protected"));
    assert!(blocking.contains(&"developmental"));
    assert!(
        blocking
            .iter()
            .any(|reason| reason.starts_with("commitment:")),
        "{blocking:?}"
    );

    // --- the real cycle ----------------------------------------------------
    let real = engine.forget_cycle(now(), false).await.unwrap();
    assert!(!real.dry_run);
    assert_eq!(real.archived, vec![old_id]);
    assert_eq!(real.refused.len(), 3);
    assert_eq!(
        harness
            .scalar("SELECT count(*) FROM memories WHERE status = 'archived'")
            .await,
        1
    );
    assert_eq!(
        harness
            .scalar(&format!(
                "SELECT count(*) FROM memories WHERE id = '{}' AND status = 'archived'",
                mm_core::ulid_string(&old_id)
            ))
            .await,
        1
    );
    for spared in [protected_id, developmental_id, committed_id] {
        assert_eq!(
            harness
                .scalar(&format!(
                    "SELECT count(*) FROM memories WHERE id = '{}' AND status = 'active'",
                    mm_core::ulid_string(&spared)
                ))
                .await,
            1,
            "{} must remain active",
            mm_core::ulid_string(&spared)
        );
    }

    // --- idempotence -------------------------------------------------------
    let second = engine.forget_cycle(now(), false).await.unwrap();
    assert!(
        second.archived.is_empty(),
        "an archived record is not reconsidered"
    );
    assert_eq!(second.refused.len(), 3);
    assert_eq!(
        harness
            .scalar("SELECT count(*) FROM memories WHERE status = 'archived'")
            .await,
        1
    );

    harness.shutdown().await;
}

#[tokio::test]
async fn every_refusal_and_archival_is_logged_with_its_reason() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let old_id = common::ulid(10);
    let protected_id = common::ulid(20);
    engine.remember(stale(10, "an old note")).await.unwrap();
    engine
        .remember(stale(20, "a note worth keeping").with_protected(true))
        .await
        .unwrap();

    let report = engine.forget_cycle(now(), false).await.unwrap();
    assert_eq!(report.archived, vec![old_id]);
    assert_eq!(report.refused.len(), 1);
    assert_eq!(
        report.refused[0].memory_id,
        mm_core::ulid_string(&protected_id)
    );
    assert_eq!(report.refused[0].attempted_action, "archive");
    assert_eq!(report.refused[0].kind, "episodic");
    assert!(!report.refused[0].retention.is_empty());

    let refusals = harness.records_of(codes::MEMORY_PROTECTED_REFUSAL);
    assert_eq!(refusals.len(), 1, "each refusal is its own record");
    let fields = &refusals[0]["fields"];
    assert_eq!(fields["memory_id"], mm_core::ulid_string(&protected_id));
    assert_eq!(fields["kind"], "episodic");
    assert_eq!(fields["attempted_action"], "archive");
    assert_eq!(fields["blocking_reference"], "protected");

    let forgets = harness.records_of(codes::MEMORY_FORGET);
    let skips: Vec<_> = forgets
        .iter()
        .filter(|record| record["fields"]["action"] == "skip")
        .collect();
    let archives: Vec<_> = forgets
        .iter()
        .filter(|record| record["fields"]["action"] == "archive")
        .collect();
    assert_eq!(skips.len(), 1, "a refusal is also a forget decision");
    assert_eq!(
        skips[0]["fields"]["memory_id"],
        mm_core::ulid_string(&protected_id)
    );
    assert_eq!(skips[0]["fields"]["reason"], "protected");
    assert_eq!(archives.len(), 1);
    assert_eq!(
        archives[0]["fields"]["memory_id"],
        mm_core::ulid_string(&old_id)
    );

    harness.shutdown().await;
}

#[tokio::test]
async fn the_recorded_retention_is_the_one_the_curve_computes() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let protected_id = common::ulid(20);
    engine
        .remember(stale(20, "a note that must outlive the cycle").with_protected(true))
        .await
        .unwrap();
    engine.forget_cycle(now(), false).await.unwrap();

    let stored = harness
        .query(&format!(
            "SELECT retention FROM memories WHERE id = '{}'",
            mm_core::ulid_string(&protected_id)
        ))
        .await[0]["retention"]
        .as_f64()
        .expect("the cycle records a score for every active record");

    let expected = retention_of(stale_instant(), now(), WEEK_NS, 0.0, 0);
    assert!(
        (stored - expected).abs() < 1e-9,
        "stored {stored} but the curve says {expected}"
    );
    // The value is genuinely below the floor, which is why the refusal had to
    // happen: this is a real protection, not an accident of the arithmetic.
    assert!(stored < mm_memory::forgetting::DEFAULT_THRESHOLD);
    assert!(stored > 0.0, "the curve must not have underflowed here");

    harness.shutdown().await;
}
