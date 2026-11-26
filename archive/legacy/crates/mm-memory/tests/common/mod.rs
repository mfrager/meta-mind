//! Shared harness for the Phase 5 integration tests.
//!
//! Every test runs against a throwaway SQLite store, an in-memory graph, and a
//! logger whose sink is a file — the same three things a real run has, so a test
//! can assert on the tables, the `/memory` graph, and the emitted records instead
//! of on internal state.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use mm_core::{Config, Params, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{JsonlSink, Level, Logger, RedactionPolicy};
use mm_memory::{Episode, Memory, MemoryEngine, MemoryKind};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;
use serde_json::Value;

/// The repository root, which is where the committed fixtures live.
pub fn repo_root() -> PathBuf {
    Config::repo_root()
}

/// A repository fixture path.
pub fn fixture(relative: &str) -> PathBuf {
    repo_root().join(relative)
}

/// The episode stream the consolidation and eval tests read.
pub fn episodes_path() -> PathBuf {
    fixture("bench/memory/episodes.jsonl")
}

/// The gold set `memory eval` grades against.
pub fn gold_path() -> PathBuf {
    fixture("bench/memory/gold.jsonl")
}

/// The thresholds configuration.
pub fn thresholds_path() -> PathBuf {
    fixture("bench/memory/thresholds.toml")
}

/// The adversarial fixtures that must be flagged, never silently accepted.
pub fn adversarial_dir() -> PathBuf {
    fixture("bench/memory/adversarial")
}

/// Read a JSONL file: one JSON value per non-empty, non-`#` line.
pub fn jsonl(path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect()
}

/// One test's kernel: store, graph, logger, and identifier factory.
pub struct Harness {
    /// Kept alive so the store, graph, and sink files outlive the test.
    pub dir: tempfile::TempDir,
    /// The memory organ's tabular store.
    pub store: SqliteStore,
    /// The `/memory` graph.
    pub graph: GraphStore,
    /// The file the logger writes JSONL to.
    pub log_path: PathBuf,
    /// The configuration the store was opened from.
    pub cfg: Config,
    /// The shared logger.
    pub logger: Arc<Logger>,
    /// The shared monotonic identifier factory.
    pub ids: Arc<UlidFactory>,
}

impl Harness {
    /// Build a harness whose store and graph start empty.
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::for_data_dir(dir.path());
        std::fs::create_dir_all(&cfg.store.data_dir).unwrap();
        let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
        store.migrate().await.unwrap();
        let shapes = cfg.memory_shapes_file();
        let graph = GraphStore::in_memory(&shapes).await.unwrap();
        let log_path = dir.path().join("mm.jsonl");
        let sink = JsonlSink::open(&log_path).unwrap();
        let logger = Arc::new(Logger::new(
            Level::Trace,
            vec![Box::new(sink)],
            Some(Arc::new(store.clone())),
            RedactionPolicy::kernel_default(),
        ));
        let ids = Arc::new(UlidFactory::open(&cfg.ulid_watermark_path()).unwrap());
        Harness {
            dir,
            store,
            graph,
            log_path,
            cfg,
            logger,
            ids,
        }
    }

    /// Open a memory engine over this harness.
    pub async fn engine(&self) -> MemoryEngine {
        MemoryEngine::open(
            &self.store,
            self.graph.handle(),
            Arc::clone(&self.logger),
            Arc::clone(&self.ids),
        )
        .await
        .unwrap()
    }

    /// Every emitted record, parsed.
    pub fn records(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log_path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// Every emitted record with `event_code`.
    pub fn records_of(&self, event_code: &str) -> Vec<Value> {
        self.records()
            .into_iter()
            .filter(|record| record["event_code"] == event_code)
            .collect()
    }

    /// The raw text of every record, for a "did anything leak" check.
    pub fn raw_log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    /// Every audited event code, in order.
    pub async fn audit_codes(&self) -> Vec<String> {
        self.store
            .audit_rows()
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.event_code)
            .collect()
    }

    /// Run a scalar query against the store.
    pub async fn scalar(&self, sql: &str) -> i64 {
        Tabular::query_json(&self.store, sql, Params::new())
            .await
            .unwrap()
            .first()
            .and_then(Value::as_object)
            .and_then(|map| map.values().next())
            .and_then(Value::as_i64)
            .unwrap_or(0)
    }

    /// Run a query against the store.
    pub async fn query(&self, sql: &str) -> Vec<Value> {
        Tabular::query_json(&self.store, sql, Params::new())
            .await
            .unwrap()
    }

    /// Shut the graph and store down.
    pub async fn shutdown(self) {
        self.graph.shutdown().await.unwrap();
        self.store.close().await;
    }
}

/// A deterministic ULID for a test.
pub fn ulid(n: u128) -> Ulid {
    Ulid::from_parts(1_700_000_000_000, n)
}

/// A deterministic instant, in whole epoch seconds.
pub fn ts(seconds: u64) -> Timestamp {
    Timestamp::from_epoch_seconds(seconds)
}

/// An episodic memory with a deterministic id.
pub fn memory(n: u128, content: &str, at: Timestamp) -> Memory {
    Memory::draft(ulid(n), MemoryKind::Episodic, content, at).unwrap()
}

/// An episode for the ingestion path.
pub fn episode(reference: &str, topic: &str, content: &str, entities: &[&str]) -> Episode {
    Episode {
        reference: Some(reference.to_string()),
        kind: Some("episodic".to_string()),
        topic: Some(topic.to_string()),
        content: content.to_string(),
        confidence: 0.8,
        importance: 0.6,
        entities: entities.iter().map(|e| e.to_string()).collect(),
        category: None,
        valid_from: None,
    }
}

/// Write an episode stream to a temporary file, returning its path.
///
/// `TempDir` is returned alongside so the caller keeps it alive.
pub fn write_episodes(episodes: &[Episode]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("episodes.jsonl");
    let body: String = episodes
        .iter()
        .map(|episode| serde_json::to_string(episode).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, format!("{body}\n")).unwrap();
    (dir, path)
}
