//! Shared harness for the Phase 4 integration tests.
//!
//! Every test runs against a throwaway SQLite store, an in-memory graph, and a
//! logger whose sink is a file — the same three things a real run has, so a test
//! can assert on the tables, the `/being` graph, and the emitted records instead
//! of on internal state.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use mm_being::BeingFacade;
use mm_core::{Config, Params, Tabular, Ulid, UlidFactory};
use mm_log::{JsonlSink, Level, Logger, RedactionPolicy};
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

/// The adversarial corpus the `being verify` gate reads.
pub fn corpus_path() -> PathBuf {
    fixture("bench/being/invariants.jsonl")
}

/// One test's kernel: store, graph, logger, and identifier factory.
pub struct Harness {
    /// Kept alive so the store, graph, and sink files outlive the test.
    pub dir: tempfile::TempDir,
    /// The being's tabular store.
    pub store: SqliteStore,
    /// The `/being` provenance graph.
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
        let shapes = cfg.being_shapes_file();
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

    /// Open a being over this harness; a second call over the same harness is a
    /// second writer over the same store, which is how the concurrency test runs.
    pub async fn facade(&self) -> BeingFacade {
        BeingFacade::open(
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

    /// Fund an account out of band, with a matching ledger row.
    ///
    /// The facade has no funding operation — a spend is the only mutation it
    /// records — so a test that needs a positive balance seeds the account and its
    /// ledger delta together, keeping `sum(delta) == balance` true.
    pub async fn fund(&self, kind: &str, balance: f64, unit: &str) {
        let id = mm_core::ulid_string(&self.ids.next());
        Tabular::execute(
            &self.store,
            "INSERT INTO resource_accounts (kind, balance, unit, updated_ulid) VALUES (?, ?, ?, ?) \
             ON CONFLICT(kind) DO UPDATE SET balance = excluded.balance",
            vec![
                kind.to_string().into(),
                balance.into(),
                unit.to_string().into(),
                id.clone().into(),
            ],
        )
        .await
        .unwrap();
        Tabular::execute(
            &self.store,
            "INSERT INTO resource_ledger (id, kind, delta, balance_after, purpose, ref_ulid, \
             created_ulid) VALUES (?, ?, ?, ?, 'allowance', NULL, ?)",
            vec![
                id.clone().into(),
                kind.to_string().into(),
                balance.into(),
                balance.into(),
                id.into(),
            ],
        )
        .await
        .unwrap();
    }

    /// Install a budget policy for a resource kind.
    pub async fn policy(&self, kind: &str, period: &str, limit: f64, hard: bool) {
        Tabular::execute(
            &self.store,
            "INSERT INTO budget_policies (kind, period, limit_amount, hard) VALUES (?, ?, ?, ?) \
             ON CONFLICT(kind) DO UPDATE SET period = excluded.period, \
             limit_amount = excluded.limit_amount, hard = excluded.hard",
            vec![
                kind.to_string().into(),
                period.to_string().into(),
                limit.into(),
                (if hard { 1i64 } else { 0i64 }).into(),
            ],
        )
        .await
        .unwrap();
    }

    /// Shut the graph and store down.
    pub async fn shutdown(self) {
        self.graph.shutdown().await.unwrap();
        self.store.close().await;
    }
}

/// Read a JSONL fixture: one JSON value per non-empty, non-`#` line.
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

/// A deterministic user ULID for a test.
pub fn user(n: u128) -> Ulid {
    Ulid::from_parts(1_700_000_000_000, n)
}

/// A deterministic non-user ULID for a test.
pub fn ulid(n: u128) -> Ulid {
    Ulid::from_parts(1_700_000_000_000, n)
}
