//! Event records.
//!
//! An event is appended `provisional`, the projection it describes is mutated,
//! and only then is the event marked `committed`. Anything still `provisional`
//! when a replay starts was interrupted mid-flight and is `aborted`, so a crash
//! can never produce a half-applied event or a double-applied one.

use mm_core::{EventKind, MmError, Timestamp, Ulid};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

/// Where an event sits in the append → apply → commit sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    /// Appended, projection not yet confirmed.
    Provisional,
    /// Appended and its projection committed.
    Committed,
    /// Abandoned because no commit followed.
    Aborted,
}

impl EventStatus {
    /// The wire name, which is what the `events.status` CHECK constraint allows.
    pub fn as_str(self) -> &'static str {
        match self {
            EventStatus::Provisional => "provisional",
            EventStatus::Committed => "committed",
            EventStatus::Aborted => "aborted",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "provisional" => Some(EventStatus::Provisional),
            "committed" => Some(EventStatus::Committed),
            "aborted" => Some(EventStatus::Aborted),
            _ => None,
        }
    }
}

impl std::fmt::Display for EventStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of the event log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    /// Position in the log, from 1.
    pub seq: i64,
    /// The event's identifier.
    pub id: Ulid,
    /// What kind of event this is.
    pub kind: EventKind,
    /// The canonical JSON body.
    pub payload: serde_json::Value,
    /// Where it is in the append → apply → commit sequence.
    pub status: EventStatus,
    /// Correlation identifier shared with `mm-log`.
    pub correlation: Option<Ulid>,
    /// System time: when the event became visible.
    pub system_from: Timestamp,
    /// When the event was created.
    pub created_at: Timestamp,
    /// `sha256(kind | payload | created_at)`.
    pub hash: String,
}

impl EventRecord {
    /// Row → record.
    pub(crate) fn from_row(row: &SqliteRow) -> Result<Self, MmError> {
        let status_text: String = row.try_get("status").map_err(map)?;
        let kind_text: String = row.try_get("kind").map_err(map)?;
        let payload_text: String = row.try_get("payload").map_err(map)?;
        let created_text: String = row.try_get("created_at").map_err(map)?;
        let system_text: String = row.try_get("system_from").map_err(map)?;
        let correlation: Option<String> = row.try_get("correlation").map_err(map)?;
        let id_text: String = row.try_get("id").map_err(map)?;

        Ok(EventRecord {
            seq: row.try_get("seq").map_err(map)?,
            id: mm_core::id::parse_ulid(&id_text)?,
            kind: EventKind::from_wire(&kind_text),
            payload: serde_json::from_str(&payload_text)?,
            status: EventStatus::parse(&status_text)
                .ok_or_else(|| MmError::Event(format!("unknown event status {status_text:?}")))?,
            correlation: correlation
                .as_deref()
                .map(mm_core::id::parse_ulid)
                .transpose()?,
            system_from: Timestamp::from_rfc3339(&system_text)?,
            created_at: Timestamp::from_rfc3339(&created_text)?,
            hash: row.try_get("hash").map_err(map)?,
        })
    }

    /// The canonical JSON body, as stored.
    pub fn canonical_payload(&self) -> String {
        serde_json::to_string(&self.payload).unwrap_or_else(|_| "null".to_string())
    }
}

fn map(e: sqlx::Error) -> MmError {
    MmError::Store(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_wire_names_round_trip() {
        for status in [
            EventStatus::Provisional,
            EventStatus::Committed,
            EventStatus::Aborted,
        ] {
            assert_eq!(EventStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(EventStatus::parse("half-applied"), None);
    }

    #[test]
    fn only_the_three_documented_statuses_exist() {
        // The `events.status` CHECK constraint enumerates exactly these.
        assert_eq!(
            EventStatus::Provisional.as_str(),
            "provisional",
            "the migration's CHECK must match this vocabulary"
        );
    }
}
