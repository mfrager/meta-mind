//! Conversation state contracts: one topic, one depth, and a bounded typed
//! status dictionary.
//!
//! This module is deliberately independent of RDF and the HTTP layer. The
//! canonical serializer here is the only serializer used by the store, the
//! prompt block, and the inspector DTO; later phases add persistence around
//! these types instead of introducing a second state shape.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::agentstream::{AgentstreamEvent, AgentstreamStore, EventProvenance, EventType};
use crate::engine::RdfEngine;
use crate::rdf::{self, session_iri};

/// Maximum topic label length, in Unicode scalar values.
pub const MAX_TOPIC_CHARS: usize = 160;
/// Maximum custom status keys.
pub const MAX_STATUS_KEYS: usize = 24;
/// Maximum canonical status JSON size.
pub const MAX_STATUS_BYTES: usize = 2 * 1024;
/// Maximum text value length.
pub const MAX_TEXT_CHARS: usize = 500;
/// Maximum list items.
pub const MAX_LIST_ITEMS: usize = 8;
/// Maximum list item length.
pub const MAX_LIST_ITEM_CHARS: usize = 120;
/// Maximum ordinary reference id length.
pub const MAX_REF_ID_CHARS: usize = 1_024;

/// Who wrote the current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StateOrigin {
    /// The operator.
    User,
    /// The conversation manager.
    #[default]
    Cm,
    /// The session/automation subsystem.
    System,
}

impl StateOrigin {
    /// Stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Cm => "cm",
            Self::System => "system",
        }
    }
}

/// The conversation's current topic and bounded working state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationState {
    /// Owning session.
    pub session_id: String,
    /// Monotonic optimistic-concurrency revision.
    pub revision: u64,
    /// Plain-language topic label. `None` until the CM names it.
    pub topic: Option<String>,
    /// Drill level for the topic, 0..=7.
    pub depth: u8,
    /// Typed status variables, sorted by key.
    pub status: BTreeMap<String, StatusValue>,
    /// Last write instant, Unix ms.
    pub updated_at: i64,
    /// Writer of the last mutation.
    pub updated_by: StateOrigin,
}

impl ConversationState {
    /// A new empty state. Topic is intentionally not seeded.
    pub fn new(session_id: impl Into<String>, now_ms: i64) -> Self {
        Self {
            session_id: session_id.into(),
            revision: 1,
            topic: None,
            depth: 0,
            status: BTreeMap::new(),
            updated_at: now_ms,
            updated_by: StateOrigin::System,
        }
    }

    /// Validate every invariant that does not require another store.
    pub fn validate(&self) -> Result<(), ConversationError> {
        if self.session_id.trim().is_empty() {
            return Err(ConversationError::InvalidArguments(
                "session_id must not be empty".into(),
            ));
        }
        if self.revision == 0 {
            return Err(ConversationError::InvalidArguments(
                "revision must be positive".into(),
            ));
        }
        if self.depth > 7 {
            return Err(ConversationError::InvalidArguments(
                "depth must be between 0 and 7".into(),
            ));
        }
        if let Some(topic) = &self.topic {
            let trimmed = topic.trim();
            if trimmed.is_empty() {
                return Err(ConversationError::InvalidArguments(
                    "topic must not be empty when set".into(),
                ));
            }
            if trimmed.chars().count() > MAX_TOPIC_CHARS {
                return Err(ConversationError::InvalidArguments(format!(
                    "topic exceeds {MAX_TOPIC_CHARS} characters"
                )));
            }
        }
        if self.status.len() > MAX_STATUS_KEYS {
            return Err(ConversationError::StateTooLarge {
                bytes: self.status_bytes()?,
                keys: self.status.len(),
            });
        }
        for (key, value) in &self.status {
            validate_status_key(key)?;
            validate_reserved_value(key, value)?;
            value.validate()?;
        }
        let bytes = self.status_bytes()?;
        if bytes > MAX_STATUS_BYTES {
            return Err(ConversationError::StateTooLarge {
                bytes,
                keys: self.status.len(),
            });
        }
        Ok(())
    }

    /// Canonical status JSON, used by the store, prompt, and wire projection.
    pub fn status_json(&self) -> Result<String, ConversationError> {
        Ok(serde_json::to_string(&self.status)?)
    }

    /// Canonical status byte length.
    pub fn status_bytes(&self) -> Result<usize, ConversationError> {
        Ok(serde_json::to_vec(&self.status)?.len())
    }

    /// The complete canonical state JSON (not the `[STATE]` envelope).
    pub fn canonical_json(&self) -> Result<String, ConversationError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Set the topic; returns whether anything changed.
    pub fn set_topic(
        &mut self,
        topic: Option<String>,
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<bool, ConversationError> {
        let normalized = topic.map(|value| value.trim().to_owned());
        if normalized.as_deref().is_some_and(str::is_empty) {
            return Err(ConversationError::InvalidArguments(
                "topic must not be empty".into(),
            ));
        }
        if normalized
            .as_deref()
            .is_some_and(|value| value.contains(['\n', '\r']))
        {
            return Err(ConversationError::InvalidArguments(
                "topic must not contain newlines".into(),
            ));
        }
        if normalized
            .as_deref()
            .is_some_and(|value| value.chars().count() > MAX_TOPIC_CHARS)
        {
            return Err(ConversationError::InvalidArguments(format!(
                "topic exceeds {MAX_TOPIC_CHARS} characters"
            )));
        }
        if self.topic == normalized {
            return Ok(false);
        }
        self.topic = normalized;
        self.touch(origin, now_ms);
        Ok(true)
    }

    /// Set the depth; returns whether anything changed.
    pub fn set_depth(
        &mut self,
        depth: u8,
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<bool, ConversationError> {
        if depth > 7 {
            return Err(ConversationError::InvalidArguments(
                "depth must be between 0 and 7".into(),
            ));
        }
        if self.depth == depth {
            return Ok(false);
        }
        self.depth = depth;
        self.touch(origin, now_ms);
        Ok(true)
    }

    /// Set one status variable.
    pub fn set_status(
        &mut self,
        key: impl Into<String>,
        value: StatusValue,
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<bool, ConversationError> {
        let key = key.into();
        validate_status_key(&key)?;
        validate_reserved_value(&key, &value)?;
        value.validate()?;
        if self.status.get(&key) == Some(&value) {
            return Ok(false);
        }
        let mut candidate = self.status.clone();
        candidate.insert(key, value);
        if candidate.len() > MAX_STATUS_KEYS {
            return Err(ConversationError::StateTooLarge {
                bytes: serde_json::to_vec(&candidate)?.len(),
                keys: candidate.len(),
            });
        }
        let bytes = serde_json::to_vec(&candidate)?.len();
        if bytes > MAX_STATUS_BYTES {
            return Err(ConversationError::StateTooLarge {
                bytes,
                keys: candidate.len(),
            });
        }
        self.status = candidate;
        self.touch(origin, now_ms);
        Ok(true)
    }

    /// Merge several status values as one revision. Validation is performed on
    /// the complete candidate map before anything is written.
    pub fn set_status_values(
        &mut self,
        values: impl IntoIterator<Item = (String, StatusValue)>,
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<bool, ConversationError> {
        let mut candidate = self.status.clone();
        for (key, value) in values {
            validate_status_key(&key)?;
            validate_reserved_value(&key, &value)?;
            value.validate()?;
            candidate.insert(key, value);
        }
        if candidate.len() > MAX_STATUS_KEYS {
            return Err(ConversationError::StateTooLarge {
                bytes: serde_json::to_vec(&candidate)?.len(),
                keys: candidate.len(),
            });
        }
        let bytes = serde_json::to_vec(&candidate)?.len();
        if bytes > MAX_STATUS_BYTES {
            return Err(ConversationError::StateTooLarge {
                bytes,
                keys: candidate.len(),
            });
        }
        if candidate == self.status {
            return Ok(false);
        }
        self.status = candidate;
        self.touch(origin, now_ms);
        Ok(true)
    }

    /// Remove several status variables as one revision.
    pub fn unset_status_values(
        &mut self,
        keys: &[String],
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<bool, ConversationError> {
        let mut candidate = self.status.clone();
        for key in keys {
            candidate.remove(key);
        }
        if candidate == self.status {
            return Ok(false);
        }
        self.status = candidate;
        self.touch(origin, now_ms);
        Ok(true)
    }

    /// Remove one status variable; absent is a no-op.
    pub fn unset_status(
        &mut self,
        key: &str,
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<bool, ConversationError> {
        if self.status.remove(key).is_some() {
            self.touch(origin, now_ms);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn touch(&mut self, origin: StateOrigin, now_ms: i64) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = now_ms;
        self.updated_by = origin;
    }
}

/// One typed status value. Wire serialization is untagged: a text is a JSON
/// string, a ref is a JSON object, and so on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StatusValue {
    /// Short text.
    Text(String),
    /// Integer.
    Integer(i64),
    /// Boolean.
    Bool(bool),
    /// Ordered list of short strings.
    List(Vec<String>),
    /// A labeled reference.
    Ref(StatusRef),
}

impl StatusValue {
    /// Validate type-local bounds.
    pub fn validate(&self) -> Result<(), ConversationError> {
        match self {
            Self::Text(value) => {
                if value.chars().count() > MAX_TEXT_CHARS {
                    return Err(ConversationError::InvalidArguments(format!(
                        "text exceeds {MAX_TEXT_CHARS} characters"
                    )));
                }
            }
            Self::List(values) => {
                if values.len() > MAX_LIST_ITEMS {
                    return Err(ConversationError::InvalidArguments(format!(
                        "list exceeds {MAX_LIST_ITEMS} items"
                    )));
                }
                if values
                    .iter()
                    .any(|value| value.chars().count() > MAX_LIST_ITEM_CHARS)
                {
                    return Err(ConversationError::InvalidArguments(format!(
                        "list item exceeds {MAX_LIST_ITEM_CHARS} characters"
                    )));
                }
            }
            Self::Ref(reference) => reference.validate()?,
            Self::Integer(_) | Self::Bool(_) => {}
        }
        Ok(())
    }

    /// A short type label for errors and tests.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Integer(_) => "integer",
            Self::Bool(_) => "bool",
            Self::List(_) => "list",
            Self::Ref(_) => "ref",
        }
    }
}

/// A reference to an external object. It is a label, not a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusRef {
    /// Reference kind.
    pub kind: RefKind,
    /// Stable id or workspace-relative path.
    pub id: String,
    /// Optional operator-facing label.
    pub label: Option<String>,
    /// Whether the owning store verified it at write time.
    pub verified: bool,
}

impl StatusRef {
    /// Validate and normalize a reference.
    pub fn validate(&self) -> Result<(), ConversationError> {
        if self.id.trim().is_empty() {
            return Err(ConversationError::InvalidArguments(
                "ref id must not be empty".into(),
            ));
        }
        if self.id.chars().count() > MAX_REF_ID_CHARS {
            return Err(ConversationError::InvalidArguments(format!(
                "ref id exceeds {MAX_REF_ID_CHARS} characters"
            )));
        }
        if self.kind == RefKind::File
            && (self.id.starts_with('/') || self.id.split('/').any(|part| part == ".."))
        {
            return Err(ConversationError::InvalidArguments(
                "file ref must be workspace-relative and normalized".into(),
            ));
        }
        Ok(())
    }
}

/// Reference kinds allowed in conversation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// Workspace-relative file.
    File,
    /// Project entity.
    Project,
    /// Plan entity.
    Plan,
    /// Conversation frame.
    Frame,
    /// Agent run.
    AgentRun,
    /// Memory record.
    Memory,
    /// External URL.
    Url,
    /// Agent artifact.
    Artifact,
}

/// Artifact refs use the same shape as status refs.
pub type ArtifactRef = StatusRef;

/// The reserved status keys and their allowed shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservedStatusType {
    /// Short text.
    Text,
    /// Integer.
    Integer,
    /// Boolean.
    Bool,
    /// String list.
    List,
    /// Text or a reference.
    TextOrRef,
    /// A reference.
    Ref,
    /// The fixed phase enum.
    Phase,
}

/// Look up a reserved key's expected type.
pub fn reserved_status_type(key: &str) -> Option<ReservedStatusType> {
    match key {
        "phase" => Some(ReservedStatusType::Phase),
        "intent" | "notes" | "frame_step" => Some(ReservedStatusType::Text),
        "project" => Some(ReservedStatusType::TextOrRef),
        "tasks" | "next" | "blockers" | "questions" => Some(ReservedStatusType::List),
        "focus" => Some(ReservedStatusType::Ref),
        "frame_attempt" | "next_utterance_at" => Some(ReservedStatusType::Integer),
        "awaiting_user" => Some(ReservedStatusType::Bool),
        _ => None,
    }
}

/// Validate a status key, including custom-key syntax.
pub fn validate_status_key(key: &str) -> Result<(), ConversationError> {
    if key == "topic" || key == "depth" {
        return Err(ConversationError::InvalidArguments(format!(
            "{key} is not a status variable"
        )));
    }
    if key.is_empty() || key.len() > 32 {
        return Err(ConversationError::InvalidArguments(
            "status key must be 1–32 bytes".into(),
        ));
    }
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return Err(ConversationError::InvalidArguments(
            "status key must not be empty".into(),
        ));
    };
    if !first.is_ascii_lowercase() {
        return Err(ConversationError::InvalidArguments(
            "status key must start with a lowercase letter".into(),
        ));
    }
    if !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_') {
        return Err(ConversationError::InvalidArguments(
            "status key may contain only lowercase letters, digits, and underscores".into(),
        ));
    }
    Ok(())
}

fn validate_reserved_value(key: &str, value: &StatusValue) -> Result<(), ConversationError> {
    let Some(expected) = reserved_status_type(key) else {
        return Ok(());
    };
    let valid = match expected {
        ReservedStatusType::Text => matches!(value, StatusValue::Text(_)),
        ReservedStatusType::Integer => matches!(value, StatusValue::Integer(_)),
        ReservedStatusType::Bool => matches!(value, StatusValue::Bool(_)),
        ReservedStatusType::List => matches!(value, StatusValue::List(_)),
        ReservedStatusType::TextOrRef => {
            matches!(value, StatusValue::Text(_) | StatusValue::Ref(_))
        }
        ReservedStatusType::Ref => matches!(value, StatusValue::Ref(_)),
        ReservedStatusType::Phase => matches!(
            value,
            StatusValue::Text(text)
                if matches!(
                    text.as_str(),
                    "idle" | "working" | "waiting" | "blocked" | "done"
                )
        ),
    };
    if !valid {
        return Err(ConversationError::InvalidArguments(format!(
            "{key} expects {}, found {}",
            match expected {
                ReservedStatusType::Text => "text",
                ReservedStatusType::Integer => "integer",
                ReservedStatusType::Bool => "bool",
                ReservedStatusType::List => "list",
                ReservedStatusType::TextOrRef => "text or ref",
                ReservedStatusType::Ref => "ref",
                ReservedStatusType::Phase => "idle|working|waiting|blocked|done",
            },
            value.kind_name()
        )));
    }
    if key == "frame_step" {
        if let StatusValue::Text(text) = value {
            if text.chars().count() > 80 {
                return Err(ConversationError::InvalidArguments(
                    "frame_step exceeds 80 characters".into(),
                ));
            }
        }
    }
    if key == "frame_attempt" {
        if let StatusValue::Integer(attempt) = value {
            if !(0..=5).contains(attempt) {
                return Err(ConversationError::InvalidArguments(
                    "frame_attempt must be between 0 and 5".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Conversation-contract failures.
#[derive(Debug, Error)]
pub enum ConversationError {
    /// A field failed validation.
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    /// The status map exceeds a count or byte cap.
    #[error("state too large: {keys} keys / {bytes} bytes")]
    StateTooLarge {
        /// Canonical byte size.
        bytes: usize,
        /// Number of keys.
        keys: usize,
    },
    /// An optimistic revision check failed.
    #[error("conflict: expected revision {expected}, current revision {current}")]
    Conflict {
        /// Expected revision.
        expected: u64,
        /// Current revision.
        current: u64,
    },
    /// The frame stack cannot accept the operation.
    #[error("frame stack limit reached")]
    FrameLimit,
    /// A frame operation is invalid.
    #[error("invalid frame: {0}")]
    FrameInvalid(String),
    /// The session is not accepting writes.
    #[error("session ended")]
    SessionEnded,
    /// Serialization failed.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// The graph or append-only event store failed.
    #[error("conversation persistence: {0}")]
    Persistence(String),
}

/// Result alias for conversation operations.
pub type Result<T, E = ConversationError> = std::result::Result<T, E>;

/// One conversation-state event kind. The wire names are stable and additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationEventKind {
    /// Topic changed.
    TopicUpdated,
    /// Depth changed.
    DepthUpdated,
    /// Status variables changed.
    StatusUpdated,
    /// Factory event for a newly opened session.
    SessionOpened,
    /// Factory event for a resumed session.
    SessionResumed,
    /// Session ended.
    SessionEnded,
    /// Session soft-deleted or purged.
    SessionDeleted,
    /// Lazy directory created.
    SessionDirectoryCreated,
    /// Trace manifest registered.
    TraceRegistered,
    /// Presence changed.
    PresenceChanged,
    /// Work item changed.
    WorkItemChanged,
    /// Frame pushed.
    FramePushed,
    /// Frame replaced or updated.
    FrameUpdated,
    /// Frame popped.
    FramePopped,
}

impl ConversationEventKind {
    /// Stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TopicUpdated => "topic_updated",
            Self::DepthUpdated => "depth_updated",
            Self::StatusUpdated => "status_updated",
            Self::SessionOpened => "session_opened",
            Self::SessionResumed => "session_resumed",
            Self::SessionEnded => "session_ended",
            Self::SessionDeleted => "session_deleted",
            Self::SessionDirectoryCreated => "session_directory_created",
            Self::TraceRegistered => "trace_registered",
            Self::PresenceChanged => "presence_changed",
            Self::WorkItemChanged => "work_item_changed",
            Self::FramePushed => "frame_pushed",
            Self::FrameUpdated => "frame_updated",
            Self::FramePopped => "frame_popped",
        }
    }

    /// The agentstream event type used for this conversation event.
    pub fn event_type(self) -> EventType {
        match self {
            Self::TopicUpdated => EventType::TopicUpdated,
            Self::DepthUpdated => EventType::DepthUpdated,
            Self::StatusUpdated => EventType::StatusUpdated,
            Self::SessionOpened => EventType::SessionOpened,
            Self::SessionResumed => EventType::SessionResumed,
            Self::SessionEnded => EventType::SessionEnded,
            Self::SessionDeleted => EventType::SessionDeleted,
            Self::SessionDirectoryCreated => EventType::SessionDirectoryCreated,
            Self::TraceRegistered => EventType::TraceRegistered,
            Self::PresenceChanged => EventType::PresenceChanged,
            Self::WorkItemChanged => EventType::WorkItemChanged,
            Self::FramePushed => EventType::FramePushed,
            Self::FrameUpdated => EventType::FrameUpdated,
            Self::FramePopped => EventType::FramePopped,
        }
    }
}

/// A compact, replayable conversation/session event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationEvent {
    /// Event kind.
    pub kind: ConversationEventKind,
    /// Owning session.
    pub session_id: String,
    /// State revision after the write.
    pub revision: u64,
    /// Writer.
    pub origin: StateOrigin,
    /// New topic, for topic events.
    pub topic: Option<String>,
    /// New depth, for depth events.
    pub depth: Option<u8>,
    /// Status values set by the event.
    #[serde(default)]
    pub set: BTreeMap<String, StatusValue>,
    /// Status keys removed by the event.
    #[serde(default)]
    pub unset: Vec<String>,
    /// State update instant, when known.
    #[serde(default)]
    pub updated_at: Option<i64>,
}

impl ConversationEvent {
    /// A topic diff.
    pub fn topic_updated(
        session_id: impl Into<String>,
        revision: u64,
        topic: Option<String>,
        origin: StateOrigin,
    ) -> Self {
        Self {
            kind: ConversationEventKind::TopicUpdated,
            session_id: session_id.into(),
            revision,
            origin,
            topic,
            depth: None,
            set: BTreeMap::new(),
            unset: Vec::new(),
            updated_at: None,
        }
    }

    /// A depth diff.
    pub fn depth_updated(
        session_id: impl Into<String>,
        revision: u64,
        depth: u8,
        origin: StateOrigin,
    ) -> Self {
        Self {
            kind: ConversationEventKind::DepthUpdated,
            session_id: session_id.into(),
            revision,
            origin,
            topic: None,
            depth: Some(depth),
            set: BTreeMap::new(),
            unset: Vec::new(),
            updated_at: None,
        }
    }

    /// A status diff.
    pub fn status_updated(
        session_id: impl Into<String>,
        revision: u64,
        set: BTreeMap<String, StatusValue>,
        unset: Vec<String>,
        origin: StateOrigin,
    ) -> Self {
        Self {
            kind: ConversationEventKind::StatusUpdated,
            session_id: session_id.into(),
            revision,
            origin,
            topic: None,
            depth: None,
            set,
            unset,
            updated_at: None,
        }
    }

    /// A session/frame lifecycle event with no conversation-state diff.
    pub fn lifecycle(
        kind: ConversationEventKind,
        session_id: impl Into<String>,
        revision: u64,
        origin: StateOrigin,
    ) -> Self {
        Self {
            kind,
            session_id: session_id.into(),
            revision,
            origin,
            topic: None,
            depth: None,
            set: BTreeMap::new(),
            unset: Vec::new(),
            updated_at: None,
        }
    }

    /// Attach the state update instant.
    #[must_use]
    pub fn with_updated_at(mut self, updated_at: i64) -> Self {
        self.updated_at = Some(updated_at);
        self
    }

    fn payload(&self) -> Result<serde_json::Value, ConversationError> {
        let mut payload = serde_json::json!({
            "schema": "mm.conversation.v1",
            "sessionId": self.session_id,
            "revision": self.revision,
            "op": self.kind.as_str(),
            "origin": self.origin,
        });
        let object = payload
            .as_object_mut()
            .expect("conversation event payload is an object");
        if let Some(topic) = &self.topic {
            object.insert("topic".into(), serde_json::Value::String(topic.clone()));
        }
        if let Some(depth) = self.depth {
            object.insert("depth".into(), serde_json::json!(depth));
        }
        if !self.set.is_empty() {
            object.insert("set".into(), serde_json::to_value(&self.set)?);
        }
        if !self.unset.is_empty() {
            object.insert("unset".into(), serde_json::to_value(&self.unset)?);
        }
        if let Some(updated_at) = self.updated_at {
            object.insert("updatedAt".into(), serde_json::json!(updated_at));
        }
        Ok(payload)
    }
}

/// Durable conversation-state store backed by RDF and agentstream.
#[derive(Debug, Clone)]
pub struct ConversationStore {
    engine: Arc<RdfEngine>,
    agentstream: Arc<AgentstreamStore>,
}

impl ConversationStore {
    /// Build over the shared engine and append-only event store.
    pub fn new(engine: Arc<RdfEngine>, agentstream: Arc<AgentstreamStore>) -> Self {
        Self {
            engine,
            agentstream,
        }
    }

    /// Load the highest-revision materialized state.
    pub async fn load(
        &self,
        session_id: &str,
    ) -> Result<Option<ConversationState>, ConversationError> {
        rdf::load_as(
            &self.engine,
            &rdf::conversation_graph(session_id),
            &session_iri(session_id),
        )
        .await
        .map_err(|error| ConversationError::Persistence(error.to_string()))
    }

    /// Save a state with optimistic concurrency and append the matching events.
    pub async fn save(
        &self,
        state: &ConversationState,
        expected_revision: u64,
    ) -> Result<ConversationState, ConversationError> {
        state.validate()?;
        let current = self.load(&state.session_id).await?;
        let current_revision = current.as_ref().map_or(0, |value| value.revision);
        if current_revision != expected_revision {
            return Err(ConversationError::Conflict {
                expected: expected_revision,
                current: current_revision,
            });
        }
        if current.as_ref() == Some(state) {
            return Ok(state.clone());
        }
        self.write_state(state).await?;
        self.append_changes(current.as_ref(), state).await?;
        Ok(state.clone())
    }

    /// Append one event without changing materialized state.
    pub async fn append_event(&self, event: ConversationEvent) -> Result<(), ConversationError> {
        let payload = event.payload()?;
        let source = event.origin.as_str();
        self.agentstream
            .log_event(AgentstreamEvent::new(
                event.session_id.clone(),
                event.kind.event_type(),
                payload,
                EventProvenance {
                    source: source.into(),
                    model: None,
                },
            ))
            .await
            .map_err(|error| ConversationError::Persistence(error.to_string()))?;
        Ok(())
    }

    /// Rebuild the materialized state by replaying conversation events.
    #[allow(clippy::too_many_lines)]
    pub async fn replay(&self, session_id: &str) -> Result<ConversationState, ConversationError> {
        let events = self
            .agentstream
            .events(session_id)
            .await
            .map_err(|error| ConversationError::Persistence(error.to_string()))?;
        let mut state = ConversationState::new(session_id, 0);
        for event in events {
            let Some(revision) = event
                .payload
                .get("revision")
                .and_then(serde_json::Value::as_u64)
            else {
                continue;
            };
            match event.event_type {
                EventType::TopicUpdated => {
                    state.topic = event
                        .payload
                        .get("topic")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    state.revision = revision;
                    apply_event_metadata(&mut state, &event);
                }
                EventType::DepthUpdated => {
                    if let Some(depth) = event
                        .payload
                        .get("depth")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| u8::try_from(value).ok())
                    {
                        state.depth = depth;
                    }
                    state.revision = revision;
                    apply_event_metadata(&mut state, &event);
                }
                EventType::StatusUpdated => {
                    if let Some(set) = event.payload.get("set") {
                        let values: BTreeMap<String, StatusValue> =
                            serde_json::from_value(set.clone())?;
                        state.status.extend(values);
                    }
                    if let Some(unset) = event
                        .payload
                        .get("unset")
                        .and_then(serde_json::Value::as_array)
                    {
                        for key in unset.iter().filter_map(serde_json::Value::as_str) {
                            state.status.remove(key);
                        }
                    }
                    state.revision = revision;
                    apply_event_metadata(&mut state, &event);
                }
                _ => {}
            }
        }
        state.validate()?;
        Ok(state)
    }

    async fn write_state(&self, state: &ConversationState) -> Result<(), ConversationError> {
        let graph = rdf::conversation_graph(&state.session_id);
        let subject = session_iri(&state.session_id);
        rdf::upsert(
            &self.engine,
            &graph,
            &subject,
            &format!("{}ConversationState", rdf::MM_NS),
            state.topic.as_deref(),
            &serde_json::to_value(state)?,
        )
        .await
        .map_err(|error| ConversationError::Persistence(error.to_string()))?;
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
                &format!("{}depth", rdf::MM_NS),
                rdf::typed_lit(state.depth.to_string(), &format!("{}integer", rdf::XSD_NS)),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}updatedAt", rdf::MM_NS),
                rdf::typed_lit(
                    state.updated_at.to_string(),
                    &format!("{}dateTime", rdf::XSD_NS),
                ),
            ),
            RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}updatedBy", rdf::MM_NS),
                rdf::lit(state.updated_by.as_str()),
            ),
        ];
        if let Some(topic) = &state.topic {
            quads.push(RdfEngine::quad(
                &graph,
                &subject,
                &format!("{}topic", rdf::MM_NS),
                rdf::lit(topic.clone()),
            ));
        }
        self.engine
            .insert_all(quads)
            .await
            .map_err(|error| ConversationError::Persistence(error.to_string()))?;
        Ok(())
    }

    async fn append_changes(
        &self,
        current: Option<&ConversationState>,
        next: &ConversationState,
    ) -> Result<(), ConversationError> {
        let (old_topic, old_depth, old_status) = current.map_or((None, 0, None), |value| {
            (value.topic.clone(), value.depth, Some(&value.status))
        });
        if old_topic != next.topic {
            self.append_event(
                ConversationEvent::topic_updated(
                    &next.session_id,
                    next.revision,
                    next.topic.clone(),
                    next.updated_by,
                )
                .with_updated_at(next.updated_at),
            )
            .await?;
        }
        if old_depth != next.depth {
            self.append_event(
                ConversationEvent::depth_updated(
                    &next.session_id,
                    next.revision,
                    next.depth,
                    next.updated_by,
                )
                .with_updated_at(next.updated_at),
            )
            .await?;
        }
        let empty_status = BTreeMap::new();
        let old_status = old_status.unwrap_or(&empty_status);
        if old_status != &next.status {
            let mut set = BTreeMap::new();
            for (key, value) in &next.status {
                if old_status.get(key) != Some(value) {
                    set.insert(key.clone(), value.clone());
                }
            }
            let unset: Vec<String> = old_status
                .keys()
                .filter(|key| !next.status.contains_key(*key))
                .cloned()
                .collect();
            self.append_event(
                ConversationEvent::status_updated(
                    &next.session_id,
                    next.revision,
                    set,
                    unset,
                    next.updated_by,
                )
                .with_updated_at(next.updated_at),
            )
            .await?;
        }
        Ok(())
    }
}

fn apply_event_metadata(state: &mut ConversationState, event: &AgentstreamEvent) {
    if let Some(origin) = event
        .payload
        .get("origin")
        .and_then(serde_json::Value::as_str)
    {
        state.updated_by = match origin {
            "user" => StateOrigin::User,
            "system" => StateOrigin::System,
            _ => StateOrigin::Cm,
        };
    }
    state.updated_at = event
        .payload
        .get("updatedAt")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(event.timestamp);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ConversationState {
        ConversationState::new("01S", 1_000)
    }

    #[test]
    fn canonical_json_is_stable_and_sorted() {
        let mut state = state();
        state
            .set_status(
                "phase",
                StatusValue::Text("working".into()),
                StateOrigin::Cm,
                1_001,
            )
            .unwrap();
        state
            .set_status(
                "tasks",
                StatusValue::List(vec!["reproduce [doing]".into()]),
                StateOrigin::Cm,
                1_002,
            )
            .unwrap();
        let first = state.canonical_json().unwrap();
        let second = state.canonical_json().unwrap();
        assert_eq!(first, second);
        assert!(first.contains("\"phase\":\"working\""));
        assert!(first.contains("\"tasks\":[\"reproduce [doing]\"]"));
        assert_eq!(state.revision, 3);
    }

    #[test]
    fn topic_and_depth_are_independent_and_no_topic_is_seeded() {
        let mut state = state();
        assert!(state.topic.is_none());
        assert!(state.set_depth(2, StateOrigin::Cm, 1_001).unwrap());
        assert_eq!(state.topic, None);
        assert!(state
            .set_topic(
                Some("  auth refresh flake  ".into()),
                StateOrigin::Cm,
                1_002
            )
            .unwrap());
        assert_eq!(state.topic.as_deref(), Some("auth refresh flake"));
        assert_eq!(state.depth, 2);
        assert!(!state
            .set_topic(Some("auth refresh flake".into()), StateOrigin::Cm, 1_003)
            .unwrap());
    }

    #[test]
    fn reserved_types_are_not_coerced() {
        let mut state = state();
        let error = state
            .set_status(
                "frame_attempt",
                StatusValue::Text("1".into()),
                StateOrigin::Cm,
                1_001,
            )
            .unwrap_err();
        assert!(matches!(error, ConversationError::InvalidArguments(_)));
        let error = state
            .set_status(
                "phase",
                StatusValue::Text("busy".into()),
                StateOrigin::Cm,
                1_001,
            )
            .unwrap_err();
        assert!(matches!(error, ConversationError::InvalidArguments(_)));
    }

    #[test]
    fn status_caps_reject_rather_than_truncate() {
        let mut state = state();
        for index in 0..MAX_STATUS_KEYS {
            state
                .set_status(
                    format!("custom_{index}"),
                    StatusValue::Text("x".into()),
                    StateOrigin::Cm,
                    1_000 + index as i64,
                )
                .unwrap();
        }
        let error = state
            .set_status(
                "one_more",
                StatusValue::Text("x".into()),
                StateOrigin::Cm,
                1_100,
            )
            .unwrap_err();
        assert!(matches!(error, ConversationError::StateTooLarge { .. }));
        assert_eq!(state.status.len(), MAX_STATUS_KEYS);
    }

    #[test]
    fn long_text_is_rejected_not_coerced() {
        let mut state = state();
        let error = state
            .set_status(
                "notes",
                StatusValue::Text("x".repeat(MAX_TEXT_CHARS + 1)),
                StateOrigin::Cm,
                1_001,
            )
            .unwrap_err();
        assert!(matches!(error, ConversationError::InvalidArguments(_)));
    }

    #[test]
    fn status_ref_round_trips_as_an_object() {
        let value = StatusValue::Ref(StatusRef {
            kind: RefKind::File,
            id: "src/auth/refresh.rs".into(),
            label: None,
            verified: true,
        });
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"file","id":"src/auth/refresh.rs","label":null,"verified":true}"#
        );
        let parsed: StatusValue = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, value);
    }

    #[test]
    fn topic_depth_keys_are_rejected() {
        for key in ["topic", "depth"] {
            assert!(validate_status_key(key).is_err());
        }
    }
}
