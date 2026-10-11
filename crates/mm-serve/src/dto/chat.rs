//! Chat stream wire types (plan `09` §3.2).
//!
//! M0 ships the run-lifecycle frames — the ones the SSE codec demo and the
//! transcript's run-state machine need first. M5 extends this file with the
//! turn (`llm_*`), `tool_call`/`tool_result`, `block`, `input_request`/
//! `input_answer`, `focus`, `status`, `progress`, and `final_response` frames
//! exactly as specified in plan `09` §3.2. Changes are additive within
//! `wireVersion: 1`; nothing here is renamed once shipped.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The turn lifecycle phases (plan `08` §3.1). The UI draws one rail segment
/// per phase in these colors: plan=violet, act=indigo, verify=green,
/// repair=amber, final=azure (plan `02` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Intent parsing and the chosen operation sequence.
    Plan,
    /// Operations executing (tools, writes).
    Act,
    /// Proofs, verifications, gate results.
    Verify,
    /// Repair loops (structured-output repair, sandbox fixes).
    Repair,
    /// The final response to the user.
    Final,
}

/// The run-state machine the transcript reducer tracks (plan `08` §2.1):
/// `processing -> reconnecting -> completed | failed | detached`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChatRunState {
    /// Frames are arriving.
    Processing,
    /// The stream dropped; the client is resuming from its last sequence.
    Reconnecting,
    /// Terminal: the run finished cleanly.
    Completed,
    /// Terminal: the run failed; `reason` explains.
    Failed,
    /// The connection ended without a terminal event; the backend may still
    /// be running (plan `00` §1.2).
    Detached,
}

/// One numbered SSE frame of a chat run (plan `09` §3.2).
///
/// The `type` tag is the frame discriminator; `id: <seq>` on the wire is the
/// monotonic sequence the client dedups by and resumes from
/// (`GET /api/chat/{requestId}/stream?from=<seq>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatFrame {
    /// The run began.
    Started { query: String },
    /// The durable run is identified; reconnection uses these ids.
    Run {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        query: String,
    },
    /// The client lost the stream and is reconnecting (server- or client-emitted).
    Reconnecting { attempt: u32 },
    /// Terminal: the run completed.
    Completed {
        /// Which kernel subsystems the run touched.
        subsystems: Vec<String>,
    },
    /// Terminal: the run failed.
    Failed {
        error: String,
        /// The kernel's verbatim explanation, when one exists.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}
