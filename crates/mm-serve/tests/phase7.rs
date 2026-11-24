//! Phase 7 integration: CME routing, session introspection, migration and
//! multi-session isolation.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use mm_serve::controller::ControllerModel;
use mm_serve::feature_flags::FeatureFlags;
use mm_serve::routes::router;
use mm_serve::state::{AppState, ConsoleLock};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

async fn boot_cme() -> (TempDir, Arc<AppState>, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = mm_core::Config::for_data_dir(dir.path());
    let lock = ConsoleLock::acquire(dir.path()).unwrap();
    let flags = FeatureFlags {
        cme_enabled: true,
        // These tests target the conversation-manager contract and its tool
        // surface, not the three-pass runner, which is on by default.
        three_pass_v2: false,
        ..FeatureFlags::default()
    };
    let state = Arc::new(
        AppState::boot_with_flags(cfg, lock, flags)
            .await
            .expect("boot with CME"),
    );
    *state.controller_model.lock().unwrap() = mm_serve::controller::ControllerModel::Off;
    let app = router(Arc::clone(&state));
    (dir, state, app)
}

fn chat_request(session_id: &str, query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/cme/chat")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "sessionId": session_id, "query": query }).to_string(),
        ))
        .unwrap()
}

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

#[tokio::test]
async fn cme_chat_returns_frames_and_the_session_snapshot_and_metrics() {
    let (_dir, state, app) = boot_cme().await;
    let session_id = state.ids.next_string();
    state.chat.create_session(session_id.clone()).unwrap();

    let response = app
        .clone()
        .oneshot(chat_request(&session_id, "hello cme"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw = String::from_utf8(body_bytes(response).await).unwrap();
    assert!(raw.contains("\"type\":\"started\""), "{raw}");
    assert!(raw.contains("\"type\":\"run\""), "{raw}");
    assert!(raw.contains("\"text\":\"Echo: hello cme\""), "{raw}");
    assert!(raw.contains("\"type\":\"final_response\""), "{raw}");
    assert!(raw.contains("\"type\":\"completed\""), "{raw}");

    let snapshot = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/cme/sessions/{session_id}/snapshot"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.status(), StatusCode::OK);
    let snapshot_body: Value = serde_json::from_slice(&body_bytes(snapshot).await).unwrap();
    assert_eq!(snapshot_body["session_id"], session_id);
    assert_eq!(snapshot_body["turns_completed"], 1);
    assert_eq!(snapshot_body["session"]["state"], "active");
    assert_eq!(snapshot_body["proactive"]["state"], "idle");

    let metrics = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/cme/sessions/{session_id}/metrics"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(metrics.status(), StatusCode::OK);
    let metrics_text = String::from_utf8(body_bytes(metrics).await).unwrap();
    assert!(
        metrics_text.contains("cme_sm_transitions_total"),
        "{metrics_text}"
    );
    assert!(metrics_text.contains("cme_observability_uptime_ms"));

    let debug = app
        .oneshot(
            Request::builder()
                .uri("/debug/cme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(debug.status(), StatusCode::OK);
    let page = String::from_utf8(body_bytes(debug).await).unwrap();
    assert!(page.contains(&session_id));
}

#[tokio::test]
async fn cme_logs_the_turn_and_frames_to_unified_memory() {
    let (_dir, state, app) = boot_cme().await;
    let session_id = state.ids.next_string();
    state.chat.create_session(session_id.clone()).unwrap();

    let response = app
        .oneshot(chat_request(&session_id, "remember this turn"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = body_bytes(response).await;

    let turns = state
        .memory_system
        .episodic
        .turns(&mm_data::SessionId::new(session_id.clone()))
        .await
        .unwrap();
    assert_eq!(turns.len(), 1, "the memory bridge logs one episodic turn");
    let events = state.agentstream.events(&session_id).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == mm_data::EventType::UserMessage),
        "the user message is in agentstream"
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == mm_data::EventType::AgentMessage),
        "the agent frames are in agentstream"
    );
}

#[tokio::test]
async fn the_chat_route_uses_cme_when_the_feature_flag_is_on() {
    let (_dir, _state, app) = boot_cme().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"query":"flagged"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw = String::from_utf8(body_bytes(response).await).unwrap();
    assert!(raw.contains("\"text\":\"Echo: flagged\""), "{raw}");
}

#[tokio::test]
async fn cme_publishes_transitions_to_the_session_event_stream() {
    let (_dir, state, _app) = boot_cme().await;
    let session_id = state.ids.next_string();
    state.chat.create_session(session_id.clone()).unwrap();
    let executor = state.cme_executor();
    state
        .cme
        .run_turn(&session_id, "warm up".into(), executor)
        .await
        .unwrap();

    let mut events = state.cme.subscribe(&session_id).expect("event stream");
    let executor = state.cme_executor();
    state
        .cme
        .run_turn(&session_id, "stream this".into(), executor)
        .await
        .unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
        .await
        .expect("a transition within the timeout")
        .expect("a transition");
    assert!(!event.sm_type.is_empty());
    assert!(!event.event.is_empty());
}

#[tokio::test]
async fn five_sessions_keep_their_own_state_and_counters() {
    let (_dir, state, _app) = boot_cme().await;
    let mut ids = Vec::new();
    for persona in 0..5 {
        let id = state.ids.next_string();
        state.chat.create_session(id.clone()).unwrap();
        let executor = state.cme_executor();
        let outcome = state
            .cme
            .run_turn(&id, format!("persona {persona}"), executor)
            .await
            .unwrap();
        assert!(outcome.is_success());
        ids.push(id);
    }
    for id in &ids {
        let snapshot = state.cme.snapshot(id).expect("snapshot");
        assert_eq!(snapshot.turns_completed, 1);
        assert_eq!(snapshot.proactive_queue_length, 0);
        assert_eq!(snapshot.session_id, *id);
        assert_eq!(snapshot.preferences.values.get("tools"), Some(&1.0));
    }
}

#[tokio::test]
async fn cme_exposes_the_conversation_tool_contract() {
    let (_dir, state, app) = boot_cme().await;
    let mock = Arc::new(mm_llm::MockClient::script(vec![
        mm_llm::ScriptedResponse::text("").with_tool_calls(vec![mm_llm::ProviderToolCall {
            id: "call-topic".into(),
            name: "conversation.topic".into(),
            arguments: r#"{"action":"set","topic":"the guide","expected_revision":1}"#.into(),
        }]),
        mm_llm::ScriptedResponse::text("The topic is recorded."),
    ]));
    *state.controller_model.lock().unwrap() = ControllerModel::Scripted(mock.clone());

    let session_id = state.ids.next_string();
    state.chat.create_session(session_id.clone()).unwrap();
    let response = app
        .oneshot(chat_request(&session_id, "set the topic"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw = String::from_utf8(body_bytes(response).await).unwrap();
    assert!(raw.contains(r#""type":"tool_call""#), "{raw}");
    assert!(raw.contains(r#""name":"conversation.topic""#), "{raw}");
    assert!(raw.contains(r#""type":"tool_result""#), "{raw}");
    assert!(raw.contains(r#""success":true"#), "{raw}");
    assert!(raw.contains("The topic is recorded."), "{raw}");

    let seen = mock.seen();
    assert!(seen.len() >= 2, "the model sees the tool result");
    assert!(
        seen[0]
            .tools
            .iter()
            .any(|tool| tool.name == "conversation.topic"),
        "the conversation surface is advertised"
    );
    assert_eq!(
        seen[0].tools.len(),
        17,
        "the CM sees exactly the seventeen reviewed schemas"
    );
    assert!(
        seen[1].messages.iter().any(|message| {
            message.role == mm_llm::Role::Tool
                && message.name.as_deref() == Some("conversation.topic")
        }),
        "the tool result is fed back to the model"
    );
    let state_bundle = state.state_service.load(&session_id).await.unwrap();
    assert_eq!(
        state_bundle.conversation.topic.as_deref(),
        Some("the guide")
    );

    let events = state.agentstream.events(&session_id).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == mm_data::EventType::ToolCall),
        "the tool call is in agentstream"
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == mm_data::EventType::ToolResult),
        "the tool result is in agentstream"
    );
}

#[tokio::test]
async fn migration_copies_sqlite_graph_and_legacy_chat_sessions() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();

    // A valid legacy SQLite database and a valid (empty) RocksDB store.
    let old_sqlite = mm_data::SqliteStore::open(&old.path().join("sqlite"))
        .await
        .unwrap();
    drop(old_sqlite);
    std::fs::copy(
        old.path().join("sqlite/metamind.db"),
        old.path().join("metamind.db"),
    )
    .unwrap();
    let shapes = mm_core::Config::for_data_dir(old.path()).shapes_file();
    let graph = mm_store_graph::GraphStore::open(&old.path().join("graph"), &shapes)
        .await
        .unwrap();
    drop(graph);

    let session_id = "01jphase7legacysession0000";
    std::fs::write(
        old.path().join("chat-sessions.json"),
        json!({
            "sessions": {
                session_id: { "title": "Legacy conversation", "run_ids": ["r1"] }
            },
            "runs": {}
        })
        .to_string(),
    )
    .unwrap();

    let report = mm_data::migrate_from_mm_serve(old.path(), new.path())
        .await
        .unwrap();
    assert!(report.sqlite_copied);
    assert!(report.graph_copied);
    assert_eq!(report.sessions_imported, 1);
    assert!(new.path().join("sqlite/metamind.db").is_file());
    assert!(new.path().join("cubecast/main").is_dir());

    let reopened = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(new.path().to_path_buf()),
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let record = reopened
        .memory_system
        .agentstream
        .session(session_id)
        .await
        .unwrap()
        .expect("migrated session");
    assert_eq!(record.title.as_deref(), Some("Legacy conversation"));
}

/// A lightweight load smoke. It is ignored by default; run with
/// `cargo test -p mm-serve --test phase7 -- --ignored` when benchmarking.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "load smoke: 25 sessions x 2 turns"]
async fn cme_load_smoke() {
    let (_dir, state, _app) = boot_cme().await;
    for persona in 0..25 {
        let id = state.ids.next_string();
        state.chat.create_session(id.clone()).unwrap();
        for turn in 0..2 {
            let executor = state.cme_executor();
            let outcome = state
                .cme
                .run_turn(&id, format!("load {persona}-{turn}"), executor)
                .await
                .unwrap();
            assert!(outcome.is_success());
        }
        let snapshot = state.cme.snapshot(&id).expect("snapshot");
        assert_eq!(snapshot.turns_completed, 2);
    }
}
