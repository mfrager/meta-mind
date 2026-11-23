//! Hermes: a dedicated OpenAI-compatible agent client.
//!
//! This is deliberately separate from `mm-llm::OpenAiClient`. Agent turns have
//! arbitrary tool payloads, long timeouts, and an SSE-or-JSON transport choice;
//! the substrate is for typed metamind calls. The client supports `http://`
//! endpoints only, matching the local Hermes deployment, and never logs the API
//! key.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

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
#[derive(Debug)]
pub struct HermesBackend {
    config: Option<HermesConfig>,
    probe: Mutex<HermesProbe>,
}

impl HermesBackend {
    /// A backend for an explicit configuration.
    pub fn new(config: HermesConfig) -> Self {
        Self {
            config: Some(config),
            probe: Mutex::new(HermesProbe {
                available: false,
                reason: Some("not probed".into()),
                models: Vec::new(),
            }),
        }
    }

    /// Resolve the backend from the environment or a config file.
    pub fn from_env() -> Self {
        Self::from_config_file(Some(std::path::Path::new("config/agents.toml")))
    }

    /// Resolve the backend from an explicit config file.
    pub fn from_config_file(path: Option<&std::path::Path>) -> Self {
        let config = HermesConfig::from_env_or_file(path);
        let reason = if config.is_some() {
            Some("not probed".into())
        } else {
            Some(format!("{BASE_URL_ENV} is unset"))
        };
        Self {
            config,
            probe: Mutex::new(HermesProbe {
                available: false,
                reason,
                models: Vec::new(),
            }),
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

    /// Probe `GET /models` and record availability.
    pub async fn refresh(&self) -> HermesProbe {
        let Some(config) = &self.config else {
            return self.probe();
        };
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking({
                let config = config.clone();
                move || fetch_models(&config)
            }),
        )
        .await;
        let probe = match result {
            Ok(Ok(Ok(models))) => HermesProbe {
                available: true,
                reason: None,
                models,
            },
            Ok(Ok(Err(error))) => HermesProbe {
                available: false,
                reason: Some(error),
                models: Vec::new(),
            },
            Ok(Err(error)) => HermesProbe {
                available: false,
                reason: Some(format!("probe failed: {error}")),
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
        let body = json!({
            "model": model,
            "messages": messages,
            "stream": config.stream,
            "temperature": 0.2,
        });
        let config_for_call = config.clone();
        let result = tokio::time::timeout(
            Duration::from_millis(timeout),
            tokio::task::spawn_blocking(move || {
                post_completion(&config_for_call, &body, config_for_call.stream)
            }),
        )
        .await
        .map_err(|_| AgentError::Timeout(timeout))?
        .map_err(|error| AgentError::Other(format!("Hermes task failed: {error}")))?;
        result?.ok_or_else(|| AgentError::Other("empty Hermes response".into()))
    }
}

fn build_messages(request: &AgentRequest) -> Result<Vec<Value>> {
    let mut messages = Vec::new();
    if let Some(system) = &request.context {
        if !system.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": system }));
        }
    }
    if request.messages.is_empty() {
        messages.push(json!({ "role": "user", "content": request.input }));
    } else {
        for message in &request.messages {
            messages.push(json!({ "role": message.role, "content": message.content }));
        }
    }
    Ok(messages)
}

fn fetch_models(config: &HermesConfig) -> std::result::Result<Vec<String>, String> {
    let response = http_request(config, "GET", "/models", None, false)
        .map_err(|error| format!("{}: {error}", config.base_url))?;
    if response.status != 200 {
        return Err(format!(
            "GET {}/models returned HTTP {}",
            config.base_url.trim_end_matches('/'),
            response.status
        ));
    }
    let value: Value = serde_json::from_slice(&response.body)
        .map_err(|error| format!("invalid /models JSON: {error}"))?;
    Ok(value
        .get("data")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("id").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default())
}

fn post_completion(
    config: &HermesConfig,
    body: &Value,
    stream: bool,
) -> std::result::Result<Option<AgentTurnResult>, AgentError> {
    let response = http_request(
        config,
        "POST",
        "/chat/completions",
        Some(body.to_string().as_bytes()),
        stream,
    )
    .map_err(|error| AgentError::Unavailable(format!("{}: {error}", config.base_url)))?;
    if !(200..300).contains(&response.status) {
        return Err(AgentError::Rejected(format!(
            "Hermes returned HTTP {}: {}",
            response.status,
            redact_excerpt(&response.body)
        )));
    }
    let text = String::from_utf8_lossy(&response.body);
    if !stream || !looks_like_sse(&text) {
        return parse_completion_json(&text, response.model, false, config.model.as_deref());
    }
    parse_sse(&text, response.model, config.model.as_deref())
}

fn parse_completion_json(
    text: &str,
    header_model: Option<String>,
    streamed: bool,
    configured_model: Option<&str>,
) -> Result<Option<AgentTurnResult>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| AgentError::Rejected(format!("invalid Hermes JSON: {error}")))?;
    if let Some(error) = value.get("error") {
        return Err(AgentError::Rejected(
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Hermes returned an error")
                .to_owned(),
        ));
    }
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| AgentError::Rejected("Hermes response has no choices".into()))?;
    let message = choice
        .get("message")
        .ok_or_else(|| AgentError::Rejected("Hermes choice has no message".into()))?;
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let tool_calls = parse_tool_calls(message.get("tool_calls"));
    let finish = choice.get("finish_reason").and_then(Value::as_str);
    let status = status_from(&value, finish, &tool_calls, &content);
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(header_model)
        .or_else(|| configured_model.map(str::to_owned));
    Ok(Some(AgentTurnResult {
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
        usage: parse_usage(value.get("usage")),
        streamed,
        model,
        raw_ref: None,
    }))
}

fn parse_sse(
    text: &str,
    header_model: Option<String>,
    configured_model: Option<&str>,
) -> Result<Option<AgentTurnResult>> {
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    let mut finish = None;
    let mut status_marker = None;
    let mut usage = AgentUsage::default();
    let mut model = header_model;
    let mut saw_event = false;
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        saw_event = true;
        if model.is_none() {
            model = value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if let Some(marker) = value.get("mm_status").and_then(Value::as_str) {
            status_marker = Some(marker.to_owned());
        }
        if let Some(event_usage) = value.get("usage") {
            let parsed = parse_usage(Some(event_usage));
            if parsed != AgentUsage::default() {
                usage = parsed;
            }
        }
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            continue;
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(chunk) = delta.get("content").and_then(Value::as_str) {
                content.push_str(chunk);
            }
            for call in parse_tool_calls(delta.get("tool_calls")) {
                if !tool_calls
                    .iter()
                    .any(|known: &AgentToolSummary| known.name == call.name)
                {
                    tool_calls.push(call);
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            finish = Some(reason.to_owned());
        }
    }
    if !saw_event {
        return parse_completion_json(text, model, true, configured_model);
    }
    let status = status_marker
        .as_deref()
        .and_then(parse_status)
        .unwrap_or_else(|| status_from(&Value::Null, finish.as_deref(), &tool_calls, &content));
    Ok(Some(AgentTurnResult {
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
        usage,
        streamed: true,
        model: model.or_else(|| configured_model.map(str::to_owned)),
        raw_ref: None,
    }))
}

fn status_from(
    value: &Value,
    finish: Option<&str>,
    tool_calls: &[AgentToolSummary],
    content: &str,
) -> AgentRunStatus {
    if let Some(status) = value.get("mm_status").and_then(Value::as_str) {
        if let Some(status) = parse_status(status) {
            return status;
        }
    }
    if matches!(finish, Some("tool_calls" | "length")) || !tool_calls.is_empty() {
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

fn parse_tool_calls(value: Option<&Value>) -> Vec<AgentToolSummary> {
    value
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let function = call.get("function")?;
                    let name = function.get("name").and_then(Value::as_str)?;
                    let arguments = function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .map(|arguments| arguments.chars().take(500).collect());
                    Some(AgentToolSummary {
                        name: name.to_owned(),
                        arguments,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_usage(value: Option<&Value>) -> AgentUsage {
    let Some(value) = value else {
        return AgentUsage::default();
    };
    AgentUsage {
        prompt_tokens: value
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion_tokens: value
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

fn looks_like_sse(text: &str) -> bool {
    text.lines().any(|line| line.starts_with("data:"))
}

fn redact_excerpt(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    mm_llm::redact::excerpt(&text, 300)
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
    model: Option<String>,
}

fn http_request(
    config: &HermesConfig,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    stream: bool,
) -> std::result::Result<HttpResponse, String> {
    let target = split_url(&config.base_url)?;
    let mut stream_tcp = TcpStream::connect((target.host.as_str(), target.port))
        .map_err(|error| error.to_string())?;
    stream_tcp
        .set_read_timeout(Some(Duration::from_millis(config.timeout_ms.max(1_000))))
        .map_err(|error| error.to_string())?;
    stream_tcp
        .set_write_timeout(Some(Duration::from_millis(config.timeout_ms.max(1_000))))
        .map_err(|error| error.to_string())?;
    let path = format!("{}{}", target.path.trim_end_matches('/'), path);
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nAccept: {}\r\nContent-Type: application/json\r\nConnection: close\r\n",
        target.host_header,
        if stream {
            "text/event-stream"
        } else {
            "application/json"
        }
    );
    if let Some(key) = &config.api_key {
        request.push_str(&format!("Authorization: Bearer {key}\r\n"));
    }
    if let Some(body) = body {
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    stream_tcp
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    if let Some(body) = body {
        stream_tcp
            .write_all(body)
            .map_err(|error| error.to_string())?;
    }
    stream_tcp.flush().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream_tcp);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .map_err(|error| error.to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| format!("invalid HTTP status line: {}", status_line.trim()))?;
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        headers.push(line.trim_end().to_ascii_lowercase());
    }
    let model = headers
        .iter()
        .find_map(|line| line.strip_prefix("x-mm-agent-model:"))
        .map(str::trim)
        .map(str::to_owned);
    let chunked = headers
        .iter()
        .any(|line| line.starts_with("transfer-encoding:") && line.contains("chunked"));
    let content_length = headers
        .iter()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok());
    let body = if chunked {
        read_chunked(&mut reader)?
    } else if let Some(length) = content_length {
        let mut body = vec![0u8; length];
        reader
            .read_exact(&mut body)
            .map_err(|error| error.to_string())?;
        body
    } else {
        let mut body = Vec::new();
        reader
            .read_to_end(&mut body)
            .map_err(|error| error.to_string())?;
        body
    };
    Ok(HttpResponse {
        status,
        body,
        model,
    })
}

fn read_chunked(reader: &mut BufReader<TcpStream>) -> std::result::Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader
            .read_line(&mut size_line)
            .map_err(|error| error.to_string())?;
        let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("0"), 16)
            .map_err(|error| format!("invalid chunk size: {error}"))?;
        if size == 0 {
            break;
        }
        let mut chunk = vec![0u8; size];
        reader
            .read_exact(&mut chunk)
            .map_err(|error| error.to_string())?;
        body.extend_from_slice(&chunk);
        let mut crlf = [0u8; 2];
        reader
            .read_exact(&mut crlf)
            .map_err(|error| error.to_string())?;
    }
    Ok(body)
}

struct UrlTarget {
    host: String,
    host_header: String,
    port: u16,
    path: String,
}

fn split_url(base: &str) -> std::result::Result<UrlTarget, String> {
    let Some(rest) = base.strip_prefix("http://") else {
        if base.starts_with("https://") {
            return Err("https Hermes endpoints are not supported by the local client".into());
        }
        return Err(format!("invalid Hermes base URL: {base}"));
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err("Hermes base URL has no host".into());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host.to_owned(),
            port.parse::<u16>()
                .map_err(|error| format!("invalid Hermes port: {error}"))?,
        ),
        None => (authority.to_owned(), 80),
    };
    Ok(UrlTarget {
        host_header: authority.to_owned(),
        host,
        port,
        path: path.to_owned(),
    })
}

/// Extract authenticated models from a `/models` body. Tests use it directly.
pub fn models_from_json(body: &[u8]) -> Vec<String> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value.get("data").and_then(Value::as_array).map(|models| {
                models
                    .iter()
                    .filter_map(|model| model.get("id").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect()
            })
        })
        .unwrap_or_default()
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
    fn sse_content_is_accumulated_and_done_is_terminal() {
        let body = concat!(
            "data: {\"model\":\"fake\",\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let result = parse_sse(body, None, Some("fake")).unwrap().unwrap();
        assert_eq!(result.message, "hello");
        assert_eq!(result.status, AgentRunStatus::Done);
        assert!(result.streamed);
    }

    #[test]
    fn a_question_waits_for_input() {
        let body = json!({
            "choices": [{ "message": { "content": "Which project should I use?" } }]
        });
        let result = parse_completion_json(&body.to_string(), None, false, None)
            .unwrap()
            .unwrap();
        assert_eq!(result.status, AgentRunStatus::WaitingInput);
        assert_eq!(result.next.as_deref(), Some("wait_for_input"));
    }

    #[test]
    fn urls_are_split_without_a_url_crate() {
        let target = split_url("http://127.0.0.1:9000/v1").unwrap();
        assert_eq!(target.host, "127.0.0.1");
        assert_eq!(target.port, 9000);
        assert_eq!(target.path, "/v1");
        assert!(split_url("https://example.test/v1").is_err());
    }
}
