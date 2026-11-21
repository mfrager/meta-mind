//! The RDF engine wrapper.
//!
//! The kernel's `mm-store-graph` owns the RocksDB/Oxigraph store, its single
//! writer thread and SPARQL scoping. `RdfEngine` adds the data-root layout and
//! the read/write helpers the registries and memory tiers use. `cubecast` is
//! not linked here: its pinned Oxigraph fork and the kernel's Oxigraph both
//! declare `links = "rocksdb"`, so only one can be in the binary. The swap
//! belongs to Phase 7, when `mm-store-graph` itself is replaced.

use std::path::{Path, PathBuf};

use mm_store_graph::GraphStore;
use oxrdf::{GraphName, NamedNode, Quad, Term};
use serde_json::Value;

use crate::config::EngineConfig;
use crate::error::{DataError, Result};

/// Where the persistent engine lives under the data root.
pub const ENGINE_DIR: &str = "cubecast/main";

/// An open RDF engine with its data-root configuration.
pub struct RdfEngine {
    store: GraphStore,
    config: EngineConfig,
    base_dir: PathBuf,
}

impl std::fmt::Debug for RdfEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RdfEngine")
            .field("base_dir", &self.base_dir)
            .field("persistent", &self.config.persistent)
            .field("store", &self.store.path())
            .finish()
    }
}

impl RdfEngine {
    /// Open (or create) the engine under `base_dir`.
    pub async fn open(base_dir: &Path, config: EngineConfig) -> Result<Self> {
        let shapes = shapes_dir(&config);
        let store = if config.persistent {
            let path = base_dir.join(ENGINE_DIR);
            GraphStore::open(&path, &shapes).await?
        } else {
            GraphStore::in_memory(&shapes).await?
        };
        Ok(Self {
            store,
            config,
            base_dir: base_dir.to_path_buf(),
        })
    }

    /// Wrap an already-open store. Used by tests and by callers that share the
    /// kernel's store.
    pub fn from_store(store: GraphStore, base_dir: PathBuf, config: EngineConfig) -> Self {
        Self {
            store,
            config,
            base_dir,
        }
    }

    /// The underlying graph store.
    pub fn store(&self) -> &GraphStore {
        &self.store
    }

    /// The data root.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// The engine configuration in force.
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Insert one quad. The quad carries its graph name.
    pub async fn insert(&self, quad: Quad) -> Result<()> {
        self.store.handle().insert(quad).await?;
        Ok(())
    }

    /// Insert several quads.
    pub async fn insert_all(&self, quads: Vec<Quad>) -> Result<()> {
        for quad in quads {
            self.insert(quad).await?;
        }
        Ok(())
    }

    /// Parse a Turtle document and insert it into `graph`.
    pub async fn load_turtle(&self, graph: &str, turtle: &str) -> Result<usize> {
        Ok(self.store.handle().insert_turtle(graph, turtle).await?)
    }

    /// Load a Turtle file into `graph` when it exists.
    pub async fn load_turtle_file(&self, graph: &str, path: &Path) -> Result<usize> {
        if !path.exists() {
            return Ok(0);
        }
        let turtle = std::fs::read_to_string(path)?;
        self.load_turtle(graph, &turtle).await
    }

    /// Run a SPARQL query scoped to one named graph. Rows are JSON objects.
    pub async fn sparql(&self, graph: &str, query: &str) -> Result<Vec<Value>> {
        Ok(self.store.sparql(graph, query).await?)
    }

    /// Every triple of one named graph as `{s, p, o}` objects.
    pub async fn triples(&self, graph: &str) -> Result<Vec<Value>> {
        Ok(self.store.triples(graph).await?)
    }

    /// Serialize one graph as Turtle.
    pub async fn dump_turtle(&self, graph: &str) -> Result<String> {
        Ok(self.store.dump_turtle(graph).await?)
    }

    /// Named graphs that hold data.
    pub async fn named_graphs(&self) -> Result<Vec<String>> {
        Ok(self.store.named_graphs().await?)
    }

    /// Total quads.
    pub async fn quad_count(&self) -> Result<usize> {
        Ok(self.store.quad_count().await?)
    }

    /// `ASK { ?s ?p ?o }` against one graph.
    pub async fn graph_is_ready(&self, graph: &str) -> Result<bool> {
        let query = format!("ASK {{ GRAPH <{graph}> {{ ?s ?p ?o }} }}");
        let rows = self.sparql(graph, &query).await?;
        Ok(rows
            .first()
            .and_then(|row| row.get("boolean"))
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    /// A graph name accepted by [`RdfEngine::insert`].
    pub fn graph_name(graph: &str) -> GraphName {
        GraphName::NamedNode(NamedNode::new_unchecked(graph))
    }

    /// Build a quad in `graph` from string terms.
    pub fn quad(graph: &str, subject: &str, predicate: &str, object: Term) -> Quad {
        Quad::new(
            NamedNode::new_unchecked(subject),
            NamedNode::new_unchecked(predicate),
            object,
            Self::graph_name(graph),
        )
    }

    /// The shared handle, for callers that need the raw writer.
    pub fn handle(&self) -> &mm_store_graph::GraphHandle {
        self.store.handle()
    }
}

/// The SHACL shapes directory used when opening the store. Validation is only
/// performed by callers that ask for it; a missing directory is not fatal.
fn shapes_dir(config: &EngineConfig) -> PathBuf {
    config
        .ontology_dir
        .clone()
        .unwrap_or_else(|| mm_core::Config::repo_root().join("ontology"))
        .join("shapes")
}

/// Translate a store failure into the data-layer error.
pub fn map_graph<T>(result: std::result::Result<T, mm_core::MmError>) -> Result<T> {
    result.map_err(DataError::Graph)
}
