//! Preference region: the lifecycle for one posterior per dimension.
//!
//! The numbers live in [`PreferenceLearner`](crate::preferences::PreferenceLearner);
//! this machine records whether each dimension is cold, observing, converging,
//! stable or relearning after drift.

use mm_sm::{SmEvent, StateMachine, Transition, TransitionRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;
use crate::regions::RegionSm;

/// Preference-learning lifecycle, per dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreferenceState {
    /// No observations.
    ColdStart,
    /// Gathering signals.
    Observing,
    /// Variance is decreasing.
    Converging,
    /// Stable enough to apply to the planner.
    Stable,
    /// New signals contradict the posterior.
    DriftDetected,
    /// Adapting.
    Relearning,
}

impl PreferenceState {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ColdStart => "cold_start",
            Self::Observing => "observing",
            Self::Converging => "converging",
            Self::Stable => "stable",
            Self::DriftDetected => "drift_detected",
            Self::Relearning => "relearning",
        }
    }
}

/// Events the preference region reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum PreferenceEvent {
    /// A behavioural signal.
    SignalObserved {
        /// Dimension name.
        dimension: String,
        /// −1.0 to 1.0.
        value: f32,
        /// Observation weight.
        weight: f32,
    },
    /// The posterior was updated.
    PosteriorUpdated,
    /// Variance crossed the threshold.
    ConvergenceReached,
    /// A contradiction was detected.
    DriftDetected(String),
    /// The planner consumed the value.
    AppliedToPlanner,
    /// Start over.
    ResetRequested,
}

impl PreferenceEvent {
    /// The wire name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::SignalObserved { .. } => "signal_observed",
            Self::PosteriorUpdated => "posterior_updated",
            Self::ConvergenceReached => "convergence_reached",
            Self::DriftDetected(_) => "drift_detected",
            Self::AppliedToPlanner => "applied_to_planner",
            Self::ResetRequested => "reset_requested",
        }
    }

    fn into_sm(self) -> SmEvent {
        let name = self.name();
        match self {
            Self::SignalObserved {
                dimension,
                value,
                weight,
            } => SmEvent::new(name).with_payload(serde_json::json!({
                "dimension": dimension,
                "value": value,
                "weight": weight,
            })),
            Self::DriftDetected(dimension) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "dimension": dimension }))
            }
            _ => SmEvent::new(name),
        }
    }
}

/// The preference state-machine definition.
pub fn preferences_machine() -> StateMachine {
    StateMachine::new("preferences", PreferenceState::ColdStart.as_str())
        .state(PreferenceState::Observing.as_str())
        .state(PreferenceState::Converging.as_str())
        .state(PreferenceState::Stable.as_str())
        .state(PreferenceState::DriftDetected.as_str())
        .state(PreferenceState::Relearning.as_str())
        .transition(Transition::new(
            "cold_start",
            "signal_observed",
            "observing",
        ))
        .transition(Transition::new(
            "observing",
            "convergence_reached",
            "converging",
        ))
        .transition(Transition::new(
            "converging",
            "convergence_reached",
            "stable",
        ))
        .transition(Transition::new(
            "converging",
            "drift_detected",
            "drift_detected",
        ))
        .transition(Transition::new(
            "stable",
            "drift_detected",
            "drift_detected",
        ))
        .transition(Transition::new(
            "drift_detected",
            "signal_observed",
            "relearning",
        ))
        .transition(Transition::new(
            "relearning",
            "convergence_reached",
            "converging",
        ))
        .transition(Transition::any("reset_requested", "cold_start"))
        .transition(Transition::new("stable", "applied_to_planner", "stable"))
}

/// A live preference region.
#[derive(Debug)]
pub struct PreferenceRegion {
    /// The region instance.
    pub sm: RegionSm,
}

impl PreferenceRegion {
    /// Enter the region.
    pub fn new() -> Result<Self> {
        Ok(Self {
            sm: RegionSm::new("preferences", preferences_machine())?,
        })
    }

    /// The current state.
    pub fn state(&self) -> &str {
        self.sm.state()
    }

    /// Dispatch one event.
    pub fn dispatch(&mut self, event: PreferenceEvent) -> Result<Vec<TransitionRecord>> {
        Ok(self.sm.sm.dispatch(event.into_sm())?)
    }

    /// Dispatch a raw event name, used by the session driver.
    pub fn dispatch_name(&mut self, name: &str, payload: Option<Value>) -> Result<()> {
        self.sm.dispatch_name(name, payload)?;
        Ok(())
    }
}
