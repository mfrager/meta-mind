//! `world_separation.rs` — `/world` has one door, and only `OBSERVED`/`VERIFIED`
//! walk through it.
//!
//! The test drives the engine the way the CLI does (ingest, promote, mirror) and
//! then asks the graph itself, with the plan's own SPARQL, what `/world` holds. A
//! claim the barrier refused must be absent from `/world` and present in
//! `/epistemic`, which is the whole of the separation.

mod common;

use common::Harness;
use mm_epistemic::{
    EpistemicStatus, EvidenceKind, ValidationBarrier, EPISTEMIC_GRAPH, WORLD_GRAPH,
};
use mm_store_graph::GraphStore;

/// The plan §8 query: every subject in `/world` that names a status.
fn status_query() -> String {
    format!(
        "SELECT ?s ?st WHERE {{ GRAPH <{}> {{ ?s <{}status> ?st }} }}",
        mm_core::iri::graph(WORLD_GRAPH),
        mm_core::iri::MM,
    )
}

/// Every `?st` the query returns.
async fn world_statuses(graph: &GraphStore) -> Vec<String> {
    let rows = graph.sparql(WORLD_GRAPH, &status_query()).await.unwrap();
    let mut statuses: Vec<String> = rows
        .iter()
        .filter_map(|row| row.get("st").cloned())
        .map(|value| match value {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        })
        .collect();
    statuses.sort();
    statuses
}

#[tokio::test]
async fn the_world_holds_only_observed_and_verified() {
    let harness = Harness::new().await;

    // One observation with an evidence row: admissible.
    let mut observed = Harness::claim(1, "green", EpistemicStatus::Observed);
    let evidence = Harness::evidence(1001, EvidenceKind::Observation);
    observed.evidence = vec![evidence.id];
    harness
        .engine
        .ingest(&observed, std::slice::from_ref(&evidence))
        .await
        .unwrap();

    // A report and an assumption with evidence: not admissible, however supported.
    let reported = Harness::claim(2, "amber", EpistemicStatus::Reported);
    harness.engine.ingest(&reported, &[]).await.unwrap();
    let mut assumed = Harness::claim(3, "red", EpistemicStatus::Assumed);
    assumed.evidence = vec![Harness::id(4)];
    harness.engine.ingest(&assumed, &[]).await.unwrap();

    let statuses = world_statuses(&harness.graph).await;
    assert_eq!(statuses, vec!["OBSERVED".to_string()]);
    assert!(statuses.iter().all(|s| s == "OBSERVED" || s == "VERIFIED"));

    // The refused claims are still in `/epistemic`, so nothing was silently lost.
    let epistemic = harness.graph.triples(EPISTEMIC_GRAPH).await.unwrap();
    let rendered = serde_json::to_string(&epistemic).unwrap();
    assert!(rendered.contains(&mm_core::ulid_string(&Harness::id(3))));

    harness.shutdown().await;
}

#[tokio::test]
async fn an_assumed_claim_is_refused_admission_even_with_evidence() {
    let mut assumed = Harness::claim(3, "red", EpistemicStatus::Assumed);
    assumed.evidence = vec![Harness::id(4)];
    let refusal = ValidationBarrier::admit(&assumed).unwrap_err();
    assert_eq!(refusal.status, EpistemicStatus::Assumed);

    // Evidence of the strongest kind does not change that: an assumption is not
    // an observation record.
    let authoritative = Harness::evidence(5, EvidenceKind::ExternalTool);
    assumed.evidence = vec![authoritative.id];
    assert!(ValidationBarrier::admit(&assumed).is_err());
}

#[tokio::test]
async fn a_promotion_into_the_world_adds_exactly_the_admitted_claim() {
    let harness = Harness::new().await;

    let claim = Harness::claim(1, "green", EpistemicStatus::Reported);
    harness.engine.ingest(&claim, &[]).await.unwrap();
    assert!(world_statuses(&harness.graph).await.is_empty());

    // An observation record is what makes the observation legal.
    let observation = Harness::evidence(9, EvidenceKind::Observation);
    harness
        .engine
        .verify(&Harness::id(1), &observation, EpistemicStatus::Observed)
        .await
        .unwrap();

    let statuses = world_statuses(&harness.graph).await;
    assert_eq!(statuses, vec!["OBSERVED".to_string()]);
    harness.shutdown().await;
}
