//! Proactive region: evaluate, select, plan, execute and inject suggestions.
//!
//! [`ProactiveEngine`] runs the cycle over the plugin host and injects the
//! selected items into the session; this machine records the lifecycle.

use std::collections::VecDeque;

use mm_sm::{SmEvent, StateMachine, Transition, TransitionRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::Result;
use crate::plugins::{PluginContext, PluginHost, ProactiveProposal};
use crate::regions::RegionSm;
use crate::session::Session;

/// Proactive lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProactiveState {
    /// Waiting for the next cycle.
    Idle,
    /// Scoring candidates.
    Evaluating,
    /// Selecting a diverse top-N.
    Selecting,
    /// Building a mini plan.
    Planning,
    /// Running the proactive turn.
    Executing,
    /// Merging frames into the session stream.
    Injecting,
    /// Budget exhausted or the user is busy.
    Suppressed,
    /// Cycle finished.
    Completed,
}

impl ProactiveState {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Evaluating => "evaluating",
            Self::Selecting => "selecting",
            Self::Planning => "planning",
            Self::Executing => "executing",
            Self::Injecting => "injecting",
            Self::Suppressed => "suppressed",
            Self::Completed => "completed",
        }
    }
}

/// Events the proactive region reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum ProactiveEvent {
    /// A scheduler tick.
    Tick,
    /// Budget became available.
    BudgetAvailable,
    /// Candidates were scored.
    CandidateScored(Vec<String>),
    /// A selection was made.
    SelectionMade(Vec<String>),
    /// A proactive turn was spawned.
    TurnSpawned(String),
    /// That turn completed.
    TurnCompleted,
    /// Frames were injected.
    FramesInjected,
    /// The budget ran out.
    BudgetExhausted,
    /// The user typed or spoke.
    UserActivityDetected,
}

impl ProactiveEvent {
    /// The wire name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Tick => "tick",
            Self::BudgetAvailable => "budget_available",
            Self::CandidateScored(_) => "candidates_scored",
            Self::SelectionMade(_) => "selection_made",
            Self::TurnSpawned(_) => "turn_spawned",
            Self::TurnCompleted => "turn_completed",
            Self::FramesInjected => "frames_injected",
            Self::BudgetExhausted => "budget_exhausted",
            Self::UserActivityDetected => "user_activity",
        }
    }

    fn into_sm(self) -> SmEvent {
        let name = self.name();
        match self {
            Self::CandidateScored(items) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "items": items }))
            }
            Self::SelectionMade(items) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "items": items }))
            }
            Self::TurnSpawned(id) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "turn_id": id }))
            }
            _ => SmEvent::new(name),
        }
    }
}

/// The proactive state-machine definition.
pub fn proactive_machine() -> StateMachine {
    StateMachine::new("proactive", ProactiveState::Idle.as_str())
        .state(ProactiveState::Evaluating.as_str())
        .state(ProactiveState::Selecting.as_str())
        .state(ProactiveState::Planning.as_str())
        .state(ProactiveState::Executing.as_str())
        .state(ProactiveState::Injecting.as_str())
        .state(ProactiveState::Suppressed.as_str())
        .state(ProactiveState::Completed.as_str())
        .transition(Transition::new("idle", "tick", "evaluating"))
        .transition(Transition::new(
            "evaluating",
            "candidates_scored",
            "selecting",
        ))
        .transition(Transition::new(
            "evaluating",
            "budget_exhausted",
            "suppressed",
        ))
        .transition(Transition::new("evaluating", "user_activity", "suppressed"))
        .transition(Transition::new("selecting", "selection_made", "planning"))
        .transition(Transition::new("planning", "turn_spawned", "executing"))
        .transition(Transition::new("executing", "turn_completed", "injecting"))
        .transition(Transition::new("injecting", "frames_injected", "completed"))
        .transition(Transition::new("completed", "tick", "idle"))
        .transition(Transition::new("suppressed", "tick", "idle"))
        .transition(Transition::new("suppressed", "budget_available", "idle"))
}

/// One suggestion injected into the session stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProactiveItem {
    /// Plugin that proposed it.
    pub plugin: String,
    /// Short heading.
    pub title: String,
    /// The suggestion itself.
    pub body: String,
    /// 0.0–1.0 relevance.
    pub score: f32,
    /// Trigger that fired.
    pub trigger: String,
}

impl From<ProactiveProposal> for ProactiveItem {
    fn from(proposal: ProactiveProposal) -> Self {
        Self {
            plugin: proposal.plugin,
            title: proposal.title,
            body: proposal.body,
            score: proposal.score,
            trigger: proposal.trigger,
        }
    }
}

/// How many suggestions one cycle may inject.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProactiveConfig {
    /// Maximum items injected per cycle.
    pub max_items: usize,
    /// Proposals below this score are dropped.
    pub min_score: f32,
    /// How many recent cycles a repeated title is suppressed for.
    pub repeat_window: usize,
}

impl Default for ProactiveConfig {
    fn default() -> Self {
        Self {
            max_items: 3,
            min_score: 0.0,
            repeat_window: 8,
        }
    }
}

/// Runs a proactive cycle over the plugin host and injects the winners.
#[derive(Debug)]
pub struct ProactiveEngine {
    host: PluginHost,
    config: ProactiveConfig,
    cycles: u64,
    recent: VecDeque<String>,
}

impl ProactiveEngine {
    /// Build an engine over an existing host.
    pub fn new(host: PluginHost, config: ProactiveConfig) -> Self {
        Self {
            host,
            config,
            cycles: 0,
            recent: VecDeque::new(),
        }
    }

    /// Build an engine with all built-in plugins loaded.
    pub fn with_builtins(config: ProactiveConfig) -> Result<Self> {
        Ok(Self::new(
            PluginHost::with_builtins(Default::default())?,
            config,
        ))
    }

    /// The plugin host.
    pub fn host(&self) -> &PluginHost {
        &self.host
    }

    /// Mutable access for loading plugins.
    pub fn host_mut(&mut self) -> &mut PluginHost {
        &mut self.host
    }

    /// Cycles completed.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Run one cycle and inject the selected items.
    ///
    /// The cycle starts from `idle` and resets `completed` or `suppressed` with a
    /// tick. A region that is mid-cycle (or otherwise in flight) yields nothing.
    pub fn run(
        &mut self,
        session: &mut Session,
        ctx: &PluginContext<'_>,
    ) -> Result<Vec<ProactiveItem>> {
        match session.region_state("proactive") {
            Some("idle") => {}
            Some("completed" | "suppressed") => session.dispatch_regions("tick", None)?,
            _ => return Ok(Vec::new()),
        }
        if session.region_state("proactive") != Some("idle") {
            return Ok(Vec::new());
        }
        session.dispatch_regions("tick", None)?;

        let mut proposals = self.host.evaluate(ctx);
        proposals.retain(|proposal| proposal.score >= self.config.min_score);
        proposals.retain(|proposal| !self.recent.contains(&proposal.title));
        proposals.truncate(self.config.max_items.min(self.host.remaining_executions()));

        let titles: Vec<String> = proposals
            .iter()
            .map(|proposal| proposal.title.clone())
            .collect();
        session.dispatch_regions("candidates_scored", Some(json!({ "items": titles })))?;
        session.dispatch_regions("selection_made", Some(json!({ "items": titles })))?;
        let turn_id = format!("proactive-{}", self.cycles + 1);
        session.dispatch_regions("turn_spawned", Some(json!({ "turn_id": turn_id })))?;
        session.dispatch_regions("turn_completed", None)?;

        let mut items = Vec::new();
        for proposal in proposals {
            match self.host.execute(&proposal) {
                Ok(_) => items.push(ProactiveItem::from(proposal)),
                Err(error) => {
                    // Inject what succeeded and finish the cycle so the region is
                    // not stranded in `injecting` by a misbehaving plugin.
                    session.inject_proactive(items);
                    session.dispatch_regions("frames_injected", None)?;
                    self.host.reset_cycle();
                    return Err(error);
                }
            }
        }
        for item in &items {
            self.recent.push_back(item.title.clone());
        }
        while self.recent.len() > self.config.repeat_window {
            self.recent.pop_front();
        }
        session.inject_proactive(items.clone());
        session.dispatch_regions("frames_injected", None)?;
        self.host.reset_cycle();
        self.cycles = self.cycles.saturating_add(1);
        Ok(items)
    }
}

/// A live proactive region.
#[derive(Debug)]
pub struct ProactiveRegion {
    /// The region instance.
    pub sm: RegionSm,
}

impl ProactiveRegion {
    /// Enter the region.
    pub fn new() -> Result<Self> {
        Ok(Self {
            sm: RegionSm::new("proactive", proactive_machine())?,
        })
    }

    /// The current state.
    pub fn state(&self) -> &str {
        self.sm.state()
    }

    /// Dispatch one event.
    pub fn dispatch(&mut self, event: ProactiveEvent) -> Result<Vec<TransitionRecord>> {
        Ok(self.sm.sm.dispatch(event.into_sm())?)
    }

    /// Dispatch a raw event name, used by the session driver.
    pub fn dispatch_name(&mut self, name: &str, payload: Option<Value>) -> Result<()> {
        self.sm.dispatch_name(name, payload)?;
        Ok(())
    }
}
