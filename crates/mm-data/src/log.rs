//! The per-session RDF operational log.
//!
//! This is a small append-only engine beside agentstream, not a second event
//! store. It exists for ad hoc operational notes and recent internal-action
//! outputs; raw conversation replay stays in agentstream.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::engine::RdfEngine;
use crate::rdf::{self, iri, lit, new_id, now_ms, MMD_NS};

/// The `log` graph alias.
pub const LOG_GRAPH: &str = "log";
/// `mm:LogEntry`.
pub const MM_LOG_ENTRY: &str = "https://metamind.dev/ontology#LogEntry";
/// Default per-entry message cap.
pub const MAX_LOG_MESSAGE: usize = 4_096;
/// Default per-session retention cap.
pub const MAX_LOG_ENTRIES: usize = 5_000;
/// Default retention window (30 days).
pub const LOG_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

/// Log severity, ordered from trace to error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Fine-grained trace.
    Trace,
    /// Debug.
    Debug,
    /// Informational.
    Info,
    /// Warning.
    Warn,
    /// Error.
    Error,
}

impl LogLevel {
    /// Parse the tool value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "trace" => Some(Self::Trace),
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// One operational log entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Entry id.
    pub id: String,
    /// Owning session.
    pub session_id: String,
    /// Optional trace.
    pub trace_id: Option<String>,
    /// Severity.
    pub level: LogLevel,
    /// Generic kind (for example an internal-action kind).
    pub kind: String,
    /// Message (≤ 4 KiB).
    pub message: String,
    /// Tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Session/entity/trace references.
    #[serde(default)]
    pub refs: Vec<String>,
    /// Creation instant.
    pub at: i64,
    /// `internal | cm | action | maintenance`.
    pub source: String,
    /// Per-session monotonic sequence.
    pub sequence: u64,
}

/// A read filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFilter {
    /// Optional trace.
    pub trace_id: Option<String>,
    /// Kinds to include.
    pub kinds: Vec<String>,
    /// Minimum level.
    pub level: Option<LogLevel>,
    /// Substring over message/kind.
    pub query: Option<String>,
    /// Inclusive lower bound.
    pub since: Option<i64>,
    /// Inclusive upper bound.
    pub until: Option<i64>,
}

impl LogFilter {
    /// Whether `entry` passes this filter.
    pub fn matches(&self, entry: &LogEntry) -> bool {
        if self
            .trace_id
            .as_ref()
            .is_some_and(|trace| entry.trace_id.as_ref() != Some(trace))
        {
            return false;
        }
        if !self.kinds.is_empty() && !self.kinds.iter().any(|kind| kind == &entry.kind) {
            return false;
        }
        if self.level.is_some_and(|level| entry.level < level) {
            return false;
        }
        if self
            .query
            .as_ref()
            .is_some_and(|needle| !entry.message.contains(needle) && !entry.kind.contains(needle))
        {
            return false;
        }
        if self.since.is_some_and(|since| entry.at < since) {
            return false;
        }
        if self.until.is_some_and(|until| entry.at > until) {
            return false;
        }
        true
    }
}

/// One page of entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogPage {
    /// Entries.
    pub entries: Vec<LogEntry>,
    /// Cursor for the next page.
    pub next_cursor: Option<String>,
    /// Whether another page exists.
    pub has_more: bool,
}

/// Aggregate counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogOverview {
    /// Entry count.
    pub entries: usize,
    /// Oldest instant.
    pub oldest: Option<i64>,
    /// Newest instant.
    pub latest: Option<i64>,
    /// Warning count.
    pub warnings: usize,
    /// Error count.
    pub errors: usize,
    /// Counts by source.
    pub sources: Vec<(String, usize)>,
    /// Oldest retained instant.
    pub retention_floor: i64,
}

/// The RDF log service.
#[derive(Debug, Clone)]
pub struct RdfLogService {
    engine: Arc<RdfEngine>,
}

impl RdfLogService {
    /// Build over the data engine.
    pub fn new(engine: Arc<RdfEngine>) -> Self {
        Self { engine }
    }

    /// Append one entry.
    #[allow(clippy::too_many_arguments)]
    pub async fn append(
        &self,
        session_id: &str,
        trace_id: Option<&str>,
        level: LogLevel,
        kind: &str,
        message: &str,
        tags: Vec<String>,
        refs: Vec<String>,
        source: &str,
    ) -> Result<LogEntry, LogError> {
        if session_id.trim().is_empty() {
            return Err(LogError::Invalid("log entry must name a session".into()));
        }
        if message.trim().is_empty() || message.chars().count() > MAX_LOG_MESSAGE {
            return Err(LogError::Invalid(format!(
                "log message must be 1–{MAX_LOG_MESSAGE} characters"
            )));
        }
        if crate::privacy::contains_secret(message) {
            return Err(LogError::Invalid(
                "log message looks like a secret and was refused".into(),
            ));
        }
        if tags.len() > 12 || tags.iter().any(|tag| tag.chars().count() > 64) {
            return Err(LogError::Invalid(
                "at most 12 tags of ≤ 64 characters".into(),
            ));
        }
        if refs.len() > 8 || refs.iter().any(|reference| reference.chars().count() > 512) {
            return Err(LogError::Invalid(
                "at most 8 refs of ≤ 512 characters".into(),
            ));
        }
        let existing = self.entries(session_id).await?;
        let sequence = existing
            .iter()
            .map(|entry| entry.sequence)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let entry = LogEntry {
            id: format!("{MMD_NS}log/{}", new_id()),
            session_id: session_id.to_owned(),
            trace_id: trace_id.map(str::to_owned),
            level,
            kind: kind.to_owned(),
            message: message.to_owned(),
            tags,
            refs,
            at: now_ms(),
            source: source.to_owned(),
            sequence,
        };
        self.write(&entry).await?;
        self.retain(session_id).await?;
        Ok(entry)
    }

    /// Every entry for one session ordered by sequence.
    pub async fn entries(&self, session_id: &str) -> Result<Vec<LogEntry>, LogError> {
        let mut entries: Vec<LogEntry> = rdf::list_as(&self.engine, LOG_GRAPH, MM_LOG_ENTRY)
            .await
            .map_err(store_error)?;
        entries.retain(|entry| entry.session_id == session_id);
        entries.sort_by_key(|entry| (entry.at, entry.sequence, entry.id.clone()));
        Ok(entries)
    }

    /// Counts and retention status.
    pub async fn overview(&self, session_id: &str) -> Result<LogOverview, LogError> {
        let entries = self.entries(session_id).await?;
        let mut sources: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &entries {
            *sources.entry(entry.source.clone()).or_default() += 1;
        }
        Ok(LogOverview {
            entries: entries.len(),
            oldest: entries.first().map(|entry| entry.at),
            latest: entries.last().map(|entry| entry.at),
            warnings: entries
                .iter()
                .filter(|entry| entry.level == LogLevel::Warn)
                .count(),
            errors: entries
                .iter()
                .filter(|entry| entry.level == LogLevel::Error)
                .count(),
            sources: sources.into_iter().collect(),
            retention_floor: now_ms() - LOG_RETENTION_MS,
        })
    }

    /// Newest matching entries, newest first.
    pub async fn latest(
        &self,
        session_id: &str,
        filter: &LogFilter,
        limit: usize,
    ) -> Result<Vec<LogEntry>, LogError> {
        let mut entries: Vec<LogEntry> = self
            .entries(session_id)
            .await?
            .into_iter()
            .filter(|entry| filter.matches(entry))
            .collect();
        entries.sort_by_key(|entry| {
            (
                std::cmp::Reverse(entry.at),
                std::cmp::Reverse(entry.sequence),
            )
        });
        entries.truncate(limit.min(100));
        Ok(entries)
    }

    /// One stable page in ascending order.
    pub async fn segment(
        &self,
        session_id: &str,
        filter: &LogFilter,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<LogPage, LogError> {
        let mut entries: Vec<LogEntry> = self
            .entries(session_id)
            .await?
            .into_iter()
            .filter(|entry| filter.matches(entry))
            .collect();
        entries.sort_by_key(|entry| (entry.at, entry.sequence, entry.id.clone()));
        let after = cursor.map(parse_cursor).transpose()?;
        if let Some((at, sequence, id)) = &after {
            entries.retain(|entry| {
                (entry.at, entry.sequence, entry.id.as_str()) > (*at, *sequence, id.as_str())
            });
        }
        let limit = limit.clamp(1, 100);
        let has_more = entries.len() > limit;
        entries.truncate(limit);
        let next_cursor = has_more
            .then(|| entries.last().map(encode_cursor))
            .flatten();
        Ok(LogPage {
            entries,
            next_cursor,
            has_more,
        })
    }

    /// Delete entries past the retention window or per-session cap.
    pub async fn retain(&self, session_id: &str) -> Result<usize, LogError> {
        let mut entries = self.entries(session_id).await?;
        let floor = now_ms() - LOG_RETENTION_MS;
        let excess = entries.len().saturating_sub(MAX_LOG_ENTRIES);
        let mut removed = 0usize;
        for (index, entry) in entries.drain(..).enumerate() {
            if entry.at < floor || index < excess {
                let graph = crate::graphs::resolve_graph(LOG_GRAPH);
                let deletes = self
                    .engine
                    .store()
                    .quads_for_subject(&graph, &entry.id)
                    .await
                    .map_err(store_error)?;
                self.engine
                    .store()
                    .handle()
                    .apply_batch(mm_store_graph::GraphBatch {
                        deletes,
                        inserts: Vec::new(),
                    })
                    .await
                    .map_err(store_error)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    async fn write(&self, entry: &LogEntry) -> Result<(), LogError> {
        let graph = crate::graphs::resolve_graph(LOG_GRAPH);
        let payload =
            serde_json::to_value(entry).map_err(|error| LogError::Invalid(error.to_string()))?;
        let mut payload = payload.as_object().cloned().unwrap_or_default();
        payload.insert("_rev".into(), serde_json::Value::from(rdf::next_revision()));
        let quads = vec![
            RdfEngine::quad(&graph, &entry.id, rdf::RDF_TYPE, iri(MM_LOG_ENTRY)),
            RdfEngine::quad(
                &graph,
                &entry.id,
                rdf::PAYLOAD,
                lit(serde_json::to_string(&payload).unwrap_or_default()),
            ),
            RdfEngine::quad(
                &graph,
                &entry.id,
                &format!("{}logLevel", rdf::MM_NS),
                lit(entry.level.as_str()),
            ),
            RdfEngine::quad(
                &graph,
                &entry.id,
                &format!("{}logKind", rdf::MM_NS),
                lit(entry.kind.clone()),
            ),
            RdfEngine::quad(
                &graph,
                &entry.id,
                &format!("{}sequence", rdf::MM_NS),
                lit(entry.sequence.to_string()),
            ),
        ];
        self.engine.insert_all(quads).await.map_err(store_error)
    }
}

fn encode_cursor(entry: &LogEntry) -> String {
    format!("{}:{}:{}", entry.at, entry.sequence, entry.id)
}

fn parse_cursor(cursor: &str) -> Result<(i64, u64, String), LogError> {
    let mut parts = cursor.splitn(3, ':');
    let at = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| LogError::Invalid("invalid log cursor".into()))?;
    let sequence = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| LogError::Invalid("invalid log cursor".into()))?;
    let id = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| LogError::Invalid("invalid log cursor".into()))?;
    Ok((at, sequence, id.to_owned()))
}

fn store_error(error: impl std::fmt::Display) -> LogError {
    LogError::Unavailable(error.to_string())
}

/// Log failures.
#[derive(Debug, Error)]
pub enum LogError {
    /// Invalid field.
    #[error("invalid log request: {0}")]
    Invalid(String),
    /// Store unavailable.
    #[error("log unavailable: {0}")]
    Unavailable(String),
}

/// Result alias.
pub type Result<T, E = LogError> = std::result::Result<T, E>;
