//! Improvement region: the reflect → plan → act → verify → learn loop.

use mm_sm::{SmEvent, StateMachine, Transition, TransitionRecord};
use serde_json::Value;

use crate::error::Result;
use crate::regions::RegionSm;

/// Improvement lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImprovementState {
    /// Waiting for a cycle.
    Idle,
    /// Reflecting on recent outcomes.
    Reflecting,
    /// Choosing one improvement.
    Planning,
    /// Applying it.
    Acting,
    /// Measuring the result.
    Verifying,
    /// Restoring the last verified artifact.
    RollingBack,
    /// Updating the self-model.
    Learning,
    /// Cycle complete.
    Completed,
}

impl ImprovementState {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Reflecting => "reflecting",
            Self::Planning => "planning",
            Self::Acting => "acting",
            Self::Verifying => "verifying",
            Self::RollingBack => "rolling_back",
            Self::Learning => "learning",
            Self::Completed => "completed",
        }
    }
}

/// Events the improvement region reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum ImprovementEvent {
    /// Start a cycle.
    Start,
    /// Reflection finished with findings.
    ReflectionDone(Vec<String>),
    /// A change was chosen.
    PlanReady(String),
    /// The change was applied.
    ActionDone,
    /// Verification finished.
    VerificationDone,
    /// Verification failed and the baseline is being restored.
    RollbackTriggered,
    /// The baseline was restored.
    RollbackDone,
    /// The self-model was updated.
    Learned,
    /// A cycle finished.
    CycleComplete,
    /// Failure.
    Error(String),
}

impl ImprovementEvent {
    /// The wire name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::ReflectionDone(_) => "reflection_done",
            Self::PlanReady(_) => "plan_ready",
            Self::ActionDone => "action_done",
            Self::VerificationDone => "verification_done",
            Self::RollbackTriggered => "rollback_triggered",
            Self::RollbackDone => "rollback_done",
            Self::Learned => "learned",
            Self::CycleComplete => "cycle_complete",
            Self::Error(_) => "error",
        }
    }

    fn into_sm(self) -> SmEvent {
        let name = self.name();
        match self {
            Self::ReflectionDone(findings) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "findings": findings }))
            }
            Self::PlanReady(plan) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "plan": plan }))
            }
            Self::Error(detail) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "detail": detail }))
            }
            _ => SmEvent::new(name),
        }
    }
}

/// The improvement state-machine definition.
pub fn improvement_machine() -> StateMachine {
    StateMachine::new("improvement", ImprovementState::Idle.as_str())
        .state(ImprovementState::Reflecting.as_str())
        .state(ImprovementState::Planning.as_str())
        .state(ImprovementState::Acting.as_str())
        .state(ImprovementState::Verifying.as_str())
        .state(ImprovementState::RollingBack.as_str())
        .state(ImprovementState::Learning.as_str())
        .state(ImprovementState::Completed.as_str())
        .transition(Transition::new("idle", "start", "reflecting"))
        .transition(Transition::new("reflecting", "reflection_done", "planning"))
        .transition(Transition::new("planning", "plan_ready", "acting"))
        .transition(Transition::new("acting", "action_done", "verifying"))
        .transition(Transition::new(
            "verifying",
            "verification_done",
            "learning",
        ))
        .transition(Transition::new(
            "verifying",
            "rollback_triggered",
            "rolling_back",
        ))
        .transition(Transition::new("rolling_back", "rollback_done", "idle"))
        .transition(Transition::new("learning", "learned", "completed"))
        .transition(Transition::new("completed", "cycle_complete", "idle"))
        .transition(Transition::any("error", "idle"))
}

/// A live improvement region.
#[derive(Debug)]
pub struct ImprovementRegion {
    /// The region instance.
    pub sm: RegionSm,
}

impl ImprovementRegion {
    /// Enter the region.
    pub fn new() -> Result<Self> {
        Ok(Self {
            sm: RegionSm::new("improvement", improvement_machine())?,
        })
    }

    /// The current state.
    pub fn state(&self) -> &str {
        self.sm.state()
    }

    /// Dispatch one event.
    pub fn dispatch(&mut self, event: ImprovementEvent) -> Result<Vec<TransitionRecord>> {
        Ok(self.sm.sm.dispatch(event.into_sm())?)
    }

    /// Dispatch a raw event name, used by the session driver.
    pub fn dispatch_name(&mut self, name: &str, payload: Option<Value>) -> Result<()> {
        self.sm.dispatch_name(name, payload)?;
        Ok(())
    }
}
