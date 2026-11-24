//! Agentstream: the append-only session and event store (memory Tier 1).
//!
//! Every session gets a named graph; every event is one entity in that graph
//! and one N-Triples line in the daily event log. The graph is what queries
//! read; the log is the durable, ordered record a replay could follow.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::RdfEngine;
use crate::error::{DataError, Result};
use crate::rdf::{
    self, event_iri, new_id, now_ms, session_graph, session_iri, term_value,
    AGENTSTREAM_INDEX_GRAPH,
};

/// Session graph class.
pub const SESSION_CLASS: &str = "https://metamind.dev/agentstream#Session";
/// Event graph class.
pub const EVENT_CLASS: &str = "https://metamind.dev/agentstream#Event";

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    /// The user sent a message.
    UserMessage,
    /// The agent emitted a message or chunk.
    AgentMessage,
    /// A tool was invoked.
    ToolCall,
    /// A tool returned.
    ToolResult,
    /// A proactive item was surfaced.
    Proactive,
    /// The user interrupted the agent.
    BargeIn,
    /// The modality changed (voice ↔ text).
    ModalitySwitch,
    /// A session lifecycle event.
    Lifecycle,
    /// Conversation topic changed.
    TopicUpdated,
    /// Conversation depth changed.
    DepthUpdated,
    /// Conversation status variables changed.
    StatusUpdated,
    /// One downstream agent turn completed.
    AgentTurn,
    /// A long downstream turn was summarized.
    AgentTurnSummary,
    /// A downstream agent run changed status.
    AgentStatus,
    /// A durable session was opened.
    SessionOpened,
    /// An ended session was resumed into a new epoch.
    SessionResumed,
    /// A session ended normally.
    SessionEnded,
    /// A session was soft-deleted or purged.
    SessionDeleted,
    /// The lazy session directory was created.
    SessionDirectoryCreated,
    /// A trace manifest was registered.
    TraceRegistered,
    /// Presence changed.
    PresenceChanged,
    /// A work item changed.
    WorkItemChanged,
    /// A frame was pushed.
    FramePushed,
    /// A frame was replaced or updated.
    FrameUpdated,
    /// A frame was popped.
    FramePopped,
    /// One three-pass turn pass started.
    PassStarted,
    /// One three-pass turn pass completed.
    PassCompleted,
    /// The Interpret pass registered verification checks.
    VerificationRegistered,
    /// One verification check returned.
    VerificationResult,
    /// A failed verification scheduled another Process attempt.
    RetryScheduled,
    /// A write intent was staged by the Verify pass.
    WriteStaged,
    /// A staged write was committed after approval.
    WriteCommitted,
    /// A staged write set was discarded after rejection.
    WriteDiscarded,
    /// A logical model call is about to be sent.
    LlmRequest,
    /// One actual provider round inside a logical call.
    LlmAttempt,
    /// A model call returned a response.
    LlmResponse,
    /// A model call failed before producing a response.
    LlmError,
    /// A provider model probe completed.
    LlmModels,
    /// Anything else.
    Custom,
}

impl EventType {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserMessage => "user_message",
            Self::AgentMessage => "agent_message",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::Proactive => "proactive",
            Self::BargeIn => "barge_in",
            Self::ModalitySwitch => "modality_switch",
            Self::Lifecycle => "lifecycle",
            Self::TopicUpdated => "topic_updated",
            Self::DepthUpdated => "depth_updated",
            Self::StatusUpdated => "status_updated",
            Self::AgentTurn => "agent_turn",
            Self::AgentTurnSummary => "agent_turn_summary",
            Self::AgentStatus => "agent_status",
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
            Self::PassStarted => "pass_started",
            Self::PassCompleted => "pass_completed",
            Self::VerificationRegistered => "verification_registered",
            Self::VerificationResult => "verification_result",
            Self::RetryScheduled => "retry_scheduled",
            Self::WriteStaged => "write_staged",
            Self::WriteCommitted => "write_committed",
            Self::WriteDiscarded => "write_discarded",
            Self::LlmRequest => "llm_request",
            Self::LlmAttempt => "llm_attempt",
            Self::LlmResponse => "llm_response",
            Self::LlmError => "llm_error",
            Self::LlmModels => "llm_models",
            Self::Custom => "custom",
        }
    }
}

/// Who produced an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EventProvenance {
    /// `user`, `agent`, `tool`, `system`, or a plugin name.
    pub source: String,
    /// Model that produced it, when applicable.
    pub model: Option<String>,
}

/// One append-only event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentstreamEvent {
    /// Event identifier.
    pub event_id: String,
    /// Session identifier.
    pub session_id: String,
    /// Turn identifier, when the event belongs to a turn.
    pub turn_id: Option<String>,
    /// Unix milliseconds.
    pub timestamp: i64,
    /// Event kind.
    pub event_type: EventType,
    /// Event payload.
    pub payload: Value,
    /// Origin.
    pub provenance: EventProvenance,
}

impl AgentstreamEvent {
    /// A new event with a fresh id and the current timestamp.
    pub fn new(
        session_id: impl Into<String>,
        event_type: EventType,
        payload: Value,
        provenance: EventProvenance,
    ) -> Self {
        Self {
            event_id: new_id(),
            session_id: session_id.into(),
            turn_id: None,
            timestamp: now_ms(),
            event_type,
            payload,
            provenance,
        }
    }

    /// Attach a turn.
    #[must_use]
    pub fn with_turn(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }
}

/// Metadata supplied when a session starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionMetadata {
    /// Owning user.
    pub user_id: String,
    /// Active project, if any.
    pub project_id: Option<String>,
    /// Human-readable title.
    pub title: Option<String>,
}

/// The session record stored in the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Session identifier.
    pub session_id: String,
    /// Owning user.
    pub user_id: String,
    /// Active project.
    pub project_id: Option<String>,
    /// Title.
    pub title: Option<String>,
    /// Start time.
    pub started_at: i64,
    /// Last event time.
    pub last_event_at: i64,
    /// Number of logged events.
    pub event_count: u64,
}

/// A filter over sessions.
#[derive(Debug, Clone, Default)]
pub struct SessionFilter {
    /// Restrict to a user.
    pub user_id: Option<String>,
    /// Restrict to a project.
    pub project_id: Option<String>,
    /// Maximum rows.
    pub limit: Option<usize>,
}

/// The agentstream store.
#[derive(Debug, Clone)]
pub struct AgentstreamStore {
    engine: Arc<RdfEngine>,
    base_dir: PathBuf,
}

impl AgentstreamStore {
    /// Create the store directories beneath `base_dir`.
    pub fn new(engine: Arc<RdfEngine>, base_dir: &Path) -> Result<Self> {
        for dir in ["sessions", "events", "projections"] {
            std::fs::create_dir_all(base_dir.join("agentstream").join(dir))?;
        }
        Ok(Self {
            engine,
            base_dir: base_dir.to_path_buf(),
        })
    }

    /// The engine.
    pub fn engine(&self) -> &Arc<RdfEngine> {
        &self.engine
    }

    /// The agentstream directory.
    pub fn dir(&self) -> PathBuf {
        self.base_dir.join("agentstream")
    }

    /// Create a session graph and its index entry.
    pub async fn start_session(
        &self,
        session_id: &str,
        metadata: SessionMetadata,
    ) -> Result<SessionRecord> {
        let now = now_ms();
        let record = SessionRecord {
            session_id: session_id.to_owned(),
            user_id: metadata.user_id.clone(),
            project_id: metadata.project_id.clone(),
            title: metadata.title.clone(),
            started_at: now,
            last_event_at: now,
            event_count: 0,
        };
        let graph = session_graph(session_id);
        rdf::upsert(
            &self.engine,
            &graph,
            &session_iri(session_id),
            SESSION_CLASS,
            record.title.as_deref(),
            &serde_json::to_value(&record)?,
        )
        .await?;
        rdf::upsert(
            &self.engine,
            AGENTSTREAM_INDEX_GRAPH,
            &session_iri(session_id),
            SESSION_CLASS,
            record.title.as_deref(),
            &serde_json::to_value(&record)?,
        )
        .await?;
        self.snapshot_session(session_id).await?;
        Ok(record)
    }

    /// Append one event. Returns the stored event (id and timestamp included).
    pub async fn log_event(&self, event: AgentstreamEvent) -> Result<AgentstreamEvent> {
        let graph = session_graph(&event.session_id);
        let subject = event_iri(&event.event_id);
        rdf::upsert(
            &self.engine,
            &graph,
            &subject,
            EVENT_CLASS,
            Some(event.event_type.as_str()),
            &serde_json::to_value(&event)?,
        )
        .await?;

        // The append-only log: one N-Triples line per event, grouped by UTC day.
        let payload_pred = format!("{}payload", rdf::AS_NS);
        let line = format!(
            "<{subject}> <{payload_pred}> \"{}\" .\n",
            escape_ntriples(&serde_json::to_string(&event)?),
        );
        let path = self
            .dir()
            .join("events")
            .join(format!("events-{}.nt", utc_date(event.timestamp)));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(line.as_bytes())?;

        // Update the index's last-event time and count.
        if let Some(mut record) = rdf::load_as::<SessionRecord>(
            &self.engine,
            AGENTSTREAM_INDEX_GRAPH,
            &session_iri(&event.session_id),
        )
        .await?
        {
            record.last_event_at = event.timestamp;
            record.event_count = record.event_count.saturating_add(1);
            rdf::upsert(
                &self.engine,
                AGENTSTREAM_INDEX_GRAPH,
                &session_iri(&event.session_id),
                SESSION_CLASS,
                record.title.as_deref(),
                &serde_json::to_value(&record)?,
            )
            .await?;
        }
        Ok(event)
    }

    /// The events of one session, in timestamp order.
    pub async fn events(&self, session_id: &str) -> Result<Vec<AgentstreamEvent>> {
        let graph = session_graph(session_id);
        let mut events: Vec<AgentstreamEvent> =
            rdf::list_as(&self.engine, &graph, EVENT_CLASS).await?;
        events.retain(|event| event.session_id == session_id);
        events.sort_by_key(|event| (event.timestamp, event.event_id.clone()));
        Ok(events)
    }

    /// The session index, newest first.
    pub async fn sessions(&self, filter: SessionFilter) -> Result<Vec<SessionRecord>> {
        let mut sessions: Vec<SessionRecord> =
            rdf::list_as(&self.engine, AGENTSTREAM_INDEX_GRAPH, SESSION_CLASS).await?;
        sessions.retain(|record| {
            filter
                .user_id
                .as_ref()
                .is_none_or(|user| &record.user_id == user)
                && filter
                    .project_id
                    .as_ref()
                    .is_none_or(|project| record.project_id.as_ref() == Some(project))
        });
        sessions.sort_by_key(|record| std::cmp::Reverse(record.last_event_at));
        if let Some(limit) = filter.limit {
            sessions.truncate(limit);
        }
        Ok(sessions)
    }

    /// One session record from the index.
    pub async fn session(&self, session_id: &str) -> Result<Option<SessionRecord>> {
        rdf::load_as(
            &self.engine,
            AGENTSTREAM_INDEX_GRAPH,
            &session_iri(session_id),
        )
        .await
    }

    /// Replace the on-disk Turtle snapshot of one session.
    pub async fn snapshot_session(&self, session_id: &str) -> Result<PathBuf> {
        let graph = session_graph(session_id);
        let turtle = self.engine.dump_turtle(&graph).await?;
        let path = self
            .dir()
            .join("sessions")
            .join(format!("{session_id}.ttl"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, turtle)?;
        Ok(path)
    }

    /// Write the projection indexes (`conversations`, `turns`, `tool_calls`).
    pub async fn write_projections(&self) -> Result<()> {
        let dir = self.dir().join("projections");

        let conversations: Vec<String> = self
            .sessions(SessionFilter::default())
            .await?
            .into_iter()
            .map(|record| {
                format!(
                    "<{}> <{}title> \"{}\" .",
                    session_iri(&record.session_id),
                    rdf::AS_NS,
                    record
                        .title
                        .clone()
                        .unwrap_or_default()
                        .replace('"', "\\\"")
                )
            })
            .collect();
        std::fs::write(dir.join("conversations.ttl"), conversations.join("\n"))?;

        let sessions = self.sessions(SessionFilter::default()).await?;
        let mut turns = Vec::new();
        let mut tools = Vec::new();
        for session in sessions {
            for event in self.events(&session.session_id).await? {
                let line = format!(
                    "<{}> <{}type> \"{}\" .",
                    event_iri(&event.event_id),
                    rdf::AS_NS,
                    event.event_type.as_str()
                );
                match event.event_type {
                    EventType::ToolCall | EventType::ToolResult => tools.push(line),
                    EventType::UserMessage | EventType::AgentMessage => turns.push(line),
                    _ => {}
                }
            }
        }
        std::fs::write(dir.join("turns.ttl"), turns.join("\n"))?;
        std::fs::write(dir.join("tool_calls.ttl"), tools.join("\n"))?;
        Ok(())
    }

    /// Read the current session count.
    pub async fn session_count(&self) -> Result<usize> {
        Ok(self.sessions(SessionFilter::default()).await?.len())
    }

    /// Read the raw triples of a session graph (used by export and tests).
    pub async fn session_triples(&self, session_id: &str) -> Result<Vec<Value>> {
        let graph = session_graph(session_id);
        self.engine.triples(&graph).await
    }

    /// The lexical value of an event's `type` triple, for SPARQL-style checks.
    pub async fn event_types(&self, session_id: &str) -> Result<Vec<String>> {
        let graph = session_graph(session_id);
        let rows = self.engine.triples(&graph).await?;
        let mut types = Vec::new();
        for row in rows {
            if row.get("p").and_then(term_value) == Some(format!("{}type", rdf::AS_NS).as_str()) {
                if let Some(value) = row.get("o").and_then(term_value) {
                    types.push(value.to_owned());
                }
            }
        }
        types.sort();
        Ok(types)
    }

    /// Delete nothing: the log is append-only. Kept as an explicit no-op so a
    /// caller cannot mistake `end_session` for a data-purge API.
    pub async fn end_session(&self, session_id: &str) -> Result<PathBuf> {
        self.snapshot_session(session_id).await
    }
}

fn escape_ntriples(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// `YYYY-MM-DD` for a Unix-millisecond timestamp (UTC).
pub fn utc_date(timestamp_ms: i64) -> String {
    let days = timestamp_ms.div_euclid(86_400_000);
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// A convenience constructor for a user-message event.
pub fn user_message(session_id: &str, text: impl Into<String>) -> AgentstreamEvent {
    AgentstreamEvent::new(
        session_id,
        EventType::UserMessage,
        serde_json::json!({ "text": text.into() }),
        EventProvenance {
            source: "user".to_owned(),
            model: None,
        },
    )
}

/// A convenience constructor for an agent-message event.
pub fn agent_message(session_id: &str, text: impl Into<String>) -> AgentstreamEvent {
    AgentstreamEvent::new(
        session_id,
        EventType::AgentMessage,
        serde_json::json!({ "text": text.into(), "final": true }),
        EventProvenance {
            source: "agent".to_owned(),
            model: None,
        },
    )
}

/// The payload schema for every first-class LLM call event.
pub const LLM_CALL_SCHEMA: &str = "mm.llm.call.v1";

/// A logical call is about to be sent.
pub fn llm_request(
    session_id: &str,
    call_id: &str,
    payload: Value,
    model: Option<&str>,
) -> AgentstreamEvent {
    llm_event(
        session_id,
        EventType::LlmRequest,
        llm_payload(payload, "request", Some(call_id), None),
        model,
    )
}

/// One actual provider round inside a logical call.
pub fn llm_attempt(
    session_id: &str,
    call_id: &str,
    attempt_no: u32,
    payload: Value,
    model: Option<&str>,
) -> AgentstreamEvent {
    llm_event(
        session_id,
        EventType::LlmAttempt,
        llm_payload(payload, "attempt", Some(call_id), Some(attempt_no)),
        model,
    )
}

/// A logical call returned a response.
pub fn llm_response(
    session_id: &str,
    call_id: &str,
    payload: Value,
    model: Option<&str>,
) -> AgentstreamEvent {
    llm_event(
        session_id,
        EventType::LlmResponse,
        llm_payload(payload, "response", Some(call_id), None),
        model,
    )
}

/// A logical call failed before producing a response.
pub fn llm_error(
    session_id: &str,
    call_id: &str,
    payload: Value,
    model: Option<&str>,
) -> AgentstreamEvent {
    llm_event(
        session_id,
        EventType::LlmError,
        llm_payload(payload, "error", Some(call_id), None),
        model,
    )
}

/// A provider model probe completed.
pub fn llm_models(
    session_id: &str,
    provider: &str,
    models: &[String],
    reason: Option<&str>,
) -> AgentstreamEvent {
    llm_event(
        session_id,
        EventType::LlmModels,
        serde_json::json!({
            "schema": LLM_CALL_SCHEMA,
            "phase": "models",
            "provider": provider,
            "available": reason.is_none(),
            "models": models,
            "reason": reason,
        }),
        None,
    )
}

fn llm_event(
    session_id: &str,
    event_type: EventType,
    payload: Value,
    model: Option<&str>,
) -> AgentstreamEvent {
    AgentstreamEvent::new(
        session_id,
        event_type,
        payload,
        EventProvenance {
            source: "llm".to_owned(),
            model: model.map(str::to_owned),
        },
    )
}

fn llm_payload(
    payload: Value,
    phase: &str,
    call_id: Option<&str>,
    attempt_no: Option<u32>,
) -> Value {
    let mut object = match payload {
        Value::Object(object) => object,
        other => {
            let mut object = serde_json::Map::new();
            object.insert("value".to_owned(), other);
            object
        }
    };
    object.insert(
        "schema".to_owned(),
        Value::String(LLM_CALL_SCHEMA.to_owned()),
    );
    object.insert("phase".to_owned(), Value::String(phase.to_owned()));
    if let Some(call_id) = call_id {
        object.insert("callId".to_owned(), Value::String(call_id.to_owned()));
    }
    if let Some(attempt_no) = attempt_no {
        object.insert("attemptNo".to_owned(), Value::from(attempt_no));
    }
    Value::Object(object)
}

/// Who spoke in the clean log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CleanRole {
    /// The operator.
    User,
    /// The agent's approved final reply.
    Assistant,
}

/// One clean-log turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanTurn {
    /// Speaker.
    pub role: CleanRole,
    /// Message text.
    pub text: String,
    /// The turn that produced it, when known.
    pub turn_id: String,
}

/// User messages plus approved final replies, newest-last and bounded by size.
///
/// Internal pass text, tool results, verification reasoning, and rejected attempts
/// never enter this projection. Existing traces that predate the `final` marker
/// simply contribute no assistant turns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanLog {
    /// Turns in chronological order.
    pub turns: Vec<CleanTurn>,
    /// Whether older turns were dropped to fit the bound.
    pub truncated: bool,
}

/// The default clean-log character budget (roughly 4,000 tokens).
pub const CLEAN_LOG_MAX_CHARS: usize = 16_000;

impl CleanLog {
    /// Build from an agentstream trace.
    pub fn from_events(events: &[AgentstreamEvent], max_chars: usize) -> Self {
        let mut turns = Vec::new();
        for event in events {
            match event.event_type {
                EventType::UserMessage => {
                    if let Some(text) = event
                        .payload
                        .get("text")
                        .or_else(|| event.payload.get("query"))
                        .and_then(serde_json::Value::as_str)
                    {
                        push_clean(
                            &mut turns,
                            CleanRole::User,
                            text,
                            event.turn_id.as_deref().unwrap_or_default(),
                        );
                    }
                }
                EventType::AgentMessage => {
                    if event
                        .payload
                        .get("final")
                        .and_then(serde_json::Value::as_bool)
                        != Some(true)
                    {
                        continue;
                    }
                    if let Some(text) = event
                        .payload
                        .get("text")
                        .or_else(|| event.payload.get("reply"))
                        .or_else(|| event.payload.get("content"))
                        .and_then(serde_json::Value::as_str)
                    {
                        push_clean(
                            &mut turns,
                            CleanRole::Assistant,
                            text,
                            event.turn_id.as_deref().unwrap_or_default(),
                        );
                    }
                }
                _ => {}
            }
        }
        trim_clean(turns, max_chars)
    }

    /// Append one approved turn, enforcing the size bound.
    pub fn push(
        &mut self,
        role: CleanRole,
        text: impl Into<String>,
        turn_id: impl Into<String>,
        max_chars: usize,
    ) {
        let mut turns = std::mem::take(&mut self.turns);
        let text = text.into();
        let turn_id = turn_id.into();
        push_clean(&mut turns, role, &text, &turn_id);
        *self = trim_clean(turns, max_chars);
    }

    /// Whether any turn is present.
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// The projection as plain text, one turn per line.
    pub fn render(&self) -> String {
        self.turns
            .iter()
            .map(|turn| {
                let speaker = match turn.role {
                    CleanRole::User => "User",
                    CleanRole::Assistant => "Assistant",
                };
                format!("{speaker}: {}", turn.text)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn push_clean(turns: &mut Vec<CleanTurn>, role: CleanRole, text: &str, turn_id: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    turns.push(CleanTurn {
        role,
        text: text.to_owned(),
        turn_id: turn_id.to_owned(),
    });
}

fn trim_clean(mut turns: Vec<CleanTurn>, max_chars: usize) -> CleanLog {
    let mut total = 0usize;
    let mut keep_from = turns.len();
    for (index, turn) in turns.iter().enumerate().rev() {
        let size = turn.text.chars().count() + 16;
        if total + size > max_chars {
            break;
        }
        total += size;
        keep_from = index;
    }
    let truncated = keep_from > 0;
    if truncated {
        turns.drain(..keep_from);
    }
    CleanLog { turns, truncated }
}

/// Ensure a session exists, creating it when absent.
pub async fn ensure_session(
    store: &AgentstreamStore,
    session_id: &str,
    metadata: SessionMetadata,
) -> Result<SessionRecord> {
    if let Some(record) = store.session(session_id).await? {
        return Ok(record);
    }
    store.start_session(session_id, metadata).await
}

/// Assert a session exists, returning a `NotFound` otherwise.
pub async fn require_session(store: &AgentstreamStore, session_id: &str) -> Result<SessionRecord> {
    store
        .session(session_id)
        .await?
        .ok_or_else(|| DataError::NotFound {
            kind: "session",
            id: session_id.to_owned(),
        })
}

#[cfg(test)]
mod clean_log_tests {
    use super::*;
    use serde_json::json;

    fn event(kind: EventType, payload: serde_json::Value, turn: &str) -> AgentstreamEvent {
        AgentstreamEvent::new(
            "s1",
            kind,
            payload,
            EventProvenance {
                source: "test".into(),
                model: None,
            },
        )
        .with_turn(turn)
    }

    #[test]
    fn pass_event_names_are_stable() {
        for (kind, name) in [
            (EventType::PassStarted, "pass_started"),
            (EventType::PassCompleted, "pass_completed"),
            (EventType::VerificationRegistered, "verification_registered"),
            (EventType::VerificationResult, "verification_result"),
            (EventType::RetryScheduled, "retry_scheduled"),
            (EventType::WriteStaged, "write_staged"),
            (EventType::WriteCommitted, "write_committed"),
            (EventType::WriteDiscarded, "write_discarded"),
        ] {
            assert_eq!(kind.as_str(), name);
        }
    }

    #[test]
    fn llm_event_names_are_stable() {
        for (kind, name) in [
            (EventType::LlmRequest, "llm_request"),
            (EventType::LlmAttempt, "llm_attempt"),
            (EventType::LlmResponse, "llm_response"),
            (EventType::LlmError, "llm_error"),
            (EventType::LlmModels, "llm_models"),
        ] {
            assert_eq!(kind.as_str(), name);
        }
    }

    #[test]
    fn llm_constructors_set_the_schema_and_phase() {
        let request = llm_request(
            "s1",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            json!({"provider": "default"}),
            Some("small"),
        );
        assert_eq!(request.event_type, EventType::LlmRequest);
        assert_eq!(request.payload["schema"], LLM_CALL_SCHEMA);
        assert_eq!(request.payload["phase"], "request");
        assert_eq!(request.payload["provider"], "default");
        assert_eq!(request.provenance.source, "llm");
        assert_eq!(request.provenance.model.as_deref(), Some("small"));

        let attempt = llm_attempt("s1", "01ARZ3NDEKTSV4RRFFQ69G5FAV", 2, json!({}), None);
        assert_eq!(attempt.payload["phase"], "attempt");
        assert_eq!(attempt.payload["attemptNo"], 2);

        let models = llm_models("s1", "hermes", &["model-a".into()], None);
        assert_eq!(models.payload["phase"], "models");
        assert_eq!(models.payload["available"], true);
        assert_eq!(models.payload["models"], json!(["model-a"]));
    }

    #[test]
    fn an_old_event_still_deserializes_after_the_new_variants() {
        let old = serde_json::json!({
            "event_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "session_id": "s1",
            "turn_id": null,
            "timestamp": 1,
            "event_type": "user_message",
            "payload": {"text": "hello"},
            "provenance": {"source": "user", "model": null}
        });
        let event: AgentstreamEvent = serde_json::from_value(old).unwrap();
        assert_eq!(event.event_type, EventType::UserMessage);
        assert_eq!(event.payload["text"], "hello");
    }

    #[test]
    fn only_user_messages_and_final_replies_enter_the_log() {
        let events = vec![
            event(EventType::UserMessage, json!({"query": "hello"}), "t1"),
            event(EventType::PassStarted, json!({"pass": "interpret"}), "t1"),
            event(EventType::AgentMessage, json!({"text": "thinking"}), "t1"),
            event(
                EventType::AgentMessage,
                json!({"text": "hi there", "final": true}),
                "t1",
            ),
            event(EventType::ToolCall, json!({"name": "data.query"}), "t1"),
            event(
                EventType::VerificationResult,
                json!({"check": "x", "pass": false}),
                "t1",
            ),
        ];
        let log = CleanLog::from_events(&events, CLEAN_LOG_MAX_CHARS);
        assert_eq!(log.turns.len(), 2);
        assert_eq!(log.turns[0].role, CleanRole::User);
        assert_eq!(log.turns[0].text, "hello");
        assert_eq!(log.turns[1].role, CleanRole::Assistant);
        assert_eq!(log.turns[1].text, "hi there");
        assert!(!log.truncated);
        assert_eq!(log.render(), "User: hello\nAssistant: hi there");
    }

    #[test]
    fn the_bound_keeps_the_newest_turns_and_marks_truncation() {
        let events = vec![
            event(
                EventType::UserMessage,
                json!({"text": "a".repeat(40)}),
                "t1",
            ),
            event(
                EventType::AgentMessage,
                json!({"text": "b".repeat(40), "final": true}),
                "t1",
            ),
            event(
                EventType::UserMessage,
                json!({"text": "c".repeat(40)}),
                "t2",
            ),
            event(
                EventType::AgentMessage,
                json!({"text": "d".repeat(40), "final": true}),
                "t2",
            ),
        ];
        let log = CleanLog::from_events(&events, 100);
        assert!(log.truncated);
        assert!(log.turns.len() < 4);
        assert_eq!(log.turns.last().unwrap().text, "d".repeat(40));
    }

    #[test]
    fn push_keeps_the_bound() {
        let mut log = CleanLog::default();
        log.push(CleanRole::User, "a".repeat(80), "t1", 100);
        log.push(CleanRole::Assistant, "b".repeat(80), "t1", 100);
        assert!(log.truncated);
        assert_eq!(log.turns.len(), 1);
    }

    #[test]
    fn empty_messages_are_ignored() {
        let events = vec![event(EventType::UserMessage, json!({"text": "  "}), "t1")];
        assert!(CleanLog::from_events(&events, CLEAN_LOG_MAX_CHARS).is_empty());
    }
}
