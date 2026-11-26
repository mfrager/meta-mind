//! `replay.rs` — the record is a fold, and folding it twice gives the same state.
//!
//! Two properties:
//!
//! * **Across independent runs.** Driving the same ingested inputs (fixed claim
//!   ULIDs, fixed propositions) through two fresh stores yields a byte-identical
//!   `/epistemic` graph, the same contradiction pairs, and the same cascade. The
//!   contradiction and dependency ids are functions of their inputs, so nothing in
//!   the projection depends on wall-clock time.
//! * **From the recorded transitions.** Folding `epistemic_transitions` in the
//!   order the store keeps them reproduces exactly the statuses the claims hold,
//!   each transition has exactly one PROV activity, and re-mirroring is idempotent.
//!
//! `/provenance` transition ULIDs are minted per write, so cross-run byte-identity
//! is asserted for `/epistemic` (deterministic by construction) and stability —
//! not cross-run equality — for `/provenance`.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::Harness;
use mm_core::Ulid;
use mm_epistemic::{
    rdf, Claim, ClaimKind, DepKind, EpistemicStatus, EvidenceKind, Justification,
    JustificationGraph, Proposition, EPISTEMIC_GRAPH, PROVENANCE_GRAPH,
};

/// A claim on its own subject/predicate, so only the deliberate conflict conflicts.
fn claim(n: u128, predicate: &str, object: &str, status: EpistemicStatus) -> Claim {
    Claim::new(
        Harness::id(n),
        ClaimKind::Fact,
        Proposition::literal(
            "https://metamind.dev/data/pipeline",
            &format!("https://metamind.dev/ontology#{predicate}"),
            object,
        )
        .unwrap(),
        status,
        0.7,
    )
    .unwrap()
}

/// The same sequence of writes every time: an observation, two inferences resting
/// on it, a conflict, and a retraction of the root.
async fn drive(harness: &Harness) -> Vec<Ulid> {
    let mut observed = claim(1, "deployment", "green", EpistemicStatus::Observed);
    observed.evidence = vec![Harness::id(1001)];
    harness.engine.ingest(&observed, &[]).await.unwrap();

    for (n, predicate) in [(2u128, "derivedOne"), (3, "derivedTwo")] {
        let inference = claim(n, predicate, "yes", EpistemicStatus::Inferred);
        harness.engine.ingest(&inference, &[]).await.unwrap();
    }

    // Two reports about the same predicate, disagreeing: exactly one conflict.
    for (n, object) in [(4u128, "amber"), (5, "crimson")] {
        let report = claim(n, "state", object, EpistemicStatus::Reported);
        harness.engine.ingest(&report, &[]).await.unwrap();
    }
    let created = harness.engine.detect_contradictions().await.unwrap();
    assert_eq!(created.len(), 1, "one conflict, not a cross product");

    // 2 rests on 1, 3 rests on 2.
    for (consequent, antecedent) in [(2u128, 1u128), (3, 2)] {
        harness
            .engine
            .store()
            .insert_justification(
                &Justification::new(
                    Harness::id(consequent),
                    vec![Harness::id(antecedent)],
                    DepKind::Derivation,
                    0.5,
                )
                .unwrap(),
            )
            .await
            .unwrap();
    }

    // Evidence lifts one report, which is a recorded transition.
    let document = Harness::evidence(1040, EvidenceKind::Document);
    harness
        .engine
        .verify(&Harness::id(4), &document, EpistemicStatus::Inferred)
        .await
        .unwrap();

    harness
        .engine
        .invalidate(&Harness::id(1), "source retracted")
        .await
        .unwrap()
}

#[tokio::test]
async fn two_runs_of_the_same_inputs_produce_the_same_epistemic_state() {
    let first = Harness::new().await;
    let first_cascade = drive(&first).await;
    let second = Harness::new().await;
    let second_cascade = drive(&second).await;

    assert_eq!(first_cascade, second_cascade);
    assert_eq!(
        first_cascade,
        vec![Harness::id(1), Harness::id(2), Harness::id(3)]
    );

    let a = first.engine.snapshot().await.unwrap();
    let b = second.engine.snapshot().await.unwrap();
    assert_eq!(
        rdf::quads_hash(&a.to_quads()),
        rdf::quads_hash(&b.to_quads()),
        "the /epistemic projection must not depend on wall-clock time"
    );

    // The named graph agrees, read back through the store.
    let rows_a = first.graph.triples(EPISTEMIC_GRAPH).await.unwrap();
    let rows_b = second.graph.triples(EPISTEMIC_GRAPH).await.unwrap();
    let triples_a = <Vec<mm_epistemic::EpistemicTriple> as rdf::FromRdf>::from_rows(&rows_a);
    let triples_b = <Vec<mm_epistemic::EpistemicTriple> as rdf::FromRdf>::from_rows(&rows_b);
    assert_eq!(rdf::triples_hash(&triples_a), rdf::triples_hash(&triples_b));

    // Contradictions are a function of the pair, so they repeat too.
    let contradictions_a: BTreeSet<(Ulid, Ulid)> = a
        .contradictions
        .iter()
        .map(|c| (c.claim_a.min(c.claim_b), c.claim_a.max(c.claim_b)))
        .collect();
    let contradictions_b: BTreeSet<(Ulid, Ulid)> = b
        .contradictions
        .iter()
        .map(|c| (c.claim_a.min(c.claim_b), c.claim_a.max(c.claim_b)))
        .collect();
    assert_eq!(contradictions_a, contradictions_b);
    assert_eq!(
        contradictions_a,
        BTreeSet::from([(Harness::id(4), Harness::id(5))])
    );

    first.shutdown().await;
    second.shutdown().await;
}

#[tokio::test]
async fn folding_the_transitions_reproduces_the_stored_statuses() {
    let harness = Harness::new().await;
    drive(&harness).await;

    let snapshot = harness.engine.snapshot().await.unwrap();
    let transitions = snapshot.transitions.clone();
    assert!(!transitions.is_empty());

    // Fold the recorded transitions: each subject changed at most once in this
    // sequence, so the fold is order-independent.
    let mut replayed: BTreeMap<String, &'static str> = BTreeMap::new();
    for transition in &transitions {
        if let Some(to) = transition.to_status {
            replayed.insert(mm_core::ulid_string(&transition.subject), to.as_str());
        }
    }
    for (subject, status) in &replayed {
        let claim = snapshot
            .claims
            .iter()
            .find(|c| mm_core::ulid_string(&c.id) == *subject)
            .expect("a transition names a stored claim");
        assert_eq!(
            claim.status.as_str(),
            *status,
            "replay of {subject} disagrees with the stored status"
        );
    }

    // Exactly one PROV activity per transition.
    let rows = harness.graph.triples(PROVENANCE_GRAPH).await.unwrap();
    let activities: BTreeSet<String> = rows
        .iter()
        .filter(|row| {
            row.get("p").and_then(|v| v.as_str()) == Some("http://www.w3.org/ns/prov#Activity")
        })
        .filter_map(|row| row.get("s").and_then(|v| v.as_str()).map(String::from))
        .collect();
    assert_eq!(activities.len(), transitions.len());

    // The provenance mirror is stable across repeated reads and re-writes.
    let before = rdf::quads_hash(&snapshot.provenance_to_quads());
    let again = harness.engine.snapshot().await.unwrap();
    assert_eq!(before, rdf::quads_hash(&again.provenance_to_quads()));

    harness.engine.mirror().await.unwrap();
    let after = rdf::quads_hash(
        &harness
            .engine
            .snapshot()
            .await
            .unwrap()
            .provenance_to_quads(),
    );
    assert_eq!(before, after, "re-mirroring must not change the record");

    // The cascade is recomputable from the stored dependencies.
    let edges = harness.engine.store().list_justifications().await.unwrap();
    let graph = JustificationGraph::from_edges(edges).unwrap();
    let recomputed = graph.cascade(&Harness::id(1)).all();
    assert_eq!(
        recomputed,
        vec![Harness::id(1), Harness::id(2), Harness::id(3)],
        "a cascade must be a function of the stored edges"
    );

    harness.shutdown().await;
}
