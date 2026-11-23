//! Session lifecycle, presence, work catalog, frame stack, and trace manifests.
//!
//! Phase 0 owns the data contracts. RDF persistence and the live registry are
//! later phases; this module deliberately has no store dependency so the
//! session model can be tested without opening the graph.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::agentstream::AgentstreamStore;
use crate::conversation::{
    ConversationEvent, ConversationEventKind, ConversationState, ConversationStore, StateOrigin,
};
use crate::engine::RdfEngine;
use crate::rdf::{self, session_iri};

/// Maximum live frames, including the root.
pub const MAX_FRAME_DEPTH: usize = 8;
/// Maximum parallel work items; `main` is not included.
pub const MAX_PARALLEL_WORK: usize = 7;

/// Session lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycle {
    /// Accepting turns and background work.
    Active,
    /// Finalization has begun.
    Ending,
    /// Closed; retained for resume/audit.
    Ended,
}

impl SessionLifecycle {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Ending => "ending",
            Self::Ended => "ended",
        }
    }

    /// Whether new turns and mutations are accepted.
    pub fn is_open(self) -> bool {
        self == Self::Active
    }
}

/// Console presence, independent of activity/expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    /// Recent heartbeat.
    Present,
    /// Heartbeat older than the present window.
    Away,
    /// No recent heartbeat.
    Absent,
}

/// What a work item represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    /// Foreground conversation.
    Conversation,
    /// Plan.
    Plan,
    /// Delegated agent run.
    Agent,
    /// Scheduled/internal action.
    Action,
    /// Frame utterance.
    Frame,
    /// Maintenance task.
    Maintenance,
}

/// Work status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    /// Accepted but not started.
    Queued,
    /// Actively running.
    Working,
    /// Waiting for a user or another result.
    Waiting,
    /// Blocked by an external dependency.
    Blocked,
    /// Completed successfully.
    Done,
    /// Failed.
    Failed,
    /// Cancelled.
    Cancelled,
}

impl WorkStatus {
    /// Whether this status is actively working.
    pub fn is_working(self) -> bool {
        self == Self::Working
    }

    /// Whether this status is terminal.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    /// Whether this status belongs to an active catalog.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Working | Self::Waiting | Self::Blocked
        )
    }
}

/// One item in the bounded work catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    /// Stable id; `main` for the foreground conversation.
    pub id: String,
    /// Short operator-facing title.
    pub title: String,
    /// Work kind.
    pub kind: WorkKind,
    /// Current status.
    pub status: WorkStatus,
    /// Source reference (plan step, run, action, frame).
    pub source: Option<crate::conversation::StatusRef>,
    /// Start instant, Unix ms.
    pub started_at: i64,
    /// Last activity instant, Unix ms.
    pub last_activity_at: i64,
    /// Optional interjection policy override.
    pub interjection_policy: Option<String>,
}

impl WorkItem {
    /// A foreground main work item.
    pub fn main(title: impl Into<String>, now_ms: i64) -> Self {
        Self {
            id: "main".into(),
            title: title.into(),
            kind: WorkKind::Conversation,
            status: WorkStatus::Waiting,
            source: None,
            started_at: now_ms,
            last_activity_at: now_ms,
            interjection_policy: None,
        }
    }

    /// Create a parallel work item.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        kind: WorkKind,
        now_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind,
            status: WorkStatus::Queued,
            source: None,
            started_at: now_ms,
            last_activity_at: now_ms,
            interjection_policy: None,
        }
    }

    /// Validate title/id bounds.
    pub fn validate(&self) -> Result<(), SessionError> {
        if self.id.trim().is_empty() {
            return Err(SessionError::Invalid("work id must not be empty".into()));
        }
        if self.title.trim().is_empty() {
            return Err(SessionError::Invalid("work title must not be empty".into()));
        }
        if self.title.chars().count() > 120 {
            return Err(SessionError::Invalid(
                "work title exceeds 120 characters".into(),
            ));
        }
        Ok(())
    }
}

/// The foreground item plus at most seven parallel items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkCatalog {
    /// Foreground conversation; always present.
    pub main: WorkItem,
    /// Parallel work, sorted by id.
    pub parallel: BTreeMap<String, WorkItem>,
}

impl Default for WorkCatalog {
    fn default() -> Self {
        Self::new(0)
    }
}

impl WorkCatalog {
    /// A catalog with only the main item.
    pub fn new(now_ms: i64) -> Self {
        Self {
            main: WorkItem::main("conversation", now_ms),
            parallel: BTreeMap::new(),
        }
    }

    /// Whether any item is actively working.
    pub fn working(&self) -> bool {
        self.main.status.is_working() || self.parallel.values().any(|item| item.status.is_working())
    }

    /// Whether any item is queued, working, waiting, or blocked.
    pub fn active(&self) -> bool {
        self.main.status.is_active() || self.parallel.values().any(|item| item.status.is_active())
    }

    /// All items, main first, then parallel in id order.
    pub fn items(&self) -> Vec<&WorkItem> {
        std::iter::once(&self.main)
            .chain(self.parallel.values())
            .collect()
    }

    /// Add or replace a parallel item.
    pub fn upsert(&mut self, item: WorkItem) -> Result<(), SessionError> {
        if item.id == "main" {
            if item.kind != WorkKind::Conversation {
                return Err(SessionError::Invalid(
                    "main work must remain the conversation".into(),
                ));
            }
            item.validate()?;
            self.main = item;
            return Ok(());
        }
        item.validate()?;
        if !self.parallel.contains_key(&item.id) && self.parallel.len() >= MAX_PARALLEL_WORK {
            return Err(SessionError::WorkLimit);
        }
        self.parallel.insert(item.id.clone(), item);
        Ok(())
    }

    /// Remove a terminal parallel item. `main` is never removable.
    pub fn remove_terminal(&mut self, id: &str) -> Result<bool, SessionError> {
        if id == "main" {
            return Err(SessionError::Invalid("main work cannot be removed".into()));
        }
        let terminal = self
            .parallel
            .get(id)
            .is_some_and(|item| item.status.is_terminal());
        if terminal {
            self.parallel.remove(id);
        }
        Ok(terminal)
    }

    /// Validate catalog bounds.
    pub fn validate(&self) -> Result<(), SessionError> {
        if self.main.id != "main" {
            return Err(SessionError::Invalid("main work id must be main".into()));
        }
        self.main.validate()?;
        if self.parallel.len() > MAX_PARALLEL_WORK {
            return Err(SessionError::WorkLimit);
        }
        for item in self.parallel.values() {
            item.validate()?;
        }
        Ok(())
    }
}

/// Frame kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameKind {
    /// Fresh-session intro.
    Intro,
    /// Welcome-back frame.
    Resume,
    /// Task/work frame.
    Work,
    /// Tutorial offer.
    TutorialOffer,
    /// Tutorial walkthrough.
    Tutorial,
    /// Goodbye.
    Goodbye,
    /// Operator-defined frame.
    Custom,
}

/// Frame status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameStatus {
    /// Currently executing.
    Active,
    /// Lower in the stack, temporarily inactive.
    Suspended,
    /// Finished successfully.
    Complete,
    /// Finished without completion.
    Abandoned,
}

/// One frame in the stack. Dynamic step/attempt values live in conversation
/// status, not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationFrame {
    /// Frame id.
    pub id: String,
    /// Template id.
    pub template_id: String,
    /// Pinned template version.
    pub template_version: u32,
    /// Frame kind.
    pub kind: FrameKind,
    /// Frame status.
    pub status: FrameStatus,
    /// Enter instant, Unix ms.
    pub entered_at: i64,
    /// Last update instant, Unix ms.
    pub updated_at: i64,
    /// Optional frame expiry.
    pub expires_at: Option<i64>,
}

/// A bounded frame stack: root first, top last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameStack {
    /// Shared conversation-state revision.
    pub revision: u64,
    /// Root first, top last.
    pub frames: Vec<ConversationFrame>,
}

impl FrameStack {
    /// A stack with one root frame.
    pub fn new(root: ConversationFrame) -> Self {
        Self {
            revision: 1,
            frames: vec![root],
        }
    }

    /// The current frame.
    pub fn top(&self) -> Option<&ConversationFrame> {
        self.frames.last()
    }

    /// The current frame, mutably.
    pub fn top_mut(&mut self) -> Option<&mut ConversationFrame> {
        self.frames.last_mut()
    }

    /// Push a frame, suspending the previous top.
    pub fn push(&mut self, frame: ConversationFrame) -> Result<(), SessionError> {
        if self.frames.len() >= MAX_FRAME_DEPTH {
            return Err(SessionError::FrameLimit);
        }
        if let Some(top) = self.frames.last_mut() {
            top.status = FrameStatus::Suspended;
            top.updated_at = frame.entered_at;
        }
        self.frames.push(frame);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    /// Replace the top frame without changing stack depth.
    pub fn replace_top(&mut self, frame: ConversationFrame) -> Result<(), SessionError> {
        if self.frames.is_empty() {
            return Err(SessionError::FrameInvalid(
                "cannot replace a missing top frame".into(),
            ));
        }
        *self.frames.last_mut().expect("checked non-empty") = frame;
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    /// Pop the top frame. The root is protected.
    pub fn pop(&mut self, now_ms: i64) -> Result<ConversationFrame, SessionError> {
        if self.frames.len() <= 1 {
            return Err(SessionError::FrameLimit);
        }
        let mut removed = self.frames.pop().expect("checked non-empty");
        removed.status = FrameStatus::Complete;
        removed.updated_at = now_ms;
        if let Some(top) = self.frames.last_mut() {
            top.status = FrameStatus::Active;
            top.updated_at = now_ms;
        }
        self.revision = self.revision.saturating_add(1);
        Ok(removed)
    }

    /// Validate uniqueness and bounds.
    pub fn validate(&self) -> Result<(), SessionError> {
        if self.frames.is_empty() {
            return Err(SessionError::FrameInvalid("frame stack is empty".into()));
        }
        if self.frames.len() > MAX_FRAME_DEPTH {
            return Err(SessionError::FrameLimit);
        }
        let mut seen = std::collections::BTreeSet::new();
        for frame in &self.frames {
            if !seen.insert(frame.id.as_str()) {
                return Err(SessionError::FrameInvalid("duplicate frame id".into()));
            }
        }
        Ok(())
    }
}

/// A pending frame utterance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameUtterance {
    /// Utterance id.
    pub id: String,
    /// Owning frame.
    pub frame_id: String,
    /// Due instant, Unix ms.
    pub due_at: i64,
    /// Why it was scheduled.
    pub reason: String,
    /// Attempt number.
    pub attempt: u8,
}

/// Resume/supersession metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResumeMetadata {
    /// Resume epoch.
    pub epoch: u32,
    /// Session this one resumes.
    pub resume_of: Option<String>,
    /// Number of resumes.
    pub resume_count: u32,
    /// Last instant at which the session may be resumed.
    pub resumable_until: Option<i64>,
}

/// A trace kind in a session manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceKind {
    /// User-facing CM conversation.
    Main,
    /// Delegated agent run.
    Agent,
    /// Planning sub-session.
    Plan,
    /// Inner thought.
    Think,
    /// Memory operation.
    Memory,
    /// Internal/scheduled action.
    Action,
    /// Automation supervisor.
    Automation,
    /// Introspection.
    Introspection,
    /// RDF log view.
    Log,
}

/// One agentstream trace manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTrace {
    /// Trace id.
    pub trace_id: String,
    /// Owning session.
    pub session_id: String,
    /// Trace kind.
    pub kind: TraceKind,
    /// Agentstream session id.
    pub agentstream_id: String,
    /// First sequence.
    pub first_seq: u64,
    /// Last sequence.
    pub last_seq: u64,
    /// Event count.
    pub event_count: u64,
    /// Start instant, Unix ms.
    pub started_at: i64,
    /// End instant, Unix ms.
    pub ended_at: Option<i64>,
    /// Approximate stored bytes.
    pub size_bytes: u64,
}

/// Session filter for the paginated RDF list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFilter {
    /// Optional lifecycle.
    pub status: Option<SessionLifecycle>,
    /// Optional text query.
    pub query: Option<String>,
    /// Include soft-deleted sessions.
    pub include_deleted: bool,
}

/// A session row for the Sessions tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Session id.
    pub session_id: String,
    /// Owning user id.
    #[serde(default)]
    pub user_id: String,
    /// Topic/label.
    pub topic: Option<String>,
    /// Lifecycle.
    pub lifecycle: SessionLifecycle,
    /// Presence.
    pub presence: Presence,
    /// Derived working.
    pub working: bool,
    /// Trace count.
    pub trace_count: u32,
    /// Trace kinds present.
    pub trace_kinds: Vec<TraceKind>,
    /// Last activity.
    pub last_activity_at: i64,
    /// On-demand directory, when created.
    pub session_dir: Option<String>,
    /// Whether an ended session is still resumable.
    pub resumable: bool,
    /// Whether the session is soft-deleted.
    #[serde(default)]
    pub deleted: bool,
}

/// One page of sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPage {
    /// Rows.
    pub items: Vec<SessionSummary>,
    /// Opaque next cursor.
    pub next_cursor: Option<String>,
    /// Whether another page exists.
    pub has_more: bool,
}

/// One page of trace manifests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracePage {
    /// Rows.
    pub items: Vec<SessionTrace>,
    /// Opaque next cursor.
    pub next_cursor: Option<String>,
    /// Whether another page exists.
    pub has_more: bool,
}

/// Delete mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteMode {
    /// Hide while retaining history.
    Soft,
    /// Remove RDF/log/directory after confirmation.
    Purge,
}

/// The durable session state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    /// Session id.
    pub session_id: String,
    /// Owning user.
    pub user_id: String,
    /// Lifecycle.
    pub lifecycle: SessionLifecycle,
    /// Presence.
    pub presence: Presence,
    /// Conversation revision this session's state is pinned to.
    pub revision: u64,
    /// Creation instant.
    pub opened_at: i64,
    /// Last activity instant.
    pub last_activity_at: i64,
    /// Last user message instant.
    pub last_user_input_at: Option<i64>,
    /// Last work event instant.
    pub last_work_activity_at: Option<i64>,
    /// Last heartbeat instant.
    pub last_client_seen_at: Option<i64>,
    /// Explicit expiry, when a goodbye set one.
    pub expires_at: Option<i64>,
    /// End instant.
    pub ended_at: Option<i64>,
    /// End reason.
    pub end_reason: Option<String>,
    /// Work catalog.
    pub work: WorkCatalog,
    /// Trace manifests.
    pub traces: Vec<SessionTrace>,
    /// Mandatory main trace id.
    pub main_trace: Option<String>,
    /// Pending frame utterance, when one is scheduled.
    #[serde(default)]
    pub pending_utterance: Option<FrameUtterance>,
    /// On-demand session directory.
    pub session_dir: Option<String>,
    /// Resume metadata.
    pub resume: ResumeMetadata,
    /// Superseding session.
    pub superseded_by: Option<String>,
    /// Soft-delete instant.
    pub deleted_at: Option<i64>,
    /// Soft-delete actor.
    pub deleted_by: Option<String>,
}

impl SessionState {
    /// A new active session with a main work item.
    pub fn new(session_id: impl Into<String>, user_id: impl Into<String>, now_ms: i64) -> Self {
        Self {
            session_id: session_id.into(),
            user_id: user_id.into(),
            lifecycle: SessionLifecycle::Active,
            presence: Presence::Present,
            revision: 1,
            opened_at: now_ms,
            last_activity_at: now_ms,
            last_user_input_at: None,
            last_work_activity_at: None,
            last_client_seen_at: Some(now_ms),
            expires_at: None,
            ended_at: None,
            end_reason: None,
            work: WorkCatalog::new(now_ms),
            traces: Vec::new(),
            main_trace: None,
            pending_utterance: None,
            session_dir: None,
            resume: ResumeMetadata::default(),
            superseded_by: None,
            deleted_at: None,
            deleted_by: None,
        }
    }

    /// Derived working state.
    pub fn working(&self) -> bool {
        self.work.working()
    }

    /// End the session, retaining it for resume/audit.
    pub fn end(&mut self, now_ms: i64, reason: impl Into<String>) {
        self.lifecycle = SessionLifecycle::Ended;
        self.presence = Presence::Absent;
        self.ended_at = Some(now_ms);
        self.end_reason = Some(reason.into());
        self.expires_at = None;
        self.last_activity_at = now_ms;
    }

    /// The activity deadline, preferring an explicit expiry.
    pub fn activity_deadline(&self) -> i64 {
        self.expires_at.unwrap_or_else(|| {
            self.last_user_input_at
                .unwrap_or(self.opened_at)
                .max(self.last_work_activity_at.unwrap_or(self.opened_at))
        })
    }

    /// Register a trace and enforce one main trace.
    pub fn register_trace(&mut self, trace: SessionTrace) -> Result<(), SessionError> {
        if trace.session_id != self.session_id {
            return Err(SessionError::Invalid(
                "trace belongs to another session".into(),
            ));
        }
        if trace.kind == TraceKind::Main {
            if let Some(existing) = &self.main_trace {
                if existing != &trace.trace_id {
                    return Err(SessionError::Invalid(
                        "session already has a main trace".into(),
                    ));
                }
            } else {
                self.main_trace = Some(trace.trace_id.clone());
            }
        }
        self.traces.push(trace);
        Ok(())
    }

    /// Validate session bounds and the main work invariant.
    pub fn validate(&self) -> Result<(), SessionError> {
        if self.session_id.trim().is_empty() || self.user_id.trim().is_empty() {
            return Err(SessionError::Invalid(
                "session_id and user_id must not be empty".into(),
            ));
        }
        self.work.validate()?;
        if self.lifecycle == SessionLifecycle::Ended && self.ended_at.is_none() {
            return Err(SessionError::Invalid(
                "ended session must have ended_at".into(),
            ));
        }
        if self.main_trace.is_some()
            && !self
                .traces
                .iter()
                .any(|trace| Some(&trace.trace_id) == self.main_trace.as_ref())
        {
            return Err(SessionError::Invalid(
                "main_trace does not name a registered trace".into(),
            ));
        }
        Ok(())
    }
}

/// The bundle loaded by `SessionStore`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBundle {
    /// Conversation state.
    pub conversation: ConversationState,
    /// Session lifecycle/presence/work.
    pub session: SessionState,
    /// Frame stack.
    pub frames: FrameStack,
}

/// Persistence seam for sessions. Phase 1 implements the RDF-backed store.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Load one session bundle.
    async fn load(&self, session_id: &str) -> Result<Option<SessionBundle>, SessionError>;

    /// Save session state with an optimistic revision check.
    async fn save_session(
        &self,
        state: &SessionState,
        expected_revision: u64,
    ) -> Result<SessionState, SessionError>;

    /// Save the frame stack with an optimistic revision check.
    async fn save_frames(
        &self,
        session_id: &str,
        stack: &FrameStack,
        expected_revision: u64,
    ) -> Result<FrameStack, SessionError>;

    /// List sessions from RDF with a cursor.
    async fn list_sessions(
        &self,
        user_iri: &str,
        cursor: Option<&str>,
        limit: usize,
        filter: SessionFilter,
    ) -> Result<SessionPage, SessionError>;

    /// List one session's trace manifests.
    async fn list_traces(
        &self,
        session_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<TracePage, SessionError>;

    /// Register a trace manifest.
    async fn register_trace(
        &self,
        session_id: &str,
        trace: &SessionTrace,
    ) -> Result<(), SessionError>;

    /// Soft-delete or purge a session.
    async fn delete(
        &self,
        session_id: &str,
        mode: DeleteMode,
        actor: &str,
    ) -> Result<(), SessionError>;
}

/// Session-contract failures.
#[derive(Debug, Error)]
pub enum SessionError {
    /// Invalid field or invariant.
    #[error("invalid session: {0}")]
    Invalid(String),
    /// Parallel work cap reached.
    #[error("parallel work limit reached")]
    WorkLimit,
    /// Frame stack cap reached or root protected.
    #[error("frame stack limit reached")]
    FrameLimit,
    /// Frame operation invalid.
    #[error("invalid frame: {0}")]
    FrameInvalid(String),
    /// Optimistic revision mismatch.
    #[error("conflict: expected revision {expected}, current revision {current}")]
    Conflict {
        /// Expected revision.
        expected: u64,
        /// Current revision.
        current: u64,
    },
    /// Session is not open.
    #[error("session is {0}")]
    NotOpen(String),
    /// Session does not exist.
    #[error("session not found: {0}")]
    NotFound(String),
    /// Store unavailable.
    #[error("session store unavailable: {0}")]
    Unavailable(String),
    /// Graph or append-only persistence failed.
    #[error("session persistence: {0}")]
    Persistence(String),
    /// Filesystem failure while removing a session directory.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Result alias for session operations.
pub type Result<T, E = SessionError> = std::result::Result<T, E>;

/// RDF-backed session store. It is the only writer for `mm:Session`,
/// `mm:SessionTrace`, and the frame stack; the live registry is a cache.
#[derive(Debug, Clone)]
pub struct RdfSessionStore {
    engine: Arc<RdfEngine>,
    agentstream: Arc<AgentstreamStore>,
    base_dir: PathBuf,
}

impl RdfSessionStore {
    /// Build over the shared engine, event store, and data root.
    pub fn new(
        engine: Arc<RdfEngine>,
        agentstream: Arc<AgentstreamStore>,
        base_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            engine,
            agentstream,
            base_dir: base_dir.into(),
        }
    }

    /// The engine, for callers that need a health probe.
    pub fn engine(&self) -> &Arc<RdfEngine> {
        &self.engine
    }

    async fn load_session_state(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionState>, SessionError> {
        rdf::load_as(
            &self.engine,
            &rdf::session_state_graph(session_id),
            &session_iri(session_id),
        )
        .await
        .map_err(|error| SessionError::Persistence(error.to_string()))
    }

    async fn load_conversation(
        &self,
        session_id: &str,
    ) -> Result<Option<ConversationState>, SessionError> {
        rdf::load_as(
            &self.engine,
            &rdf::conversation_graph(session_id),
            &session_iri(session_id),
        )
        .await
        .map_err(|error| SessionError::Persistence(error.to_string()))
    }

    async fn load_frames(&self, session_id: &str) -> Result<Option<FrameStack>, SessionError> {
        rdf::load_as(
            &self.engine,
            &rdf::session_state_graph(session_id),
            &rdf::frame_stack_iri(session_id),
        )
        .await
        .map_err(|error| SessionError::Persistence(error.to_string()))
    }

    async fn append(
        &self,
        kind: ConversationEventKind,
        session_id: &str,
        revision: u64,
        origin: StateOrigin,
    ) -> Result<(), SessionError> {
        ConversationStore::new(Arc::clone(&self.engine), Arc::clone(&self.agentstream))
            .append_event(ConversationEvent::lifecycle(
                kind, session_id, revision, origin,
            ))
            .await
            .map_err(|error| SessionError::Persistence(error.to_string()))
    }

    async fn write_session(&self, state: &SessionState) -> Result<(), SessionError> {
        let graph = rdf::session_state_graph(&state.session_id);
        let subject = session_iri(&state.session_id);
        rdf::upsert(
            &self.engine,
            &graph,
            &subject,
            &format!("{}Session", rdf::MM_NS),
            Some(&state.session_id),
            &serde_json::to_value(state)
                .map_err(|error| SessionError::Persistence(error.to_string()))?,
        )
        .await
        .map_err(|error| SessionError::Persistence(error.to_string()))?;
        let mut quads = vec![
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}sessionId", rdf::MM_NS),
                rdf::lit(state.session_id.clone()),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}userId", rdf::MM_NS),
                rdf::lit(state.user_id.clone()),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}lifecycle", rdf::MM_NS),
                rdf::lit(state.lifecycle.as_str()),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}presence", rdf::MM_NS),
                rdf::lit(serde_json::to_string(&state.presence).unwrap_or_default()),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}openedAt", rdf::MM_NS),
                rdf::typed_lit(
                    state.opened_at.to_string(),
                    &format!("{}integer", rdf::XSD_NS),
                ),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}lastActivityAt", rdf::MM_NS),
                rdf::typed_lit(
                    state.last_activity_at.to_string(),
                    &format!("{}integer", rdf::XSD_NS),
                ),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}resumeEpoch", rdf::MM_NS),
                rdf::typed_lit(
                    state.resume.epoch.to_string(),
                    &format!("{}integer", rdf::XSD_NS),
                ),
            ),
        ];
        for trace in &state.traces {
            quads.push(RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}hasTrace", rdf::MM_NS),
                rdf::iri(rdf::trace_iri(&trace.trace_id)),
            ));
        }
        if let Some(main_trace) = &state.main_trace {
            quads.push(RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}mainTrace", rdf::MM_NS),
                rdf::iri(rdf::trace_iri(main_trace)),
            ));
        }
        if let Some(dir) = &state.session_dir {
            quads.push(RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}sessionDir", rdf::MM_NS),
                rdf::lit(dir.clone()),
            ));
        }
        if let Some(deleted_at) = state.deleted_at {
            quads.push(RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}deletedAt", rdf::MM_NS),
                rdf::typed_lit(deleted_at.to_string(), &format!("{}integer", rdf::XSD_NS)),
            ));
        }
        self.engine
            .insert_all(quads)
            .await
            .map_err(|error| SessionError::Persistence(error.to_string()))?;
        Ok(())
    }

    async fn summary(&self, state: &SessionState) -> Result<SessionSummary, SessionError> {
        let conversation = self.load_conversation(&state.session_id).await?;
        let mut trace_kinds: Vec<TraceKind> = state.traces.iter().map(|trace| trace.kind).collect();
        trace_kinds.sort_by_key(|kind| format!("{kind:?}"));
        trace_kinds.dedup();
        Ok(SessionSummary {
            session_id: state.session_id.clone(),
            user_id: state.user_id.clone(),
            topic: conversation.and_then(|value| value.topic),
            lifecycle: state.lifecycle,
            presence: state.presence,
            working: state.working(),
            trace_count: u32::try_from(state.traces.len()).unwrap_or(u32::MAX),
            trace_kinds,
            last_activity_at: state.last_activity_at,
            session_dir: state.session_dir.clone(),
            resumable: state.lifecycle == SessionLifecycle::Ended,
            deleted: state.deleted_at.is_some(),
        })
    }

    async fn write_index(&self, state: &SessionState) -> Result<(), SessionError> {
        let summary = self.summary(state).await?;
        let subject = session_iri(&state.session_id);
        rdf::upsert(
            &self.engine,
            rdf::SESSION_INDEX_GRAPH,
            &subject,
            &format!("{}Session", rdf::MM_NS),
            summary.topic.as_deref(),
            &serde_json::to_value(&summary)
                .map_err(|error| SessionError::Persistence(error.to_string()))?,
        )
        .await
        .map_err(|error| SessionError::Persistence(error.to_string()))?;
        let quads = vec![
            RdfEngine::quad(
                rdf::SESSION_INDEX_GRAPH,
                &subject,
                &format!("{}sessionId", rdf::MM_NS),
                rdf::lit(state.session_id.clone()),
            ),
            RdfEngine::quad(
                rdf::SESSION_INDEX_GRAPH,
                &subject,
                &format!("{}userId", rdf::MM_NS),
                rdf::lit(state.user_id.clone()),
            ),
            RdfEngine::quad(
                rdf::SESSION_INDEX_GRAPH,
                &subject,
                &format!("{}lifecycle", rdf::MM_NS),
                rdf::lit(state.lifecycle.as_str()),
            ),
        ];
        self.engine
            .insert_all(quads)
            .await
            .map_err(|error| SessionError::Persistence(error.to_string()))?;
        Ok(())
    }

    async fn write_trace(&self, trace: &SessionTrace) -> Result<(), SessionError> {
        let graph = rdf::session_state_graph(&trace.session_id);
        let subject = rdf::trace_iri(&trace.trace_id);
        rdf::upsert(
            &self.engine,
            &graph,
            &subject,
            &format!("{}SessionTrace", rdf::MM_NS),
            Some(&trace.trace_id),
            &serde_json::to_value(trace)
                .map_err(|error| SessionError::Persistence(error.to_string()))?,
        )
        .await
        .map_err(|error| SessionError::Persistence(error.to_string()))?;
        let quads = vec![
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}traceId", rdf::MM_NS),
                rdf::lit(trace.trace_id.clone()),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}sessionId", rdf::MM_NS),
                rdf::lit(trace.session_id.clone()),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}traceKind", rdf::MM_NS),
                rdf::lit(serde_json::to_string(&trace.kind).unwrap_or_default()),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}agentstreamId", rdf::MM_NS),
                rdf::lit(trace.agentstream_id.clone()),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}firstSeq", rdf::MM_NS),
                rdf::typed_lit(
                    trace.first_seq.to_string(),
                    &format!("{}integer", rdf::XSD_NS),
                ),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}lastSeq", rdf::MM_NS),
                rdf::typed_lit(
                    trace.last_seq.to_string(),
                    &format!("{}integer", rdf::XSD_NS),
                ),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}eventCount", rdf::MM_NS),
                rdf::typed_lit(
                    trace.event_count.to_string(),
                    &format!("{}integer", rdf::XSD_NS),
                ),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}startedAt", rdf::MM_NS),
                rdf::typed_lit(
                    trace.started_at.to_string(),
                    &format!("{}integer", rdf::XSD_NS),
                ),
            ),
        ];
        self.engine
            .insert_all(quads)
            .await
            .map_err(|error| SessionError::Persistence(error.to_string()))?;
        Ok(())
    }

    async fn write_frames(&self, stack: &FrameStack, session_id: &str) -> Result<(), SessionError> {
        let graph = rdf::session_state_graph(session_id);
        let subject = rdf::frame_stack_iri(session_id);
        rdf::upsert(
            &self.engine,
            &graph,
            &subject,
            &format!("{}FrameStack", rdf::MM_NS),
            None,
            &serde_json::to_value(stack)
                .map_err(|error| SessionError::Persistence(error.to_string()))?,
        )
        .await
        .map_err(|error| SessionError::Persistence(error.to_string()))?;
        self.engine
            .insert_all(vec![RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}frameStackRevision", rdf::MM_NS),
                rdf::typed_lit(
                    stack.revision.to_string(),
                    &format!("{}integer", rdf::XSD_NS),
                ),
            )])
            .await
            .map_err(|error| SessionError::Persistence(error.to_string()))?;
        Ok(())
    }

    async fn append_session_changes(
        &self,
        old: Option<&SessionState>,
        next: &SessionState,
    ) -> Result<(), SessionError> {
        if old.is_none_or(|value| value.presence != next.presence) {
            self.append(
                ConversationEventKind::PresenceChanged,
                &next.session_id,
                next.revision,
                StateOrigin::System,
            )
            .await?;
        }
        if old.is_none_or(|value| value.work != next.work) {
            self.append(
                ConversationEventKind::WorkItemChanged,
                &next.session_id,
                next.revision,
                StateOrigin::System,
            )
            .await?;
        }
        if next.lifecycle == SessionLifecycle::Ended
            && old.is_none_or(|value| value.lifecycle != SessionLifecycle::Ended)
        {
            self.append(
                ConversationEventKind::SessionEnded,
                &next.session_id,
                next.revision,
                StateOrigin::System,
            )
            .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl SessionStore for RdfSessionStore {
    async fn load(&self, session_id: &str) -> Result<Option<SessionBundle>, SessionError> {
        let Some(session) = self.load_session_state(session_id).await? else {
            return Ok(None);
        };
        let conversation = self
            .load_conversation(session_id)
            .await?
            .ok_or_else(|| SessionError::Invalid("session has no conversation state".into()))?;
        let frames = self
            .load_frames(session_id)
            .await?
            .ok_or_else(|| SessionError::Invalid("session has no frame stack".into()))?;
        Ok(Some(SessionBundle {
            conversation,
            session,
            frames,
        }))
    }

    async fn save_session(
        &self,
        state: &SessionState,
        expected_revision: u64,
    ) -> Result<SessionState, SessionError> {
        state.validate()?;
        let current = self.load_session_state(&state.session_id).await?;
        let current_revision = current.as_ref().map_or(0, |value| value.revision);
        if current_revision != expected_revision {
            return Err(SessionError::Conflict {
                expected: expected_revision,
                current: current_revision,
            });
        }
        if current.as_ref() == Some(state) {
            return Ok(state.clone());
        }
        self.write_session(state).await?;
        self.write_index(state).await?;
        self.append_session_changes(current.as_ref(), state).await?;
        Ok(state.clone())
    }

    async fn save_frames(
        &self,
        session_id: &str,
        stack: &FrameStack,
        expected_revision: u64,
    ) -> Result<FrameStack, SessionError> {
        stack.validate()?;
        let current = self.load_frames(session_id).await?;
        let current_revision = current.as_ref().map_or(0, |value| value.revision);
        if current_revision != expected_revision {
            return Err(SessionError::Conflict {
                expected: expected_revision,
                current: current_revision,
            });
        }
        if current.as_ref() == Some(stack) {
            return Ok(stack.clone());
        }
        self.write_frames(stack, session_id).await?;
        let kind = match current.as_ref().map(|value| value.frames.len()) {
            Some(length) if length < stack.frames.len() => ConversationEventKind::FramePushed,
            Some(length) if length > stack.frames.len() => ConversationEventKind::FramePopped,
            _ => ConversationEventKind::FrameUpdated,
        };
        self.append(kind, session_id, stack.revision, StateOrigin::Cm)
            .await?;
        Ok(stack.clone())
    }

    async fn list_sessions(
        &self,
        user_iri: &str,
        cursor: Option<&str>,
        limit: usize,
        filter: SessionFilter,
    ) -> Result<SessionPage, SessionError> {
        let limit = limit.clamp(1, 100);
        let mut rows: Vec<SessionSummary> = rdf::list_as(
            &self.engine,
            rdf::SESSION_INDEX_GRAPH,
            &format!("{}Session", rdf::MM_NS),
        )
        .await
        .map_err(|error| SessionError::Persistence(error.to_string()))?;
        rows.retain(|row| {
            (user_iri.is_empty()
                || row.user_id == user_iri
                || rdf::user_iri(&row.user_id) == user_iri)
                && (filter.include_deleted || !row.deleted)
                && filter.status.is_none_or(|status| row.lifecycle == status)
                && filter.query.as_ref().is_none_or(|query| {
                    let query = query.to_lowercase();
                    row.session_id.to_lowercase().contains(&query)
                        || row
                            .topic
                            .as_ref()
                            .is_some_and(|topic| topic.to_lowercase().contains(&query))
                })
        });
        rows.sort_by(|a, b| {
            b.last_activity_at
                .cmp(&a.last_activity_at)
                .then_with(|| b.session_id.cmp(&a.session_id))
        });
        if let Some(cursor) = cursor {
            let cursor = decode_cursor(cursor)?;
            rows.retain(|row| {
                row.last_activity_at < cursor.at
                    || (row.last_activity_at == cursor.at && row.session_id < cursor.id)
            });
        }
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = if has_more {
            rows.last()
                .map(|row| encode_cursor(row.last_activity_at, &row.session_id))
        } else {
            None
        };
        Ok(SessionPage {
            items: rows,
            next_cursor,
            has_more,
        })
    }

    async fn list_traces(
        &self,
        session_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<TracePage, SessionError> {
        let limit = limit.clamp(1, 100);
        let mut rows: Vec<SessionTrace> = rdf::list_as(
            &self.engine,
            &rdf::session_state_graph(session_id),
            &format!("{}SessionTrace", rdf::MM_NS),
        )
        .await
        .map_err(|error| SessionError::Persistence(error.to_string()))?;
        rows.retain(|row| row.session_id == session_id);
        rows.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.trace_id.cmp(&a.trace_id))
        });
        if let Some(cursor) = cursor {
            let cursor = decode_cursor(cursor)?;
            rows.retain(|row| {
                row.started_at < cursor.at
                    || (row.started_at == cursor.at && row.trace_id < cursor.id)
            });
        }
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = if has_more {
            rows.last()
                .map(|row| encode_cursor(row.started_at, &row.trace_id))
        } else {
            None
        };
        Ok(TracePage {
            items: rows,
            next_cursor,
            has_more,
        })
    }

    async fn register_trace(
        &self,
        session_id: &str,
        trace: &SessionTrace,
    ) -> Result<(), SessionError> {
        let mut session = self
            .load_session_state(session_id)
            .await?
            .ok_or_else(|| SessionError::NotFound(session_id.into()))?;
        if session
            .traces
            .iter()
            .any(|item| item.trace_id == trace.trace_id)
        {
            return Ok(());
        }
        session.register_trace(trace.clone())?;
        session.revision = session.revision.saturating_add(1);
        self.write_trace(trace).await?;
        self.write_session(&session).await?;
        self.write_index(&session).await?;
        self.append(
            ConversationEventKind::TraceRegistered,
            session_id,
            session.revision,
            StateOrigin::System,
        )
        .await?;
        Ok(())
    }

    async fn delete(
        &self,
        session_id: &str,
        mode: DeleteMode,
        actor: &str,
    ) -> Result<(), SessionError> {
        let Some(mut session) = self.load_session_state(session_id).await? else {
            return Err(SessionError::NotFound(session_id.into()));
        };
        match mode {
            DeleteMode::Soft => {
                if session.deleted_at.is_some() {
                    return Ok(());
                }
                session.deleted_at = Some(rdf::now_ms());
                session.deleted_by = Some(actor.into());
                session.revision = session.revision.saturating_add(1);
                self.write_session(&session).await?;
                self.write_index(&session).await?;
                self.append(
                    ConversationEventKind::SessionDeleted,
                    session_id,
                    session.revision,
                    StateOrigin::User,
                )
                .await?;
            }
            DeleteMode::Purge => {
                let existing: Vec<SessionSummary> = rdf::list_as(
                    &self.engine,
                    rdf::SESSION_INDEX_GRAPH,
                    &format!("{}Session", rdf::MM_NS),
                )
                .await
                .map_err(|error| SessionError::Persistence(error.to_string()))?;
                let remaining: Vec<_> = existing
                    .into_iter()
                    .filter(|row| row.session_id != session_id)
                    .collect();
                for graph in [
                    rdf::session_state_graph(session_id),
                    rdf::conversation_graph(session_id),
                    rdf::session_graph(session_id),
                ] {
                    self.engine
                        .store()
                        .handle()
                        .clear_graph(&graph)
                        .await
                        .map_err(|error| SessionError::Persistence(error.to_string()))?;
                }
                // The RDF helper is append-only and has no per-subject delete, so
                // rebuild the small index without the purged row.
                self.engine
                    .store()
                    .handle()
                    .clear_graph(rdf::SESSION_INDEX_GRAPH)
                    .await
                    .map_err(|error| SessionError::Persistence(error.to_string()))?;
                for row in remaining {
                    if let Ok(Some(state)) = self.load_session_state(&row.session_id).await {
                        self.write_index(&state).await?;
                    }
                }
                let dir = self.base_dir.join("sessions").join(session_id);
                if dir.exists() {
                    std::fs::remove_dir_all(&dir)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct PageCursor {
    at: i64,
    id: String,
}

fn encode_cursor(at: i64, id: &str) -> String {
    format!("{at}:{id}")
}

fn decode_cursor(cursor: &str) -> Result<PageCursor, SessionError> {
    let (at, id) = cursor
        .split_once(':')
        .ok_or_else(|| SessionError::Invalid("invalid session cursor".into()))?;
    let at = at
        .parse()
        .map_err(|_| SessionError::Invalid("invalid session cursor".into()))?;
    if id.is_empty() {
        return Err(SessionError::Invalid("invalid session cursor".into()));
    }
    Ok(PageCursor {
        at,
        id: id.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: &str, now: i64) -> ConversationFrame {
        ConversationFrame {
            id: id.into(),
            template_id: format!("frame.{id}"),
            template_version: 1,
            kind: FrameKind::Work,
            status: FrameStatus::Active,
            entered_at: now,
            updated_at: now,
            expires_at: None,
        }
    }

    #[test]
    fn work_catalog_always_has_main_and_enforces_parallel_cap() {
        let mut work = WorkCatalog::new(1);
        assert_eq!(work.main.id, "main");
        assert!(!work.working());
        for index in 0..MAX_PARALLEL_WORK {
            work.upsert(WorkItem::new(
                format!("w{index}"),
                format!("work {index}"),
                WorkKind::Agent,
                1,
            ))
            .unwrap();
        }
        assert!(work
            .upsert(WorkItem::new("overflow", "overflow", WorkKind::Agent, 1))
            .is_err());
        assert!(work.remove_terminal("main").is_err());
    }

    #[test]
    fn working_is_derived_from_work_status_only() {
        let mut session = SessionState::new("s", "u", 1);
        assert!(!session.working());
        session.work.main.status = WorkStatus::Working;
        assert!(session.working());
        session.work.main.status = WorkStatus::Waiting;
        assert!(!session.working());
    }

    #[test]
    fn frame_root_is_protected_and_stack_is_capped() {
        let mut stack = FrameStack::new(frame("intro", 1));
        assert!(stack.pop(2).is_err());
        for index in 1..MAX_FRAME_DEPTH {
            stack
                .push(frame(&format!("f{index}"), index as i64))
                .unwrap();
        }
        assert!(stack.push(frame("overflow", 99)).is_err());
        assert_eq!(stack.frames.len(), MAX_FRAME_DEPTH);
        let popped = stack.pop(100).unwrap();
        assert_eq!(popped.id, "f7");
        assert_eq!(stack.top().unwrap().status, FrameStatus::Active);
    }

    #[test]
    fn main_trace_is_singular() {
        let mut session = SessionState::new("s", "u", 1);
        session
            .register_trace(SessionTrace {
                trace_id: "main".into(),
                session_id: "s".into(),
                kind: TraceKind::Main,
                agentstream_id: "main".into(),
                first_seq: 1,
                last_seq: 1,
                event_count: 1,
                started_at: 1,
                ended_at: None,
                size_bytes: 1,
            })
            .unwrap();
        assert_eq!(session.main_trace.as_deref(), Some("main"));
        let error = session
            .register_trace(SessionTrace {
                trace_id: "other".into(),
                session_id: "s".into(),
                kind: TraceKind::Main,
                agentstream_id: "other".into(),
                first_seq: 1,
                last_seq: 1,
                event_count: 1,
                started_at: 1,
                ended_at: None,
                size_bytes: 1,
            })
            .unwrap_err();
        assert!(matches!(error, SessionError::Invalid(_)));
    }

    #[test]
    fn session_activity_deadline_prefers_explicit_expiry() {
        let mut session = SessionState::new("s", "u", 100);
        session.last_user_input_at = Some(50);
        session.last_work_activity_at = Some(70);
        assert_eq!(session.activity_deadline(), 70);
        session.expires_at = Some(999);
        assert_eq!(session.activity_deadline(), 999);
    }
}
