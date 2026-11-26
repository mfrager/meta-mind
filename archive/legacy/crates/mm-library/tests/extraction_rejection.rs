//! `extraction_rejection.rs` — malformed input is refused, valid input commits.
//!
//! The plan's testing table asks for both halves: `malformed.jsonl` produces a
//! rejection ledger, and `raw.jsonl` commits through the same gate. The point is
//! that extraction has no second door — a candidate that the shapes, the duplicate
//! check or the orphan check refuse is ledgered, and nothing is written.

use std::path::PathBuf;
use std::sync::Arc;

use mm_core::{Config, UlidFactory};
use mm_library::{seed, LibraryManager, LibraryStore};
use mm_log::{Level, Logger, RedactionPolicy};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;

fn repo(relative: &str) -> PathBuf {
    Config::repo_root().join(relative)
}

fn fixture(relative: &str) -> String {
    std::fs::read_to_string(repo(relative))
        .unwrap_or_else(|e| panic!("cannot read {relative}: {e}"))
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

#[test]
fn the_malformed_fixture_is_ledgered_line_by_line() {
    let (accepted, rejected) =
        seed::extract_candidates(&fixture("bench/library/extraction/malformed.jsonl"));
    assert!(
        accepted.is_empty(),
        "a malformed line must never be accepted: {accepted:?}"
    );
    assert!(
        rejected.len() >= 3,
        "one rejection per bad line: {rejected:?}"
    );
    let mut previous = 0;
    for rejection in &rejected {
        assert!(
            rejection.line > previous,
            "rejections are ordered by 1-based line: {rejected:?}"
        );
        previous = rejection.line;
        assert!(!rejection.reason.trim().is_empty());
    }
    assert_eq!(rejected[0].line, 1);
}

#[test]
fn the_raw_fixture_parses_cleanly() {
    let (accepted, rejected) =
        seed::extract_candidates(&fixture("bench/library/extraction/raw.jsonl"));
    assert!(accepted.len() >= 4, "the fixture commits: {accepted:?}");
    assert!(rejected.is_empty(), "nothing is refused: {rejected:?}");
}

#[tokio::test]
async fn a_valid_document_commits_through_the_gate_and_a_malformed_one_does_not() {
    let h = harness().await;
    // The candidates name seeded cases as their evidence, so the orphan check can
    // only pass once the seed itself is imported.
    h.manager
        .import_document(&fixture("ontology/seed/library_seed.ttl"))
        .await
        .expect("the seed imports");

    let report = seed::extract(
        &h.manager,
        "bench/library/extraction/raw.jsonl",
        &fixture("bench/library/extraction/raw.jsonl"),
    )
    .await
    .expect("extraction runs");
    assert!(
        !report.has_rejections(),
        "a valid document is not refused: {:?}",
        report.rejected
    );
    assert_eq!(report.accepted.len(), 5);
    assert_eq!(
        report.accepted_jsonl.lines().count(),
        report.accepted.len(),
        "one accepted object per line"
    );
    assert_eq!(
        h.manager
            .store()
            .get_entry(&report.accepted[0])
            .await
            .expect("the index answers")
            .map(|row| row.version_iri),
        Some(report.accepted[0].clone()),
        "an accepted candidate is really committed"
    );

    let refused = seed::extract(
        &h.manager,
        "bench/library/extraction/malformed.jsonl",
        &fixture("bench/library/extraction/malformed.jsonl"),
    )
    .await
    .expect("extraction runs and reports");
    assert!(
        refused.has_rejections(),
        "a malformed document must exit non-zero"
    );
    assert!(refused.accepted.is_empty());

    h.graph.shutdown().await.expect("the graph closes");
}
