//! `contradiction.rs` — every seeded conflicting pair becomes exactly one
//! explicit record naming both sides.

mod common;

use common::Harness;
use mm_epistemic::{ContradictionStatus, EpistemicStatus};

#[tokio::test]
async fn each_seeded_pair_becomes_exactly_one_record() {
    let harness = Harness::new().await;
    // Three disjoint pairs: same subject and relation, different object.
    let seeded: [(u128, &str, u128, &str); 3] = [
        (1, "green", 2, "red"),
        (3, "up", 4, "down"),
        (5, "empty", 6, "full"),
    ];
    for (a, object_a, b, object_b) in seeded {
        let mut first = Harness::claim(a, object_a, EpistemicStatus::Reported);
        first.proposition.subject =
            mm_epistemic::proposition::iri_node(&format!("https://metamind.dev/data/node-{a}"))
                .unwrap();
        let mut second = Harness::claim(b, object_b, EpistemicStatus::Reported);
        second.proposition.subject =
            mm_epistemic::proposition::iri_node(&format!("https://metamind.dev/data/node-{a}"))
                .unwrap();
        harness.engine.ingest(&first, &[]).await.unwrap();
        harness.engine.ingest(&second, &[]).await.unwrap();
    }

    let created = harness.engine.detect_contradictions().await.unwrap();
    assert_eq!(created.len(), seeded.len(), "one record per seeded pair");
    for contradiction in &created {
        assert!(contradiction.names_both_sides(), "{contradiction:?}");
        assert_eq!(contradiction.status, ContradictionStatus::Open);
        assert!(
            contradiction.reason.contains("asserts both"),
            "{}",
            contradiction.reason
        );
    }

    // The records are stored, and re-detecting does not duplicate them.
    let stored = harness.store.list_contradictions().await.unwrap();
    assert_eq!(stored.len(), seeded.len());
    let again = harness.engine.detect_contradictions().await.unwrap();
    assert!(
        again.is_empty(),
        "a re-detection must not duplicate a record"
    );

    harness.shutdown().await;
}

#[tokio::test]
async fn agreeing_claims_and_different_relations_do_not_conflict() {
    let harness = Harness::new().await;
    let first = Harness::claim(1, "green", EpistemicStatus::Reported);
    let second = Harness::claim(2, "green", EpistemicStatus::Reported);
    harness.engine.ingest(&first, &[]).await.unwrap();
    harness.engine.ingest(&second, &[]).await.unwrap();
    let mut other = Harness::claim(3, "red", EpistemicStatus::Reported);
    other.proposition.predicate =
        mm_epistemic::proposition::iri_node("https://metamind.dev/ontology#owner").unwrap();
    harness.engine.ingest(&other, &[]).await.unwrap();

    let created = harness.engine.detect_contradictions().await.unwrap();
    assert!(created.is_empty(), "{created:?}");
    harness.shutdown().await;
}
