//! The private provider pool.
//!
//! Callers never see a profile adapter. The pool selects the profile named on the
//! request, constructs the thin OpenAI-compatible client for that profile, applies
//! its timeout and body policy, and records one attempt event per provider round.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mm_llm::{
    DecoderKind, DeltaSink, LlmClient, LlmConfig, LlmError, LlmRequest, LlmResponse, OpenAiClient,
    ProviderEnv, Result, SchemaId, Usage,
};
use serde_json::Value;
use ulid::Ulid;

use crate::journal::CallScope;
use crate::ProviderProfile;

/// A dispatch table over the configured profiles.
pub(crate) struct ProviderPool {
    config: LlmConfig,
    profiles: crate::ProviderProfiles,
    injected: RwLock<Option<Arc<dyn LlmClient>>>,
    schemas: RwLock<mm_llm::SchemaRegistry>,
    scopes: Arc<Mutex<BTreeMap<Ulid, Arc<CallScope>>>>,
}

impl ProviderPool {
    pub(crate) fn new(
        config: LlmConfig,
        profiles: crate::ProviderProfiles,
        injected: Option<Arc<dyn LlmClient>>,
        scopes: Arc<Mutex<BTreeMap<Ulid, Arc<CallScope>>>>,
    ) -> Self {
        Self {
            config,
            profiles,
            injected: RwLock::new(injected),
            schemas: RwLock::new(mm_llm::SchemaRegistry::new()),
            scopes,
        }
    }

    /// The profile selected by `req`.
    pub(crate) fn profile(&self, req: &LlmRequest) -> Result<ProviderProfile> {
        let name = req.provider.as_deref().unwrap_or("default");
        self.profiles.get(name).cloned().ok_or_else(|| {
            LlmError::Config(format!(
                "provider profile `{name}` is not configured; known profiles: {}",
                self.profile_names().join(", ")
            ))
        })
    }

    fn profile_names(&self) -> Vec<String> {
        self.profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect()
    }

    /// Replace the scripted/replay client, or clear it to use the profiles.
    pub(crate) fn set_injected(&self, client: Option<Arc<dyn LlmClient>>) {
        *self
            .injected
            .write()
            .unwrap_or_else(|error| error.into_inner()) = client;
    }

    fn client_for(&self, profile: &ProviderProfile) -> Result<Arc<dyn LlmClient>> {
        if let Some(client) = self
            .injected
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
        {
            return Ok(client);
        }

        Ok(Arc::new(self.provider_client(profile)?))
    }

    /// Read the profile's advertised models from its `/models` endpoint.
    ///
    /// Injected scripted/replay clients never answer a live probe; discovery is a
    /// profile transport concern.
    pub(crate) async fn models(&self, provider: &str) -> Result<Vec<String>> {
        let profile = self.profiles.get(provider).cloned().ok_or_else(|| {
            LlmError::Config(format!(
                "provider profile `{provider}` is not configured; known profiles: {}",
                self.profile_names().join(", ")
            ))
        })?;
        self.provider_client(&profile)?.models().await
    }

    fn provider_client(&self, profile: &ProviderProfile) -> Result<OpenAiClient> {
        let endpoint = env_value(&profile.endpoint_env);
        let key = env_value(&profile.api_key_env);
        let env = ProviderEnv::from_pairs(endpoint, key).ok_or_else(|| {
            LlmError::ProviderNotConfigured(format!(
                "profile `{}` requires {} and {}",
                profile.name, profile.endpoint_env, profile.api_key_env
            ))
        })?;

        let mut config = self.config.clone();
        config.defaults.timeout_ms = profile.timeout_ms.min(u64::from(u32::MAX)) as u32;
        config.defaults.structured_output = profile.structured_output;

        let schemas = self
            .schemas
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        Ok(OpenAiClient::from_env(&config, Some(env))?
            .with_schemas(schemas)
            .with_omit_max_tokens(profile.omit_max_tokens))
    }

    fn scope(&self, req: &LlmRequest) -> Option<Arc<CallScope>> {
        req.call_id.and_then(|call_id| {
            self.scopes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(&call_id)
                .cloned()
        })
    }

    fn budget_ms(&self, req: &LlmRequest, profile: &ProviderProfile) -> u32 {
        let requested = req.timeout_ms.unwrap_or(profile.timeout_ms);
        requested
            .max(1)
            .min(profile.timeout_ms.max(1))
            .min(u64::from(u32::MAX)) as u32
    }

    async fn record(
        &self,
        req: &LlmRequest,
        attempt_no: u32,
        started: Instant,
        result: &Result<LlmResponse>,
    ) {
        let Some(scope) = self.scope(req) else {
            return;
        };
        let status = match result {
            Ok(_) => "ok",
            Err(LlmError::Timeout(_)) => "timeout",
            Err(LlmError::SchemaRejected { .. }) => "schema_rejected",
            Err(_) => "transport",
        };
        let (model, usage) = match result {
            Ok(response) => (Some(response.model.as_str()), response.usage),
            Err(_) => (req.model.as_deref(), Usage::default()),
        };
        scope
            .record_attempt(
                attempt_no,
                model,
                status,
                started.elapsed().as_millis() as u64,
                usage,
            )
            .await;
    }
}

#[async_trait]
impl LlmClient for ProviderPool {
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        let profile = self.profile(req)?;
        let client = self.client_for(&profile)?;
        let attempt_no = self
            .scope(req)
            .map_or_else(|| 1, |scope| scope.next_attempt());
        let started = Instant::now();
        let budget = Duration::from_millis(u64::from(self.budget_ms(req, &profile)));

        let result = match tokio::time::timeout(budget, client.complete(req)).await {
            Ok(result) => result,
            Err(_) => Err(LlmError::Timeout(self.budget_ms(req, &profile).max(1))),
        };
        self.record(req, attempt_no, started, &result).await;
        result
    }

    async fn complete_stream(
        &self,
        req: &LlmRequest,
        sink: &mut dyn DeltaSink,
    ) -> Result<LlmResponse> {
        let profile = self.profile(req)?;
        let client = self.client_for(&profile)?;
        let attempt_no = self
            .scope(req)
            .map_or_else(|| 1, |scope| scope.next_attempt());
        let started = Instant::now();
        let budget = Duration::from_millis(u64::from(self.budget_ms(req, &profile)));

        let result = match tokio::time::timeout(budget, client.complete_stream(req, sink)).await {
            Ok(result) => result,
            Err(_) => Err(LlmError::Timeout(self.budget_ms(req, &profile).max(1))),
        };
        self.record(req, attempt_no, started, &result).await;
        result
    }

    fn provider(&self) -> &str {
        "agentstream"
    }

    fn provider_for<'a>(&'a self, req: &'a LlmRequest) -> &'a str {
        req.provider.as_deref().unwrap_or("default")
    }

    fn decoder(&self) -> DecoderKind {
        DecoderKind::None
    }

    fn provide_schema(&self, id: SchemaId, schema: Value) -> Result<()> {
        self.schemas
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .register(id.clone(), schema.clone())?;
        if let Some(client) = self
            .injected
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
        {
            client.provide_schema(id, schema)?;
        }
        Ok(())
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}
