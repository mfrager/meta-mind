//! The append-only JSONL event store.
//!
//! One file per session, one JSON object per line, compact UTF-8. `seq` is
//! allocated in memory and continued from the last valid line, so a restart
//! never reuses a cursor. A partial final line is skipped on read and the next
//! append starts on a fresh line. A live broadcast publishes the same object
//! that was written.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::event::{self, Event, NewEvent};
use crate::projection::SessionIndexRecord;

/// Rotate a session file at this size.
pub const DEFAULT_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;

/// Event-store failures.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Filesystem failure.
    #[error("event store io: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization failure.
    #[error("event store json: {0}")]
    Json(#[from] serde_json::Error),
    /// A session does not exist.
    #[error("session not found: {0}")]
    NotFound(String),
}

/// Result alias.
pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// The JSONL event store.
pub struct EventStore {
    root: PathBuf,
    events_dir: PathBuf,
    segment_bytes: u64,
    sender: broadcast::Sender<Event>,
    seqs: Mutex<HashMap<String, u64>>,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl std::fmt::Debug for EventStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EventStore")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl EventStore {
    /// Open (or create) a store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let events_dir = root.join("agentstream");
        fs::create_dir_all(&events_dir)?;
        let (sender, _) = broadcast::channel(1024);
        Ok(Self {
            root,
            events_dir,
            segment_bytes: DEFAULT_SEGMENT_BYTES,
            sender,
            seqs: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
        })
    }

    /// Override the segment size (tests use a small value).
    pub fn with_segment_bytes(mut self, bytes: u64) -> Self {
        self.segment_bytes = bytes.max(1);
        self
    }

    /// The data root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory holding session files.
    pub fn events_dir(&self) -> &Path {
        &self.events_dir
    }

    /// Subscribe to the live event bus.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    /// Create a session and its index record when absent.
    pub fn ensure_session(
        &self,
        session_id: &str,
        user_id: &str,
        title: Option<String>,
    ) -> Result<()> {
        if self.session_exists(session_id)? {
            return Ok(());
        }
        self.append(event::session_started(session_id, user_id, title.clone()))?;
        let record = SessionIndexRecord {
            session_id: session_id.to_owned(),
            user_id: user_id.to_owned(),
            title,
            created_at: event::now_ms(),
            last_seq: 0,
            updated_at: event::now_ms(),
        };
        self.append_index(&record)
    }

    /// Append an event, assigning its per-session `seq`.
    ///
    /// `llm.delta` is published live but not persisted and does not consume a
    /// sequence number.
    pub fn append(&self, new_event: NewEvent) -> Result<Event> {
        if !new_event.event_type.is_persisted() {
            let event = new_event.into_event(0, event::now_ms());
            let _ = self.sender.send(event.clone());
            return Ok(event);
        }

        let session_id = new_event.session_id.clone();
        let lock = self.session_lock(&session_id);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());

        let seq = self.next_seq(&session_id)?;
        let event = new_event.into_event(seq, event::now_ms());
        let path = self.current_segment(&session_id)?;
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;

        // A crash can leave a partial last line. Never concatenate onto it.
        if file.metadata()?.len() > 0 {
            let bytes = fs::read(&path)?;
            if bytes.last() != Some(&b'\n') {
                file.write_all(b"\n")?;
            }
        }
        let line = serde_json::to_vec(&event)?;
        file.write_all(&line)?;
        file.write_all(b"\n")?;
        file.flush()?;

        self.seqs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(session_id, seq);
        let _ = self.sender.send(event.clone());
        Ok(event)
    }

    /// Read a session's events from `from` (inclusive), optionally bounded.
    pub fn events(&self, session_id: &str, from: u64, limit: Option<usize>) -> Result<Vec<Event>> {
        let mut events = self.read_all(session_id)?;
        events.retain(|event| event.seq >= from);
        if let Some(limit) = limit {
            events.truncate(limit);
        }
        Ok(events)
    }

    /// The last assigned sequence number.
    pub fn last_seq(&self, session_id: &str) -> Result<u64> {
        if let Some(seq) = self
            .seqs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(session_id)
            .copied()
        {
            return Ok(seq);
        }
        self.scan_last_seq(session_id)
    }

    /// Every session in the index, with its current last sequence.
    pub fn sessions(&self) -> Result<Vec<SessionIndexRecord>> {
        let path = self.events_dir.join("sessions.jsonl");
        let mut records: HashMap<String, SessionIndexRecord> = HashMap::new();
        if path.exists() {
            let text = fs::read_to_string(&path)?;
            for line in text.lines() {
                let Ok(record) = serde_json::from_str::<SessionIndexRecord>(line) else {
                    continue;
                };
                let entry = records
                    .entry(record.session_id.clone())
                    .or_insert_with(|| record.clone());
                if record.updated_at >= entry.updated_at {
                    *entry = record;
                }
            }
        }
        let mut out = Vec::with_capacity(records.len());
        for mut record in records.into_values() {
            record.last_seq = self.last_seq(&record.session_id)?;
            out.push(record);
        }
        out.sort_by_key(|record| std::cmp::Reverse(record.updated_at));
        Ok(out)
    }

    /// Append one index record. Index records are append-only too.
    pub fn append_index(&self, record: &SessionIndexRecord) -> Result<()> {
        let lock = self.index_lock();
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        let path = self.events_dir.join("sessions.jsonl");
        let line = serde_json::to_vec(record)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(&line)?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }

    /// Whether a session file exists.
    pub fn session_exists(&self, session_id: &str) -> Result<bool> {
        Ok(!self.segments(session_id)?.is_empty())
    }

    /// The first sequence still retained on disk for a session.
    ///
    /// A cursor below this value has fallen behind a rotated-away segment; the
    /// server sends a snapshot instead of pretending the missing range exists.
    pub fn retained_start(&self, session_id: &str) -> Result<u64> {
        Ok(self
            .read_all(session_id)?
            .first()
            .map(|event| event.seq)
            .unwrap_or(1))
    }

    fn next_seq(&self, session_id: &str) -> Result<u64> {
        if let Some(seq) = self
            .seqs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(session_id)
            .copied()
        {
            return Ok(seq.saturating_add(1));
        }
        let last = self.scan_last_seq(session_id)?;
        Ok(last.saturating_add(1))
    }

    fn scan_last_seq(&self, session_id: &str) -> Result<u64> {
        Ok(self
            .read_all(session_id)?
            .last()
            .map(|event| event.seq)
            .unwrap_or(0))
    }

    fn read_all(&self, session_id: &str) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        for (_, path) in self.segments(session_id)? {
            let bytes = fs::read(path)?;
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Event>(line) {
                    Ok(event) if event.session_id == session_id => events.push(event),
                    // A partial or corrupt trailing line is skipped: the next
                    // append starts on a fresh line.
                    _ => continue,
                }
            }
        }
        events.sort_by_key(|event| event.seq);
        events.dedup_by_key(|event| event.seq);
        Ok(events)
    }

    fn session_lock(&self, session_id: &str) -> Arc<Mutex<()>> {
        self.locks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(session_id.to_owned())
            .or_default()
            .clone()
    }

    fn index_lock(&self) -> Arc<Mutex<()>> {
        self.session_lock("__index__")
    }

    fn current_segment(&self, session_id: &str) -> Result<PathBuf> {
        let segments = self.segments(session_id)?;
        let Some((index, path)) = segments.last() else {
            return Ok(self.events_dir.join(format!("{session_id}.jsonl")));
        };
        let size = fs::metadata(path)?.len();
        if size >= self.segment_bytes {
            return Ok(self
                .events_dir
                .join(format!("{session_id}.{}.jsonl", index + 1)));
        }
        Ok(path.clone())
    }

    /// Segments in order: `<session>.jsonl` (0), then `<session>.1.jsonl`, …
    fn segments(&self, session_id: &str) -> Result<Vec<(u32, PathBuf)>> {
        if !self.events_dir.exists() {
            return Ok(Vec::new());
        }
        let mut segments = Vec::new();
        for entry in fs::read_dir(&self.events_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(rest) = name.strip_prefix(session_id) else {
                continue;
            };
            let Some(rest) = rest.strip_prefix('.') else {
                continue;
            };
            let index = if rest == "jsonl" {
                0
            } else {
                let Some(index) = rest.strip_suffix(".jsonl") else {
                    continue;
                };
                let Ok(index) = index.parse::<u32>() else {
                    continue;
                };
                index
            };
            segments.push((index, entry.path()));
        }
        segments.sort_by_key(|(index, _)| *index);
        Ok(segments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{user_message, EventType};

    #[test]
    fn seq_is_monotonic_and_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::new(dir.path()).unwrap();
        store.ensure_session("s", "u", Some("t".into())).unwrap();
        let first = store.append(user_message("s", "t1", "one")).unwrap();
        let second = store.append(user_message("s", "t2", "two")).unwrap();
        assert!(second.seq > first.seq);
        let events = store.events("s", 1, None).unwrap();
        assert_eq!(events.len(), 3, "session start plus two user messages");
        assert_eq!(events.last().unwrap().data["text"], "two");
        let reopened = EventStore::new(dir.path()).unwrap();
        let next = reopened.append(user_message("s", "t3", "three")).unwrap();
        assert_eq!(next.seq, second.seq + 1);
    }

    #[test]
    fn a_partial_final_line_is_skipped_and_repaired_on_append() {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::new(dir.path()).unwrap();
        store.ensure_session("s", "u", None).unwrap();
        store.append(user_message("s", "t1", "one")).unwrap();
        let path = store.events_dir().join("s.jsonl");
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"v\":2,\"seq\":99").unwrap();

        let events = store.events("s", 1, None).unwrap();
        assert_eq!(events.len(), 2, "the partial line is not an event");
        store.append(user_message("s", "t2", "two")).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.ends_with('\n'), "the append starts on a fresh line");
        let lines: Vec<_> = text.lines().collect();
        assert!(
            serde_json::from_str::<Event>(lines.last().unwrap()).is_ok(),
            "the appended event is a complete line"
        );
        assert_eq!(store.events("s", 1, None).unwrap().len(), 3);
    }

    #[test]
    fn segments_rotate_and_seq_continues() {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::new(dir.path()).unwrap().with_segment_bytes(1);
        store.ensure_session("s", "u", None).unwrap();
        for index in 0..3 {
            store
                .append(user_message("s", format!("t{index}"), format!("m{index}")))
                .unwrap();
        }
        assert!(store.events_dir().join("s.1.jsonl").exists());
        let events = store.events("s", 1, None).unwrap();
        assert_eq!(events.len(), 4);
    }

    #[test]
    fn deltas_are_live_only_and_do_not_consume_seq() {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::new(dir.path()).unwrap();
        store.ensure_session("s", "u", None).unwrap();
        let last = store.last_seq("s").unwrap();
        let mut receiver = store.subscribe();
        let delta = crate::event::llm_delta("s", Some("t".into()), "c", 0, "hi");
        store.append(delta).unwrap();
        let seen = receiver.try_recv().unwrap();
        assert_eq!(seen.event_type, EventType::LlmDelta);
        assert_eq!(store.last_seq("s").unwrap(), last);
    }
}
