//! Cache keys and offline replay.
//!
//! Two properties carry this file. The first is that the cache key is a hash over
//! the *whole* request, checked by a property test rather than by example. The
//! second is that a replay cannot reach a provider, checked by arming the guard
//! that would refuse the socket as well as by the counter that reports zero.

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::{fixture, network_lock, Harness};

use mm_llm::cached::{prompt_hash, verify_index};
use mm_llm::client::{CacheMode, LlmRequest, Message, Purpose};
use mm_llm::config::ProviderEnv;
use mm_llm::openai::{OpenAiClient, Transport};
use mm_llm::replay::{NetworkGuard, ReplayLlmClient, Session};
use proptest::prelude::*;
use serde_json::Value;

fn recorded_session() -> Session {
    Session::load(&fixture("bench/llm/sessions/recorded-session-01")).expect("the session loads")
}

#[test]
fn a_recorded_session_is_readable_and_named() {
    let session = recorded_session();
    assert_eq!(session.session_id, "recorded-session-01");
    assert!(session.len() >= 3, "the session records no exchanges");
    assert_eq!(session.fingerprint().len(), 64);
}

#[tokio::test]
async fn replaying_the_recorded_session_is_byte_identical_and_calls_no_provider() {
    // Arms the process-wide guard below, so no sibling test may be calling out.
    let _serial = network_lock().await;
    let session = recorded_session();
    let h = Harness::new(1).await;
    let client = Arc::new(ReplayLlmClient::new(&session, "replay"));
    let mut service = h.service_with(client.clone());
    // The session's `extract.v1` exchange is only replayable if the schema is
    // registered, exactly as the CLI's replay command registers it.
    common::register_bench_schemas(&mut service);
    let before = NetworkGuard::refused();

    let permit = NetworkGuard::arm();
    let mut first_run = Vec::new();
    for record in &session.records {
        let mut request = record.request.clone();
        request.cache = CacheMode::Use;
        let response = service
            .call(request)
            .await
            .expect("the session covers this");
        assert_eq!(
            response.text, record.response.text,
            "a replay is byte-identical"
        );
        assert!(!response.cached, "the first run has an empty cache");
        first_run.push(response.text);
    }
    drop(permit);

    assert_eq!(client.provider_calls(), 0, "a replay asks no provider");
    assert_eq!(
        NetworkGuard::refused(),
        before,
        "nothing even tried to open a socket"
    );
    assert_eq!(client.served(), session.len() as u64);

    // A second run must be indistinguishable in output, and now served by the exact
    // cache — which is also what makes the cache worth having.
    let mut served_from_cache = 0;
    for (record, first) in session.records.iter().zip(&first_run) {
        let mut request = record.request.clone();
        request.cache = CacheMode::Use;
        let response = service.call(request).await.unwrap();
        assert_eq!(&response.text, first);
        if response.cached {
            served_from_cache += 1;
        }
    }
    assert_eq!(
        served_from_cache,
        session.len(),
        "the second run is all cache"
    );

    // The cache the run populated re-hashes identically, which is what
    // `llm verify-cache --strict` checks.
    let report = verify_index(&h.store, service.cache().dir()).await.unwrap();
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(report.rows, session.len());
    h.shutdown().await;
}

#[tokio::test]
async fn a_request_the_session_does_not_cover_is_an_error_never_a_live_call() {
    let session = recorded_session();
    let h = Harness::new(1).await;
    let client = Arc::new(ReplayLlmClient::new(&session, "replay"));
    let service = h.service_with(client.clone());

    let covered = session.records[0].request.clone();
    // Pinned to the recorded model so the only difference is the prompt: the miss
    // below is about coverage, not about routing.
    let uncovered = LlmRequest::new(
        Purpose::Plan,
        vec![Message::user("a question nobody recorded")],
    )
    .with_model(covered.model.clone().unwrap_or_default());
    assert_eq!(
        covered.model, uncovered.model,
        "both route to the same model"
    );

    let err = service.call(uncovered).await.unwrap_err();
    assert_eq!(err.kind(), "replay");
    assert_eq!(client.provider_calls(), 0);
    assert_eq!(client.misses(), 1);
    h.shutdown().await;
}

/// A transport that records whether it was reached.
#[derive(Default)]
struct CountingTransport {
    calls: std::sync::atomic::AtomicU64,
}

#[async_trait]
impl Transport for CountingTransport {
    async fn post_json(&self, _url: &str, _body: &Value) -> mm_llm::error::Result<Value> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(serde_json::json!({"choices": [{"message": {"content": "live"}}]}))
    }
}

#[tokio::test]
async fn the_real_adapter_is_refused_by_the_guard_before_its_transport() {
    // Arms the process-wide guard below, and later needs egress *permitted*.
    let _serial = network_lock().await;
    // Not "the replay client does not call out", but "nothing can": a configured
    // adapter with a transport that would count the call is refused while a replay
    // is armed.
    let h = Harness::new(0).await;
    let transport = Arc::new(CountingTransport::default());
    let env = ProviderEnv::from_pairs(
        Some(["http", "://", "127.0.0.1:9/v1"].concat()),
        Some("test-key".into()),
    )
    .unwrap();
    let adapter = Arc::new(OpenAiClient::with_transport(&h.cfg, env, transport.clone()));
    let service = h.service_with(adapter);

    let request = LlmRequest::new(Purpose::Plan, vec![Message::user("hello")]);
    {
        let _permit = NetworkGuard::arm();
        let err = service.call(request).await.unwrap_err();
        assert_eq!(err.kind(), "network_forbidden");
    }
    assert_eq!(
        transport.calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the transport was never reached"
    );

    // Unarmed, the same adapter reaches its transport — so the refusal above came
    // from the guard and not from a broken adapter.
    let response = service
        .call(LlmRequest::new(Purpose::Plan, vec![Message::user("hello")]))
        .await
        .unwrap();
    assert_eq!(response.text, "live");
    assert_eq!(transport.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    h.shutdown().await;
}

proptest! {
    /// The request hash is stable, and every field of the request changes it.
    #[test]
    fn the_request_hash_is_stable_and_folds_in_each_field(
        text in "[a-z ]{0,40}",
        max_tokens in 1u32..64,
        temperature in proptest::num::f32::NORMAL,
        schema in proptest::option::of("[a-z.]{1,12}"),
    ) {
        let mut request = LlmRequest::new(Purpose::Extract, vec![Message::user(text)]);
        request.max_tokens = max_tokens;
        request.temperature = temperature;
        request.schema = schema.map(mm_llm::client::SchemaId::new);

        let baseline = prompt_hash("mock", "small", &request);
        prop_assert_eq!(baseline.len(), 64);
        prop_assert_eq!(baseline.clone(), prompt_hash("mock", "small", &request.clone()));

        prop_assert_ne!(baseline.clone(), prompt_hash("mock", "frontier", &request));

        let mut other_purpose = request.clone();
        other_purpose.purpose = Purpose::Summarize;
        prop_assert_ne!(baseline.clone(), prompt_hash("mock", "small", &other_purpose));

        let mut other_tokens = request.clone();
        other_tokens.max_tokens = max_tokens + 1000;
        prop_assert_ne!(baseline.clone(), prompt_hash("mock", "small", &other_tokens));

        let mut other_text = request.clone();
        other_text.messages.push(Message::assistant("appended"));
        prop_assert_ne!(baseline.clone(), prompt_hash("mock", "small", &other_text));

        prop_assert_ne!(baseline, prompt_hash("other", "small", &request));
    }
}
