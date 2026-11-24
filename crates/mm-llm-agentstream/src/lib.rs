//! The agentstream-backed LLM gateway.
//!
//! This crate is the composition boundary between the provider substrate and the
//! memory layer. An application depends on [`AgentstreamLlm`] instead of
//! constructing `OpenAiClient` or `LlmService` directly.
//!
//! A gateway call has one durable shape: a required `llm_request` event before the
//! provider is touched, one best-effort `llm_attempt` event per provider round, and
//! exactly one terminal `llm_response` or `llm_error` event. Live publication is a
//! bounded, best-effort projection for the activity dock; it cannot change an
//! answer or fail a call.

#![forbid(unsafe_code)]

mod journal;
mod live;
mod pool;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use mm_data::AgentstreamStore;
use mm_log::Logger;
use mm_store_graph::GraphHandle;
use mm_store_sqlite::SqliteStore;
use tokio::sync::{broadcast, RwLock};
use ulid::Ulid;

pub use mm_llm::{
    CacheMode, CallContext, DecoderKind, LlmClient, LlmConfig, LlmError, LlmRequest, LlmResponse,
    Message, ProviderToolCall, Purpose, Result, Role, SchemaId, StructuredOut, ToolSchema, Usage,
};

use mm_llm::Complexity;
use mm_llm::{DeltaSink, LlmService, ModelSpec, StructuredOutput};

use journal::{CallJournal, CallScope, JournalSink};
use live::{LiveBus, LiveDeltaSink};
use pool::ProviderPool;

/// Environment switch for live gateway calls. Default on; an explicit "off",
/// "false", "0", or "no" makes every live call a configuration error.
pub const KILL_SWITCH_ENV: &str = "MM_LLM_AGENTSTREAM";

/// The single reason reported when the gateway is disabled.
pub const DISABLED_REASON: &str =
    "MM_LLM_AGENTSTREAM is off; live language-model calls are disabled";

/// Whether live gateway calls are enabled by the process environment.
pub fn is_enabled() -> bool {
    enabled_for(std::env::var(KILL_SWITCH_ENV).ok().as_deref())
}

fn enabled_for(value: Option<&str>) -> bool {
    !matches!(
        value
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("off" | "false" | "0" | "no")
    )
}

fn disabled_error() -> LlmError {
    LlmError::Config(DISABLED_REASON.to_owned())
}

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

    /// The default `default` and `hermes` profiles.
    pub fn standard() -> Self {
        Self(vec![
            ProviderProfile {
                name: "default".into(),
                kind: ProviderKind::OpenAiCompatible,
                endpoint_env: "OPENAI_BASE_URL".into(),
                api_key_env: "OPENAI_API_KEY".into(),
                model_env: None,
                model: None,
                timeout_ms: 30_000,
                stream: false,
                temperature: None,
                omit_max_tokens: false,
                structured_output: StructuredOutput::JsonObject,
            },
            ProviderProfile {
                name: "hermes".into(),
                kind: ProviderKind::OpenAiCompatible,
                endpoint_env: "MM_AGENT_HERMES_BASE_URL".into(),
                api_key_env: "MM_AGENT_HERMES_API_KEY".into(),
                model_env: Some("MM_AGENT_HERMES_MODEL".into()),
                model: None,
                timeout_ms: 120_000,
                stream: true,
                temperature: Some(0.2),
                omit_max_tokens: true,
                structured_output: StructuredOutput::JsonObject,
            },
        ])
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
    /// A crate-private journal override for journal-failure tests.
    pub(crate) journal_sink: Option<Arc<dyn JournalSink>>,
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
            journal_sink: None,
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

    /// Replace the production journal sink. Test-only.
    #[cfg(test)]
    pub(crate) fn with_journal_sink(mut self, sink: Arc<dyn JournalSink>) -> Self {
        self.journal_sink = Some(sink);
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
    pub fn build(self) -> Result<AgentstreamLlm> {
        let mut parts = self.parts;
        if parts.profiles.is_empty() {
            parts.profiles = ProviderProfiles::standard();
        }
        parts.profiles.validate()?;

        let config = config_with_profile_models(parts.config.clone(), &parts.profiles);
        let journal = match parts.journal_sink.clone() {
            Some(sink) => CallJournal::with_sink(
                sink,
                parts.journal_mode,
                parts.logger.clone(),
                parts.user_id.clone(),
            ),
            None => CallJournal::new(
                parts.journal.clone(),
                parts.journal_mode,
                parts.logger.clone(),
                parts.user_id.clone(),
            ),
        };
        let journal = Arc::new(journal);
        let scopes = Arc::new(Mutex::new(BTreeMap::new()));
        let pool = Arc::new(ProviderPool::new(
            config.clone(),
            parts.profiles.clone(),
            parts.injected_client.clone(),
            scopes.clone(),
        ));
        let service = LlmService::new(
            config,
            parts.store.clone(),
            parts.graph.clone(),
            parts.logger.clone(),
            pool.clone(),
        )?;

        Ok(AgentstreamLlm {
            parts,
            service: RwLock::new(service),
            pool,
            journal,
            live: Arc::new(LiveBus::new(256)),
            scopes,
            enabled: is_enabled(),
        })
    }
}

/// The outcome of one provider model probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProbe {
    /// Whether the provider answered `/models` successfully.
    pub available: bool,
    /// A redacted reason when unavailable.
    pub reason: Option<String>,
    /// The advertised model ids.
    pub models: Vec<String>,
}

/// The only public LLM client interface application code may use.
pub struct AgentstreamLlm {
    parts: GatewayParts,
    service: RwLock<LlmService>,
    pool: Arc<ProviderPool>,
    journal: Arc<CallJournal>,
    live: Arc<LiveBus>,
    scopes: Arc<Mutex<BTreeMap<Ulid, Arc<CallScope>>>>,
    enabled: bool,
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

    /// Register a typed schema with the service and every provider profile.
    pub async fn register<T: StructuredOut>(&self) -> Result<()> {
        self.service.write().await.register::<T>()
    }

    /// Register an ad-hoc schema with the service and every provider profile.
    pub async fn register_schema(&self, id: SchemaId, schema: serde_json::Value) -> Result<()> {
        self.service.write().await.register_schema(id, schema)
    }

    /// Subscribe to the live call stream for the activity dock.
    pub fn subscribe(&self) -> broadcast::Receiver<LiveLlmEvent> {
        self.live.subscribe()
    }

    /// Replace the scripted/replay client, or clear it to use provider profiles.
    ///
    /// Test harnesses use this to install a scripted controller after boot. The
    /// call still takes the same journal and live path as a provider call.
    pub fn set_injected_client(&self, client: Option<Arc<dyn LlmClient>>) {
        self.pool.set_injected(client);
    }

    /// Disable live calls without mutating the process environment in a test.
    #[cfg(test)]
    pub(crate) fn disable_for_test(&mut self) {
        self.enabled = false;
    }

    /// Probe a provider profile's `/models` endpoint.
    ///
    /// The probe is journaled as one `llm_models` event whether it succeeds or
    /// fails. An unconfigured profile produces a reason without opening a socket.
    pub async fn models(&self, provider: &str) -> ModelProbe {
        if !self.enabled {
            return ModelProbe {
                available: false,
                reason: Some(DISABLED_REASON.to_owned()),
                models: Vec::new(),
            };
        }
        let probe = match self.pool.models(provider).await {
            Ok(models) => ModelProbe {
                available: true,
                reason: None,
                models,
            },
            Err(error) => ModelProbe {
                available: false,
                reason: Some(self.journal.redacted_error(&error)),
                models: Vec::new(),
            },
        };
        let session = format!("llm/models/{}", sanitize_source(provider));
        self.journal
            .models(
                &session,
                provider,
                probe.available,
                &probe.models,
                probe.reason.as_deref(),
            )
            .await;
        probe
    }

    /// Make one logical call.
    pub async fn call(&self, req: LlmRequest) -> Result<LlmResponse> {
        if !self.enabled {
            return Err(disabled_error());
        }
        let call_id = req.call_id.unwrap_or_else(mm_llm::accounting::new_call_id);
        let outcome = self.execute(req, None, call_id).await?;
        match &outcome.result {
            Ok(response) => self.live.publish(LiveLlmEvent::Response {
                call_id,
                latency_ms: outcome.latency_ms,
                response: response.clone(),
            }),
            Err(error) => self.live.publish(LiveLlmEvent::Error {
                call_id,
                latency_ms: outcome.latency_ms,
                partial_chars: 0,
                kind: error.kind().to_owned(),
                message: self.journal.redacted_error(error),
            }),
        }
        outcome.result
    }

    /// Make one logical call and report display deltas while it runs.
    pub async fn call_stream(
        &self,
        req: LlmRequest,
        sink: &mut dyn DeltaSink,
    ) -> Result<LlmResponse> {
        if !self.enabled {
            return Err(disabled_error());
        }
        let call_id = req.call_id.unwrap_or_else(mm_llm::accounting::new_call_id);
        let mut live_sink = LiveDeltaSink::new(sink, Arc::clone(&self.live), call_id);
        let outcome = self.execute(req, Some(&mut live_sink), call_id).await?;
        live_sink.finish();
        let partial_chars = live_sink.total_chars();
        match &outcome.result {
            Ok(response) => self.live.publish(LiveLlmEvent::Response {
                call_id,
                latency_ms: outcome.latency_ms,
                response: response.clone(),
            }),
            Err(error) => self.live.publish(LiveLlmEvent::Error {
                call_id,
                latency_ms: outcome.latency_ms,
                partial_chars,
                kind: error.kind().to_owned(),
                message: self.journal.redacted_error(error),
            }),
        }
        outcome.result
    }

    async fn execute(
        &self,
        mut req: LlmRequest,
        sink: Option<&mut dyn DeltaSink>,
        call_id: Ulid,
    ) -> Result<CallOutcome> {
        req.call_id = Some(call_id);
        let context = mm_llm::current_context().unwrap_or_default();
        let profile = self.pool.profile(&req)?;
        normalize_request(&mut req, &profile)?;
        let session_id = journal::session_id(&context);
        let turn_id = journal::turn_id(&context, call_id);
        let streamed = sink.is_some();

        // The required append happens before a scope exists and before any
        // provider is resolved. If it fails in Required mode, no provider call is
        // made.
        self.journal
            .request(
                &req,
                &context,
                &profile.name,
                streamed,
                &session_id,
                &turn_id,
            )
            .await?;

        let scope = Arc::new(CallScope::new(
            Arc::clone(&self.journal),
            call_id,
            session_id,
            turn_id,
            profile.name.clone(),
        ));
        self.scopes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(call_id, Arc::clone(&scope));
        self.live.publish(LiveLlmEvent::Request {
            call_id,
            context,
            provider: profile.name.clone(),
            streamed,
            request: req.clone(),
        });

        let started = Instant::now();
        let service = self.service.read().await;
        let result = match sink {
            Some(sink) => service.call_stream(req, sink).await,
            None => service.call(req).await,
        };
        drop(service);
        self.scopes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&call_id);
        let latency_ms = started.elapsed().as_millis() as u64;

        let result = match result {
            Ok(response) => {
                // The adapter reports whether the provider actually used SSE. A
                // stream request that fell back to a JSON answer stays `false`.
                let mut response = response;
                response.streamed = response.streamed && streamed;
                self.journal.response(&scope, &response, latency_ms).await;
                Ok(response)
            }
            Err(error) => {
                self.journal.error(&scope, &error, latency_ms, 0).await;
                Err(error)
            }
        };

        Ok(CallOutcome { result, latency_ms })
    }
}

struct CallOutcome {
    result: Result<LlmResponse>,
    latency_ms: u64,
}

#[async_trait]
impl LlmClient for AgentstreamLlm {
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        self.call(req.clone()).await
    }

    async fn complete_stream(
        &self,
        req: &LlmRequest,
        sink: &mut dyn DeltaSink,
    ) -> Result<LlmResponse> {
        self.call_stream(req.clone(), sink).await
    }

    fn provider(&self) -> &str {
        "agentstream"
    }

    fn provider_for<'a>(&'a self, req: &'a LlmRequest) -> &'a str {
        req.provider.as_deref().unwrap_or("agentstream")
    }

    fn decoder(&self) -> DecoderKind {
        DecoderKind::None
    }
}

fn normalize_request(req: &mut LlmRequest, profile: &ProviderProfile) -> Result<()> {
    req.provider = Some(profile.name.clone());
    if req.model.is_none() {
        req.model = profile_model(profile);
    }
    if req.purpose == Purpose::Agent && req.model.is_none() {
        return Err(LlmError::Config(
            "an agent call must pin a model (the profile has no model configured)".into(),
        ));
    }

    let ceiling = profile.timeout_ms.max(1);
    req.timeout_ms = Some(req.timeout_ms.unwrap_or(ceiling).max(1).min(ceiling));
    if let Some(temperature) = profile.temperature {
        if req.temperature == 0.0 {
            req.temperature = temperature;
        }
    }
    Ok(())
}

fn sanitize_source(source: &str) -> String {
    let sanitized: String = source
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "default".to_owned()
    } else {
        sanitized
    }
}

fn profile_model(profile: &ProviderProfile) -> Option<String> {
    profile
        .model_env
        .as_deref()
        .and_then(|name| std::env::var(name).ok())
        .filter(|model| !model.trim().is_empty())
        .or_else(|| profile.model.clone())
}

fn config_with_profile_models(mut config: LlmConfig, profiles: &ProviderProfiles) -> LlmConfig {
    for profile in profiles.iter() {
        let Some(model) = profile_model(profile) else {
            continue;
        };
        if config.model(&model).is_none() {
            config.models.push(ModelSpec {
                name: model,
                input_cost_micros_per_1k: 0,
                output_cost_micros_per_1k: 0,
                typical_latency_ms: 0,
                complexity_ceiling: Complexity::VeryHigh,
                mock: false,
            });
        }
    }
    config
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
mod gateway_tests;

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
    fn the_kill_switch_defaults_on_and_only_explicit_off_values_disable_it() {
        assert!(enabled_for(None));
        assert!(enabled_for(Some("")));
        assert!(enabled_for(Some("on")));
        assert!(enabled_for(Some("true")));
        for value in ["off", "OFF", " false ", "0", "no"] {
            assert!(!enabled_for(Some(value)), "{value}");
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
    fn standard_profiles_cover_default_and_hermes() {
        let profiles = ProviderProfiles::standard();
        assert!(profiles.get("default").is_some());
        let hermes = profiles.get("hermes").unwrap();
        assert_eq!(hermes.model_env.as_deref(), Some("MM_AGENT_HERMES_MODEL"));
        assert!(hermes.omit_max_tokens);
        assert_eq!(hermes.temperature, Some(0.2));
    }

    #[test]
    fn model_probe_session_names_are_stable() {
        assert_eq!(sanitize_source("hermes"), "hermes");
        assert_eq!(sanitize_source("Hermes Local"), "hermes-local");
        assert_eq!(sanitize_source(""), "default");
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

    #[test]
    fn a_profile_model_is_added_to_routing_config() {
        let mut hermes = profile("hermes");
        hermes.model = Some("agent-model".into());
        let config =
            config_with_profile_models(LlmConfig::default(), &ProviderProfiles::new(vec![hermes]));
        assert!(config.model("agent-model").is_some());
    }

    #[test]
    fn request_normalization_applies_profile_policy() {
        let mut hermes = profile("hermes");
        hermes.model = Some("agent-model".into());
        hermes.timeout_ms = 500;
        hermes.temperature = Some(0.2);
        let mut request =
            LlmRequest::new(Purpose::Agent, vec![Message::user("hello")]).with_timeout(5_000);
        normalize_request(&mut request, &hermes).unwrap();
        assert_eq!(request.provider.as_deref(), Some("hermes"));
        assert_eq!(request.model.as_deref(), Some("agent-model"));
        assert_eq!(request.timeout_ms, Some(500));
        assert_eq!(request.temperature, 0.2);
    }
}
