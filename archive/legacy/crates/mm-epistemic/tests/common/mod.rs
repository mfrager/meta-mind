//! Shared harness for the `mm-epistemic` integration tests.
//!
//! It opens a throwaway SQLite store under a temporary directory, an in-memory
//! graph with the epistemic shapes, and an engine over both — so a test exercises
//! the same validate → append → mutate → mirror path a CLI command does, without
//! touching the kernel's real state.

#![allow(dead_code)]

use std::sync::Arc;

use mm_core::{Config, Ulid, UlidFactory};
use mm_epistemic::{
    Claim, ClaimKind, EpistemicEngine, EpistemicStatus, Evidence, EvidenceKind, Proposition,
    SqliteEpistemicStore,
};
use mm_log::{Level, Logger, RedactionPolicy};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;

/// An open engine over a throwaway store and graph.
pub struct Harness {
    /// The temporary directory; dropped last.
    pub dir: tempfile::TempDir,
    /// The epistemic store.
    pub store: Arc<SqliteEpistemicStore>,
    /// The graph, kept alive so the engine's handle stays valid.
    pub graph: GraphStore,
    /// The engine under test.
    pub engine: EpistemicEngine,
}

impl Harness {
    /// Open a fresh harness.
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::for_data_dir(dir.path());
        std::fs::create_dir_all(&cfg.store.data_dir).unwrap();
        let sqlite = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
        sqlite.migrate().await.unwrap();
        let shapes = Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("epistemic_shapes.ttl");
        let graph = GraphStore::in_memory(&shapes).await.unwrap();
        let logger = Arc::new(Logger::new(
            Level::Trace,
            Vec::new(),
            Some(Arc::new(sqlite.clone())),
            RedactionPolicy::kernel_default(),
        ));
        let ids = Arc::new(UlidFactory::new().with_persist_every(1_000_000));
        let store = Arc::new(SqliteEpistemicStore::new(sqlite, logger, ids));
        let engine = EpistemicEngine::new(store.clone(), graph.handle().clone());
        Harness {
            dir,
            store,
            graph,
            engine,
        }
    }

    /// The shapes file this harness validates against.
    pub fn shapes() -> std::path::PathBuf {
        Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("epistemic_shapes.ttl")
    }

    /// A claim id fixture.
    pub fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    /// A claim about `object` on a fixed proposition.
    pub fn claim(n: u128, object: &str, status: EpistemicStatus) -> Claim {
        Claim::new(
            Harness::id(n),
            ClaimKind::Fact,
            Proposition::literal(
                "https://metamind.dev/data/pipeline",
                "https://metamind.dev/ontology#deploymentState",
                object,
            )
            .unwrap(),
            status,
            0.7,
        )
        .unwrap()
    }

    /// Evidence of the given kind.
    pub fn evidence(n: u128, kind: EvidenceKind) -> Evidence {
        Evidence::from_content(Harness::id(n), kind, None, "proof", 0.95).unwrap()
    }

    /// A claim that is already world-admissible.
    pub fn observed(n: u128, object: &str) -> Claim {
        let mut claim = Harness::claim(n, object, EpistemicStatus::Observed);
        claim.evidence = vec![Harness::id(n + 1_000)];
        claim
    }

    /// Close the graph.
    pub async fn shutdown(self) {
        self.graph.shutdown().await.unwrap();
    }
}
