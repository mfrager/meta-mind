//! The LLM error type.
//!
//! One error type for the whole substrate, because every failure has to be
//! *accountable*: a call that fails is still a call that happened, and the
//! `kind()` of the failure is what the `llm_calls.status`/`error_kind` columns
//! record. A stringly-typed error would leave an unaccountable gap in the ledger.

/// A failure in the LLM substrate.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// The provider endpoint or key is absent from the environment.
    ///
    /// There is deliberately no fallback endpoint: an unconfigured provider is a
    /// configuration fact, not something to paper over with a default host.
    #[error("llm provider is not configured: {0}")]
    ProviderNotConfigured(String),
    /// The request could not reach or be answered by the provider.
    #[error("llm transport error: {0}")]
    Transport(String),
    /// The provider did not answer inside the configured budget.
    #[error("llm call timed out after {0} ms")]
    Timeout(u32),
    /// The output did not satisfy the registered schema and repair did not fix it.
    #[error("llm schema rejected for `{schema_id}`: {detail}")]
    SchemaRejected {
        /// The schema the output had to satisfy.
        schema_id: String,
        /// Why it did not.
        detail: String,
    },
    /// A cache lookup was required to hit and did not.
    #[error("llm cache miss for {0}")]
    CacheMiss(String),
    /// A file operation failed.
    #[error("llm io error: {0}")]
    Io(String),
    /// A tabular store operation failed.
    #[error("llm database error: {0}")]
    Db(String),
    /// The configuration file or environment is invalid.
    #[error("llm config error: {0}")]
    Config(String),
    /// The cost budget for the call was exhausted before a model could be chosen.
    #[error("llm budget exhausted: {0}")]
    BudgetExhausted(String),
    /// A replay session was missing, malformed, or did not cover a request.
    #[error("llm replay error: {0}")]
    Replay(String),
    /// Something tried to open a socket while the network guard was armed.
    #[error("network egress is forbidden: {0}")]
    NetworkForbidden(String),
}

impl LlmError {
    /// A stable machine-readable kind, recorded in `llm_calls.error_kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            LlmError::ProviderNotConfigured(_) => "provider_not_configured",
            LlmError::Transport(_) => "transport",
            LlmError::Timeout(_) => "timeout",
            LlmError::SchemaRejected { .. } => "schema_rejected",
            LlmError::CacheMiss(_) => "cache_miss",
            LlmError::Io(_) => "io",
            LlmError::Db(_) => "db",
            LlmError::Config(_) => "config",
            LlmError::BudgetExhausted(_) => "budget_exhausted",
            LlmError::Replay(_) => "replay",
            LlmError::NetworkForbidden(_) => "network_forbidden",
        }
    }

    /// The `llm_calls.status` this error maps to.
    pub fn status(&self) -> crate::accounting::CallStatus {
        use crate::accounting::CallStatus;
        match self {
            LlmError::Timeout(_) => CallStatus::Timeout,
            LlmError::SchemaRejected { .. } => CallStatus::SchemaRejected,
            LlmError::ProviderNotConfigured(_)
            | LlmError::Transport(_)
            | LlmError::NetworkForbidden(_)
            | LlmError::BudgetExhausted(_) => CallStatus::ProviderError,
            _ => CallStatus::ProviderError,
        }
    }

    /// A dotted code for CLI output, mirroring `MmError::code`.
    pub fn code(&self) -> &'static str {
        match self {
            LlmError::ProviderNotConfigured(_) => "mm.llm.provider_unconfigured",
            LlmError::Transport(_) => "mm.llm.transport",
            LlmError::Timeout(_) => "mm.llm.timeout",
            LlmError::SchemaRejected { .. } => "mm.llm.schema_rejected",
            LlmError::CacheMiss(_) => "mm.llm.cache_miss",
            LlmError::Io(_) => "mm.llm.io",
            LlmError::Db(_) => "mm.llm.db",
            LlmError::Config(_) => "mm.llm.config",
            LlmError::BudgetExhausted(_) => "mm.llm.budget_exhausted",
            LlmError::Replay(_) => "mm.llm.replay",
            LlmError::NetworkForbidden(_) => "mm.llm.network_forbidden",
        }
    }
}

impl From<std::io::Error> for LlmError {
    fn from(e: std::io::Error) -> Self {
        LlmError::Io(e.to_string())
    }
}

impl From<mm_core::MmError> for LlmError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(m) => LlmError::Db(m),
            other => LlmError::Config(other.to_string()),
        }
    }
}

/// The substrate result alias.
pub type Result<T> = std::result::Result<T, LlmError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::CallStatus;

    #[test]
    fn every_error_maps_to_a_distinct_kind_and_a_status() {
        let errors = [
            LlmError::ProviderNotConfigured("x".into()),
            LlmError::Transport("x".into()),
            LlmError::Timeout(10),
            LlmError::SchemaRejected {
                schema_id: "s".into(),
                detail: "d".into(),
            },
            LlmError::CacheMiss("h".into()),
            LlmError::Io("x".into()),
            LlmError::Db("x".into()),
            LlmError::Config("x".into()),
            LlmError::BudgetExhausted("x".into()),
            LlmError::Replay("x".into()),
            LlmError::NetworkForbidden("x".into()),
        ];
        let mut kinds: Vec<&str> = errors.iter().map(LlmError::kind).collect();
        kinds.sort_unstable();
        let unique = {
            let mut k = kinds.clone();
            k.dedup();
            k
        };
        assert_eq!(unique.len(), kinds.len(), "error kinds must be distinct");
        assert_eq!(
            LlmError::Timeout(1).status(),
            CallStatus::Timeout,
            "a timeout is recorded as a timeout, not a generic provider error"
        );
        assert_eq!(
            LlmError::SchemaRejected {
                schema_id: "s".into(),
                detail: "d".into()
            }
            .status(),
            CallStatus::SchemaRejected
        );
    }

    #[test]
    fn an_unconfigured_provider_is_a_configuration_fact() {
        let e = LlmError::ProviderNotConfigured("OPENAI_BASE_URL is unset".into());
        assert_eq!(e.kind(), "provider_not_configured");
        assert_eq!(e.status(), CallStatus::ProviderError);
        // The message names what is missing, so the fix is obvious.
        assert!(e.to_string().contains("OPENAI_BASE_URL"));
    }
}
