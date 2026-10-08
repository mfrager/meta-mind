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

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::client::{DecoderKind, LlmClient, LlmRequest, LlmResponse, Usage};
use crate::config::{LlmConfig, ProviderEnv};
use crate::error::{LlmError, Result};
use crate::replay::NetworkGuard;
use crate::schema::SchemaRegistry;

/// How a request reaches an endpoint.
#[async_trait]
pub trait Transport: Send + Sync {
    /// POST `body` as JSON to `url` and return the parsed response body.
    async fn post_json(&self, url: &str, body: &Value) -> Result<Value>;
}

/// The built-in transport: HTTP/1.1 over TCP, no TLS.
///
/// TLS is deliberately not implemented here. A kernel endpoint is expected to be a
/// local terminator (a sidecar proxy, a unix socket bridge); making the adapter
/// carry a TLS stack would put certificate handling on the substrate's critical
/// path for no benefit. An `https` endpoint is refused with a message that says
/// what to do instead, which is better than failing at handshake time with a
/// confusing error.
#[derive(Debug, Clone, Copy)]
pub struct TcpTransport {
    timeout_ms: u32,
}

impl TcpTransport {
    /// A transport that gives up after `timeout_ms`.
    pub fn new(timeout_ms: u32) -> Self {
        TcpTransport { timeout_ms }
    }
}

#[async_trait]
impl Transport for TcpTransport {
    async fn post_json(&self, url: &str, body: &Value) -> Result<Value> {
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
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
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

        let mut raw = Vec::new();
        tokio::time::timeout(budget, stream.read_to_end(&mut raw))
            .await
            .map_err(|_| LlmError::Timeout(self.timeout_ms))?
            .map_err(|e| LlmError::Transport(format!("cannot read the response: {e}")))?;

        parse_http_response(&raw)
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
    schemas: SchemaRegistry,
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
        Ok(OpenAiClient {
            env,
            config: config.clone(),
            schemas: SchemaRegistry::new(),
            transport: Arc::new(TcpTransport::new(config.defaults.timeout_ms)),
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
            schemas: SchemaRegistry::new(),
            transport,
        }
    }

    /// Give the adapter the registered schemas, so it can ask for native JSON
    /// Schema output.
    pub fn with_schemas(mut self, schemas: SchemaRegistry) -> Self {
        self.schemas = schemas;
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
        let messages: Vec<Value> = req
            .messages
            .iter()
            .map(|m| {
                json!({
                    "role": match m.role {
                        crate::client::Role::System => "system",
                        crate::client::Role::User => "user",
                        crate::client::Role::Assistant => "assistant",
                    },
                    "content": m.content,
                })
            })
            .collect();

        let mut body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
        });

        if let Some(schema_id) = &req.schema {
            // The provider-native path of the invariant: ask for JSON Schema output
            // when the endpoint cannot be constrained at decode time. The
            // response is still validated afterwards — a provider that claims
            // `strict` is a claim, not a guarantee.
            let schema = self.schemas.get(schema_id).ok_or_else(|| {
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
        Ok(body)
    }

    /// Map a provider answer into a [`LlmResponse`].
    pub fn parse_response(&self, value: &Value, requested_model: &str) -> Result<LlmResponse> {
        let choice = value
            .get("choices")
            .and_then(|c| c.get(0))
            .ok_or_else(|| LlmError::Transport("the endpoint returned no choices".into()))?;
        let text = choice
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .ok_or_else(|| LlmError::Transport("the endpoint returned no message content".into()))?
            .to_string();
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
        })
    }
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

    fn provider(&self) -> &str {
        "openai"
    }

    fn decoder(&self) -> DecoderKind {
        // Never constrained: a hosted endpoint does not expose logits, so output is
        // requested as JSON Schema and validated afterwards.
        DecoderKind::ProviderNative
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
