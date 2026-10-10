//! `policy_versioning.rs` — a policy version is written once and never edited.
//!
//! What this file proves, over the real stores:
//!
//! * importing the committed seed registers the seeded policy as **version 1**,
//!   with no parent;
//! * each new version is one number higher and names the version before it, so the
//!   chain is provenance rather than a counter;
//! * the rows written earlier are **byte-identical** after later versions exist —
//!   an in-place update path would show up here as a changed `behavior_ttl`;
//! * fitness is appended per version, so version 3 has no fitness row while
//!   version 2 has two trials and a mean.
//!
//! The harness mirrors `mm-epistemic/tests/common/mod.rs`: a throwaway data
//! directory, a migrated store, an in-memory graph with the library's shapes, and a
//! manager over both — the same path a CLI command takes.

// The temporary directory is held for its lifetime rather than read.
#![allow(dead_code)]

use std::sync::Arc;

use mm_core::{ActivationCondition, Config, UlidFactory};
use mm_epistemic::{Outcome, OutcomeStatus};
use mm_library::entry::iri;
use mm_library::policy::{PolicyBehavior, PolicyDelta, Scope};
use mm_library::store::LibraryStore;
use mm_library::LibraryManager;
use mm_log::{Level, Logger, RedactionPolicy};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;

/// The seeded policy's name and head IRI.
const NAME: &str = "prefer_simpler_solution";
const HEAD: &str = "https://metamind.dev/policy/prefer_simpler_solution";

/// An open manager over a throwaway store and graph.
struct Harness {
    dir: tempfile::TempDir,
    graph: GraphStore,
    manager: LibraryManager,
}

impl Harness {
    /// Open a fresh harness and import the committed seed corpus.
    async fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let cfg = Config::for_data_dir(dir.path());
        std::fs::create_dir_all(&cfg.store.data_dir).expect("the data directory");
        let sqlite = SqliteStore::open(&cfg.store.sqlite_file)
            .await
            .expect("the store opens");
        sqlite.migrate().await.expect("the migrations apply");

        let logger = Arc::new(Logger::new(
            Level::Error,
            Vec::new(),
            Some(Arc::new(sqlite.clone())),
            RedactionPolicy::kernel_default(),
        ));
        let ids = Arc::new(UlidFactory::new().with_persist_every(1_000_000));
        let store = LibraryStore::new(sqlite, logger, ids);

        let shapes = Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("library.shacl.ttl");
        let graph = GraphStore::in_memory(&shapes)
            .await
            .expect("the graph opens");
        let manager = LibraryManager::new(store, Some(graph.handle().clone()));

        let seed =
            std::fs::read_to_string(Config::repo_root().join("ontology/seed/library_seed.ttl"))
                .expect("the committed seed corpus");
        let report = manager
            .import_document(&seed)
            .await
            .expect("the seed imports cleanly");
        assert!(
            report.rejected.is_empty(),
            "the committed seed must import without a refusal: {:?}",
            report.rejected
        );
        assert!(
            report
                .accepted
                .iter()
                .any(|entry| entry.starts_with("https://metamind.dev/policy/")),
            "the seed registers its policy: {:?}",
            report.accepted
        );

        Harness {
            dir,
            graph,
            manager,
        }
    }

    /// The store under test.
    fn store(&self) -> &LibraryStore {
        self.manager.store()
    }

    /// A delta that changes the behaviour of the seeded policy.
    fn delta(simplicity: f64) -> PolicyDelta {
        PolicyDelta {
            behavior: PolicyBehavior::from_weights([simplicity, 0.3, 0.3]),
            scope: Scope::Global,
            activation: ActivationCondition::new("two candidate solutions are both viable")
                .expect("a literal condition"),
            confidence: 0.55,
            reason: "an evolution candidate".to_string(),
        }
    }
}

#[tokio::test]
async fn versions_are_minted_in_order_and_name_their_parent() {
    let harness = Harness::new().await;

    let seeded = harness
        .store()
        .policy_history(HEAD)
        .await
        .expect("the seeded policy has a history");
    assert_eq!(seeded.len(), 1, "the seed registers exactly one version");
    assert_eq!(seeded[0].version, 1);
    assert_eq!(seeded[0].parent_version, None, "version 1 has no parent");
    assert_eq!(
        seeded[0].behavior, "weights:1.0+0.2+0.2",
        "the version keeps the behaviour the seed declared"
    );
    assert!(
        (seeded[0].confidence - 0.6).abs() < 1e-9,
        "the version keeps the seed's confidence, got {}",
        seeded[0].confidence
    );

    let second = harness
        .manager
        .new_policy_version(NAME, Harness::delta(0.6), &[])
        .await
        .expect("a second version is authored");
    assert_eq!(second.version, 2);
    assert_eq!(
        second.parent.map(|node| node.as_str().to_string()),
        Some("https://metamind.dev/policy/prefer_simpler_solution@1".to_string()),
        "version 2 descends from version 1"
    );

    let third = harness
        .manager
        .new_policy_version(NAME, Harness::delta(0.7), &[])
        .await
        .expect("a third version is authored");
    assert_eq!(third.version, 3);

    let history = harness
        .store()
        .policy_history(HEAD)
        .await
        .expect("the history is readable");
    assert_eq!(
        history
            .iter()
            .map(|row| (row.version, row.parent_version))
            .collect::<Vec<_>>(),
        vec![(1, None), (2, Some(1)), (3, Some(2))],
        "the chain is contiguous and parented"
    );
    assert_eq!(
        harness.store().head_version(HEAD).await.unwrap(),
        3,
        "the head tracks the last version"
    );
}

#[tokio::test]
async fn an_earlier_version_is_byte_identical_after_later_ones_exist() {
    let harness = Harness::new().await;

    let before = harness.store().policy_history(HEAD).await.unwrap();
    let first_row = before[0].clone();

    harness
        .manager
        .new_policy_version(NAME, Harness::delta(0.6), &[])
        .await
        .expect("a second version is authored");
    harness
        .manager
        .new_policy_version(NAME, Harness::delta(0.7), &[])
        .await
        .expect("a third version is authored");

    let after = harness.store().policy_history(HEAD).await.unwrap();
    assert_eq!(
        after[0], first_row,
        "version 1 must be untouched: there is no in-place update path"
    );
    assert_eq!(after.len(), 3);
    assert_eq!(
        after[0].behavior, "weights:1.0+0.2+0.2",
        "and its behaviour still reads as it was written"
    );
}

#[tokio::test]
async fn fitness_appends_per_version_and_leaves_untried_versions_empty() {
    let harness = Harness::new().await;

    harness
        .manager
        .new_policy_version(NAME, Harness::delta(0.6), &[])
        .await
        .expect("a second version is authored");
    harness
        .manager
        .new_policy_version(NAME, Harness::delta(0.7), &[])
        .await
        .expect("a third version is authored");

    let after_success = harness
        .manager
        .record_fitness(HEAD, 2, &Outcome::new(OutcomeStatus::Success))
        .await
        .expect("the first trial is recorded");
    assert_eq!(after_success.trials, 1);
    assert_eq!(after_success.successes, 1);
    assert!((after_success.mean_utility - 1.0).abs() < 1e-9);

    let after_failure = harness
        .manager
        .record_fitness(HEAD, 2, &Outcome::new(OutcomeStatus::Failure))
        .await
        .expect("the second trial is recorded");
    assert_eq!(after_failure.trials, 2);
    assert_eq!(after_failure.successes, 1);
    assert!(
        (after_failure.mean_utility - 0.5).abs() < 1e-9,
        "the running mean folds both outcomes, got {}",
        after_failure.mean_utility
    );

    let stored = harness
        .store()
        .fitness_of(HEAD, 2)
        .await
        .expect("the fitness row is readable")
        .expect("version 2 has a fitness row");
    assert_eq!(stored.0, 2, "two trials");
    assert_eq!(stored.1, 1, "one success");
    assert!((stored.2 - 0.5).abs() < 1e-9);

    assert!(
        harness.store().fitness_of(HEAD, 3).await.unwrap().is_none(),
        "a version nobody tried has no fitness row"
    );
    assert!(
        harness.store().fitness_of(HEAD, 1).await.unwrap().is_none(),
        "and neither does the seeded one"
    );

    // The graph is closed explicitly so a dropped writer cannot outlive the test's
    // temporary directory; the directory itself is dropped with the harness.
    harness.graph.shutdown().await.expect("the graph closes");
}

#[tokio::test]
async fn the_seeded_policy_head_is_the_iri_the_gate_names() {
    let harness = Harness::new().await;
    let head = iri::head_of("https://metamind.dev/policy/prefer_simpler_solution@1")
        .expect("a versioned IRI has a head");
    assert_eq!(head, HEAD);
    assert!(
        !harness
            .store()
            .policy_history(&head)
            .await
            .unwrap()
            .is_empty(),
        "the seeded head resolves"
    );
    assert!(
        harness
            .store()
            .policy_history("https://metamind.dev/policy/never_authored")
            .await
            .unwrap()
            .is_empty(),
        "a policy nobody authored has no versions"
    );
    harness.graph.shutdown().await.expect("the graph closes");
}
