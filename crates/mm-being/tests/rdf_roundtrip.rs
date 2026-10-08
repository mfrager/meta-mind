//! The `/being` mirror round-trips (plan §4.3, §7, §8).
//!
//! The mirror is a *projection*, not a second authority: it is rewritten from the
//! in-memory snapshot on every commit, so a triple that is not in the snapshot
//! cannot survive in the graph. These tests pin that down from the outside — they
//! write through the facade, read `/being` back, and require the two to agree by
//! value, with a canonical hash that does not depend on insertion order.

mod common;

use common::Harness;
use mm_being::rdf::{mirror, quads_hash, require_identity, triples_hash};
use mm_being::{BeingOp, BeingSnapshot, BeingTriple, FromRdf, GoalOrigin, GoalStatus, BEING_GRAPH};

/// The mirror of a live facade's snapshot rebuilds exactly the quads it holds.
#[tokio::test]
async fn a_live_snapshot_round_trips_through_the_being_graph() {
    let h = Harness::new().await;
    let facade = h.facade().await;
    let snap = facade.snapshot();

    let written = mirror(h.graph.handle(), &snap).await.unwrap();
    let expected = snap.to_quads();
    assert_eq!(written, expected.len());

    let rows = h.graph.triples(BEING_GRAPH).await.unwrap();
    let rebuilt = <Vec<BeingTriple> as FromRdf>::from_rows(&rows);

    let mut source: Vec<BeingTriple> = expected.iter().map(BeingTriple::from_quad).collect();
    source.sort();

    assert_eq!(
        rebuilt, source,
        "the mirror must not lose or invent a triple"
    );
    assert_eq!(
        triples_hash(&rebuilt),
        quads_hash(&expected),
        "a read-back hash must equal the snapshot hash"
    );

    h.shutdown().await;
}

/// Insertion order does not change the canonical hash of `/being`.
#[tokio::test]
async fn insertion_order_does_not_change_the_canonical_hash() {
    let h = Harness::new().await;
    let facade = h.facade().await;
    let snap = facade.snapshot();
    let quads = snap.to_quads();
    assert!(quads.len() > 4, "the identity alone is several triples");

    h.graph.handle().clear_graph(BEING_GRAPH).await.unwrap();
    for quad in &quads {
        h.graph.handle().insert(quad.clone()).await.unwrap();
    }
    let forward = h.graph.canonical_hash(BEING_GRAPH).await.unwrap();

    h.graph.handle().clear_graph(BEING_GRAPH).await.unwrap();
    for quad in quads.iter().rev() {
        h.graph.handle().insert(quad.clone()).await.unwrap();
    }
    let reversed = h.graph.canonical_hash(BEING_GRAPH).await.unwrap();

    assert_eq!(
        forward, reversed,
        "a canonical hash is independent of insertion order"
    );
    h.shutdown().await;
}

/// State committed through the facade reaches `/being`, and a fresh mirror writes
/// exactly the snapshot's quads — the graph is a projection of the tables.
#[tokio::test]
async fn committed_state_is_projected_into_the_being_graph() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;

    facade
        .apply(BeingOp::AddGoal {
            description: "ship Phase 4".into(),
            priority: 0.9,
            origin: GoalOrigin::Being,
        })
        .await
        .unwrap();

    let goal_id = facade
        .goals()
        .keys()
        .next()
        .cloned()
        .expect("the goal was added");
    let goal_iri = mm_core::iri::data(&goal_id).into_string();

    let rows = h.graph.triples(BEING_GRAPH).await.unwrap();
    assert!(
        rows.iter().any(|row| {
            row["s"].as_str() == Some(goal_iri.as_str())
                && row["p"]
                    .as_str()
                    .is_some_and(|p| p.ends_with("#goalStatus"))
        }),
        "the committed goal must appear in /being"
    );

    // Re-mirroring writes exactly what the snapshot holds: nothing stale survives.
    let snap = facade.snapshot();
    let written = mirror(h.graph.handle(), &snap).await.unwrap();
    assert_eq!(written, snap.to_quads().len());

    h.shutdown().await;
}

/// A goal's status round-trips: after a transition the mirror carries the new one.
#[tokio::test]
async fn a_transitioned_goal_carries_its_new_status_in_the_mirror() {
    let h = Harness::new().await;
    let mut facade = h.facade().await;

    facade
        .apply(BeingOp::AddGoal {
            description: "answer the user's question".into(),
            priority: 0.5,
            origin: GoalOrigin::User,
        })
        .await
        .unwrap();
    let goal_id = facade.goals().keys().next().cloned().unwrap();

    // A live caller leaves `from` to the facade, which reads the stored row.
    facade
        .apply(BeingOp::TransitionGoal {
            id: goal_id,
            from: None,
            to: GoalStatus::Fulfilled,
            reason: "answered".into(),
        })
        .await
        .unwrap();

    let goal_iri = mm_core::iri::data(&goal_id).into_string();
    let rows = h.graph.triples(BEING_GRAPH).await.unwrap();
    // A plain `xsd:string` literal renders as a bare JSON string; a typed one (and
    // every numeric axis) renders as `{ "value": …, "datatype": … }`.
    let status = rows
        .iter()
        .find(|row| {
            row["s"].as_str() == Some(goal_iri.as_str())
                && row["p"]
                    .as_str()
                    .is_some_and(|p| p.ends_with("#goalStatus"))
        })
        .and_then(|row| match &row["o"] {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Object(map) => map
                .get("value")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            _ => None,
        });

    assert_eq!(
        status.as_deref(),
        Some(GoalStatus::Fulfilled.as_str()),
        "the mirror must show the status the transition reached"
    );

    h.shutdown().await;
}

/// A snapshot with no identity is refused: an empty graph would read as "this
/// being has no self", which is a different claim from "not yet created".
#[tokio::test]
async fn a_mirror_without_an_identity_is_refused() {
    let err = require_identity(&BeingSnapshot::default()).unwrap_err();
    assert_eq!(err.kind(), "config");
    assert!(BeingSnapshot::default().to_quads().is_empty());
}
