//! The agentstream-backed LLM gateway.
//!
//! This crate is the composition boundary between the provider substrate and the
//! memory layer. P0 establishes the public contract only: application crates will
//! depend on [`AgentstreamLlm`] instead of constructing `OpenAiClient` or
//! `LlmService` directly.
//!
//! The gateway itself is deliberately not implemented here yet. The types below
//! are the stable seam that P1 fills with journaling, provider dispatch, and the
//! live event bus.

#![forbid(unsafe_code)]

use std::sync::Arc;

use mm_data::AgentstreamStore;
use mm_log::Logger;
use mm_store_graph::GraphHandle;
use mm_store_sqlite::SqliteStore;

pub use mm_llm::{
    CacheMode, CallContext, DecoderKind, LlmClient, LlmConfig, LlmError, LlmRequest, LlmResponse,
    Message, ProviderToolCall, Purpose, Result, Role, SchemaId, StructuredOut, ToolSchema, Usage,
};

use mm_llm::StructuredOutput;
use ulid::Ulid;

/// Whether a failed journal write may be ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JournalMode {
    /// A call is never sent without its request entry. Production uses this.
    #[default]
    Required,
    /// Journal failures are reported but do not block the call. Tests only.
    BestEffort,
}

/// The provider protocol a profile speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProviderKind {
    /// An OpenAI-compatible `/chat/completions` endpoint.
    #[default]
    OpenAiCompatible,
}

/// One named provider endpoint and its transport policy.
///
/// Endpoint URLs and keys are resolved from the named environment variables;
/// they are never embedded in configuration or a profile value.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderProfile {
    /// Stable profile name used on [`LlmRequest::provider`].
    pub name: String,
    /// Protocol spoken by the endpoint.
    pub kind: ProviderKind,
    /// Environment variable holding the base URL.
    pub endpoint_env: String,
    /// Environment variable holding the bearer token.
    pub api_key_env: String,
    /// Optional environment variable holding the model name.
    pub model_env: Option<String>,
    /// Optional configured model, used when the environment is silent.
    pub model: Option<String>,
    /// Per-call wall-clock ceiling.
    pub timeout_ms: u64,
    /// Whether the profile prefers SSE.
    pub stream: bool,
    /// Optional fixed sampling temperature.
    pub temperature: Option<f32>,
    /// Hermes historically omits `max_tokens`; profiles preserve that.
    pub omit_max_tokens: bool,
    /// How structured calls ask for JSON.
    pub structured_output: StructuredOutput,
}

/// A validated collection of provider profiles.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderProfiles(Vec<ProviderProfile>);

impl ProviderProfiles {
    /// Build a profile set. Call [`Self::validate`] before constructing a gateway.
    pub fn new(profiles: Vec<ProviderProfile>) -> Self {
        Self(profiles)
    }

    /// The profiles in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &ProviderProfile> {
        self.0.iter()
    }

    /// The number of profiles.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no profiles are configured.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Find a profile by name.
    pub fn get(&self, name: &str) -> Option<&ProviderProfile> {
        self.0.iter().find(|profile| profile.name == name)
    }

    /// Consume the set.
    pub fn into_vec(self) -> Vec<ProviderProfile> {
        self.0
    }

    /// Reject empty and duplicate names before a call can select one.
    pub fn validate(&self) -> Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        for profile in &self.0 {
            if profile.name.trim().is_empty() {
                return Err(LlmError::Config(
                    "a provider profile must have a non-empty name".into(),
                ));
            }
            if !seen.insert(profile.name.as_str()) {
                return Err(LlmError::Config(format!(
                    "provider profile `{}` is declared more than once",
                    profile.name
                )));
            }
        }
        Ok(())
    }
}

/// Everything needed to construct a gateway.
///
/// The injected client is for scripted and recorded-replay runs. It still goes
/// through the gateway, so test calls are journaled and published like live ones.
pub struct GatewayParts {
    /// The resolved LLM configuration.
    pub config: LlmConfig,
    /// The agentstream journal.
    pub journal: Arc<AgentstreamStore>,
    /// The accounting ledger store.
    pub store: SqliteStore,
    /// The provenance graph handle.
    pub graph: GraphHandle,
    /// The audit logger.
    pub logger: Arc<Logger>,
    /// Provider profiles, including `default` and `hermes`.
    pub profiles: ProviderProfiles,
    /// An optional scripted/replay client.
    pub injected_client: Option<Arc<dyn LlmClient>>,
    /// Whether a journal failure blocks the call.
    pub journal_mode: JournalMode,
    /// User id used when the gateway must create a system session.
    pub user_id: String,
}

impl GatewayParts {
    /// Assemble the required parts with production defaults.
    pub fn new(
        config: LlmConfig,
        journal: Arc<AgentstreamStore>,
        store: SqliteStore,
        graph: GraphHandle,
        logger: Arc<Logger>,
        profiles: ProviderProfiles,
    ) -> Self {
        Self {
            config,
            journal,
            store,
            graph,
            logger,
            profiles,
            injected_client: None,
            journal_mode: JournalMode::Required,
            user_id: "system".to_owned(),
        }
    }

    /// Inject a scripted or replay client.
    #[must_use]
    pub fn with_injected_client(mut self, client: Arc<dyn LlmClient>) -> Self {
        self.injected_client = Some(client);
        self
    }

    /// Override the journal mode. Production must keep [`JournalMode::Required`].
    #[must_use]
    pub fn with_journal_mode(mut self, mode: JournalMode) -> Self {
        self.journal_mode = mode;
        self
    }

    /// Override the user id used for system-created sessions.
    #[must_use]
    pub fn with_user(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = user_id.into();
        self
    }
}

/// The builder for [`AgentstreamLlm`].
pub struct GatewayBuilder {
    parts: GatewayParts,
}

impl GatewayBuilder {
    /// Start from assembled parts.
    pub fn new(parts: GatewayParts) -> Self {
        Self { parts }
    }

    /// Validate the contract and construct the gateway.
    ///
    /// P0 performs only profile validation; P1 adds the service, journal, and
    /// provider pool.
    pub fn build(self) -> Result<AgentstreamLlm> {
        self.parts.profiles.validate()?;
        Ok(AgentstreamLlm { parts: self.parts })
    }
}

/// The only public LLM client interface application code may use.
///
/// P0 holds the resolved parts and exposes profile inspection. P1 adds
/// `LlmClient`, schema registration, journal writes, and the live bus.
pub struct AgentstreamLlm {
    parts: GatewayParts,
}

impl AgentstreamLlm {
    /// Begin constructing a gateway.
    pub fn builder(parts: GatewayParts) -> GatewayBuilder {
        GatewayBuilder::new(parts)
    }

    /// The configured profiles.
    pub fn provider_profiles(&self) -> &[ProviderProfile] {
        &self.parts.profiles.0
    }

    /// The journal mode in force.
    pub fn journal_mode(&self) -> JournalMode {
        self.parts.journal_mode
    }
}

/// One live event for the console activity dock.
///
/// Deltas are display-only. The terminal response carries the authoritative text.
#[derive(Debug, Clone)]
pub enum LiveLlmEvent {
    /// A logical call is about to reach the provider.
    Request {
        /// The stable call id.
        call_id: Ulid,
        /// Where the call came from.
        context: CallContext,
        /// The selected profile.
        provider: String,
        /// Whether the caller asked for deltas.
        streamed: bool,
        /// The neutral request.
        request: LlmRequest,
    },
    /// A coalesced display fragment.
    Delta {
        /// The stable call id.
        call_id: Ulid,
        /// The fragment.
        text: String,
        /// Total characters emitted so far.
        total_chars: u32,
        /// Whether the display cap was reached.
        truncated: bool,
    },
    /// The call returned.
    Response {
        /// The stable call id.
        call_id: Ulid,
        /// Wall-clock latency.
        latency_ms: u64,
        /// The neutral response.
        response: LlmResponse,
    },
    /// The call failed.
    Error {
        /// The stable call id.
        call_id: Ulid,
        /// Wall-clock latency.
        latency_ms: u64,
        /// Characters reported before the failure.
        partial_chars: u32,
        /// Stable error kind.
        kind: String,
        /// Redacted, bounded error.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str) -> ProviderProfile {
        ProviderProfile {
            name: name.into(),
            kind: ProviderKind::OpenAiCompatible,
            endpoint_env: "TEST_BASE_URL".into(),
            api_key_env: "TEST_API_KEY".into(),
            model_env: None,
            model: None,
            timeout_ms: 1_000,
            stream: false,
            temperature: None,
            omit_max_tokens: false,
            structured_output: StructuredOutput::JsonSchema,
        }
    }

    #[test]
    fn journal_mode_defaults_to_required() {
        assert_eq!(JournalMode::default(), JournalMode::Required);
    }

    #[test]
    fn profiles_reject_empty_and_duplicate_names() {
        assert!(ProviderProfiles::new(vec![profile("default")])
            .validate()
            .is_ok());
        assert!(ProviderProfiles::new(vec![profile("")]).validate().is_err());
        assert!(
            ProviderProfiles::new(vec![profile("default"), profile("default")])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn profiles_are_findable_by_name() {
        let profiles = ProviderProfiles::new(vec![profile("default"), profile("hermes")]);
        assert_eq!(
            profiles.get("hermes").map(|p| p.name.as_str()),
            Some("hermes")
        );
        assert!(profiles.get("missing").is_none());
        assert_eq!(profiles.len(), 2);
    }

    #[test]
    fn live_events_are_cloneable() {
        let event = LiveLlmEvent::Error {
            call_id: Ulid::from_parts(1, 2),
            latency_ms: 3,
            partial_chars: 0,
            kind: "transport".into(),
            message: "boom".into(),
        };
        let cloned = event.clone();
        assert!(matches!(cloned, LiveLlmEvent::Error { .. }));
    }
}
