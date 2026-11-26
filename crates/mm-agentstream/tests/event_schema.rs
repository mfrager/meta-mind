//! The agentstream v2 event-schema contract.
//!
//! This is the list Phase 2 and Phase 4 implement against. Changing a name or the
//! envelope version is a deliberate contract change, not an incidental refactor.

use mm_agentstream::{EventType, NewEvent, SCHEMA_VERSION};

const EXPECTED: &[&str] = &[
    "session.started",
    "session.ended",
    "turn.started",
    "turn.completed",
    "turn.failed",
    "pass.started",
    "pass.completed",
    "pass.failed",
    "pass.retry",
    "prompt.rendered",
    "llm.request",
    "llm.attempt",
    "llm.delta",
    "llm.response",
    "llm.error",
    "tool.call",
    "tool.result",
    "state.updated",
    "verification.registered",
    "verification.result",
    "internal.action",
    "internal.error",
    "agent.message",
    "user.message",
];

#[test]
fn the_event_name_list_is_frozen() {
    let actual = EventType::ALL
        .into_iter()
        .map(EventType::as_str)
        .collect::<Vec<_>>();
    assert_eq!(actual, EXPECTED);
    for name in EXPECTED {
        assert_eq!(EventType::parse(name).unwrap().as_str(), *name);
    }
}

#[test]
fn the_envelope_is_version_two_and_camel_case() {
    assert_eq!(SCHEMA_VERSION, 2);
    let event = NewEvent::new(
        "session",
        Some("turn".into()),
        EventType::TurnStarted,
        "turn.pipeline",
        None,
        serde_json::json!({ "userInput": "hi", "text": "hi" }),
    )
    .into_event(1, 1_764_115_200_001);
    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(value["v"], 2);
    assert_eq!(value["seq"], 1);
    assert_eq!(value["sessionId"], "session");
    assert_eq!(value["turnId"], "turn");
    assert_eq!(value["type"], "turn.started");
    assert_eq!(value["source"], "turn.pipeline");
    assert_eq!(value["data"]["userInput"], "hi");
    assert_eq!(value["id"].as_str().unwrap().len(), 26);
}

#[test]
fn deltas_are_the_only_live_only_event() {
    for event_type in EventType::ALL {
        assert_eq!(
            event_type.is_persisted(),
            event_type.as_str() != "llm.delta"
        );
    }
}
