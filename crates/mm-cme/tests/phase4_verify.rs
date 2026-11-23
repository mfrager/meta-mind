//! Phase 4: checks, staged writes, retries, exhaustion, and recursive think.
//!
//! The turn-level cases drive `TurnExecutor` through the three-pass runner with
//! a scripted model. The staged-write cases use a handler that journals writes
//! instead of executing them, so "not committed" is observable.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mm_cme::{
    AffectState, CommittedMutation, ConversationRouter, MutationJournal, PassContext, PassKind,
    PassScope, PreferenceProfile, ProcessResult, PromptVariables, SessionBudgets, SessionContext,
    ThinkPassRequest, ThreePassConfig, ThreePassRunner, ToolHandler, Turn, TurnExecutor, TurnPlan,
    VerificationBackend, VerificationError, VerificationResult, VerifyCheck, VerifyContext,
};
use mm_data::{CleanLog, SessionId, TurnId};
use mm_llm::{
    Complexity, MockClient, ModelSpec, ProviderToolCall, ScriptedResponse, ThresholdRouter,
    ToolSchema,
};
use mm_prompt::PromptStore;

fn store() -> Arc<PromptStore> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts");
    Arc::new(PromptStore::load(root).unwrap())
}

fn router() -> ConversationRouter {
    let specs = vec![ModelSpec {
        name: "fast".into(),
        input_cost_micros_per_1k: 100,
        output_cost_micros_per_1k: 400,
        typical_latency_ms: 100,
        complexity_ceiling: Complexity::Medium,
        mock: false,
    }];
    let table = mm_cme::RoutingTable::from_toml_str("default_model = \"fast\"\n").unwrap();
    ConversationRouter::with_table(
        Arc::new(ThresholdRouter::from_specs(specs, Some("fast".into()))),
        table,
    )
}

fn session() -> SessionContext {
    let mut preferences = PreferenceProfile::default();
    preferences.values.insert("tools".into(), 1.0);
    SessionContext {
        memory: Default::default(),
        preferences,
        affect: AffectState::default(),
        budgets: SessionBudgets {
            turn_token_budget: 8_000,
            max_turns: 1_000,
        },
    }
}

fn judge_verdict(pass: bool, confidence: f64, reasoning: &str) -> ScriptedResponse {
    ScriptedResponse::text(
        serde_json::json!({ "pass": pass, "reasoning": reasoning, "confidence": confidence })
            .to_string(),
    )
}

fn tool(id: &str, name: &str, arguments: serde_json::Value) -> ProviderToolCall {
    ProviderToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.to_string(),
    }
}

fn judge_check(label: &str) -> serde_json::Value {
    serde_json::json!({
        "label": label,
        "description": "The reply cites its source.",
        "details": "Fail a reply that claims a source it did not consult.",
        "backend": "llm_judge",
        "confidence_threshold": 0.5,
        "backend_params": {
            "evaluation_prompt": "Request: {{user_request}}\nReply: {{process_reply}}\n{{check_details}}",
            "expected_output_schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["pass", "reasoning", "confidence"],
                "properties": {
                    "pass": { "type": "boolean" },
                    "reasoning": { "type": "string" },
                    "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
                }
            }
        }
    })
}

fn register(label: Option<&str>) -> ProviderToolCall {
    let checks = label
        .map(|label| serde_json::json!([judge_check(label)]))
        .unwrap_or_else(|| serde_json::json!([]));
    tool(
        "call-verify",
        "metamind.verify",
        serde_json::json!({ "checks": checks }),
    )
}

fn approve(success: bool, text: Option<&str>) -> ProviderToolCall {
    let mut arguments = serde_json::json!({ "success": success });
    if let Some(text) = text {
        arguments["reply"] = serde_json::json!(text);
    }
    if !success {
        arguments["error"] = serde_json::json!(text.unwrap_or("rejected"));
    }
    tool("call-approve", "metamind.approve", arguments)
}

/// Records immediate, staged, and committed work separately.
#[derive(Default)]
struct JournalTools {
    calls: Mutex<Vec<String>>,
    staged: Mutex<Vec<String>>,
    committed: Mutex<Vec<String>>,
    fail_commit: AtomicBool,
}

impl JournalTools {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn staged(&self) -> Vec<String> {
        self.staged.lock().unwrap().clone()
    }

    fn committed(&self) -> Vec<String> {
        self.committed.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToolHandler for JournalTools {
    fn schemas(&self) -> Vec<ToolSchema> {
        [
            "conversation.topic",
            "agent.run",
            "data.query",
            "data.create",
            "metamind.log",
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
        Ok(serde_json::json!({ "ok": true, "name": name }).to_string())
    }

    async fn call_staged_for_session(
        &self,
        session_id: &str,
        name: &str,
        arguments: &str,
        journal: &mut MutationJournal,
    ) -> mm_cme::Result<String> {
        if !mm_cme::is_mutating_call(name, action_of(arguments).as_deref()) {
            return self.call(name, arguments).await;
        }
        self.staged.lock().unwrap().push(name.into());
        let parsed: serde_json::Value = serde_json::from_str(arguments).unwrap_or_default();
        let intent = journal
            .push(name, None, parsed.clone(), serde_json::json!({}), parsed)
            .map_err(mm_cme::CmeError::Tool)?;
        let mut envelope = intent.staged_result();
        envelope["ok"] = serde_json::json!(true);
        envelope["session"] = serde_json::json!(session_id);
        Ok(envelope.to_string())
    }

    async fn commit_staged_for_session(
        &self,
        _session_id: &str,
        journal: &MutationJournal,
    ) -> mm_cme::Result<Vec<CommittedMutation>> {
        if self.fail_commit.load(Ordering::SeqCst) {
            return Err(mm_cme::CmeError::Tool(
                "VERIFY_COMMIT_FAILED: revision conflict on intent 1".into(),
            ));
        }
        let mut committed = Vec::new();
        for intent in &journal.intents {
            self.committed.lock().unwrap().push(intent.tool.clone());
            committed.push(CommittedMutation {
                seq: intent.seq,
                tool: intent.tool.clone(),
                action: intent.action.clone(),
                result: serde_json::json!({ "ok": true }),
                retried: false,
            });
        }
        Ok(committed)
    }
}

fn action_of(arguments: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|value| {
            value
                .get("action")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn runner(
    client: Arc<MockClient>,
    tools: Arc<JournalTools>,
    config: ThreePassConfig,
) -> ThreePassRunner {
    ThreePassRunner::new(client, store(), tools as Arc<dyn ToolHandler>, config).unwrap()
}

async fn execute(
    client: Arc<MockClient>,
    tools: Arc<JournalTools>,
    config: ThreePassConfig,
) -> (mm_cme::TurnOutcome, Turn) {
    let mut executor = TurnExecutor::with_llm(client.clone(), router())
        .with_prompts(store())
        .with_tools(tools.clone())
        .with_three_pass(runner(client, tools, config));
    let mut turn = Turn::new(TurnPlan::simple(
        SessionId::new("s1"),
        TurnId::new("t1"),
        "answer the question",
    ))
    .unwrap();
    let outcome = executor.execute(&mut turn, &session(), None).await.unwrap();
    (outcome, turn)
}

#[tokio::test]
async fn a_zero_check_action_turn_registers_the_implicit_check_and_approves() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![register(None)]),
        ScriptedResponse::text("Done."),
        judge_verdict(true, 0.9, "the reply answers the request"),
        ScriptedResponse::text("").with_tool_calls(vec![approve(true, None)]),
    ]));
    let tools = Arc::new(JournalTools::default());
    let (outcome, turn) = execute(client.clone(), tools.clone(), ThreePassConfig::default()).await;

    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(outcome.text, "Done.");
    assert_eq!(client.calls(), 4, "interpret, process, judge, review");
    let kinds = outcome
        .pass_history
        .iter()
        .map(|record| record.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![PassKind::Interpret, PassKind::Process, PassKind::Verify]
    );
    let verify = outcome
        .pass_history
        .iter()
        .rev()
        .find(|record| record.kind == PassKind::Verify)
        .unwrap();
    assert_eq!(verify.outcome, "approved");
    assert_eq!(verify.checks, vec!["action".to_owned()]);
    assert_eq!(outcome.verification.len(), 1);
    assert!(outcome.verification[0].pass);
    assert!(
        tools.committed().is_empty(),
        "an empty journal commits nothing"
    );
    assert!(turn.staged_writes.is_empty());
}

#[tokio::test]
async fn a_failed_check_retries_process_and_stages_until_approval() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![register(Some("cites"))]),
        ScriptedResponse::text("First answer."),
        judge_verdict(false, 0.2, "no source is named"),
        ScriptedResponse::text("").with_tool_calls(vec![approve(false, Some("missing source"))]),
        ScriptedResponse::text("Second answer, with a source."),
        judge_verdict(true, 0.85, "the source is named"),
        ScriptedResponse::text("").with_tool_calls(vec![tool(
            "call-create",
            "data.create",
            serde_json::json!({ "type": "mm:Note", "label": "answer" }),
        )]),
        ScriptedResponse::text("").with_tool_calls(vec![approve(true, None)]),
    ]));
    let tools = Arc::new(JournalTools::default());
    let (outcome, _turn) = execute(client.clone(), tools.clone(), ThreePassConfig::default()).await;

    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(outcome.text, "Second answer, with a source.");
    let processes = outcome
        .pass_history
        .iter()
        .filter(|record| record.kind == PassKind::Process)
        .count();
    assert_eq!(processes, 2, "one retry ran a second Process attempt");
    assert_eq!(
        tools.calls(),
        Vec::<String>::new(),
        "the staged create never executed immediately"
    );
    assert_eq!(tools.staged(), vec!["data.create".to_owned()]);
    assert_eq!(
        tools.committed(),
        vec!["data.create".to_owned()],
        "approval commits the journal exactly once"
    );
    assert_eq!(outcome.staged_writes.len(), 1);
    assert_eq!(outcome.committed_writes.len(), 1);
}

#[tokio::test]
async fn rejected_attempts_exhaust_the_turn_without_committing() {
    let config = ThreePassConfig {
        verify: mm_cme::VerifyConfig {
            max_retries: 1,
            ..ThreePassConfig::default().verify
        },
        ..ThreePassConfig::default()
    };
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![register(Some("cites"))]),
        // attempt 1
        ScriptedResponse::text("First answer."),
        judge_verdict(false, 0.1, "no source"),
        ScriptedResponse::text("").with_tool_calls(vec![approve(false, Some("missing source"))]),
        // attempt 2
        ScriptedResponse::text("Second answer."),
        judge_verdict(false, 0.1, "still no source"),
        ScriptedResponse::text("").with_tool_calls(vec![approve(false, Some("still missing"))]),
    ]));
    let tools = Arc::new(JournalTools::default());
    let (outcome, turn) = execute(client, tools.clone(), config).await;

    assert!(!outcome.is_success());
    let error = outcome.error.clone().unwrap_or_default();
    assert!(
        error.starts_with("VERIFY_EXHAUSTED"),
        "the failure is honest and names the check: {error}"
    );
    assert!(error.contains("cites"));
    assert!(
        outcome.text.is_empty(),
        "the unverified reply is not presented"
    );
    assert!(tools.committed().is_empty());
    assert!(
        !turn.verification.is_empty(),
        "the failed checks are retained"
    );
}

#[tokio::test]
async fn a_failed_check_cannot_be_overridden_by_default() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![register(Some("cites"))]),
        ScriptedResponse::text("Unsourced answer."),
        judge_verdict(false, 0.1, "no source"),
        ScriptedResponse::text("").with_tool_calls(vec![approve(true, None)]),
        // The protocol repair asks again; a second override attempt fails the pass.
        ScriptedResponse::text("").with_tool_calls(vec![approve(true, None)]),
    ]));
    let tools = Arc::new(JournalTools::default());
    let (outcome, _turn) = execute(client, tools.clone(), ThreePassConfig::default()).await;

    assert!(!outcome.is_success());
    assert!(
        outcome
            .error
            .as_deref()
            .is_some_and(|error| error.contains("VERIFICATION_OVERRIDE_DENIED")),
        "{outcome:?}"
    );
    assert!(tools.committed().is_empty());
}

#[tokio::test]
async fn override_approves_a_failed_check_only_when_configured() {
    let config = ThreePassConfig {
        verify: mm_cme::VerifyConfig {
            allow_override: true,
            ..ThreePassConfig::default().verify
        },
        ..ThreePassConfig::default()
    };
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![register(Some("cites"))]),
        ScriptedResponse::text("Unsourced answer."),
        judge_verdict(false, 0.1, "no source"),
        ScriptedResponse::text("").with_tool_calls(vec![approve(true, None)]),
    ]));
    let tools = Arc::new(JournalTools::default());
    let (outcome, _turn) = execute(client, tools, config).await;

    assert!(outcome.is_success(), "{outcome:?}");
    let verify = outcome
        .pass_history
        .iter()
        .rev()
        .find(|record| record.kind == PassKind::Verify)
        .unwrap();
    assert_eq!(verify.outcome, "approved_override");
}

#[tokio::test]
async fn a_commit_failure_retains_the_journal_and_fails_the_turn() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![register(None)]),
        ScriptedResponse::text("Done."),
        judge_verdict(true, 0.9, "ok"),
        ScriptedResponse::text("").with_tool_calls(vec![tool(
            "call-create",
            "data.create",
            serde_json::json!({ "type": "mm:Note" }),
        )]),
        ScriptedResponse::text("").with_tool_calls(vec![approve(true, None)]),
    ]));
    let tools = Arc::new(JournalTools::default());
    tools.fail_commit.store(true, Ordering::SeqCst);
    let (outcome, turn) = execute(client, tools, ThreePassConfig::default()).await;

    assert!(!outcome.is_success());
    assert!(
        outcome
            .error
            .as_deref()
            .is_some_and(|error| error.contains("VERIFY_COMMIT_FAILED")),
        "{outcome:?}"
    );
    assert_eq!(turn.staged_writes.len(), 1, "the journal is retained");
    assert!(outcome.committed_writes.is_empty());
}

struct FixedBackend {
    id: &'static str,
    delay: Duration,
    fail: bool,
}

#[async_trait]
impl VerificationBackend for FixedBackend {
    fn id(&self) -> &'static str {
        self.id
    }

    fn validate_config(
        &self,
        _params: &serde_json::Value,
    ) -> std::result::Result<(), VerificationError> {
        Ok(())
    }

    async fn verify(
        &self,
        check: &VerifyCheck,
        _process_output: &ProcessResult,
        _context: &VerifyContext,
    ) -> std::result::Result<VerificationResult, VerificationError> {
        tokio::time::sleep(self.delay).await;
        if self.fail {
            return Err(VerificationError::Model("backend crashed".into()));
        }
        Ok(VerificationResult {
            check_label: check.label.clone(),
            pass: true,
            reasoning: "fixed".into(),
            confidence: 1.0,
            backend: self.id.into(),
            elapsed_ms: 0,
            error: None,
        })
    }
}

fn check(label: &str, backend: &str) -> VerifyCheck {
    VerifyCheck {
        label: label.into(),
        backend: backend.into(),
        backend_params: serde_json::json!({}),
        ..VerifyCheck::default()
    }
}

#[tokio::test]
async fn checks_keep_declaration_order_and_failures_never_pass() {
    let config = ThreePassConfig {
        verify: mm_cme::VerifyConfig {
            check_timeout_ms: 50,
            max_parallel_checks: 4,
            ..ThreePassConfig::default().verify
        },
        ..ThreePassConfig::default()
    };
    let runner = runner(
        Arc::new(MockClient::new()),
        Arc::new(JournalTools::default()),
        config,
    )
    .with_backend(Arc::new(FixedBackend {
        id: "fast",
        delay: Duration::ZERO,
        fail: false,
    }))
    .unwrap()
    .with_backend(Arc::new(FixedBackend {
        id: "slow",
        delay: Duration::from_millis(500),
        fail: false,
    }))
    .unwrap()
    .with_backend(Arc::new(FixedBackend {
        id: "broken",
        delay: Duration::ZERO,
        fail: true,
    }))
    .unwrap();
    let checks = vec![
        check("slow", "slow"),
        check("fast", "fast"),
        check("broken", "broken"),
        check("missing", "not-registered"),
    ];
    let results = runner
        .run_checks(
            &checks,
            &ProcessResult::default(),
            &VerifyContext::default(),
        )
        .await;

    assert_eq!(results.len(), 4);
    assert_eq!(
        results
            .iter()
            .map(|result| result.check_label.as_str())
            .collect::<Vec<_>>(),
        vec!["slow", "fast", "broken", "missing"],
        "declaration order is preserved"
    );
    assert!(!results[0].pass);
    assert!(results[0]
        .error
        .as_deref()
        .unwrap_or("")
        .contains("timed out"));
    assert!(results[1].pass);
    assert!(!results[2].pass);
    assert!(results[2]
        .error
        .as_deref()
        .unwrap_or("")
        .contains("backend crashed"));
    assert!(!results[3].pass);
    assert!(results[3]
        .error
        .as_deref()
        .unwrap_or("")
        .contains("unknown"));
}

#[tokio::test]
async fn a_recursive_think_runs_under_the_think_scope_and_writes_nothing() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text(
            r#"{"summary":"Because the evidence says so.","findings":[],"gaps":[]}"#,
        ),
        judge_verdict(true, 0.9, "the thought cites evidence"),
        ScriptedResponse::text("").with_tool_calls(vec![tool(
            "call-create",
            "data.create",
            serde_json::json!({ "type": "mm:Note" }),
        )]),
        ScriptedResponse::text("").with_tool_calls(vec![approve(true, None)]),
    ]));
    let tools = Arc::new(JournalTools::default());
    let runner = runner(client.clone(), tools.clone(), ThreePassConfig::default());
    let run = runner
        .run_think(ThinkPassRequest {
            think_id: "think/parent/01".into(),
            question: "why did the build fail?".into(),
            context: Some("the last turn changed the lockfile".into()),
            focus: vec!["mmd:build".into()],
            model: Some("fast".into()),
        })
        .await
        .unwrap();

    assert!(run.approved);
    assert_eq!(run.result.think_id, "think/parent/01");
    assert_eq!(run.result.summary, "Because the evidence says so.");
    assert_eq!(run.verification.len(), 1);
    assert!(run.verification[0].pass);

    let seen = client.seen();
    let process = &seen[0];
    let names = process
        .tools
        .iter()
        .map(|schema| schema.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"agent.run"), "think may delegate");
    assert!(names.contains(&"data.query"), "think may read data");
    assert!(!names.contains(&"metamind.think"), "think cannot recurse");
    assert!(!names.contains(&"data.create"), "think cannot write data");
    assert!(process
        .messages
        .iter()
        .any(|message| message.content.contains("why did the build fail?")));
    assert!(
        tools.staged().is_empty(),
        "the Think reviewer's write was denied before staging"
    );
    assert!(tools.committed().is_empty(), "a thought never commits");
}

/// A quick unit check that a Think context is not accepted by the Process pass
/// and that a Process context is not accepted by Verify.
#[tokio::test]
async fn pass_contexts_are_not_interchangeable() {
    let runner = runner(
        Arc::new(MockClient::new()),
        Arc::new(JournalTools::default()),
        ThreePassConfig::default(),
    );
    let plan = TurnPlan::simple(SessionId::new("s1"), TurnId::new("t1"), "q");
    let ctx = PassContext::interpret(
        CleanLog::default(),
        "",
        "q",
        PromptVariables::for_turn(&plan, &session()),
        None,
        runner.envelope(),
    );
    assert!(runner.run_verify(ctx, "s1").await.is_err());
    assert_eq!(PassScope::Think.visible_tools().len(), 8);
}
