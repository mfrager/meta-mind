//! The provider-neutral request, response, and client trait.
//!
//! These are the only types a caller of the substrate sees. Every provider is
//! reduced to this shape by an adapter, so nothing above this layer can depend on
//! a vendor's JSON, and swapping providers cannot change a caller.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::error::Result;

/// Why a call is being made.
///
/// A `Purpose` is not a label for the log: it selects the model, decides whether
/// the semantic cache is allowed, and is the axis `llm stats` aggregates on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// Turn input into an interpretation.
    Interpret,
    /// Produce a plan.
    Plan,
    /// Pull structured facts out of text.
    Extract,
    /// Critique an artifact.
    Critique,
    /// Condense text.
    Summarize,
    /// Review code.
    CodeReview,
    /// Assign a label.
    Classify,
    /// Explain a failure.
    Diagnose,
    /// Talk with the operator as the being. Not a semantic-cache purpose.
    Converse,
    /// A downstream agent turn. Pinned to the agent provider profile and never
    /// served by the semantic cache.
    Agent,
}

impl Purpose {
    /// Every purpose, in a stable order.
    pub const ALL: [Purpose; 10] = [
        Purpose::Interpret,
        Purpose::Plan,
        Purpose::Extract,
        Purpose::Critique,
        Purpose::Summarize,
        Purpose::CodeReview,
        Purpose::Classify,
        Purpose::Diagnose,
        Purpose::Converse,
        Purpose::Agent,
    ];

    /// The stored form (`code_review`, not `codereview`).
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::Interpret => "interpret",
            Purpose::Plan => "plan",
            Purpose::Extract => "extract",
            Purpose::Critique => "critique",
            Purpose::Summarize => "summarize",
            Purpose::CodeReview => "code_review",
            Purpose::Classify => "classify",
            Purpose::Diagnose => "diagnose",
            Purpose::Converse => "converse",
            Purpose::Agent => "agent",
        }
    }

    /// Parse the stored form.
    pub fn parse(s: &str) -> Option<Self> {
        Purpose::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

/// How the cache is consulted for a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    /// Consult and populate the cache.
    #[default]
    Use,
    /// Ignore the cache entirely (still records the call).
    Bypass,
    /// Ignore any hit and overwrite the entry.
    Refresh,
    /// Serve only recorded responses; a miss is an error, never a request.
    Replay,
}

/// Who wrote a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Instructions that frame the call.
    System,
    /// The caller's input.
    User,
    /// A previous model turn.
    Assistant,
    /// A tool result fed back into the same turn.
    Tool,
}

/// A function the provider may call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    /// The tool name (`memory.recall`).
    pub name: String,
    /// One line the model sees.
    pub description: String,
    /// Draft 2020-12 parameters object.
    pub parameters: serde_json::Value,
}

/// One tool call on an assistant message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolCall {
    /// The provider's call id, echoed on the tool result.
    pub id: String,
    /// The tool name.
    pub name: String,
    /// Arguments as a JSON string, the shape OpenAI-compatible servers send.
    pub arguments: String,
}

/// One message in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Who wrote it.
    pub role: Role,
    /// The content.
    pub content: String,
    /// Tool name, on a tool result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The call this result answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool calls proposed by an assistant message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ProviderToolCall>,
}

impl Message {
    /// A system message.
    pub fn system(content: impl Into<String>) -> Self {
        Message {
            role: Role::System,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    /// A user message.
    pub fn user(content: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    /// An assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Message {
            role: Role::Assistant,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    /// An assistant message that proposes tool calls.
    pub fn assistant_calls(content: impl Into<String>, calls: Vec<ProviderToolCall>) -> Self {
        Message {
            role: Role::Assistant,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: calls,
        }
    }

    /// A tool result.
    pub fn tool(
        id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Message {
            role: Role::Tool,
            content: content.into(),
            name: Some(name.into()),
            tool_call_id: Some(id.into()),
            tool_calls: Vec::new(),
        }
    }
}

/// The name of a registered strict schema.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SchemaId(String);

impl SchemaId {
    /// Name a schema.
    pub fn new(name: impl Into<String>) -> Self {
        SchemaId(name.into())
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SchemaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which decoder produced (or was asked to produce) the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecoderKind {
    /// Token-level constrained decoding via llguidance.
    Llguidance,
    /// Token-level constrained decoding via XGrammar.
    XGrammar,
    /// The provider was asked for JSON Schema output natively.
    ProviderNative,
    /// No constraint: the output is validated after the fact.
    #[default]
    None,
}

impl DecoderKind {
    /// Every decoder, in preference order (most constraining first).
    pub const ALL: [DecoderKind; 4] = [
        DecoderKind::Llguidance,
        DecoderKind::XGrammar,
        DecoderKind::ProviderNative,
        DecoderKind::None,
    ];

    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            DecoderKind::Llguidance => "llguidance",
            DecoderKind::XGrammar => "xgrammar",
            DecoderKind::ProviderNative => "provider_native",
            DecoderKind::None => "none",
        }
    }

    /// Parse the stored form.
    pub fn parse(s: &str) -> Option<Self> {
        DecoderKind::ALL.into_iter().find(|d| d.as_str() == s)
    }

    /// True when the decoder makes the output schema-valid by construction.
    pub fn is_constrained(self) -> bool {
        matches!(self, DecoderKind::Llguidance | DecoderKind::XGrammar)
    }
}

/// What one call cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt tokens billed.
    pub tokens_in: u32,
    /// Completion tokens billed.
    pub tokens_out: u32,
    /// Cost in millionths of a currency unit.
    pub cost_micros: i64,
    /// Wall-clock latency.
    pub latency_ms: u32,
}

/// The completion ceiling a builder uses until the service applies its config.
pub const DEFAULT_MAX_TOKENS: u32 = 1024;

/// A request to the substrate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    /// Why the call is being made.
    pub purpose: Purpose,
    /// The model to use, or `None` to let the router choose.
    pub model: Option<String>,
    /// The conversation.
    pub messages: Vec<Message>,
    /// The schema the output must satisfy, when it is a structured call.
    pub schema: Option<SchemaId>,
    /// Completion token ceiling.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// How the cache is consulted.
    pub cache: CacheMode,
    /// The trace this call belongs to.
    pub trace_id: Option<Ulid>,
    /// Functions the provider may call. Empty on a schema request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSchema>,
    /// Provider profile name. `None` means the default profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Per-call wall-clock ceiling. `None` uses the profile's timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Stable call identifier assigned by the agentstream gateway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<Ulid>,
}

impl LlmRequest {
    /// A request with the substrate's defaults for purpose `purpose`.
    pub fn new(purpose: Purpose, messages: Vec<Message>) -> Self {
        LlmRequest {
            purpose,
            model: None,
            messages,
            schema: None,
            max_tokens: DEFAULT_MAX_TOKENS,
            temperature: 0.0,
            cache: CacheMode::Use,
            trace_id: None,
            tools: Vec::new(),
            provider: None,
            timeout_ms: None,
            call_id: None,
        }
    }

    /// Require structured output for `schema`.
    pub fn with_schema(mut self, schema: SchemaId) -> Self {
        self.schema = Some(schema);
        self
    }

    /// Pin the model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the cache mode.
    pub fn with_cache(mut self, cache: CacheMode) -> Self {
        self.cache = cache;
        self
    }

    /// Attach a trace id.
    pub fn with_trace(mut self, trace_id: Ulid) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Offer these tools. A request cannot also carry a schema.
    pub fn with_tools(mut self, tools: Vec<ToolSchema>) -> Self {
        self.tools = tools;
        self
    }

    /// Select a named provider profile.
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Set a per-call wall-clock ceiling.
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Attach the call id the gateway and ledger will share.
    pub fn with_call_id(mut self, call_id: Ulid) -> Self {
        self.call_id = Some(call_id);
        self
    }

    /// A bounded excerpt of the last user message, for redacted logs.
    pub fn user_excerpt(&self, max_chars: usize) -> String {
        let last = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .unwrap_or("");
        crate::redact::excerpt(last, max_chars)
    }
}

/// The answer to a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmResponse {
    /// The call's identifier, shared with the ledger and the log.
    pub call_id: Ulid,
    /// The completion text.
    pub text: String,
    /// Reasoning characters the provider reported before answering, when it
    /// exposed them. Diagnostic only: never part of `text`, and zero for a
    /// provider that keeps its reasoning private.
    #[serde(default)]
    pub reasoning_chars: u32,
    /// The model that actually answered.
    pub model: String,
    /// What it cost.
    pub usage: Usage,
    /// Whether this came from the cache.
    pub cached: bool,
    /// Whether the text satisfied its schema.
    pub schema_valid: bool,
    /// Which decoder produced it.
    pub decoder: DecoderKind,
    /// Tool calls on the assistant message. Empty when the answer is text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ProviderToolCall>,
    /// The provider's stop reason, when it supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// Non-content provider metadata needed by a caller (for example Hermes's
    /// `mm_status`). Never contains prompts, keys, headers, or tool arguments.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_metadata: BTreeMap<String, Value>,
    /// Whether the provider used server-sent events.
    #[serde(default)]
    pub streamed: bool,
}

/// Receives the text of a completion while it is still being produced.
///
/// A sink is deliberately the narrowest surface a stream needs: it cannot fail,
/// cannot reorder, and cannot see anything except the text the model emitted. A
/// caller that wants the final answer reads it from [`LlmResponse::text`], which
/// is the concatenation of every delta; the sink is for display only.
pub trait DeltaSink: Send {
    /// Accept one fragment of completion text, in order.
    fn delta(&mut self, delta: &str);
}

/// Collects deltas into a string. Useful for a caller that wants a streamed call
/// but only cares about the assembled text.
#[derive(Debug, Default, Clone)]
pub struct CollectSink(pub String);

impl DeltaSink for CollectSink {
    fn delta(&mut self, delta: &str) {
        self.0.push_str(delta);
    }
}

/// A provider adapter.
///
/// A client knows one provider. It does not cache, route, account, or log — those
/// are the substrate's job, and keeping them out of the adapter is what makes every
/// provider interchangeable.
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Complete a request.
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse>;

    /// Complete a request, reporting text deltas as they arrive.
    ///
    /// The default is correct but not live: it performs the ordinary call and
    /// offers the whole answer as one delta. Adapters that can stream override it;
    /// callers can therefore depend on the method everywhere without requiring
    /// every provider to support it.
    async fn complete_stream(
        &self,
        req: &LlmRequest,
        sink: &mut dyn DeltaSink,
    ) -> Result<LlmResponse> {
        let response = self.complete(req).await?;
        if !response.text.is_empty() {
            sink.delta(&response.text);
        }
        Ok(response)
    }

    /// The provider's name, recorded in the ledger.
    fn provider(&self) -> &str;

    /// The provider selected for `req`.
    ///
    /// A client that dispatches by request (a provider pool) overrides this so
    /// the ledger and cache key name the selected profile rather than the pool.
    fn provider_for<'a>(&'a self, _req: &'a LlmRequest) -> &'a str {
        self.provider()
    }

    /// True when the adapter makes schema-valid output by construction.
    fn decoder(&self) -> DecoderKind {
        DecoderKind::None
    }

    /// Make a schema available to an adapter that asks the provider for native
    /// JSON Schema output.
    ///
    /// The service owns the authoritative registry and validates every answer
    /// afterwards. Adapters that can also constrain the request need the same
    /// definition; the default is a no-op for clients that cannot.
    fn provide_schema(&self, _id: SchemaId, _schema: Value) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purposes_round_trip_through_their_stored_form() {
        for purpose in Purpose::ALL {
            assert_eq!(Purpose::parse(purpose.as_str()), Some(purpose));
        }
        assert_eq!(Purpose::CodeReview.as_str(), "code_review");
        assert_eq!(Purpose::parse("codereview"), None);
    }

    #[test]
    fn decoders_round_trip_and_know_which_ones_constrain() {
        for decoder in DecoderKind::ALL {
            assert_eq!(DecoderKind::parse(decoder.as_str()), Some(decoder));
        }
        assert!(DecoderKind::Llguidance.is_constrained());
        assert!(DecoderKind::XGrammar.is_constrained());
        assert!(!DecoderKind::ProviderNative.is_constrained());
        assert!(!DecoderKind::None.is_constrained());
    }

    #[test]
    fn the_cache_defaults_to_use() {
        assert_eq!(CacheMode::default(), CacheMode::Use);
    }

    #[test]
    fn a_request_builder_keeps_the_purpose_and_adds_the_schema() {
        let req = LlmRequest::new(
            Purpose::Extract,
            vec![Message::system("be terse"), Message::user("hello world")],
        )
        .with_schema(SchemaId::new("extract.v1"))
        .with_model("small");
        assert_eq!(req.purpose, Purpose::Extract);
        assert_eq!(req.schema.as_ref().unwrap().as_str(), "extract.v1");
        assert_eq!(req.model.as_deref(), Some("small"));
        assert_eq!(req.user_excerpt(5), "hello");
    }

    #[test]
    fn new_request_fields_are_additive_and_round_trip() {
        let call_id = Ulid::from_parts(7, 9);
        let req = LlmRequest::new(Purpose::Agent, vec![Message::user("hello")])
            .with_provider("hermes")
            .with_timeout(120_000)
            .with_call_id(call_id);
        assert_eq!(req.provider.as_deref(), Some("hermes"));
        assert_eq!(req.timeout_ms, Some(120_000));
        assert_eq!(req.call_id, Some(call_id));
        let encoded = serde_json::to_value(&req).unwrap();
        let decoded: LlmRequest = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn a_request_without_the_new_fields_still_deserializes() {
        let old = serde_json::json!({
            "purpose": "converse",
            "model": null,
            "messages": [{"role": "user", "content": "hi"}],
            "schema": null,
            "max_tokens": 1024,
            "temperature": 0.0,
            "cache": "use",
            "trace_id": null,
            "tools": []
        });
        let req: LlmRequest = serde_json::from_value(old).unwrap();
        assert_eq!(req.provider, None);
        assert_eq!(req.timeout_ms, None);
        assert_eq!(req.call_id, None);
    }

    #[test]
    fn response_metadata_is_additive_and_round_trips() {
        let mut response = LlmResponse {
            call_id: Ulid::from_parts(1, 2),
            text: "ok".into(),
            reasoning_chars: 0,
            model: "small".into(),
            usage: Usage::default(),
            cached: false,
            schema_valid: true,
            decoder: DecoderKind::None,
            tool_calls: Vec::new(),
            finish_reason: Some("stop".into()),
            provider_metadata: BTreeMap::from([("mm_status".into(), Value::from("done"))]),
            streamed: true,
        };
        let encoded = serde_json::to_value(&response).unwrap();
        let decoded: LlmResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, response);

        response.finish_reason = None;
        response.provider_metadata.clear();
        response.streamed = false;
        let old = serde_json::json!({
            "call_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "text": "ok",
            "model": "small",
            "usage": {"tokens_in": 0, "tokens_out": 0, "cost_micros": 0, "latency_ms": 0},
            "cached": false,
            "schema_valid": true,
            "decoder": "none",
            "tool_calls": []
        });
        let old: LlmResponse = serde_json::from_value(old).unwrap();
        assert_eq!(old.finish_reason, None);
        assert!(old.provider_metadata.is_empty());
        assert!(!old.streamed);
    }
}
