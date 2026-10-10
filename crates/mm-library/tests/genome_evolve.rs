//! `genome_evolve.rs` — a generation retains only fitness gains, and every
//! candidate is an immutable version.
//!
//! Two of the plan's risks are answered here: "genome evolution burns budget /
//! drifts" wants a hard cap and a retain-only-gains rule, and "every candidate
//! immutable and rollback-able" wants a candidate to be a *new* version rather
//! than an edit of an old one. Both are measured against the store rather than
//! asserted from the run's own report.

use std::path::PathBuf;
use std::sync::Arc;

use mm_core::{Config, Param, Tabular, UlidFactory};
use mm_library::genome::{evolve, EvaluationSet, EvoBudget};
use mm_library::{LibraryManager, LibraryStore};
use mm_log::{Level, Logger, RedactionPolicy};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;

const POLICY: &str = "https://metamind.dev/policy/prefer_simpler_solution";

fn repo(relative: &str) -> PathBuf {
    Config::repo_root().join(relative)
}

/// A manager over a throwaway store and an in-memory `/library` graph.
struct Harness {
    _dir: tempfile::TempDir,
    graph: GraphStore,
    manager: LibraryManager,
}

async fn harness() -> Harness {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let cfg = Config::for_data_dir(dir.path());
    std::fs::create_dir_all(&cfg.store.data_dir).expect("the data directory");
    let sqlite = SqliteStore::open(&cfg.store.sqlite_file)
        .await
        .expect("a fresh store");
    sqlite.migrate().await.expect("the migrations");
    let shapes = repo("ontology/shapes/library.shacl.ttl");
    let graph = GraphStore::in_memory(&shapes)
        .await
        .expect("the graph store");
    let logger = Arc::new(Logger::new(
        Level::Trace,
        Vec::new(),
        Some(Arc::new(sqlite.clone())),
        RedactionPolicy::kernel_default(),
    ));
    let ids = Arc::new(UlidFactory::new().with_persist_every(1_000_000));
    let store = LibraryStore::new(sqlite, logger, ids);
    let manager = LibraryManager::new(store, Some(graph.handle().clone()));
    Harness {
        _dir: dir,
        graph,
        manager,
    }
}

/// The seed's policy has to exist before evolution can extend it.
async fn harness_with_seed() -> Harness {
    let h = harness().await;
    let seed = std::fs::read_to_string(repo("ontology/seed/library_seed.ttl"))
        .expect("the committed seed");
    h.manager
        .import_document(&seed)
        .await
        .expect("the seed imports");
    h
}

/// Two cases per axis, so a weighting that favours any one axis scores a third.
fn evals() -> EvaluationSet {
    EvaluationSet::from_jsonl(
        r#"{"ideal": [0.6, 0.25, 0.15], "expected": "simplicity"}
{"ideal": [0.55, 0.3, 0.15], "expected": "simplicity"}
{"ideal": [0.2, 0.6, 0.2], "expected": "speed"}
{"ideal": [0.25, 0.5, 0.25], "expected": "speed"}
{"ideal": [0.15, 0.25, 0.6], "expected": "safety"}
{"ideal": [0.2, 0.2, 0.6], "expected": "safety"}"#,
    )
    .expect("the inline evaluation set parses")
}

/// A policy version's stored content hash, read straight from the index.
async fn content_hash_of(manager: &LibraryManager, version: i64) -> String {
    let rows = Tabular::query_json(
        manager.store().sqlite(),
        "SELECT content_hash FROM policy_versions WHERE version = ?",
        vec![Param::Int(version)],
    )
    .await
    .expect("the query runs");
    rows.first()
        .and_then(|row| row.get("content_hash"))
        .and_then(|value| value.as_str())
        .expect("the version exists")
        .to_string()
}

#[tokio::test]
async fn a_generation_retains_only_gains_and_every_candidate_is_a_new_version() {
    let h = harness_with_seed().await;
    let budget = EvoBudget::default_budget();
    let report = evolve(&h.manager, "prefer_simpler_solution", &budget, &evals())
        .await
        .expect("the run completes");

    assert!(report.versions_created > 0, "the run must try something");
    assert!(
        report.versions_created <= budget.max_versions,
        "the hard budget caps the run: {} > {}",
        report.versions_created,
        budget.max_versions
    );

    let history = h
        .manager
        .store()
        .policy_history(POLICY)
        .await
        .expect("the history is readable");
    assert_eq!(
        history.len() as u32,
        1 + report.versions_created,
        "every candidate is an immutable version"
    );
    assert_eq!(history[0].version, 1, "the seeded version is still first");
    for window in history.windows(2) {
        assert_eq!(
            window[1].parent_version,
            Some(window[0].version),
            "versions are parented to the previous version: {window:?}"
        );
        assert!(
            window[1].version > window[0].version,
            "versions are monotonic"
        );
    }

    // `retained` means *strictly* better than the parent at the time it was tried;
    // a candidate that merely matched its parent is kept, and marked, but not
    // retained.
    let mut parent_fitness = 0.0_f64;
    let mut retained = 0usize;
    for generation in &report.generations {
        assert!(
            generation.retained <= 1,
            "at most one candidate per generation beats the parent"
        );
        for candidate in &generation.candidates {
            if candidate.retained {
                assert!(
                    candidate.fitness > parent_fitness,
                    "v{} was retained at {} against a parent at {}",
                    candidate.version,
                    candidate.fitness,
                    parent_fitness
                );
                parent_fitness = candidate.fitness;
                retained += 1;
            } else {
                assert!(
                    candidate.fitness <= parent_fitness,
                    "v{} did not beat its parent at {} but was flagged retained",
                    candidate.version,
                    parent_fitness
                );
            }
        }
    }
    assert_eq!(
        retained,
        report
            .generations
            .iter()
            .map(|g| g.retained as usize)
            .sum::<usize>(),
        "the report's per-generation counts agree with its candidates"
    );

    h.graph.shutdown().await.expect("the graph closes");
}

#[tokio::test]
async fn a_second_run_appends_versions_and_never_rewrites_the_first() {
    let h = harness_with_seed().await;
    let budget = EvoBudget::default_budget();
    let first = evolve(&h.manager, "prefer_simpler_solution", &budget, &evals())
        .await
        .expect("the first run completes");
    let first_hash = content_hash_of(&h.manager, 1).await;
    let first_len = h
        .manager
        .store()
        .policy_history(POLICY)
        .await
        .expect("the history is readable")
        .len();

    let second = evolve(&h.manager, "prefer_simpler_solution", &budget, &evals())
        .await
        .expect("the second run completes");
    assert!(second.versions_created > 0);
    assert_eq!(
        content_hash_of(&h.manager, 1).await,
        first_hash,
        "version 1 is immutable: a second run may not rewrite it"
    );
    let second_len = h
        .manager
        .store()
        .policy_history(POLICY)
        .await
        .expect("the history is readable")
        .len();
    assert_eq!(
        second_len,
        first_len + second.versions_created as usize,
        "the second run appended {} versions",
        second.versions_created
    );
    assert!(
        second.generations.len() <= budget.generations as usize,
        "the run respects its generation count"
    );
    assert!(
        first.best_version.is_some(),
        "the seeded run starts from v1"
    );

    h.graph.shutdown().await.expect("the graph closes");
}

#[tokio::test]
async fn the_budget_cap_stops_the_run_where_it_says_it_will() {
    let h = harness_with_seed().await;
    let budget = EvoBudget {
        generations: 10,
        candidates_per_generation: 5,
        max_versions: 4,
    };
    let report = evolve(&h.manager, "prefer_simpler_solution", &budget, &evals())
        .await
        .expect("the run completes");
    assert_eq!(
        report.versions_created, 4,
        "the run stops at `max_versions`, not at `generations`"
    );
    assert_eq!(
        h.manager
            .store()
            .policy_history(POLICY)
            .await
            .expect("the history is readable")
            .len(),
        5,
        "the seed plus four candidates"
    );

    h.graph.shutdown().await.expect("the graph closes");
}

#[test]
fn a_budget_of_zero_generations_is_a_refusal() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("budget.toml");
    std::fs::write(
        &path,
        "generations = 0\ncandidates_per_generation = 3\nmax_versions = 12\n",
    )
    .expect("the fixture writes");
    let error = EvoBudget::load(&path).expect_err("a no-op run is worse than a failure");
    assert_eq!(error.code(), "validation");

    let path = dir.path().join("ok.toml");
    std::fs::write(
        &path,
        "generations = 3\ncandidates_per_generation = 3\nmax_versions = 12\n",
    )
    .expect("the fixture writes");
    let budget = EvoBudget::load(&path).expect("the committed budget shape loads");
    assert_eq!(budget, EvoBudget::default_budget());
}
