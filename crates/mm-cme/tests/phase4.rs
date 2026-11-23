//! Phase 4: planning, routing, execution, tools and voice tiers.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mm_cme::chunk::ChunkDirective;
use mm_cme::executor::ToolHandler;
use mm_cme::session::{MemoryContext, PreferenceProfile, SessionBudgets};
use mm_cme::{
    ConfirmationHandler, ConversationRouter, ConversationRoutingRequest, DriverMessage,
    EchoResponder, MockAsr, MockTts, ProsodyController, ResponseModality, RoutingTable, Session,
    SessionContext, SessionDriver, Turn, TurnExecutor, TurnOutcome, TurnPlan, TurnPlanner,
    VoiceCapabilities, VoicePipeline, VoiceTier, VoiceTurnConfig,
};
use mm_data::{SessionId, TurnId, UserId};
use mm_llm::config::Complexity;
use mm_llm::{
    LlmClient, LlmRequest, LlmResponse, MockClient, ModelSpec, ProviderToolCall, ScriptedResponse,
    ThresholdRouter,
};

fn specs() -> Vec<ModelSpec> {
    vec![
        ModelSpec {
            name: "fast".into(),
            input_cost_micros_per_1k: 100,
            output_cost_micros_per_1k: 400,
            typical_latency_ms: 100,
            complexity_ceiling: Complexity::Medium,
            mock: false,
        },
        ModelSpec {
            name: "deep".into(),
            input_cost_micros_per_1k: 1_000,
            output_cost_micros_per_1k: 4_000,
            typical_latency_ms: 900,
            complexity_ceiling: Complexity::High,
            mock: false,
        },
    ]
}

fn router() -> ConversationRouter {
    let table = RoutingTable::from_toml_str(
        r#"
        default_model = "fast"
        [modality]
        drill_down = "deep"
        [voice]
        tier2 = "deep"
        tier3 = "deep"
        "#,
    )
    .unwrap();
    ConversationRouter::with_table(
        Arc::new(ThresholdRouter::from_specs(specs(), Some("fast".into()))),
        table,
    )
}

fn session_context() -> SessionContext {
    SessionContext {
        memory: MemoryContext::default(),
        preferences: PreferenceProfile::default(),
        affect: Default::default(),
        budgets: SessionBudgets::default(),
    }
}

#[test]
fn planner_is_deterministic_and_builds_modality_shaped_chunks() {
    let planner = TurnPlanner;
    let ctx = mm_cme::TurnContext::user_input(
        TurnId::new("t1"),
        SessionId::new("s1"),
        "give me the details and sources",
    );
    let plan = planner.build_plan(&ctx, &session_context()).unwrap();
    assert_eq!(plan.modality, ResponseModality::DrillDown);
    assert_eq!(plan.chunks.len(), 3);
    assert_eq!(plan.chunks[0].chunk_id.as_str(), "overview");
    assert!(plan.chunks[1].buffered);
    assert_eq!(plan.budget.tokens, 7_998);

    let again = planner.build_plan(&ctx, &session_context()).unwrap();
    assert_eq!(plan, again);
}

#[test]
fn routing_reacts_to_modality_and_voice_tier() {
    let router = router();
    let drill = router
        .choose(&ConversationRoutingRequest::text(
            "explain in detail",
            ResponseModality::DrillDown,
        ))
        .unwrap();
    assert_eq!(drill.model, "deep");

    let voice = router
        .choose(
            &ConversationRoutingRequest::text("hello", ResponseModality::Explain)
                .with_voice_tier(VoiceTier::Tier2),
        )
        .unwrap();
    assert_eq!(voice.model, "deep");

    let text = router
        .choose(&ConversationRoutingRequest::text(
            "hello",
            ResponseModality::Explain,
        ))
        .unwrap();
    assert_eq!(text.model, "fast");
}

#[test]
fn the_committed_routing_table_loads_and_pins_valid_models() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/routing_table.toml");
    let table = RoutingTable::load(&path).expect("routing table");
    assert_eq!(
        table.model_for(ResponseModality::Explain, None, false),
        Some("gemini/gemini-2.5-flash")
    );
    assert_eq!(
        table.model_for(ResponseModality::Explain, Some(VoiceTier::Tier3), false),
        Some("auto/remote")
    );
}

#[tokio::test]
async fn executor_drives_turn_through_planning_to_completion() {
    let client = Arc::new(MockClient::script(vec![ScriptedResponse::text(
        "Here is the answer.",
    )]));
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
    assert_eq!(outcome.text, "Here is the answer.");
    assert_eq!(outcome.model.as_deref(), Some("fast"));
    assert_eq!(outcome.chunks_completed, 2);
    assert_eq!(
        turn.state(),
        mm_cme::TurnState::Completed.as_str(),
        "executor reaches completed"
    );
}

/// Records the model each request pins, so a routed model cannot be dropped on
/// the way to the adapter (the live path failed exactly that way).
struct ModelRecorder {
    inner: Arc<MockClient>,
    seen: std::sync::Mutex<Vec<Option<String>>>,
}

#[async_trait]
impl LlmClient for ModelRecorder {
    async fn complete(&self, request: &LlmRequest) -> mm_llm::Result<LlmResponse> {
        self.seen.lock().unwrap().push(request.model.clone());
        self.inner.complete(request).await
    }

    fn provider(&self) -> &str {
        "recording"
    }
}

#[tokio::test]
async fn the_routed_model_is_pinned_on_the_request() {
    let recorder = Arc::new(ModelRecorder {
        inner: Arc::new(MockClient::script(vec![ScriptedResponse::text(
            "Here is the answer.",
        )])),
        seen: std::sync::Mutex::new(Vec::new()),
    });
    let mut executor =
        TurnExecutor::with_llm(Arc::clone(&recorder) as Arc<dyn LlmClient>, router());
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
    assert_eq!(
        recorder.seen.lock().unwrap().as_slice(),
        [Some("fast".to_owned())],
        "the request must carry the model the router chose"
    );
}

#[derive(Debug, Default)]
struct RecordingTools;

#[async_trait]
impl ToolHandler for RecordingTools {
    async fn call(&self, name: &str, arguments: &str) -> mm_cme::Result<String> {
        Ok(format!("{name}:{arguments}:42"))
    }
}

#[derive(Debug, Default)]
struct CountingTools {
    calls: std::sync::atomic::AtomicUsize,
}

impl CountingTools {
    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl ToolHandler for CountingTools {
    async fn call(&self, _name: &str, _arguments: &str) -> mm_cme::Result<String> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok("ran".into())
    }
}

#[derive(Debug)]
struct ApproveTools;

#[async_trait]
impl ConfirmationHandler for ApproveTools {
    async fn confirm(&self, _name: &str, _arguments: &str) -> mm_cme::Result<bool> {
        Ok(true)
    }
}

#[derive(Debug)]
struct DenyTools;

#[async_trait]
impl ConfirmationHandler for DenyTools {
    async fn confirm(&self, _name: &str, _arguments: &str) -> mm_cme::Result<bool> {
        Ok(false)
    }
}

fn confirming_context() -> SessionContext {
    let mut context = session_context();
    context.preferences.values.insert("tools".into(), 1.0);
    context
        .preferences
        .values
        .insert("confirm_tools".into(), 1.0);
    context
}

fn tool_call_client() -> Arc<MockClient> {
    Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![ProviderToolCall {
            id: "call-1".into(),
            name: "lookup".into(),
            arguments: "{\"key\":\"answer\"}".into(),
        }]),
        ScriptedResponse::text("The answer is 42."),
    ]))
}

#[tokio::test]
async fn executor_runs_a_provider_tool_call_through_a_tool_chunk() {
    let client = tool_call_client();
    let mut executor =
        TurnExecutor::with_llm(client, router()).with_tools(Arc::new(RecordingTools));
    let mut context = session_context();
    context.preferences.values.insert("tools".into(), 1.0);
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t2"),
        "look up the answer",
    ))
    .unwrap();
    let outcome = executor.execute(&mut turn, &context, None).await.unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(turn.tool_results.len(), 1);
    assert_eq!(turn.tool_results[0], "lookup:{\"key\":\"answer\"}:42");
    assert_eq!(outcome.tool_calls.len(), 1);
    assert_eq!(outcome.tool_calls[0].name, "lookup");
    assert!(outcome.tool_calls[0].ok);
    assert_eq!(
        turn.chunks
            .iter()
            .filter(|chunk| chunk.state() == "completed")
            .count(),
        3,
        "prose chunks plus the tool chunk complete"
    );
}

#[tokio::test]
async fn executor_runs_a_tool_only_after_confirmation() {
    let tools = Arc::new(CountingTools::default());
    let mut executor = TurnExecutor::with_llm(tool_call_client(), router())
        .with_tools(tools.clone())
        .with_confirmation_handler(Arc::new(ApproveTools));
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t4"),
        "look up the answer",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &confirming_context(), None)
        .await
        .unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(tools.calls(), 1, "the approved tool ran once");
    assert_eq!(turn.tool_results, vec!["ran".to_owned()]);
}

#[tokio::test]
async fn executor_records_a_denied_tool_and_finishes_the_turn() {
    let tools = Arc::new(CountingTools::default());
    let mut executor = TurnExecutor::with_llm(tool_call_client(), router())
        .with_tools(tools.clone())
        .with_confirmation_handler(Arc::new(DenyTools));
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t5"),
        "look up the answer",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &confirming_context(), None)
        .await
        .unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(tools.calls(), 0, "a denied tool never runs");
    assert_eq!(
        turn.tool_results,
        vec!["tool `lookup` denied by the user".to_owned()]
    );
    assert_eq!(outcome.text, "The answer is 42.");
}

#[tokio::test]
async fn executor_reports_a_model_failure_as_a_failed_turn() {
    let client = Arc::new(MockClient::script(vec![ScriptedResponse::failing(
        "provider unavailable",
    )]));
    let mut executor = TurnExecutor::with_llm(client, router());
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t3"),
        "hello",
    ))
    .unwrap();
    let outcome: TurnOutcome = executor
        .execute(&mut turn, &session_context(), None)
        .await
        .unwrap();
    assert!(!outcome.is_success());
    assert_eq!(outcome.state, "failed");
    assert!(outcome.error.unwrap().contains("provider unavailable"));
}

#[test]
fn voice_negotiation_respects_provider_capabilities() {
    let mut region = mm_cme::VoiceRegion::new().unwrap();
    let negotiation = region
        .negotiate(VoiceTier::Tier3, VoiceCapabilities::new(true, false, false))
        .unwrap();
    assert_eq!(negotiation.tier, VoiceTier::Tier1);
    assert!(negotiation.degraded);
    assert_eq!(region.state(), "tier1_request_response");
}

#[tokio::test]
async fn voice_tiers_run_with_mock_asr_and_tts() {
    let mut pipeline = VoicePipeline::new(
        Box::new(MockAsr::once("what is the weather")),
        Box::new(MockTts::default()),
        Box::new(mm_cme::voice::audio::EnergyVad::default()),
    );
    let turn = pipeline
        .run_tier1(
            vec![mm_cme::AudioChunk::tone(4_000, 160, 16_000)],
            &EchoResponder,
            &mm_being::Affect::neutral(),
        )
        .await
        .unwrap();
    assert_eq!(turn.tier, VoiceTier::Tier1);
    assert_eq!(turn.reply, "Echo: what is the weather");
    assert!(!turn.audio.is_empty());
}

#[test]
fn prosody_follows_affect_and_modality() {
    let params = ProsodyController.compute(
        &mm_being::Affect {
            arousal: 0.5,
            warmth: 0.5,
            ..mm_being::Affect::neutral()
        },
        Some(ResponseModality::Explain),
    );
    assert!(params.rate > 1.0);
    assert!(params.warmth > 0.5);
    assert!((params.pause_factor - 1.2).abs() < 0.001);
}

#[tokio::test]
async fn driver_uses_the_executor_and_the_negotiated_voice_tier() {
    let mut session = Session::new(UserId::new("u1"), None).unwrap();
    session
        .dispatch(mm_cme::SessionEvent::InitComplete)
        .unwrap();
    session.dispatch_regions("start_voice", None).unwrap();
    session
        .dispatch_regions(
            "tier_negotiated",
            Some(serde_json::json!({ "tier": "tier2" })),
        )
        .unwrap();
    assert_eq!(session.voice_tier(), Some(VoiceTier::Tier2));

    let client = Arc::new(MockClient::script(vec![ScriptedResponse::text("voiced")]));
    let executor = TurnExecutor::with_llm(client, router());
    let (driver, messages, mut outcomes) =
        SessionDriver::spawn_with_executor(session, Duration::from_secs(3_600), executor);
    let handle = tokio::spawn(driver.run());

    messages
        .send(DriverMessage::StartTurn {
            query: "hello".into(),
        })
        .unwrap();
    let outcome = outcomes.recv().await.unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(outcome.model.as_deref(), Some("deep"));
    assert_eq!(outcome.text, "voiced");
    assert_eq!(outcome.chunks_completed, 3, "two prose chunks and audio");

    messages.send(DriverMessage::Shutdown).unwrap();
    handle.await.unwrap().unwrap();
}

#[test]
fn voice_config_is_carried_on_the_plan() {
    let planner = TurnPlanner;
    let mut ctx = mm_cme::TurnContext::user_input(TurnId::new("t1"), SessionId::new("s1"), "hello");
    ctx.voice_config = Some(VoiceTurnConfig::for_tier(VoiceTier::Tier2));
    let plan = planner.build_plan(&ctx, &session_context()).unwrap();
    assert_eq!(plan.voice_config.unwrap().tier, VoiceTier::Tier2);
    assert!(plan
        .chunks
        .iter()
        .any(|chunk| matches!(chunk.kind, mm_cme::ChunkKind::Audio)));
    assert!(!plan.chunks.iter().any(|chunk| !chunk.depends_on.is_empty()));
    let _ = ChunkDirective::text("x", "x");
}
