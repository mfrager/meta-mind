//! RDF vocabulary, graph IRIs and the append-only entity helper.
//!
//! Every record is stored as a small set of triples: `rdf:type`, a JSON payload
//! (the record itself), and a revision number. Updates append a new payload at
//! a higher revision instead of deleting the old one, so the store stays
//! append-only (which is what the agentstream event log promises) and readers
//! always select the highest revision. The JSON payload keeps the RDF surface
//! stable while the Rust schema can evolve; typed projections can be added in
//! later phases without migrating existing records.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use oxrdf::{Literal, NamedNode, Term};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::RdfEngine;
use crate::error::{DataError, Result};

/// MetaMind ontology namespace.
pub const MM_NS: &str = "https://metamind.dev/ontology#";
/// Agentstream namespace.
pub const AS_NS: &str = "https://metamind.dev/agentstream#";
/// Memory namespace.
pub const MEM_NS: &str = "https://metamind.dev/memory#";
/// Data namespace.
pub const MMD_NS: &str = "https://metamind.dev/data/";
/// FOAF namespace.
pub const FOAF_NS: &str = "http://xmlns.com/foaf/0.1/";
/// SKOS namespace.
pub const SKOS_NS: &str = "http://www.w3.org/2004/02/skos/core#";
/// DCAT namespace.
pub const DCAT_NS: &str = "http://www.w3.org/ns/dcat#";
/// DCTerms namespace.
pub const DCT_NS: &str = "http://purl.org/dc/terms/";
/// DOAP namespace.
pub const DOAP_NS: &str = "http://usefulinc.com/ns/doap#";
/// PROV-O namespace.
pub const PROV_NS: &str = "http://www.w3.org/ns/prov#";
/// RDF namespace.
pub const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
/// XSD namespace.
pub const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema#";
/// RDFS namespace.
pub const RDFS_NS: &str = "http://www.w3.org/2000/01/rdf-schema#";
/// OWL namespace.
pub const OWL_NS: &str = "http://www.w3.org/2002/07/owl#";

/// `rdf:type`.
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
/// The JSON payload predicate.
pub const PAYLOAD: &str = "https://metamind.dev/ontology#payload";
/// The append revision predicate.
pub const REVISION: &str = "https://metamind.dev/ontology#revision";
/// Last-updated predicate.
pub const UPDATED_AT: &str = "https://metamind.dev/ontology#updatedAt";

/// Graph holding repository ontology axioms.
pub const ONTOLOGY_GRAPH: &str = "https://metamind.dev/graph/ontology";

/// The graph for one agentstream session.
pub fn session_graph(session_id: &str) -> String {
    format!("https://metamind.dev/agentstream/session/{session_id}")
}

/// The graph for the session index.
pub const AGENTSTREAM_INDEX_GRAPH: &str = "https://metamind.dev/agentstream/index";

/// The graph for a session's durable lifecycle/work/frame state.
pub fn session_state_graph(session_id: &str) -> String {
    format!("https://metamind.dev/graph/session/{session_id}")
}

/// The graph for a session's materialized conversation state.
pub fn conversation_graph(session_id: &str) -> String {
    format!("https://metamind.dev/graph/conversation/{session_id}")
}

/// The paginated session index.
pub const SESSION_INDEX_GRAPH: &str = "https://metamind.dev/graph/session-index";

/// A trace manifest IRI.
pub fn trace_iri(id: &str) -> String {
    format!("urn:metamind:trace:{id}")
}

/// A frame-stack IRI.
pub fn frame_stack_iri(session_id: &str) -> String {
    format!("urn:metamind:frames:{session_id}")
}

/// The graph for one compressed segment.
pub fn segment_graph(segment_id: &str) -> String {
    format!("https://metamind.dev/memory/segments/{segment_id}")
}

/// The graph listing segments for retrieval.
pub const SEGMENT_INDEX_GRAPH: &str = "https://metamind.dev/memory/segments/index";

/// The graph for one project's memory.
pub fn project_memory_graph(project_id: &str) -> String {
    format!("https://metamind.dev/memory/projects/{project_id}")
}

/// The graph for one user's memory.
pub fn user_memory_graph(user_id: &str) -> String {
    format!("https://metamind.dev/memory/users/{user_id}")
}

/// The graph for one user's home metadata.
pub fn user_graph(user_id: &str) -> String {
    format!("https://metamind.dev/users/{user_id}")
}

/// The graph for the user index.
pub const USER_INDEX_GRAPH: &str = "https://metamind.dev/users/index";

/// The graph for one project's home metadata.
pub fn project_graph(project_id: &str) -> String {
    format!("https://metamind.dev/projects/{project_id}")
}

/// The graph for the project index.
pub const PROJECT_INDEX_GRAPH: &str = "https://metamind.dev/projects/index";

/// The graph for one app's metadata.
pub fn app_graph(app_id: &str) -> String {
    format!("https://metamind.dev/apps/{app_id}")
}

/// The graph for the app index.
pub const APP_INDEX_GRAPH: &str = "https://metamind.dev/apps/index";

/// The graph for one dataset's metadata.
pub fn dataset_graph(dataset_id: &str) -> String {
    format!("https://metamind.dev/datasets/{dataset_id}")
}

/// The graph for the DCAT catalog.
pub const DATASET_CATALOG_GRAPH: &str = "https://metamind.dev/datasets/catalog";

/// A user IRI.
pub fn user_iri(id: &str) -> String {
    format!("{MMD_NS}user/{id}")
}

/// The pre-Phase-12 spelling of a user IRI, retained only for migration.
pub fn legacy_user_iri(id: &str) -> String {
    format!("urn:metamind:user:{id}")
}

/// A project IRI.
pub fn project_iri(id: &str) -> String {
    format!("{MMD_NS}project/{id}")
}

/// The pre-Phase-12 spelling of a project IRI, retained only for migration.
pub fn legacy_project_iri(id: &str) -> String {
    format!("urn:metamind:project:{id}")
}

/// An app IRI.
pub fn app_iri(id: &str) -> String {
    format!("urn:metamind:app:{id}")
}

/// A dataset IRI.
pub fn dataset_iri(id: &str) -> String {
    format!("urn:metamind:dataset:{id}")
}

/// A session IRI.
pub fn session_iri(id: &str) -> String {
    format!("urn:metamind:session:{id}")
}

/// A turn IRI.
pub fn turn_iri(id: &str) -> String {
    format!("urn:metamind:turn:{id}")
}

/// A segment IRI.
pub fn segment_iri(id: &str) -> String {
    format!("urn:metamind:segment:{id}")
}

/// An entity IRI in the agentstream namespace.
pub fn event_iri(id: &str) -> String {
    format!("urn:metamind:event:{id}")
}

/// Current wall-clock time in Unix milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

static REVISION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A process-monotonic revision, seeded with wall-clock milliseconds so it keeps
/// increasing across restarts even when writes land in the same millisecond.
pub fn next_revision() -> u64 {
    let now = now_ms().max(0) as u64;
    let mut current = REVISION_COUNTER.load(Ordering::Relaxed);
    loop {
        let next = current.saturating_add(1).max(now << 16);
        match REVISION_COUNTER.compare_exchange_weak(
            current,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(observed) => current = observed,
        }
    }
}

/// A simple string literal.
pub fn lit(value: impl Into<String>) -> Term {
    Term::Literal(Literal::new_simple_literal(value))
}

/// An IRI term.
pub fn iri(value: impl Into<String>) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(value))
}

/// A typed literal; `datatype` is a full IRI.
pub fn typed_lit(value: impl Into<String>, datatype: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(
        value,
        NamedNode::new_unchecked(datatype),
    ))
}

/// Read a term rendered by `mm-store-graph::term_to_json` back to its lexical
/// value. Typed and language literals are represented as objects.
pub fn term_value(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => Some(text.as_str()),
        Value::Object(map) => map.get("value").and_then(Value::as_str),
        _ => None,
    }
}

/// Insert (append) an entity's current payload and revision.
///
/// The `class` is written as `rdf:type`; `name` is written as `mm:name` when
/// supplied so indexes can be inspected without decoding every payload.
pub async fn upsert(
    engine: &RdfEngine,
    graph: &str,
    subject: &str,
    class: &str,
    name: Option<&str>,
    payload: &Value,
) -> Result<()> {
    let revision = next_revision();
    let mut stamped = payload.clone();
    if let Value::Object(map) = &mut stamped {
        map.insert("_rev".to_owned(), Value::from(revision));
    }
    let mut quads = vec![
        RdfEngine::quad(
            graph,
            subject,
            RDF_TYPE,
            Term::NamedNode(NamedNode::new_unchecked(class)),
        ),
        RdfEngine::quad(
            graph,
            subject,
            PAYLOAD,
            lit(serde_json::to_string(&stamped)?),
        ),
        RdfEngine::quad(
            graph,
            subject,
            REVISION,
            typed_lit(revision.to_string(), &format!("{XSD_NS}unsignedLong")),
        ),
        RdfEngine::quad(
            graph,
            subject,
            UPDATED_AT,
            typed_lit(now_ms().to_string(), &format!("{XSD_NS}dateTime")),
        ),
    ];
    if let Some(name) = name {
        quads.push(RdfEngine::quad(
            graph,
            subject,
            &format!("{MM_NS}name"),
            lit(name),
        ));
    }
    engine.insert_all(quads).await
}

/// The graph IRI a name resolves to, so a query names the same graph the
/// store's scoping rule checks for.
fn graph_iri(graph: &str) -> String {
    mm_store_graph::graph_iri(graph)
}

/// The payload with the highest `_rev`.
fn newest_payload(payloads: impl IntoIterator<Item = String>) -> Option<Value> {
    let mut best: Option<(u64, Value)> = None;
    for text in payloads {
        let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let revision = value
            .get("_rev")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if let Value::Object(map) = &mut value {
            map.remove("_rev");
        }
        if best
            .as_ref()
            .is_none_or(|(current, _)| revision >= *current)
        {
            best = Some((revision, value));
        }
    }
    best.map(|(_, value)| value)
}

/// Load the payload of the highest revision of `subject`.
///
/// The read names the subject and predicate in the query. Reading the whole
/// graph here and filtering in Rust made every lookup a full-graph scan, and
/// [`list_as`] calls this once per subject, so a status snapshot cost
/// `O(subjects)` full scans of the graph.
pub async fn load(engine: &RdfEngine, graph: &str, subject: &str) -> Result<Option<Value>> {
    let query = format!(
        "SELECT ?o WHERE {{ GRAPH <{}> {{ <{subject}> <{PAYLOAD}> ?o }} }}",
        graph_iri(graph)
    );
    let rows = engine.sparql(graph, &query).await?;
    Ok(newest_payload(
        rows.iter()
            .filter_map(|row| row.get("o").and_then(term_value))
            .map(str::to_owned),
    ))
}

/// Load and decode the highest-revision payload of `subject`.
pub async fn load_as<T: DeserializeOwned>(
    engine: &RdfEngine,
    graph: &str,
    subject: &str,
) -> Result<Option<T>> {
    match load(engine, graph, subject).await? {
        Some(value) => serde_json::from_value(value)
            .map(Some)
            .map_err(|error| DataError::Rdf(format!("decode {subject}: {error}"))),
        None => Ok(None),
    }
}

/// The subjects in `graph` typed as `class`.
pub async fn subjects_of_class(
    engine: &RdfEngine,
    graph: &str,
    class: &str,
) -> Result<Vec<String>> {
    let query = format!(
        "SELECT DISTINCT ?s WHERE {{ GRAPH <{}> {{ ?s <{RDF_TYPE}> <{class}> }} }}",
        graph_iri(graph)
    );
    let rows = engine.sparql(graph, &query).await?;
    let mut out: Vec<String> = rows
        .iter()
        .filter_map(|row| row.get("s").and_then(term_value))
        .map(str::to_owned)
        .collect();
    out.sort();
    out.dedup();
    Ok(out)
}

/// Load every entity of `class` in `graph`.
pub async fn list_as<T: DeserializeOwned>(
    engine: &RdfEngine,
    graph: &str,
    class: &str,
) -> Result<Vec<T>> {
    // One query for the whole class: its subjects and every revision of their
    // payloads. The subject-at-a-time shape this replaces cost one full-graph
    // read per row, which is quadratic on the session and index graphs.
    let query = format!(
        "SELECT ?s ?o WHERE {{ GRAPH <{}> {{ ?s <{RDF_TYPE}> <{class}> . ?s <{PAYLOAD}> ?o }} }}",
        graph_iri(graph)
    );
    let rows = engine.sparql(graph, &query).await?;
    let mut payloads: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for row in rows {
        let (Some(subject), Some(text)) = (
            row.get("s").and_then(term_value),
            row.get("o").and_then(term_value),
        ) else {
            continue;
        };
        payloads
            .entry(subject.to_owned())
            .or_default()
            .push(text.to_owned());
    }
    // `BTreeMap` iterates subjects in sorted order, which is the order the
    // previous sorted-subject walk produced.
    let mut out = Vec::with_capacity(payloads.len());
    for (subject, revisions) in payloads {
        let Some(value) = newest_payload(revisions) else {
            continue;
        };
        out.push(serde_json::from_value(value).map_err(|error| {
            DataError::Rdf(format!("decode {subject}: {error}"))
        })?);
    }
    Ok(out)
}

/// A record shared by every registry: its identifier and when it was created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordMeta {
    /// Creation time, Unix milliseconds.
    pub created_at: i64,
    /// Last update, Unix milliseconds.
    pub updated_at: i64,
}

impl Default for RecordMeta {
    fn default() -> Self {
        let now = now_ms();
        Self {
            created_at: now,
            updated_at: now,
        }
    }
}

/// Escape a value for a Turtle string literal.
pub fn turtle_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Generate a lowercase ULID.
pub fn new_id() -> String {
    ulid::Ulid::gen().to_string().to_ascii_lowercase()
}
