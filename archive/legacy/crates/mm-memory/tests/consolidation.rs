//! Consolidation: episodes → patterns → a summary tree, with the lineage intact.
//!
//! The phase's headline claim is a trade: consolidating a stream must *reduce* the
//! number of live memories while making every source still reconstructible. These
//! tests check both halves, and the property test checks that the reduction holds
//! for any number of episodes rather than for one lucky fixture.

mod common;

use std::collections::BTreeSet;

use mm_core::Timestamp;
use mm_log::codes;
use mm_memory::{Episode, TimeInterval};
use proptest::prelude::*;

use common::Harness;

/// The three topics the synthetic stream uses.
const TOPICS: [&str; 3] = ["build-breakage", "review-feedback", "toolchain-drift"];

/// What each topic's episodes say.
///
/// The three topics deliberately share almost no vocabulary. A cluster is
/// generalized from the topic cue, so episodes that all read alike would test the
/// near-duplicate merge instead of the generalization.
const UTTERANCES: [[&str; 3]; 3] = [
    [
        "the workspace build failed after the upgrade",
        "the build broke again on a clean checkout",
        "the build stayed broken until the pin was restored",
    ],
    [
        "a review asked for clearer naming in the diff",
        "the review wanted the duplicate branch removed",
        "the review pushed back on the split module",
    ],
    [
        "the pinned toolchain drifted from the manifest",
        "the compiler version moved under the lockfile",
        "an unpinned dependency pulled a newer edition",
    ],
];

/// Nine episodes: three topics, three episodes each.
fn stream() -> Vec<Episode> {
    let mut episodes = Vec::new();
    for (topic_index, topic) in TOPICS.iter().enumerate() {
        for (step, utterance) in UTTERANCES[topic_index].iter().enumerate() {
            episodes.push(common::episode(
                &format!("ep-{topic_index}-{step}"),
                topic,
                utterance,
                &[topic, "workspace"],
            ));
        }
    }
    episodes
}

/// The window that contains everything just ingested.
///
/// The upper bound is deliberately in the future: an episode is recorded at the
/// instant ingestion runs, so a window that ended "now" would exclude it.
fn window(now: Timestamp) -> TimeInterval {
    TimeInterval {
        from: Timestamp::from_epoch_seconds(now.seconds.saturating_sub(30 * 86_400)),
        until: Some(Timestamp::from_epoch_seconds(now.seconds + 3_600)),
    }
}

/// Every id a consolidation step names still exists, and every source resolves to
/// a target through `memory_consolidations`.
async fn assert_sources_resolve(harness: &Harness) {
    let steps = harness
        .query("SELECT target_id, source_ids FROM memory_consolidations ORDER BY id")
        .await;
    assert!(!steps.is_empty(), "a consolidation must record a step");
    for step in &steps {
        let target = step["target_id"].as_str().expect("a target id");
        assert_eq!(
            harness
                .scalar(&format!(
                    "SELECT count(*) FROM memories WHERE id = '{target}'"
                ))
                .await,
            1,
            "the target {target} must still exist"
        );
        let members: Vec<String> =
            serde_json::from_str(step["source_ids"].as_str().unwrap_or("[]")).unwrap();
        assert!(!members.is_empty(), "a step must name its sources");
        for source in &members {
            assert_eq!(
                harness
                    .scalar(&format!(
                        "SELECT count(*) FROM memories WHERE id = '{source}'"
                    ))
                    .await,
                1,
                "the source {source} was deleted; archiving must never delete"
            );
        }
    }
}

#[tokio::test]
async fn consolidating_a_stream_reduces_the_count_and_keeps_every_source() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let (_dir, path) = common::write_episodes(&stream());
    let ingested = engine
        .ingest_episodes(&path, &harness.logger)
        .await
        .unwrap();
    assert_eq!(ingested.len(), 9);
    assert_eq!(harness.scalar("SELECT count(*) FROM memories").await, 9);

    // Ingestion is keyed on the episode's stable `ref`, so a rerun adds nothing.
    let again = engine
        .ingest_episodes(&path, &harness.logger)
        .await
        .unwrap();
    assert_eq!(
        again, ingested,
        "the same references must resolve to the same ids"
    );
    assert_eq!(
        harness.scalar("SELECT count(*) FROM memories").await,
        9,
        "a rerun must not duplicate the stream"
    );

    let now = Timestamp::now();
    let outcome = engine.consolidate(window(now)).await.unwrap();

    // The reduction: 9 episodes became 3 patterns plus a summary.
    assert!(outcome.reduced(), "consolidation must reduce the count");
    assert!(outcome.after < outcome.before, "{outcome:?}");
    assert_eq!(outcome.before, 9);
    assert_eq!(outcome.generalized, 3, "one semantic memory per topic");
    assert_eq!(
        outcome.archived.len(),
        9 + outcome.merged,
        "every episode was folded away (plus any near-duplicate target)"
    );
    assert!(
        outcome.merged + outcome.generalized == 3,
        "a merge would fold two clusters, not add one: {outcome:?}"
    );
    assert_eq!(harness.scalar("SELECT count(*) FROM memories").await, 13);

    // Nothing was deleted: all nine sources are still rows, archived rather than
    // removed, and every one is reachable from its target.
    assert_eq!(
        harness
            .scalar("SELECT count(*) FROM memories WHERE status = 'archived'")
            .await,
        9,
        "archiving must leave the rows behind"
    );
    assert_eq!(
        harness
            .scalar("SELECT count(*) FROM memories WHERE status = 'active'")
            .await,
        outcome.after as i64,
    );
    assert!(outcome.provenance_closure_ok);
    assert_sources_resolve(&harness).await;

    for archived in &outcome.archived {
        let id = mm_core::ulid_string(archived);
        assert_eq!(
            harness
                .scalar(&format!(
                    "SELECT count(*) FROM memory_links \
                     WHERE from_id IN (SELECT target_id FROM memory_consolidations) \
                     AND to_id = '{id}' AND relation = 'consolidated_from'"
                ))
                .await,
            1,
            "the archived source {id} must have a consolidated_from link"
        );
    }

    // A second run over the same window finds nothing left to fold: the sources
    // are archived, so no cluster survives. Nothing is lost, whatever else moves.
    let before_second = harness.scalar("SELECT count(*) FROM memories").await;
    let second = engine.consolidate(window(now)).await.unwrap();
    assert_eq!(
        second.archived.len(),
        0,
        "an already-archived source must not be folded twice"
    );
    assert_eq!(second.generalized, 0, "no episodes remain to generalize");
    assert!(
        harness.scalar("SELECT count(*) FROM memories").await >= before_second,
        "a rerun must not delete anything"
    );

    harness.shutdown().await;
}

#[tokio::test]
async fn the_summary_tree_is_well_formed_and_communities_are_labelled() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;
    let (_dir, path) = common::write_episodes(&stream());
    engine
        .ingest_episodes(&path, &harness.logger)
        .await
        .unwrap();
    let outcome = engine.consolidate(window(Timestamp::now())).await.unwrap();

    let rows = harness
        .query("SELECT memory_id, parent_id, level, member_ids FROM memory_summaries ORDER BY id")
        .await;
    assert_eq!(rows.len(), outcome.summaries);
    assert!(!rows.is_empty(), "a tree must have at least one node");

    let mut level_one: Vec<String> = Vec::new();
    let mut roots = 0usize;
    for row in &rows {
        let level = row["level"].as_u64().expect("a level");
        assert!(
            level >= 1,
            "the leaves are the memories themselves, not tree nodes"
        );
        if level == 1 {
            level_one.push(row["memory_id"].as_str().unwrap_or_default().to_string());
        }
        let members: Vec<String> =
            serde_json::from_str(row["member_ids"].as_str().unwrap_or("[]")).unwrap();
        assert!(
            members.len() >= 2,
            "a summary node that covers fewer than two memories summarizes nothing new"
        );
        for member in &members {
            assert_eq!(
                harness
                    .scalar(&format!(
                        "SELECT count(*) FROM memories WHERE id = '{member}'"
                    ))
                    .await,
                1,
                "the summarized member {member} must exist"
            );
        }
        if row["parent_id"].is_null() {
            roots += 1;
        }
    }
    let expected_roots = if level_one.len() >= 2 {
        1
    } else {
        level_one.len()
    };
    assert_eq!(
        roots, expected_roots,
        "a root exists only when there is more than one child to cover"
    );

    // Communities are attached to their cluster's own target, so no extra memory
    // row is created for one.
    let communities = harness
        .query("SELECT memory_id, label, member_ids FROM memory_communities ORDER BY id")
        .await;
    assert_eq!(communities.len(), outcome.communities);
    assert_eq!(
        communities.len() + outcome.merged,
        3,
        "one community per surviving topic cluster"
    );
    for community in &communities {
        let label = community["label"].as_str().expect("a label");
        assert!(
            TOPICS.contains(&label),
            "unexpected community label {label}"
        );
        let members: Vec<String> =
            serde_json::from_str(community["member_ids"].as_str().unwrap_or("[]")).unwrap();
        assert!(members.len() >= 2);
        for member in &members {
            assert_eq!(
                harness
                    .scalar(&format!(
                        "SELECT count(*) FROM memories WHERE id = '{member}'"
                    ))
                    .await,
                1
            );
        }
    }

    // One `memory.consolidate` record per cluster, one `memory.summary.build` per
    // node.
    assert_eq!(
        harness.records_of(codes::MEMORY_CONSOLIDATE).len(),
        outcome.generalized + outcome.merged
    );
    assert_eq!(
        harness.records_of(codes::MEMORY_SUMMARY_BUILD).len(),
        outcome.summaries
    );

    harness.shutdown().await;
}

#[tokio::test]
async fn a_cluster_of_one_is_not_a_pattern() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;
    let episodes = vec![
        common::episode("single-a", "lonely", "one thing happened", &["thing"]),
        common::episode("pair-a", "repeated", "a pattern began", &["pattern"]),
        common::episode("pair-b", "repeated", "the pattern recurred", &["pattern"]),
    ];
    let (_dir, path) = common::write_episodes(&episodes);
    engine
        .ingest_episodes(&path, &harness.logger)
        .await
        .unwrap();

    let outcome = engine.consolidate(window(Timestamp::now())).await.unwrap();
    assert_eq!(
        outcome.generalized, 1,
        "only the cluster with two members generalizes"
    );
    assert_eq!(outcome.archived.len(), 2);
    assert_eq!(
        harness
            .scalar("SELECT count(*) FROM memories WHERE status = 'active'")
            .await,
        outcome.after as i64
    );
    // The lone episode is untouched, which is the point: one occurrence is not a
    // pattern, and generalizing it would invent a regularity.
    assert_eq!(
        harness
            .scalar(
                "SELECT count(*) FROM memories WHERE status = 'active' \
                 AND kind = 'episodic'"
            )
            .await,
        1
    );

    harness.shutdown().await;
}

proptest! {
    /// For any stream size, consolidation never grows the active set and never
    /// loses a source. The concrete runs above cover the interesting shapes; this
    /// covers the boundary ones.
    ///
    /// The async body returns the violations it found rather than asserting
    /// inside the block, so a failure names what broke instead of reporting a
    /// macro expansion.
    #[test]
    fn consolidation_never_grows_the_set_nor_loses_a_source(count in 1usize..=8) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let failures: Vec<String> = runtime.block_on(async {
            let harness = Harness::new().await;
            let mut engine = harness.engine().await;
            let episodes: Vec<Episode> = (0..count)
                .map(|i| {
                    let topic = TOPICS[i % TOPICS.len()];
                    common::episode(
                        &format!("prop-{i}"),
                        topic,
                        &format!("{topic} occurrence {i}"),
                        &[topic],
                    )
                })
                .collect();
            let (_dir, path) = common::write_episodes(&episodes);
            engine.ingest_episodes(&path, &harness.logger).await.unwrap();
            let outcome = engine.consolidate(window(Timestamp::now())).await.unwrap();

            let mut failures = Vec::new();
            if outcome.after > outcome.before {
                failures.push(format!("the set grew: {outcome:?}"));
            }
            if outcome.archived.len() > count {
                failures.push(format!(
                    "more sources archived ({}) than episodes ingested ({count})",
                    outcome.archived.len()
                ));
            }

            let steps = harness
                .query("SELECT target_id, source_ids FROM memory_consolidations ORDER BY id")
                .await;
            let known: BTreeSet<String> = harness
                .query("SELECT id FROM memories")
                .await
                .iter()
                .filter_map(|row| row["id"].as_str().map(str::to_string))
                .collect();
            for step in &steps {
                let target = step["target_id"].as_str().unwrap_or_default();
                if !known.contains(target) {
                    failures.push(format!("target {target} does not resolve"));
                }
                let members: Vec<String> =
                    serde_json::from_str(step["source_ids"].as_str().unwrap_or("[]"))
                        .unwrap_or_default();
                for source in members {
                    if !known.contains(&source) {
                        failures.push(format!("source {source} vanished"));
                    }
                }
            }
            harness.shutdown().await;
            failures
        });
        prop_assert!(failures.is_empty(), "{failures:?}");
    }
}

/// SPARQL over `/memory` reconstructs every archived episode from the
/// consolidation that replaced it (plan §8): the mirror answers both
/// `mm:consolidatedFrom` and `prov:wasDerivedFrom`.
///
/// The reconciliation tests read the tables back; this one asks the *projection*
/// the same question, because a mirror that reconciles by count can still be the
/// wrong graph.
#[tokio::test]
async fn sparql_reconstructs_every_archived_episode_from_its_target() {
    let harness = Harness::new().await;
    let mut engine = harness.engine().await;

    let (_dir, path) = common::write_episodes(&stream());
    engine
        .ingest_episodes(&path, &harness.logger)
        .await
        .unwrap();
    let outcome = engine.consolidate(window(Timestamp::now())).await.unwrap();
    assert!(outcome.reduced(), "{outcome:?}");

    /// The IRIs bound to one `SELECT` variable.
    fn bound(rows: &[serde_json::Value], key: &str) -> BTreeSet<String> {
        rows.iter()
            .filter_map(|row| row[key].as_str().map(str::to_string))
            .collect()
    }

    let graph = mm_core::iri::graph(mm_memory::rdf::MEMORY_GRAPH);

    // The consolidation node names its members.
    let rows = harness
        .graph
        .sparql(
            mm_memory::rdf::MEMORY_GRAPH,
            &format!(
                "PREFIX mm: <https://metamind.dev/ontology#>\n\
                 SELECT ?source WHERE {{ GRAPH <{graph}> {{\n\
                   ?c mm:Consolidation ?c ; mm:consolidatedFrom ?source }} }}"
            ),
        )
        .await
        .unwrap();
    let from_consolidation = bound(&rows, "source");
    assert!(
        !from_consolidation.is_empty(),
        "the mirror must record the consolidation step"
    );

    // The PROV reading of the same edge.
    let rows = harness
        .graph
        .sparql(
            mm_memory::rdf::MEMORY_GRAPH,
            &format!(
                "PREFIX prov: <http://www.w3.org/ns/prov#>\n\
                 SELECT ?source WHERE {{ GRAPH <{graph}> {{ ?target prov:wasDerivedFrom ?source }} }}"
            ),
        )
        .await
        .unwrap();
    let from_prov = bound(&rows, "source");

    for archived in &outcome.archived {
        let iri = mm_core::iri::data(archived).into_string();
        assert!(
            from_consolidation.contains(&iri),
            "mm:consolidatedFrom must reach {iri}"
        );
        assert!(
            from_prov.contains(&iri),
            "prov:wasDerivedFrom must reach {iri}"
        );
    }

    harness.shutdown().await;
}
