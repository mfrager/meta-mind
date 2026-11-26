//! Agentstream wire types.
//!
//! These are `serde` shapes only. `Event` itself is the persisted envelope and is
//! served unchanged.

use mm_agentstream::{Event, SessionIndexRecord};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A request to start a turn.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "agentstream.ts")]
pub struct StartTurnRequest {
    /// Existing session, or a new one when absent.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The operator's text.
    pub text: String,
    /// Optional user id for a new session.
    #[serde(default)]
    pub user_id: Option<String>,
}

/// The 202 response to `POST /api/turns`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agentstream.ts")]
pub struct TurnAccepted {
    /// The session the turn belongs to.
    pub session_id: String,
    /// The new turn.
    pub turn_id: String,
    /// The next event sequence the client should read.
    pub seq: u64,
}

/// A page of persisted events.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agentstream.ts")]
pub struct EventPage {
    /// Owning session.
    pub session_id: String,
    /// The requested cursor.
    pub from: u64,
    /// The cursor for the next page.
    pub next: u64,
    /// Events, in `seq` order.
    pub events: Vec<Event>,
}

/// The session list.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agentstream.ts")]
pub struct SessionList {
    /// Sessions, newest activity first.
    pub sessions: Vec<SessionIndexRecord>,
}

/// Health.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agentstream.ts")]
pub struct HealthEnvelope {
    /// Always `ok` when the process is serving.
    pub status: String,
    /// Binary version.
    pub version: String,
    /// The data root in use.
    pub data_dir: String,
    /// Number of sessions in the log.
    pub sessions: usize,
    /// Turns started since boot.
    pub turns_started: u64,
    /// Turns completed since boot.
    pub turns_completed: u64,
    /// Turns failed since boot.
    pub turns_failed: u64,
}
