//! The provider-neutral request, response, and client trait.
//!
//! These are the only types a caller of the substrate sees. Every provider is
//! reduced to this shape by an adapter, so nothing above this layer can depend on
//! a vendor's JSON, and swapping providers cannot change a caller.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
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
}

impl Purpose {
    /// Every purpose, in a stable order.
    pub const ALL: [Purpose; 8] = [
        Purpose::Interpret,
        Purpose::Plan,
        Purpose::Extract,
        Purpose::Critique,
        Purpose::Summarize,
        Purpose::CodeReview,
        Purpose::Classify,
        Purpose::Diagnose,
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
}

/// One message in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Who wrote it.
    pub role: Role,
    /// The content.
    pub content: String,
}

impl Message {
    /// A system message.
    pub fn system(content: impl Into<String>) -> Self {
        Message {
            role: Role::System,
            content: content.into(),
        }
    }

    /// A user message.
    pub fn user(content: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: content.into(),
        }
    }

    /// An assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Message {
            role: Role::Assistant,
            content: content.into(),
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
}

impl LlmRequest {
    /// A request with the substrate's defaults for purpose `purpose`.
    pub fn new(purpose: Purpose, messages: Vec<Message>) -> Self {
        LlmRequest {
            purpose,
            model: None,
            messages,
            schema: None,
            max_tokens: 1024,
            temperature: 0.0,
            cache: CacheMode::Use,
            trace_id: None,
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

    /// The provider's name, recorded in the ledger.
    fn provider(&self) -> &str;

    /// True when the adapter makes schema-valid output by construction.
    fn decoder(&self) -> DecoderKind {
        DecoderKind::None
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
}
