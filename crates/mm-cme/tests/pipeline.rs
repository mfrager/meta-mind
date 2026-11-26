//! End-to-end tests for the single turn pipeline.
//!
//! Every scenario drives a scripted mock through the real gateway and asserts
//! the JSONL the server will serve, not an internal intermediate.

use std::path::Path;
use std::sync::Arc;

use mm_agentstream::{EventStore, EventType, LlmGateway, SessionState};
use mm_cme::{Persona, ThreePassConfig, TurnPipeline, TurnRequest};
use mm_core::{SessionId, TurnId};
use mm_llm::{LlmClient, MockClient, ProviderToolCall, ScriptedResponse, ToolSchema};
use mm_prompt::PromptStore;
use serde_json::{json, Value};

struct Harness {
    _dir: tempfile::TempDir,
    store: Arc<EventStore>,
    client: Arc<MockClient>,
    pipeline: TurnPipeline,
}

fn harness(script: Vec<ScriptedResponse>, config: ThreePassConfig) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(EventStore::new(dir.path()).unwrap());
    let client = Arc::new(MockClient::script(script));
    let gateway: Arc<dyn LlmClient> = Arc::new(LlmGateway::new(client.clone(), store.clone()));
    let prompts = Arc::new(
        PromptStore::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"))
            .unwrap(),
    );
    let pipeline = TurnPipeline::new(
        store.clone(),
        gateway,
        prompts,
        config,
        "mock-small",
        Persona::default(),
    )
    .unwrap();
    Harness {
        _dir: dir,
        store,
        client,
        pipeline,
    }
}

fn tool(id: &str, name: &str, arguments: Value) -> ProviderToolCall {
    ProviderToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.to_string(),
    }
}

fn approve(success: bool, reply: Option<&str>, error: Option<&str>) -> ProviderToolCall {
    let mut arguments = json!({ "success": success });
    if let Some(reply) = reply {
        arguments["reply"] = json!(reply);
    }
    if let Some(error) = error {
        arguments["error"] = json!(error);
    }
    tool("call-approve", "metamind.approve", arguments)
}

fn verify(checks: Value) -> ProviderToolCall {
    tool(
        "call-verify",
        "metamind.verify",
        json!({
            "checks": checks,
            "process": { "task": "answer the request" }
        }),
    )
}

fn check(label: &str) -> Value {
    json!({
        "label": label,
        "description": "The reply cites its source.",
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

fn verdict(pass: bool, confidence: f64, reasoning: &str) -> ScriptedResponse {
    ScriptedResponse::text(
        json!({ "pass": pass, "reasoning": reasoning, "confidence": confidence }).to_string(),
    )
}

fn request(user_input: &str) -> TurnRequest {
    TurnRequest {
        session_id: SessionId::new("s1"),
        turn_id: TurnId::new("t1"),
        user_input: user_input.into(),
    }
}

fn events(harness: &Harness) -> Vec<mm_agentstream::Event> {
    harness.store.events("s1", 1, None).unwrap()
}

#[tokio::test]
async fn a_fast_path_turn_answers_in_interpret_and_logs_the_reply() {
    let harness = harness(
        vec![ScriptedResponse::text("").with_tool_calls(vec![approve(true, Some("Paris."), None)])],
        ThreePassConfig::default(),
    );
    let result = harness
        .pipeline
        .run(request("What is the capital of France?"))
        .await
        .unwrap();
    assert_eq!(result.reply, "Paris.");
    assert_eq!(result.attempts, 1);
    assert!(result.verified);

    let events = events(&harness);
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::TurnStarted));
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::AgentMessage));
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::TurnCompleted));
    assert!(!events
        .iter()
        .any(|event| event.event_type == EventType::StateUpdated));
    let request_event = events
        .iter()
        .find(|event| event.event_type == EventType::LlmRequest)
        .unwrap();
    assert_eq!(request_event.data["tools"].as_array().unwrap().len(), 2);
    assert!(!request_event.data["messages"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn an_action_turn_logs_a_verified_reply_and_a_committed_state_update() {
    let harness = harness(
        vec![
            ScriptedResponse::text("").with_tool_calls(vec![verify(json!([]))]),
            ScriptedResponse::text("").with_tool_calls(vec![tool(
                "call-topic",
                "conversation.topic",
                json!({ "topic": "incident summary" }),
            )]),
            ScriptedResponse::text("The deploy finished at 14:02."),
            verdict(true, 0.9, "the reply answers the request"),
            ScriptedResponse::text("").with_tool_calls(vec![approve(true, None, None)]),
        ],
        ThreePassConfig::default(),
    );
    let result = harness
        .pipeline
        .run(request("answer the question"))
        .await
        .unwrap();
    assert_eq!(result.reply, "The deploy finished at 14:02.");
    assert!(result.verified);

    let events = events(&harness);
    for kind in [
        EventType::PassStarted,
        EventType::PromptRendered,
        EventType::LlmRequest,
        EventType::LlmResponse,
        EventType::ToolCall,
        EventType::ToolResult,
        EventType::VerificationRegistered,
        EventType::VerificationResult,
        EventType::StateUpdated,
        EventType::InternalAction,
        EventType::AgentMessage,
        EventType::TurnCompleted,
    ] {
        assert!(
            events.iter().any(|event| event.event_type == kind),
            "missing {kind:?}"
        );
    }
    let actions = events
        .iter()
        .filter(|event| event.event_type == EventType::InternalAction)
        .filter_map(|event| event.data["action"].as_str())
        .collect::<Vec<_>>();
    for expected in [
        "state.load",
        "state.stage",
        "state.commit",
        "budget.reserve",
        "dispatch",
    ] {
        assert!(
            actions.contains(&expected),
            "missing internal action {expected}"
        );
    }

    let state = mm_agentstream::projection::session_state(&events);
    assert_eq!(state.topic.as_deref(), Some("incident summary"));
    assert_eq!(state.revision, 1);

    let process_call = events
        .iter()
        .find(|event| event.event_type == EventType::LlmRequest && event.data["pass"] == "process")
        .unwrap();
    assert_eq!(process_call.data["tools"].as_array().unwrap().len(), 4);
    let messages = process_call.data["messages"].as_array().unwrap();
    assert!(messages.iter().any(|message| message["role"] == "system"
        && message["content"]
            .as_str()
            .unwrap()
            .contains("Available tools:")));
    assert!(messages
        .iter()
        .any(|message| message["content"] == "answer the question"));
}

#[tokio::test]
async fn a_rejection_retries_process_and_then_approves() {
    let harness = harness(
        vec![
            ScriptedResponse::text("").with_tool_calls(vec![verify(json!([check("cites")]))]),
            ScriptedResponse::text("First answer."),
            verdict(false, 0.1, "no source is named"),
            ScriptedResponse::text("").with_tool_calls(vec![approve(
                false,
                None,
                Some("missing source"),
            )]),
            ScriptedResponse::text("Second answer, with a source."),
            verdict(true, 0.9, "the source is named"),
            ScriptedResponse::text("").with_tool_calls(vec![approve(true, None, None)]),
        ],
        ThreePassConfig::default(),
    );
    let result = harness
        .pipeline
        .run(request("Summarise the incident and cite the source."))
        .await
        .unwrap();
    assert_eq!(result.reply, "Second answer, with a source.");
    assert_eq!(result.attempts, 2);

    let events = events(&harness);
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::PassRetry));
    assert!(
        events.iter().any(|event| {
            event.event_type == EventType::InternalAction && event.data["action"] == "state.discard"
        }),
        "a rejected attempt must record that its staged state was discarded"
    );
    let process_calls = events
        .iter()
        .filter(|event| {
            event.event_type == EventType::LlmRequest && event.data["pass"] == "process"
        })
        .count();
    assert_eq!(process_calls, 2);
    let retry_request = events
        .iter()
        .filter(|event| {
            event.event_type == EventType::LlmRequest && event.data["pass"] == "process"
        })
        .nth(1)
        .unwrap();
    let rendered = serde_json::to_string(&retry_request.data["messages"]).unwrap();
    assert!(rendered.contains("missing source"), "{rendered}");
}

#[tokio::test]
async fn exhaustion_fails_closed_instead_of_silently_succeeding() {
    let mut config = ThreePassConfig::default();
    config.verify.max_retries = 0;
    let harness = harness(
        vec![
            ScriptedResponse::text("").with_tool_calls(vec![verify(json!([check("cites")]))]),
            ScriptedResponse::text("First answer."),
            verdict(false, 0.1, "no source is named"),
            ScriptedResponse::text("").with_tool_calls(vec![approve(
                false,
                None,
                Some("missing source"),
            )]),
        ],
        config,
    );
    let error = harness
        .pipeline
        .run(request("Summarise the incident and cite the source."))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "VERIFY_EXHAUSTED");
    let events = events(&harness);
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::TurnFailed));
    assert!(!events
        .iter()
        .any(|event| event.event_type == EventType::AgentMessage));
}

#[tokio::test]
async fn every_recorded_request_carries_the_same_messages_and_tools_the_client_saw() {
    let harness = harness(
        vec![
            ScriptedResponse::text("").with_tool_calls(vec![verify(json!([]))]),
            ScriptedResponse::text("answer"),
            verdict(true, 0.9, "ok"),
            ScriptedResponse::text("").with_tool_calls(vec![approve(true, None, None)]),
        ],
        ThreePassConfig::default(),
    );
    harness
        .pipeline
        .run(request("answer the question"))
        .await
        .unwrap();

    let events = events(&harness);
    let requests = events
        .iter()
        .filter(|event| event.event_type == EventType::LlmRequest)
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 4, "interpret, process, judge, verify");
    let seen = harness.client.seen();
    assert_eq!(seen.len(), requests.len());
    for (request_event, request) in requests.iter().zip(seen.iter()) {
        assert_eq!(
            request_event.data["messages"],
            serde_json::to_value(&request.messages).unwrap(),
            "the logged messages are the messages the client saw"
        );
        assert_eq!(
            request_event.data["tools"],
            serde_json::to_value(&request.tools).unwrap(),
            "the logged tools are the tools the client saw"
        );
    }
    let prompts = events
        .iter()
        .filter(|event| event.event_type == EventType::PromptRendered)
        .collect::<Vec<_>>();
    assert_eq!(prompts.len(), 3, "interpret, process, verify templates");
    for request_event in &requests {
        let messages = request_event.data["messages"].as_array().unwrap();
        assert!(!messages.is_empty());
        assert_eq!(
            request_event.data["messageCount"].as_u64().unwrap() as usize,
            messages.len()
        );
        let tools = request_event.data["tools"].as_array().unwrap();
        assert_eq!(
            request_event.data["toolCount"].as_u64().unwrap() as usize,
            tools.len()
        );
        for tool in tools {
            let schema: ToolSchema = serde_json::from_value(tool.clone()).unwrap();
            assert!(!schema.description.is_empty());
            assert!(schema.parameters.is_object());
        }
        if request_event.data["purpose"] != "critique" {
            let instance_id = request_event.data["promptInstanceId"]
                .as_str()
                .unwrap_or_else(|| panic!("unlinked request: {request_event:?}"));
            let hash = request_event.data["promptHash"].as_str().unwrap();
            assert!(prompts.iter().any(|prompt| {
                prompt.data["instanceId"] == instance_id && prompt.data["hash"] == hash
            }));
        }
    }
    // The judge is attributed to Verify, not to an unattributed session.
    assert!(harness
        .store
        .events("unattributed", 1, None)
        .unwrap()
        .is_empty());
    let judge = requests
        .iter()
        .find(|event| event.data["purpose"] == "critique")
        .unwrap();
    assert_eq!(judge.data["pass"], "verify");
}

#[test]
fn the_state_projection_round_trips_through_json() {
    let state = SessionState::default();
    let encoded = serde_json::to_value(&state).unwrap();
    let decoded: SessionState = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, state);
}
