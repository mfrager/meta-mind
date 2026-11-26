//! Errors raised by the turn pipeline.

/// A turn-pipeline failure.
#[derive(Debug, thiserror::Error)]
pub enum CmeError {
    /// Configuration could not be loaded or is invalid.
    #[error("config: {0}")]
    Config(String),
    /// A prompt strategy or fragment is missing or cannot render.
    #[error("prompt: {0}")]
    Prompt(#[from] mm_prompt::PromptError),
    /// The model substrate failed.
    #[error("llm: {0}")]
    Llm(#[from] mm_llm::LlmError),
    /// A tool call was rejected or failed to parse.
    #[error("tool: {0}")]
    Tool(String),
    /// Verification failed to run or its configuration was invalid.
    #[error("verification: {0}")]
    Verification(String),
    /// The model did not follow the pass protocol.
    #[error("protocol: {0}")]
    Protocol(String),
    /// Verify rejected every attempt.
    #[error("VERIFY_EXHAUSTED: the reply could not be verified")]
    VerifyExhausted {
        /// The reasons collected from the final attempt.
        reasons: Vec<String>,
    },
    /// The event store failed.
    #[error("event store: {0}")]
    Store(#[from] mm_agentstream::StoreError),
    /// A JSON payload could not be encoded.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl CmeError {
    /// A stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            CmeError::Config(_) => "CONFIG",
            CmeError::Prompt(_) => "PROMPT_UNAVAILABLE",
            CmeError::Llm(_) => "LLM",
            CmeError::Tool(_) => "TOOL",
            CmeError::Verification(_) => "VERIFICATION",
            CmeError::Protocol(_) => "PROTOCOL",
            CmeError::VerifyExhausted { .. } => "VERIFY_EXHAUSTED",
            CmeError::Store(_) => "EVENT_STORE",
            CmeError::Json(_) => "JSON",
        }
    }
}

impl From<crate::verification::VerificationError> for CmeError {
    fn from(error: crate::verification::VerificationError) -> Self {
        Self::Verification(error.to_string())
    }
}

/// Result alias.
pub type Result<T, E = CmeError> = std::result::Result<T, E>;
