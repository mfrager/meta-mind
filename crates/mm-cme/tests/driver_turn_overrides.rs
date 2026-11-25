//! A session driver outlives a request, so every turn must hand it that turn's
//! observer and prompt bundle. Without that, turn 2 streams into turn 1's run
//! and runs on turn 1's state block.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mm_cme::{
    ConversationRouter, DriverMessage, PassKind, PassObserver, PromptVariables, RoutingTable,
    Session, SessionDriver, ThreePassConfig, ThreePassRunner, TurnExecutor, TurnOverrides,
    UnavailableTools,
};
use mm_data::UserId;
use mm_llm::{
    Complexity, LlmClient, MockClient, ModelSpec, ProviderToolCall, ScriptedResponse,
    ThresholdRouter,
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
    let table = RoutingTable::from_toml_str("default_model = \"fast\"\n").unwrap();
    ConversationRouter::with_table(
        Arc::new(ThresholdRouter::from_specs(specs, Some("fast".into()))),
        table,
    )
}

fn variables(topic: &str) -> PromptVariables {
    let mut vars = PromptVariables::new();
    vars.set("state_block", format!("[STATE]\ntopic: {topic}"));
    vars.set("being_name", "Metamind");
    vars.set("user_name", "operator");
    vars.set("workspace_label", "workspace/test");
    vars.set("tone", "warm, direct, honest");
    vars.set("budget_class", "quick");
    vars.set("tools_summary", "metamind.verify, metamind.approve");
    vars.set("user_input", "hello");
    vars
}

fn approve(reply: &str) -> ProviderToolCall {
    ProviderToolCall {
        id: "call-approve".into(),
        name: "metamind.approve".into(),
        arguments: serde_json::json!({ "success": true, "reply": reply }).to_string(),
    }
}

#[derive(Default)]
struct RecordingObserver {
    passes: Mutex<Vec<String>>,
}

impl PassObserver for RecordingObserver {
    fn pass_started(&self, pass: PassKind, attempt: u32, _model: Option<&str>) {
        self.passes
            .lock()
            .unwrap()
            .push(format!("{}:{attempt}", pass.as_str()));
    }
}

fn executor(client: Arc<MockClient>) -> TurnExecutor {
    let tools: Arc<dyn mm_cme::ToolHandler> = Arc::new(UnavailableTools);
    let llm: Arc<dyn LlmClient> = client.clone();
    let runner =
        ThreePassRunner::new(llm, store(), Arc::clone(&tools), ThreePassConfig::default()).unwrap();
    TurnExecutor::with_llm(client, router())
        .with_prompts(store())
        .with_tools(tools)
        .with_three_pass(runner)
}

#[tokio::test]
async fn each_turn_uses_its_own_observer_and_prompt_variables() {
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text("").with_tool_calls(vec![approve("one")]),
        ScriptedResponse::text("").with_tool_calls(vec![approve("two")]),
        ScriptedResponse::text("").with_tool_calls(vec![approve("three")]),
    ]));
    let session = Session::new(UserId::new("operator"), None).unwrap();
    let (driver, messages, mut outcomes) = SessionDriver::spawn_with_executor(
        session,
        Duration::from_secs(60),
        executor(client.clone()),
    );
    let handle = tokio::spawn(driver.run());

    let first = Arc::new(RecordingObserver::default());
    messages
        .send(DriverMessage::StartTurnWith {
            query: "first".into(),
            overrides: TurnOverrides {
                pass_observer: Some(first.clone()),
                prompt_variables: Some(variables("one")),
                prompt_variant: None,
            },
        })
        .unwrap();
    assert!(outcomes.recv().await.unwrap().is_success());

    let second = Arc::new(RecordingObserver::default());
    messages
        .send(DriverMessage::StartTurnWith {
            query: "second".into(),
            overrides: TurnOverrides {
                pass_observer: Some(second.clone()),
                prompt_variables: Some(variables("two")),
                prompt_variant: None,
            },
        })
        .unwrap();
    assert!(outcomes.recv().await.unwrap().is_success());

    // A turn without overrides must not inherit the previous turn's observer.
    messages
        .send(DriverMessage::StartTurn {
            query: "third".into(),
        })
        .unwrap();
    assert!(outcomes.recv().await.unwrap().is_success());

    messages.send(DriverMessage::Shutdown).unwrap();
    handle.await.unwrap().unwrap();

    // Each observer sees its own turn and nothing from the other turn.
    assert_eq!(first.passes.lock().unwrap().as_slice(), ["interpret:1"]);
    assert_eq!(second.passes.lock().unwrap().as_slice(), ["interpret:1"]);

    // The second turn prompt carries the second bundle, not the first.
    let seen = client.seen();
    assert_eq!(seen.len(), 3);
    let first_prompt: String = seen[0]
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let second_prompt: String = seen[1]
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(first_prompt.contains("topic: one"), "{first_prompt}");
    assert!(second_prompt.contains("topic: two"), "{second_prompt}");
    assert!(
        !second_prompt.contains("topic: one"),
        "the first turn's bundle must not leak: {second_prompt}"
    );
}
