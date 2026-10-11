//! Focus-document wire types (plan `05` §2, `09` §2.3).
//!
//! Every artifact the left pane can show — document, chart, log, tree, mind
//! map, diff, proof, … — is a `FocusDoc` in viewer or editor mode. M0 ships the
//! identity/status vocabulary; the per-kind snapshots and the versioned focus
//! channel arrive with the document pane (M3), additive within `wireVersion: 1`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// What kind of artifact a focus document is (plan `05` §5). Drives the
/// renderer registry, the pane tab icon chip, and the kind color on the pane
/// header's left edge (plan `02` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum DocKind {
    /// Markdown document or note.
    Prose,
    /// Chart or dashboard.
    Chart,
    /// Log stream.
    Log,
    /// Tabular records.
    Table,
    /// Tree viewer/editor (plan `06`).
    Tree,
    /// Mind map (plan `07`).
    Mindmap,
    /// Change set / version diff.
    Diff,
    /// Verification proof (plan `08` §4.2).
    Proof,
    /// Code / SEM / Turtle / JSON source.
    Code,
    /// Structured record editor.
    Form,
    /// Event timeline / trace / replay.
    Timeline,
    /// A run (loop / episode / sandbox) composition (plan `05` §5.6).
    Run,
    /// A bounded decision record.
    Decision,
    /// Budget instruments (plan `05` §5.9).
    Budget,
}

/// The "current document status" chip language (plan `05` §4, `02` §3.2).
/// The UI renders these as chips; several require a `reason` (see `FocusDoc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum DocStatus {
    /// Edited but not saved.
    Draft,
    /// Saved.
    Saved,
    /// Streaming from a live source.
    Live,
    /// The source changed since this snapshot.
    Stale,
    /// A run is executing behind this document.
    Running,
    /// Verified by a gate/proof.
    Verified,
    /// Failed; `reason` explains.
    Failed,
    /// Blocked (e.g. firewall); `reason` explains.
    Blocked,
    /// No runtime/provider here; `reason` is mandatory.
    Unavailable,
    /// Partially working; `reason` says what still works.
    Degraded,
    /// Waiting at the promotion gate.
    PromotionPending,
    /// Written to production through a promotion.
    Promoted,
    /// Rolled back.
    RolledBack,
}

/// What the document supports (plan `05` §2); the pane hides actions that are
/// not in the list instead of failing when they are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum DocCapability {
    /// Editor mode is available.
    Edit,
    /// Export menu is available.
    Export,
    /// Version history is available.
    History,
    /// A provenance strip/lineage is available.
    Provenance,
    /// "Focus in chat" / ask-about-selection is available.
    FocusChat,
    /// Manual refresh is meaningful (non-live snapshots).
    Refresh,
    /// The document streams from a live source.
    Live,
}

/// Epistemic status stamps (plan `02` §3.3) — shown as badges on claims,
/// values, and document headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum EpistemicStatus {
    /// Recorded observation.
    Observation,
    /// Assumption on the ledger.
    Assumption,
    /// Derived by inference.
    Inference,
    /// No recorded provenance; the UI never invents a badge.
    Unverified,
    /// Contradicted by other ledger entries.
    Contradicted,
}

/// Where a document came from (plan `05` §2) — shown as the source chip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DocSource {
    pub kind: DocSourceKind,
    /// The chat message / run / link that opened it, when there is one.
    #[serde(rename = "refId", default, skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
}

/// The opener of a focus document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum DocSourceKind {
    /// Opened from a chat run or message.
    Chat,
    /// Opened from navigation (sidebar, palette, tree).
    Nav,
    /// Opened from the command palette.
    Palette,
    /// Opened from a deep link.
    Link,
    /// Pushed by a live stream.
    Stream,
}

/// One focus document (plan `05` §2).
///
/// `snapshot` is the kind-specific payload; typed per-kind snapshot structs
/// land with the document pane (M3) and will narrow this field additively.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct FocusDoc {
    /// Envelope version, always 1 within `wireVersion: 1`.
    pub v: u32,
    /// ULID, lowercase.
    pub id: String,
    pub kind: DocKind,
    pub title: String,
    /// Mono value line: ids, paths, counts (plan `05` §1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: DocStatus,
    /// Mandatory for `unavailable`, `degraded`, `blocked`, and `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epistemic: Option<EpistemicStatus>,
    /// Monotonic snapshot version (the focus channel's `?from=` cursor).
    pub version: u64,
    pub source: DocSource,
    pub capabilities: Vec<DocCapability>,
    /// Kind-specific payload. Typed per-kind snapshots replace this in M3.
    #[ts(type = "unknown")]
    pub snapshot: Value,
}
