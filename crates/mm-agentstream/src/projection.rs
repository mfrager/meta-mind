//! Projections rebuilt from the event log.
//!
//! The log is authoritative. Every projection here is a pure fold over events
//! and can be thrown away and rebuilt at any time.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::event::{Event, EventType};
use crate::state::SessionState;

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
    /// The turn that produced it.
    pub turn_id: String,
}

/// User messages plus approved final replies, newest last and bounded by size.
///
/// Internal pass text, tool results, verification reasoning, and rejected
/// attempts never enter this projection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanLog {
    /// Turns in chronological order.
    pub turns: Vec<CleanTurn>,
    /// Whether older turns were dropped to fit the bound.
    pub truncated: bool,
}

/// The default clean-log character budget.
pub const CLEAN_LOG_MAX_CHARS: usize = 16_000;

impl CleanLog {
    /// Build from a session's events.
    pub fn from_events(events: &[Event], max_chars: usize) -> Self {
        let mut turns = Vec::new();
        for event in events {
            match event.event_type {
                EventType::UserMessage => {
                    if let Some(text) = event.data.get("text").and_then(Value::as_str) {
                        push_clean(
                            &mut turns,
                            CleanRole::User,
                            text,
                            event.turn_id.as_deref().unwrap_or_default(),
                        );
                    }
                }
                EventType::AgentMessage => {
                    if event.data.get("final").and_then(Value::as_bool) != Some(true) {
                        continue;
                    }
                    if let Some(text) = event.data.get("text").and_then(Value::as_str) {
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

    /// Append one turn, enforcing the size bound.
    pub fn push(&mut self, role: CleanRole, text: impl Into<String>, turn_id: impl Into<String>) {
        let mut turns = std::mem::take(&mut self.turns);
        push_clean(&mut turns, role, &text.into(), &turn_id.into());
        *self = trim_clean(turns, CLEAN_LOG_MAX_CHARS);
    }

    /// Whether any turn is present.
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// The projection as one line per turn.
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

    /// The bounded turns as LLM messages.
    pub fn messages(&self) -> Vec<mm_llm::Message> {
        self.turns
            .iter()
            .map(|turn| match turn.role {
                CleanRole::User => mm_llm::Message::user(turn.text.clone()),
                CleanRole::Assistant => mm_llm::Message::assistant(turn.text.clone()),
            })
            .collect()
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

/// Fold `session.started` and `state.updated` into the current state.
pub fn session_state(events: &[Event]) -> SessionState {
    let mut state = SessionState::started();
    for event in events {
        if event.event_type == EventType::StateUpdated {
            if let Some(after) = event.data.get("after") {
                if let Ok(next) = serde_json::from_value::<SessionState>(after.clone()) {
                    state = next;
                }
            }
        }
    }
    state
}

/// A compact summary of one turn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSummary {
    /// Turn identifier.
    pub turn_id: String,
    /// Whether the turn completed.
    pub completed: bool,
    /// Whether it failed.
    pub failed: bool,
    /// The approved reply, when there is one.
    pub reply: Option<String>,
    /// The model that answered.
    pub model: Option<String>,
    /// Process attempts.
    pub attempts: u32,
    /// Number of passes seen.
    pub passes: u32,
    /// Number of model calls.
    pub calls: u32,
    /// Number of verification results.
    pub checks: u32,
}

impl TurnSummary {
    /// Fold one turn's events.
    pub fn from_events(turn_id: &str, events: &[Event]) -> Self {
        let mut summary = TurnSummary {
            turn_id: turn_id.to_owned(),
            ..TurnSummary::default()
        };
        for event in events
            .iter()
            .filter(|event| event.turn_id.as_deref() == Some(turn_id))
        {
            match event.event_type {
                EventType::TurnCompleted => {
                    summary.completed = true;
                    summary.reply = event
                        .data
                        .get("reply")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    summary.model = event
                        .data
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    summary.attempts = event
                        .data
                        .get("attempts")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                }
                EventType::TurnFailed => summary.failed = true,
                EventType::PassStarted => summary.passes += 1,
                EventType::LlmRequest => summary.calls += 1,
                EventType::VerificationResult => summary.checks += 1,
                _ => {}
            }
        }
        summary
    }
}

/// One record in the cross-session index.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agentstream.ts")]
pub struct SessionIndexRecord {
    /// Session identifier.
    pub session_id: String,
    /// Owning user.
    pub user_id: String,
    /// Optional title.
    pub title: Option<String>,
    /// Creation time.
    pub created_at: u64,
    /// Last event sequence observed.
    pub last_seq: u64,
    /// Last activity time.
    pub updated_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{agent_message, user_message};
    use crate::state::StatePatch;

    fn events() -> Vec<Event> {
        let mut state = SessionState::default();
        StatePatch {
            topic: Some(Some("incident".into())),
            depth: Some(1),
            ..StatePatch::default()
        }
        .apply(&mut state);
        vec![
            user_message("s", "t1", "hello").into_event(1, 10),
            agent_message("s", "t1", "hi there", true).into_event(2, 11),
            crate::event::state_updated(
                "s",
                "t1",
                state.revision,
                serde_json::json!({}),
                serde_json::json!(state),
            )
            .into_event(3, 12),
            user_message("s", "t2", "again").into_event(4, 13),
        ]
    }

    #[test]
    fn the_clean_log_keeps_only_user_and_final_assistant_turns() {
        let log = CleanLog::from_events(&events(), CLEAN_LOG_MAX_CHARS);
        assert_eq!(log.turns.len(), 3);
        assert_eq!(log.turns[0].role, CleanRole::User);
        assert_eq!(log.turns[1].role, CleanRole::Assistant);
        assert_eq!(
            log.render(),
            "User: hello\nAssistant: hi there\nUser: again"
        );
    }

    #[test]
    fn state_is_folded_from_state_updated() {
        let state = session_state(&events());
        assert_eq!(state.topic.as_deref(), Some("incident"));
        assert_eq!(state.revision, 1);
    }

    #[test]
    fn a_turn_summary_counts_passes_and_calls() {
        let summary = TurnSummary::from_events("t1", &events());
        assert!(!summary.completed);
        assert_eq!(summary.passes, 0);
    }
}
