//! Phase 4: staged Verify writes and their commit-time revision guard.

use std::sync::Arc;

use mm_cme::{MutationJournal, ToolHandler};
use mm_data::DataService;
use mm_serve::cm_tools::CmTools;
use mm_serve::state_service::StateService;
use serde_json::{json, Map, Value};

fn prompts() -> Arc<mm_prompt::PromptStore> {
    Arc::new(
        mm_prompt::PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("prompt tree"),
    )
}

async fn fixture() -> (CmTools, String, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let service: Arc<DataService> = Arc::clone(&data.data_service);
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
        .open_session("phase-four-verify", None, 1_000)
        .await
        .unwrap()
        .session
        .session_id;
    let tools = CmTools::new(state)
        .with_session(session.clone())
        .with_data(service);
    (tools, session, temp)
}

fn create_args(label: &str) -> Value {
    let mut properties = Map::new();
    properties.insert("rdfs:label".into(), json!(label));
    json!({ "type": "mm:Note", "properties": properties })
}

fn update_args(subject: &str, label: &str, revision: u64) -> Value {
    let mut properties = Map::new();
    properties.insert("rdfs:label".into(), json!(label));
    json!({ "subject": subject, "properties": properties, "expected_revision": revision })
}

async fn call(tools: &CmTools, name: &str, arguments: Value) -> Value {
    serde_json::from_str(
        &tools
            .call(name, &arguments.to_string())
            .await
            .expect("tool envelope"),
    )
    .expect("valid envelope JSON")
}

async fn stage(tools: &CmTools, session: &str, arguments: Value) -> Value {
    let mut journal = MutationJournal::new(session);
    serde_json::from_str(
        &tools
            .call_staged_for_session(session, "data.update", &arguments.to_string(), &mut journal)
            .await
            .expect("staged envelope"),
    )
    .expect("valid envelope JSON")
}

async fn label_of(tools: &CmTools, subject: &str) -> String {
    let queried = call(tools, "data.query", json!({ "subject": subject })).await;
    queried["result"]["entities"][0]["label"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn a_staged_write_is_validated_but_not_executed() {
    let (tools, session, _temp) = fixture().await;
    let created = call(&tools, "data.create", create_args("first")).await;
    let iri = created["result"]["entity"]["iri"]
        .as_str()
        .unwrap()
        .to_owned();
    let revision = created["result"]["entity"]["revision"].as_u64().unwrap();

    let staged = stage(&tools, &session, update_args(&iri, "staged", revision)).await;
    assert_eq!(staged["result"]["staged"], json!(true), "{staged}");
    assert_eq!(
        label_of(&tools, &iri).await,
        "first",
        "staging returned an intent, not a write"
    );

    let mut journal = MutationJournal::new(&session);
    tools
        .call_staged_for_session(
            &session,
            "data.update",
            &update_args(&iri, "staged", revision).to_string(),
            &mut journal,
        )
        .await
        .unwrap();
    let committed = tools
        .commit_staged_for_session(&session, &journal)
        .await
        .unwrap();
    assert_eq!(committed.len(), 1);
    assert!(!committed[0].retried);
    assert_eq!(label_of(&tools, &iri).await, "staged");
}

#[tokio::test]
async fn a_revision_conflict_is_retried_once_against_fresh_state() {
    let (tools, session, _temp) = fixture().await;
    let created = call(&tools, "data.create", create_args("first")).await;
    let iri = created["result"]["entity"]["iri"]
        .as_str()
        .unwrap()
        .to_owned();
    let revision = created["result"]["entity"]["revision"].as_u64().unwrap();

    let mut journal = MutationJournal::new(&session);
    tools
        .call_staged_for_session(
            &session,
            "data.update",
            &update_args(&iri, "staged", revision).to_string(),
            &mut journal,
        )
        .await
        .unwrap();

    // Someone else writes between staging and approval.
    let concurrent = call(
        &tools,
        "data.update",
        update_args(&iri, "concurrent", revision),
    )
    .await;
    assert_eq!(concurrent["ok"], json!(true), "{concurrent}");

    let committed = tools
        .commit_staged_for_session(&session, &journal)
        .await
        .expect("the conflict is retried once");
    assert_eq!(committed.len(), 1);
    assert!(committed[0].retried, "the commit reports its retry");
    assert_eq!(label_of(&tools, &iri).await, "staged");
}

#[tokio::test]
async fn a_commit_that_cannot_converge_fails_and_names_the_intent() {
    let (tools, session, _temp) = fixture().await;
    let created = call(&tools, "data.create", create_args("first")).await;
    let iri = created["result"]["entity"]["iri"]
        .as_str()
        .unwrap()
        .to_owned();
    let revision = created["result"]["entity"]["revision"].as_u64().unwrap();

    let mut journal = MutationJournal::new(&session);
    tools
        .call_staged_for_session(
            &session,
            "data.update",
            &update_args(&iri, "staged", revision).to_string(),
            &mut journal,
        )
        .await
        .unwrap();

    // The subject disappears before approval, so no retry can succeed.
    let deleted = call(&tools, "data.delete", json!({ "subject": iri })).await;
    assert_eq!(deleted["ok"], json!(true), "{deleted}");

    let error = tools
        .commit_staged_for_session(&session, &journal)
        .await
        .expect_err("the commit cannot converge");
    let message = error.to_string();
    assert!(
        message.contains("VERIFY_COMMIT_FAILED") && message.contains("data.update"),
        "{message}"
    );
}
