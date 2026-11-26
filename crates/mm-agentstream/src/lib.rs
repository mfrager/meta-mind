//! `mm-agentstream` — the only durable turn log.
//!
//! One JSON object per line, one file per session, a per-session `seq`, a live
//! broadcast for SSE, rebuildable projections, and the LLM gateway that records
//! every model request with its complete messages and tool schemas.
#![forbid(unsafe_code)]

pub mod event;
pub mod gateway;
pub mod projection;
pub mod state;
pub mod store;

pub use event::{
    agent_message, internal_action, internal_error, llm_attempt, llm_delta, llm_error, llm_request,
    llm_response, pass_completed, pass_failed, pass_retry, pass_started, prompt_rendered,
    session_ended, session_started, state_updated, tool_call, tool_result, turn_completed,
    turn_failed, turn_started, user_message, verification_registered, verification_result, Event,
    EventType, NewEvent, SCHEMA_VERSION,
};
pub use gateway::LlmGateway;
pub use projection::{CleanLog, CleanRole, CleanTurn, SessionIndexRecord, TurnSummary};
pub use state::{Frame, FrameCommand, SessionState, StatePatch};
pub use store::{EventStore, StoreError};

/// The lowercase Crockford rendering of an identifier.
pub fn ulid_string(id: &ulid::Ulid) -> String {
    id.to_string().to_ascii_lowercase()
}
