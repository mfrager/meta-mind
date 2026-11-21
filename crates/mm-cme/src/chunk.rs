//! Chunk: the leaf state machine that emits one unit of a turn.
//!
//! A chunk is planned before the model runs and then driven by the stream:
//! `pending → streaming → (tool_executing ⇄ tool_waiting_confirm) → completed`.
//! A structured card can hold its output in `buffering` until the whole payload
//! is valid. Barge-in cancels chunks rather than failing them.

use mm_sm::{SmEvent, SmInstance, StateMachine, Transition, TransitionRecord};
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// A chunk identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChunkId(pub String);

impl ChunkId {
    /// Wrap a value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ChunkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a chunk emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkKind {
    /// Prose.
    Text,
    /// Structured data (a card, a table).
    Structured,
    /// A tool call and its result.
    ToolCall,
    /// Audio for voice mode.
    Audio,
}

impl ChunkKind {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Structured => "structured",
            Self::ToolCall => "tool_call",
            Self::Audio => "audio",
        }
    }
}

/// The part of the plan that describes one chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkDirective {
    /// Identifier allocated by the planner.
    pub chunk_id: ChunkId,
    /// What it emits.
    pub kind: ChunkKind,
    /// Short label (`answer`, `summary`, `tool:search`).
    pub label: String,
    /// Chunks that must complete first.
    #[serde(default)]
    pub depends_on: Vec<ChunkId>,
    /// Whether the whole payload must be buffered before delivery.
    #[serde(default)]
    pub buffered: bool,
    /// Detail level 1–5 the planner asked for.
    #[serde(default = "default_detail")]
    pub detail: u8,
    /// The tool this chunk runs, on a tool-call chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

fn default_detail() -> u8 {
    3
}

impl ChunkDirective {
    /// A text chunk with no dependencies.
    pub fn text(chunk_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            chunk_id: ChunkId::new(chunk_id),
            kind: ChunkKind::Text,
            label: label.into(),
            depends_on: Vec::new(),
            buffered: false,
            detail: 3,
            tool: None,
        }
    }

    /// A text chunk at a specific detail level.
    pub fn text_at(chunk_id: impl Into<String>, label: impl Into<String>, detail: u8) -> Self {
        let mut directive = Self::text(chunk_id, label);
        directive.detail = detail.clamp(1, 5);
        directive
    }

    /// A structured (card/table) chunk that buffers until its payload is whole.
    pub fn structured(chunk_id: impl Into<String>, label: impl Into<String>, detail: u8) -> Self {
        let mut directive = Self::text_at(chunk_id, label, detail);
        directive.kind = ChunkKind::Structured;
        directive.buffered = true;
        directive
    }

    /// A tool-call chunk.
    pub fn tool(
        chunk_id: impl Into<String>,
        label: impl Into<String>,
        tool: impl Into<String>,
    ) -> Self {
        let mut directive = Self::text(chunk_id, label);
        directive.kind = ChunkKind::ToolCall;
        directive.tool = Some(tool.into());
        directive
    }

    /// A voice audio chunk.
    pub fn audio(chunk_id: impl Into<String>, label: impl Into<String>) -> Self {
        let mut directive = Self::text(chunk_id, label);
        directive.kind = ChunkKind::Audio;
        directive.buffered = true;
        directive
    }
}

/// Chunk lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkState {
    /// Waiting for dependencies.
    Pending,
    /// Emitting tokens.
    Streaming,
    /// Holding output until it is complete.
    Buffering,
    /// A tool is running.
    ToolExecuting,
    /// A tool is waiting for the user to confirm.
    ToolWaitingConfirm,
    /// Done.
    Completed,
    /// Failed.
    Failed,
    /// Cancelled by barge-in or shutdown.
    Cancelled,
}

impl ChunkState {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Streaming => "streaming",
            Self::Buffering => "buffering",
            Self::ToolExecuting => "tool_executing",
            Self::ToolWaitingConfirm => "tool_waiting_confirm",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Events a chunk reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum ChunkEvent {
    /// Every dependency completed.
    DependencyMet,
    /// One streamed token.
    LlmToken(String),
    /// The model finished the chunk.
    LlmComplete,
    /// The chunk needs a tool.
    ToolCall,
    /// The tool returned.
    ToolResult,
    /// The tool needs confirmation.
    ConfirmationRequired,
    /// The user answered the confirmation gate.
    ConfirmationAnswer,
    /// A buffered payload is complete.
    FlushBuffer,
    /// Barge-in or cancellation.
    Cancel,
    /// Failure.
    Error(String),
}

impl ChunkEvent {
    /// The wire name dispatched to `mm-sm`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::DependencyMet => "dependency_met",
            Self::LlmToken(_) => "llm_token",
            Self::LlmComplete => "llm_complete",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::ConfirmationRequired => "confirmation_required",
            Self::ConfirmationAnswer => "confirmation_answer",
            Self::FlushBuffer => "flush_buffer",
            Self::Cancel => "cancel",
            Self::Error(_) => "error",
        }
    }

    fn into_sm(self) -> SmEvent {
        let name = self.name();
        match self {
            Self::LlmToken(token) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "token": token }))
            }
            Self::Error(detail) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "detail": detail }))
            }
            _ => SmEvent::new(name),
        }
    }
}

/// The chunk state-machine definition.
pub fn chunk_machine() -> StateMachine {
    chunk_machine_at(ChunkState::Pending)
}

/// The chunk state-machine definition with an explicit initial state. Structured
/// chunks start in `buffering` so nothing is emitted until the payload is whole.
pub fn chunk_machine_at(initial: ChunkState) -> StateMachine {
    StateMachine::new("chunk", initial.as_str())
        .state(ChunkState::Pending.as_str())
        .state(ChunkState::Streaming.as_str())
        .state(ChunkState::Buffering.as_str())
        .state(ChunkState::ToolExecuting.as_str())
        .state(ChunkState::ToolWaitingConfirm.as_str())
        .state(ChunkState::Completed.as_str())
        .state(ChunkState::Failed.as_str())
        .state(ChunkState::Cancelled.as_str())
        .transition(Transition::new(
            ChunkState::Pending.as_str(),
            "dependency_met",
            ChunkState::Streaming.as_str(),
        ))
        .transition(Transition::new(
            ChunkState::Streaming.as_str(),
            "tool_call",
            ChunkState::ToolExecuting.as_str(),
        ))
        .transition(Transition::new(
            ChunkState::ToolExecuting.as_str(),
            "confirmation_required",
            ChunkState::ToolWaitingConfirm.as_str(),
        ))
        .transition(Transition::new(
            ChunkState::ToolWaitingConfirm.as_str(),
            "confirmation_answer",
            ChunkState::ToolExecuting.as_str(),
        ))
        .transition(Transition::new(
            ChunkState::ToolExecuting.as_str(),
            "tool_result",
            ChunkState::Streaming.as_str(),
        ))
        .transition(Transition::new(
            ChunkState::Streaming.as_str(),
            "llm_complete",
            ChunkState::Completed.as_str(),
        ))
        .transition(Transition::new(
            ChunkState::Buffering.as_str(),
            "flush_buffer",
            ChunkState::Completed.as_str(),
        ))
        .transition(Transition::any("cancel", ChunkState::Cancelled.as_str()))
        .transition(Transition::any("error", ChunkState::Failed.as_str()))
}

/// A live chunk.
#[derive(Debug)]
pub struct Chunk {
    /// The state-machine instance.
    pub sm: SmInstance,
    /// The planned directive.
    pub directive: ChunkDirective,
    /// The identifier.
    pub chunk_id: ChunkId,
    /// The parent turn, when the chunk was spawned.
    pub parent_turn: Option<String>,
    /// Optional transition observer, inherited from the turn.
    pub observer: Option<crate::observability::ObserverHandle>,
}

impl Chunk {
    /// Spawn a chunk in `pending`.
    pub fn new(directive: ChunkDirective, parent_turn: Option<String>) -> Result<Self> {
        let chunk_id = directive.chunk_id.clone();
        let initial = if directive.buffered {
            ChunkState::Buffering
        } else {
            ChunkState::Pending
        };
        let sm = SmInstance::start_with_data(
            chunk_machine_at(initial),
            serde_json::json!({ "buffer": "", "tool_calls": [] }),
        )?;
        Ok(Self {
            sm,
            directive,
            chunk_id,
            parent_turn,
            observer: None,
        })
    }

    /// The current state.
    pub fn state(&self) -> &str {
        self.sm.state().as_str()
    }

    /// Whether the chunk reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self.state(), "completed" | "failed" | "cancelled")
    }

    /// The accumulated text.
    pub fn buffer(&self) -> &str {
        self.sm
            .data()
            .get("buffer")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    }

    /// Dispatch one event, appending tokens to the buffer.
    pub fn dispatch(&mut self, event: ChunkEvent) -> Result<Vec<TransitionRecord>> {
        if let ChunkEvent::LlmToken(token) = &event {
            let buffer = self
                .sm
                .data()
                .get("buffer")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned();
            self.sm.data_mut()["buffer"] = serde_json::Value::String(buffer + token);
        }
        let records = self.sm.dispatch(event.into_sm())?;
        if let Some(observer) = &self.observer {
            for record in &records {
                observer.on_transition(record);
            }
        }
        Ok(records)
    }

    /// Start a chunk whose dependencies are already met.
    pub fn start(&mut self) -> Result<Vec<TransitionRecord>> {
        self.dispatch(ChunkEvent::DependencyMet)
    }

    /// Cancel a live chunk.
    pub fn cancel(&mut self) -> Result<Vec<TransitionRecord>> {
        self.dispatch(ChunkEvent::Cancel)
    }

    /// Complete a buffered chunk with its whole payload.
    pub fn flush(&mut self, payload: &str) -> Result<Vec<TransitionRecord>> {
        self.sm.data_mut()["payload"] = serde_json::Value::String(payload.to_owned());
        self.dispatch(ChunkEvent::FlushBuffer)
    }

    /// The buffered payload, for structured and audio chunks.
    pub fn payload(&self) -> Option<&str> {
        self.sm
            .data()
            .get("payload")
            .and_then(serde_json::Value::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_transition_table_is_complete() {
        let cases = [
            (ChunkEvent::DependencyMet, ChunkState::Streaming),
            (ChunkEvent::ToolCall, ChunkState::ToolExecuting),
            (ChunkEvent::LlmComplete, ChunkState::Completed),
            (ChunkEvent::Cancel, ChunkState::Cancelled),
            (ChunkEvent::Error("boom".into()), ChunkState::Failed),
        ];
        for (event, expected) in cases {
            let mut chunk = Chunk::new(ChunkDirective::text("c1", "answer"), None).expect("chunk");
            if matches!(event, ChunkEvent::ToolCall | ChunkEvent::LlmComplete) {
                chunk.start().expect("start");
            }
            chunk.dispatch(event.clone()).expect("dispatch");
            assert_eq!(chunk.state(), expected.as_str(), "event {event:?}");
            assert!(
                chunk.is_terminal()
                    == matches!(
                        expected,
                        ChunkState::Completed | ChunkState::Failed | ChunkState::Cancelled
                    )
            );
        }
    }

    #[test]
    fn tokens_accumulate_in_the_buffer() {
        let mut chunk = Chunk::new(ChunkDirective::text("c1", "answer"), None).unwrap();
        chunk.start().unwrap();
        chunk.dispatch(ChunkEvent::LlmToken("Hel".into())).unwrap();
        chunk.dispatch(ChunkEvent::LlmToken("lo".into())).unwrap();
        assert_eq!(chunk.buffer(), "Hello");
        chunk.dispatch(ChunkEvent::LlmComplete).unwrap();
        assert!(chunk.is_terminal());
    }
}
