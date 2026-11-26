//! Orthogonal regions: subsystems that run in parallel with the session.
//!
//! Each region is an `mm-sm` instance entered when the session starts. The
//! session driver forwards every session event to all regions; a region ignores
//! events it has no transition for, so unrelated traffic is harmless.

pub mod improvement;
pub mod memory;
pub mod plugins;
pub mod preferences;
pub mod proactive;
pub mod voice;

use mm_sm::{SmEvent, SmInstance, StateMachine, TransitionRecord};
use serde_json::Value;

use crate::error::Result;

/// A named region instance.
#[derive(Debug)]
pub struct RegionSm {
    /// Region name (`proactive`, `voice`, …).
    pub name: &'static str,
    /// The state-machine instance.
    pub sm: SmInstance,
}

impl RegionSm {
    /// Enter a region in its machine's initial state.
    pub fn new(name: &'static str, machine: StateMachine) -> Result<Self> {
        Ok(Self {
            name,
            sm: SmInstance::start(machine)?,
        })
    }

    /// The current state.
    pub fn state(&self) -> &str {
        self.sm.state().as_str()
    }

    /// Dispatch a raw event name with an optional payload.
    pub fn dispatch_name(
        &mut self,
        name: &str,
        payload: Option<Value>,
    ) -> Result<Vec<TransitionRecord>> {
        let mut event = SmEvent::new(name);
        if let Some(payload) = payload {
            event = event.with_payload(payload);
        }
        Ok(self.sm.dispatch(event)?)
    }
}

/// The state and transition names of every region, for tests and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionSnapshot {
    /// Region name.
    pub name: &'static str,
    /// Current state.
    pub state: String,
    /// Transitions taken so far.
    pub transitions: u64,
}

impl RegionSnapshot {
    /// Snapshot one region.
    pub fn of(region: &RegionSm) -> Self {
        Self {
            name: region.name,
            state: region.state().to_owned(),
            transitions: region.sm.transition_count(),
        }
    }
}
