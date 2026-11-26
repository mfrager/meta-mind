//! Golden capture for the three-pass turn contract.
//!
//! This is the Phase 1 reference the streamlined core is implemented against.
//! It drives the current `TurnExecutor` + `ThreePassRunner` with the scripted
//! `MockClient` and records, per scenario:
//!
//! - every LLM request exactly as sent, including the full rendered messages and
//!   the complete tool schemas visible to that pass;
//! - every LLM response, with usage, text, and tool calls;
//! - the pass history, verification results, staged/committed writes, and the
//!   final reply;
//! - a JSONL projection in the agentstream v2 envelope, which is the shape the
//!   Phase 2 gateway must persist.
//!
//! The checked-in files under `tests/fixtures/golden/` are compared on every
//! run. Regenerate them deliberately:
//!
//! ```text
//! REGENERATE_GOLDEN=1 cargo test -p mm-cme --test golden_capture
//! ```
//!
//! Volatile values are normalized before writing: wall-clock millisecond fields
//! are zeroed and the mock's per-call ULID is dropped. Everything else — prompt
//! text, prompt hashes, tool lists, ordering, verification verdicts — is asserted
//! verbatim.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mm_cme::{
    is_mutating_call, AffectState, CmeError, CommittedMutation, ConversationRouter,
    MutationJournal, PassKind, PassScope, PreferenceProfile, SessionBudgets, SessionContext,
    ThreePassConfig, ThreePassRunner, ToolHandler, Turn, TurnExecutor, TurnOutcome, TurnPlan,
    WORK_TOOLS, CONTROL_TOOLS,
};
use mm_data::{SessionId, TurnId};
use mm_llm::{
    CallContext, Complexity, DecoderKind, DeltaSink, LlmClient, LlmRequest, LlmResponse, MockClient,
    ModelSpec, ProviderToolCall, Result as LlmResult, ScriptedResponse, ThresholdRouter,
    ToolSchema,
};
use mm_prompt::PromptStore;
use serde_json::{json, Value};

const SESSION: &str = "s1";
const TURN: &str = "t1";
const EVENT_BASE_MS: u64 = 1_764_115_200_000;

/// A pinned `[STATE]` block. The test harness has no data layer, so the block is
/// supplied explicitly instead of being rendered from `mm-data`. Its shape
/// matches `mm_cme::state_block`; the values are stable so fixtures are too.
const STATE_BLOCK: &str = concat!(
    "[STATE user=operator rev=3 as_of=2026-11-25T00:00:00Z]\n",
    r#"{"topic":"incident summary","depth":1,"status":"active","frames":[],"tasks":[],"nextActions":[],"blockers":[],"questions":[]}"#,
);

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn store() -> Arc<PromptStore> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts");
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

/// One recorded call: the request as sent, the attribution the gateway will
/// persist, and the response or transport error.
struct RecordedCall {
    context: Option<CallContext>,
    request: LlmRequest,
    response: Option<LlmResponse>,
    error: Option<String>,
}

/// A mock that remembers the call context, the request, and the response.
///
/// The runner wraps each call in `mm_llm::scope`, so `current_context()` inside
/// `complete` is the pass/round attribution the gateway should log.
struct RecordingClient {
    inner: MockClient,
    calls: Mutex<Vec<RecordedCall>>,
}

impl RecordingClient {
    fn new(inner: MockClient) -> Self {
        Self {
            inner,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn take(&self) -> Vec<RecordedCall> {
        std::mem::take(&mut *self.calls.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn record(
        &self,
        request: &LlmRequest,
        outcome: LlmResult<LlmResponse>,
        streamed: bool,
    ) -> LlmResult<LlmResponse> {
        let mut response = outcome;
        if let Ok(value) = &mut response {
            value.streamed = streamed;
        }
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(RecordedCall {
                context: mm_llm::current_context(),
                request: request.clone(),
                response: response.as_ref().ok().cloned(),
                error: response.as_ref().err().map(ToString::to_string),
            });
        response
    }
}

#[async_trait]
impl LlmClient for RecordingClient {
    async fn complete(&self, req: &LlmRequest) -> LlmResult<LlmResponse> {
        let response = self.inner.complete(req).await;
        self.record(req, response, false)
    }

    async fn complete_stream(
        &self,
        req: &LlmRequest,
        sink: &mut dyn DeltaSink,
    ) -> LlmResult<LlmResponse> {
        let response = self.inner.complete_stream(req, sink).await;
        self.record(req, response, true)
    }

    fn provider(&self) -> &str {
        self.inner.provider()
    }

    fn decoder(&self) -> DecoderKind {
        self.inner.decoder()
    }
}

/// A tool handler that offers the real catalog and journals mutations instead of
/// executing them. Staging is what makes `staged_writes` observable without a
/// data layer.
#[derive(Default)]
struct GoldenTools;

#[async_trait]
impl ToolHandler for GoldenTools {
    fn schemas(&self) -> Vec<ToolSchema> {
        WORK_TOOLS
            .into_iter()
            .chain(CONTROL_TOOLS)
            .map(|name| ToolSchema {
                name: name.into(),
                description: name.into(),
                parameters: json!({ "type": "object", "additionalProperties": true }),
            })
            .collect()
    }

    async fn call(&self, name: &str, _arguments: &str) -> mm_cme::Result<String> {
        Ok(json!({ "ok": true, "name": name }).to_string())
    }

    async fn call_staged_for_session(
        &self,
        session_id: &str,
        name: &str,
        arguments: &str,
        journal: &mut MutationJournal,
    ) -> mm_cme::Result<String> {
        let action = serde_json::from_str::<Value>(arguments)
            .ok()
            .and_then(|value| value.get("action").and_then(Value::as_str).map(str::to_owned));
        if !is_mutating_call(name, action.as_deref()) {
            return self.call(name, arguments).await;
        }
        let parsed: Value = serde_json::from_str(arguments).unwrap_or_default();
        let intent = journal
            .push(name, None, parsed.clone(), json!({}), parsed)
            .map_err(CmeError::Tool)?;
        let mut envelope = intent.staged_result();
        envelope["ok"] = json!(true);
        envelope["session"] = json!(session_id);
        Ok(envelope.to_string())
    }

    async fn commit_staged_for_session(
        &self,
        _session_id: &str,
        journal: &MutationJournal,
    ) -> mm_cme::Result<Vec<CommittedMutation>> {
        Ok(journal
            .intents
            .iter()
            .map(|intent| CommittedMutation {
                seq: intent.seq,
                tool: intent.tool.clone(),
                action: intent.action.clone(),
                result: json!({ "ok": true }),
                retried: false,
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Scenario {
    /// Interpret answers directly; no Process pass runs.
    FastPath,
    /// Interpret registers no checks, Process answers, the implicit check is
    /// judged, and Verify approves.
    ActionApproved,
    /// The first Verify attempt fails the check and rejects the reply; Process
    /// retries, stages a write, and the second Verify attempt approves.
    ActionRetryThenApproved,
}

impl Scenario {
    const ALL: [Scenario; 3] = [
        Scenario::FastPath,
        Scenario::ActionApproved,
        Scenario::ActionRetryThenApproved,
    ];

    fn name(self) -> &'static str {
        match self {
            Scenario::FastPath => "fast_path",
            Scenario::ActionApproved => "action_approved",
            Scenario::ActionRetryThenApproved => "action_retry_then_approved",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Scenario::FastPath => {
                "Interpret answers with metamind.approve; no Process or Verify pass runs."
            }
            Scenario::ActionApproved => {
                "Interpret registers no checks; Process answers; the implicit action check is \
                 judged and Verify approves."
            }
            Scenario::ActionRetryThenApproved => {
                "Verify fails the cites check and rejects the first reply; Process retries, \
                 stages a data.create, and the second Verify attempt approves and commits."
            }
        }
    }

    fn user_input(self) -> &'static str {
        match self {
            Scenario::FastPath => "What is the capital of France?",
            Scenario::ActionApproved => "answer the question",
            Scenario::ActionRetryThenApproved => {
                "Summarise the incident and cite the source you read."
            }
        }
    }

    fn script(self) -> Vec<ScriptedResponse> {
        match self {
            Scenario::FastPath => vec![ScriptedResponse::text("").with_tool_calls(vec![approve(
                true,
                Some("Paris."),
            )])],
            Scenario::ActionApproved => vec![
                ScriptedResponse::text("").with_tool_calls(vec![register(None)]),
                ScriptedResponse::text("The deploy finished at 14:02."),
                judge_verdict(true, 0.9, "the reply answers the request"),
                ScriptedResponse::text("").with_tool_calls(vec![approve(true, None)]),
            ],
            Scenario::ActionRetryThenApproved => vec![
                ScriptedResponse::text("").with_tool_calls(vec![register(Some("cites"))]),
                ScriptedResponse::text("First answer."),
                judge_verdict(false, 0.2, "no source is named"),
                ScriptedResponse::text("").with_tool_calls(vec![approve(
                    false,
                    Some("missing source"),
                )]),
                ScriptedResponse::text("Second answer, with a source."),
                judge_verdict(true, 0.85, "the source is named"),
                ScriptedResponse::text("").with_tool_calls(vec![tool(
                    "call-create",
                    "data.create",
                    json!({ "type": "mm:Note", "label": "answer" }),
                )]),
                ScriptedResponse::text("").with_tool_calls(vec![approve(true, None)]),
            ],
        }
    }
}

fn tool(id: &str, name: &str, arguments: Value) -> ProviderToolCall {
    ProviderToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.to_string(),
    }
}

fn approve(success: bool, text: Option<&str>) -> ProviderToolCall {
    let mut arguments = json!({ "success": success });
    if let Some(text) = text {
        arguments["reply"] = json!(text);
    }
    if !success {
        arguments["error"] = json!(text.unwrap_or("rejected"));
    }
    tool("call-approve", "metamind.approve", arguments)
}

fn register(label: Option<&str>) -> ProviderToolCall {
    let checks = label
        .map(|label| {
            json!([{
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
            }])
        })
        .unwrap_or_else(|| json!([]));
    tool("call-verify", "metamind.verify", json!({ "checks": checks }))
}

fn judge_verdict(pass: bool, confidence: f64, reasoning: &str) -> ScriptedResponse {
    ScriptedResponse::text(
        json!({ "pass": pass, "reasoning": reasoning, "confidence": confidence }).to_string(),
    )
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

async fn capture(scenario: Scenario) -> (Value, TurnOutcome, Vec<RecordedCall>) {
    let client = Arc::new(RecordingClient::new(MockClient::script(scenario.script())));
    let tools = Arc::new(GoldenTools);
    let config = ThreePassConfig::default();
    let runner = ThreePassRunner::new(
        client.clone(),
        store(),
        tools.clone() as Arc<dyn ToolHandler>,
        config.clone(),
    )
    .unwrap();
    let plan = TurnPlan::simple(SessionId::new(SESSION), TurnId::new(TURN), scenario.user_input());
    let mut variables = mm_cme::PromptVariables::for_turn(&plan, &session());
    variables.set("state_block", STATE_BLOCK);
    let mut executor = TurnExecutor::with_llm(client.clone(), router())
        .with_prompts(store())
        .with_prompt_variables(variables)
        .with_tools(tools)
        .with_three_pass(runner);
    let mut turn = Turn::new(plan).unwrap();
    let outcome = executor.execute(&mut turn, &session(), None).await.unwrap();
    let calls = client.take();

    let fixture = json!({
        "fixture": scenario.name(),
        "description": scenario.description(),
        "capturedAt": "2026-11-25",
        "source": "mm-cme TurnExecutor + ThreePassRunner with mm_llm::MockClient",
        "request": {
            "sessionId": SESSION,
            "turnId": TURN,
            "userInput": scenario.user_input(),
        },
        "toolScope": {
            "interpret": PassScope::Interpret.visible_tools(),
            "process": PassScope::Process.visible_tools(),
            "verify": PassScope::Verify.visible_tools(),
        },
        "calls": calls.iter().enumerate().map(|(index, call)| call_json(index, call)).collect::<Vec<_>>(),
        "outcome": normalize_json(serde_json::to_value(&outcome).unwrap()),
    });
    (fixture, outcome, calls)
}

fn call_json(index: usize, call: &RecordedCall) -> Value {
    let mut value = serde_json::to_value(&call.request).unwrap();
    let context = call.context.clone().unwrap_or_default();
    value["index"] = json!(index);
    value["callId"] = json!(format!("call-{index}"));
    value["pass"] = json!(pass_of(&call.request, &context));
    value["round"] = json!(context.round);
    value["trigger"] = json!(context.trigger);
    value["tool"] = json!(context.tool);
    value["response"] = call
        .response
        .as_ref()
        .map(|response| normalize_json(serde_json::to_value(response).unwrap()))
        .unwrap_or(Value::Null);
    value["error"] = json!(call.error);
    normalize_json(value)
}

/// The pass a call belongs to. The runner sets the pass context; the LLM judge
/// does not, so its `critique` purpose attributes it to Verify.
fn pass_of(request: &LlmRequest, context: &CallContext) -> &'static str {
    if let Some(pass) = context.pass.as_deref() {
        return match pass {
            "interpret" => "interpret",
            "process" => "process",
            "verify" => "verify",
            "think" => "think",
            _ => "other",
        };
    }
    match request.purpose.as_str() {
        "interpret" => "interpret",
        "converse" => "process",
        "critique" => "verify",
        _ => "other",
    }
}

/// Zero wall-clock fields and drop the mock's per-call ULID so the fixtures are
/// byte-stable across machines and runs.
fn normalize_json(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                match key.as_str() {
                    "elapsed_ms" | "wall_ms" => *child = json!(0),
                    "call_id" if child.is_string() => *child = json!("call"),
                    _ => *child = normalize_json(std::mem::take(child)),
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                *item = normalize_json(std::mem::take(item));
            }
        }
        _ => {}
    }
    value
}

// ---------------------------------------------------------------------------
// Event projection
// ---------------------------------------------------------------------------

fn event(seq: u64, kind: &str, source: &str, model: Option<&str>, data: Value) -> Value {
    json!({
        "v": 2,
        "seq": seq,
        "id": format!("evt-{seq:04}"),
        "sessionId": SESSION,
        "turnId": TURN,
        "at": EVENT_BASE_MS + seq,
        "type": kind,
        "source": source,
        "model": model,
        "data": data,
    })
}

/// Project the captured turn into the agentstream v2 lines Phase 2 must write.
fn project_events(user_input: &str, outcome: &TurnOutcome, calls: &[RecordedCall]) -> Vec<Value> {
    let mut events = Vec::new();
    let mut push = |kind: &str, source: &str, model: Option<&str>, data: Value| {
        let seq = events.len() as u64 + 1;
        events.push(event(seq, kind, source, model, data));
    };

    push(
        "turn.started",
        "turn.pipeline",
        None,
        json!({ "userInput": user_input }),
    );

    let mut cursor = 0;
    for record in &outcome.pass_history {
        let pass = record.kind.as_str();
        push(
            "pass.started",
            &format!("pass.{pass}"),
            outcome.model.as_deref(),
            json!({ "pass": pass, "attempt": record.attempt, "model": outcome.model }),
        );
        while cursor < calls.len() {
            let call = &calls[cursor];
            let context = call.context.clone().unwrap_or_default();
            if pass_of(&call.request, &context) != pass {
                break;
            }
            push(
                "llm.request",
                "llm.gateway",
                call.request.model.as_deref(),
                llm_request_data(cursor, record, call),
            );
            if let Some(response) = &call.response {
                push(
                    "llm.response",
                    "llm.gateway",
                    Some(&response.model),
                    json!({
                        "callId": format!("call-{cursor}"),
                        "status": "ok",
                        "model": response.model,
                        "text": response.text,
                        "toolCalls": response.tool_calls,
                        "usage": response.usage,
                        "latencyMs": response.usage.latency_ms,
                        "finishReason": response.finish_reason,
                        "reasoningChars": response.reasoning_chars,
                        "cached": response.cached,
                        "streamed": response.streamed,
                    }),
                );
            } else {
                push(
                    "llm.error",
                    "llm.gateway",
                    call.request.model.as_deref(),
                    json!({
                        "callId": format!("call-{cursor}"),
                        "kind": "transport",
                        "message": call.error,
                        "latencyMs": 0,
                        "partialChars": 0,
                    }),
                );
            }
            if let Some(response) = &call.response {
                for call_out in &response.tool_calls {
                    push(
                        "tool.call",
                        &format!("tool.{}", call_out.name),
                        call.request.model.as_deref(),
                        json!({
                            "callId": call_out.id,
                            "name": call_out.name,
                            "arguments": call_out.arguments,
                            "pass": pass,
                            "attempt": record.attempt,
                            "round": context.round,
                        }),
                    );
                }
            }
            cursor += 1;
        }
        for result in &record.verification {
            push(
                "verification.result",
                "verification",
                None,
                json!({
                    "label": result.check_label,
                    "backend": result.backend,
                    "pass": result.pass,
                    "confidence": result.confidence,
                    "reasoning": result.reasoning,
                    "durationMs": result.elapsed_ms,
                    "attempt": record.attempt,
                }),
            );
        }
        push(
            "pass.completed",
            &format!("pass.{pass}"),
            outcome.model.as_deref(),
            json!({
                "pass": pass,
                "attempt": record.attempt,
                "finished": record.outcome,
                "checks": record.checks,
                "durationMs": 0,
            }),
        );
    }

    // A call that no pass record covers would mean the projection lost a model
    // call; fail loudly instead of writing an incomplete event log.
    assert_eq!(
        cursor,
        calls.len(),
        "every recorded call must be attributed to a pass"
    );

    let attempts = outcome
        .pass_history
        .iter()
        .filter(|record| record.kind == PassKind::Process)
        .count();
    if outcome.is_success() {
        push(
            "turn.completed",
            "turn.pipeline",
            outcome.model.as_deref(),
            json!({
                "reply": outcome.text,
                "model": outcome.model,
                "attempts": attempts,
                "verified": !outcome.verification.is_empty(),
                "usage": { "tokensIn": 0, "tokensOut": 0 },
                "durationMs": 0,
            }),
        );
    } else {
        push(
            "turn.failed",
            "turn.pipeline",
            outcome.model.as_deref(),
            json!({ "error": outcome.error, "kind": "turn", "attempts": attempts }),
        );
    }
    events
}

fn llm_request_data(index: usize, record: &mm_cme::PassRecord, call: &RecordedCall) -> Value {
    let request = &call.request;
    let context = call.context.clone().unwrap_or_default();
    json!({
        "callId": format!("call-{index}"),
        "pass": pass_of(request, &context),
        "attempt": record.attempt,
        "round": context.round,
        "provider": request.provider.clone().unwrap_or_else(|| "default".into()),
        "purpose": request.purpose.as_str(),
        "strategy": Value::Null,
        "promptInstanceId": Value::Null,
        "promptHash": record.prompt_hash,
        "params": {
            "model": request.model,
            "temperature": request.temperature,
            "maxTokens": request.max_tokens,
            "cache": request.cache,
            "schema": request.schema.as_ref().map(|schema| schema.as_str()),
        },
        "messageCount": request.messages.len(),
        "toolCount": request.tools.len(),
        "messages": request.messages,
        "tools": request.tools,
    })
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden")
}

#[tokio::test]
async fn golden_fixtures_match_the_pipeline() {
    let regenerate = std::env::var_os("REGENERATE_GOLDEN").is_some();
    let dir = golden_dir();
    std::fs::create_dir_all(dir.join("events")).unwrap();

    for scenario in Scenario::ALL {
        let (fixture, outcome, calls) = capture(scenario).await;
        let events = project_events(scenario.user_input(), &outcome, &calls);

        let fixture_text = format!(
            "{}\n",
            serde_json::to_string_pretty(&fixture).unwrap()
        );
        let events_text = events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";

        let fixture_path = dir.join(format!("{}.json", scenario.name()));
        let events_path = dir.join("events").join(format!("{}.jsonl", scenario.name()));

        if regenerate {
            std::fs::write(&fixture_path, &fixture_text).unwrap();
            std::fs::write(&events_path, &events_text).unwrap();
            continue;
        }

        let expected_fixture = std::fs::read_to_string(&fixture_path).unwrap_or_else(|error| {
            panic!(
                "{} is missing ({error}); regenerate with REGENERATE_GOLDEN=1",
                fixture_path.display()
            )
        });
        assert_eq!(
            fixture_text,
            expected_fixture,
            "{} drifted from the live pipeline",
            fixture_path.display()
        );
        let expected_events = std::fs::read_to_string(&events_path).unwrap();
        assert_eq!(
            events_text,
            expected_events,
            "{} drifted from the live pipeline",
            events_path.display()
        );
    }
}
