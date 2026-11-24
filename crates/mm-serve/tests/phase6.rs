//! Phase 6 exit criteria: Hermes turns, isolated agentstreams, continuation,
//! summarization, terminal runs, and unreachable-service errors.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mm_agent::hermes::{API_KEY_ENV, BASE_URL_ENV, MODEL_ENV};
use mm_agent::{AgentRuntime, HermesBackend, HermesConfig, TurnSummarizer};
use mm_cme::ToolHandler;
use mm_data::agentstream::EventType;
use mm_serve::cm_tools::CmTools;
use mm_serve::state_service::StateService;
use serde_json::{json, Value};

#[derive(Clone)]
struct FakeHermes {
    stream: bool,
    long_first: bool,
    calls: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
}

async fn models() -> Json<Value> {
    Json(json!({ "data": [{ "id": "fake-hermes" }] }))
}

async fn completions(State(state): State<FakeHermes>, Json(request): Json<Value>) -> Response {
    state
        .bodies
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(request.clone());
    let turn = request
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
                .count()
        })
        .unwrap_or(1)
        .max(1);
    state.calls.fetch_add(1, Ordering::SeqCst);
    if state.long_first && turn == 1 {
        return Json(json!({
            "model": "fake-hermes",
            "mm_status": "running",
            "choices": [{ "message": { "content": "long ".repeat(7_000) }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 100, "completion_tokens": 7000 }
        }))
        .into_response();
    }
    let status = if turn < 3 { "running" } else { "done" };
    let content = if turn >= 3 {
        "The refresh test is fixed; the suite passes."
    } else {
        "I inspected the test and found a seed-dependent clock path."
    };
    if state.stream {
        let body = format!(
            concat!(
                "data: {{\"model\":\"fake-hermes\",\"mm_status\":\"{}\",",
                "\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
                "data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}],",
                "\"usage\":{{\"prompt_tokens\":12,\"completion_tokens\":34}}}}\n\n",
                "data: [DONE]\n\n"
            ),
            status, content
        );
        return ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response();
    }
    Json(json!({
        "model": "fake-hermes",
        "mm_status": status,
        "choices": [{ "message": { "content": content }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 34 }
    }))
    .into_response()
}

async fn fake_hermes(stream: bool, long_first: bool) -> (String, Arc<Mutex<Vec<Value>>>) {
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(completions))
        .with_state(FakeHermes {
            stream,
            long_first,
            calls: Arc::new(AtomicUsize::new(0)),
            bodies: Arc::clone(&bodies),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/v1"), bodies)
}

struct Fixture {
    tools: CmTools,
    agentstream: Arc<mm_data::AgentstreamStore>,
    bodies: Arc<Mutex<Vec<Value>>>,
    _temp: tempfile::TempDir,
    _env: tokio::sync::OwnedMutexGuard<()>,
}

static ENV_LOCK: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();

async fn hermes_env(base_url: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = ENV_LOCK.get_or_init(|| Arc::new(tokio::sync::Mutex::new(())));
    let guard = Arc::clone(lock).lock_owned().await;
    std::env::set_var(BASE_URL_ENV, base_url);
    std::env::set_var(API_KEY_ENV, "super-secret");
    std::env::set_var(MODEL_ENV, "fake-hermes");
    guard
}

fn prompts() -> Arc<mm_prompt::PromptStore> {
    Arc::new(
        mm_prompt::PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("prompt tree"),
    )
}

async fn fixture(
    base_url: &str,
    stream: bool,
    _long_first: bool,
    bodies: Arc<Mutex<Vec<Value>>>,
) -> Fixture {
    let env = hermes_env(base_url).await;
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let prompts = prompts();
    let gateway = fake_gateway(&data, temp.path()).await;
    let hermes = Arc::new(HermesBackend::with_gateway(
        HermesConfig {
            base_url: base_url.to_owned(),
            api_key: Some("super-secret".into()),
            model: Some("fake-hermes".into()),
            stream,
            timeout_ms: 5_000,
            max_context_tokens: 96_000,
        },
        gateway,
    ));
    hermes.refresh().await;
    let summarizer =
        TurnSummarizer::new(None, Arc::clone(&prompts), None).with_thresholds(1_000, 100_000);
    let runtime = Arc::new(AgentRuntime::new(
        Arc::clone(&data.agentstream),
        prompts,
        data.base_dir.clone(),
        Some(hermes),
        summarizer,
    ));
    let state = Arc::new(
        StateService::new(
            Arc::clone(&data.engine),
            Arc::clone(&data.agentstream),
            prompt_store(),
            data.base_dir.clone(),
        )
        .unwrap(),
    );
    let session = state
        .open_session("phase-six-user", None, 1_000)
        .await
        .unwrap()
        .session
        .session_id;
    let tools = CmTools::new(state)
        .with_session(session)
        .with_agent(runtime);
    Fixture {
        tools,
        agentstream: Arc::clone(&data.agentstream),
        bodies,
        _temp: temp,
        _env: env,
    }
}

async fn fake_gateway(
    data: &mm_data::InitResult,
    dir: &std::path::Path,
) -> Arc<mm_llm_agentstream::AgentstreamLlm> {
    let sqlite = mm_store_sqlite::SqliteStore::open(&dir.join("kernel.db"))
        .await
        .unwrap();
    sqlite.migrate().await.unwrap();
    Arc::new(
        mm_llm_agentstream::GatewayBuilder::new(mm_llm_agentstream::GatewayParts::new(
            fake_llm_config(dir),
            Arc::clone(&data.agentstream),
            sqlite,
            data.engine.handle().clone(),
            Arc::new(mm_log::Logger::new(
                mm_log::Level::Error,
                Vec::new(),
                None,
                mm_log::RedactionPolicy::kernel_default(),
            )),
            mm_llm_agentstream::ProviderProfiles::standard(),
        ))
        .build()
        .expect("gateway builds"),
    )
}

fn fake_llm_config(dir: &std::path::Path) -> mm_llm::LlmConfig {
    let mut config = mm_llm::LlmConfig {
        cache_dir: dir.join("llm-cache"),
        ..mm_llm::LlmConfig::default()
    };
    for name in ["fake-hermes", "fake"] {
        config.models.push(mm_llm::ModelSpec {
            name: name.into(),
            input_cost_micros_per_1k: 0,
            output_cost_micros_per_1k: 0,
            typical_latency_ms: 1,
            complexity_ceiling: mm_llm::Complexity::VeryHigh,
            mock: false,
        });
    }
    config
}

fn prompt_store() -> Arc<mm_prompt::PromptStore> {
    prompts()
}

async fn call(tools: &CmTools, arguments: Value) -> Value {
    let raw = tools
        .call("agent.run", &arguments.to_string())
        .await
        .expect("agent tool returned an envelope");
    serde_json::from_str(&raw).expect("valid JSON envelope")
}

fn assert_ok(envelope: &Value) {
    assert_eq!(envelope["ok"], json!(true), "{envelope}");
}

#[tokio::test]
async fn a_run_produces_three_turns_and_a_terminal_error_on_the_fourth() {
    let (base_url, bodies) = fake_hermes(false, false).await;
    let fixture = fixture(&base_url, false, false, Arc::clone(&bodies)).await;

    let first = call(
        &fixture.tools,
        json!({"input": "Investigate the refresh flake"}),
    )
    .await;
    assert_ok(&first);
    let run_id = first["result"]["runId"].as_str().unwrap().to_owned();
    assert_eq!(first["result"]["turn"], json!(1));
    assert_eq!(first["result"]["status"], json!("running"));
    assert_eq!(first["result"]["next"], json!("continue"));
    assert!(first["result"]["rawRef"]
        .as_str()
        .unwrap()
        .contains("agentstream/agent/"));
    assert!(!first["result"]["workspace"].as_str().unwrap().is_empty());

    let request = fixture
        .bodies
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .first()
        .cloned()
        .expect("one provider request");
    assert_eq!(request["model"], json!("fake-hermes"));
    assert_eq!(request["stream"], json!(false));
    assert!(
        (request["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6,
        "{request}"
    );
    assert!(request.get("max_tokens").is_none(), "{request}");
    assert_eq!(request["messages"][0]["role"], json!("system"));

    let second = call(
        &fixture.tools,
        json!({"run_id": run_id, "input": "Also try seed 42."}),
    )
    .await;
    assert_ok(&second);
    assert_eq!(second["result"]["turn"], json!(2));
    assert_eq!(second["result"]["status"], json!("running"));

    let third = call(
        &fixture.tools,
        json!({"run_id": run_id, "input": "Continue."}),
    )
    .await;
    assert_ok(&third);
    assert_eq!(third["result"]["turn"], json!(3));
    assert_eq!(third["result"]["status"], json!("done"));
    assert_eq!(third["result"]["next"], json!("done"));

    let fourth = call(
        &fixture.tools,
        json!({"run_id": run_id, "input": "Continue."}),
    )
    .await;
    assert_eq!(fourth["ok"], json!(false));
    assert_eq!(fourth["error"]["code"], json!("RUN_TERMINAL"));

    let events = fixture
        .agentstream
        .events(&format!("agent/{run_id}"))
        .await
        .unwrap();
    let types: Vec<&str> = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    assert!(types.contains(&"user_message"));
    assert!(types.contains(&"agent_turn"));
    assert!(types.contains(&"agent_status"));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::AgentTurn)
            .count(),
        3
    );
    for event_type in [
        EventType::LlmRequest,
        EventType::LlmAttempt,
        EventType::LlmResponse,
    ] {
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == event_type)
                .count(),
            3,
            "{event_type:?}"
        );
    }
    let probe = fixture
        .agentstream
        .events("llm/models/hermes")
        .await
        .unwrap();
    let probe = probe
        .iter()
        .find(|event| event.event_type == EventType::LlmModels)
        .expect("models probe event");
    assert_eq!(probe.payload["available"], json!(true));
    assert_eq!(probe.payload["models"], json!(["fake-hermes"]));
}

#[tokio::test]
async fn sse_turns_are_accumulated_and_marked_streamed() {
    let (base_url, bodies) = fake_hermes(true, false).await;
    let fixture = fixture(&base_url, true, false, Arc::clone(&bodies)).await;
    let first = call(&fixture.tools, json!({"input": "Inspect the test"})).await;
    assert_ok(&first);
    assert_eq!(first["result"]["streamed"], json!(true));
    assert_eq!(first["result"]["status"], json!("running"));
    assert_eq!(first["result"]["next"], json!("continue"));
    assert_eq!(
        first["result"]["message"],
        json!("I inspected the test and found a seed-dependent clock path.")
    );
    assert_eq!(first["result"]["usage"]["completionTokens"], json!(34));
    let request = fixture
        .bodies
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .first()
        .cloned()
        .expect("one provider request");
    assert_eq!(request["stream"], json!(true));
    assert!(request.get("max_tokens").is_none(), "{request}");
}

#[tokio::test]
async fn a_json_answer_falls_back_when_streaming_was_requested() {
    let (base_url, bodies) = fake_hermes(false, false).await;
    let fixture = fixture(&base_url, true, false, Arc::clone(&bodies)).await;
    let first = call(&fixture.tools, json!({"input": "Inspect the test"})).await;
    assert_ok(&first);
    assert_eq!(first["result"]["streamed"], json!(false));
    assert_eq!(
        first["result"]["message"],
        json!("I inspected the test and found a seed-dependent clock path.")
    );
    let bodies = fixture
        .bodies
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert_eq!(bodies.len(), 2, "the adapter retried without SSE");
    assert_eq!(bodies[0]["stream"], json!(true));
    assert_eq!(bodies[1]["stream"], json!(false));
}

#[tokio::test]
async fn long_turns_are_summarized_while_the_raw_event_is_retained() {
    let (base_url, bodies) = fake_hermes(false, true).await;
    let fixture = fixture(&base_url, false, true, Arc::clone(&bodies)).await;
    let first = call(
        &fixture.tools,
        json!({"input": "Do a very long investigation"}),
    )
    .await;
    assert_ok(&first);
    let run_id = first["result"]["runId"].as_str().unwrap().to_owned();
    assert_eq!(first["result"]["summary"]["summaryFailed"], json!(true));
    assert!(first["result"]["message"].as_str().unwrap().chars().count() < 1_000);
    let events = fixture
        .agentstream
        .events(&format!("agent/{run_id}"))
        .await
        .unwrap();
    let summary_event = events
        .iter()
        .find(|event| event.event_type == EventType::AgentTurnSummary)
        .expect("summary event");
    assert_eq!(
        summary_event.payload["summary"]["summaryFailed"],
        json!(true)
    );
    let raw_event = events
        .iter()
        .find(|event| event.event_type == EventType::AgentTurn)
        .expect("turn event");
    assert_eq!(
        raw_event.payload["raw"].as_str().unwrap().chars().count(),
        35_000
    );
}

#[tokio::test]
async fn unreachable_hermes_is_unavailable_with_a_redacted_reason() {
    let base_url = "http://127.0.0.1:1/v1";
    let _env = hermes_env(base_url).await;
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let prompts = prompts();
    let gateway = fake_gateway(&data, temp.path()).await;
    let hermes = Arc::new(HermesBackend::with_gateway(
        HermesConfig {
            base_url: base_url.into(),
            api_key: Some("super-secret".into()),
            model: Some("fake".into()),
            stream: false,
            timeout_ms: 500,
            max_context_tokens: 96_000,
        },
        gateway,
    ));
    hermes.refresh().await;
    let runtime = Arc::new(AgentRuntime::new(
        Arc::clone(&data.agentstream),
        Arc::clone(&prompts),
        data.base_dir.clone(),
        Some(hermes),
        TurnSummarizer::new(None, prompts, None),
    ));
    let state = Arc::new(
        StateService::new(
            Arc::clone(&data.engine),
            Arc::clone(&data.agentstream),
            prompt_store(),
            data.base_dir.clone(),
        )
        .unwrap(),
    );
    let session = state
        .open_session("phase-six-user", None, 1_000)
        .await
        .unwrap()
        .session
        .session_id;
    let tools = CmTools::new(state)
        .with_session(session)
        .with_agent(runtime);
    let envelope = call(&tools, json!({"input": "Check the flake"})).await;
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["error"]["code"], json!("UNAVAILABLE"));
    let message = envelope["error"]["message"].as_str().unwrap();
    assert!(message.contains("http://127.0.0.1:1/v1"), "{message}");
    assert!(!message.contains("super-secret"), "{message}");
}
