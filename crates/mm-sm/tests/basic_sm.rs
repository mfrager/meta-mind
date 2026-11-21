//! The Phase 1 gate: the primitives the conversation engine composes with.

use mm_sm::{SmInstance, SmSnapshot, StateMachine, Transition};

fn started(machine: StateMachine) -> SmInstance {
    SmInstance::start(machine).expect("a valid machine starts")
}

#[test]
fn simple_transition() {
    let machine = StateMachine::new("job", "idle")
        .state("running")
        .state("done")
        .transition(Transition::new("idle", "start", "running"))
        .transition(Transition::new("running", "finish", "done"));
    let mut instance = started(machine);

    assert!(instance.is_in("idle"));
    let records = instance.dispatch("start").expect("dispatch");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].from.as_str(), "idle");
    assert_eq!(records[0].to.as_str(), "running");
    assert_eq!(records[0].event, "start");
    assert!(instance.is_in("running"));
    assert_eq!(instance.transition_count(), 1);

    instance.dispatch("finish").expect("dispatch");
    assert!(instance.is_in("done"));

    // An event no transition is keyed on changes nothing.
    assert!(instance.dispatch("nonsense").expect("dispatch").is_empty());
    assert!(instance.is_in("done"));
}

#[test]
fn guard_blocks_transition() {
    let machine = StateMachine::new("gate", "closed")
        .state("open")
        .transition(
            Transition::new("closed", "go", "open")
                .guard(|ctx| ctx.data["allowed"].as_bool() == Some(true)),
        );
    let mut instance =
        SmInstance::start_with_data(machine, serde_json::json!({ "allowed": false }))
            .expect("a valid machine starts");

    assert!(instance.dispatch("go").expect("dispatch").is_empty());
    assert!(instance.is_in("closed"));

    *instance.data_mut() = serde_json::json!({ "allowed": true });
    let records = instance.dispatch("go").expect("dispatch");
    assert_eq!(records.len(), 1);
    assert!(instance.is_in("open"));
}

#[test]
fn action_on_transition() {
    let machine = StateMachine::new("counter", "idle").transition(
        Transition::new("idle", "bump", "idle").action(|ctx| {
            let count = ctx.data["count"].as_u64().unwrap_or(0) + 1;
            ctx.data["count"] = serde_json::json!(count);
            Ok(())
        }),
    );
    let mut instance = started(machine);

    instance.dispatch("bump").expect("dispatch");
    instance.dispatch("bump").expect("dispatch");
    assert_eq!(instance.data()["count"], 2);
    assert!(instance.is_in("idle"));
}

#[test]
fn hierarchical_spawn_child() {
    let parent_machine = StateMachine::new("session", "idle")
        .state("active")
        .transition(Transition::new("idle", "go", "active"));
    let child_machine = StateMachine::new("turn", "waiting")
        .state("running")
        .transition(Transition::new("waiting", "go", "running"));
    let mut parent = started(parent_machine);

    let child_id = {
        let child = parent.spawn_child(child_machine).expect("spawn");
        child.id().to_string()
    };
    assert_eq!(parent.children().len(), 1);
    assert_eq!(
        parent.child(&child_id).expect("child").parent_id(),
        Some(parent.id())
    );

    // A dispatch reaches the child as well as the parent.
    let records = parent.dispatch("go").expect("dispatch");
    assert_eq!(records.len(), 2);
    assert!(parent.is_in("active"));
    assert!(parent.child(&child_id).expect("child").is_in("running"));
    assert_eq!(parent.descendants().len(), 1);

    let detached = parent.remove_child(&child_id).expect("child was attached");
    assert_eq!(detached.id(), child_id);
    assert!(parent.child(&child_id).is_none());
}

#[test]
fn orthogonal_regions_parallel() {
    let planner = StateMachine::new("planner", "idle")
        .state("working")
        .transition(Transition::new("idle", "tick", "working"));
    let voice = StateMachine::new("voice", "quiet")
        .state("listening")
        .transition(Transition::new("quiet", "tick", "listening"));
    let machine = StateMachine::new("session", "open")
        .region(planner)
        .region(voice);
    let mut instance = started(machine);

    assert_eq!(
        instance.orthogonal_current("planner").map(|s| s.as_str()),
        Some("idle")
    );
    assert_eq!(
        instance.orthogonal_current("voice").map(|s| s.as_str()),
        Some("quiet")
    );

    let records = instance.dispatch("tick").expect("dispatch");
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.depth == 1));
    assert_eq!(
        instance.orthogonal_current("planner").map(|s| s.as_str()),
        Some("working")
    );
    assert_eq!(
        instance.orthogonal_current("voice").map(|s| s.as_str()),
        Some("listening")
    );
    // The parent has no transition for the event and stays put.
    assert!(instance.is_in("open"));

    // A region can also be entered after the instance has started.
    let memory = StateMachine::new("memory", "episodic")
        .state("compressing")
        .transition(Transition::new("episodic", "compress", "compressing"));
    instance.enter_region(memory).expect("enter region");
    instance.dispatch("compress").expect("dispatch");
    assert_eq!(
        instance.orthogonal_current("memory").map(|s| s.as_str()),
        Some("compressing")
    );
    assert_eq!(instance.orthogonal_states().len(), 3);
}

#[test]
fn history_pseudostate_restore() {
    let machine = StateMachine::new("turn", "streaming")
        .state("interrupted")
        .history("interrupted")
        .transition(Transition::new("streaming", "barge_in", "interrupted"))
        .transition(Transition::to_history("interrupted", "resume"));
    let mut instance = started(machine);

    instance.dispatch("barge_in").expect("dispatch");
    assert!(instance.is_in("interrupted"));
    assert_eq!(
        instance.resume_target().map(|state| state.as_str()),
        Some("streaming")
    );

    instance.dispatch("resume").expect("dispatch");
    assert!(instance.is_in("streaming"));
    assert!(instance.resume_target().is_none());
}

#[test]
fn snapshot_round_trip() {
    let child_machine = StateMachine::new("job", "idle")
        .state("done")
        .transition(Transition::new("idle", "run", "done"));
    let parent_machine = StateMachine::new("session", "open")
        .region(
            StateMachine::new("cache", "cold")
                .state("warm")
                .transition(Transition::new("cold", "load", "warm")),
        )
        .region(
            StateMachine::new("voice", "quiet")
                .state("talking")
                .transition(Transition::new("quiet", "speak", "talking")),
        )
        .state("closed");
    let mut parent = started(parent_machine.clone());
    parent
        .spawn_child(child_machine.clone())
        .expect("spawn child")
        .dispatch("run")
        .expect("dispatch");
    parent.dispatch("load").expect("dispatch");
    parent.dispatch("speak").expect("dispatch");

    let snapshot = parent.snapshot();
    let json = serde_json::to_string(&snapshot).expect("serialize");
    let decoded: SmSnapshot = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, snapshot);

    let restored =
        SmInstance::restore(parent_machine, &decoded, &[child_machine]).expect("restore");
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(
        restored.region("cache").expect("region").state().as_str(),
        "warm"
    );
    assert_eq!(
        restored
            .child(restored.children()[0].id())
            .expect("child")
            .state()
            .as_str(),
        "done"
    );
    assert_eq!(restored.children()[0].parent_id(), Some(restored.id()));
}
