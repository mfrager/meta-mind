//! Hermes: a dedicated OpenAI-compatible agent backend.
//!
//! Hermes owns configuration, health, and translation only. Provider transport,
//! SSE decoding, `/models` discovery, and journaling belong to
//! [`AgentstreamLlm`]; this module maps an [`AgentRequest`] onto a neutral
//! [`LlmRequest`] and maps the [`LlmResponse`] back into an
//! [`AgentTurnResult`].

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use mm_llm::{CacheMode, CallContext, CollectSink, LlmError, LlmRequest, Message, Purpose, Role};
use mm_llm_agentstream::AgentstreamLlm;

use crate::{
    AgentArtifact, AgentBackend, AgentError, AgentRequest, AgentRunStatus, AgentToolSummary,
    AgentTurnResult, AgentUsage, Result,
};

/// Environment variable for the Hermes base URL.
pub const BASE_URL_ENV: &str = "MM_AGENT_HERMES_BASE_URL";
/// Environment variable for the bearer token.
pub const API_KEY_ENV: &str = "MM_AGENT_HERMES_API_KEY";
/// Environment variable for the model name.
pub const MODEL_ENV: &str = "MM_AGENT_HERMES_MODEL";
/// Environment variable for SSE streaming.
pub const STREAM_ENV: &str = "MM_AGENT_HERMES_STREAM";
/// Environment variable for the per-turn timeout.
pub const TIMEOUT_ENV: &str = "MM_AGENT_HERMES_TIMEOUT_MS";
/// Environment variable for the context budget.
pub const MAX_CONTEXT_ENV: &str = "MM_AGENT_HERMES_MAX_CONTEXT_TOKENS";

/// Resolved Hermes configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HermesConfig {
    /// OpenAI-compatible base, for example `http://127.0.0.1:9000/v1`.
    pub base_url: String,
    /// Optional bearer token.
    pub api_key: Option<String>,
    /// Model name, or a model discovered from `/models`.
    pub model: Option<String>,
    /// Use SSE when the service supports it.
    pub stream: bool,
    /// Per-turn wall clock.
    pub timeout_ms: u64,
    /// Approximate context budget before replay truncation.
    pub max_context_tokens: u32,
}

impl HermesConfig {
    /// Resolve from the environment, then the optional `[hermes]` config file.
    pub fn from_env_or_file(path: Option<&std::path::Path>) -> Option<Self> {
        if let Some(config) = Self::from_env() {
            return Some(config);
        }
        let path = path?;
        let text = std::fs::read_to_string(path).ok()?;
        let value: toml::Value = toml::from_str(&text).ok()?;
        let hermes = value.get("hermes")?;
        let base_url = hermes
            .get("endpoint")
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if base_url.is_empty() {
            return None;
        }
        let model = hermes
            .get("model")
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_owned);
        let api_key = std::env::var(API_KEY_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty());
        Some(Self {
            base_url,
            api_key,
            model,
            stream: std::env::var(STREAM_ENV)
                .ok()
                .map(|value| !matches!(value.as_str(), "0" | "false" | "off"))
                .unwrap_or(true),
            timeout_ms: std::env::var(TIMEOUT_ENV)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(120_000),
            max_context_tokens: std::env::var(MAX_CONTEXT_ENV)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(96_000),
        })
    }

    /// Resolve from environment variables. Returns `None` when no base URL is set.
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var(BASE_URL_ENV).ok()?.trim().to_owned();
        if base_url.is_empty() {
            return None;
        }
        let api_key = std::env::var(API_KEY_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty());
        let model = std::env::var(MODEL_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty());
        Some(Self {
            base_url,
            api_key,
            model,
            stream: std::env::var(STREAM_ENV)
                .ok()
                .map(|value| !matches!(value.as_str(), "0" | "false" | "off"))
                .unwrap_or(true),
            timeout_ms: std::env::var(TIMEOUT_ENV)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(120_000),
            max_context_tokens: std::env::var(MAX_CONTEXT_ENV)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(96_000),
        })
    }
}

/// A backend availability snapshot for health and the inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HermesProbe {
    /// Whether `/models` answered.
    pub available: bool,
    /// Redacted reason when unavailable.
    pub reason: Option<String>,
    /// Advertised models.
    pub models: Vec<String>,
}

/// The Hermes backend.
pub struct HermesBackend {
    config: Option<HermesConfig>,
    gateway: Option<Arc<AgentstreamLlm>>,
    probe: Mutex<HermesProbe>,
}

impl std::fmt::Debug for HermesBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HermesBackend")
            .field("config", &self.config)
            .field("gateway", &self.gateway.is_some())
            .field("probe", &self.probe)
            .finish()
    }
}

impl HermesBackend {
    /// A backend for an explicit configuration and the boot gateway.
    pub fn with_gateway(config: HermesConfig, gateway: Arc<AgentstreamLlm>) -> Self {
        Self {
            config: Some(config),
            gateway: Some(gateway),
            probe: Mutex::new(HermesProbe {
                available: false,
                reason: Some("not probed".into()),
                models: Vec::new(),
            }),
        }
    }

    /// An unavailable backend with an exact, redacted reason.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            config: None,
            gateway: None,
            probe: Mutex::new(HermesProbe {
                available: false,
                reason: Some(reason.into()),
                models: Vec::new(),
            }),
        }
    }

    /// Resolve the backend from the environment or a config file and the gateway.
    pub fn from_env(gateway: Option<Arc<AgentstreamLlm>>) -> Self {
        Self::from_config_file(Some(std::path::Path::new("config/agents.toml")), gateway)
    }

    /// Resolve the backend from an explicit config file and the gateway.
    pub fn from_config_file(
        path: Option<&std::path::Path>,
        gateway: Option<Arc<AgentstreamLlm>>,
    ) -> Self {
        let Some(config) = HermesConfig::from_env_or_file(path) else {
            return Self::unavailable(format!("{BASE_URL_ENV} is unset"));
        };
        match gateway {
            Some(gateway) => Self::with_gateway(config, gateway),
            None => Self::unavailable("the language-model gateway is unavailable"),
        }
    }

    /// The configured per-turn timeout.
    pub fn timeout_ms(&self) -> u64 {
        self.config
            .as_ref()
            .map(|config| config.timeout_ms)
            .unwrap_or(120_000)
    }

    /// The configured context budget.
    pub fn max_context_tokens(&self) -> u32 {
        self.config
            .as_ref()
            .map(|config| config.max_context_tokens)
            .unwrap_or(96_000)
    }

    /// The current probe snapshot.
    pub fn probe(&self) -> HermesProbe {
        self.probe
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Probe `/models` through the gateway and record availability.
    pub async fn refresh(&self) -> HermesProbe {
        let Some(config) = &self.config else {
            return self.probe();
        };
        let Some(gateway) = &self.gateway else {
            let probe = HermesProbe {
                available: false,
                reason: Some("the language-model gateway is unavailable".into()),
                models: Vec::new(),
            };
            *self.probe.lock().unwrap_or_else(|error| error.into_inner()) = probe.clone();
            return probe;
        };
        let probe =
            match tokio::time::timeout(Duration::from_secs(3), gateway.models("hermes")).await {
                Ok(probe) if probe.available => HermesProbe {
                    available: true,
                    reason: None,
                    models: probe.models,
                },
                Ok(probe) => HermesProbe {
                    available: false,
                    reason: Some(format!(
                        "{}: {}",
                        config.base_url,
                        probe
                            .reason
                            .unwrap_or_else(|| "the backend did not answer /models".into())
                    )),
                    models: Vec::new(),
                },
                Err(_) => HermesProbe {
                    available: false,
                    reason: Some("probe timed out after 3000 ms".into()),
                    models: Vec::new(),
                },
            };
        *self.probe.lock().unwrap_or_else(|error| error.into_inner()) = probe.clone();
        probe
    }

    fn resolved_model(&self) -> Option<String> {
        let config = self.config.as_ref()?;
        config.model.clone().or_else(|| {
            self.probe
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .models
                .first()
                .cloned()
        })
    }

    fn config(&self) -> Result<&HermesConfig> {
        self.config
            .as_ref()
            .ok_or_else(|| AgentError::Unavailable(format!("{BASE_URL_ENV} is unset")))
    }
}

#[async_trait]
impl AgentBackend for HermesBackend {
    async fn run_turn(&self, request: &AgentRequest) -> Result<AgentTurnResult> {
        let config = self.config()?.clone();
        let gateway = self.gateway.as_ref().ok_or_else(|| {
            AgentError::Unavailable("the language-model gateway is unavailable".into())
        })?;
        let mut probe = self.probe();
        if !probe.available {
            probe = self.refresh().await;
        }
        if !probe.available {
            return Err(AgentError::Unavailable(format!(
                "{} ({})",
                config.base_url,
                probe
                    .reason
                    .unwrap_or_else(|| "the backend did not answer /models".into())
            )));
        }
        let model = self
            .resolved_model()
            .ok_or_else(|| AgentError::Rejected("no Hermes model is configured".into()))?;
        let messages = build_messages(request)?;
        let timeout = request.timeout_ms.max(1).min(config.timeout_ms.max(1));
        let llm_request = LlmRequest::new(Purpose::Agent, messages)
            .with_model(model)
            .with_provider("hermes")
            .with_cache(CacheMode::Bypass)
            .with_timeout(timeout);
        let context = CallContext::new("agent").with_run(request.run_id.clone());

        let result = tokio::time::timeout(
            Duration::from_millis(timeout),
            mm_llm::scope(context, async {
                if config.stream {
                    let mut sink = CollectSink::default();
                    gateway.call_stream(llm_request, &mut sink).await
                } else {
                    gateway.call(llm_request).await
                }
            }),
        )
        .await
        .map_err(|_| AgentError::Timeout(timeout))?
        .map_err(map_llm_error)?;

        Ok(map_response(result))
    }
}

fn build_messages(request: &AgentRequest) -> Result<Vec<Message>> {
    let mut messages = Vec::new();
    if let Some(system) = &request.context {
        if !system.trim().is_empty() {
            messages.push(Message::system(system));
        }
    }
    if request.messages.is_empty() {
        messages.push(Message::user(request.input.clone()));
    } else {
        for message in &request.messages {
            let role = match message.role.as_str() {
                "system" => Role::System,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            messages.push(Message {
                role,
                content: message.content.clone(),
                name: None,
                tool_call_id: None,
                tool_calls: Vec::new(),
            });
        }
    }
    Ok(messages)
}

fn map_response(response: mm_llm::LlmResponse) -> AgentTurnResult {
    let content = response.text.clone();
    let tool_calls: Vec<AgentToolSummary> = response
        .tool_calls
        .iter()
        .map(|call| AgentToolSummary {
            name: call.name.clone(),
            arguments: Some(call.arguments.chars().take(500).collect()),
        })
        .collect();
    let status = status_from(&response, &tool_calls, &content);
    AgentTurnResult {
        run_id: String::new(),
        turn: 0,
        status,
        workspace: std::path::PathBuf::new(),
        message: content.clone(),
        summary: content,
        summary_detail: None,
        tool_calls,
        artifacts: Vec::new(),
        next: next_for(status),
        error: None,
        usage: AgentUsage {
            prompt_tokens: u64::from(response.usage.tokens_in),
            completion_tokens: u64::from(response.usage.tokens_out),
        },
        streamed: response.streamed,
        model: (!response.model.is_empty()).then_some(response.model),
        raw_ref: None,
    }
}

fn map_llm_error(error: LlmError) -> AgentError {
    match error {
        LlmError::Timeout(ms) => AgentError::Timeout(u64::from(ms)),
        LlmError::ProviderNotConfigured(message)
        | LlmError::Transport(message)
        | LlmError::NetworkForbidden(message) => AgentError::Unavailable(message),
        LlmError::Config(message) => AgentError::Rejected(message),
        LlmError::SchemaRejected { schema_id, detail } => {
            AgentError::Rejected(format!("{schema_id}: {detail}"))
        }
        LlmError::CacheMiss(message)
        | LlmError::BudgetExhausted(message)
        | LlmError::Replay(message) => AgentError::Rejected(message),
        LlmError::Io(message) | LlmError::Db(message) | LlmError::Journal(message) => {
            AgentError::Other(message)
        }
    }
}

fn status_from(
    response: &mm_llm::LlmResponse,
    tool_calls: &[AgentToolSummary],
    content: &str,
) -> AgentRunStatus {
    if let Some(status) = response
        .provider_metadata
        .get("mm_status")
        .and_then(Value::as_str)
        .and_then(parse_status)
    {
        return status;
    }
    if matches!(
        response.finish_reason.as_deref(),
        Some("tool_calls" | "length")
    ) || !tool_calls.is_empty()
    {
        return AgentRunStatus::Running;
    }
    if asks_question(content) {
        return AgentRunStatus::WaitingInput;
    }
    AgentRunStatus::Done
}

fn parse_status(value: &str) -> Option<AgentRunStatus> {
    match value.trim().to_ascii_lowercase().as_str() {
        "queued" => Some(AgentRunStatus::Queued),
        "running" => Some(AgentRunStatus::Running),
        "waiting" => Some(AgentRunStatus::Waiting),
        "waiting_input" | "waitinginput" => Some(AgentRunStatus::WaitingInput),
        "done" | "complete" | "completed" => Some(AgentRunStatus::Done),
        "failed" | "error" => Some(AgentRunStatus::Failed),
        "cancelled" | "canceled" => Some(AgentRunStatus::Cancelled),
        _ => None,
    }
}

fn next_for(status: AgentRunStatus) -> Option<String> {
    match status {
        AgentRunStatus::Running | AgentRunStatus::Waiting => Some("continue".into()),
        AgentRunStatus::WaitingInput => Some("wait_for_input".into()),
        _ => None,
    }
}

fn asks_question(content: &str) -> bool {
    let trimmed = content.trim_end();
    trimmed.ends_with('?')
        || trimmed.lines().rev().take(3).any(|line| {
            line.trim_start()
                .to_ascii_lowercase()
                .starts_with("question:")
        })
}

/// A compact artifact from a relative path in agent text.
pub fn artifact_for_path(path: &str) -> AgentArtifact {
    AgentArtifact {
        kind: "file".into(),
        id: path.to_owned(),
        label: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_waits_for_input() {
        assert_eq!(
            status_from(
                &mm_llm::LlmResponse {
                    call_id: mm_llm::accounting::new_call_id(),
                    text: "Which project should I use?".into(),
                    model: "fake".into(),
                    usage: mm_llm::Usage::default(),
                    cached: false,
                    schema_valid: true,
                    decoder: mm_llm::DecoderKind::None,
                    tool_calls: Vec::new(),
                    finish_reason: Some("stop".into()),
                    provider_metadata: Default::default(),
                    streamed: false,
                },
                &[],
                "Which project should I use?"
            ),
            AgentRunStatus::WaitingInput
        );
    }

    #[test]
    fn tool_calls_keep_the_run_active_and_truncate_arguments() {
        let response = mm_llm::LlmResponse {
            call_id: mm_llm::accounting::new_call_id(),
            text: String::new(),
            model: "fake".into(),
            usage: mm_llm::Usage::default(),
            cached: false,
            schema_valid: true,
            decoder: mm_llm::DecoderKind::None,
            tool_calls: vec![mm_llm::ProviderToolCall {
                id: "call-1".into(),
                name: "read".into(),
                arguments: "x".repeat(900),
            }],
            finish_reason: Some("tool_calls".into()),
            provider_metadata: Default::default(),
            streamed: false,
        };
        let mapped = map_response(response);
        assert_eq!(mapped.status, AgentRunStatus::Running);
        assert_eq!(mapped.next.as_deref(), Some("continue"));
        assert_eq!(mapped.tool_calls[0].name, "read");
        assert_eq!(
            mapped.tool_calls[0]
                .arguments
                .as_deref()
                .unwrap()
                .chars()
                .count(),
            500
        );
    }

    #[test]
    fn mm_status_overrides_finish_reason() {
        let response = mm_llm::LlmResponse {
            call_id: mm_llm::accounting::new_call_id(),
            text: "Still working.".into(),
            model: "fake".into(),
            usage: mm_llm::Usage::default(),
            cached: false,
            schema_valid: true,
            decoder: mm_llm::DecoderKind::None,
            tool_calls: Vec::new(),
            finish_reason: Some("stop".into()),
            provider_metadata: [("mm_status".into(), Value::String("running".into()))].into(),
            streamed: true,
        };
        assert_eq!(map_response(response).status, AgentRunStatus::Running);
    }

    #[test]
    fn a_system_context_precedes_replayed_messages() {
        let request = AgentRequest {
            run_id: "run".into(),
            session_id: "session".into(),
            user_id: String::new(),
            input: "ignored when messages exist".into(),
            kind: crate::AgentKind::Hermes,
            mode: crate::AgentTurnMode::New,
            workspace: std::path::PathBuf::from("/tmp"),
            context: Some("system".into()),
            messages: vec![
                crate::AgentMessage {
                    role: "user".into(),
                    content: "first".into(),
                },
                crate::AgentMessage {
                    role: "assistant".into(),
                    content: "second".into(),
                },
            ],
            timeout_ms: 1_000,
        };
        let messages = build_messages(&request).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[1].content, "first");
        assert_eq!(messages[2].role, Role::Assistant);
    }
}
