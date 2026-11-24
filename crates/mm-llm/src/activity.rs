//! Opt-in observation of model activity.
//!
//! Callers of the substrate depend on [`LlmClient`](crate::LlmClient) and
//! nothing else. That is what makes one decorator enough to see every call:
//! [`ObservedClient`] wraps another client, reports the request before it is
//! sent, tails a streaming answer as it arrives, and reports the outcome — while
//! forwarding the exact same request, response, and error the inner client
//! produced.
//!
//! Observation is deliberately best-effort and synchronous. A sink cannot fail,
//! return a value, or suspend the call; the trait's methods have no `Result`, so
//! an observer can never make a model call fail. Deltas are coalesced and capped
//! ([`DELTA_FLUSH_CHARS`], [`DELTA_FLUSH_MS`], [`DELTA_CAP_CHARS`]) because this
//! surface is for display, not accounting: the authoritative reply text is still
//! [`LlmResponse::text`].
//!
//! [`CallContext`] labels a call with the work that caused it. It is attached
//! with [`scope`], a task-local that survives `.await` inside the scoped future.
//! Work that cannot inherit a scope is still attributed: the wrapper carries a
//! default context, so no call is anonymous.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ulid::Ulid;

use crate::accounting;
use crate::client::{DecoderKind, DeltaSink, LlmClient, LlmRequest, LlmResponse};
use crate::error::{LlmError, Result};

/// Where a model call came from and which unit of work it belongs to.
///
/// All fields are cheap to clone because a scope clones its context per call.
/// `source` is a stable subsystem name (`chat`, `agent`, `automation`,
/// `process`, `summarize`, `other`); the remaining fields are absent when the
/// call is not part of a larger piece of work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallContext {
    /// The subsystem that made the call.
    pub source: String,
    /// The conversation session, when the call belongs to one.
    pub session_id: Option<String>,
    /// The chat run id, when the call belongs to one.
    pub run_id: Option<String>,
    /// The three-pass phase (`interpret`, `process`, `verify`, `think`).
    pub pass: Option<String>,
    /// The model round inside the phase.
    pub round: Option<u32>,
}

impl CallContext {
    /// A context with a source and nothing else.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            session_id: None,
            run_id: None,
            pass: None,
            round: None,
        }
    }

    /// Attach the conversation session.
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Attach the chat run.
    pub fn with_run(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    /// Attach the pass and round.
    pub fn with_pass(mut self, pass: impl Into<String>, round: Option<u32>) -> Self {
        self.pass = Some(pass.into());
        self.round = round;
        self
    }
}

impl Default for CallContext {
    fn default() -> Self {
        Self::new("other")
    }
}

/// Receives activity for display.
///
/// Implementations must be cheap and must not block. They are called on the
/// model-call task, so an observer that performs I/O or takes a contended lock
/// delays the call it is observing.
pub trait ActivitySink: Send + Sync {
    /// A call is about to reach the inner client.
    ///
    /// `provider` is the inner client's provider name; `streamed` says whether
    /// the caller asked for deltas. Both are known before the call is made.
    fn request(
        &self,
        call_id: Ulid,
        context: &CallContext,
        provider: &str,
        streamed: bool,
        request: &LlmRequest,
    );

    /// A coalesced fragment of streaming text.
    ///
    /// `total_chars` counts every character emitted by the call so far, not just
    /// this fragment. `truncated` is true on the final reported fragment once
    /// [`DELTA_CAP_CHARS`] was reached.
    fn delta(&self, call_id: Ulid, text: &str, total_chars: u32, truncated: bool);

    /// The call answered.
    fn response(&self, call_id: Ulid, latency_ms: u64, response: &LlmResponse);

    /// The call failed. `partial_chars` is the text reported before the failure.
    fn error(&self, call_id: Ulid, latency_ms: u64, partial_chars: u32, error: &LlmError);
}

/// Characters buffered before a streaming fragment is reported.
pub const DELTA_FLUSH_CHARS: usize = 256;
/// Longest a streaming fragment waits before it is reported.
pub const DELTA_FLUSH_MS: u64 = 50;
/// Most text reported per call. Display only; the response keeps the full text.
pub const DELTA_CAP_CHARS: usize = 2_000;

/// Bounds on how much streamed text is reported for one call.
///
/// The defaults are the display constants; `mm-serve` overrides the window from
/// `MM_LLM_ACTIVITY_DELTA_MS` without touching this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaPolicy {
    /// Characters buffered before a fragment is reported.
    pub flush_chars: usize,
    /// Longest a fragment waits before it is reported.
    pub flush_ms: u64,
    /// Most text reported for one call.
    pub cap_chars: usize,
}

impl Default for DeltaPolicy {
    fn default() -> Self {
        Self {
            flush_chars: DELTA_FLUSH_CHARS,
            flush_ms: DELTA_FLUSH_MS,
            cap_chars: DELTA_CAP_CHARS,
        }
    }
}

tokio::task_local! {
    static CALL_CONTEXT: CallContext;
}

/// Run `future` with `context` available to [`current_context`].
///
/// The value is visible for the whole scoped future, including across `.await`.
/// Work moved to a newly spawned task does not inherit it; those paths pass a
/// default context to [`ObservedClient`] instead.
pub fn scope<F>(context: CallContext, future: F) -> impl Future<Output = F::Output>
where
    F: Future,
{
    CALL_CONTEXT.scope(context, future)
}

/// The context of the innermost [`scope`] on this task, if any.
pub fn current_context() -> Option<CallContext> {
    CALL_CONTEXT.try_with(Clone::clone).ok()
}

/// A client decorator that reports activity and forwards everything else.
pub struct ObservedClient {
    inner: Arc<dyn LlmClient>,
    sink: Arc<dyn ActivitySink>,
    context: CallContext,
    delta: DeltaPolicy,
}

impl ObservedClient {
    /// Wrap `inner`, reporting to `sink`.
    ///
    /// The wrapper's context is used when the call is not inside a [`scope`].
    /// Use [`ObservedClient::with_context`] to name the default subsystem.
    pub fn new(inner: Arc<dyn LlmClient>, sink: Arc<dyn ActivitySink>) -> Self {
        Self {
            inner,
            sink,
            context: CallContext::default(),
            delta: DeltaPolicy::default(),
        }
    }

    /// Set the context used when the call is not inside a [`scope`].
    pub fn with_context(mut self, context: CallContext) -> Self {
        self.context = context;
        self
    }

    /// Override the streaming report window.
    pub fn with_delta_policy(mut self, delta: DeltaPolicy) -> Self {
        self.delta = delta;
        self
    }

    fn context(&self) -> CallContext {
        current_context().unwrap_or_else(|| self.context.clone())
    }
}

impl std::fmt::Debug for ObservedClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObservedClient")
            .field("provider", &self.inner.provider())
            .field("source", &self.context.source)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl LlmClient for ObservedClient {
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        let call_id = accounting::new_call_id();
        let context = self.context();
        let started = Instant::now();
        self.sink
            .request(call_id, &context, self.inner.provider(), false, request);
        match self.inner.complete(request).await {
            Ok(response) => {
                self.sink.response(call_id, elapsed_ms(started), &response);
                Ok(response)
            }
            Err(error) => {
                self.sink.error(call_id, elapsed_ms(started), 0, &error);
                Err(error)
            }
        }
    }

    async fn complete_stream(
        &self,
        request: &LlmRequest,
        sink: &mut dyn DeltaSink,
    ) -> Result<LlmResponse> {
        let call_id = accounting::new_call_id();
        let context = self.context();
        let started = Instant::now();
        self.sink
            .request(call_id, &context, self.inner.provider(), true, request);
        let mut tee = TeeSink::new(sink, Arc::clone(&self.sink), call_id, self.delta);
        let result = self.inner.complete_stream(request, &mut tee).await;
        tee.finish();
        let partial_chars = tee.total_chars();
        drop(tee);
        match result {
            Ok(response) => {
                self.sink.response(call_id, elapsed_ms(started), &response);
                Ok(response)
            }
            Err(error) => {
                self.sink
                    .error(call_id, elapsed_ms(started), partial_chars, &error);
                Err(error)
            }
        }
    }

    fn provider(&self) -> &str {
        self.inner.provider()
    }

    fn decoder(&self) -> DecoderKind {
        self.inner.decoder()
    }

    fn provide_schema(&self, id: crate::client::SchemaId, schema: serde_json::Value) -> Result<()> {
        self.inner.provide_schema(id, schema)
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

/// Forwards deltas to the caller and reports a coalesced copy to the sink.
struct TeeSink<'a> {
    inner: &'a mut dyn DeltaSink,
    sink: Arc<dyn ActivitySink>,
    call_id: Ulid,
    policy: DeltaPolicy,
    buffer: String,
    total_chars: u32,
    truncated: bool,
    last_flush: Instant,
}

impl<'a> TeeSink<'a> {
    fn new(
        inner: &'a mut dyn DeltaSink,
        sink: Arc<dyn ActivitySink>,
        call_id: Ulid,
        policy: DeltaPolicy,
    ) -> Self {
        Self {
            inner,
            sink,
            call_id,
            policy,
            buffer: String::new(),
            total_chars: 0,
            truncated: false,
            last_flush: Instant::now(),
        }
    }

    fn total_chars(&self) -> u32 {
        self.total_chars
    }

    /// Report whatever is buffered, even after a failure.
    fn finish(&mut self) {
        self.flush();
    }

    fn flush(&mut self) {
        if self.buffer.is_empty() {
            self.last_flush = Instant::now();
            return;
        }
        self.sink
            .delta(self.call_id, &self.buffer, self.total_chars, self.truncated);
        self.buffer.clear();
        self.last_flush = Instant::now();
    }
}

impl DeltaSink for TeeSink<'_> {
    fn delta(&mut self, delta: &str) {
        // The caller's sink sees every fragment, exactly as it would without us.
        self.inner.delta(delta);
        if self.truncated || delta.is_empty() {
            return;
        }

        let remaining = self
            .policy
            .cap_chars
            .saturating_sub(self.total_chars as usize);
        let chars = delta.chars().count();
        if chars >= remaining {
            self.buffer.extend(delta.chars().take(remaining));
            self.total_chars = self.policy.cap_chars as u32;
            self.truncated = true;
            self.flush();
            return;
        }

        self.buffer.push_str(delta);
        self.total_chars += chars as u32;
        if self.buffer.chars().count() >= self.policy.flush_chars
            || self.last_flush.elapsed() >= Duration::from_millis(self.policy.flush_ms)
        {
            self.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::client::{CollectSink, Message, Purpose, Usage};
    use crate::mock::{MockClient, ScriptedResponse};

    /// One observed callback, flattened for assertions.
    #[derive(Debug, Clone, PartialEq)]
    enum Recorded {
        Request {
            call_id: Ulid,
            context: CallContext,
            purpose: Purpose,
            model: Option<String>,
        },
        Delta {
            call_id: Ulid,
            text: String,
            total_chars: u32,
            truncated: bool,
        },
        Response {
            call_id: Ulid,
            cached: bool,
            model: String,
        },
        Error {
            call_id: Ulid,
            kind: &'static str,
            partial_chars: u32,
        },
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<Recorded>>,
    }

    impl RecordingSink {
        fn events(&self) -> Vec<Recorded> {
            self.events
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }

        fn push(&self, event: Recorded) {
            self.events
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(event);
        }
    }

    impl ActivitySink for RecordingSink {
        fn request(
            &self,
            call_id: Ulid,
            context: &CallContext,
            _provider: &str,
            _streamed: bool,
            request: &LlmRequest,
        ) {
            self.push(Recorded::Request {
                call_id,
                context: context.clone(),
                purpose: request.purpose,
                model: request.model.clone(),
            });
        }

        fn delta(&self, call_id: Ulid, text: &str, total_chars: u32, truncated: bool) {
            self.push(Recorded::Delta {
                call_id,
                text: text.to_owned(),
                total_chars,
                truncated,
            });
        }

        fn response(&self, call_id: Ulid, _latency_ms: u64, response: &LlmResponse) {
            self.push(Recorded::Response {
                call_id,
                cached: response.cached,
                model: response.model.clone(),
            });
        }

        fn error(&self, call_id: Ulid, _latency_ms: u64, partial_chars: u32, error: &LlmError) {
            self.push(Recorded::Error {
                call_id,
                kind: error.kind(),
                partial_chars,
            });
        }
    }

    /// A client whose streaming call emits `chunks` in order.
    struct StreamingClient {
        chunks: Vec<String>,
        fail_after: Option<usize>,
    }

    impl StreamingClient {
        fn chunks(chunks: &[&str]) -> Self {
            Self {
                chunks: chunks.iter().map(|chunk| chunk.to_string()).collect(),
                fail_after: None,
            }
        }

        fn failing_after(chunks: &[&str], fail_after: usize) -> Self {
            Self {
                chunks: chunks.iter().map(|chunk| chunk.to_string()).collect(),
                fail_after: Some(fail_after),
            }
        }

        fn response(&self, text: String) -> LlmResponse {
            LlmResponse {
                call_id: accounting::new_call_id(),
                text,
                model: "stream-mock".into(),
                usage: Usage {
                    tokens_in: 10,
                    tokens_out: 20,
                    cost_micros: 0,
                    latency_ms: 3,
                },
                cached: false,
                schema_valid: true,
                decoder: DecoderKind::None,
                tool_calls: Vec::new(),
                finish_reason: None,
                provider_metadata: Default::default(),
                streamed: false,
            }
        }
    }

    #[async_trait]
    impl LlmClient for StreamingClient {
        async fn complete(&self, _request: &LlmRequest) -> Result<LlmResponse> {
            Ok(self.response(self.chunks.concat()))
        }

        async fn complete_stream(
            &self,
            _request: &LlmRequest,
            sink: &mut dyn DeltaSink,
        ) -> Result<LlmResponse> {
            for (index, chunk) in self.chunks.iter().enumerate() {
                if self.fail_after == Some(index) {
                    return Err(LlmError::Transport("stream reset".into()));
                }
                sink.delta(chunk);
            }
            Ok(self.response(self.chunks.concat()))
        }

        fn provider(&self) -> &str {
            "stream-mock"
        }
    }

    /// A client that answers from an exact cache.
    struct CachedClient;

    #[async_trait]
    impl LlmClient for CachedClient {
        async fn complete(&self, _request: &LlmRequest) -> Result<LlmResponse> {
            Ok(LlmResponse {
                call_id: accounting::new_call_id(),
                text: "from cache".into(),
                model: "cached-model".into(),
                usage: Usage::default(),
                cached: true,
                schema_valid: true,
                decoder: DecoderKind::None,
                tool_calls: Vec::new(),
                finish_reason: None,
                provider_metadata: Default::default(),
                streamed: false,
            })
        }

        fn provider(&self) -> &str {
            "cache"
        }
    }

    fn request() -> LlmRequest {
        LlmRequest::new(Purpose::Converse, vec![Message::user("hello")])
    }

    fn sink_arc(sink: &Arc<RecordingSink>) -> Arc<dyn ActivitySink> {
        Arc::clone(sink) as Arc<dyn ActivitySink>
    }

    fn observed(client: impl LlmClient + 'static, sink: Arc<RecordingSink>) -> ObservedClient {
        let sink: Arc<dyn ActivitySink> = sink;
        ObservedClient::new(Arc::new(client), sink)
    }

    #[tokio::test]
    async fn a_call_reports_its_request_then_response() {
        let sink = Arc::new(RecordingSink::default());
        let client = observed(
            MockClient::script(vec![ScriptedResponse::text("hi")]),
            Arc::clone(&sink),
        );

        let response = client.complete(&request()).await.unwrap();
        assert_eq!(response.text, "hi");
        let events = sink.events();
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(events[0], Recorded::Request { .. }));
        assert!(matches!(events[1], Recorded::Response { .. }));

        let call_id = match &events[0] {
            Recorded::Request { call_id, .. } => *call_id,
            other => panic!("expected request, got {other:?}"),
        };
        match &events[1] {
            Recorded::Response {
                call_id: id,
                model,
                cached,
            } => {
                assert_eq!(*id, call_id, "both events share one call id");
                assert_eq!(model, "mock-small");
                assert!(!cached);
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_stream_reports_coalesced_deltas_then_the_response() {
        let sink = Arc::new(RecordingSink::default());
        let client = observed(
            StreamingClient::chunks(&["hello ", "world"]),
            Arc::clone(&sink),
        );
        let mut collect = CollectSink::default();

        let response = client
            .complete_stream(&request(), &mut collect)
            .await
            .unwrap();
        assert_eq!(response.text, "hello world");
        assert_eq!(collect.0, "hello world", "the caller's sink is untouched");

        let events = sink.events();
        let kinds: Vec<&str> = events
            .iter()
            .map(|event| match event {
                Recorded::Request { .. } => "request",
                Recorded::Delta { .. } => "delta",
                Recorded::Response { .. } => "response",
                Recorded::Error { .. } => "error",
            })
            .collect();
        assert_eq!(kinds, ["request", "delta", "response"]);

        match &events[1] {
            Recorded::Delta {
                text,
                total_chars,
                truncated,
                ..
            } => {
                assert_eq!(text, "hello world");
                assert_eq!(*total_chars, 11);
                assert!(!truncated);
            }
            other => panic!("expected delta, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_failed_stream_keeps_the_partial_text_and_the_error() {
        let sink = Arc::new(RecordingSink::default());
        let client = observed(
            StreamingClient::failing_after(&["partial ", "answer"], 1),
            Arc::clone(&sink),
        );
        let mut collect = CollectSink::default();

        let error = client
            .complete_stream(&request(), &mut collect)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stream reset"));
        assert_eq!(collect.0, "partial ");

        let events = sink.events();
        assert!(matches!(events[0], Recorded::Request { .. }));
        assert!(matches!(events[1], Recorded::Delta { .. }));
        match events.last() {
            Some(Recorded::Error {
                kind,
                partial_chars,
                ..
            }) => {
                assert_eq!(*kind, "transport");
                assert_eq!(*partial_chars, 8, "partial text is counted, not discarded");
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_failed_call_reports_the_error_without_a_response() {
        let sink = Arc::new(RecordingSink::default());
        let client = observed(
            MockClient::script(vec![ScriptedResponse::failing("connection reset")]),
            Arc::clone(&sink),
        );

        let error = client.complete(&request()).await.unwrap_err();
        assert_eq!(error.kind(), "transport");
        let events = sink.events();
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(events[0], Recorded::Request { .. }));
        match &events[1] {
            Recorded::Error { kind, .. } => assert_eq!(*kind, "transport"),
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_cached_response_is_still_observed() {
        let sink = Arc::new(RecordingSink::default());
        let client = observed(CachedClient, Arc::clone(&sink));

        let response = client.complete(&request()).await.unwrap();
        assert_eq!(response.text, "from cache");
        assert!(response.cached);
        let events = sink.events();
        match events.last() {
            Some(Recorded::Response { cached, .. }) => assert!(cached),
            other => panic!("expected a cached response, got {other:?}"),
        }
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn delta_text_is_capped_and_marked_truncated() {
        let sink = Arc::new(RecordingSink::default());
        let chunks = vec!["0123456789".repeat(10); 40];
        let client = observed(
            StreamingClient {
                chunks,
                fail_after: None,
            },
            Arc::clone(&sink),
        );
        let mut collect = CollectSink::default();

        let response = client
            .complete_stream(&request(), &mut collect)
            .await
            .unwrap();
        assert_eq!(
            response.text.len(),
            4_000,
            "the caller still gets everything"
        );
        assert_eq!(collect.0.len(), 4_000);

        let deltas: Vec<(String, u32, bool)> = sink
            .events()
            .into_iter()
            .filter_map(|event| match event {
                Recorded::Delta {
                    text,
                    total_chars,
                    truncated,
                    ..
                } => Some((text, total_chars, truncated)),
                _ => None,
            })
            .collect();
        assert!(!deltas.is_empty());
        let reported: usize = deltas.iter().map(|(text, _, _)| text.len()).sum();
        assert_eq!(reported, DELTA_CAP_CHARS, "reporting stops at the cap");
        assert_eq!(deltas.last().unwrap().1, DELTA_CAP_CHARS as u32);
        assert!(
            deltas.last().unwrap().2,
            "the last fragment is marked truncated"
        );
    }

    #[tokio::test]
    async fn a_scope_names_the_work_that_made_the_call() {
        let sink = Arc::new(RecordingSink::default());
        let client = observed(
            MockClient::script(vec![ScriptedResponse::text("hi")]),
            Arc::clone(&sink),
        );
        let context = CallContext::new("chat")
            .with_session("session-1")
            .with_run("run-1")
            .with_pass("process", Some(2));

        scope(context.clone(), async { client.complete(&request()).await })
            .await
            .unwrap();

        match &sink.events()[0] {
            Recorded::Request { context: seen, .. } => assert_eq!(seen, &context),
            other => panic!("expected request, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_wrapper_default_attributes_calls_outside_a_scope() {
        let sink = Arc::new(RecordingSink::default());
        let client = ObservedClient::new(
            Arc::new(MockClient::script(vec![ScriptedResponse::text("hi")])),
            sink_arc(&sink),
        )
        .with_context(CallContext::new("summarize"));

        client.complete(&request()).await.unwrap();
        match &sink.events()[0] {
            Recorded::Request { context, .. } => assert_eq!(context.source, "summarize"),
            other => panic!("expected request, got {other:?}"),
        }
    }

    #[test]
    fn provider_and_decoder_are_delegated() {
        let sink = Arc::new(RecordingSink::default());
        let inner = MockClient::new()
            .named("custom")
            .constrained(DecoderKind::ProviderNative);
        let client = ObservedClient::new(Arc::new(inner), sink_arc(&sink));
        assert_eq!(client.provider(), "custom");
        assert_eq!(client.decoder(), DecoderKind::ProviderNative);
    }

    #[test]
    fn a_context_defaults_to_other() {
        assert_eq!(CallContext::default().source, "other");
    }
}
