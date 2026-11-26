//! Phase 7 exit criteria: Pi runs through the same agent contract, missing
//! binaries are unavailable, workspace escapes are forbidden, no shell is
//! advertised, and stale runs recover as failed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mm_agent::{AgentRun, AgentRuntime, PiBackend, PiSettings, TurnSummarizer};
use mm_cme::ToolHandler;
use mm_data::agentstream::EventType;
use mm_serve::cm_tools::CmTools;
use mm_serve::state_service::StateService;
use serde_json::{json, Value};

fn prompts() -> Arc<mm_prompt::PromptStore> {
    Arc::new(
        mm_prompt::PromptStore::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("prompt tree"),
    )
}

async fn data() -> (tempfile::TempDir, mm_data::InitResult) {
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    (temp, data)
}

async fn tools_with_pi(data: &mm_data::InitResult, temp: &Path, backend: PiBackend) -> CmTools {
    let prompts = prompts();
    let summarizer = TurnSummarizer::new(None, Arc::clone(&prompts), None);
    let runtime = Arc::new(
        AgentRuntime::new(
            Arc::clone(&data.agentstream),
            prompts,
            temp.to_path_buf(),
            None,
            summarizer,
        )
        .with_pi(Arc::new(backend)),
    );
    let state = Arc::new(
        StateService::new(
            Arc::clone(&data.engine),
            Arc::clone(&data.agentstream),
            prompts_for_state(),
            temp.to_path_buf(),
        )
        .unwrap(),
    );
    let session = state
        .open_session("phase-seven-user", None, 1_000)
        .await
        .unwrap()
        .session
        .session_id;
    CmTools::new(state)
        .with_session(session)
        .with_agent(runtime)
}

fn prompts_for_state() -> Arc<mm_prompt::PromptStore> {
    prompts()
}

async fn call(tools: &CmTools, arguments: Value) -> Value {
    let raw = tools
        .call("agent.run", &arguments.to_string())
        .await
        .expect("agent tool returned an envelope");
    serde_json::from_str(&raw).expect("valid JSON envelope")
}

#[cfg(unix)]
fn recorded_pi_script(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../mm-agent/tests/fixtures/pi_recorded_turn.jsonl");
    let script = dir.join("fake_pi");
    let body = format!(
        "#!/bin/sh\nread line\nid=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\nsed \"s/__ID__/$id/g\" '{}'\n",
        fixture.display()
    );
    std::fs::write(&script, body).unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();
    script
}

#[cfg(unix)]
#[tokio::test]
async fn a_recorded_pi_turn_replays_through_the_agent_contract() {
    let (temp, data) = data().await;
    let script = recorded_pi_script(temp.path());
    let backend = PiBackend::new(PiSettings {
        binary: script,
        provider: "recorded".into(),
        model: "pi-test".into(),
        timeout_ms: 5_000,
    });
    assert!(backend.probe().available);
    assert_eq!(backend.tools(), vec!["read", "write", "edit", "ls"]);
    assert!(!backend.tools().iter().any(|tool| tool == "bash"));
    let tools = tools_with_pi(&data, temp.path(), backend).await;

    let envelope = call(
        &tools,
        json!({"agent": "pi", "input": "Scaffold the module"}),
    )
    .await;
    assert_eq!(envelope["ok"], json!(true), "{envelope}");
    let result = &envelope["result"];
    assert_eq!(result["kind"], json!("pi"));
    assert_eq!(result["status"], json!("done"));
    assert_eq!(result["streamed"], json!(true));
    assert!(result["message"]
        .as_str()
        .unwrap()
        .contains("Module scaffolded"));
    assert!(result["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|artifact| artifact["id"] == "src/lib.rs"));
    assert!(result["toolCalls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "write"));
    let run_id = result["runId"].as_str().unwrap();
    let events = data
        .agentstream
        .events(&format!("agent/{run_id}"))
        .await
        .unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::AgentTurn));
}

#[tokio::test]
async fn a_missing_pi_binary_is_unavailable() {
    let (temp, data) = data().await;
    let backend = PiBackend::new(PiSettings {
        binary: PathBuf::from("/nonexistent/pi-that-is-not-installed"),
        provider: "recorded".into(),
        model: "pi-test".into(),
        timeout_ms: 500,
    });
    let tools = tools_with_pi(&data, temp.path(), backend).await;
    let envelope = call(&tools, json!({"agent": "pi", "input": "Scaffold"})).await;
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["error"]["code"], json!("UNAVAILABLE"));
    assert!(envelope["error"]["message"]
        .as_str()
        .unwrap()
        .contains("pi-that-is-not-installed"));
}

#[tokio::test]
async fn a_workspace_escape_is_forbidden() {
    let (temp, data) = data().await;
    let backend = PiBackend::new(PiSettings {
        binary: PathBuf::from("/nonexistent/pi-that-is-not-installed"),
        provider: "recorded".into(),
        model: "pi-test".into(),
        timeout_ms: 500,
    });
    let tools = tools_with_pi(&data, temp.path(), backend).await;
    let envelope = call(
        &tools,
        json!({"agent": "pi", "input": "Scaffold", "workspace": "../outside"}),
    )
    .await;
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["error"]["code"], json!("FORBIDDEN"));
}

#[tokio::test]
async fn stale_runs_recover_as_failed() {
    let (temp, data) = data().await;
    let stale = AgentRun {
        run_id: "01STALE".into(),
        session_id: "session".into(),
        agentstream_id: "agent/01STALE".into(),
        kind: mm_agent::AgentKind::Pi,
        status: mm_agent::AgentRunStatus::Running,
        turns: 1,
        task: "scaffold".into(),
        origin: mm_agent::AgentOrigin::Cm,
        started_at: 1,
        updated_at: 1,
        last_summary: Some("working".into()),
        last_error: None,
        usage: mm_agent::AgentUsage::default(),
        workspace: temp.path().join("workspace"),
    };
    std::fs::write(
        temp.path().join("agent-runs.json"),
        serde_json::to_vec_pretty(&vec![stale]).unwrap(),
    )
    .unwrap();
    let runtime = AgentRuntime::new(
        Arc::clone(&data.agentstream),
        prompts(),
        temp.path().to_path_buf(),
        None,
        TurnSummarizer::new(None, prompts(), None),
    );
    let recovered = runtime.status("01STALE").await.unwrap();
    assert_eq!(recovered.status, mm_agent::AgentRunStatus::Failed);
    assert_eq!(recovered.last_error.as_deref(), Some("server restarted"));
}
