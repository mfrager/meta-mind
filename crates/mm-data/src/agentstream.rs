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
        events.sort_by_key(|event| event.timestamp);
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
        serde_json::json!({ "text": text.into() }),
        EventProvenance {
            source: "agent".to_owned(),
            model: None,
        },
    )
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
