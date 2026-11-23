//! Phase 5: one durable process step runs Process → Verify without Interpret.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use mm_cme::{
    CmeError, Result as CmeResult, StepPassRequest, ThreePassConfig, ThreePassRunner, ToolHandler,
};
use mm_llm::{MockClient, ProviderToolCall, ScriptedResponse, ToolSchema};
use mm_prompt::PromptStore;

struct NoTools;

#[async_trait]
impl ToolHandler for NoTools {
    fn schemas(&self) -> Vec<ToolSchema> {
        Vec::new()
    }

    async fn call(&self, _name: &str, _arguments: &str) -> CmeResult<String> {
        Err(CmeError::Tool("no tools".into()))
    }
}

fn store() -> Arc<PromptStore> {
    Arc::new(
        PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .unwrap(),
    )
}

fn verdict(pass: bool, reasoning: &str) -> ScriptedResponse {
    ScriptedResponse::text(
        serde_json::json!({ "pass": pass, "reasoning": reasoning, "confidence": 0.9 }).to_string(),
    )
}

fn request() -> StepPassRequest {
    StepPassRequest {
        session_id: "session-1".into(),
        step_id: "step-1".into(),
        task: "ship the fix".into(),
        title: "Implement".into(),
        instructions: "Edit the parser and run its tests.".into(),
        prior: BTreeMap::new(),
        doc: Some("Run the checks before claiming success.".into()),
        check: Some("the parser tests pass".into()),
        model: Some("fast".into()),
    }
}

#[tokio::test]
async fn a_step_runs_process_then_verify_and_returns_tokens() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("parser fixed; tests pass"),
        verdict(true, "the tests pass"),
        ScriptedResponse::text("").with_tool_calls(vec![ProviderToolCall {
            id: "approve-1".into(),
            name: "metamind.approve".into(),
            arguments: serde_json::json!({
                "success": true,
                "reply": "parser fixed; tests pass",
            })
            .to_string(),
        }]),
    ]));
    let runner = ThreePassRunner::new(
        client.clone(),
        store(),
        Arc::new(NoTools),
        ThreePassConfig::default(),
    )
    .unwrap();
    let run = runner.run_step(request()).await.unwrap();
    assert_eq!(run.reply, "parser fixed; tests pass");
    assert!(run.approved);
    assert_eq!(run.verification.len(), 1);
    assert_eq!(
        client.calls(),
        3,
        "process call, judge call and Verify review call"
    );
}

#[tokio::test]
async fn a_rejected_step_is_an_error() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("I thought about it"),
        verdict(false, "no test output was shown"),
    ]));
    let runner = ThreePassRunner::new(
        client,
        store(),
        Arc::new(NoTools),
        ThreePassConfig::default(),
    )
    .unwrap();
    let error = runner.run_step(request()).await.unwrap_err();
    assert!(error.to_string().contains("step verification failed"));
    assert!(error.to_string().contains("no test output"));
}
