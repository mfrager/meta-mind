//! Shared harness for the integration tests.
//!
//! Every test runs against a throwaway store, an in-memory provenance graph, and a
//! logger whose sink is a file — the same three things a real run has, so a test
//! can assert on the ledger, the graph, and the emitted records instead of on
//! internal state.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use mm_llm::client::{LlmClient, SchemaId};
use mm_llm::config::LlmConfig;
use mm_llm::service::LlmService;
use mm_log::{Level, Logger, RedactionPolicy};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;

/// Held by any test that arms the process-wide `NetworkGuard`, asserts on its
/// refusal counter, or needs egress to be *permitted*. Cargo runs the tests in one
/// binary on parallel threads, and the guard is deliberately process-wide, so
/// without this lock two tests would observe each other's arming. An async mutex
/// because the guard is held across the test's await points.
static NETWORK_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Take the network-guard lock for the rest of the test.
pub async fn network_lock() -> tokio::sync::MutexGuard<'static, ()> {
    NETWORK_LOCK.lock().await
}

/// Register the repository's `bench/llm/schemas.json`, the same way the CLI does,
/// so a recorded session that names `extract.v1` is enforceable in a test too.
/// A missing file is not an error: the fixtures are the test's own business.
pub fn register_bench_schemas(service: &mut LlmService) {
    let path = fixture("bench/llm/schemas.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    let entries: std::collections::BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    for (id, schema) in entries {
        service
            .register_schema(SchemaId::new(id), schema)
            .expect("the bench schemas are well formed");
    }
}

/// The repository root, which is where the committed fixtures live.
pub fn repo_root() -> PathBuf {
    mm_core::Config::repo_root()
}

/// A repository fixture path.
pub fn fixture(relative: &str) -> PathBuf {
    repo_root().join(relative)
}

/// A configuration with two routable models and a bounded repair loop.
pub fn config_toml(max_repair_attempts: u32) -> String {
    format!(
        r#"
[defaults]
decoder = "llguidance"
semantic_cache_purposes = ["summarize"]
semantic_cache_threshold = 0.97
max_repair_attempts = {max_repair_attempts}
timeout_ms = 30000
max_tokens = 256

[[model]]
name = "small"
input_cost_micros_per_1k = 150
output_cost_micros_per_1k = 600
typical_latency_ms = 400
complexity_ceiling = "medium"

[[model]]
name = "frontier"
input_cost_micros_per_1k = 2500
output_cost_micros_per_1k = 10000
typical_latency_ms = 2500
complexity_ceiling = "very_high"

[[model]]
name = "mock-small"
input_cost_micros_per_1k = 100
output_cost_micros_per_1k = 400
typical_latency_ms = 5
complexity_ceiling = "high"
"#
    )
}

/// One test's kernel: store, graph, logger, and configuration.
pub struct Harness {
    /// Kept alive so the store, graph, and sink files outlive the test.
    pub dir: tempfile::TempDir,
    /// The ledger and audit store.
    pub store: SqliteStore,
    /// The provenance graph.
    pub graph: GraphStore,
    /// The file the logger writes JSONL to.
    pub log_path: PathBuf,
    /// The configuration the service is built from.
    pub cfg: LlmConfig,
    /// The logger every service in the test shares.
    pub logger: Arc<Logger>,
}

impl Harness {
    /// Build a harness whose store and graph start empty.
    pub async fn new(max_repair_attempts: u32) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("metamind.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let shapes = fixture("ontology/shapes/mm-shapes.ttl");
        let graph = GraphStore::in_memory(&shapes).await.unwrap();
        let log_path = dir.path().join("mm.jsonl");
        let sink = mm_log::JsonlSink::open(&log_path).unwrap();
        let logger = Arc::new(Logger::new(
            Level::Debug,
            vec![Box::new(sink)],
            Some(Arc::new(store.clone())),
            RedactionPolicy::kernel_default(),
        ));
        let cfg = LlmConfig::from_toml_str(&config_toml(max_repair_attempts)).unwrap();
        Harness {
            dir,
            store,
            graph,
            log_path,
            cfg,
            logger,
        }
    }

    /// A service over this harness, rooted at a per-test cache directory.
    ///
    /// Schemas are *not* registered here; a test that needs one registers it (or
    /// calls [`register_bench_schemas`]), so re-registering a name stays an error a
    /// test can see rather than a surprise the harness hides.
    pub fn service_with(&self, client: Arc<dyn LlmClient>) -> LlmService {
        LlmService::new(
            self.cfg.clone(),
            self.store.clone(),
            self.graph.handle().clone(),
            Arc::clone(&self.logger),
            client,
        )
        .unwrap()
        .with_cache_dir(self.dir.path().join("llm_cache"))
    }

    /// Every emitted record, parsed.
    pub fn records(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(&self.log_path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// Every emitted record with `event_code`.
    pub fn records_of(&self, event_code: &str) -> Vec<serde_json::Value> {
        self.records()
            .into_iter()
            .filter(|record| record["event_code"] == event_code)
            .collect()
    }

    /// The raw text of every record, for a "did anything leak" check.
    pub fn raw_log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    /// The `call_id`s recorded in the audited `llm.accounting.commit` records.
    pub async fn committed_call_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for row in self.store.audit_rows().await.unwrap() {
            if row.event_code != mm_log::codes::LLM_ACCOUNTING_COMMIT {
                continue;
            }
            if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&row.payload) {
                if let Some(id) = payload["call_id"].as_str() {
                    out.push(id.to_string());
                }
            }
        }
        out
    }

    /// The `call_id`s in the ledger.
    pub async fn ledger_call_ids(&self) -> Vec<String> {
        let index: &dyn mm_core::Tabular = &self.store;
        index
            .query_json(
                "SELECT id FROM llm_calls ORDER BY id",
                mm_core::Params::new(),
            )
            .await
            .unwrap()
            .iter()
            .filter_map(|row| row["id"].as_str().map(str::to_string))
            .collect()
    }

    /// Shut the graph down; the store's pool closes when the harness drops.
    pub async fn shutdown(self) {
        self.graph.shutdown().await.unwrap();
        self.store.close().await;
    }
}
