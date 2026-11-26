//! Phase 2: the Interpret pass and the fast path.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use mm_cme::{
    BudgetLimits, InterpretResult, PassContext, PassFinish, PassKind, PassScope, PromptVariables,
    ThreePassConfig, ThreePassRunner, ToolHandler, TurnEnvelope, MAX_PASS_ROUNDS,
};
use mm_data::{CleanLog, CleanRole, CleanTurn};
use mm_llm::{LlmClient, MockClient, ProviderToolCall, ScriptedResponse, ToolSchema};
use mm_prompt::PromptStore;

use mm_cme::{
    AffectState, ConversationRouter, MemoryContext, PreferenceProfile, RoutingTable,
    SessionBudgets, SessionContext, Turn, TurnExecutor, TurnPlan,
};
use mm_data::{SessionId, TurnId};

fn store() -> Arc<PromptStore> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts");
    Arc::new(PromptStore::load(root).unwrap())
}

fn variables() -> PromptVariables {
    let mut vars = PromptVariables::new();
    vars.set("state_block", "[STATE]\ntopic: test");
    vars.set("being_name", "Metamind");
    vars.set("user_name", "operator");
    vars.set("workspace_label", "workspace/test");
    vars.set("tone", "warm, direct, honest");
    vars.set("budget_class", "quick");
    vars.set(
        "tools_summary",
        "conversation.topic, metamind.verify, metamind.approve",
    );
    vars.set("user_input", "hello");
    vars
}

fn context(clean_log: CleanLog) -> PassContext {
    PassContext::interpret(
        clean_log,
        "[STATE]\ntopic: test",
        "hello",
        variables(),
        Some("mock-small".into()),
        TurnEnvelope::standard(5),
    )
}

fn approve(reply: &str) -> ProviderToolCall {
    ProviderToolCall {
        id: "call-approve".into(),
        name: "metamind.approve".into(),
        arguments: serde_json::json!({ "success": true, "reply": reply }).to_string(),
    }
}

fn verify(process_task: &str) -> ProviderToolCall {
    ProviderToolCall {
        id: "call-verify".into(),
        name: "metamind.verify".into(),
        arguments: serde_json::json!({
            "checks": [],
            "process": { "task": process_task, "constraints": ["stay brief"] }
        })
        .to_string(),
    }
}

fn runner(client: Arc<MockClient>, tools: Arc<dyn ToolHandler>) -> ThreePassRunner {
    ThreePassRunner::new(client, store(), tools, ThreePassConfig::default()).unwrap()
}

fn router() -> ConversationRouter {
    use mm_llm::{Complexity, ModelSpec, ThresholdRouter};
    let specs = vec![ModelSpec {
        name: "fast".into(),
        input_cost_micros_per_1k: 100,
        output_cost_micros_per_1k: 400,
        typical_latency_ms: 100,
        complexity_ceiling: Complexity::Medium,
        mock: false,
    }];
    let table = RoutingTable::from_toml_str("default_model = \"fast\"\n").unwrap();
    ConversationRouter::with_table(
        Arc::new(ThresholdRouter::from_specs(specs, Some("fast".into()))),
        table,
    )
}

struct RefreshingProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl mm_cme::PromptContextProvider for RefreshingProvider {
    async fn prompt_variables(
        &self,
        _session_id: &str,
    ) -> std::result::Result<PromptVariables, String> {
        let round = self.calls.fetch_add(1, Ordering::SeqCst);
        let mut vars = variables();
        vars.set(
            "state_block",
            format!(
                "[STATE]\ntopic: {}",
                if round == 0 { "before" } else { "after" }
            ),
        );
        Ok(vars)
    }
}

fn session_context() -> SessionContext {
    SessionContext {
        memory: MemoryContext::default(),
        preferences: PreferenceProfile::default(),
        affect: AffectState::default(),
        budgets: SessionBudgets::default(),
    }
}

#[derive(Default)]
struct RecordingTools {
    calls: AtomicUsize,
}

#[async_trait]
impl ToolHandler for RecordingTools {
    fn schemas(&self) -> Vec<ToolSchema> {
        vec![ToolSchema {
            name: "conversation.topic".into(),
            description: "topic".into(),
            parameters: serde_json::json!({ "type": "object" }),
        }]
    }

    async fn call(&self, name: &str, arguments: &str) -> mm_cme::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({ "ok": true, "name": name, "arguments": arguments }).to_string())
    }
}

#[test]
fn the_default_runner_has_no_turn_budget() {
    let runner = runner(
        Arc::new(MockClient::new()),
        Arc::new(RecordingTools::default()),
    );
    assert!(!runner.config().budget.enabled);
    assert_eq!(runner.envelope().limits(), BudgetLimits::UNLIMITED);
}

#[tokio::test]
async fn interpret_stops_at_the_pass_round_ceiling_when_the_envelope_is_unlimited() {
    let client = Arc::new(MockClient::constant(
        ScriptedResponse::text("").with_tool_calls(vec![ProviderToolCall {
            id: "call-topic".into(),
            name: "conversation.topic".into(),
            arguments: serde_json::json!({ "action": "get" }).to_string(),
        }]),
    ));
    let tools = Arc::new(RecordingTools::default());
    let mut ctx = context(CleanLog::default());
    ctx.envelope = TurnEnvelope::unlimited(5);
    let outcome = runner(
        Arc::clone(&client),
        Arc::clone(&tools) as Arc<dyn ToolHandler>,
    )
    .run_interpret(ctx, "s1")
    .await
    .unwrap();
    assert_eq!(outcome.finished, PassFinish::Exhausted);
    assert_eq!(client.calls(), u64::from(MAX_PASS_ROUNDS));
    assert_eq!(tools.calls.load(Ordering::SeqCst), MAX_PASS_ROUNDS as usize);
}

#[tokio::test]
async fn a_greeting_is_one_call_and_a_fast_path() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![approve("Hello. How can I help?")])
    ]));
    let tools = Arc::new(RecordingTools::default());
    let outcome = runner(Arc::clone(&client), tools)
        .run_interpret(context(CleanLog::default()), "s1")
        .await
        .unwrap();
    assert_eq!(outcome.finished, PassFinish::FastPath);
    assert_eq!(outcome.text, "Hello. How can I help?");
    assert_eq!(outcome.prompt_instance_ids.len(), 1);
    assert_eq!(client.calls(), 1);
    assert!(outcome.checks.is_empty());
    assert_eq!(outcome.tool_emissions.len(), 1);
    assert_eq!(outcome.tool_emissions[0].name, "metamind.approve");
}

#[tokio::test]
async fn an_action_request_registers_a_check_and_a_process_brief() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![verify("Find the answer")])
    ]));
    let outcome = runner(Arc::clone(&client), Arc::new(RecordingTools::default()))
        .run_interpret(context(CleanLog::default()), "s1")
        .await
        .unwrap();
    assert_eq!(outcome.finished, PassFinish::Action);
    assert!(outcome.is_action());
    assert!(outcome.error.is_none());
    assert_eq!(
        outcome.checks.len(),
        1,
        "the implicit action check is added"
    );
    assert_eq!(outcome.checks[0].label, "action");
    assert_eq!(outcome.checks[0].backend, "llm_judge");
    let brief = outcome.process.unwrap();
    assert_eq!(brief.task, "Find the answer");
    assert_eq!(brief.constraints, vec!["stay brief"]);
    assert!(brief.suggested_tools.is_none());
    assert_eq!(client.calls(), 1);
}

#[tokio::test]
async fn text_without_a_control_call_is_repaired_once_then_fails() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("I think the answer is 42."),
        ScriptedResponse::text("Still just text."),
    ]));
    let outcome = runner(Arc::clone(&client), Arc::new(RecordingTools::default()))
        .run_interpret(context(CleanLog::default()), "s1")
        .await
        .unwrap();
    assert_eq!(outcome.finished, PassFinish::ProtocolError);
    assert_eq!(outcome.error.as_deref(), Some("INTERPRET_PROTOCOL"));
    assert!(outcome.text.is_empty(), "no partial reply is committed");
    assert_eq!(client.calls(), 2);
    let repair = client.last().unwrap();
    assert!(
        repair
            .messages
            .iter()
            .any(|message| message.content.contains("metamind.approve")),
        "the repair turn tells the model the protocol"
    );
}

#[tokio::test]
async fn interpret_cannot_call_a_work_tool() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![ProviderToolCall {
            id: "call-data".into(),
            name: "data.query".into(),
            arguments: r#"{"action":"get"}"#.into(),
        }]),
        ScriptedResponse::text("").with_tool_calls(vec![approve("Done.")]),
    ]));
    let tools = Arc::new(RecordingTools::default());
    let outcome = runner(
        Arc::clone(&client),
        Arc::clone(&tools) as Arc<dyn ToolHandler>,
    )
    .run_interpret(context(CleanLog::default()), "s1")
    .await
    .unwrap();
    assert_eq!(outcome.finished, PassFinish::FastPath);
    assert_eq!(tools.calls.load(Ordering::SeqCst), 0);
    assert!(outcome.tool_emissions.iter().any(|emission| {
        !emission.ok
            && emission.name == "data.query"
            && emission.result.contains("TOOL_NOT_AVAILABLE")
    }));
}

#[tokio::test]
async fn conversation_state_is_the_only_direct_write() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![ProviderToolCall {
            id: "call-topic".into(),
            name: "conversation.topic".into(),
            arguments: serde_json::json!({
                "action": "set",
                "topic": "greeting",
                "expected_revision": 1
            })
            .to_string(),
        }]),
        ScriptedResponse::text("").with_tool_calls(vec![approve("Hello.")]),
    ]));
    let tools = Arc::new(RecordingTools::default());
    let outcome = runner(
        Arc::clone(&client),
        Arc::clone(&tools) as Arc<dyn ToolHandler>,
    )
    .run_interpret(context(CleanLog::default()), "s1")
    .await
    .unwrap();
    assert_eq!(outcome.finished, PassFinish::FastPath);
    assert_eq!(tools.calls.load(Ordering::SeqCst), 1);
    assert!(outcome
        .tool_emissions
        .iter()
        .any(|emission| emission.name == "conversation.topic" && emission.ok));
}

#[tokio::test]
async fn the_clean_log_is_transcript_and_internals_stay_out() {
    let clean_log = CleanLog {
        turns: vec![
            CleanTurn {
                role: CleanRole::User,
                text: "earlier request".into(),
                turn_id: "t0".into(),
            },
            CleanTurn {
                role: CleanRole::Assistant,
                text: "earlier approved reply".into(),
                turn_id: "t0".into(),
            },
        ],
        truncated: false,
    };
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![approve("ok")])
    ]));
    let outcome = runner(Arc::clone(&client), Arc::new(RecordingTools::default()))
        .run_interpret(context(clean_log), "s1")
        .await
        .unwrap();
    assert_eq!(outcome.finished, PassFinish::FastPath);
    let request = client.last().unwrap();
    assert!(request
        .messages
        .iter()
        .any(|message| message.content == "earlier request"));
    assert!(request
        .messages
        .iter()
        .any(|message| message.content == "earlier approved reply"));
    assert_eq!(
        request
            .messages
            .last()
            .map(|message| message.content.as_str()),
        Some("hello")
    );
}

#[tokio::test]
async fn the_executor_fast_path_makes_one_call_and_skips_process() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![approve("Hello there.")])
    ]));
    let client_dyn: Arc<dyn LlmClient> = client.clone();
    let mut executor = TurnExecutor::with_llm(client_dyn, router())
        .with_prompts(store())
        .with_three_pass(runner(
            Arc::clone(&client),
            Arc::new(RecordingTools::default()),
        ));
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
    assert_eq!(outcome.text, "Hello there.");
    assert_eq!(client.calls(), 1);
    assert_eq!(outcome.pass_history.len(), 1);
    assert_eq!(outcome.pass_history[0].kind, PassKind::Interpret);
    assert_eq!(outcome.pass_history[0].outcome, "fast_path");
}

#[tokio::test]
async fn the_executor_runs_process_and_verifies_after_an_action() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![verify("Answer it")]),
        ScriptedResponse::text("Here is the work."),
        ScriptedResponse::text(
            serde_json::json!({"pass": true, "reasoning": "ok", "confidence": 0.9}).to_string(),
        ),
        ScriptedResponse::text("").with_tool_calls(vec![approve("Here is the work.")]),
    ]));
    let client_dyn: Arc<dyn LlmClient> = client.clone();
    let mut executor = TurnExecutor::with_llm(client_dyn, router())
        .with_prompts(store())
        .with_three_pass(runner(
            Arc::clone(&client),
            Arc::new(RecordingTools::default()),
        ));
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t1"),
        "do the work",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &session_context(), None)
        .await
        .unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(outcome.text, "Here is the work.");
    assert_eq!(client.calls(), 4, "interpret, process, judge, review");
    assert_eq!(
        outcome
            .pass_history
            .iter()
            .map(|record| record.kind)
            .collect::<Vec<_>>(),
        vec![PassKind::Interpret, PassKind::Process, PassKind::Verify]
    );
    assert_eq!(outcome.pass_history[0].outcome, "action");
    assert_eq!(outcome.pass_history[1].outcome, "completed");
    assert_eq!(outcome.pass_history[2].outcome, "approved");
}

#[tokio::test]
async fn interpret_rebuilds_state_before_each_round() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![ProviderToolCall {
            id: "call-topic".into(),
            name: "conversation.topic".into(),
            arguments: serde_json::json!({ "action": "get" }).to_string(),
        }]),
        ScriptedResponse::text("").with_tool_calls(vec![approve("Hello.")]),
    ]));
    let provider = Arc::new(RefreshingProvider {
        calls: AtomicUsize::new(0),
    });
    let runner = runner(Arc::clone(&client), Arc::new(RecordingTools::default()))
        .with_context_provider(provider);
    let outcome = runner
        .run_interpret(context(CleanLog::default()), "s1")
        .await
        .unwrap();
    assert_eq!(outcome.finished, PassFinish::FastPath);
    let last = client.last().unwrap();
    assert!(
        last.messages
            .iter()
            .any(|message| message.content.contains("topic: after")),
        "the second round sees the rebuilt state"
    );
}

#[test]
fn the_interpret_context_carries_no_process_or_verify_data() {
    let ctx = context(CleanLog::default());
    assert_eq!(ctx.kind, PassKind::Interpret);
    assert_eq!(ctx.scope, PassScope::Interpret);
    assert!(ctx.interpret.is_none());
    assert!(ctx.process.is_none());
    assert!(ctx.verification.is_empty());
    assert!(ctx.retry.is_none());
    assert_eq!(InterpretResult::default().checks.len(), 0);
    assert_eq!(mm_cme::PassScope::Interpret.visible_tools().len(), 6);
}
