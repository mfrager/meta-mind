//! Phase 6 exit criteria: Hermes turns, isolated agentstreams, continuation,
//! summarization, terminal runs, and unreachable-service errors.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
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
}

async fn models() -> Json<Value> {
    Json(json!({ "data": [{ "id": "fake-hermes" }] }))
}

async fn completions(State(state): State<FakeHermes>, Json(request): Json<Value>) -> Response {
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

async fn fake_hermes(stream: bool, long_first: bool) -> String {
    let app = Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(completions))
        .with_state(FakeHermes {
            stream,
            long_first,
            calls: Arc::new(AtomicUsize::new(0)),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}/v1")
}

struct Fixture {
    tools: CmTools,
    agentstream: Arc<mm_data::AgentstreamStore>,
    _temp: tempfile::TempDir,
}

fn prompts() -> Arc<mm_prompt::PromptStore> {
    Arc::new(
        mm_prompt::PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("prompt tree"),
    )
}

async fn fixture(base_url: &str, stream: bool, _long_first: bool) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let prompts = prompts();
    let hermes = Arc::new(HermesBackend::new(HermesConfig {
        base_url: base_url.to_owned(),
        api_key: Some("super-secret".into()),
        model: Some("fake-hermes".into()),
        stream,
        timeout_ms: 5_000,
        max_context_tokens: 96_000,
    }));
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
        _temp: temp,
    }
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
    let base_url = fake_hermes(false, false).await;
    let fixture = fixture(&base_url, false, false).await;

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
}

#[tokio::test]
async fn sse_turns_are_accumulated_and_marked_streamed() {
    let base_url = fake_hermes(true, false).await;
    let fixture = fixture(&base_url, true, false).await;
    let first = call(&fixture.tools, json!({"input": "Inspect the test"})).await;
    assert_ok(&first);
    assert_eq!(first["result"]["streamed"], json!(true));
    assert_eq!(
        first["result"]["message"],
        json!("I inspected the test and found a seed-dependent clock path.")
    );
    assert_eq!(first["result"]["usage"]["completionTokens"], json!(34));
}

#[tokio::test]
async fn long_turns_are_summarized_while_the_raw_event_is_retained() {
    let base_url = fake_hermes(false, true).await;
    let fixture = fixture(&base_url, false, true).await;
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
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let prompts = prompts();
    let hermes = Arc::new(HermesBackend::new(HermesConfig {
        base_url: "http://127.0.0.1:1/v1".into(),
        api_key: Some("super-secret".into()),
        model: Some("fake".into()),
        stream: false,
        timeout_ms: 500,
        max_context_tokens: 96_000,
    }));
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
