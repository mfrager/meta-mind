//! Focus-document catalog for the left pane (plan `05`).
//!
//! Seeded documents are fixtures the pane can open before chat exists. Their
//! status and `reason` are the words the UI must show verbatim. Drafts and
//! history live in this process, beside the data directory the kernel owns.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::dto::focus::{DocCapability, DocKind, DocSource, DocSourceKind, DocStatus, FocusDoc};

pub const PROSE_ID: &str = "01jdocprose000000000000000";
pub const CHART_ID: &str = "01jdocchart000000000000000";
pub const LOG_ID: &str = "01jdoclog00000000000000000";
pub const CODE_ID: &str = "01jdoccode0000000000000000";
pub const JSON_ID: &str = "01jdocjson0000000000000000";
pub const FORM_ID: &str = "01jdocform0000000000000000";
pub const DIFF_ID: &str = "01jdocdiff0000000000000000";
pub const PROOF_ID: &str = "01jdocproof000000000000000";
pub const TABLE_ID: &str = "01jdoctable000000000000000";
pub const TIMELINE_ID: &str = "01jdoctime0000000000000000";
pub const RUN_ID: &str = "01jdocrun00000000000000000";
pub const TREE_ID: &str = "01jdoctree0000000000000000";
pub const MAP_ID: &str = "01jdocmap00000000000000000";

const TREE_REASON: &str =
    "This tree is a fixture. Live ontology records are not on this route yet.";
const MAP_REASON: &str = "This map is a fixture. Live being records are not on this route yet.";

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub version: u64,
    pub saved_at: String,
    pub actor: String,
    pub status: DocStatus,
}

struct Entry {
    doc: FocusDoc,
    history: Vec<HistoryEntry>,
}

/// Process-local focus documents. One catalog per booted server.
pub struct FocusCatalog {
    inner: Mutex<BTreeMap<String, Entry>>,
}

impl FocusCatalog {
    pub fn seed() -> Self {
        let mut map = BTreeMap::new();
        for doc in seed_docs() {
            let id = doc.id.clone();
            map.insert(
                id,
                Entry {
                    doc,
                    history: Vec::new(),
                },
            );
        }
        FocusCatalog {
            inner: Mutex::new(map),
        }
    }

    pub fn get(&self, id: &str) -> Option<FocusDoc> {
        self.inner
            .lock()
            .ok()?
            .get(id)
            .map(|entry| entry.doc.clone())
    }

    pub fn list(&self) -> Vec<FocusDoc> {
        self.inner
            .lock()
            .map(|map| map.values().map(|entry| entry.doc.clone()).collect())
            .unwrap_or_default()
    }

    pub fn live_since(&self, version: u64) -> Vec<FocusDoc> {
        self.list()
            .into_iter()
            .filter(|doc| doc.capabilities.contains(&DocCapability::Live) && doc.version > version)
            .collect()
    }

    pub fn history(&self, id: &str) -> Option<Vec<HistoryEntry>> {
        self.inner
            .lock()
            .ok()?
            .get(id)
            .map(|entry| entry.history.clone())
    }

    /// Autosave a draft body. Returns the updated document.
    pub fn save_draft(&self, id: &str, body: &str) -> Result<FocusDoc, String> {
        let mut map = self
            .inner
            .lock()
            .map_err(|_| "focus catalog lock".to_string())?;
        let entry = map.get_mut(id).ok_or_else(|| format!("no document {id}"))?;
        if !entry.doc.capabilities.contains(&DocCapability::Edit) {
            return Err(format!("{id} cannot be edited"));
        }
        let mut snapshot = entry.doc.snapshot.clone();
        match entry.doc.kind {
            DocKind::Prose => {
                snapshot["markdown"] = json!(body);
            }
            DocKind::Code => {
                snapshot["text"] = json!(body);
            }
            DocKind::Form => {
                snapshot["title"] = json!(body);
            }
            _ => {
                snapshot["draft"] = json!(body);
            }
        }
        entry.doc.version += 1;
        entry.doc.status = DocStatus::Draft;
        entry.doc.snapshot = snapshot;
        let saved_at = epoch_seconds();
        entry.history.push(HistoryEntry {
            version: entry.doc.version,
            saved_at: saved_at.clone(),
            actor: "console".into(),
            status: DocStatus::Draft,
        });
        entry.doc.subtitle = Some(format!("draft v{}", entry.doc.version));
        Ok(entry.doc.clone())
    }

    /// Note a change set id on the document after the kernel records it.
    pub fn attach_changeset(&self, id: &str, changeset_id: &str) -> Option<FocusDoc> {
        let mut map = self.inner.lock().ok()?;
        let entry = map.get_mut(id)?;
        entry.doc.version += 1;
        entry.doc.status = DocStatus::Draft;
        entry.doc.subtitle = Some(format!("draft ({changeset_id})"));
        entry.history.push(HistoryEntry {
            version: entry.doc.version,
            saved_at: epoch_seconds(),
            actor: "changeset".into(),
            status: DocStatus::Draft,
        });
        Some(entry.doc.clone())
    }
}

fn epoch_seconds() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}

fn seed_docs() -> Vec<FocusDoc> {
    vec![
        make(
            PROSE_ID,
            DocKind::Prose,
            "Console note",
            DocStatus::Saved,
            None,
            1,
            vec![
                DocCapability::Edit,
                DocCapability::Export,
                DocCapability::History,
                DocCapability::Provenance,
                DocCapability::FocusChat,
            ],
            json!({
                "markdown": "A note kept in the document pane.\n\nEdits that touch production go out as a change set.",
                "production": true,
                "path": "docs/console-note.md"
            }),
        ),
        make(
            CHART_ID,
            DocKind::Chart,
            "Self-model divergence",
            DocStatus::Degraded,
            Some("These series are a fixture. Live chart streams are not on the wire yet.".into()),
            1,
            vec![DocCapability::Export, DocCapability::Refresh],
            json!({
                "recipe": "self_model_divergence",
                "note": "Fixture series. Not a live self-model report.",
                "series": [
                    { "name": "Actual", "points": [{"x": "1", "y": 1}, {"x": "2", "y": 2}, {"x": "3", "y": 2}, {"x": "4", "y": 3}] },
                    { "name": "Model", "points": [{"x": "1", "y": 1}, {"x": "2", "y": 1}, {"x": "3", "y": 2}, {"x": "4", "y": 2}] }
                ]
            }),
        ),
        make(
            LOG_ID,
            DocKind::Log,
            "System log",
            DocStatus::Live,
            None,
            1,
            vec![DocCapability::Live, DocCapability::Export],
            json!({ "stream": "system", "count": 1_000_000 }),
        ),
        make(
            CODE_ID,
            DocKind::Code,
            "Being outline",
            DocStatus::Saved,
            None,
            1,
            vec![DocCapability::Edit, DocCapability::Export],
            json!({ "language": "sem", "text": "being\n  identity\n" }),
        ),
        make(
            JSON_ID,
            DocKind::Code,
            "sample.json",
            DocStatus::Saved,
            None,
            1,
            vec![DocCapability::Export],
            json!({ "language": "json", "text": "{\n  \"ok\": true\n}\n" }),
        ),
        make(
            FORM_ID,
            DocKind::Form,
            "Record",
            DocStatus::Saved,
            None,
            1,
            vec![DocCapability::Edit],
            json!({ "title": "Doctor", "owner": "local" }),
        ),
        make(
            DIFF_ID,
            DocKind::Diff,
            "console-note.md",
            DocStatus::Saved,
            None,
            1,
            vec![DocCapability::Export],
            json!({ "before": "A note.\n", "after": "A note kept in the document pane.\n" }),
        ),
        make(
            PROOF_ID,
            DocKind::Proof,
            "Obligation",
            DocStatus::Verified,
            None,
            1,
            vec![DocCapability::Export],
            json!({ "goal": "The note stays a draft until a change set exists.", "steps": ["Read the draft.", "Require a reason.", "Record the change set."] }),
        ),
        make(
            TABLE_ID,
            DocKind::Table,
            "Services",
            DocStatus::Saved,
            None,
            1,
            vec![DocCapability::Export],
            json!({ "rows": [
                { "id": "1", "name": "sqlite", "state": "ok" },
                { "id": "2", "name": "graph", "state": "unavailable" }
            ]}),
        ),
        make(
            TIMELINE_ID,
            DocKind::Timeline,
            "Boot",
            DocStatus::Degraded,
            Some(
                "These events are the boot fixture. A loop's stage timings are not recorded."
                    .into(),
            ),
            1,
            vec![DocCapability::Provenance],
            json!({ "events": [
                { "at": "t0", "label": "Server booted", "kind": "run" },
                { "at": "t1", "label": "Health answered", "kind": "log" }
            ]}),
        ),
        make(
            RUN_ID,
            DocKind::Run,
            "Sample run",
            DocStatus::Saved,
            Some("No stage timings are recorded for this run.".into()),
            1,
            vec![DocCapability::Refresh, DocCapability::Live],
            json!({ "stages": [
                { "name": "plan", "state": "pending" },
                { "name": "act", "state": "pending" },
                { "name": "check", "state": "pending" }
            ]}),
        ),
        make(
            TREE_ID,
            DocKind::Tree,
            "Ontology",
            DocStatus::Degraded,
            Some(TREE_REASON.into()),
            1,
            vec![
                DocCapability::Refresh,
                DocCapability::Export,
                DocCapability::FocusChat,
            ],
            json!({ "tree": "ontology" }),
        ),
        make(
            MAP_ID,
            DocKind::Mindmap,
            "Being map",
            DocStatus::Degraded,
            Some(MAP_REASON.into()),
            1,
            vec![
                DocCapability::Refresh,
                DocCapability::Export,
                DocCapability::FocusChat,
            ],
            json!({ "map": "being" }),
        ),
    ]
}

fn make(
    id: &str,
    kind: DocKind,
    title: &str,
    status: DocStatus,
    reason: Option<String>,
    version: u64,
    capabilities: Vec<DocCapability>,
    snapshot: Value,
) -> FocusDoc {
    FocusDoc {
        v: 1,
        id: id.into(),
        kind,
        title: title.into(),
        subtitle: None,
        status,
        reason,
        epistemic: None,
        version,
        source: DocSource {
            kind: DocSourceKind::Nav,
            ref_id: None,
        },
        capabilities,
        snapshot,
    }
}
