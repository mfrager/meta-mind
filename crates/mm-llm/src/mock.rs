//! The scripted client.
//!
//! Tests and fixture runs must not touch a provider, and they must be able to
//! produce output a *correct* provider would never produce — a truncated payload,
//! an extra field, a string where the schema wants an integer. That is the whole
//! point of the adversarial fixtures, so the mock serves whatever it is told and
//! never validates: validation is the substrate's job and the thing under test.
//!
//! A mock with nothing scripted **fails loudly** rather than inventing a plausible
//! answer. A test that forgot to script a response should break, not pass on
//! synthesised output.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::client::{DecoderKind, LlmClient, LlmRequest, LlmResponse, Usage};
use crate::error::{LlmError, Result};

/// One scripted completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptedResponse {
    /// The completion text, exactly as the mock will return it.
    pub text: String,
    /// The model name to report.
    pub model: String,
    /// Prompt tokens to report.
    pub tokens_in: u32,
    /// Completion tokens to report.
    pub tokens_out: u32,
    /// Latency to report.
    pub latency_ms: u32,
    /// Whether the text is *claimed* to be schema-valid. The service still checks;
    /// this only lets a test model a provider that lies.
    pub schema_valid: bool,
    /// When set, the call fails with a transport error carrying this message.
    pub error: Option<String>,
}

impl ScriptedResponse {
    /// A valid-looking completion.
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        let tokens_out = (text.split_whitespace().count() as u32).max(1);
        ScriptedResponse {
            text,
            model: "mock-small".into(),
            tokens_in: 16,
            tokens_out,
            latency_ms: 12,
            schema_valid: true,
            error: None,
        }
    }

    /// A provider that cannot be reached.
    pub fn failing(message: impl Into<String>) -> Self {
        ScriptedResponse {
            text: String::new(),
            model: "mock-small".into(),
            tokens_in: 0,
            tokens_out: 0,
            latency_ms: 0,
            schema_valid: false,
            error: Some(message.into()),
        }
    }

    /// Set the reported model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Set the reported token counts.
    pub fn with_tokens(mut self, tokens_in: u32, tokens_out: u32) -> Self {
        self.tokens_in = tokens_in;
        self.tokens_out = tokens_out;
        self
    }

    /// Set the reported latency.
    pub fn with_latency(mut self, latency_ms: u32) -> Self {
        self.latency_ms = latency_ms;
        self
    }

    /// Claim schema validity the text does not have.
    pub fn claiming_valid(mut self) -> Self {
        self.schema_valid = true;
        self
    }
}

/// A client that answers from a script.
#[derive(Debug)]
pub struct MockClient {
    provider: String,
    decoder: DecoderKind,
    script: Mutex<VecDeque<ScriptedResponse>>,
    constant: Option<ScriptedResponse>,
    seen: Mutex<Vec<LlmRequest>>,
    calls: AtomicU64,
}

impl Default for MockClient {
    fn default() -> Self {
        MockClient::new()
    }
}

impl MockClient {
    /// A mock with nothing scripted.
    pub fn new() -> Self {
        MockClient {
            provider: "mock".into(),
            decoder: DecoderKind::None,
            script: Mutex::new(VecDeque::new()),
            constant: None,
            seen: Mutex::new(Vec::new()),
            calls: AtomicU64::new(0),
        }
    }

    /// A mock that serves `script` in order, then fails.
    pub fn script(script: Vec<ScriptedResponse>) -> Self {
        MockClient {
            script: Mutex::new(script.into()),
            ..MockClient::new()
        }
    }

    /// A mock that serves the same answer to every call.
    ///
    /// This is how the repair path is exercised: the first decode fails validation,
    /// the repair pass asks again, and the same unsatisfying answer comes back — so
    /// the call must end in `SchemaRejected` rather than in a coerced value.
    pub fn constant(response: ScriptedResponse) -> Self {
        MockClient {
            constant: Some(response),
            ..MockClient::new()
        }
    }

    /// Report a different provider name.
    pub fn named(mut self, provider: impl Into<String>) -> Self {
        self.provider = provider.into();
        self
    }

    /// Report a constrained decoder, as a logits-exposing endpoint would.
    pub fn constrained(mut self, decoder: DecoderKind) -> Self {
        self.decoder = decoder;
        self
    }

    /// Append one answer to the script.
    pub fn push(&self, response: ScriptedResponse) {
        self.script
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(response);
    }

    /// How many calls have been served.
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    /// Every request the mock saw, in order.
    pub fn seen(&self) -> Vec<LlmRequest> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The most recent request, if any.
    pub fn last(&self) -> Option<LlmRequest> {
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last()
            .cloned()
    }
}

#[async_trait]
impl LlmClient for MockClient {
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.clone());

        let next = self
            .script
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .or_else(|| self.constant.clone())
            .ok_or_else(|| {
                LlmError::Transport(
                    "the mock has no scripted response for this call; script one".into(),
                )
            })?;

        if let Some(message) = next.error {
            return Err(LlmError::Transport(message));
        }

        Ok(LlmResponse {
            call_id: crate::accounting::new_call_id(),
            text: next.text,
            model: if next.model.is_empty() {
                req.model.clone().unwrap_or_else(|| "mock-small".into())
            } else {
                next.model
            },
            usage: Usage {
                tokens_in: next.tokens_in,
                tokens_out: next.tokens_out,
                cost_micros: 0,
                latency_ms: next.latency_ms,
            },
            cached: false,
            schema_valid: next.schema_valid,
            decoder: self.decoder,
        })
    }

    fn provider(&self) -> &str {
        &self.provider
    }

    fn decoder(&self) -> DecoderKind {
        self.decoder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Message, Purpose};

    fn request() -> LlmRequest {
        LlmRequest::new(Purpose::Plan, vec![Message::user("hello")])
    }

    #[tokio::test]
    async fn a_scripted_mock_serves_in_order_and_records_every_request() {
        let client = MockClient::script(vec![
            ScriptedResponse::text("first"),
            ScriptedResponse::text("second"),
        ]);
        let first = client.complete(&request()).await.unwrap();
        assert_eq!(first.text, "first");
        assert_eq!(first.model, "mock-small");
        let second = client.complete(&request()).await.unwrap();
        assert_eq!(second.text, "second");
        assert_eq!(client.calls(), 2);
        assert_eq!(client.seen().len(), 2);

        // Running out of script is an error, not a fabricated answer.
        assert!(client.complete(&request()).await.is_err());
    }

    #[tokio::test]
    async fn a_constant_mock_repeats_so_the_repair_path_can_be_exercised() {
        let client = MockClient::constant(ScriptedResponse::text(r#"{"name": }"#));
        assert_eq!(
            client.complete(&request()).await.unwrap().text,
            r#"{"name": }"#
        );
        assert_eq!(
            client.complete(&request()).await.unwrap().text,
            r#"{"name": }"#
        );
        assert_eq!(client.calls(), 2);
    }

    #[tokio::test]
    async fn a_failing_script_becomes_a_transport_error() {
        let client = MockClient::script(vec![ScriptedResponse::failing("connection reset")]);
        let err = client.complete(&request()).await.unwrap_err();
        assert_eq!(err.kind(), "transport");
        assert!(err.to_string().contains("connection reset"));
    }
}
