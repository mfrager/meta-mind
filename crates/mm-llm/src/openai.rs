//! The OpenAI-compatible adapter.
//!
//! One adapter for every endpoint that speaks `/chat/completions`. It is the only
//! module in the substrate that knows a vendor's JSON, and it is deliberately thin:
//! it builds a request, hands it to a [`Transport`], and maps the answer back into
//! [`LlmResponse`]. It does not cache, route, retry, account, or validate — those
//! belong to the substrate, and keeping them out is what makes a second adapter a
//! small change rather than a second implementation of the whole contract.
//!
//! **The endpoint is never in source.** `OPENAI_BASE_URL` and `OPENAI_API_KEY` are
//! read at construction, and when either is unset the adapter is
//! [`LlmError::ProviderNotConfigured`] rather than pointed at a default host. A
//! hard-coded fallback would mean a misconfigured kernel silently sends prompts to
//! somebody else's server; a test scans this crate's sources to prove there is none.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::client::{DecoderKind, DeltaSink, LlmClient, LlmRequest, LlmResponse, SchemaId, Usage};
use crate::config::{LlmConfig, ProviderEnv, StructuredOutput};
use crate::error::{LlmError, Result};
use crate::replay::NetworkGuard;
use crate::schema::SchemaRegistry;

/// Receives server-sent event payloads from a streaming transport.
///
/// A named trait rather than `FnMut` keeps the transport object-safe and avoids
/// higher-ranked lifetime friction at the call boundary.
pub trait SseHandler: Send {
    /// Accept one `data:` payload.
    fn data(&mut self, data: &str);
}

/// How a request reaches an endpoint.
#[async_trait]
pub trait Transport: Send + Sync {
    /// POST `body` as JSON to `url` and return the parsed response body.
    async fn post_json(&self, url: &str, body: &Value) -> Result<Value>;

    /// POST `body` and report each server-sent event's `data:` payload.
    ///
    /// Returns `false` when the transport cannot stream, so the adapter can fall
    /// back to an ordinary call. A transport that starts a request and then
    /// discovers the endpoint answered with plain JSON must also return `false`
    /// rather than half-reporting.
    async fn post_sse(
        &self,
        _url: &str,
        _body: &Value,
        _handler: &mut dyn SseHandler,
    ) -> Result<bool> {
        Ok(false)
    }
}

/// The built-in transport: HTTP/1.1 over TCP, no TLS.
///
/// TLS is deliberately not implemented here. A kernel endpoint is expected to be a
/// local terminator (a sidecar proxy, a unix socket bridge); making the adapter
/// carry a TLS stack would put certificate handling on the substrate's critical
/// path for no benefit. An `https` endpoint is refused with a message that says
/// what to do instead, which is better than failing at handshake time with a
/// confusing error.
#[derive(Clone)]
pub struct TcpTransport {
    timeout_ms: u32,
    // The bearer token, if the endpoint wants one. Hand-written Debug below so it
    // never reaches a formatter.
    api_key: Option<String>,
}

impl std::fmt::Debug for TcpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TcpTransport")
            .field("timeout_ms", &self.timeout_ms)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl TcpTransport {
    /// A transport that gives up after `timeout_ms`.
    pub fn new(timeout_ms: u32) -> Self {
        TcpTransport {
            timeout_ms,
            api_key: None,
        }
    }

    /// Send `Authorization: Bearer <key>` with every request.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }
}

impl TcpTransport {
    /// Open a socket and write one request, returning the connected stream so the
    /// caller can read the answer incrementally.
    async fn send(&self, url: &str, body: &Value) -> Result<(tokio::net::TcpStream, String)> {
        let (scheme, host, port, path) = split_url(url)?;
        if scheme != "http" {
            return Err(LlmError::Transport(format!(
                "the built-in transport speaks plain HTTP only (got `{scheme}://`); \
                 terminate TLS in front of the kernel and point OPENAI_BASE_URL at it"
            )));
        }
        NetworkGuard::attempt(&host)?;

        let payload = serde_json::to_vec(body)
            .map_err(|e| LlmError::Config(format!("cannot encode the request body: {e}")))?;
        // A key carrying CR/LF would smuggle its own headers, so it is refused
        // rather than written into the request line block.
        let auth = match &self.api_key {
            Some(key) if key.contains(['\r', '\n']) => {
                return Err(LlmError::Config(
                    "OPENAI_API_KEY must not contain a newline".into(),
                ))
            }
            Some(key) => format!("Authorization: Bearer {key}\r\n"),
            None => String::new(),
        };
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
             Accept: text/event-stream\r\n\
             {auth}Content-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        let budget = Duration::from_millis(u64::from(self.timeout_ms.max(1)));

        let mut stream = tokio::time::timeout(
            budget,
            tokio::net::TcpStream::connect((host.as_str(), port)),
        )
        .await
        .map_err(|_| LlmError::Timeout(self.timeout_ms))?
        .map_err(|e| LlmError::Transport(format!("cannot connect to {host}:{port}: {e}")))?;

        tokio::time::timeout(budget, async {
            stream.write_all(head.as_bytes()).await?;
            stream.write_all(&payload).await?;
            stream.flush().await
        })
        .await
        .map_err(|_| LlmError::Timeout(self.timeout_ms))?
        .map_err(|e| LlmError::Transport(format!("cannot write the request: {e}")))?;

        Ok((stream, host))
    }

    /// Read one socket chunk. The timeout is per read, not per stream, because a
    /// long completion is allowed to take longer than one request timeout overall.
    async fn read_chunk(
        &self,
        stream: &mut tokio::net::TcpStream,
        buf: &mut [u8],
    ) -> Result<usize> {
        let budget = Duration::from_millis(u64::from(self.timeout_ms.max(1)));
        tokio::time::timeout(budget, stream.read(buf))
            .await
            .map_err(|_| LlmError::Timeout(self.timeout_ms))?
            .map_err(|e| LlmError::Transport(format!("cannot read the response: {e}")))
    }
}

#[async_trait]
impl Transport for TcpTransport {
    async fn post_json(&self, url: &str, body: &Value) -> Result<Value> {
        let (mut stream, host) = self.send(url, body).await?;
        let mut raw = Vec::new();
        let budget = Duration::from_millis(u64::from(self.timeout_ms.max(1)));
        tokio::time::timeout(budget, stream.read_to_end(&mut raw))
            .await
            .map_err(|_| LlmError::Timeout(self.timeout_ms))?
            .map_err(|e| LlmError::Transport(format!("cannot read a response from {host}: {e}")))?;
        parse_http_response(&raw)
    }

    async fn post_sse(
        &self,
        url: &str,
        body: &Value,
        on_data: &mut dyn SseHandler,
    ) -> Result<bool> {
        let (mut stream, _) = self.send(url, body).await?;
        let mut raw = Vec::new();
        let mut buf = [0u8; 8_192];

        // Headers first, so a non-2xx answer is an error rather than an empty
        // stream that looks like a model with nothing to say.
        let header_end = loop {
            if let Some(index) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                break index;
            }
            if raw.len() > 64 * 1024 {
                return Err(LlmError::Transport(
                    "malformed HTTP response: header block is too large".into(),
                ));
            }
            let n = self.read_chunk(&mut stream, &mut buf).await?;
            if n == 0 {
                return Err(LlmError::Transport(
                    "malformed HTTP response: no header terminator".into(),
                ));
            }
            raw.extend_from_slice(&buf[..n]);
        };
        let head = String::from_utf8_lossy(&raw[..header_end]).to_string();
        let status: u16 = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .ok_or_else(|| LlmError::Transport("malformed HTTP response: no status line".into()))?;
        let body_start = header_end + 4;
        let mut rest: Vec<u8> = raw.split_off(body_start);
        if !(200..300).contains(&status) {
            while let Ok(n) = self.read_chunk(&mut stream, &mut buf).await {
                if n == 0 {
                    break;
                }
                rest.extend_from_slice(&buf[..n]);
                if rest.len() > 4_096 {
                    break;
                }
            }
            return Err(LlmError::Transport(format!(
                "endpoint answered HTTP {status}: {}",
                String::from_utf8_lossy(&rest)
                    .chars()
                    .take(200)
                    .collect::<String>()
            )));
        }

        let chunked = head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked");
        // A JSON content type means the endpoint ignored `stream`; report that so
        // the adapter can make one ordinary call instead of parsing nothing.
        if !chunked
            && head
                .to_ascii_lowercase()
                .contains("content-type: application/json")
        {
            return Ok(false);
        }
        let length = content_length(&head);
        let mut parser = SseParser::default();
        let mut delivered = 0usize;
        let mut chunk_buffer = ChunkedDecoder::default();

        loop {
            if !rest.is_empty() {
                let bytes = if chunked {
                    let bytes = match chunk_buffer.push(&rest) {
                        Some(bytes) => bytes,
                        None => return Ok(false),
                    };
                    rest.clear();
                    bytes
                } else {
                    let take = length
                        .map(|length| length.saturating_sub(delivered).min(rest.len()))
                        .unwrap_or(rest.len());
                    delivered += take;
                    rest.drain(..take).collect()
                };
                parser.feed(&bytes, on_data);
                if chunk_buffer.finished || length.is_some_and(|length| delivered >= length) {
                    break;
                }
            }
            let n = self.read_chunk(&mut stream, &mut buf).await?;
            if n == 0 {
                break;
            }
            rest.extend_from_slice(&buf[..n]);
        }
        parser.flush(on_data);
        Ok(true)
    }
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

/// Incremental decoder for `Transfer-Encoding: chunked` bodies.
#[derive(Debug, Default)]
struct ChunkedDecoder {
    buffer: Vec<u8>,
    finished: bool,
}

impl ChunkedDecoder {
    /// Decode as much of `bytes` as forms complete chunks. `None` means the body
    /// was not chunked after all (or was malformed); the caller falls back.
    fn push(&mut self, bytes: &[u8]) -> Option<Vec<u8>> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            if self.finished {
                break;
            }
            let Some(line_end) = self.buffer.windows(2).position(|w| w == b"\r\n") else {
                break;
            };
            let size_text = String::from_utf8_lossy(&self.buffer[..line_end]).to_string();
            let size_text = size_text.split(';').next().unwrap_or("").trim();
            let Ok(size) = usize::from_str_radix(size_text, 16) else {
                return None;
            };
            if self.buffer.len() < line_end + 2 + size + 2 {
                break;
            }
            out.extend_from_slice(&self.buffer[line_end + 2..line_end + 2 + size]);
            self.buffer.drain(..line_end + 2 + size + 2);
            if size == 0 {
                self.finished = true;
                break;
            }
        }
        Some(out)
    }
}

/// Incremental `text/event-stream` parser. It only cares about `data:` lines,
/// which is all an OpenAI-compatible endpoint sends.
#[derive(Debug, Default)]
struct SseParser {
    line_buffer: Vec<u8>,
    data: String,
}

impl SseParser {
    fn feed(&mut self, bytes: &[u8], on_data: &mut dyn SseHandler) {
        self.line_buffer.extend_from_slice(bytes);
        while let Some(index) = self.line_buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.line_buffer.drain(..=index).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1])
                .trim_end_matches('\r')
                .to_string();
            if line.is_empty() {
                if !self.data.is_empty() {
                    let payload = std::mem::take(&mut self.data);
                    on_data.data(payload.trim_end_matches('\n'));
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                self.data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                self.data.push('\n');
            }
        }
    }

    fn flush(&mut self, on_data: &mut dyn SseHandler) {
        if !self.data.is_empty() {
            let payload = std::mem::take(&mut self.data);
            on_data.data(payload.trim_end_matches('\n'));
        }
    }
}

/// Accumulates an OpenAI-compatible stream into a final response while
/// forwarding each content delta to the caller's sink.
struct StreamAccumulator<'a> {
    sink: &'a mut dyn DeltaSink,
    text: &'a mut String,
    tool_calls: &'a mut BTreeMap<usize, (String, String, String)>,
    tokens_in: &'a mut u32,
    tokens_out: &'a mut u32,
    answered_model: &'a mut String,
    failure: &'a mut Option<LlmError>,
}

impl SseHandler for StreamAccumulator<'_> {
    fn data(&mut self, data: &str) {
        if data.trim() == "[DONE]" {
            return;
        }
        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(error) => {
                self.failure.get_or_insert_with(|| {
                    LlmError::Transport(format!("malformed SSE event: {error}"))
                });
                return;
            }
        };
        if let Some(name) = value.get("model").and_then(Value::as_str) {
            *self.answered_model = name.to_string();
        }
        if let Some(usage) = value.get("usage") {
            *self.tokens_in = usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(u64::from(*self.tokens_in)) as u32;
            *self.tokens_out = usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(u64::from(*self.tokens_out)) as u32;
        }
        let Some(choice) = value.get("choices").and_then(|c| c.get(0)) else {
            return;
        };
        let delta = match choice.get("delta") {
            Some(delta) => delta,
            None => match choice.get("message") {
                Some(message) => message,
                None => return,
            },
        };
        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                self.text.push_str(content);
                self.sink.delta(content);
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for (position, call) in calls.iter().enumerate() {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|index| index as usize)
                    .unwrap_or(position);
                let entry = self.tool_calls.entry(index).or_default();
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    entry.0 = id.to_string();
                }
                if let Some(function) = call.get("function") {
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        entry.1.push_str(name);
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                        entry.2.push_str(arguments);
                    }
                }
            }
        }
    }
}

fn split_url(url: &str) -> Result<(String, String, u16, String)> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| LlmError::Config(format!("endpoint `{url}` has no scheme")))?;
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host.to_string(),
            port.parse::<u16>()
                .map_err(|_| LlmError::Config(format!("endpoint `{url}` has a bad port")))?,
        ),
        None => (
            authority.to_string(),
            if scheme == "http" { 80 } else { 443 },
        ),
    };
    if host.is_empty() {
        return Err(LlmError::Config(format!("endpoint `{url}` has no host")));
    }
    Ok((scheme.to_string(), host, port, path))
}

fn parse_http_response(raw: &[u8]) -> Result<Value> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| {
            LlmError::Transport("malformed HTTP response: no header terminator".into())
        })?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let status: u16 = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| LlmError::Transport("malformed HTTP response: no status line".into()))?;
    let body = &raw[split + 4..];
    if !(200..300).contains(&status) {
        return Err(LlmError::Transport(format!(
            "endpoint answered HTTP {status}: {}",
            String::from_utf8_lossy(body)
                .chars()
                .take(200)
                .collect::<String>()
        )));
    }
    serde_json::from_slice(body)
        .map_err(|e| LlmError::Transport(format!("endpoint answered with a non-JSON body: {e}")))
}

/// The adapter.
pub struct OpenAiClient {
    // `env` holds a key, so the struct's Debug rendering below is hand-written.
    env: ProviderEnv,
    config: LlmConfig,
    schemas: RwLock<SchemaRegistry>,
    transport: Arc<dyn Transport>,
}

impl std::fmt::Debug for OpenAiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key never reaches a formatter, however the adapter is logged.
        f.debug_struct("OpenAiClient")
            .field("env", &self.env.redacted())
            .field("models", &self.config.models.len())
            .finish_non_exhaustive()
    }
}

impl OpenAiClient {
    /// Build from the environment, or report that no provider is configured.
    pub fn new(config: &LlmConfig) -> Result<Self> {
        Self::from_env(config, ProviderEnv::from_env())
    }

    /// Build from explicit values (so the decision is testable without a process
    /// environment).
    pub fn from_env(config: &LlmConfig, env: Option<ProviderEnv>) -> Result<Self> {
        let env = env.ok_or_else(|| {
            LlmError::ProviderNotConfigured(
                "OPENAI_BASE_URL and OPENAI_API_KEY must both be set".into(),
            )
        })?;
        let transport =
            TcpTransport::new(config.defaults.timeout_ms).with_api_key(env.api_key.clone());
        Ok(OpenAiClient {
            env,
            config: config.clone(),
            schemas: RwLock::new(SchemaRegistry::new()),
            transport: Arc::new(transport),
        })
    }

    /// The same adapter over a different transport (tests, and a deployment that
    /// terminates transport elsewhere).
    pub fn with_transport(
        config: &LlmConfig,
        env: ProviderEnv,
        transport: Arc<dyn Transport>,
    ) -> Self {
        OpenAiClient {
            env,
            config: config.clone(),
            schemas: RwLock::new(SchemaRegistry::new()),
            transport,
        }
    }

    /// Give the adapter the registered schemas, so it can ask for native JSON
    /// Schema output.
    pub fn with_schemas(self, schemas: SchemaRegistry) -> Self {
        *self.schemas.write().unwrap_or_else(|e| e.into_inner()) = schemas;
        self
    }

    /// The endpoint key, with the secret rendered out.
    pub fn env(&self) -> &ProviderEnv {
        &self.env
    }

    /// The URL a completion is posted to.
    pub fn endpoint(&self) -> String {
        format!(
            "{}/chat/completions",
            self.env.base_url.trim_end_matches('/')
        )
    }

    /// The request body for `req`, answered by `model`.
    pub fn request_body(&self, req: &LlmRequest, model: &str) -> Result<Value> {
        if req.schema.is_some() && !req.tools.is_empty() {
            return Err(LlmError::Config(
                "a request cannot carry both a schema and tools".into(),
            ));
        }
        let messages: Vec<Value> = req.messages.iter().map(message_json).collect();

        let mut body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
        });
        if !req.tools.is_empty() {
            body["tool_choice"] = json!("auto");
            body["tools"] = Value::Array(
                req.tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": tool.name,
                                "description": tool.description,
                                "parameters": tool.parameters,
                            }
                        })
                    })
                    .collect(),
            );
        }

        if let Some(schema_id) = &req.schema {
            match self.config.defaults.structured_output {
                StructuredOutput::JsonSchema => {
                    // The provider-native path of the invariant: ask for JSON Schema
                    // output when the endpoint cannot be constrained at decode time.
                    // The response is still validated afterwards — a provider that
                    // claims `strict` is a claim, not a guarantee.
                    let schema = self
                        .schemas
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(schema_id)
                        .cloned()
                        .ok_or_else(|| {
                            LlmError::Config(format!("schema `{schema_id}` is not registered"))
                        })?;
                    body["response_format"] = json!({
                        "type": "json_schema",
                        "json_schema": {
                            "name": schema_id.as_str(),
                            "strict": true,
                            "schema": schema,
                        }
                    });
                }
                StructuredOutput::JsonObject => {
                    // NIM-backed endpoints return an empty completion for
                    // `json_schema`; `json_object` is their supported mode. The
                    // service still validates the answer against the full schema.
                    body["response_format"] = json!({ "type": "json_object" });
                }
                StructuredOutput::None => {}
            }
        }
        Ok(body)
    }

    /// Map a provider answer into a [`LlmResponse`].
    pub fn parse_response(&self, value: &Value, requested_model: &str) -> Result<LlmResponse> {
        let choice = value
            .get("choices")
            .and_then(|c| c.get(0))
            .ok_or_else(|| LlmError::Transport("the endpoint returned no choices".into()))?;
        let message = choice.get("message").ok_or_else(|| {
            LlmError::Transport("the endpoint returned no message content".into())
        })?;
        let text = match message.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        let tool_calls = parse_tool_calls(message);
        if text.trim().is_empty() && tool_calls.is_empty() {
            let finish = choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let reasoning_chars = message
                .get("reasoning_content")
                .or_else(|| message.get("reasoning"))
                .and_then(Value::as_str)
                .map(str::chars)
                .map(Iterator::count)
                .unwrap_or(0);
            let detail = if reasoning_chars > 0 {
                format!(
                    "the endpoint returned {reasoning_chars} reasoning characters but no visible text; disable thinking or raise max_tokens"
                )
            } else {
                "the endpoint returned neither text nor tool calls".to_string()
            };
            return Err(LlmError::Transport(format!(
                "{detail} (finish_reason: {finish})"
            )));
        }
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(requested_model)
            .to_string();
        let tokens_in = usage_field(value, "prompt_tokens");
        let tokens_out = usage_field(value, "completion_tokens");
        let cost_micros = self
            .config
            .model(&model)
            .or_else(|| self.config.model(requested_model))
            .map(|spec| spec.cost_micros(tokens_in, tokens_out))
            .unwrap_or(0);

        Ok(LlmResponse {
            call_id: crate::accounting::new_call_id(),
            text,
            model,
            usage: Usage {
                tokens_in,
                tokens_out,
                cost_micros,
                latency_ms: 0,
            },
            cached: false,
            schema_valid: false,
            decoder: DecoderKind::ProviderNative,
            tool_calls,
        })
    }
}

fn message_json(message: &crate::client::Message) -> Value {
    let mut body = json!({
        "role": match message.role {
            crate::client::Role::System => "system",
            crate::client::Role::User => "user",
            crate::client::Role::Assistant => "assistant",
            crate::client::Role::Tool => "tool",
        },
        "content": message.content,
    });
    if let Some(name) = &message.name {
        body["name"] = json!(name);
    }
    if let Some(id) = &message.tool_call_id {
        body["tool_call_id"] = json!(id);
    }
    if !message.tool_calls.is_empty() {
        body["tool_calls"] = Value::Array(
            message
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": call.arguments,
                        }
                    })
                })
                .collect(),
        );
    }
    body
}

fn parse_tool_calls(message: &Value) -> Vec<crate::client::ProviderToolCall> {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let function = call.get("function")?;
                    let name = function.get("name")?.as_str()?.to_string();
                    let arguments = match function.get("arguments") {
                        Some(Value::String(text)) => text.clone(),
                        Some(other) => other.to_string(),
                        None => "{}".into(),
                    };
                    let id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("call")
                        .to_string();
                    Some(crate::client::ProviderToolCall {
                        id,
                        name,
                        arguments,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn usage_field(value: &Value, key: &str) -> u32 {
    value
        .get("usage")
        .and_then(|u| u.get(key))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        let model = req
            .model
            .clone()
            .or_else(|| self.config.defaults.default_model.clone())
            .ok_or_else(|| {
                LlmError::Config("no model was requested and no default_model is configured".into())
            })?;
        // The guard sits in front of the transport here *and* inside it, so a
        // replayed run cannot open a socket even if an adapter forgot.
        NetworkGuard::attempt(&self.env.base_url)?;
        let body = self.request_body(req, &model)?;
        let answer = self.transport.post_json(&self.endpoint(), &body).await?;
        self.parse_response(&answer, &model)
    }

    async fn complete_stream(
        &self,
        req: &LlmRequest,
        sink: &mut dyn DeltaSink,
    ) -> Result<LlmResponse> {
        // Structured output is validated as a whole and never shown while it is
        // half-written, so it does not stream.
        if req.schema.is_some() {
            return self.complete(req).await;
        }
        let model = req
            .model
            .clone()
            .or_else(|| self.config.defaults.default_model.clone())
            .ok_or_else(|| {
                LlmError::Config("no model was requested and no default_model is configured".into())
            })?;
        NetworkGuard::attempt(&self.env.base_url)?;
        let mut body = self.request_body(req, &model)?;
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });

        let mut text = String::new();
        let mut tool_calls: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
        let mut tokens_in = 0u32;
        let mut tokens_out = 0u32;
        let mut answered_model = model.clone();
        let mut failure: Option<LlmError> = None;

        let streamed = {
            let mut accumulator = StreamAccumulator {
                sink,
                text: &mut text,
                tool_calls: &mut tool_calls,
                tokens_in: &mut tokens_in,
                tokens_out: &mut tokens_out,
                answered_model: &mut answered_model,
                failure: &mut failure,
            };
            self.transport
                .post_sse(&self.endpoint(), &body, &mut accumulator)
                .await?
        };

        if !streamed {
            let response = self.complete(req).await?;
            if !response.text.is_empty() {
                sink.delta(&response.text);
            }
            return Ok(response);
        }
        if let Some(error) = failure {
            return Err(error);
        }

        let calls: Vec<crate::client::ProviderToolCall> = tool_calls
            .into_values()
            .filter(|(_, name, _)| !name.is_empty())
            .map(|(id, name, arguments)| crate::client::ProviderToolCall {
                id: if id.is_empty() { "call".into() } else { id },
                name,
                arguments: if arguments.is_empty() {
                    "{}".into()
                } else {
                    arguments
                },
            })
            .collect();
        // Some proxies ignore `stream` without changing the content type. One
        // ordinary call is the honest recovery; it still reports one delta.
        if text.is_empty() && calls.is_empty() {
            let response = self.complete(req).await?;
            if !response.text.is_empty() {
                sink.delta(&response.text);
            }
            return Ok(response);
        }

        let cost_micros = self
            .config
            .model(&answered_model)
            .or_else(|| self.config.model(&model))
            .map(|spec| spec.cost_micros(tokens_in, tokens_out))
            .unwrap_or(0);
        Ok(LlmResponse {
            call_id: crate::accounting::new_call_id(),
            text,
            model: answered_model,
            usage: Usage {
                tokens_in,
                tokens_out,
                cost_micros,
                latency_ms: 0,
            },
            cached: false,
            schema_valid: true,
            decoder: DecoderKind::ProviderNative,
            tool_calls: calls,
        })
    }

    fn provider(&self) -> &str {
        "openai"
    }

    fn decoder(&self) -> DecoderKind {
        // Never constrained: a hosted endpoint does not expose logits, so output is
        // requested as JSON Schema and validated afterwards.
        DecoderKind::ProviderNative
    }

    fn provide_schema(&self, id: SchemaId, schema: Value) -> Result<()> {
        let mut schemas = self.schemas.write().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = schemas.get(&id) {
            return if existing == &schema {
                Ok(())
            } else {
                Err(LlmError::Config(format!(
                    "schema `{id}` is already registered with a different definition"
                )))
            };
        }
        schemas.register(id, schema)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Message, Purpose, SchemaId};

    struct StubTransport {
        answer: Value,
        calls: std::sync::atomic::AtomicU64,
    }

    #[async_trait]
    impl Transport for StubTransport {
        async fn post_json(&self, _url: &str, _body: &Value) -> Result<Value> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.answer.clone())
        }
    }

    fn env() -> ProviderEnv {
        ProviderEnv::from_pairs(
            Some("http://127.0.0.1:8080/v1".into()),
            Some("test-key".into()),
        )
        .unwrap()
    }

    fn config() -> LlmConfig {
        LlmConfig::from_toml_str(
            r#"
[[model]]
name = "small"
input_cost_micros_per_1k = 150
output_cost_micros_per_1k = 600
typical_latency_ms = 400
complexity_ceiling = "medium"
"#,
        )
        .unwrap()
    }

    #[test]
    fn an_unconfigured_provider_is_reported_not_defaulted() {
        let err = OpenAiClient::from_env(&config(), None).unwrap_err();
        assert_eq!(err.kind(), "provider_not_configured");
        assert!(err.to_string().contains("OPENAI_BASE_URL"), "{err}");
    }

    #[test]
    fn the_endpoint_comes_from_the_environment() {
        let client = OpenAiClient::from_env(&config(), Some(env())).unwrap();
        assert_eq!(
            client.endpoint(),
            "http://127.0.0.1:8080/v1/chat/completions"
        );
        assert_eq!(client.provider(), "openai");
        assert_eq!(client.decoder(), DecoderKind::ProviderNative);
    }

    #[test]
    fn a_structured_request_asks_for_native_json_schema() {
        let mut schemas = SchemaRegistry::new();
        schemas
            .register(
                SchemaId::new("rating.v1"),
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["label"],
                    "properties": { "label": { "type": "string", "enum": ["low", "high"] } }
                }),
            )
            .unwrap();
        let client = OpenAiClient::from_env(&config(), Some(env()))
            .unwrap()
            .with_schemas(schemas);

        let req = LlmRequest::new(
            Purpose::Classify,
            vec![Message::system("be terse"), Message::user("rate this")],
        )
        .with_schema(SchemaId::new("rating.v1"))
        .with_model("small");
        let body = client.request_body(&req, "small").unwrap();
        assert_eq!(body["model"], "small");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "rate this");
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(body["response_format"]["json_schema"]["name"], "rating.v1");

        // An unregistered schema is a configuration error, not a silent omission.
        let unknown = LlmRequest::new(Purpose::Classify, vec![Message::user("x")])
            .with_schema(SchemaId::new("nope"));
        assert!(client.request_body(&unknown, "small").is_err());
    }

    #[test]
    fn json_object_mode_does_not_send_a_json_schema() {
        let mut cfg = config();
        cfg.defaults.structured_output = StructuredOutput::JsonObject;
        let client = OpenAiClient::from_env(&cfg, Some(env())).unwrap();
        let req = LlmRequest::new(Purpose::Classify, vec![Message::user("rate this")])
            .with_schema(SchemaId::new("rating.v1"))
            .with_model("small");
        let body = client.request_body(&req, "small").unwrap();
        assert_eq!(body["response_format"]["type"], "json_object");
        assert!(body["response_format"].get("json_schema").is_none());
    }

    #[test]
    fn a_schema_provided_after_construction_reaches_the_request() {
        let client = OpenAiClient::from_env(&config(), Some(env())).unwrap();
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["text"],
            "properties": { "text": { "type": "string" } }
        });
        client
            .provide_schema(SchemaId::new("controller.intro"), schema)
            .unwrap();

        let req = LlmRequest::new(Purpose::Converse, vec![Message::user("hello")])
            .with_schema(SchemaId::new("controller.intro"))
            .with_model("small");
        let body = client.request_body(&req, "small").unwrap();
        assert_eq!(
            body["response_format"]["json_schema"]["name"],
            "controller.intro"
        );
    }

    #[test]
    fn a_provider_answer_maps_onto_the_neutral_response_with_cost() {
        let client = OpenAiClient::from_env(&config(), Some(env())).unwrap();
        let answer = json!({
            "model": "small",
            "choices": [{ "message": { "role": "assistant", "content": "ok" } }],
            "usage": { "prompt_tokens": 1000, "completion_tokens": 1000 }
        });
        let response = client.parse_response(&answer, "small").unwrap();
        assert_eq!(response.text, "ok");
        assert_eq!(response.model, "small");
        assert_eq!(response.usage.tokens_in, 1000);
        assert_eq!(response.usage.tokens_out, 1000);
        assert_eq!(response.usage.cost_micros, 150 + 600);

        assert!(client.parse_response(&json!({}), "small").is_err());
        assert!(response.tool_calls.is_empty());
    }

    pub(super) fn tools_body() {
        let client = OpenAiClient::from_env(&config(), Some(env())).unwrap();
        let req = LlmRequest::new(Purpose::Converse, vec![Message::user("how do I use this?")])
            .with_model("small")
            .with_tools(vec![crate::client::ToolSchema {
                name: "guide.read".into(),
                description: "Read one guide chapter.".into(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id"],
                    "properties": { "id": { "type": "string" } }
                }),
            }]);
        let body = client.request_body(&req, "small").unwrap();
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "guide.read");
        assert_eq!(body["tool_choice"], "auto");
        assert!(body.get("response_format").is_none());

        let mixed = req.clone().with_schema(SchemaId::new("controller.intro"));
        assert!(client.request_body(&mixed, "small").is_err());

        let answer = json!({
            "model": "small",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": { "name": "guide.read", "arguments": "{\"id\":\"start\"}" }
                    }]
                }
            }]
        });
        let response = client.parse_response(&answer, "small").unwrap();
        assert!(response.text.is_empty());
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "guide.read");
        assert_eq!(response.tool_calls[0].arguments, "{\"id\":\"start\"}");
    }

    struct SseTransport {
        events: Vec<String>,
        streamed: bool,
    }

    #[async_trait]
    impl Transport for SseTransport {
        async fn post_json(&self, _url: &str, _body: &Value) -> Result<Value> {
            Ok(json!({"choices":[{"message":{"content":"fallback"}}]}))
        }

        async fn post_sse(
            &self,
            _url: &str,
            _body: &Value,
            handler: &mut dyn SseHandler,
        ) -> Result<bool> {
            for event in &self.events {
                handler.data(event);
            }
            Ok(self.streamed)
        }
    }

    #[tokio::test]
    async fn a_stream_is_assembled_and_forwarded_delta_by_delta() {
        use crate::client::CollectSink;

        let transport = Arc::new(SseTransport {
            streamed: true,
            events: vec![
                json!({"model":"small","choices":[{"delta":{"content":"Hel"}}]}).to_string(),
                json!({"choices":[{"delta":{"content":"lo"}}]}).to_string(),
                json!({
                    "choices": [],
                    "usage": {"prompt_tokens": 7, "completion_tokens": 2}
                })
                .to_string(),
                "[DONE]".into(),
            ],
        });
        let client = OpenAiClient::with_transport(&config(), env(), transport);
        let req = LlmRequest::new(Purpose::Converse, vec![Message::user("hi")]).with_model("small");
        let mut sink = CollectSink::default();
        let response = client.complete_stream(&req, &mut sink).await.unwrap();

        assert_eq!(response.text, "Hello");
        assert_eq!(sink.0, "Hello");
        assert_eq!(response.usage.tokens_in, 7);
        assert_eq!(response.usage.tokens_out, 2);
        assert!(response.schema_valid);
    }

    #[tokio::test]
    async fn accumulated_tool_call_fragments_become_one_call() {
        use crate::client::CollectSink;

        let transport = Arc::new(SseTransport {
            streamed: true,
            events: vec![
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"data.","arguments":"{\"q\":"}}]}}]}).to_string(),
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"search","arguments":"\"x\"}"}}]}}]}).to_string(),
            ],
        });
        let client = OpenAiClient::with_transport(&config(), env(), transport);
        let req = LlmRequest::new(Purpose::Converse, vec![Message::user("hi")]).with_model("small");
        let mut sink = CollectSink::default();
        let response = client.complete_stream(&req, &mut sink).await.unwrap();

        assert_eq!(sink.0, "");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "data.search");
        assert_eq!(response.tool_calls[0].arguments, "{\"q\":\"x\"}");
    }

    #[tokio::test]
    async fn a_transport_without_streaming_falls_back_to_one_delta() {
        use crate::client::CollectSink;

        let transport = Arc::new(StubTransport {
            answer: json!({"choices":[{"message":{"content":"plain"}}]}),
            calls: std::sync::atomic::AtomicU64::new(0),
        });
        let client = OpenAiClient::with_transport(&config(), env(), transport);
        let req = LlmRequest::new(Purpose::Converse, vec![Message::user("hi")]).with_model("small");
        let mut sink = CollectSink::default();
        let response = client.complete_stream(&req, &mut sink).await.unwrap();

        assert_eq!(response.text, "plain");
        assert_eq!(sink.0, "plain");
    }

    #[test]
    fn the_sse_parser_keeps_partial_lines_until_they_complete() {
        #[derive(Default)]
        struct Collect(Vec<String>);
        impl SseHandler for Collect {
            fn data(&mut self, data: &str) {
                self.0.push(data.to_string());
            }
        }
        let mut parser = SseParser::default();
        let mut collected = Collect::default();
        parser.feed(b"data: {\"a\":", &mut collected);
        assert!(collected.0.is_empty());
        parser.feed(b"1}\n\ndata: [DONE]\n\n", &mut collected);
        assert_eq!(collected.0, vec!["{\"a\":1}", "[DONE]"]);
    }

    #[tokio::test]
    async fn an_armed_network_guard_stops_the_adapter_before_the_transport() {
        let transport = Arc::new(StubTransport {
            answer: json!({"choices":[{"message":{"content":"x"}}]}),
            calls: std::sync::atomic::AtomicU64::new(0),
        });
        let client = OpenAiClient::with_transport(&config(), env(), transport.clone());
        let req = LlmRequest::new(Purpose::Plan, vec![Message::user("hi")]).with_model("small");

        {
            let _permit = NetworkGuard::arm();
            let err = client.complete(&req).await.unwrap_err();
            assert_eq!(err.kind(), "network_forbidden");
        }
        assert_eq!(
            transport.calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the transport must not be reached while the guard is armed"
        );
        assert!(client.complete(&req).await.is_ok());
    }

    #[tokio::test]
    async fn the_configured_key_is_sent_as_a_bearer_header() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let body = r#"{"choices":[{"message":{"content":"ok"}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });

        let env =
            ProviderEnv::from_pairs(Some(format!("http://{addr}/v1")), Some("sk-test".into()))
                .unwrap();
        let client = OpenAiClient::from_env(&config(), Some(env)).unwrap();
        let req = LlmRequest::new(Purpose::Converse, vec![Message::user("hi")]).with_model("small");
        assert_eq!(client.complete(&req).await.unwrap().text, "ok");

        let seen = server.await.unwrap();
        assert!(
            seen.contains("Authorization: Bearer sk-test\r\n"),
            "the key must go on the wire: {seen}"
        );
    }

    #[test]
    fn urls_are_split_without_a_url_crate() {
        assert_eq!(
            split_url("http://127.0.0.1:9/v1").unwrap(),
            ("http".into(), "127.0.0.1".into(), 9, "/v1".into())
        );
        assert_eq!(
            split_url("https://example.invalid").unwrap(),
            ("https".into(), "example.invalid".into(), 443, "/".into())
        );
        assert!(split_url("no-scheme").is_err());
        assert!(split_url("http://host:not-a-port/").is_err());
    }
}

#[cfg(test)]
mod tools_body {
    #[test]
    fn tools_body() {
        super::tests::tools_body();
    }
}
