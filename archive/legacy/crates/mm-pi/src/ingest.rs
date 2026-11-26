//! Ingesting a recorded session: the rows, and the `/code` mirror.
//!
//! Two jobs, and the order matters. First the session becomes kernel records —
//! `pi_sessions` for the session, `pi_events` for every record, gapless by `seq` — so the
//! stream is archived exactly as the agent wrote it. Then the session becomes RDF in the
//! `/code` graph, so a module can be traced to the session that generated it.
//!
//! Four decisions carry the module:
//!
//! * **The session's ULID is the one in its header.** The ingest does not mint an
//!   identity: the file already names its session, and re-ingesting the same file must
//!   land on the same row. That is what makes `pi ingest` safe to run twice, and it is
//!   also what lets a change set point at the session that produced it.
//! * **`pi_events` holds the bytes, not a re-rendering.** Each row's `payload_json` is the
//!   line the agent sent. A parsed copy would be easier to query and worse as evidence.
//! * **Events are written `ON CONFLICT DO NOTHING`.** A second ingest of the same file is
//!   a no-op per sequence number rather than a duplicate stream. The session row is
//!   upserted, so a re-ingest can add the end time a running session did not have.
//! * **The mirror never clears the graph.** `/code` also holds the Phase 2 scan's modules,
//!   files and symbols, so this writes into it rather than replacing it. (The scan itself
//!   rewrites `/code` from the tree; a `codex scan` therefore drops session nodes, and
//!   `pi ingest` is what puts them back. The alternative — a second graph for session
//!   metadata — would split the code graph in two for one node type.)
//!
//! The graph vocabulary is `ontology/code.ttl`'s `Pi sessions (Phase 11)` block, and the
//! constraints are `mmc:PiSessionShape`, `mmc:EditShape` and `mmc:ToolCallShape` in
//! `ontology/shapes/mmc-shapes.ttl`: a session states its file, its start and its event
//! count; an edit states its path, its sequence and the session that made it; a tool call
//! states its tool, its sequence and its session. Integers are written as
//! `"…"^^xsd:integer`, because a plain literal would be an `xsd:string` and the shapes ask
//! for an integer.

use std::path::{Path, PathBuf};

use mm_core::{iri, ulid_string, Param, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;
use serde_json::json;

use crate::error::{PiError, Result};
use crate::session::{parse_session_file, ParsedSession, PiRecord};
// The edit and tool-call views are what a caller building a change set needs, so they are
// part of this module's surface rather than a second name for the session module's.
pub use crate::session::{PiEdit, PiToolCall};
use crate::TARGET;

/// The named graph session metadata is mirrored into.
pub const CODE_GRAPH: &str = "code";

/// The `mmc:` namespace.
const MMC: &str = "https://metamind.dev/code#";
/// The `xsd:` namespace, for typed integer literals.
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
/// The RDF namespace, for `rdf:type`.
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

const INSERT_SESSION: &str = "INSERT INTO pi_sessions \
 (id, session_file, changeset_ulid, model, cwd, started_at, ended_at, event_count) \
 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
 ON CONFLICT(id) DO UPDATE SET \
   session_file = excluded.session_file, \
   changeset_ulid = COALESCE(excluded.changeset_ulid, pi_sessions.changeset_ulid), \
   model = excluded.model, \
   cwd = excluded.cwd, \
   ended_at = excluded.ended_at, \
   event_count = excluded.event_count";

const INSERT_EVENT: &str = "INSERT INTO pi_events (id, session_ulid, seq, kind, payload_json) \
 VALUES (?, ?, ?, ?, ?) \
 ON CONFLICT(session_ulid, seq) DO NOTHING";

/// A recorded session, parsed and ready to be written.
#[derive(Clone, Debug, PartialEq)]
pub struct PiSessionGraph {
    /// The session's ULID, from its header.
    pub session_id: Ulid,
    /// The file the session was read from.
    pub session_file: PathBuf,
    /// The model that ran.
    pub model: String,
    /// The directory the agent edited in.
    pub cwd: PathBuf,
    /// When it started.
    pub started_at: Timestamp,
    /// When it ended, if it did.
    pub ended_at: Option<Timestamp>,
    /// How many records the file held.
    pub event_count: u32,
    /// The session this one continues, when its first record names one.
    pub parent_session: Option<Ulid>,
    /// The records, in stream order.
    pub records: Vec<PiRecord>,
    /// The raw line for each record, aligned with `records`.
    pub raw_records: Vec<String>,
    /// The edits the session made.
    pub edits: Vec<PiEdit>,
    /// The tools it invoked.
    pub tool_calls: Vec<PiToolCall>,
    /// The change set this session's output was assembled into, when known.
    pub changeset: Option<Ulid>,
}

impl PiSessionGraph {
    /// Build from an already-parsed session.
    pub fn from_parsed(session_file: PathBuf, parsed: ParsedSession) -> Self {
        let edits = parsed.edits();
        let tool_calls = parsed.tool_calls();
        PiSessionGraph {
            session_id: parsed.header.session_id,
            session_file,
            model: parsed.header.model.clone(),
            cwd: parsed.header.cwd.clone(),
            started_at: parsed.header.started_at,
            ended_at: parsed.ended_at,
            event_count: parsed.event_count(),
            parent_session: parsed.parent_session(),
            records: parsed.records,
            raw_records: parsed.raw,
            edits,
            tool_calls,
            changeset: None,
        }
    }

    /// Read and parse a session file.
    pub fn parse(path: &Path) -> Result<Self> {
        let parsed = parse_session_file(path)?;
        Ok(PiSessionGraph::from_parsed(path.to_path_buf(), parsed))
    }

    /// Read and parse a session file, recording the change set it belongs to.
    pub fn parse_for(path: &Path, changeset: Option<Ulid>) -> Result<Self> {
        let mut graph = PiSessionGraph::parse(path)?;
        graph.changeset = changeset;
        Ok(graph)
    }

    /// The session's instance IRI.
    pub fn session_iri(&self) -> String {
        iri::data(&self.session_id).into_string()
    }

    /// The files the session edited, in stream order, without repeats.
    pub fn edited_paths(&self) -> Vec<PathBuf> {
        let mut seen: Vec<PathBuf> = Vec::new();
        for edit in &self.edits {
            if !seen.contains(&edit.path) {
                seen.push(edit.path.clone());
            }
        }
        seen
    }

    /// A deterministic Turtle rendering of the session, its edits and its tool calls.
    ///
    /// Deterministic in the sense a replay needs: the same records render to the same
    /// bytes, statement for statement, so two ingestions compare equal without a
    /// canonicalizer in between.
    pub fn canonical_turtle(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("@prefix mmc: <{MMC}> .\n"));
        out.push_str(&format!("@prefix xsd: <{XSD}> .\n"));
        out.push_str(&format!("@prefix rdf: <{RDF}> .\n\n"));

        let session = self.session_iri();
        // The properties are collected and then terminated, so the statement's final
        // `;` cannot be forgotten by a later edit that adds one more line.
        let mut properties: Vec<String> = Vec::new();
        properties.push(format!(
            "mmc:sessionFile {}",
            literal(&self.session_file.display().to_string())
        ));
        if !self.model.is_empty() {
            properties.push(format!("mmc:sessionModel {}", literal(&self.model)));
        }
        if !self.cwd.as_os_str().is_empty() {
            properties.push(format!(
                "mmc:cwd {}",
                literal(&self.cwd.display().to_string())
            ));
        }
        properties.push(format!(
            "mmc:startedAt {}",
            literal(&self.started_at.to_rfc3339())
        ));
        if let Some(ended) = self.ended_at {
            properties.push(format!("mmc:endedAt {}", literal(&ended.to_rfc3339())));
        }
        properties.push(format!(
            "mmc:eventCount {}",
            integer(i64::from(self.event_count))
        ));
        if let Some(parent) = self.parent_session {
            properties.push(format!(
                "mmc:parentSession <{}>",
                iri::data(&parent).into_string()
            ));
        }
        if let Some(changeset) = self.changeset {
            properties.push(format!(
                "mmc:producedChangeset <{}>",
                iri::data(&changeset).into_string()
            ));
        }
        out.push_str(&format!("<{session}> rdf:type mmc:PiSession ;\n"));
        let last = properties.len().saturating_sub(1);
        for (index, property) in properties.iter().enumerate() {
            let terminator = if index == last { " ." } else { " ;" };
            out.push_str(&format!("    {property}{terminator}\n"));
        }

        for edit in &self.edits {
            let node =
                iri::data(&self.derived_id("edit", edit.seq, &edit.path.display().to_string()))
                    .into_string();
            out.push_str(&format!("\n<{node}> rdf:type mmc:Edit ;\n"));
            out.push_str(&format!(
                "    mmc:path {} ;\n",
                literal(&edit.path.display().to_string())
            ));
            out.push_str(&format!("    mmc:seq {} ;\n", integer(i64::from(edit.seq))));
            out.push_str(&format!("    mmc:editKind {} ;\n", literal(&edit.kind)));
            out.push_str(&format!("    mmc:generatedBy <{session}> .\n"));
        }

        for call in &self.tool_calls {
            let node = iri::data(&self.derived_id("tool_call", call.seq, &call.tool)).into_string();
            out.push_str(&format!("\n<{node}> rdf:type mmc:ToolCall ;\n"));
            out.push_str(&format!("    mmc:toolName {} ;\n", literal(&call.tool)));
            out.push_str(&format!("    mmc:seq {} ;\n", integer(i64::from(call.seq))));
            out.push_str(&format!("    mmc:generatedBy <{session}> .\n"));
        }

        out
    }

    /// Write the session into the `/code` graph, returning how many triples were added.
    ///
    /// The graph is *not* cleared: `/code` is shared with the Phase 2 scan.
    pub async fn mirror(&self, graph: &GraphStore) -> Result<usize> {
        let turtle = self.canonical_turtle();
        graph
            .handle()
            .insert_turtle(CODE_GRAPH, &turtle)
            .await
            .map_err(|e| PiError::Graph(format!("/code mirror: {e}")))
    }

    /// A ULID derived from the session and a record's identity.
    ///
    /// Derived rather than minted, so re-ingesting a file produces the *same* node IRIs
    /// and the graph stays a set rather than growing a copy per ingest.
    fn derived_id(&self, role: &str, seq: u32, detail: &str) -> Ulid {
        let material = format!(
            "{role}\u{1f}{}\u{1f}{seq}\u{1f}{detail}",
            ulid_string(&self.session_id)
        );
        let digest = mm_core::content_hash(material.as_bytes());
        let bytes = digest.as_bytes();
        let mut parts = [0u8; 16];
        for (index, slot) in parts.iter_mut().enumerate() {
            let high = hex_nibble(bytes[index * 2]) << 4;
            let low = hex_nibble(bytes[index * 2 + 1]);
            *slot = high | low;
        }
        // The high 3 bits of a ULID's first byte are the timestamp's top bits; clearing
        // them keeps the value in range rather than aliasing into a larger timestamp.
        parts[0] &= 0b0000_0111;
        Ulid::from_bytes(parts)
    }
}

/// Ingest a session: the rows now, the mirror when the caller asks for it.
pub async fn ingest_session(
    path: &Path,
    store: &SqliteStore,
    logger: &Logger,
    ids: &UlidFactory,
) -> Result<PiSessionGraph> {
    ingest_session_for(path, store, logger, ids, None).await
}

/// Ingest a session and record the change set it belongs to.
pub async fn ingest_session_for(
    path: &Path,
    store: &SqliteStore,
    logger: &Logger,
    ids: &UlidFactory,
    changeset: Option<Ulid>,
) -> Result<PiSessionGraph> {
    let graph = PiSessionGraph::parse_for(path, changeset)?;
    let session_id = ulid_string(&graph.session_id);
    let ingested_at = Timestamp::now().to_rfc3339();

    store
        .execute(
            INSERT_SESSION,
            vec![
                Param::Text(session_id.clone()),
                Param::Text(graph.session_file.display().to_string()),
                Param::opt_text(changeset.map(|id| ulid_string(&id))),
                Param::opt_text(none_if_empty(&graph.model)),
                Param::opt_text(Some(graph.cwd.display().to_string())),
                Param::Text(graph.started_at.to_rfc3339()),
                Param::opt_text(graph.ended_at.map(|at| at.to_rfc3339())),
                Param::Int(i64::from(graph.event_count)),
            ],
        )
        .await
        .map_err(|e| PiError::Store(format!("pi_sessions: {e}")))?;

    for record in &graph.records {
        let raw = graph
            .raw_records
            .get((record.seq.saturating_sub(1)) as usize)
            .cloned()
            .unwrap_or_default();
        store
            .execute(
                INSERT_EVENT,
                vec![
                    Param::Text(ids.next_string()),
                    Param::Text(session_id.clone()),
                    Param::Int(i64::from(record.seq)),
                    Param::Text(record.kind.as_str().to_string()),
                    Param::Text(raw),
                ],
            )
            .await
            .map_err(|e| PiError::Store(format!("pi_events seq {}: {e}", record.seq)))?;
    }

    logger
        .audit(
            Level::Info,
            codes::PI_SESSION_INGEST,
            TARGET,
            Some(graph.session_id),
            json!({
                "session_id": session_id,
                "session_file": graph.session_file.display().to_string(),
                "model": graph.model,
                "event_count": graph.event_count,
                "changeset_ulid": changeset.map(|id| ulid_string(&id)).unwrap_or_else(|| "-".to_string()),
                "edits": graph.edits.len(),
                "tool_calls": graph.tool_calls.len(),
                "inherits": graph.parent_session.map(|id| ulid_string(&id)),
                "ingested_at": ingested_at,
            }),
        )
        .await
        .map_err(|e| PiError::Log(e.to_string()))?;

    for edit in &graph.edits {
        logger
            .audit(
                Level::Info,
                codes::PI_EDIT,
                TARGET,
                Some(graph.session_id),
                json!({
                    "session_id": session_id,
                    "seq": edit.seq,
                    "file": edit.path.display().to_string(),
                    "kind": edit.kind,
                }),
            )
            .await
            .map_err(|e| PiError::Log(e.to_string()))?;
    }

    Ok(graph)
}

/// A Turtle string literal, escaped.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// A typed integer literal. A plain `"3"` would be an `xsd:string` and the shapes ask
/// for `xsd:integer`.
fn integer(value: i64) -> String {
    format!("\"{value}\"^^xsd:integer")
}

/// `None` for an empty string, so an absent model is `NULL` rather than `''`.
fn none_if_empty(text: &str) -> Option<String> {
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// The value of one hex digit, for turning a content hash into a ULID's bytes.
fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_log::{CollectSink, RedactionPolicy};
    use std::sync::Arc;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name)
    }

    fn shapes() -> PathBuf {
        mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("mmc-shapes.ttl")
    }

    async fn context() -> (tempfile::TempDir, SqliteStore, Logger, UlidFactory) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        let logger = Logger::new(
            Level::Info,
            vec![Box::new(CollectSink::new())],
            Some(Arc::new(store.clone())),
            RedactionPolicy::empty(),
        );
        let ids = UlidFactory::open(&dir.path().join("ulid.watermark")).unwrap();
        (dir, store, logger, ids)
    }

    async fn count(store: &SqliteStore, table: &str) -> i64 {
        let rows = store
            .query_json(&format!("SELECT count(*) AS n FROM {table}"), Vec::new())
            .await
            .unwrap();
        rows[0]["n"].as_i64().unwrap_or(-1)
    }

    #[test]
    fn the_recorded_session_parses_into_the_module_it_scaffolded() {
        let graph =
            PiSessionGraph::parse(&fixture("recorded_session_module_scaffold_01.jsonl")).unwrap();
        assert_eq!(graph.event_count, 8);
        assert_eq!(graph.model, "pi-test");
        assert!(graph.ended_at.is_some());
        assert_eq!(graph.edits.len(), 3, "three files were edited");
        assert!(
            graph.tool_calls.len() >= 2,
            "the session read the skill and wrote files"
        );
        assert_eq!(
            graph.edited_paths(),
            vec![
                PathBuf::from("modules/cognition/calibration/plugin.toml"),
                PathBuf::from("modules/cognition/calibration/src/lib.rs"),
                PathBuf::from("modules/cognition/calibration/manual/module.md"),
            ]
        );
        assert_eq!(graph.records.len(), graph.raw_records.len());
    }

    #[test]
    fn the_multibyte_fixture_parses_and_keeps_its_separators() {
        // The file encodes the separators as `\u2028`/`\u2029` escapes, so what this
        // asserts is the decoding half of the property: they come back as content. The
        // record count pins the other half — a reader that split on a Unicode line break
        // would produce a different number of records, because the following fragment is
        // not a record. The raw-byte case is `framing`'s own unit test, where the
        // separator is written into the chunk as bytes rather than as an escape.
        let graph = PiSessionGraph::parse(&fixture("recorded_session_multibyte.jsonl")).unwrap();
        // The file is five lines: a header and four records. `event_count` counts
        // records, not lines — the header describes the session, it is not part of it — and
        // the count is what pins the split behaviour: a reader that treated a Unicode line
        // break as a separator would not come back with these four.
        assert_eq!(graph.event_count, 4, "one record per line, no splits");
        let text = graph
            .records
            .iter()
            .map(|record| record.text.clone())
            .collect::<Vec<String>>()
            .join("\n");
        assert!(text.contains('\u{2028}'), "{text:?}");
        assert!(text.contains('\u{2029}'), "{text:?}");
        assert_eq!(graph.edits.len(), 1);
        assert_eq!(graph.tool_calls.len(), 1);
    }

    #[test]
    fn the_truncated_fixture_is_a_hard_error() {
        let error =
            PiSessionGraph::parse(&fixture("recorded_session_truncated.jsonl")).unwrap_err();
        assert_eq!(error.code(), "truncated", "{error}");
        assert!(error.to_string().contains("truncated"));
    }

    #[test]
    fn the_turtle_names_every_predicate_the_shapes_require() {
        let graph =
            PiSessionGraph::parse(&fixture("recorded_session_module_scaffold_01.jsonl")).unwrap();
        let turtle = graph.canonical_turtle();
        for predicate in [
            "mmc:PiSession",
            "mmc:sessionFile",
            "mmc:startedAt",
            "mmc:eventCount",
            "mmc:Edit",
            "mmc:path",
            "mmc:seq",
            "mmc:generatedBy",
            "mmc:ToolCall",
            "mmc:toolName",
        ] {
            assert!(turtle.contains(predicate), "missing {predicate}:\n{turtle}");
        }
        assert!(turtle.contains("^^xsd:integer"), "integers are typed");
        assert_eq!(
            turtle,
            graph.canonical_turtle(),
            "rendering is deterministic"
        );
        // Derived node IRIs are stable across parses, so the graph stays a set.
        let again =
            PiSessionGraph::parse(&fixture("recorded_session_module_scaffold_01.jsonl")).unwrap();
        assert_eq!(turtle, again.canonical_turtle());
    }

    #[tokio::test]
    async fn the_mirror_satisfies_the_code_shapes_and_is_idempotent() {
        let graph =
            PiSessionGraph::parse(&fixture("recorded_session_module_scaffold_01.jsonl")).unwrap();
        let store = GraphStore::in_memory(&shapes()).await.unwrap();
        let inserted = graph.mirror(&store).await.unwrap();
        assert!(
            inserted > 10,
            "the session, its edits and its calls: {inserted}"
        );
        let after_first = store.canonical_ntriples(CODE_GRAPH).await.unwrap();

        let report = store.validate_with(CODE_GRAPH, &shapes()).await.unwrap();
        assert!(
            report.conforms,
            "the mirror must satisfy the /code shapes: {:?}",
            report.violations
        );

        // A second ingest of the same file adds nothing: the derived ids are stable, so
        // the graph is the same set of quads rather than a second copy of them.
        graph.mirror(&store).await.unwrap();
        assert_eq!(
            store.canonical_ntriples(CODE_GRAPH).await.unwrap(),
            after_first,
            "re-mirroring the same session is idempotent"
        );
    }

    #[tokio::test]
    async fn ingesting_writes_gapless_events_and_survives_a_second_run() {
        let (_dir, store, logger, ids) = context().await;
        let path = fixture("recorded_session_module_scaffold_01.jsonl");
        let graph = ingest_session(&path, &store, &logger, &ids).await.unwrap();
        assert_eq!(count(&store, "pi_sessions").await, 1);
        assert_eq!(
            count(&store, "pi_events").await,
            i64::from(graph.event_count),
            "one event per record"
        );

        let rows = store
            .query_json(
                "SELECT seq FROM pi_events WHERE session_ulid = ? ORDER BY seq",
                vec![Param::Text(ulid_string(&graph.session_id))],
            )
            .await
            .unwrap();
        let seqs: Vec<i64> = rows
            .iter()
            .map(|row| row["seq"].as_i64().unwrap())
            .collect();
        assert_eq!(
            seqs,
            (1..=i64::from(graph.event_count)).collect::<Vec<_>>(),
            "seq is 1..=n with no holes"
        );

        // Re-ingesting the same file is idempotent, and the raw line is archived.
        ingest_session(&path, &store, &logger, &ids).await.unwrap();
        assert_eq!(count(&store, "pi_sessions").await, 1);
        assert_eq!(
            count(&store, "pi_events").await,
            i64::from(graph.event_count)
        );
        let payloads = store
            .query_json(
                "SELECT payload_json FROM pi_events ORDER BY seq LIMIT 1",
                Vec::new(),
            )
            .await
            .unwrap();
        let raw = payloads[0]["payload_json"].as_str().unwrap();
        assert!(raw.contains("\"parentId\""), "the bytes are kept: {raw}");
    }

    #[tokio::test]
    async fn the_truncated_fixture_never_reaches_the_store() {
        let (_dir, store, logger, ids) = context().await;
        let error = ingest_session(
            &fixture("recorded_session_truncated.jsonl"),
            &store,
            &logger,
            &ids,
        )
        .await
        .unwrap_err();
        assert!(error.is_recording_fault(), "{error}");
        assert_eq!(count(&store, "pi_sessions").await, 0);
        assert_eq!(count(&store, "pi_events").await, 0);
    }
}
