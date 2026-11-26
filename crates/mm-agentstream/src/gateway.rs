//! The LLM gateway.
//!
//! Every model call goes through this wrapper. It appends the complete request
//! (`messages` and `tools`, not previews) before the provider is touched, then
//! publishes live deltas and appends the terminal response or error. A failed
//! request append is fail-closed: the provider is never called.

use std::sync::Arc;

use async_trait::async_trait;
use mm_llm::{
    accounting::new_call_id, CallContext, DecoderKind, DeltaSink, LlmClient, LlmError, LlmRequest,
    LlmResponse, Result, SchemaId, ToolSchema,
};
use serde_json::{json, Value};

use crate::event::{self, EventType};
use crate::store::EventStore;

/// Wraps a provider client with agentstream capture.
pub struct LlmGateway {
    client: Arc<dyn LlmClient>,
    store: Arc<EventStore>,
}

impl std::fmt::Debug for LlmGateway {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LlmGateway")
            .field("provider", &self.client.provider())
            .finish_non_exhaustive()
    }
}

impl LlmGateway {
    /// Wrap `client`, writing every call to `store`.
    pub fn new(client: Arc<dyn LlmClient>, store: Arc<EventStore>) -> Self {
        Self { client, store }
    }

    /// The underlying store.
    pub fn store(&self) -> &Arc<EventStore> {
        &self.store
    }

    fn attribution() -> CallContext {
        mm_llm::current_context().unwrap_or_default()
    }

    fn session_of(context: &CallContext) -> String {
        context
            .session_id
            .clone()
            .unwrap_or_else(|| "unattributed".into())
    }

    fn request_append(
        &self,
        request: &LlmRequest,
        call_id: &str,
        context: &CallContext,
        streamed: bool,
    ) -> Result<()> {
        let session_id = Self::session_of(context);
        let turn_id = context.run_id.clone();
        let attempt = attempt_of(context);
        let round = context.round.unwrap_or(1);
        let strategy = context.origin.clone();
        let messages = serde_json::to_value(&request.messages)
            .map_err(|error| LlmError::Journal(error.to_string()))?;
        let tools = serde_json::to_value(&request.tools)
            .map_err(|error| LlmError::Journal(error.to_string()))?;
        let params = json!({
            "model": request.model,
            "temperature": request.temperature,
            "maxTokens": request.max_tokens,
            "cache": request.cache,
            "schema": request.schema.as_ref().map(SchemaId::as_str),
            "streamed": streamed,
            "timeoutMs": request.timeout_ms,
        });
        let event = event::llm_request(
            session_id,
            turn_id,
            call_id,
            context.pass.as_deref(),
            attempt,
            round,
            request.provider.as_deref().unwrap_or("default"),
            request.purpose.as_str(),
            strategy.as_deref(),
            context.prompt_instance_id.as_deref(),
            context.prompt_hash.as_deref(),
            params,
            messages,
            tools,
        );
        self.store
            .append(event)
            .map(|_| ())
            .map_err(|error| LlmError::Journal(error.to_string()))
    }

    fn response_append(&self, call_id: &str, context: &CallContext, response: &LlmResponse) {
        let session_id = Self::session_of(context);
        let turn_id = context.run_id.clone();
        let tool_calls = serde_json::to_value(&response.tool_calls).unwrap_or(Value::Null);
        let _ = self.store.append(event::llm_attempt(
            session_id.clone(),
            turn_id.clone(),
            call_id,
            1,
            "ok",
            Some(&response.model),
            u64::from(response.usage.latency_ms),
            response.usage.tokens_in,
            response.usage.tokens_out,
        ));
        let _ = self.store.append(event::llm_response(
            session_id,
            turn_id,
            call_id,
            &response.model,
            &response.text,
            tool_calls,
            response.usage.tokens_in,
            response.usage.tokens_out,
            u64::from(response.usage.latency_ms),
            response.finish_reason.as_deref(),
            response.reasoning_chars,
            response.cached,
            response.streamed,
        ));
    }

    fn error_append(&self, call_id: &str, context: &CallContext, error: &LlmError) {
        let session_id = Self::session_of(context);
        let turn_id = context.run_id.clone();
        let _ = self.store.append(event::llm_error(
            session_id,
            turn_id,
            call_id,
            error.kind(),
            &error.to_string(),
            0,
            0,
        ));
    }
}

fn attempt_of(context: &CallContext) -> u32 {
    context
        .step
        .as_deref()
        .and_then(|step| step.strip_prefix("attempt="))
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

#[async_trait]
impl LlmClient for LlmGateway {
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        let context = Self::attribution();
        let call_id = request.call_id.unwrap_or_else(new_call_id);
        let call_id_text = mm_core::ulid_string(&call_id);
        let mut request = request.clone();
        request.call_id = Some(call_id);

        self.request_append(&request, &call_id_text, &context, false)?;
        match self.client.complete(&request).await {
            Ok(response) => {
                self.response_append(&call_id_text, &context, &response);
                Ok(response)
            }
            Err(error) => {
                self.error_append(&call_id_text, &context, &error);
                Err(error)
            }
        }
    }

    async fn complete_stream(
        &self,
        request: &LlmRequest,
        sink: &mut dyn DeltaSink,
    ) -> Result<LlmResponse> {
        let context = Self::attribution();
        let call_id = request.call_id.unwrap_or_else(new_call_id);
        let call_id_text = mm_core::ulid_string(&call_id);
        let mut request = request.clone();
        request.call_id = Some(call_id);

        self.request_append(&request, &call_id_text, &context, true)?;
        let session_id = Self::session_of(&context);
        let turn_id = context.run_id.clone();
        let mut publishing = PublishingSink {
            inner: sink,
            store: &self.store,
            session_id,
            turn_id,
            call_id: call_id_text.clone(),
            index: 0,
        };
        match self.client.complete_stream(&request, &mut publishing).await {
            Ok(response) => {
                self.response_append(&call_id_text, &context, &response);
                Ok(response)
            }
            Err(error) => {
                self.error_append(&call_id_text, &context, &error);
                Err(error)
            }
        }
    }

    fn provider(&self) -> &str {
        self.client.provider()
    }

    fn provider_for<'a>(&'a self, request: &'a LlmRequest) -> &'a str {
        self.client.provider_for(request)
    }

    fn decoder(&self) -> DecoderKind {
        self.client.decoder()
    }

    fn provide_schema(&self, id: SchemaId, schema: Value) -> Result<()> {
        self.client.provide_schema(id, schema)
    }
}

struct PublishingSink<'a> {
    inner: &'a mut dyn DeltaSink,
    store: &'a EventStore,
    session_id: String,
    turn_id: Option<String>,
    call_id: String,
    index: u32,
}

impl DeltaSink for PublishingSink<'_> {
    fn delta(&mut self, delta: &str) {
        self.inner.delta(delta);
        if delta.is_empty() {
            return;
        }
        let event = event::llm_delta(
            self.session_id.clone(),
            self.turn_id.clone(),
            &self.call_id,
            self.index,
            delta,
        );
        self.index = self.index.saturating_add(1);
        let _ = self.store.append(event);
    }
}

/// Whether `event_type` is a model event the gateway owns.
pub fn is_gateway_event(event_type: EventType) -> bool {
    matches!(
        event_type,
        EventType::LlmRequest
            | EventType::LlmAttempt
            | EventType::LlmDelta
            | EventType::LlmResponse
            | EventType::LlmError
    )
}

/// Tools the gateway saw in the last request, for tests and diagnostics.
pub fn offered_tools(request: &LlmRequest) -> Vec<ToolSchema> {
    request.tools.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_llm::{
        scope, Message, MockClient, ProviderToolCall, Purpose, ScriptedResponse, ToolSchema,
    };

    fn store() -> (tempfile::TempDir, Arc<EventStore>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(EventStore::new(dir.path()).unwrap());
        store.ensure_session("s", "u", None).unwrap();
        (dir, store)
    }

    #[tokio::test]
    async fn the_request_carries_every_message_and_tool() {
        let (_dir, store) = store();
        let client = Arc::new(MockClient::script(vec![ScriptedResponse::text("ok")]));
        let gateway = LlmGateway::new(client, store.clone());
        let request = LlmRequest::new(
            Purpose::Converse,
            vec![Message::system("system prompt"), Message::user("hello")],
        )
        .with_tools(vec![ToolSchema {
            name: "conversation.topic".into(),
            description: "set the topic".into(),
            parameters: json!({"type": "object", "properties": {"topic": {"type": "string"}}}),
        }])
        .with_model("small");
        let context = CallContext::new("turn.pipeline")
            .with_session("s")
            .with_run("t")
            .with_pass("process", Some(1))
            .with_origin("cm.process.v1");
        scope(context, gateway.complete(&request)).await.unwrap();

        let events = store.events("s", 1, None).unwrap();
        let request_event = events
            .iter()
            .find(|event| event.event_type == EventType::LlmRequest)
            .unwrap();
        assert_eq!(request_event.data["messages"].as_array().unwrap().len(), 2);
        assert_eq!(
            request_event.data["messages"][0]["content"],
            "system prompt"
        );
        assert_eq!(request_event.data["tools"].as_array().unwrap().len(), 1);
        assert_eq!(
            request_event.data["tools"][0]["parameters"]["properties"]["topic"]["type"],
            "string"
        );
        assert_eq!(request_event.data["strategy"], "cm.process.v1");
        assert_eq!(request_event.data["pass"], "process");
        assert_eq!(request_event.model.as_deref(), Some("small"));
        assert!(events
            .iter()
            .any(|event| event.event_type == EventType::LlmResponse));
    }

    #[tokio::test]
    async fn a_stream_publishes_deltas_without_persisting_them() {
        let (_dir, store) = store();
        let client = Arc::new(MockClient::script(vec![ScriptedResponse::text("hello")]));
        let gateway = LlmGateway::new(client, store.clone());
        let request = LlmRequest::new(Purpose::Converse, vec![Message::user("hi")]);
        let mut sink = mm_llm::CollectSink::default();
        let context = CallContext::new("turn.pipeline")
            .with_session("s")
            .with_run("t")
            .with_pass("process", Some(1));
        scope(context, gateway.complete_stream(&request, &mut sink))
            .await
            .unwrap();
        assert_eq!(sink.0, "hello");
        let events = store.events("s", 1, None).unwrap();
        assert!(!events
            .iter()
            .any(|event| event.event_type == EventType::LlmDelta));
        assert!(events
            .iter()
            .any(|event| event.event_type == EventType::LlmResponse));
    }

    #[tokio::test]
    async fn a_missing_script_is_a_transport_error_and_is_logged() {
        let (_dir, store) = store();
        let gateway = LlmGateway::new(Arc::new(MockClient::new()), store.clone());
        let request = LlmRequest::new(Purpose::Converse, vec![Message::user("hi")]);
        let context = CallContext::new("turn.pipeline")
            .with_session("s")
            .with_run("t");
        let error = scope(context, gateway.complete(&request))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "transport");
        assert!(store
            .events("s", 1, None)
            .unwrap()
            .iter()
            .any(|event| event.event_type == EventType::LlmError));
    }

    #[test]
    fn gateway_events_are_exactly_the_llm_family() {
        for event_type in EventType::ALL {
            assert_eq!(
                is_gateway_event(event_type),
                event_type.as_str().starts_with("llm.")
            );
        }
    }

    #[test]
    fn offered_tools_are_the_request_tools() {
        let request = LlmRequest::new(Purpose::Converse, vec![]).with_tools(vec![ToolSchema {
            name: "x".into(),
            description: "x".into(),
            parameters: json!({}),
        }]);
        assert_eq!(offered_tools(&request).len(), 1);
        let _ = ProviderToolCall {
            id: "c".into(),
            name: "x".into(),
            arguments: "{}".into(),
        };
    }
}
