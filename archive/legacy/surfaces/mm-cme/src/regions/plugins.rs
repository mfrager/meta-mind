//! Plugin region: loading, validation, registration and the plugin sandbox.
//!
//! The host that loads and runs plugins is [`PluginHost`](crate::plugins::PluginHost);
//! this machine records the lifecycle around it.

use mm_sm::{SmEvent, StateMachine, Transition, TransitionRecord};
use serde_json::Value;

use crate::error::Result;
use crate::regions::RegionSm;

/// Plugin lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    /// Loading a plugin.
    Loading,
    /// Checking its manifest and capabilities.
    Validating,
    /// Triggers are registered.
    Registered,
    /// Evaluating proposals.
    Running,
    /// Suspended by budget or resource pressure.
    Suspended,
    /// Hot reloading.
    Updating,
    /// Unloading.
    Unloading,
    /// Load or validation failed.
    Failed,
}

impl PluginState {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Validating => "validating",
            Self::Registered => "registered",
            Self::Running => "running",
            Self::Suspended => "suspended",
            Self::Updating => "updating",
            Self::Unloading => "unloading",
            Self::Failed => "failed",
        }
    }
}

/// Events the plugin region reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum PluginEvent {
    /// Load a plugin.
    LoadRequest(String),
    /// Validation passed with a manifest summary.
    ValidationOk(String),
    /// Validation failed.
    ValidationFailed(String),
    /// A trigger fired.
    TriggerFired(String),
    /// A proposal was received.
    ProposalReceived,
    /// Execution finished.
    ExecutionComplete,
    /// Resources are tight.
    BudgetPressure,
    /// Resources recovered.
    BudgetAvailable,
    /// Hot reload.
    ReloadRequest(String),
    /// Reload finished.
    UpdateComplete,
    /// Unload.
    UnloadRequest(String),
    /// Unload finished.
    UnloadComplete,
}

impl PluginEvent {
    /// The wire name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::LoadRequest(_) => "load_request",
            Self::ValidationOk(_) => "validation_ok",
            Self::ValidationFailed(_) => "validation_failed",
            Self::TriggerFired(_) => "trigger_fired",
            Self::ProposalReceived => "proposal_received",
            Self::ExecutionComplete => "execution_complete",
            Self::BudgetPressure => "budget_pressure",
            Self::BudgetAvailable => "budget_available",
            Self::ReloadRequest(_) => "reload_request",
            Self::UpdateComplete => "update_complete",
            Self::UnloadRequest(_) => "unload_request",
            Self::UnloadComplete => "unload_complete",
        }
    }

    fn into_sm(self) -> SmEvent {
        let name = self.name();
        match self {
            Self::LoadRequest(id) | Self::ReloadRequest(id) | Self::UnloadRequest(id) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "plugin": id }))
            }
            Self::ValidationOk(summary) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "manifest": summary }))
            }
            Self::ValidationFailed(detail) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "detail": detail }))
            }
            Self::TriggerFired(trigger) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "trigger": trigger }))
            }
            _ => SmEvent::new(name),
        }
    }
}

/// The plugin state-machine definition.
pub fn plugins_machine() -> StateMachine {
    StateMachine::new("plugins", PluginState::Loading.as_str())
        .state(PluginState::Validating.as_str())
        .state(PluginState::Registered.as_str())
        .state(PluginState::Running.as_str())
        .state(PluginState::Suspended.as_str())
        .state(PluginState::Updating.as_str())
        .state(PluginState::Unloading.as_str())
        .state(PluginState::Failed.as_str())
        .transition(Transition::new("loading", "load_request", "validating"))
        .transition(Transition::new("validating", "validation_ok", "registered"))
        .transition(Transition::new("validating", "validation_failed", "failed"))
        .transition(Transition::new("registered", "trigger_fired", "running"))
        .transition(Transition::new(
            "running",
            "execution_complete",
            "registered",
        ))
        .transition(Transition::new(
            "registered",
            "budget_pressure",
            "suspended",
        ))
        .transition(Transition::new("running", "budget_pressure", "suspended"))
        .transition(Transition::new(
            "suspended",
            "budget_available",
            "registered",
        ))
        .transition(Transition::new("registered", "reload_request", "updating"))
        .transition(Transition::new("updating", "update_complete", "registered"))
        .transition(Transition::new("registered", "unload_request", "unloading"))
        .transition(Transition::new("unloading", "load_request", "validating"))
        .transition(Transition::new("failed", "load_request", "validating"))
        .transition(Transition::new("updating", "validation_failed", "failed"))
}

/// A live plugin region.
#[derive(Debug)]
pub struct PluginRegion {
    /// The region instance.
    pub sm: RegionSm,
}

impl PluginRegion {
    /// Enter the region.
    pub fn new() -> Result<Self> {
        Ok(Self {
            sm: RegionSm::new("plugins", plugins_machine())?,
        })
    }

    /// The current state.
    pub fn state(&self) -> &str {
        self.sm.state()
    }

    /// Dispatch one event.
    pub fn dispatch(&mut self, event: PluginEvent) -> Result<Vec<TransitionRecord>> {
        Ok(self.sm.sm.dispatch(event.into_sm())?)
    }

    /// Dispatch a raw event name, used by the session driver.
    pub fn dispatch_name(&mut self, name: &str, payload: Option<Value>) -> Result<()> {
        self.sm.dispatch_name(name, payload)?;
        Ok(())
    }
}
