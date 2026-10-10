//! The `/memory` mirror round-trips canonically and actually bites (plan §4.2, §7,
//! §8).
//!
//! The mirror is a *projection*, not a second authority: it is rewritten from the
//! tables on every mutation, so a triple that is not in the snapshot cannot survive
//! in the graph. These tests write through the engine, read `/memory` back, and
//! require the two to agree by value — and they validate the result against the
//! SHACL shapes, on a clean graph and on a deliberately broken one.

mod common;

use common::{ts, ulid, Harness};
use mm_memory::rdf::{
    mirror, quads_hash, snapshot, triples_hash, FromRdf, MemoryTriple, MEMORY_GRAPH,
};
use mm_memory::{Memory, MemoryKind};

/// A memory with a real source, so both branches of the source rule are exercised.
fn sourced(n: u128, content: &str) -> Memory {
    common::memory(n, content, ts(1_700_000_000)).with_source(ulid(900 + n))
}

/// The mirror of a live engine rebuilds exactly the quads it holds, and its hashes
/// agree from either side.
#[tokio::test]
async fn a_live_snapshot_round_trips_through_the_memory_graph() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    engine
        .remember(sourced(
            1,
            "the workspace build broke after the toolchain moved",
        ))
        .await
        .unwrap();
    engine
        .remember(sourced(
            2,
            "pinning the toolchain fixed the workspace build",
        ))
        .await
        .unwrap();
    engine
        .remember(
            common::memory(3, "the user prefers dark mode", ts(1_700_000_000))
                .with_entities(vec![ulid(700)]),
        )
        .await
        .unwrap();

    let snap = snapshot(engine.store()).await.unwrap();
    assert_eq!(snap.memories.len(), 3);

    let written = mirror(h.graph.handle(), &snap).await.unwrap();
    let expected = snap.to_quads();
    assert_eq!(written, expected.len());

    let rows = h.graph.triples(MEMORY_GRAPH).await.unwrap();
    let rebuilt = <Vec<MemoryTriple> as FromRdf>::from_rows(&rows);

    let mut source: Vec<MemoryTriple> = expected.iter().map(MemoryTriple::from_quad).collect();
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

/// Insertion order does not change the canonical hash of `/memory`.
#[tokio::test]
async fn insertion_order_does_not_change_the_canonical_hash() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    engine
        .remember(sourced(1, "one memory with a source"))
        .await
        .unwrap();
    engine
        .remember(sourced(2, "another memory with a source"))
        .await
        .unwrap();

    let quads = snapshot(engine.store()).await.unwrap().to_quads();
    assert!(quads.len() > 10, "each memory is many triples");

    h.graph.handle().clear_graph(MEMORY_GRAPH).await.unwrap();
    for quad in &quads {
        h.graph.handle().insert(quad.clone()).await.unwrap();
    }
    let forward = h.graph.canonical_hash(MEMORY_GRAPH).await.unwrap();

    h.graph.handle().clear_graph(MEMORY_GRAPH).await.unwrap();
    for quad in quads.iter().rev() {
        h.graph.handle().insert(quad.clone()).await.unwrap();
    }
    let reversed = h.graph.canonical_hash(MEMORY_GRAPH).await.unwrap();

    assert_eq!(
        forward, reversed,
        "a canonical hash is independent of insertion order"
    );

    h.shutdown().await;
}

/// Every memory node names exactly one source, and a memory with no recorded
/// source names `mm:unknownSource` explicitly rather than omitting the predicate.
#[tokio::test]
async fn every_memory_names_exactly_one_source() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    engine
        .remember(sourced(1, "a memory with a real source"))
        .await
        .unwrap();
    engine
        .remember(common::memory(
            2,
            "a memory with no recorded source",
            ts(1_700_000_000),
        ))
        .await
        .unwrap();

    let rows = h.graph.triples(MEMORY_GRAPH).await.unwrap();
    let source_predicate = format!("{}source", mm_core::iri::MM);
    let sourced_iri = mm_core::iri::data(&ulid(901)).into_string();
    let unknown = format!("{}unknownSource", mm_core::iri::MM);
    let node = mm_core::iri::data(&ulid(2)).into_string();

    // Exactly one `mm:source` per memory node, so the shape's maxCount 1 is met.
    let per_node: std::collections::BTreeMap<String, usize> = rows
        .iter()
        .filter(|row| row["p"].as_str() == Some(source_predicate.as_str()))
        .fold(std::collections::BTreeMap::new(), |mut acc, row| {
            if let Some(subject) = row["s"].as_str() {
                *acc.entry(subject.to_string()).or_default() += 1;
            }
            acc
        });
    assert_eq!(per_node.len(), 2, "both memories must name a source");
    assert!(
        per_node.values().all(|count| *count == 1),
        "a memory names exactly one source: {per_node:?}"
    );

    let source_of = |subject: &str| -> Option<String> {
        rows.iter()
            .find(|row| {
                row["s"].as_str() == Some(subject)
                    && row["p"].as_str() == Some(source_predicate.as_str())
            })
            .and_then(|row| row["o"].as_str().map(str::to_string))
    };
    assert_eq!(
        source_of(&mm_core::iri::data(&ulid(1)).into_string()).as_deref(),
        Some(sourced_iri.as_str())
    );
    assert_eq!(
        source_of(&node).as_deref(),
        Some(unknown.as_str()),
        "a memory with no source must say so explicitly"
    );

    h.shutdown().await;
}

/// A clean mirror conforms to every `/memory` shape.
#[tokio::test]
async fn a_clean_mirror_validates_with_zero_violations() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    engine
        .remember(sourced(1, "the toolchain pin fixed the workspace build"))
        .await
        .unwrap();
    engine
        .remember(
            common::memory(2, "the user prefers dark mode", ts(1_700_000_000))
                .with_entities(vec![ulid(700)]),
        )
        .await
        .unwrap();
    // A developmental memory is part of the immutable ledger and must be protected
    // — it exercises the one shape that exists for that rule alone.
    engine
        .remember(
            Memory::draft(
                ulid(3),
                MemoryKind::Developmental,
                "I keep what I learned",
                ts(1),
            )
            .unwrap()
            .with_protected(true),
        )
        .await
        .unwrap();

    let shapes = h.cfg.memory_shapes_file();
    assert!(shapes.exists(), "{} must exist", shapes.display());
    let report = h.graph.validate_with(MEMORY_GRAPH, &shapes).await.unwrap();
    assert!(
        report.conforms,
        "a clean mirror must conform, got: {:?}",
        report.violations
    );
    assert_eq!(report.violations.len(), 0);

    h.shutdown().await;
}

/// The shapes actually bite: a bare `mm:Memory` node with no `mm:memoryKind` is a
/// violation, so a mirror that lost a predicate cannot pass validation.
#[tokio::test]
async fn a_broken_mirror_is_reported_as_a_violation() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    engine
        .remember(sourced(1, "a well formed memory"))
        .await
        .unwrap();

    let shapes = h.cfg.memory_shapes_file();
    let clean = h.graph.validate_with(MEMORY_GRAPH, &shapes).await.unwrap();
    assert!(clean.conforms, "{:?}", clean.violations);

    // One extra triple: a memory node typed as `mm:Memory` with none of the
    // predicates the shape requires.
    let bare = format!("{}data{}", mm_core::iri::DATA, "01h00000000000000000000000");
    h.graph
        .handle()
        .insert_turtle(
            MEMORY_GRAPH,
            &format!("<{bare}> a <{}Memory> .", mm_core::iri::MM),
        )
        .await
        .unwrap();

    let report = h.graph.validate_with(MEMORY_GRAPH, &shapes).await.unwrap();
    assert!(
        !report.conforms,
        "an under-specified memory node must be a violation"
    );
    assert!(
        !report.violations.is_empty(),
        "the shapes must report what was missing"
    );

    h.shutdown().await;
}

/// Archived memories stay in the mirror: forgetting moves a record out of
/// retrieval without erasing its lineage.
#[tokio::test]
async fn an_archived_memory_keeps_its_place_in_the_mirror() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    engine
        .remember(sourced(1, "a memory that will be archived"))
        .await
        .unwrap();

    // Archive far in the future so the record is unambiguously below threshold.
    let archived = engine.forget_cycle(ts(1_900_000_000), false).await.unwrap();
    assert_eq!(
        archived.archived,
        vec![ulid(1)],
        "the record must be archived, not deleted"
    );
    assert_eq!(
        h.scalar("SELECT count(*) FROM memories").await,
        1,
        "archiving must not remove the row"
    );

    let snap = snapshot(engine.store()).await.unwrap();
    assert_eq!(snap.memories.len(), 1);
    assert_eq!(snap.memories[0].status, mm_memory::RecordStatus::Archived);
    assert_eq!(
        snap.to_quads().len(),
        mirror(h.graph.handle(), &snap).await.unwrap()
    );

    let shapes = h.cfg.memory_shapes_file();
    let report = h.graph.validate_with(MEMORY_GRAPH, &shapes).await.unwrap();
    assert!(
        report.conforms,
        "an archived memory must still satisfy the shapes: {:?}",
        report.violations
    );

    h.shutdown().await;
}
