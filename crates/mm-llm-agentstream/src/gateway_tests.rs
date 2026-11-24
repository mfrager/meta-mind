use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mm_core::{Params, Tabular};
use mm_data::agentstream::{AgentstreamEvent, EventType, SessionMetadata};
use mm_data::config::EngineConfig;
use mm_data::{AgentstreamStore, RdfEngine};
use mm_llm::{
    CacheMode, CallContext, CollectSink, LlmClient, LlmConfig, LlmRequest, Message, MockClient,
    Purpose, SchemaId, ScriptedResponse,
};
use mm_log::{Level, Logger, RedactionPolicy};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;
use serde_json::{json, Value};

use crate::journal::JournalSink;
use crate::{
    AgentstreamLlm, GatewayParts, ProviderKind, ProviderProfile, ProviderProfiles, StructuredOutput,
};

const CONFIG: &str = r#"
[defaults]
decoder = "none"
max_repair_attempts = 1
timeout_ms = 5000
max_tokens = 128

[[model]]
name = "small"
input_cost_micros_per_1k = 0
output_cost_micros_per_1k = 0
typical_latency_ms = 1
complexity_ceiling = "very_high"

[[model]]
name = "mock-small"
mock = true
input_cost_micros_per_1k = 0
output_cost_micros_per_1k = 0
typical_latency_ms = 1
complexity_ceiling = "very_high"
"#;

#[derive(Default)]
struct FakeJournal {
    events: Mutex<Vec<AgentstreamEvent>>,
    starts: Mutex<Vec<String>>,
    fail_start: AtomicBool,
    fail_append: AtomicBool,
}

impl FakeJournal {
    fn events(&self) -> Vec<AgentstreamEvent> {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn starts(&self) -> Vec<String> {
        self.starts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

#[async_trait]
impl JournalSink for FakeJournal {
    async fn start_session(
        &self,
        session_id: &str,
        _metadata: SessionMetadata,
    ) -> std::result::Result<(), String> {
        if self.fail_start.load(Ordering::SeqCst) {
            return Err("journal unavailable".into());
        }
        self.starts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(session_id.to_owned());
        Ok(())
    }

    async fn append(
        &self,
        event: AgentstreamEvent,
    ) -> std::result::Result<AgentstreamEvent, String> {
        if self.fail_append.load(Ordering::SeqCst) {
            return Err("journal unavailable".into());
        }
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event.clone());
        Ok(event)
    }
}

struct Case {
    gateway: AgentstreamLlm,
    store: SqliteStore,
    journal: Arc<FakeJournal>,
    _graph: GraphStore,
    _dir: tempfile::TempDir,
}

struct Harness {
    dir: tempfile::TempDir,
    parts: GatewayParts,
    store: SqliteStore,
    graph: GraphStore,
    journal: Arc<FakeJournal>,
}

impl Harness {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("metamind.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("mm-shapes.ttl");
        let graph = GraphStore::in_memory(&shapes).await.unwrap();
        let logger = Arc::new(Logger::new(
            Level::Error,
            Vec::new(),
            None,
            RedactionPolicy::kernel_default(),
        ));
        let mut config = LlmConfig::from_toml_str(CONFIG).unwrap();
        config.cache_dir = dir.path().join("cache");
        let engine = RdfEngine::open(
            dir.path(),
            EngineConfig {
                persistent: false,
                ..EngineConfig::default()
            },
        )
        .await
        .unwrap();
        let agentstream = Arc::new(AgentstreamStore::new(Arc::new(engine), dir.path()).unwrap());
        let journal = Arc::new(FakeJournal::default());
        let parts = GatewayParts::new(
            config,
            agentstream,
            store.clone(),
            graph.handle().clone(),
            logger,
            crate::ProviderProfiles::standard(),
        )
        .with_journal_sink(journal.clone());
        Self {
            dir,
            parts,
            store,
            graph,
            journal,
        }
    }

    fn with_profiles(mut self, profiles: ProviderProfiles) -> Self {
        self.parts.profiles = profiles;
        self
    }

    fn case(self, client: Arc<dyn LlmClient>) -> Case {
        let mut parts = self.parts;
        parts.injected_client = Some(client);
        let gateway = AgentstreamLlm::builder(parts)
            .build()
            .expect("gateway builds");
        Case {
            gateway,
            store: self.store,
            journal: self.journal,
            _graph: self.graph,
            _dir: self.dir,
        }
    }
}

fn request(text: &str) -> LlmRequest {
    LlmRequest::new(Purpose::Converse, vec![Message::user(text)])
        .with_model("small")
        .with_cache(CacheMode::Use)
}

fn kinds(events: &[AgentstreamEvent]) -> Vec<EventType> {
    events.iter().map(|event| event.event_type).collect()
}

fn call_id(event: &AgentstreamEvent) -> String {
    event.payload["callId"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn mock(text: &str) -> Arc<MockClient> {
    Arc::new(MockClient::constant(
        ScriptedResponse::text(text).with_model("small"),
    ))
}

fn default_profile() -> ProviderProfile {
    ProviderProfile {
        name: "default".into(),
        kind: ProviderKind::OpenAiCompatible,
        endpoint_env: "TEST_BASE_URL".into(),
        api_key_env: "TEST_API_KEY".into(),
        model_env: None,
        model: None,
        timeout_ms: 20,
        stream: false,
        temperature: None,
        omit_max_tokens: false,
        structured_output: StructuredOutput::JsonObject,
    }
}

struct SlowClient;

#[async_trait]
impl LlmClient for SlowClient {
    async fn complete(&self, req: &LlmRequest) -> mm_llm::Result<mm_llm::LlmResponse> {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        Ok(mm_llm::LlmResponse {
            call_id: req.call_id.unwrap_or_default(),
            text: "late".into(),
            model: "small".into(),
            usage: mm_llm::Usage::default(),
            cached: false,
            schema_valid: true,
            decoder: mm_llm::DecoderKind::None,
            tool_calls: Vec::new(),
            finish_reason: None,
            provider_metadata: Default::default(),
            streamed: false,
        })
    }

    fn provider(&self) -> &str {
        "slow"
    }
}

#[tokio::test]
async fn one_successful_call_writes_request_attempt_response_and_shares_the_ledger_id() {
    let client = mock("hello");
    let case = Harness::new()
        .await
        .case(client.clone() as Arc<dyn LlmClient>);
    let context = CallContext::new("chat").with_session("sess-1");
    let response = mm_llm::scope(context, case.gateway.call(request("hello")))
        .await
        .unwrap();

    let events = case.journal.events();
    assert_eq!(
        kinds(&events),
        vec![
            EventType::LlmRequest,
            EventType::LlmAttempt,
            EventType::LlmResponse
        ]
    );
    assert_eq!(events[0].session_id, "sess-1");
    assert_eq!(case.journal.starts(), vec!["sess-1".to_owned()]);
    assert_eq!(client.calls(), 1);

    let id = mm_core::ulid_string(&response.call_id);
    assert_eq!(call_id(&events[0]), id);
    assert_eq!(call_id(&events[1]), id);
    assert_eq!(call_id(&events[2]), id);

    let rows = case
        .store
        .query_json("SELECT id FROM llm_calls", Params::new())
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], json!(id));
}

#[tokio::test]
async fn the_kill_switch_is_one_config_error_with_no_journal_or_provider_work() {
    let client = mock("hello");
    let mut case = Harness::new()
        .await
        .case(client.clone() as Arc<dyn LlmClient>);
    case.gateway.disable_for_test();

    let error = case.gateway.call(request("hello")).await.unwrap_err();
    assert_eq!(error.kind(), "config");
    assert!(error.to_string().contains(crate::DISABLED_REASON));
    let probe = case.gateway.models("default").await;
    assert!(!probe.available);
    assert_eq!(probe.reason.as_deref(), Some(crate::DISABLED_REASON));
    assert_eq!(client.calls(), 0);
    assert!(case.journal.events().is_empty());
}

#[tokio::test]
async fn a_failing_call_writes_request_attempt_and_error() {
    let client = Arc::new(MockClient::script(vec![ScriptedResponse::failing(
        "connection reset",
    )]));
    let case = Harness::new().await.case(client as Arc<dyn LlmClient>);
    let error = case.gateway.call(request("hello")).await.unwrap_err();

    assert_eq!(error.kind(), "transport");
    let events = case.journal.events();
    assert_eq!(
        kinds(&events),
        vec![
            EventType::LlmRequest,
            EventType::LlmAttempt,
            EventType::LlmError
        ]
    );
    assert_eq!(events[2].payload["kind"], "transport");
}

#[tokio::test]
async fn a_repair_round_writes_one_request_two_attempts_and_one_response() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("{}").with_model("small"),
        ScriptedResponse::text(r#"{"name":"ok"}"#).with_model("small"),
    ]));
    let case = Harness::new()
        .await
        .case(client.clone() as Arc<dyn LlmClient>);
    case.gateway
        .register_schema(
            SchemaId::new("greeting"),
            json!({
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
                "additionalProperties": false
            }),
        )
        .await
        .unwrap();

    let mut req = request("hello").with_schema(SchemaId::new("greeting"));
    req.purpose = Purpose::Extract;
    let response = case.gateway.call(req).await.unwrap();

    assert_eq!(response.text, r#"{"name":"ok"}"#);
    assert_eq!(client.calls(), 2);
    let events = case.journal.events();
    assert_eq!(
        kinds(&events),
        vec![
            EventType::LlmRequest,
            EventType::LlmAttempt,
            EventType::LlmAttempt,
            EventType::LlmResponse
        ]
    );
    assert_eq!(events[1].payload["attemptNo"], 1);
    assert_eq!(events[2].payload["attemptNo"], 2);
    assert_eq!(events[3].payload["attemptCount"], 2);
}

#[tokio::test]
async fn a_journal_failure_performs_zero_provider_calls() {
    let client = mock("hello");
    let case = Harness::new()
        .await
        .case(client.clone() as Arc<dyn LlmClient>);
    case.journal.fail_start.store(true, Ordering::SeqCst);

    let error = case.gateway.call(request("hello")).await.unwrap_err();
    assert_eq!(error.kind(), "journal");
    assert_eq!(client.calls(), 0);
    assert!(case.journal.events().is_empty());
}

#[tokio::test]
async fn a_cache_hit_writes_one_request_and_one_cached_response_without_an_attempt() {
    let client = mock("hello");
    let case = Harness::new()
        .await
        .case(client.clone() as Arc<dyn LlmClient>);
    let req = request("hello");

    let first = case.gateway.call(req.clone()).await.unwrap();
    let second = case.gateway.call(req).await.unwrap();
    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(client.calls(), 1);

    let events = case.journal.events();
    assert_eq!(
        kinds(&events),
        vec![
            EventType::LlmRequest,
            EventType::LlmAttempt,
            EventType::LlmResponse,
            EventType::LlmRequest,
            EventType::LlmResponse
        ]
    );
    assert_eq!(events[4].payload["cached"], true);
    assert_eq!(events[4].payload["status"], "cached");
    assert_eq!(events[4].payload["attemptCount"], 0);
}

#[tokio::test]
async fn a_streamed_call_writes_one_request_and_one_response_not_token_events() {
    let client = mock("hello");
    let case = Harness::new().await.case(client as Arc<dyn LlmClient>);
    let mut live = case.gateway.subscribe();
    let mut sink = CollectSink::default();

    let response = case
        .gateway
        .call_stream(request("hello"), &mut sink)
        .await
        .unwrap();
    assert_eq!(sink.0, "hello");
    assert!(response.streamed);

    let events = case.journal.events();
    assert_eq!(
        kinds(&events),
        vec![
            EventType::LlmRequest,
            EventType::LlmAttempt,
            EventType::LlmResponse
        ]
    );
    assert_eq!(events[0].payload["streamed"], true);
    assert_eq!(events[2].payload["streamed"], true);

    let mut live_kinds = Vec::new();
    while let Ok(event) = live.try_recv() {
        live_kinds.push(match event {
            crate::LiveLlmEvent::Request { .. } => "request",
            crate::LiveLlmEvent::Delta { .. } => "delta",
            crate::LiveLlmEvent::Response { .. } => "response",
            crate::LiveLlmEvent::Error { .. } => "error",
        });
    }
    assert_eq!(live_kinds, vec!["request", "delta", "response"]);
}

#[tokio::test]
async fn an_unknown_profile_is_a_config_error_before_any_journal_or_provider_work() {
    let client = mock("hello");
    let case = Harness::new()
        .await
        .case(client.clone() as Arc<dyn LlmClient>);
    let error = case
        .gateway
        .call(request("hello").with_provider("missing"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "config");
    assert_eq!(client.calls(), 0);
    assert!(case.journal.events().is_empty());
}

#[tokio::test]
async fn an_agent_call_journals_into_its_own_run_session_and_turn() {
    let client = mock("hello");
    let case = Harness::new().await.case(client as Arc<dyn LlmClient>);
    let context = CallContext::new("agent").with_run("run-42");
    let mut req = request("hello");
    req.purpose = Purpose::Agent;

    mm_llm::scope(context, case.gateway.call(req))
        .await
        .unwrap();
    let events = case.journal.events();
    assert_eq!(case.journal.starts(), vec!["agent/run-42".to_owned()]);
    assert_eq!(events[0].session_id, "agent/run-42");
    assert_eq!(events[0].turn_id.as_deref(), Some("run-42"));
}

#[tokio::test]
async fn the_profile_timeout_bounds_a_slow_provider_and_is_journaled_as_timeout() {
    let case = Harness::new()
        .await
        .with_profiles(ProviderProfiles::new(vec![default_profile()]))
        .case(Arc::new(SlowClient));
    let error = case.gateway.call(request("hello")).await.unwrap_err();
    assert_eq!(error.kind(), "timeout");
    let events = case.journal.events();
    assert_eq!(
        kinds(&events),
        vec![
            EventType::LlmRequest,
            EventType::LlmAttempt,
            EventType::LlmError
        ]
    );
    assert_eq!(events[1].payload["status"], "timeout");
}

#[tokio::test]
async fn a_hermes_profile_pins_its_model_and_provider_on_the_request() {
    let client = Arc::new(MockClient::constant(
        ScriptedResponse::text("hello").with_model("agent-model"),
    ));
    let hermes = ProviderProfile {
        name: "hermes".into(),
        kind: ProviderKind::OpenAiCompatible,
        endpoint_env: "TEST_HERMES_URL".into(),
        api_key_env: "TEST_HERMES_KEY".into(),
        model_env: None,
        model: Some("agent-model".into()),
        timeout_ms: 1000,
        stream: true,
        temperature: Some(0.2),
        omit_max_tokens: true,
        structured_output: StructuredOutput::JsonObject,
    };
    let case = Harness::new()
        .await
        .with_profiles(ProviderProfiles::new(vec![hermes]))
        .case(client.clone() as Arc<dyn LlmClient>);
    let req = LlmRequest::new(Purpose::Agent, vec![Message::user("hello")]).with_provider("hermes");
    case.gateway.call(req).await.unwrap();

    let seen = client.last().unwrap();
    assert_eq!(seen.provider.as_deref(), Some("hermes"));
    assert_eq!(seen.model.as_deref(), Some("agent-model"));
    assert_eq!(seen.temperature, 0.2);
    let events = case.journal.events();
    assert_eq!(events[0].payload["provider"], "hermes");
    assert_eq!(events[0].payload["model"], "agent-model");
}

#[tokio::test]
async fn an_unconfigured_model_probe_is_journaled_without_dialing() {
    let mut profile = default_profile();
    profile.name = "probe-missing".into();
    profile.endpoint_env = "MM_TEST_PROBE_ABSENT_URL".into();
    profile.api_key_env = "MM_TEST_PROBE_ABSENT_KEY".into();
    let case = Harness::new()
        .await
        .with_profiles(ProviderProfiles::new(vec![profile]))
        .case(mock("unused") as Arc<dyn LlmClient>);

    let probe = case.gateway.models("probe-missing").await;
    assert!(!probe.available);
    assert!(probe.models.is_empty());
    assert!(
        probe
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("MM_TEST_PROBE_ABSENT_URL"),
        "{probe:?}"
    );

    let events = case.journal.events();
    assert_eq!(kinds(&events), vec![EventType::LlmModels]);
    assert_eq!(events[0].session_id, "llm/models/probe-missing");
    assert_eq!(events[0].payload["available"], json!(false));
}

#[test]
fn event_payloads_never_include_the_prompt() {
    let value = json!({"messageCount": 1, "promptHash": "hash"});
    let object: &serde_json::Map<String, Value> = value.as_object().unwrap();
    assert!(!object.contains_key("messages"));
    assert!(!object.contains_key("content"));
    assert_eq!(object.get("messageCount").and_then(Value::as_u64), Some(1));
}
