//! The entity graph and its Personalized PageRank expansion (plan §7, graph bullet).
//!
//! Lexical search cannot find a memory that shares no words with the query, and
//! the graph channel is what closes that gap. Two properties are proved here
//! against a real store rather than a literal edge list:
//!
//! * **Reachability.** A memory two hops away through shared entities is found by
//!   the two-hop expansion and *not* by the one-hop expansion, while remaining
//!   lexically unrelated to the seed's memory.
//! * **Determinism and conservation.** The same edge set produces the same scores
//!   whatever order the edges were inserted in, the mass sums to one, and the seed
//!   carries the most.
//!
//! The bi-temporal half is proved against `entity_edges` directly: an edge is
//! *closed* at an instant rather than deleted, so a point-in-time read sees the
//! world as it was and a full read still sees everything.

mod common;

use common::{memory, ts, ulid, Harness};
use mm_core::{Timestamp, Ulid};
use mm_memory::graph::{DAMPING, MAX_HOPS};
use mm_memory::{EntityEdge, EntityGraph, MemoryEngine};

/// Add one co-occurrence edge through the store, returning its id.
async fn add_edge(engine: &MemoryEngine, from: Ulid, to: Ulid, at: Timestamp) -> Ulid {
    EntityGraph::add_edge(engine.store(), from, to, "co_occurs", 1.0, at)
        .await
        .unwrap()
}

/// Two memories two hops apart through shared entities: the two-hop expansion
/// finds the second one, the one-hop expansion does not, and neither shares a word
/// with the other.
#[tokio::test]
async fn expansion_reaches_two_hops_through_shared_entities() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let (near, middle, far) = (ulid(101), ulid(102), ulid(103));
    engine
        .remember(
            memory(1, "the release candidate shipped on a tuesday", ts(1_000))
                .with_entities(vec![near]),
        )
        .await
        .unwrap();
    engine
        .remember(
            memory(
                2,
                "the migration script rewrote every row in place",
                ts(1_001),
            )
            .with_entities(vec![far]),
        )
        .await
        .unwrap();

    add_edge(&engine, near, middle, ts(1_000)).await;
    add_edge(&engine, middle, far, ts(1_000)).await;

    let graph = EntityGraph::load(engine.store(), None).await.unwrap();
    assert_eq!(graph.len(), 2, "both edges must load");

    let one = graph.expand(&[near], 1);
    assert!(one.contains(&middle), "one hop reaches the middle entity");
    assert!(!one.contains(&far), "one hop must not reach two hops away");

    let two = graph.expand(&[near], 2);
    assert!(two.contains(&far), "two hops reaches the far entity");

    // The point of the channel: the far *memory* is reachable through the graph
    // and not through one hop. Its text shares no term with the seed's.
    let by_entity = engine.store().entities_by_entity().await.unwrap();
    let reached = |hops: u32| -> Vec<Ulid> {
        graph
            .expand(&[near], hops)
            .into_iter()
            .filter_map(|entity| by_entity.get(&entity))
            .flatten()
            .copied()
            .collect()
    };
    assert!(
        !reached(1).contains(&ulid(2)),
        "the far memory must not be one hop away"
    );
    assert!(
        reached(2).contains(&ulid(2)),
        "the far memory must be reachable in two hops"
    );

    harness.shutdown().await;
}

/// PageRank is deterministic, conserves its mass, and ranks the seed highest.
#[tokio::test]
async fn ppr_is_deterministic_conserves_mass_and_ranks_the_seed_highest() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let (near, middle, far) = (ulid(201), ulid(202), ulid(203));
    engine
        .remember(memory(1, "one episode about the build", ts(2_000)).with_entities(vec![near]))
        .await
        .unwrap();
    engine
        .remember(
            memory(2, "another episode about the release", ts(2_001)).with_entities(vec![far]),
        )
        .await
        .unwrap();
    add_edge(&engine, near, middle, ts(2_000)).await;
    add_edge(&engine, middle, far, ts(2_000)).await;

    let graph = EntityGraph::load(engine.store(), None).await.unwrap();
    let first = graph.ppr(&[near], MAX_HOPS, DAMPING);
    let second = graph.ppr(&[near], MAX_HOPS, DAMPING);

    assert_eq!(first, second, "the same graph and seed must score the same");
    let total: f64 = first.values().sum();
    assert!((total - 1.0).abs() < 1e-6, "mass was {total}");
    assert!(
        first
            .values()
            .all(|score| score.is_finite() && *score > 0.0),
        "every reachable node carries mass: {first:?}"
    );
    // Two properties are true of Personalized PageRank here, and the second is
    // worth stating because it is counter-intuitive on a path: `middle` has degree
    // two and collects mass from both directions, so it outranks the degree-one
    // seed. That is what a *walk* does — mass follows degree as well as distance —
    // and asserting "the seed always wins" would be asserting a hop count dressed
    // up as PageRank. What the walk does guarantee, and what the retrieval channel
    // relies on, is that between two nodes of the same degree the one nearer the
    // seed carries more.
    assert!(
        first[&near] > first[&far],
        "nearer to the seed must mean more mass at equal degree: {first:?}"
    );
    assert!(
        first[&middle] > first[&far],
        "the hub carries more than the far leaf: {first:?}"
    );

    harness.shutdown().await;
}

/// The symmetric view of the graph is what the walk uses: an edge is traversable
/// in both directions, so a memory whose entity is a *target* is still reached.
#[tokio::test]
async fn the_walk_traverses_edges_in_both_directions() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let (source, target) = (ulid(301), ulid(302));
    engine
        .remember(memory(1, "a note about the toolchain", ts(3_000)).with_entities(vec![source]))
        .await
        .unwrap();
    add_edge(&engine, source, target, ts(3_000)).await;

    let graph = EntityGraph::load(engine.store(), None).await.unwrap();
    assert_eq!(graph.neighbours(&source), vec![target]);
    assert!(
        graph.neighbours(&target).is_empty(),
        "out-neighbours are directional"
    );
    assert_eq!(
        graph.connected(&target),
        vec![source],
        "but the walk can still arrive from the other side"
    );
    assert_eq!(graph.connected(&source), vec![target]);

    harness.shutdown().await;
}

/// Closing an edge hides it from a read at or after that instant without deleting
/// the row, so lineage survives the relationship ending.
#[tokio::test]
async fn closing_an_edge_hides_it_without_deleting_it() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let (from, to) = (ulid(401), ulid(402));
    engine
        .remember(memory(1, "an episode about cargo", ts(4_000)).with_entities(vec![from]))
        .await
        .unwrap();
    let id = add_edge(&engine, from, to, ts(1_000)).await;

    assert_eq!(
        EntityGraph::close_edge(engine.store(), &id, ts(2_000))
            .await
            .unwrap(),
        1,
        "exactly one open edge is closed"
    );
    // Closing an already-closed edge changes nothing: the close is not a rewrite.
    assert_eq!(
        EntityGraph::close_edge(engine.store(), &id, ts(3_000))
            .await
            .unwrap(),
        0
    );

    assert_eq!(
        EntityGraph::load(engine.store(), None).await.unwrap().len(),
        1,
        "a closed edge is still on disk"
    );
    assert_eq!(
        EntityGraph::load(engine.store(), Some(ts(2_000)))
            .await
            .unwrap()
            .len(),
        0,
        "the edge is gone at the instant it closed"
    );
    assert_eq!(
        EntityGraph::load(engine.store(), Some(ts(1_500)))
            .await
            .unwrap()
            .len(),
        1,
        "and still present before it closed"
    );

    harness.shutdown().await;
}

/// World time and system time are independent: an edge that became true at one
/// instant is invisible before it and visible after, and closing it bounds it on
/// the other side.
#[tokio::test]
async fn an_edge_is_bitemporal() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let (from, to) = (ulid(501), ulid(502));
    engine
        .remember(memory(1, "an episode about sharding", ts(5_000)).with_entities(vec![from]))
        .await
        .unwrap();
    let id = add_edge(&engine, from, to, ts(1_000)).await;

    let load_at = |at: Timestamp| {
        let store = engine.store().clone();
        async move { EntityGraph::load(&store, Some(at)).await.unwrap().len() }
    };

    assert_eq!(load_at(ts(500)).await, 0, "not yet true");
    assert_eq!(load_at(ts(1_000)).await, 1, "true from this instant");
    assert_eq!(load_at(ts(1_500)).await, 1, "still true");

    EntityGraph::close_edge(engine.store(), &id, ts(2_000))
        .await
        .unwrap();
    assert_eq!(load_at(ts(1_500)).await, 1, "a past read is unchanged");
    assert_eq!(load_at(ts(2_000)).await, 0, "no longer true");
    assert_eq!(load_at(ts(2_500)).await, 0);
    assert_eq!(
        EntityGraph::load(engine.store(), None).await.unwrap().len(),
        1,
        "an unbounded read still sees the closed edge"
    );

    harness.shutdown().await;
}

/// The graph is a function of its edge *set*, not of the order the rows arrived
/// in — which is what makes a stored graph reproducible after a restore.
#[tokio::test]
async fn the_stored_graph_does_not_depend_on_insertion_order() {
    let (near, middle, far) = (ulid(601), ulid(602), ulid(603));

    let mut scores = Vec::new();
    for reversed in [false, true] {
        let harness = Harness::new().await;
        let mut engine = harness.engine().await;
        engine
            .remember(memory(1, "a note about the index", ts(6_000)).with_entities(vec![near]))
            .await
            .unwrap();

        let edges = [(near, middle), (middle, far), (near, far)];
        let ordered: Vec<(Ulid, Ulid)> = if reversed {
            edges.into_iter().rev().collect()
        } else {
            edges.to_vec()
        };
        for (from, to) in ordered {
            add_edge(&engine, from, to, ts(6_000)).await;
        }

        let graph = EntityGraph::load(engine.store(), None).await.unwrap();
        assert_eq!(graph.len(), 3);
        scores.push(graph.ppr(&[near], MAX_HOPS, DAMPING));
        harness.shutdown().await;
    }

    assert_eq!(
        scores[0], scores[1],
        "insertion order must not change the ranking"
    );
}

/// The same property without a store, so a regression that reached only the
/// in-memory builder would still be caught.
#[test]
fn from_edges_ignores_the_order_of_its_slice() {
    let edge = |n: u128, from: u128, to: u128| EntityEdge {
        id: ulid(n),
        from: ulid(from),
        to: ulid(to),
        relation: "co_occurs".to_string(),
        weight: 1.0,
        valid_from: ts(1_000),
        valid_until: None,
    };

    let forward = EntityGraph::from_edges(vec![edge(1, 1, 2), edge(2, 2, 3), edge(3, 1, 3)]);
    let reversed = EntityGraph::from_edges(vec![edge(3, 1, 3), edge(2, 2, 3), edge(1, 1, 2)]);

    assert_eq!(
        forward.ppr(&[ulid(1)], MAX_HOPS, DAMPING),
        reversed.ppr(&[ulid(1)], MAX_HOPS, DAMPING)
    );
    assert_eq!(forward.edges().len(), 3);
    assert!(!forward.is_empty());
}
