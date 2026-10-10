//! Hybrid retrieval: channel accounting, determinism, filters, and the graph walk
//! that reaches a memory sharing no words with the query (plan §4.3, §7, §9).
//!
//! Every assertion here is about the *published* shape of a recall — the five
//! channel parts, the ULID tie-break, the access row, and the `memory.recall`
//! record. A retrieval whose reasons cannot be audited is a retrieval nobody can
//! debug later, so the parts are pinned numerically rather than trusted.

mod common;

use common::{ts, ulid, Harness};
use mm_core::Timestamp;
use mm_memory::{EntityGraph, Memory, MemoryFilter, MemoryKind, RecallQuery, Tier, CHANNELS};

/// The instant every recall in this file is measured at, so recency is a constant.
fn now() -> Timestamp {
    ts(1_700_000_000)
}

/// Round a float so a snapshot pins magnitudes without pinning float noise.
fn round(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

/// The lexically most relevant memory ranks first.
#[tokio::test]
async fn the_best_lexical_match_ranks_first() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    for (n, content) in [
        (
            1u128,
            "the workspace build failed because the toolchain was not pinned",
        ),
        (2, "the user prefers dark mode in the editor"),
        (3, "a cat sat on the keyboard during the release"),
        (4, "coffee is best brewed at ninety degrees"),
    ] {
        engine
            .remember(common::memory(n, content, now()))
            .await
            .unwrap();
    }

    let hits = engine
        .recall(RecallQuery::new("workspace build toolchain", 4, now()))
        .await
        .unwrap();
    assert!(!hits.is_empty());
    assert_eq!(
        hits[0].memory.id,
        ulid(1),
        "the memory about the build must rank first"
    );

    h.shutdown().await;
}

/// Every hit carries five channel parts, in the plan's order, and the parts sum to
/// the score *before* the confidence gate.
#[tokio::test]
async fn every_hit_accounts_for_every_channel() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    for (n, content) in [
        (
            1u128,
            "pinned the toolchain after the workspace build broke",
        ),
        (2, "the release notes were drafted on friday"),
        (3, "the build cache is rebuilt every morning"),
    ] {
        engine
            .remember(common::memory(n, content, now()))
            .await
            .unwrap();
    }

    let hits = engine
        .recall(RecallQuery::new("workspace build toolchain", 3, now()))
        .await
        .unwrap();
    assert_eq!(hits.len(), 3);

    for hit in &hits {
        assert_eq!(
            hit.parts.len(),
            CHANNELS.len(),
            "a hit must account for all five channels"
        );
        for (part, expected) in hit.parts.iter().zip(CHANNELS) {
            assert_eq!(
                part.channel, expected,
                "parts must be in the plan's channel order"
            );
            assert!(
                (part.contribution - part.raw * part.weight).abs() < 1e-9,
                "{} contributed {} but raw*weight is {}",
                part.channel,
                part.contribution,
                part.raw * part.weight
            );
            assert!(
                part.contribution >= 0.0,
                "{} contributed a negative amount",
                part.channel
            );
        }
        let gate = 0.5 + 0.5 * f64::from(hit.memory.confidence);
        let ungated: f64 = hit.parts.iter().map(|part| part.contribution).sum();
        assert!(
            (ungated - hit.score / gate).abs() < 1e-9,
            "parts must sum to the ungated score: {ungated} vs {}",
            hit.score / gate
        );
        // Every channel's weight is the one the plan pins.
        let expected_weights = [0.35, 0.30, 0.20, 0.10, 0.05];
        for (part, weight) in hit.parts.iter().zip(expected_weights) {
            assert!(
                (part.weight - weight).abs() < 1e-12,
                "{} has weight {}",
                part.channel,
                part.weight
            );
        }
    }

    h.shutdown().await;
}

/// The channel weights, the ULID tie-break, and the `parts` shape are pinned in a
/// snapshot so an accidental rebalance cannot slip through review.
#[tokio::test]
async fn the_channel_parts_are_pinned_in_a_snapshot() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    for (n, content) in [
        (1u128, "the workspace build broke after the toolchain moved"),
        (2, "a different memory entirely about brewing coffee"),
    ] {
        engine
            .remember(common::memory(n, content, now()))
            .await
            .unwrap();
    }

    let hits = engine
        .recall(RecallQuery::new("workspace build", 2, now()))
        .await
        .unwrap();
    assert_eq!(hits[0].memory.id, ulid(1), "the build memory must win");

    let parts: Vec<serde_json::Value> = hits[0]
        .parts
        .iter()
        .map(|part| {
            serde_json::json!({
                "channel": part.channel.as_str(),
                "raw": round(part.raw),
                "weight": round(part.weight),
                "contribution": round(part.contribution),
            })
        })
        .collect();
    insta::assert_json_snapshot!("retrieval_parts", parts);

    h.shutdown().await;
}

/// `k` bounds the result, and equal scores are ordered by ascending ULID.
#[tokio::test]
async fn ties_break_on_ulid_and_k_bounds_the_result() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    // Two memories with identical content, recorded at the same instant and with
    // the same confidence and importance — so every channel scores them equally.
    // They differ only in `validity.from`, which is what keeps the second from
    // being refused as an exact duplicate.
    let shared = "the build was pinned to a known toolchain";
    let left = Memory::new(
        ulid(1),
        MemoryKind::Episodic,
        shared,
        mm_memory::TimeInterval::open(ts(1_600_000_000)),
        0.5,
        0.5,
        ulid(1),
        now(),
    )
    .unwrap();
    let right = Memory::new(
        ulid(2),
        MemoryKind::Episodic,
        shared,
        mm_memory::TimeInterval::open(ts(1_600_000_100)),
        0.5,
        0.5,
        ulid(2),
        now(),
    )
    .unwrap();
    engine.remember(left).await.unwrap();
    engine.remember(right).await.unwrap();

    let hits = engine
        .recall(RecallQuery::new(shared, 5, now()))
        .await
        .unwrap();
    assert_eq!(hits.len(), 2, "both tied memories must be reachable");
    assert!(
        (hits[0].score - hits[1].score).abs() < 1e-12,
        "the two memories are constructed to tie: {} vs {}",
        hits[0].score,
        hits[1].score
    );
    assert_eq!(
        hits[0].memory.id,
        ulid(1),
        "a tie must be broken by the smaller ULID"
    );
    assert_eq!(hits[1].memory.id, ulid(2));

    // The tie-break is a property of the whole ordering, not just of the first two.
    for pair in hits.windows(2) {
        assert!(pair[0].score >= pair[1].score - 1e-12);
        if (pair[0].score - pair[1].score).abs() < 1e-12 {
            assert!(pair[0].memory.id < pair[1].memory.id);
        }
    }

    let one = engine
        .recall(RecallQuery::new(shared, 1, now()))
        .await
        .unwrap();
    assert_eq!(one.len(), 1, "k must bound the result");

    h.shutdown().await;
}

/// Two identical recalls return the same ids in the same order.
#[tokio::test]
async fn recall_is_deterministic() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    for (n, content) in [
        (1u128, "the gate failed on the being invariants"),
        (2, "the gate failed on the memory shapes"),
        (3, "the gate passed after the fix"),
    ] {
        engine
            .remember(common::memory(n, content, now()))
            .await
            .unwrap();
    }

    let ids =
        |hits: &[mm_memory::RecallHit]| hits.iter().map(|hit| hit.memory.id).collect::<Vec<_>>();
    let first = ids(&engine
        .recall(RecallQuery::new("the gate failed", 3, now()))
        .await
        .unwrap());
    let second = ids(&engine
        .recall(RecallQuery::new("the gate failed", 3, now()))
        .await
        .unwrap());
    assert_eq!(first, second, "a recall must replay exactly");
    assert_eq!(first.len(), 3);

    h.shutdown().await;
}

/// A recall records an access per hit and emits a `memory.recall` record naming
/// every channel's candidate count.
#[tokio::test]
async fn a_recall_records_an_access_and_a_log_record() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    engine
        .remember(common::memory(1, "the toolchain was pinned", now()))
        .await
        .unwrap();

    let hits = engine
        .recall(RecallQuery::new("toolchain", 1, now()))
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);

    assert_eq!(
        h.scalar("SELECT count(*) FROM memory_access").await,
        1,
        "every hit must be recorded as an access"
    );
    let rows = h
        .query("SELECT memory_id, query_hash, score, trace_id FROM memory_access")
        .await;
    assert_eq!(
        rows[0]["memory_id"].as_str(),
        Some(mm_core::ulid_string(&ulid(1)).as_str())
    );
    assert!(rows[0]["query_hash"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
    assert!(rows[0]["trace_id"].as_str().is_some_and(|s| !s.is_empty()));

    let records = h.records_of(mm_log::codes::MEMORY_RECALL);
    assert_eq!(records.len(), 1, "one recall, one record");
    let fields = &records[0]["fields"];
    for field in [
        "query_hash",
        "k",
        "lexical_n",
        "vector_n",
        "graph_n",
        "fallback",
    ] {
        assert!(
            !fields[field].is_null(),
            "memory.recall must carry {field}: {fields}"
        );
    }
    assert_eq!(fields["k"].as_u64(), Some(1));
    assert!(fields["lexical_n"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(fields["fallback"].as_bool(), Some(false));
    assert_eq!(
        fields["result_ids"].as_array().map(Vec::len),
        Some(1),
        "the record must name the ids it returned"
    );
    assert_eq!(fields["scores"].as_array().map(Vec::len), Some(1));

    // The rerank record carries the weights that produced the score.
    let reranks = h.records_of(mm_log::codes::MEMORY_RERANK);
    assert_eq!(reranks.len(), 1);
    assert!(!reranks[0]["fields"]["weights"].is_null());
    assert_eq!(
        reranks[0]["fields"]["weights"]["lexical"].as_f64(),
        Some(0.35)
    );

    h.shutdown().await;
}

/// A kind filter excludes other kinds; a tier filter excludes other tiers.
#[tokio::test]
async fn kind_and_tier_filters_restrict_the_candidate_set() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    engine
        .remember(common::memory(1, "the build broke on monday", now()))
        .await
        .unwrap();
    engine
        .remember(
            Memory::draft(
                ulid(2),
                MemoryKind::Semantic,
                "builds break when unpinned",
                now(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    engine
        .remember(
            common::memory(3, "the build was pinned on tuesday", now()).with_tier(Tier::Archival),
        )
        .await
        .unwrap();

    let semantic = engine
        .recall(RecallQuery::new("build", 10, now()).with_kinds(vec![MemoryKind::Semantic]))
        .await
        .unwrap();
    assert!(!semantic.is_empty());
    assert!(
        semantic
            .iter()
            .all(|hit| hit.memory.kind == MemoryKind::Semantic),
        "a kind filter must exclude every other kind"
    );

    let episodic = engine
        .recall(RecallQuery::new("build", 10, now()).with_kinds(vec![MemoryKind::Episodic]))
        .await
        .unwrap();
    assert!(episodic
        .iter()
        .all(|hit| hit.memory.kind == MemoryKind::Episodic));

    // Exactly one memory was written at the archival tier, so a filter on that tier
    // reaches it and nothing else.
    let archival = engine
        .recall(RecallQuery::new("build", 10, now()).with_tiers(vec![Tier::Archival]))
        .await
        .unwrap();
    assert!(
        !archival.is_empty(),
        "the archival memory must be reachable"
    );
    assert!(
        archival.iter().all(|hit| hit.memory.tier == Tier::Archival),
        "a tier filter must exclude every other tier"
    );

    let recall_tier = engine
        .recall(RecallQuery::new("build", 10, now()).with_tiers(vec![Tier::Recall]))
        .await
        .unwrap();
    assert!(!recall_tier.is_empty());
    assert!(recall_tier
        .iter()
        .all(|hit| hit.memory.tier == Tier::Recall));
    assert!(
        !recall_tier
            .iter()
            .any(|hit| hit.memory.tier == Tier::Archival),
        "a recall-tier filter must exclude the archival memory"
    );

    h.shutdown().await;
}

/// The graph channel reaches a memory that shares no words with the query
/// (HippoRAG's expansion): B is found through the entity it shares with A.
#[tokio::test]
async fn the_graph_channel_reaches_a_memory_with_no_lexical_overlap() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    let shared_entity = ulid(500);
    let other_entity = ulid(501);

    // A: matches the query lexically and mentions the shared entity.
    engine
        .remember(
            common::memory(1, "the deployment pipeline was rate limited", now())
                .with_entities(vec![shared_entity]),
        )
        .await
        .unwrap();
    // B: shares no words with the query at all, but names the same entity.
    engine
        .remember(
            common::memory(2, "kubernetes ingress quota exhaustion", now())
                .with_entities(vec![shared_entity]),
        )
        .await
        .unwrap();
    // C: the control — unrelated words and no entity, so it can only be reached by
    // the recency and importance terms.
    engine
        .remember(common::memory(
            3,
            "a quiet afternoon with no incidents",
            now(),
        ))
        .await
        .unwrap();

    // One edge out of the shared entity, so the walk has somewhere to go.
    EntityGraph::add_edge(
        engine.store(),
        shared_entity,
        other_entity,
        "related",
        1.0,
        ts(1),
    )
    .await
    .unwrap();

    let hits = engine
        .recall(RecallQuery::new("deployment pipeline", 10, now()))
        .await
        .unwrap();

    let find = |id: mm_core::Ulid| hits.iter().find(|hit| hit.memory.id == id);
    let b = find(ulid(2)).expect("the entity-linked memory must be retrieved");
    let c = find(ulid(3));

    assert!(
        b.parts
            .iter()
            .any(|part| part.channel == mm_memory::Channel::Graph && part.raw > 0.0),
        "the graph channel must be what reached it"
    );
    match c {
        Some(control) => {
            let control_graph = control
                .parts
                .iter()
                .find(|part| part.channel == mm_memory::Channel::Graph)
                .map(|part| part.contribution)
                .unwrap_or(0.0);
            assert_eq!(
                control_graph, 0.0,
                "a memory with no entities must earn nothing from the graph"
            );
            assert!(
                b.score > control.score,
                "the graph-linked memory ({}) must outrank the unlinked one ({})",
                b.score,
                control.score
            );
        }
        None => panic!("the control memory must still be a candidate"),
    }

    // The shared entity is also visible on the store side, which is what the walk
    // seeded from.
    let by_entity = engine.store().entities_by_entity().await.unwrap();
    assert_eq!(
        by_entity.get(&shared_entity).map(Vec::len),
        Some(2),
        "both memories must name the shared entity"
    );

    h.shutdown().await;
}

/// A query with no lexical match still returns ranked hits: the candidate set is
/// the whole store, not only what FTS5 matched, and the record reports the counts
/// the fallback flag is derived from.
#[tokio::test]
async fn a_query_with_no_lexical_match_still_ranks_the_candidate_set() {
    let h = Harness::new().await;
    let mut engine = h.engine().await;

    engine
        .remember(common::memory(1, "alpha bravo charlie", now()))
        .await
        .unwrap();

    let hits = engine
        .recall(RecallQuery::new("zulu yankee xray", 1, now()))
        .await
        .unwrap();
    assert_eq!(
        hits.len(),
        1,
        "the candidate set is the whole store, not only the lexical matches"
    );
    assert_eq!(hits[0].parts[0].raw, 0.0, "nothing matched lexically");

    // `fallback` is defined as "neither the lexical nor the vector channel produced
    // a single candidate". The record must report it from the counts rather than
    // assert it, so this test checks the definition rather than a constant.
    let records = h.records_of(mm_log::codes::MEMORY_RECALL);
    let fields = &records[0]["fields"];
    assert_eq!(fields["lexical_n"].as_u64(), Some(0));
    let lexical_n = fields["lexical_n"].as_u64().unwrap();
    let vector_n = fields["vector_n"].as_u64().unwrap();
    assert_eq!(
        fields["fallback"].as_bool(),
        Some(lexical_n + vector_n == 0),
        "fallback must be derived from the channel counts: {fields}"
    );

    // The index is what the vector channel consults, and it holds every active
    // memory, so a reopened engine answers the same query.
    assert_eq!(engine.index().len(), 1);
    assert_eq!(engine.store().count(&MemoryFilter::all()).await.unwrap(), 1);
    h.shutdown().await;
}

/// With nothing to search, a recall returns nothing and *does* set the fallback
/// flag — the one case where both channels are genuinely empty.
#[tokio::test]
async fn a_recall_over_an_empty_store_reports_the_fallback() {
    let h = Harness::new().await;
    let engine = h.engine().await;
    assert_eq!(engine.index().len(), 0);

    let hits = engine
        .recall(RecallQuery::new("anything at all", 5, now()))
        .await
        .unwrap();
    assert!(hits.is_empty(), "an empty store has nothing to rank");

    let records = h.records_of(mm_log::codes::MEMORY_RECALL);
    assert_eq!(records.len(), 1, "the attempt is still recorded");
    let fields = &records[0]["fields"];
    assert_eq!(fields["lexical_n"].as_u64(), Some(0));
    assert_eq!(fields["vector_n"].as_u64(), Some(0));
    assert_eq!(fields["graph_n"].as_u64(), Some(0));
    assert_eq!(fields["fallback"].as_bool(), Some(true));
    assert_eq!(fields["result_ids"].as_array().map(Vec::len), Some(0));
    assert_eq!(h.scalar("SELECT count(*) FROM memory_access").await, 0);

    h.shutdown().await;
}
