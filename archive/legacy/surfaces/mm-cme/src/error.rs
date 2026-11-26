//! Errors raised by the conversation engine.

/// A conversation-engine failure.
#[derive(Debug, thiserror::Error)]
pub enum CmeError {
    /// A state-machine definition or dispatch failed.
    #[error("state machine: {0}")]
    Sm(#[from] mm_sm::SmError),
    /// A driver invariant was violated.
    #[error("driver: {0}")]
    Driver(String),
    /// A turn was started while another turn is active.
    #[error("session {session} already has an active turn")]
    TurnInProgress {
        /// Session identifier.
        session: String,
    },
    /// The session is not accepting turns.
    #[error("session {session} is in state {state} and cannot start a turn")]
    NotActive {
        /// Session identifier.
        session: String,
        /// Current state.
        state: String,
    },
    /// A chunk or plan referenced an unknown identifier.
    #[error("{kind} not found: {id}")]
    NotFound {
        /// Record kind.
        kind: &'static str,
        /// Identifier.
        id: String,
    },
    /// A driver channel closed.
    #[error("driver channel closed")]
    ChannelClosed,
    /// The LLM substrate failed.
    #[error("llm: {0}")]
    Llm(#[from] mm_llm::LlmError),
    /// A voice pipeline failed.
    #[error("voice: {0}")]
    Voice(String),
    /// A tool call failed.
    #[error("tool: {0}")]
    Tool(String),
    /// A plugin failed to load, evaluate or execute.
    #[error("plugin: {0}")]
    Plugin(String),
    /// The data layer (memory, continuity, sync) failed.
    #[error("data: {0}")]
    Data(#[from] mm_data::DataError),
    /// A routing table could not be loaded or was invalid.
    #[error("routing table: {0}")]
    RoutingTable(String),
    /// A pass exhausted its budget.
    #[error(transparent)]
    Budget(#[from] crate::budget::BudgetError),
    /// A verification backend or check failed.
    #[error("verification: {0}")]
    Verification(String),
}

impl From<crate::verification::VerificationError> for CmeError {
    fn from(error: crate::verification::VerificationError) -> Self {
        Self::Verification(error.to_string())
    }
}

/// A result carrying [`CmeError`].
pub type Result<T> = std::result::Result<T, CmeError>;
