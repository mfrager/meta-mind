//! Phase 3: the extracted Process pass, delegation, and legacy parity.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mm_cme::{
    AffectState, BudgetClass, ConversationRouter, MemoryContext, PassConfig, PassKind,
    PreferenceProfile, ProcessPass, ProcessRequest, PromptVariables, RetryContext, RetryFailure,
    RoutingTable, SessionBudgets, SessionContext, ThreePassConfig, ThreePassRunner, ToolHandler,
    Turn, TurnExecutor, TurnPlan,
};
use mm_data::{CleanLog, CleanRole, CleanTurn, SessionId, TurnId};
use mm_llm::{MockClient, ProviderToolCall, ScriptedResponse, ToolSchema};
use mm_prompt::{PromptRequest, PromptStore};

fn store() -> Arc<PromptStore> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts");
    Arc::new(PromptStore::load(root).unwrap())
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

fn session(tools: bool, token_budget: u64) -> SessionContext {
    let mut preferences = PreferenceProfile::default();
    if tools {
        preferences.values.insert("tools".into(), 1.0);
    }
    SessionContext {
        memory: MemoryContext::default(),
        preferences,
        affect: AffectState::default(),
        budgets: SessionBudgets {
            turn_token_budget: token_budget,
            max_turns: 1_000,
        },
    }
}

fn verify(task: &str, constraints: &[&str]) -> ProviderToolCall {
    ProviderToolCall {
        id: "call-verify".into(),
        name: "metamind.verify".into(),
        arguments: serde_json::json!({
            "checks": [],
            "process": {
                "task": task,
                "constraints": constraints,
                "suggested_tools": ["data.query"]
            }
        })
        .to_string(),
    }
}

fn judge_verdict(pass: bool, confidence: f64) -> ScriptedResponse {
    ScriptedResponse::text(
        serde_json::json!({
            "pass": pass,
            "reasoning": "the reply addresses the task",
            "confidence": confidence
        })
        .to_string(),
    )
}

fn approve() -> ProviderToolCall {
    tool(
        "call-approve",
        "metamind.approve",
        serde_json::json!({ "success": true }),
    )
}

fn tool(id: &str, name: &str, arguments: serde_json::Value) -> ProviderToolCall {
    ProviderToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.to_string(),
    }
}

#[derive(Default)]
struct RecordingTools {
    calls: Mutex<Vec<String>>,
}

impl RecordingTools {
    fn names(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToolHandler for RecordingTools {
    fn schemas(&self) -> Vec<ToolSchema> {
        [
            "conversation.topic",
            "agent.run",
            "data.query",
            "data.create",
            "metamind.think",
            "metamind.verify",
            "metamind.approve",
        ]
        .into_iter()
        .map(|name| ToolSchema {
            name: name.into(),
            description: name.into(),
            parameters: serde_json::json!({ "type": "object" }),
        })
        .collect()
    }

    async fn call(&self, name: &str, _arguments: &str) -> mm_cme::Result<String> {
        self.calls.lock().unwrap().push(name.into());
        Ok(serde_json::json!({
            "ok": true,
            "name": name,
            "prompt_tokens": 12,
            "completion_tokens": 4,
            "usage": { "promptTokens": 12, "completionTokens": 4 }
        })
        .to_string())
    }
}

fn runner(
    client: Arc<MockClient>,
    tools: Arc<RecordingTools>,
    config: ThreePassConfig,
) -> ThreePassRunner {
    ThreePassRunner::new(client, store(), tools as Arc<dyn ToolHandler>, config).unwrap()
}

#[tokio::test]
async fn the_legacy_path_keeps_its_prompt_tool_sequence_and_outcome() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![tool(
            "call-topic",
            "conversation.topic",
            serde_json::json!({ "action": "get" }),
        )]),
        ScriptedResponse::text("Done."),
    ]));
    let tools = Arc::new(RecordingTools::default());
    let mut executor = TurnExecutor::with_llm(client.clone(), router())
        .with_prompts(store())
        .with_tools(tools.clone());
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t1"),
        "inspect the topic",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &session(true, 8_000), None)
        .await
        .unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(outcome.text, "Done.");
    assert_eq!(tools.names(), vec!["conversation.topic"]);
    assert_eq!(client.calls(), 2);
    assert_eq!(
        outcome
            .tool_calls
            .iter()
            .map(|call| call.name.as_str())
            .collect::<Vec<_>>(),
        vec!["conversation.topic"]
    );
    assert_eq!(outcome.prompt_hash, turn.prompt_hash);
    assert_eq!(
        outcome.prompt_hash.as_deref().map(str::len),
        Some(64),
        "the legacy turn pins a full prompt hash"
    );
    assert!(outcome.pass_history.is_empty());
}

#[tokio::test]
async fn an_action_process_uses_the_interpret_brief_and_the_process_strategy() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![verify(
            "Search the graph for the answer",
            &["stay brief", "cite evidence"],
        )]),
        ScriptedResponse::text("").with_tool_calls(vec![tool(
            "call-data",
            "data.query",
            serde_json::json!({ "class": "Note" }),
        )]),
        ScriptedResponse::text("The answer, with evidence."),
        judge_verdict(true, 0.9),
        ScriptedResponse::text("").with_tool_calls(vec![approve()]),
    ]));
    let tools = Arc::new(RecordingTools::default());
    let mut executor = TurnExecutor::with_llm(client.clone(), router())
        .with_prompts(store())
        .with_tools(tools.clone())
        .with_three_pass(runner(
            client.clone(),
            tools.clone(),
            ThreePassConfig::default(),
        ));
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t1"),
        "answer the question",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &session(true, 8_000), None)
        .await
        .unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(outcome.text, "The answer, with evidence.");
    assert_eq!(tools.names(), vec!["data.query"]);
    let seen = client.seen();
    assert_eq!(seen.len(), 5, "interpret, process, judge, review");
    assert_eq!(seen[0].purpose, mm_llm::Purpose::Interpret);
    let process_prompt = seen[1]
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(process_prompt.contains("Search the graph for the answer"));
    assert!(process_prompt.contains("- stay brief"));
    assert!(process_prompt.contains("- cite evidence"));
    assert!(process_prompt.contains("data.query"));
    assert!(
        seen[1]
            .tools
            .iter()
            .any(|schema| schema.name == "data.query"),
        "the Process scope advertises data tools"
    );
    assert_eq!(client.calls(), 5, "interpret, process, judge, review");
    assert_eq!(
        outcome
            .pass_history
            .iter()
            .map(|record| record.kind)
            .collect::<Vec<_>>(),
        vec![PassKind::Interpret, PassKind::Process, PassKind::Verify]
    );
}

#[tokio::test]
async fn process_allows_delegation_and_writes_but_denies_control_tools() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![verify("Do the work", &[])]),
        ScriptedResponse::text("").with_tool_calls(vec![
            tool(
                "call-approve",
                "metamind.approve",
                serde_json::json!({ "success": true }),
            ),
            tool(
                "call-data",
                "data.create",
                serde_json::json!({ "class": "Note" }),
            ),
            tool(
                "call-agent",
                "agent.run",
                serde_json::json!({ "input": "look" }),
            ),
        ]),
        ScriptedResponse::text("Worked."),
        judge_verdict(true, 0.9),
        ScriptedResponse::text("").with_tool_calls(vec![approve()]),
    ]));
    let tools = Arc::new(RecordingTools::default());
    let mut executor = TurnExecutor::with_llm(client.clone(), router())
        .with_prompts(store())
        .with_tools(tools.clone())
        .with_three_pass(runner(
            client.clone(),
            tools.clone(),
            ThreePassConfig::default(),
        ));
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t1"),
        "do the work",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &session(true, 8_000), None)
        .await
        .unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(tools.names(), vec!["data.create", "agent.run"]);
    let approve = outcome
        .tool_calls
        .iter()
        .find(|call| call.name == "metamind.approve")
        .expect("the denied control call is recorded");
    assert!(!approve.ok);
    assert!(approve.result.contains("process pass"));
    assert!(outcome
        .tool_calls
        .iter()
        .any(|call| call.name == "agent.run" && call.ok));
    assert!(outcome
        .tool_calls
        .iter()
        .any(|call| call.name == "data.create" && call.ok));
}

#[tokio::test]
async fn process_allows_the_thinker_but_not_verify() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![verify("Think about it", &[])]),
        ScriptedResponse::text("").with_tool_calls(vec![
            tool(
                "call-think",
                "metamind.think",
                serde_json::json!({ "question": "why" }),
            ),
            tool(
                "call-verify",
                "metamind.verify",
                serde_json::json!({ "checks": [] }),
            ),
        ]),
        ScriptedResponse::text("Thought about it."),
        judge_verdict(true, 0.9),
        ScriptedResponse::text("").with_tool_calls(vec![approve()]),
    ]));
    let tools = Arc::new(RecordingTools::default());
    let mut executor = TurnExecutor::with_llm(client.clone(), router())
        .with_prompts(store())
        .with_tools(tools.clone())
        .with_three_pass(runner(
            client.clone(),
            tools.clone(),
            ThreePassConfig::default(),
        ));
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t1"),
        "think about it",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &session(true, 8_000), None)
        .await
        .unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(tools.names(), vec!["metamind.think"]);
    assert!(outcome
        .tool_calls
        .iter()
        .any(|call| call.name == "metamind.verify" && !call.ok));
}

#[tokio::test]
async fn a_child_operation_cannot_exceed_the_process_remainder() {
    let config = ThreePassConfig {
        process: PassConfig {
            budget_class: BudgetClass::Memory,
            strategy: "cm.process.v1".into(),
        },
        ..ThreePassConfig::default()
    };
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![verify("Delegate", &[])]),
        ScriptedResponse::text("").with_tool_calls(vec![
            tool(
                "call-agent",
                "agent.run",
                serde_json::json!({ "input": "look" }),
            ),
            tool(
                "call-think",
                "metamind.think",
                serde_json::json!({ "question": "why" }),
            ),
        ]),
        ScriptedResponse::text("I could not delegate or think within budget."),
        judge_verdict(true, 0.9),
        ScriptedResponse::text("").with_tool_calls(vec![approve()]),
    ]));
    let tools = Arc::new(RecordingTools::default());
    let mut executor = TurnExecutor::with_llm(client.clone(), router())
        .with_prompts(store())
        .with_tools(tools.clone())
        .with_three_pass(runner(client.clone(), tools.clone(), config));
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t1"),
        "delegate the lookup",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &session(true, 1_000), None)
        .await
        .unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert!(tools.names().is_empty(), "neither child ran");
    for name in ["agent.run", "metamind.think"] {
        let denial = outcome
            .tool_calls
            .iter()
            .find(|call| call.name == name)
            .unwrap_or_else(|| panic!("{name} denial is recorded"));
        assert!(!denial.ok);
        assert!(denial.result.contains("BUDGET"));
    }
}

#[tokio::test]
async fn process_budget_exhaustion_fails_with_budget_and_no_verify() {
    let config = ThreePassConfig {
        process: PassConfig {
            budget_class: BudgetClass::Quick,
            strategy: "cm.process.v1".into(),
        },
        ..ThreePassConfig::default()
    };
    let repeated = (0..9)
        .map(|_| ScriptedResponse::text("").with_tool_calls(vec![verify("Work", &[])]))
        .collect();
    let client = Arc::new(MockClient::script(repeated));
    let tools = Arc::new(RecordingTools::default());
    let mut executor = TurnExecutor::with_llm(client.clone(), router())
        .with_prompts(store())
        .with_tools(tools.clone())
        .with_three_pass(runner(client.clone(), tools.clone(), config));
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t1"),
        "work until the budget ends",
    ))
    .unwrap();
    let outcome = executor
        .execute(&mut turn, &session(true, 8_000), None)
        .await
        .unwrap();
    assert!(!outcome.is_success());
    assert!(
        outcome
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("BUDGET:")),
        "{outcome:?}"
    );
    assert!(outcome.text.is_empty());
    assert_eq!(
        outcome
            .pass_history
            .iter()
            .map(|record| record.kind)
            .collect::<Vec<_>>(),
        vec![PassKind::Interpret],
        "the failed Process pass never reached Verify"
    );
}

#[tokio::test]
async fn a_process_with_no_final_text_fails_incomplete() {
    let plan = TurnPlan::simple(SessionId::new("s1"), TurnId::new("t1"), "say nothing");
    let client = Arc::new(MockClient::script(vec![ScriptedResponse::text("")]));
    let pass = ProcessPass::new(
        client,
        Some(store()),
        Arc::new(RecordingTools::default()),
        Arc::new(mm_cme::DenyUnconfirmed),
        None,
        BudgetClass::Standard,
    );
    let mut turn = Turn::new(plan.clone()).unwrap();
    let error = pass
        .run(
            &mut turn,
            &plan,
            "fast",
            PromptVariables::for_turn(&plan, &session(false, 8_000)),
            ProcessRequest {
                application: "cm.process.v1".into(),
                scope: Some(mm_cme::PassScope::Process),
                process: Some(mm_cme::ProcessContext::default()),
                retry: None,
                clean_log: CleanLog::default(),
                variant: None,
                require_reply: true,
                child_budget: true,
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("PROCESS_INCOMPLETE"));
}

#[test]
fn the_legacy_prompt_fixture_hashes_the_same_content() {
    let plan = TurnPlan::simple(SessionId::new("s1"), TurnId::new("t1"), "inspect the topic");
    let mut variables = PromptVariables::for_turn(&plan, &session(true, 8_000));
    variables.set("budget_class", "standard");
    variables.set("budget_rounds", "32");
    variables.set("budget_tokens", "120000");
    variables.set("budget_wall_ms", "600000");
    let state_block = variables.get("state_block").unwrap_or_default().to_owned();
    let request = PromptRequest {
        application: "cm.turn".into(),
        variant: None,
        variables: variables.as_map().clone(),
        state_block: Some(state_block),
        transcript: Vec::new(),
        user_input: Some(plan.user_query.clone()),
    };
    let instance = store().render_request(&request).unwrap();
    assert_eq!(
        instance.hash, "fdcd32f2b8900938c1e78695c4d7719295d2fa9df16ababf817ef8f9185a60aa",
        "the legacy Process prompt is a reviewed fixture"
    );
}

#[tokio::test]
async fn retry_context_appears_only_on_a_retry_attempt() {
    let plan = TurnPlan::simple(SessionId::new("s1"), TurnId::new("t1"), "try again");
    let variables = PromptVariables::for_turn(&plan, &session(false, 8_000));
    let tools = Arc::new(RecordingTools::default());
    let clean_log = CleanLog {
        turns: vec![
            CleanTurn {
                role: CleanRole::User,
                text: "earlier request".into(),
                turn_id: "t0".into(),
            },
            CleanTurn {
                role: CleanRole::Assistant,
                text: "earlier answer".into(),
                turn_id: "t0".into(),
            },
        ],
        truncated: false,
    };

    let fresh_client = Arc::new(MockClient::script(vec![ScriptedResponse::text("fresh")]));
    let fresh = ProcessPass::new(
        fresh_client.clone(),
        Some(store()),
        tools.clone(),
        Arc::new(mm_cme::DenyUnconfirmed),
        None,
        BudgetClass::Standard,
    );
    let mut fresh_turn = Turn::new(plan.clone()).unwrap();
    fresh
        .run(
            &mut fresh_turn,
            &plan,
            "fast",
            variables.clone(),
            ProcessRequest {
                application: "cm.process.v1".into(),
                scope: Some(mm_cme::PassScope::Process),
                process: Some(mm_cme::ProcessContext {
                    task: "try again".into(),
                    ..mm_cme::ProcessContext::default()
                }),
                retry: None,
                clean_log: clean_log.clone(),
                variant: None,
                require_reply: true,
                child_budget: true,
            },
        )
        .await
        .unwrap();
    assert!(!fresh_client.seen()[0]
        .messages
        .iter()
        .any(|message| message.content.contains("RETRY_CONTEXT")));

    let retry_client = Arc::new(MockClient::script(vec![ScriptedResponse::text("retried")]));
    let retry = ProcessPass::new(
        retry_client.clone(),
        Some(store()),
        tools,
        Arc::new(mm_cme::DenyUnconfirmed),
        None,
        BudgetClass::Standard,
    );
    let mut retry_turn = Turn::new(plan.clone()).unwrap();
    retry
        .run(
            &mut retry_turn,
            &plan,
            "fast",
            variables,
            ProcessRequest {
                application: "cm.process.v1".into(),
                scope: Some(mm_cme::PassScope::Process),
                process: Some(mm_cme::ProcessContext::default()),
                retry: Some(RetryContext {
                    attempt: 2,
                    max_attempts: 5,
                    prior_reply_digest: "sha256:dead".into(),
                    failures: vec![RetryFailure {
                        label: "cites".into(),
                        reasoning: "no source".into(),
                        confidence: 0.3,
                    }],
                    error: "cites failed".into(),
                    prior_effects: Vec::new(),
                }),
                clean_log,
                variant: None,
                require_reply: true,
                child_budget: true,
            },
        )
        .await
        .unwrap();
    let prompt = retry_client.seen()[0]
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(prompt.contains("RETRY_CONTEXT"));
    assert!(prompt.contains("attempt: 2/5"));
    assert!(prompt.contains("- cites: no source (confidence 0.30)"));
    assert!(prompt.contains("error: cites failed"));
}
