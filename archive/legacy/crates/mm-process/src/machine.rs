//! The per-template state machine.
//!
//! A definition is generated from the template's execution layout: one state per
//! group, plus `created`, `waiting`, `paused`, `completed`, `cancelled` and
//! `failed`. Group transitions are `advance`; waits and pauses dispatch `wait` and
//! `pause`; resuming names the group (`resume:<group>`), which keeps the machine
//! deterministic without a history state whose predecessor would have to be
//! stored outside the snapshot.
//!
//! The snapshot is serialized into the instance document. On recovery the
//! definition is rebuilt from the pinned template version and the snapshot is
//! restored, so a restart does not reset a running process to its first step.

use std::collections::BTreeMap;

use mm_data::process::{ProcessError, ProcessInstance, ProcessTemplate, StepStatus};
use mm_sm::{SmEvent, SmInstance, SmSnapshot, StateId, StateMachine, Transition};
use serde_json::{json, Value};

use crate::template::Layout;

/// The machine definition name for a template.
pub fn machine_name(template: &ProcessTemplate) -> String {
    format!("mm.process:{}", template.uri)
}

/// Build the definition from a template layout.
pub fn build(template: &ProcessTemplate, layout: &Layout) -> Result<StateMachine, ProcessError> {
    if layout.groups.is_empty() {
        return Err(ProcessError::Invalid(
            "template has no execution groups".into(),
        ));
    }
    let name = machine_name(template);
    let first = step_state(&template.steps[layout.groups[0].steps[0]].id);
    let mut machine = StateMachine::new(name, "created")
        .state("waiting")
        .state("paused")
        .state("completed")
        .state("cancelled")
        .state("failed");
    for group in &layout.groups {
        let leader = step_state(&template.steps[group.steps[0]].id);
        machine = machine.state(leader);
    }
    machine = machine.transition(Transition::new("created", "start", first));
    for (index, group) in layout.groups.iter().enumerate() {
        let leader = step_state(&template.steps[group.steps[0]].id);
        let next = layout
            .groups
            .get(index + 1)
            .map(|next| step_state(&template.steps[next.steps[0]].id));
        machine = machine
            .transition(Transition::new(
                leader.clone(),
                "advance",
                next.unwrap_or_else(|| "completed".to_owned()),
            ))
            .transition(Transition::new(leader.clone(), "wait", "waiting"))
            .transition(Transition::new(leader.clone(), "pause", "paused"))
            .transition(Transition::new(leader.clone(), "fail", "failed"))
            .transition(Transition::new(leader.clone(), "cancel", "cancelled"))
            .transition(Transition::new(leader.clone(), "retry", leader.clone()));
    }
    for group in &layout.groups {
        let leader = step_state(&template.steps[group.steps[0]].id);
        let event = format!("resume:{leader}");
        machine = machine
            .transition(Transition::new("waiting", event.clone(), leader.clone()))
            .transition(Transition::new("paused", event.clone(), leader.clone()))
            .transition(Transition::new("waiting", "cancel", "cancelled"))
            .transition(Transition::new("paused", "cancel", "cancelled"));
    }
    Ok(machine)
}

/// The mm-sm state name for a template step.
pub fn step_state(step_id: &str) -> String {
    format!("step:{step_id}")
}

/// Start a fresh instance's machine and return its snapshot.
pub fn start(
    template: &ProcessTemplate,
    layout: &Layout,
    instance: &ProcessInstance,
) -> Result<SmSnapshot, ProcessError> {
    let machine = build(template, layout)?;
    let mut sm = SmInstance::start_with_data(
        machine,
        json!({
            "instance": instance.id,
            "template": template.uri,
            "currentGroup": Value::Null,
        }),
    )
    .map_err(|error| ProcessError::Invalid(format!("state machine start failed: {error}")))?;
    sm.dispatch(SmEvent::new("start"))
        .map_err(|error| ProcessError::Invalid(format!("state machine start failed: {error}")))?;
    Ok(sm.snapshot())
}

/// Dispatch an event on a restored machine, refusing illegal transitions.
pub fn dispatch(
    template: &ProcessTemplate,
    layout: &Layout,
    snapshot: &SmSnapshot,
    event: &str,
) -> Result<SmSnapshot, ProcessError> {
    let machine = build(template, layout)?;
    let mut sm = SmInstance::restore(machine, snapshot, &[]).map_err(|error| {
        ProcessError::Invalid(format!("process state could not be restored: {error}"))
    })?;
    sm.dispatch(SmEvent::new(event))
        .map_err(|error| ProcessError::Invalid(format!("illegal process transition: {error}")))?;
    Ok(sm.snapshot())
}

/// A snapshot positioned at a group's leader without replaying transitions.
///
/// Retry uses this after a terminal transition, when no legal path back to the
/// group exists; `restore` validates the state is declared on the definition.
pub fn at_state(
    template: &ProcessTemplate,
    layout: &Layout,
    group: &crate::template::Group,
    instance: &ProcessInstance,
) -> Result<SmSnapshot, ProcessError> {
    let machine = build(template, layout)?;
    let leader = step_state(&template.steps[group.steps[0]].id);
    let snapshot = SmSnapshot {
        id: instance.id.clone(),
        parent: instance.parent.clone(),
        machine: machine.name.clone(),
        state: StateId::new(leader),
        resume: None,
        data: json!({
            "instance": instance.id,
            "template": template.uri,
            "currentGroup": group.key,
        }),
        transitions: 0,
        regions: Vec::new(),
        children: Vec::new(),
    };
    // Validate the state exists on the definition before the engine persists it.
    SmInstance::restore(machine, &snapshot, &[]).map_err(|error| {
        ProcessError::Invalid(format!("retry state could not be restored: {error}"))
    })?;
    Ok(snapshot)
}

/// The current group key implied by a snapshot, when it names a group.
pub fn current_group(
    template: &ProcessTemplate,
    layout: &Layout,
    snapshot: &SmSnapshot,
) -> Option<String> {
    layout.groups.iter().find_map(|group| {
        let leader = step_state(&template.steps[group.steps[0]].id);
        (snapshot.state.as_str() == leader).then(|| group.key.clone())
    })
}

/// Whether the snapshot is in a wait/pause state.
pub fn is_waiting(snapshot: &SmSnapshot) -> bool {
    snapshot.state.as_str() == "waiting"
}

/// Whether the snapshot is paused.
pub fn is_paused(snapshot: &SmSnapshot) -> bool {
    snapshot.state.as_str() == "paused"
}

/// Whether the snapshot has finished.
pub fn is_terminal(snapshot: &SmSnapshot) -> bool {
    matches!(
        snapshot.state.as_str(),
        "completed" | "cancelled" | "failed"
    )
}

/// The step states that were interrupted, for recovery.
pub fn interrupted_steps(instance: &ProcessInstance) -> Vec<String> {
    instance
        .steps
        .iter()
        .filter(|(_, state)| state.status == StepStatus::Running)
        .map(|(id, _)| id.clone())
        .collect()
}

/// A compact, serializable view used by the inspector.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineView {
    /// The current state name.
    pub state: String,
    /// Groups and their leaders.
    pub groups: BTreeMap<String, String>,
}
