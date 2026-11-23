//! Phase 8: prompt strategy selection, canonical state, budgets, and fail-closed
//! model calls.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use mm_cme::budget::{BudgetClass, BudgetLedger};
use mm_cme::prompt::{PromptVariables, STANDARD_SLOTS};
use mm_cme::session::{MemoryContext, PreferenceProfile, SessionBudgets};
use mm_cme::state_block::{BudgetProjection, StateView, UserProjection};
use mm_cme::{ConversationRouter, RoutingTable, SessionContext, Turn, TurnExecutor, TurnPlan};
use mm_cme::{PromptContextProvider, ToolHandler};
use mm_data::{
    ConversationFrame, ConversationState, FrameKind, FrameStack, FrameStatus, SessionBundle,
    SessionId, SessionState, StatusValue, TurnId,
};
use mm_llm::config::Complexity;
use mm_llm::{
    LlmClient, LlmRequest, LlmResponse, ModelSpec, ProviderToolCall, ThresholdRouter, ToolSchema,
};
use mm_prompt::{PromptMessage, PromptRequest, PromptStore};

fn store() -> Arc<PromptStore> {
    Arc::new(
        PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("prompt tree"),
    )
}

fn bundle() -> SessionBundle {
    let mut conversation = ConversationState::new("01S", 1_000);
    conversation.topic = Some("auth refresh flake".into());
    conversation.depth = 2;
    conversation.status.insert(
        "next".into(),
        StatusValue::List(vec![
            "reproduce the flake".into(),
            "run the auth test 20x".into(),
        ]),
    );
    conversation
        .status
        .insert("frame_step".into(), StatusValue::Text("connect".into()));
    let session = SessionState::new("01S", "01U", 1_000);
    let frames = FrameStack::new(ConversationFrame {
        id: "root".into(),
        template_id: "frame.intro".into(),
        template_version: 1,
        kind: FrameKind::Intro,
        status: FrameStatus::Active,
        entered_at: 1_000,
        updated_at: 1_000,
        expires_at: None,
    });
    SessionBundle {
        conversation,
        session,
        frames,
    }
}

fn user() -> UserProjection {
    UserProjection {
        iri: "mmd:current-user".into(),
        entity: "mmd:user/01U".into(),
        name: "Ada".into(),
        workspace: "workspace/01U".into(),
    }
}

fn session_context() -> SessionContext {
    SessionContext {
        memory: MemoryContext::default(),
        preferences: PreferenceProfile::default(),
        affect: Default::default(),
        budgets: SessionBudgets::default(),
    }
}

fn router() -> ConversationRouter {
    let specs = vec![ModelSpec {
        name: "fast".into(),
        input_cost_micros_per_1k: 100,
        output_cost_micros_per_1k: 400,
        typical_latency_ms: 100,
        complexity_ceiling: Complexity::High,
        mock: false,
    }];
    let table = RoutingTable::from_toml_str("default_model = \"fast\"").unwrap();
    ConversationRouter::with_table(
        Arc::new(ThresholdRouter::from_specs(specs, Some("fast".into()))),
        table,
    )
}

#[test]
fn strategy_selection_covers_standard_and_variants() {
    let store = store();
    assert_eq!(
        store.strategy_for("cm.turn", None).unwrap().id,
        "cm.turn.v1"
    );
    assert_eq!(
        store.strategy_for("cm.turn", Some("pending")).unwrap().id,
        "cm.turn.pending.v1"
    );
    assert_eq!(
        store.strategy_for("cm.turn", Some("resume")).unwrap().id,
        "cm.turn.resume.v1"
    );
    assert_eq!(
        store.strategy_for("cm.turn", Some("repair")).unwrap().id,
        "cm.repair.v1"
    );
    assert!(store.strategy_for("does.not.exist", None).is_err());
}

#[test]
fn the_instance_contains_character_state_tools_pending_and_budget() {
    let store = store();
    let bundle = bundle();
    let budget = BudgetLedger::new(BudgetClass::Standard).snapshot();
    let variables = PromptVariables::from_bundle(&bundle, user(), budget).unwrap();
    let state_block = variables.get("state_block").unwrap().to_owned();
    let request = PromptRequest {
        application: "cm.turn".into(),
        variant: Some("pending".into()),
        variables: variables.as_map().clone(),
        state_block: Some(state_block),
        transcript: vec![PromptMessage {
            role: "transcript".into(),
            content: "prior turn".into(),
        }],
        user_input: Some("Use the failing seed".into()),
    };
    let first = store.render_request(&request).unwrap();
    let second = store.render_request(&request).unwrap();
    assert_eq!(first.hash, second.hash);
    assert_eq!(first.fragments.len(), 15);
    assert_eq!(first.messages.last().unwrap().role, "user");
    assert_eq!(
        first.messages.last().unwrap().content,
        "Use the failing seed"
    );
    for needle in [
        "first person",
        "Your current user is Ada",
        "Available tools",
        "source files through agent.run",
        "You own the conversation state",
        "Pending work",
        "auth refresh flake",
        "Your budget is standard",
        "[STATE user=mmd:current-user",
        "frame.intro",
    ] {
        assert!(
            first.text.contains(needle),
            "missing {needle:?} in {}",
            first.text
        );
    }
    assert!(first.text.contains("\"next\""));
    assert!(first.audit().get("instanceId").is_some());
}

#[test]
fn empty_and_repair_states_are_explicit() {
    let mut empty = bundle();
    empty.conversation = ConversationState::new("01S", 1_000);
    empty.conversation.status.clear();
    let block = StateView::from_bundle(
        &empty,
        user(),
        BudgetProjection::from(BudgetLedger::new(BudgetClass::Quick).snapshot()),
    )
    .render()
    .unwrap();
    assert!(block.json.contains("\"topic\":null"));
    assert!(block.json.contains("\"depth\":0"));
    assert!(block.json.contains("\"status\":{}"));

    let store = store();
    let mut variables = PromptVariables::from_bundle(
        &bundle(),
        user(),
        BudgetLedger::new(BudgetClass::Standard).snapshot(),
    )
    .unwrap();
    variables.set("repair_reason", "stale revision 16, current 17");
    let state_block = variables.get("state_block").unwrap().to_owned();
    let request = PromptRequest {
        application: "cm.turn".into(),
        variant: Some("repair".into()),
        variables: variables.as_map().clone(),
        state_block: Some(state_block),
        transcript: Vec::new(),
        user_input: Some("retry".into()),
    };
    let instance = store.render_request(&request).unwrap();
    assert_eq!(instance.strategy, "cm.repair.v1");
    assert!(instance.text.contains("stale revision 16"));
}

#[test]
fn state_block_rebuilds_after_a_write_and_keeps_every_field() {
    let mut bundle = bundle();
    let first = StateView::from_bundle(
        &bundle,
        user(),
        BudgetProjection::from(BudgetLedger::new(BudgetClass::Standard).snapshot()),
    )
    .render()
    .unwrap();
    bundle
        .conversation
        .set_topic(
            Some("deployment checklist".into()),
            mm_data::StateOrigin::Cm,
            2_000,
        )
        .unwrap();
    let second = StateView::from_bundle(
        &bundle,
        user(),
        BudgetProjection::from(BudgetLedger::new(BudgetClass::Standard).snapshot()),
    )
    .render()
    .unwrap();
    assert!(first.text.contains("auth refresh flake"));
    assert!(second.text.contains("deployment checklist"));
    assert!(second.header.contains("rev=2"));
    for field in [
        "topic",
        "depth",
        "status",
        "revision",
        "updatedAt",
        "updatedBy",
    ] {
        assert!(second.json.contains(field), "missing {field}");
    }
}

#[test]
fn standard_slots_are_complete_enough_for_every_strategy() {
    let store = store();
    let mut variables = PromptVariables::new();
    for slot in STANDARD_SLOTS {
        variables.set(*slot, "fixture");
    }
    for strategy in store.strategies() {
        for slot in &strategy.slots {
            variables.set(slot.clone(), "fixture");
        }
        store.render(&strategy.id, variables.as_map()).unwrap();
    }
}

struct CountingClient {
    calls: AtomicUsize,
}

#[async_trait]
impl LlmClient for CountingClient {
    async fn complete(&self, _request: &LlmRequest) -> mm_llm::Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        panic!("a model call must not happen without a PromptInstance");
    }

    fn provider(&self) -> &str {
        "counting"
    }
}

#[tokio::test]
async fn a_missing_prompt_fails_closed_before_the_model() {
    let client = Arc::new(CountingClient {
        calls: AtomicUsize::new(0),
    });
    let mut executor = TurnExecutor::with_llm(Arc::clone(&client) as Arc<dyn LlmClient>, router())
        .without_prompts();
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t1"),
        "hello",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &session_context(), None)
        .await
        .unwrap();
    assert!(!outcome.is_success());
    assert!(outcome
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("PROMPT_UNAVAILABLE"));
    assert_eq!(client.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn budget_denial_is_a_budget_error_not_success() {
    let mut ledger = BudgetLedger::new(BudgetClass::Frame);
    ledger.reserve_round().unwrap();
    ledger.reserve_round().unwrap();
    let error = ledger.reserve_round().unwrap_err();
    assert!(error.to_string().starts_with("BUDGET:"));
}

#[test]
fn the_automatic_inference_fixtures_have_targets() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/phase8_evals.json")).unwrap();
    assert_eq!(fixture["schema"], "mm.cme.prompt-evals.v1");
    let scenarios = fixture["scenarios"].as_array().unwrap();
    assert!(scenarios.len() >= 8);
    assert!(scenarios.iter().all(|scenario| {
        scenario["id"].is_string()
            && scenario["target"].as_f64().is_some()
            && scenario["expect"].is_object()
    }));
    assert!(scenarios
        .iter()
        .any(|scenario| scenario["id"] == "secret_refused"));
    assert!(scenarios
        .iter()
        .any(|scenario| scenario["id"] == "duplicate_avoided"));
}

#[test]
fn the_executor_has_no_hidden_trigger_or_system_prompt_literal() {
    let source = include_str!("../src/executor.rs");
    for forbidden in [
        "call `conversation.topic`",
        "topic changed",
        "you must update status",
        "This is a proactive aside",
        "You are the Metamind assistant",
    ] {
        assert!(
            !source.contains(forbidden),
            "executor contains prompt trigger/literal: {forbidden}"
        );
    }
    assert!(source.contains("PROMPT_UNAVAILABLE"));
}

struct CountingProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl PromptContextProvider for CountingProvider {
    async fn prompt_variables(&self, _session_id: &str) -> Result<PromptVariables, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut variables = PromptVariables::new();
        for slot in STANDARD_SLOTS {
            variables.set(*slot, "fixture");
        }
        variables.set("state_block", "[STATE unavailable: test store closed]");
        Ok(variables)
    }
}

struct NoopTool;

#[async_trait]
impl ToolHandler for NoopTool {
    fn schemas(&self) -> Vec<ToolSchema> {
        vec![ToolSchema {
            name: "conversation.topic".into(),
            description: "set the topic".into(),
            parameters: serde_json::json!({"type": "object"}),
        }]
    }

    async fn call(&self, _name: &str, _arguments: &str) -> mm_cme::Result<String> {
        Ok("{\"ok\":true}".into())
    }
}

#[tokio::test]
async fn every_tool_round_rebuilds_the_prompt_from_the_provider() {
    use mm_llm::{MockClient, ScriptedResponse};
    let provider = Arc::new(CountingProvider {
        calls: AtomicUsize::new(0),
    });
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![ProviderToolCall {
            id: "call-1".into(),
            name: "conversation.topic".into(),
            arguments: "{\"topic\":\"auth\"}".into(),
        }]),
        ScriptedResponse::text("done"),
    ]));
    let mut session = session_context();
    session.preferences.values.insert("tools".into(), 1.0);
    let plan = TurnPlan::simple(SessionId::new("s1"), TurnId::new("t1"), "hello");
    let mut turn = Turn::new(plan).unwrap();
    let mut executor = TurnExecutor::with_llm(client, router())
        .with_tools(Arc::new(NoopTool))
        .with_prompt_provider(provider.clone());
    let outcome = executor.execute(&mut turn, &session, None).await.unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_successful_model_call_records_the_prompt_instance() {
    use mm_llm::{MockClient, ScriptedResponse};
    let client = Arc::new(MockClient::script(vec![ScriptedResponse::text("done")]));
    let mut executor = TurnExecutor::with_llm(client, router());
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t1"),
        "hello",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &session_context(), None)
        .await
        .unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert!(outcome.prompt_instance_id.is_some());
    assert_eq!(outcome.prompt_hash.as_ref().unwrap().len(), 64);
}
