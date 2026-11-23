//! Phase 2 exit criteria: the CM tool surface and its envelope behavior.

use std::sync::Arc;

use mm_cme::{
    ConversationRouter, Session, ToolHandler, Turn, TurnExecutor, TurnLoopPolicy, TurnPlan,
};
use mm_data::{SessionId, StatusValue, TurnId, UserId};
use mm_llm::{MockClient, ProviderToolCall, ScriptedResponse, ThresholdRouter};
use mm_serve::cm_tools::CmTools;
use mm_serve::state_service::StateService;
use mm_serve::tool_schemas::TOOL_NAMES;
use serde_json::Value;

fn prompts() -> Arc<mm_prompt::PromptStore> {
    Arc::new(
        mm_prompt::PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("prompt tree"),
    )
}

async fn fixture() -> (Arc<StateService>, String, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let state = Arc::new(
        StateService::new(
            Arc::clone(&data.engine),
            Arc::clone(&data.agentstream),
            prompts(),
            data.base_dir.clone(),
        )
        .unwrap(),
    );
    let session = state
        .open_session("phase-two-user", None, 1_000)
        .await
        .unwrap()
        .session
        .session_id;
    (state, session, temp)
}

fn parse(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap()
}

#[tokio::test]
async fn the_public_catalog_is_exactly_the_seventeen_schemas() {
    let (state, session, _temp) = fixture().await;
    let tools = CmTools::new(state).with_session(session);
    let names: Vec<String> = tools
        .schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();
    assert_eq!(names, TOOL_NAMES);
    for name in names {
        assert!(!name.starts_with("catalog."));
        assert!(!name.contains("shell"));
        assert!(!name.contains("url"));
    }
}

#[tokio::test]
async fn a_fake_model_can_drive_state_and_frames_until_done() {
    let (state, session, _temp) = fixture().await;
    let tools = Arc::new(CmTools::new(Arc::clone(&state)).with_session(session.clone()));
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![
            ProviderToolCall {
                id: "topic".into(),
                name: "conversation.topic".into(),
                arguments: r#"{"action":"set","topic":"auth refresh","expected_revision":1}"#
                    .into(),
            },
            ProviderToolCall {
                id: "depth".into(),
                name: "conversation.depth".into(),
                arguments: r#"{"action":"set","depth":2,"expected_revision":2}"#.into(),
            },
            ProviderToolCall {
                id: "status".into(),
                name: "conversation.status".into(),
                arguments: r#"{"action":"set","values":{"phase":"working"},"expected_revision":3}"#
                    .into(),
            },
            ProviderToolCall {
                id: "frame".into(),
                name: "conversation.frame".into(),
                arguments: r#"{"action":"push","template_id":"frame.work","expected_revision":1}"#
                    .into(),
            },
        ]),
        ScriptedResponse::text("I have the topic and the work frame in place."),
    ]));
    let router = ConversationRouter::with_table(
        Arc::new(ThresholdRouter::from_specs(
            vec![],
            Some("mock-small".into()),
        )),
        Default::default(),
    );
    let mut executor = TurnExecutor::with_llm(client, router)
        .with_tools(tools)
        .with_loop_policy(TurnLoopPolicy::ContinueUntilDone);
    assert_eq!(executor.loop_policy(), TurnLoopPolicy::ContinueUntilDone);

    let mut session_context = Session::new(UserId::new("phase-two-user"), None)
        .unwrap()
        .context;
    session_context
        .preferences
        .values
        .insert("tools".into(), 1.0);
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new(session.clone()),
        TurnId::new("phase2-turn"),
        "set up the work",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &session_context, None)
        .await
        .unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(outcome.tool_calls.len(), 4);
    let topic: Value = serde_json::from_str(&outcome.tool_calls[0].result).unwrap();
    assert_eq!(topic["state"]["topic"], "auth refresh");
    assert!(topic["stateRevision"].as_u64().unwrap() >= 1);
    let depth: Value = serde_json::from_str(&outcome.tool_calls[1].result).unwrap();
    assert_eq!(depth["state"]["depth"], 2);
    let status: Value = serde_json::from_str(&outcome.tool_calls[2].result).unwrap();
    assert_eq!(status["state"]["status"]["phase"], "working");
    let frame: Value = serde_json::from_str(&outcome.tool_calls[3].result).unwrap();
    assert_eq!(frame["result"]["top"]["template_id"], "frame.work");

    let bundle = state.load(&session).await.unwrap();
    assert_eq!(bundle.conversation.topic.as_deref(), Some("auth refresh"));
    assert_eq!(bundle.conversation.depth, 2);
    assert_eq!(bundle.frames.frames.len(), 2);
    assert_eq!(
        bundle.conversation.status.get("phase"),
        Some(&StatusValue::Text("working".into()))
    );
}

#[tokio::test]
async fn invalid_json_conflicts_and_topology_are_typed() {
    let (state, session, _temp) = fixture().await;
    let tools = CmTools::new(state).with_session(session.clone());

    let invalid = parse(&tools.call("conversation.topic", "{not json").await.unwrap());
    assert_eq!(invalid["error"]["code"], "INVALID_JSON");

    let first = parse(
        &tools
            .call(
                "conversation.topic",
                r#"{"action":"set","topic":"one","expected_revision":1}"#,
            )
            .await
            .unwrap(),
    );
    assert!(first["ok"].as_bool().unwrap());
    let stale = parse(
        &tools
            .call(
                "conversation.topic",
                r#"{"action":"set","topic":"two","expected_revision":1}"#,
            )
            .await
            .unwrap(),
    );
    assert_eq!(stale["error"]["code"], "CONFLICT");
    assert_eq!(stale["error"]["retryable"], true);
    assert_eq!(stale["state"]["topic"], "one");

    for key in ["topic", "depth", "frame_stack", "working"] {
        let raw = format!(r#"{{"action":"set","values":{{"{key}":"x"}},"expected_revision":2}}"#);
        let response = parse(&tools.call("conversation.status", &raw).await.unwrap());
        assert_eq!(
            response["error"]["code"], "INVALID_ARGUMENTS",
            "key {key}: {response}"
        );
    }
}

#[tokio::test]
async fn frame_schedule_and_cancel_are_full_state_operations() {
    let (state, session, _temp) = fixture().await;
    let tools = CmTools::new(state).with_session(session);

    let pushed = parse(
        &tools
            .call(
                "conversation.frame",
                r#"{"action":"push","template_id":"frame.work","expected_revision":1}"#,
            )
            .await
            .unwrap(),
    );
    assert_eq!(pushed["result"]["top"]["template_id"], "frame.work");
    let scheduled = parse(
        &tools
            .call(
                "conversation.frame",
                r#"{"action":"schedule","delay_ms":1000,"expected_revision":2}"#,
            )
            .await
            .unwrap(),
    );
    assert!(scheduled["ok"].as_bool().unwrap(), "{scheduled}");
    assert!(
        scheduled["result"]["utterance"]["due_at"]
            .as_i64()
            .unwrap_or(0)
            > 0,
        "{scheduled}"
    );
    assert_eq!(scheduled["result"]["utterance"]["reason"], "step");

    let cancelled = parse(
        &tools
            .call(
                "conversation.frame",
                r#"{"action":"cancel","expected_revision":3}"#,
            )
            .await
            .unwrap(),
    );
    assert!(cancelled["result"]["utterance"].is_null(), "{cancelled}");
    assert_eq!(cancelled["state"]["status"]["next_utterance_at"], 0);
    let popped = parse(
        &tools
            .call(
                "conversation.frame",
                r#"{"action":"pop","expected_revision":2}"#,
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        popped["result"]["stack"]["frames"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn agent_run_requires_a_runtime_and_stubs_are_visible() {
    let (state, session, _temp) = fixture().await;
    let tools = CmTools::new(state).with_session(session);
    let run = parse(
        &tools
            .call(
                "agent.run",
                r#"{"input":"Please inspect the auth refresh code."}"#,
            )
            .await
            .unwrap(),
    );
    assert_eq!(run["error"]["code"], "NOT_AVAILABLE");

    let rejected = parse(
        &tools
            .call("agent.run", r#"{"input":"do work","shell":"rm -rf /"}"#)
            .await
            .unwrap(),
    );
    assert_eq!(rejected["error"]["code"], "INVALID_ARGUMENTS");

    let stub = parse(&tools.call("data.search", r#"{"query":"x"}"#).await.unwrap());
    assert_eq!(stub["error"]["code"], "NOT_AVAILABLE");
}
