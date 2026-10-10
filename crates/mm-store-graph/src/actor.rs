//! The RDF store actor.
//!
//! Oxigraph's `Store` is synchronous and owns a single write path. Metamind
//! therefore funnels every write through **one dedicated writer thread** with a
//! bounded mailbox, so producers apply natural backpressure instead of queueing
//! without limit. Reads use a second handle to the same `Store` on
//! `spawn_blocking`; Oxigraph reads are thread-safe and never take the mailbox.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mm_core::store::{Graph, ShaclReport, ShaclViolation};
use mm_core::MmError;
use oxigraph::io::{RdfFormat, RdfParser};
use oxigraph::model::{GraphName, GraphNameRef, NamedNode, NamedOrBlankNode, Quad};
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};

use crate::graphs::{canonical_hash_of_turtle, term_to_json};
use crate::shacl::validate_turtle_text;

/// The mailbox capacity: writers wait once this many commands are queued.
pub const MAILBOX_CAPACITY: usize = 1024;

enum GraphCmd {
    Insert {
        quad: Quad,
        ack: oneshot::Sender<Result<(), MmError>>,
    },
    InsertTurtle {
        graph: String,
        turtle: String,
        ack: oneshot::Sender<Result<usize, MmError>>,
    },
    ClearGraph {
        graph: String,
        ack: oneshot::Sender<Result<usize, MmError>>,
    },
    Sparql {
        graph: String,
        query: String,
        ack: oneshot::Sender<Result<Vec<Value>, MmError>>,
    },
    LoadOntology {
        files: Vec<PathBuf>,
        ack: oneshot::Sender<Result<usize, MmError>>,
    },
    Flush {
        ack: oneshot::Sender<Result<(), MmError>>,
    },
    Shutdown {
        ack: oneshot::Sender<()>,
    },
}

/// A cloneable write handle to the graph store.
#[derive(Debug, Clone)]
pub struct GraphHandle {
    tx: mpsc::Sender<GraphCmd>,
}

impl GraphHandle {
    /// Insert one quad (the quad carries its own graph name).
    pub async fn insert(&self, quad: Quad) -> Result<(), MmError> {
        let (ack, rx) = oneshot::channel();
        self.send(GraphCmd::Insert { quad, ack }).await?;
        rx.await
            .map_err(|e| MmError::Graph(format!("graph actor dropped the ack: {e}")))?
    }

    /// Parse Turtle and insert it into `graph` (a bare name or a full graph IRI).
    ///
    /// The document is parsed as bare Turtle and each resulting triple is
    /// re-homed into the named graph; the text is never wrapped in `GRAPH { … }`,
    /// because TriG forbids `@prefix` inside a graph block while every Turtle
    /// fixture begins with one.
    pub async fn insert_turtle(&self, graph: &str, turtle: &str) -> Result<usize, MmError> {
        let (ack, rx) = oneshot::channel();
        self.send(GraphCmd::InsertTurtle {
            graph: graph.to_string(),
            turtle: turtle.to_string(),
            ack,
        })
        .await?;
        rx.await
            .map_err(|e| MmError::Graph(format!("graph actor dropped the ack: {e}")))?
    }

    /// Remove every quad in a named graph, returning how many were removed.
    ///
    /// A projection that is rewritten from scratch each time (the `/code` graph) is
    /// cleared first: re-inserting an unchanged document is idempotent in RDF, but a
    /// node that disappeared from the source would otherwise linger forever.
    pub async fn clear_graph(&self, graph: &str) -> Result<usize, MmError> {
        let (ack, rx) = oneshot::channel();
        self.send(GraphCmd::ClearGraph {
            graph: graph.to_string(),
            ack,
        })
        .await?;
        rx.await
            .map_err(|e| MmError::Graph(format!("graph actor dropped the ack: {e}")))?
    }

    /// Run a SPARQL query scoped to one named graph.
    ///
    /// Reads normally go through [`GraphStore`]'s own reader handle, which this type does
    /// not hold: a `GraphHandle` is the *write* half. A caller that holds only the handle
    /// — Phase 10's `/tools` mirror and the `graph.query` tool — therefore has its reads
    /// travel the same mailbox its writes do. The query runs on the writer thread, which
    /// briefly delays writes; that is the price of a handle that owns no store, and it is
    /// the honest one, because the alternative is a second store handle per call site.
    pub async fn sparql(&self, graph: &str, query: &str) -> Result<Vec<Value>, MmError> {
        let (ack, rx) = oneshot::channel();
        self.send(GraphCmd::Sparql {
            graph: graph.to_string(),
            query: query.to_string(),
            ack,
        })
        .await?;
        rx.await
            .map_err(|e| MmError::Graph(format!("graph actor dropped the ack: {e}")))?
    }

    /// Bulk-load ontology files into the default graph as one atomic batch.
    pub async fn load_ontology(&self, files: Vec<PathBuf>) -> Result<usize, MmError> {
        let (ack, rx) = oneshot::channel();
        self.send(GraphCmd::LoadOntology { files, ack }).await?;
        rx.await
            .map_err(|e| MmError::Graph(format!("graph actor dropped the ack: {e}")))?
    }

    /// Flush the graph transaction.
    pub async fn flush(&self) -> Result<(), MmError> {
        let (ack, rx) = oneshot::channel();
        self.send(GraphCmd::Flush { ack }).await?;
        rx.await
            .map_err(|e| MmError::Graph(format!("graph actor dropped the ack: {e}")))?
    }

    /// Stop the writer and wait for it.
    pub async fn shutdown(&self) -> Result<(), MmError> {
        let (ack, rx) = oneshot::channel();
        self.send(GraphCmd::Shutdown { ack }).await?;
        let _ = rx.await;
        Ok(())
    }

    async fn send(&self, cmd: GraphCmd) -> Result<(), MmError> {
        self.tx
            .send(cmd)
            .await
            .map_err(|_| MmError::Graph("graph writer is no longer running".into()))
    }
}

fn writer_loop(store: Arc<Store>, mut rx: mpsc::Receiver<GraphCmd>) {
    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            GraphCmd::Insert { quad, ack } => {
                let _ = ack.send(store.insert(&quad).map_err(map_store));
            }
            GraphCmd::InsertTurtle { graph, turtle, ack } => {
                let _ = ack.send(parse_and_insert(&store, &graph, &turtle));
            }
            GraphCmd::ClearGraph { graph, ack } => {
                let _ = ack.send(clear_graph(&store, &graph));
            }
            GraphCmd::Sparql { graph, query, ack } => {
                let _ = ack.send(scoped_query(&store, &graph, &query));
            }
            GraphCmd::LoadOntology { files, ack } => {
                let _ = ack.send(load_ontology_files(&store, &files));
            }
            GraphCmd::Flush { ack } => {
                let _ = ack.send(store.flush().map_err(map_store));
            }
            GraphCmd::Shutdown { ack } => {
                let _ = ack.send(());
                break;
            }
        }
    }
}

/// Parse a Turtle document and insert every triple into `graph`, ignoring the
/// graph names in the source (bare Turtle has none).
fn parse_and_insert(store: &Store, graph: &str, turtle: &str) -> Result<usize, MmError> {
    let name = named_graph(graph)?;
    let parser = RdfParser::from_format(RdfFormat::Turtle);
    let mut n = 0usize;
    for quad in parser.for_slice(turtle.as_bytes()) {
        let quad = quad.map_err(|e| MmError::Codec(format!("turtle parse error: {e}")))?;
        let quad = Quad::new(
            quad.subject,
            quad.predicate,
            quad.object,
            GraphName::NamedNode(name.clone()),
        );
        store.insert(&quad).map_err(map_store)?;
        n += 1;
    }
    Ok(n)
}

/// Remove every quad in a named graph.
fn clear_graph(store: &Store, graph: &str) -> Result<usize, MmError> {
    let name = named_graph(graph)?;
    let graph_ref = GraphNameRef::NamedNode((&name).into());
    let mut removed = 0usize;
    for quad in store.quads_for_pattern(None, None, None, Some(graph_ref)) {
        quad.map_err(map_store)?;
        removed += 1;
    }
    store.clear_graph(graph_ref).map_err(map_store)?;
    Ok(removed)
}

fn load_ontology_files(store: &Store, files: &[PathBuf]) -> Result<usize, MmError> {
    if files.is_empty() {
        return Ok(0);
    }
    let before = store.len().map_err(map_store)?;
    let mut loader = store.bulk_loader();
    for file in files {
        let mut bytes = Vec::new();
        std::fs::File::open(file)
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .map_err(|e| MmError::Store(format!("cannot read {}: {e}", file.display())))?;
        loader
            .load_from_slice(RdfFormat::Turtle, &bytes)
            .map_err(|e| MmError::Codec(format!("{}: {e}", file.display())))?;
    }
    // One commit: either the whole ontology lands or none of it does.
    loader
        .commit()
        .map_err(|e| MmError::Graph(format!("ontology commit failed: {e}")))?;
    let after = store.len().map_err(map_store)?;
    Ok(after.saturating_sub(before))
}

fn map_store(e: impl std::fmt::Display) -> MmError {
    MmError::Graph(e.to_string())
}

/// The RDF store: a write handle plus a read handle to the same `Store`.
pub struct GraphStore {
    handle: GraphHandle,
    reader: Arc<Store>,
    /// The writer thread, held so [`GraphStore`]'s `Drop` can join it.
    writer: Option<std::thread::JoinHandle<()>>,
    /// Where the RocksDB files live, or `None` for an in-memory store.
    path: Option<PathBuf>,
    shapes_path: PathBuf,
}

/// Stop the writer, and wait for it.
///
/// Two threads hold the `Store` — this one and the writer — so whichever drops
/// its reference last is the one that destroys it. Without this, that can be the
/// wrong one: the process exits, the writer is still inside RocksDB, and the
/// store's internal mutexes are torn down underneath a live call, which glibc
/// reports as `pthread lock: Invalid argument` and an abort *after* the command
/// has already printed its result. Asking the writer to stop and joining it drops
/// the writer's reference here — so the store is destroyed once, in one thread,
/// with no writer mid-operation.
impl Drop for GraphStore {
    fn drop(&mut self) {
        // `Drop` cannot await, so the shutdown goes out with `try_send`. The
        // mailbox is empty by the time a command finishes with a store, so the
        // first attempt is normally the one that lands. A full mailbox means a
        // clone of the handle is mid-send from another thread, which is a reason
        // to wait a moment rather than to block forever; and if it never drains,
        // the channel's own close is what stops the writer, so the join is skipped
        // rather than allowed to hang.
        let mut asked = false;
        for _ in 0..64 {
            let (ack, _unread) = oneshot::channel();
            match self.handle.tx.try_send(GraphCmd::Shutdown { ack }) {
                Ok(()) => {
                    asked = true;
                    break;
                }
                // The writer has already stopped: there is nothing left to ask.
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    asked = true;
                    break;
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
        if asked {
            if let Some(writer) = self.writer.take() {
                let _ = writer.join();
            }
        }
    }
}

impl std::fmt::Debug for GraphStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphStore")
            .field("path", &self.path)
            .field("shapes_path", &self.shapes_path)
            .finish_non_exhaustive()
    }
}

impl GraphStore {
    /// Open a persistent store at `path`, validating against `shapes_path`.
    pub async fn open(path: &Path, shapes_path: &Path) -> Result<Self, MmError> {
        std::fs::create_dir_all(path)?;
        let store = Store::open(path)
            .map_err(|e| MmError::Graph(format!("cannot open {}: {e}", path.display())))?;
        Self::from_store(store, Some(path.to_path_buf()), shapes_path)
    }

    /// An in-memory store, for tests.
    pub async fn in_memory(shapes_path: &Path) -> Result<Self, MmError> {
        let store = Store::new().map_err(map_store)?;
        Self::from_store(store, None, shapes_path)
    }

    fn from_store(
        store: Store,
        path: Option<PathBuf>,
        shapes_path: &Path,
    ) -> Result<Self, MmError> {
        let store = Arc::new(store);
        let (tx, rx) = mpsc::channel(MAILBOX_CAPACITY);
        let writer = Arc::clone(&store);
        let writer = std::thread::Builder::new()
            .name("mm-graph-writer".into())
            .spawn(move || writer_loop(writer, rx))
            .map_err(|e| MmError::Graph(format!("cannot start the graph writer: {e}")))?;
        Ok(GraphStore {
            handle: GraphHandle { tx },
            reader: store,
            writer: Some(writer),
            path,
            shapes_path: shapes_path.to_path_buf(),
        })
    }

    /// The write handle.
    pub fn handle(&self) -> &GraphHandle {
        &self.handle
    }

    /// The storage directory, if this store is persistent.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The shapes graph used by [`Graph::validate`].
    pub fn shapes_path(&self) -> &Path {
        &self.shapes_path
    }

    /// Total quads (default graph triples included).
    pub async fn quad_count(&self) -> Result<usize, MmError> {
        let store = Arc::clone(&self.reader);
        tokio::task::spawn_blocking(move || store.len().map_err(map_store))
            .await
            .map_err(|e| MmError::Internal(format!("read task failed: {e}")))?
    }

    /// Triples in the default graph (the T-Box).
    ///
    /// This deliberately passes `GraphNameRef::DefaultGraph` rather than `None`:
    /// in Oxigraph `None` means *any* graph, which would fold every runtime graph
    /// into the ontology count.
    pub async fn ontology_triple_count(&self) -> Result<usize, MmError> {
        let store = Arc::clone(&self.reader);
        tokio::task::spawn_blocking(move || {
            let mut n = 0usize;
            for quad in store.quads_for_pattern(None, None, None, Some(GraphNameRef::DefaultGraph))
            {
                quad.map_err(map_store)?;
                n += 1;
            }
            Ok(n)
        })
        .await
        .map_err(|e| MmError::Internal(format!("read task failed: {e}")))?
    }

    /// Quads in one named graph.
    pub async fn graph_quad_count(&self, graph: &str) -> Result<usize, MmError> {
        let name = named_graph(graph)?;
        self.count_pattern(Some(name)).await
    }

    async fn count_pattern(&self, graph: Option<NamedNode>) -> Result<usize, MmError> {
        let store = Arc::clone(&self.reader);
        tokio::task::spawn_blocking(move || {
            let mut n = 0usize;
            let name = graph.map(GraphName::NamedNode);
            for quad in store.quads_for_pattern(None, None, None, name.as_ref().map(|g| g.as_ref()))
            {
                quad.map_err(map_store)?;
                n += 1;
            }
            Ok(n)
        })
        .await
        .map_err(|e| MmError::Internal(format!("read task failed: {e}")))?
    }

    /// Every named graph in the store, as full IRIs.
    pub async fn named_graphs(&self) -> Result<Vec<String>, MmError> {
        let store = Arc::clone(&self.reader);
        tokio::task::spawn_blocking(move || {
            let mut names = Vec::new();
            for name in store.named_graphs() {
                match name.map_err(map_store)? {
                    NamedOrBlankNode::NamedNode(node) => names.push(node.into_string()),
                    NamedOrBlankNode::BlankNode(node) => names.push(format!("_:{node}")),
                }
            }
            names.sort();
            Ok(names)
        })
        .await
        .map_err(|e| MmError::Internal(format!("read task failed: {e}")))?
    }

    /// Serialize one named graph as Turtle.
    pub async fn dump_turtle(&self, graph: &str) -> Result<String, MmError> {
        let store = Arc::clone(&self.reader);
        let name = named_graph(graph)?;
        tokio::task::spawn_blocking(move || {
            let bytes = store
                .dump_graph_to_writer(
                    GraphNameRef::NamedNode(name.as_ref()),
                    RdfFormat::Turtle,
                    Vec::new(),
                )
                .map_err(map_store)?;
            String::from_utf8(bytes)
                .map_err(|e| MmError::Codec(format!("turtle dump is not UTF-8: {e}")))
        })
        .await
        .map_err(|e| MmError::Internal(format!("read task failed: {e}")))?
    }

    /// Replace a named graph's contents with a Turtle document.
    ///
    /// The graph is cleared first so a node that vanished from the source cannot
    /// survive in the projection. Returns how many triples the document inserted.
    pub async fn replace_turtle(&self, graph: &str, turtle: &str) -> Result<usize, MmError> {
        self.handle.clear_graph(graph).await?;
        self.handle.insert_turtle(graph, turtle).await
    }

    /// The canonical content hash of one named graph.
    ///
    /// Deterministic serialization is what makes replay and diffing possible:
    /// two graphs with the same triples hash the same regardless of insertion
    /// order or blank-node labels.
    pub async fn canonical_hash(&self, graph: &str) -> Result<String, MmError> {
        let turtle = self.dump_turtle(graph).await?;
        canonical_hash_of_turtle(&turtle)
    }

    /// The canonical, order-independent N-Triples rendering of one named graph.
    pub async fn canonical_ntriples(&self, graph: &str) -> Result<String, MmError> {
        let turtle = self.dump_turtle(graph).await?;
        crate::graphs::canonical_ntriples_of_turtle(&turtle)
    }

    /// Validate one named graph against a shapes file.
    pub async fn validate_with(
        &self,
        graph: &str,
        shapes_path: &Path,
    ) -> Result<ShaclReport, MmError> {
        let shapes = std::fs::read_to_string(shapes_path).map_err(|e| {
            MmError::Graph(format!("cannot read shapes {}: {e}", shapes_path.display()))
        })?;
        let data = self.dump_turtle(graph).await?;
        validate_turtle_text(&shapes, &data, graph)
    }

    /// Validate one named graph against the configured shapes.
    pub async fn validate_named_graph(&self, graph: &str) -> Result<ShaclReport, MmError> {
        let shapes = self.shapes_path.clone();
        self.validate_with(graph, &shapes).await
    }

    /// Oxigraph's own internal storage self-check.
    pub async fn storage_invariants_ok(&self) -> Result<bool, MmError> {
        let store = Arc::clone(&self.reader);
        tokio::task::spawn_blocking(move || store.validate().map(|_| true).map_err(map_store))
            .await
            .map_err(|e| MmError::Internal(format!("read task failed: {e}")))?
    }

    /// Run a SPARQL query and return rows as JSON objects.
    ///
    /// The query must name the graph it reads: either `GRAPH <iri> { … }` or
    /// `FROM <iri>`. A query that mentions no graph is rejected rather than
    /// silently reading across all of them, because no fact is authoritative in
    /// two graphs.
    pub async fn sparql(&self, graph: &str, query: &str) -> Result<Vec<Value>, MmError> {
        let store = Arc::clone(&self.reader);
        let graph = graph.to_string();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || scoped_query(&store, &graph, &query))
            .await
            .map_err(|e| MmError::Internal(format!("read task failed: {e}")))?
    }

    /// Read every triple of a named graph as JSON objects.
    pub async fn triples(&self, graph: &str) -> Result<Vec<Value>, MmError> {
        let name = named_graph(graph)?;
        let query = format!(
            "SELECT ?s ?p ?o WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }}",
            name.as_str()
        );
        let mut rows = self.sparql(graph, &query).await?;
        rows.sort_by_key(|r| r.to_string());
        Ok(rows)
    }

    /// Shut the writer down cleanly.
    pub async fn shutdown(&self) -> Result<(), MmError> {
        self.handle.shutdown().await
    }
}

/// The full IRI of a named graph, accepting either a bare name or a full IRI.
pub fn graph_iri(graph: &str) -> String {
    if graph.starts_with("http") {
        graph.to_string()
    } else {
        mm_core::iri::graph(graph)
    }
}

fn named_graph(graph: &str) -> Result<NamedNode, MmError> {
    NamedNode::new(graph_iri(graph))
        .map_err(|e| MmError::Graph(format!("invalid graph name {graph:?}: {e}")))
}

/// Run a query that must name the graph it reads.
///
/// The scoping rule lives here, in one function, because both read paths use it: the
/// store's own reader handle and the write handle's query command. A query that mentions
/// no graph is rejected rather than silently reading across all of them, because no fact
/// is authoritative in two graphs.
fn scoped_query(store: &Store, graph: &str, query: &str) -> Result<Vec<Value>, MmError> {
    let name = named_graph(graph)?;
    let iri = name.as_str();
    if !query.contains(iri) {
        return Err(MmError::Graph(format!(
            "query is not scoped to <{iri}>: name the graph with GRAPH <{iri}> {{ … }} or FROM <{iri}>"
        )));
    }
    run_query(store, query)
}

fn run_query(store: &Store, query: &str) -> Result<Vec<Value>, MmError> {
    let results = SparqlEvaluator::new()
        .parse_query(query)
        .map_err(|e| MmError::Codec(format!("SPARQL syntax error: {e}")))?
        .on_store(store)
        .execute()
        .map_err(|e| MmError::Graph(format!("SPARQL evaluation failed: {e}")))?;
    match results {
        QueryResults::Solutions(iter) => {
            let mut rows = Vec::new();
            for solution in iter {
                let solution =
                    solution.map_err(|e| MmError::Graph(format!("SPARQL result error: {e}")))?;
                let mut map = Map::new();
                for (variable, term) in solution.iter() {
                    map.insert(variable.as_str().to_string(), term_to_json(term));
                }
                rows.push(Value::Object(map));
            }
            Ok(rows)
        }
        QueryResults::Boolean(answer) => Ok(vec![serde_json::json!({ "boolean": answer })]),
        QueryResults::Graph(iter) => {
            let mut rows = Vec::new();
            for triple in iter {
                let triple =
                    triple.map_err(|e| MmError::Graph(format!("SPARQL result error: {e}")))?;
                rows.push(serde_json::json!({
                    "s": term_to_json(&triple.subject.into()),
                    "p": term_to_json(&triple.predicate.into()),
                    "o": term_to_json(&triple.object),
                }));
            }
            Ok(rows)
        }
    }
}

#[async_trait::async_trait]
impl Graph for GraphStore {
    async fn insert(&self, graph: &str, quad: Quad) -> Result<(), MmError> {
        let name = named_graph(graph)?;
        let quad = Quad::new(
            quad.subject,
            quad.predicate,
            quad.object,
            GraphName::NamedNode(name),
        );
        self.handle.insert(quad).await
    }

    async fn sparql(&self, graph: &str, query: &str) -> Result<Vec<Value>, MmError> {
        GraphStore::sparql(self, graph, query).await
    }

    async fn validate(&self, graph: &str) -> Result<ShaclReport, MmError> {
        self.validate_named_graph(graph).await
    }
}

/// Map the vendored validator's report onto the kernel's shape.
pub fn map_shacl_report(report: rdf_shacl::ShaclReport) -> ShaclReport {
    ShaclReport {
        conforms: report.conforms,
        violations: report
            .results
            .into_iter()
            .map(|v| ShaclViolation {
                path: v.path,
                message: v.message,
                severity: v.severity,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURTLE: &str = r#"
        @prefix mm:  <https://metamind.dev/ontology#> .
        @prefix mmd: <https://metamind.dev/data/> .
        mmd:identity01 a mm:Identity ;
            mm:iri mmd:identity01 .
    "#;

    async fn store() -> GraphStore {
        GraphStore::in_memory(Path::new("unused-shapes.ttl"))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn insert_turtle_routes_its_triples_into_the_named_graph() {
        let store = store().await;
        let inserted = store.handle().insert_turtle("being", TURTLE).await.unwrap();
        assert_eq!(inserted, 2, "the fixture declares two triples");

        // The triples live in `/being`, and the default graph stays T-Box-only.
        assert_eq!(store.ontology_triple_count().await.unwrap(), 0);
        assert_eq!(store.graph_quad_count("being").await.unwrap(), 2);
        assert_eq!(store.triples("being").await.unwrap().len(), 2);
        store.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn replace_turtle_drops_triples_that_are_no_longer_present() {
        let store = store().await;
        store
            .handle()
            .insert_turtle("code", "<https://a> <https://p> <https://o> .")
            .await
            .unwrap();
        let inserted = store
            .replace_turtle("code", "<https://b> <https://p> <https://o> .")
            .await
            .unwrap();
        assert_eq!(inserted, 1);
        assert_eq!(store.graph_quad_count("code").await.unwrap(), 1);
        let rows = store.triples("code").await.unwrap();
        assert!(rows[0].to_string().contains("https://b"), "{rows:?}");
        store.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn insert_turtle_rejects_a_malformed_document() {
        let store = store().await;
        let err = store
            .handle()
            .insert_turtle("being", "this is not turtle .")
            .await
            .unwrap_err();
        assert!(matches!(err, MmError::Codec(_)), "got {err:?}");
        assert_eq!(store.graph_quad_count("being").await.unwrap(), 0);
        store.shutdown().await.unwrap();
    }
}
