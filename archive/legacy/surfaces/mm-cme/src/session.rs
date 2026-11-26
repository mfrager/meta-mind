//! Session: the top-level state machine with six orthogonal regions.
//!
//! The session owns lifecycle and the shared context. Per-turn work is a
//! [`Turn`] spawned by the driver; the regions run in
//! parallel and receive every session event, ignoring ones they do not handle.

use std::collections::BTreeMap;

use mm_data::{ProjectId, SessionId, UserId};
use mm_sm::{SmEvent, SmInstance, StateMachine, Transition, TransitionRecord};
use serde::{Deserialize, Serialize};

use crate::error::{CmeError, Result};
use crate::regions::proactive::ProactiveItem;
use crate::regions::RegionSnapshot;
use crate::turn::{Turn, TurnOutcome, TurnPlan};
use crate::voice::{VoiceTier, VoiceTurnConfig};

/// Session lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Loading memory and regions.
    Initializing,
    /// Accepting input.
    Active,
    /// The user stepped away.
    Paused,
    /// Explicitly suspended.
    Suspended,
    /// Restoring context.
    Resuming,
    /// Shutting down.
    Terminating,
    /// Closed.
    Terminated,
}

impl SessionState {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Suspended => "suspended",
            Self::Resuming => "resuming",
            Self::Terminating => "terminating",
            Self::Terminated => "terminated",
        }
    }

    /// Whether the session accepts a new turn.
    pub fn accepts_turns(self) -> bool {
        self == Self::Active
    }
}

/// The shared context every region reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionContext {
    /// Project and previous session, for resume.
    pub memory: MemoryContext,
    /// Merged preference values.
    pub preferences: PreferenceProfile,
    /// Current affect.
    pub affect: AffectState,
    /// Budgets for this session.
    pub budgets: SessionBudgets,
}

/// The memory slice of the shared context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MemoryContext {
    /// Project.
    pub project_id: Option<ProjectId>,
    /// Previous session.
    pub previous_session_id: Option<SessionId>,
    /// Greeting produced by session continuity.
    #[serde(default)]
    pub greeting: Option<String>,
    /// Project narrative restored from memory.
    #[serde(default)]
    pub project_summary: Option<String>,
    /// Topics from the previous session.
    #[serde(default)]
    pub previous_topics: Vec<String>,
}

/// Merged preference values, by dimension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PreferenceProfile {
    /// Dimension → −1.0..1.0.
    pub values: BTreeMap<String, f32>,
}

impl PreferenceProfile {
    /// A value, defaulting to neutral.
    pub fn value(&self, dimension: &str) -> f32 {
        self.values.get(dimension).copied().unwrap_or(0.0)
    }
}

/// Affect in force.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AffectState {
    /// Valence, −1.0 to 1.0.
    pub valence: f32,
    /// Arousal, 0.0 to 1.0.
    pub arousal: f32,
    /// Confidence, 0.0 to 1.0.
    pub confidence: f32,
}

impl Default for AffectState {
    fn default() -> Self {
        Self {
            valence: 0.0,
            arousal: 0.5,
            confidence: 0.5,
        }
    }
}

/// Budgets for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBudgets {
    /// Tokens available per turn.
    pub turn_token_budget: u64,
    /// Turns allowed in the session.
    pub max_turns: u32,
}

impl Default for SessionBudgets {
    fn default() -> Self {
        Self {
            turn_token_budget: 8_000,
            max_turns: 1_000,
        }
    }
}

/// Events that drive the session.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// Regions and memory are ready.
    InitComplete,
    /// The user stepped away.
    UserAway,
    /// The user came back.
    UserReturn,
    /// Explicit suspend.
    SuspendRequest,
    /// Resume a suspended session.
    ResumeRequest,
    /// Resume finished.
    ResumeComplete,
    /// Shut down.
    ShutdownRequest,
    /// Every region reached a terminal state.
    RegionsTerminated,
    /// The user sent input.
    UserInput(String),
    /// Voice audio arrived.
    VoiceAudio,
    /// A scheduler tick.
    Tick,
    /// A turn finished.
    TurnCompleted(Box<TurnOutcome>),
    /// Barge-in from the UI.
    BargeIn,
}

impl SessionEvent {
    /// The wire name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::InitComplete => "init_complete",
            Self::UserAway => "user_away",
            Self::UserReturn => "user_return",
            Self::SuspendRequest => "suspend_request",
            Self::ResumeRequest => "resume_request",
            Self::ResumeComplete => "resume_complete",
            Self::ShutdownRequest => "shutdown_request",
            Self::RegionsTerminated => "regions_terminated",
            Self::UserInput(_) => "user_input",
            Self::VoiceAudio => "voice_audio",
            Self::Tick => "tick",
            Self::TurnCompleted(_) => "turn_completed",
            Self::BargeIn => "barge_in",
        }
    }

    fn into_sm(self) -> SmEvent {
        let name = self.name();
        match self {
            Self::UserInput(text) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "text": text }))
            }
            Self::TurnCompleted(outcome) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "outcome": outcome }))
            }
            _ => SmEvent::new(name),
        }
    }
}

/// The six region names, in driver dispatch order.
pub const REGION_NAMES: [&str; 6] = [
    "proactive",
    "voice",
    "memory",
    "preferences",
    "plugins",
    "improvement",
];

/// The session state-machine definition, including its regions.
pub fn session_machine() -> StateMachine {
    StateMachine::new("session", SessionState::Initializing.as_str())
        .state(SessionState::Active.as_str())
        .state(SessionState::Paused.as_str())
        .state(SessionState::Suspended.as_str())
        .state(SessionState::Resuming.as_str())
        .state(SessionState::Terminating.as_str())
        .state(SessionState::Terminated.as_str())
        .transition(Transition::new("initializing", "init_complete", "active"))
        .transition(Transition::new("active", "user_away", "paused"))
        .transition(Transition::new("paused", "user_return", "active"))
        .transition(Transition::new("active", "suspend_request", "suspended"))
        .transition(Transition::new("suspended", "resume_request", "resuming"))
        .transition(Transition::new("resuming", "resume_complete", "active"))
        .transition(Transition::new("active", "shutdown_request", "terminating"))
        .transition(Transition::any("shutdown_request", "terminating"))
        .transition(Transition::new(
            "terminating",
            "regions_terminated",
            "terminated",
        ))
        .region(crate::regions::proactive::proactive_machine())
        .region(crate::regions::voice::voice_machine())
        .region(crate::regions::memory::memory_machine())
        .region(crate::regions::preferences::preferences_machine())
        .region(crate::regions::plugins::plugins_machine())
        .region(crate::regions::improvement::improvement_machine())
}

/// A live session.
#[derive(Debug)]
pub struct Session {
    /// The state-machine instance (with its six regions).
    pub sm: SmInstance,
    /// Session identifier.
    pub id: SessionId,
    /// User.
    pub user_id: UserId,
    /// Project, if any.
    pub project_id: Option<ProjectId>,
    /// Shared context.
    pub context: SessionContext,
    /// The turn currently running, if any.
    pub active_turn: Option<Turn>,
    /// Turns completed in this session.
    pub turns_completed: u32,
    /// Proactive items injected into the session stream.
    pub proactive_items: Vec<ProactiveItem>,
    /// Barge-ins observed in this session.
    pub barge_in_count: u32,
    /// Optional transition observer for the session and its regions.
    pub observer: Option<crate::observability::ObserverHandle>,
}

impl Session {
    /// Create a session in `initializing`.
    pub fn new(user_id: impl Into<UserId>, project_id: Option<ProjectId>) -> Result<Self> {
        let context = SessionContext {
            memory: MemoryContext {
                project_id: project_id.clone(),
                previous_session_id: None,
                ..MemoryContext::default()
            },
            ..SessionContext::default()
        };
        let sm = SmInstance::start_with_data(
            session_machine(),
            serde_json::json!({ "session": context }),
        )?;
        Ok(Self {
            sm,
            id: SessionId::new(mm_data::rdf::new_id()),
            user_id: user_id.into(),
            project_id,
            context,
            active_turn: None,
            turns_completed: 0,
            proactive_items: Vec::new(),
            barge_in_count: 0,
            observer: None,
        })
    }

    fn notify(&self, records: &[TransitionRecord]) {
        if let Some(observer) = &self.observer {
            for record in records {
                observer.on_transition(record);
            }
        }
    }

    /// Record one barge-in.
    pub fn note_barge_in(&mut self) {
        self.barge_in_count = self.barge_in_count.saturating_add(1);
    }

    /// Inject proactive items into the session stream.
    ///
    /// Items are appended in the order the proactive engine surfaced them; a UI
    /// reads them with [`Session::drain_proactive`].
    pub fn inject_proactive(&mut self, items: impl IntoIterator<Item = ProactiveItem>) {
        self.proactive_items.extend(items);
    }

    /// Take the proactive items surfaced since the last drain.
    pub fn drain_proactive(&mut self) -> Vec<ProactiveItem> {
        std::mem::take(&mut self.proactive_items)
    }

    /// The current state.
    pub fn state(&self) -> &str {
        self.sm.state().as_str()
    }

    /// The typed current state.
    pub fn session_state(&self) -> Option<SessionState> {
        match self.state() {
            "initializing" => Some(SessionState::Initializing),
            "active" => Some(SessionState::Active),
            "paused" => Some(SessionState::Paused),
            "suspended" => Some(SessionState::Suspended),
            "resuming" => Some(SessionState::Resuming),
            "terminating" => Some(SessionState::Terminating),
            "terminated" => Some(SessionState::Terminated),
            _ => None,
        }
    }

    /// Dispatch one session event.
    ///
    /// `mm-sm` forwards the event to the orthogonal regions as part of the same
    /// dispatch, so the returned records already include every region transition.
    pub fn dispatch(&mut self, event: SessionEvent) -> Result<Vec<TransitionRecord>> {
        let records = self.sm.dispatch(event.into_sm())?;
        self.notify(&records);
        Ok(records)
    }

    /// Dispatch a raw event name to every region, without touching the session
    /// machine. Used for region-only traffic such as `start_voice`.
    pub fn dispatch_regions(
        &mut self,
        name: &str,
        payload: Option<serde_json::Value>,
    ) -> Result<()> {
        let mut event = SmEvent::new(name);
        if let Some(payload) = payload {
            event = event.with_payload(payload);
        }
        let mut records = Vec::new();
        for region in REGION_NAMES {
            if let Some(instance) = self.sm.region_mut(region) {
                records.extend(instance.dispatch(event.clone())?);
            }
        }
        self.notify(&records);
        Ok(())
    }

    /// One region's current state.
    pub fn region_state(&self, name: &str) -> Option<&str> {
        self.sm.region(name).map(|region| region.state().as_str())
    }

    /// A snapshot of every region.
    pub fn region_snapshots(&self) -> Vec<RegionSnapshot> {
        REGION_NAMES
            .iter()
            .copied()
            .filter_map(|name| {
                self.sm.region(name).map(|region| RegionSnapshot {
                    name,
                    state: region.state().as_str().to_owned(),
                    transitions: region.transition_count(),
                })
            })
            .collect()
    }

    /// Begin a turn, if the session is active and idle.
    pub fn begin_turn(&mut self, query: impl Into<String>) -> Result<&mut Turn> {
        let state = self.state().to_owned();
        if !SessionState::Active.as_str().eq(&state) {
            return Err(CmeError::NotActive {
                session: self.id.to_string(),
                state,
            });
        }
        if self.active_turn.is_some() {
            return Err(CmeError::TurnInProgress {
                session: self.id.to_string(),
            });
        }
        let turn_id = mm_data::TurnId::new(mm_data::rdf::new_id());
        let plan = TurnPlan::simple(self.id.clone(), turn_id, query);
        self.active_turn = Some(Turn::new(plan)?);
        Ok(self.active_turn.as_mut().expect("turn was just inserted"))
    }

    /// The active voice tier, derived from the voice region's state.
    pub fn voice_tier(&self) -> Option<VoiceTier> {
        match self.region_state("voice")? {
            "tier1_request_response" => Some(VoiceTier::Tier1),
            "tier2_streaming" | "recovering" | "barged_in" => Some(VoiceTier::Tier2),
            "tier3_full_duplex" => Some(VoiceTier::Tier3),
            _ => None,
        }
    }

    /// The voice configuration a turn should carry, if the session is in voice.
    pub fn voice_turn_config(&self) -> Option<VoiceTurnConfig> {
        self.voice_tier().map(VoiceTurnConfig::for_tier)
    }

    /// The active turn, if any.
    pub fn active_turn(&self) -> Option<&Turn> {
        self.active_turn.as_ref()
    }

    /// Mutable access to the active turn.
    pub fn active_turn_mut(&mut self) -> Option<&mut Turn> {
        self.active_turn.as_mut()
    }

    /// Finish the active turn and record it.
    pub fn finish_turn(&mut self) -> Result<TurnOutcome> {
        let turn = self.active_turn.take().ok_or_else(|| CmeError::NotFound {
            kind: "active turn",
            id: self.id.to_string(),
        })?;
        let outcome = turn.outcome();
        self.turns_completed = self.turns_completed.saturating_add(1);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::new(UserId::new("u1"), None).unwrap()
    }

    #[test]
    fn session_lifecycle_transitions() {
        let mut session = session();
        assert_eq!(session.state(), SessionState::Initializing.as_str());
        session.dispatch(SessionEvent::InitComplete).unwrap();
        assert_eq!(session.state(), SessionState::Active.as_str());
        session.dispatch(SessionEvent::UserAway).unwrap();
        assert_eq!(session.state(), SessionState::Paused.as_str());
        session.dispatch(SessionEvent::UserReturn).unwrap();
        session.dispatch(SessionEvent::SuspendRequest).unwrap();
        assert_eq!(session.state(), SessionState::Suspended.as_str());
        session.dispatch(SessionEvent::ResumeRequest).unwrap();
        assert_eq!(session.state(), SessionState::Resuming.as_str());
        session.dispatch(SessionEvent::ResumeComplete).unwrap();
        assert_eq!(session.state(), SessionState::Active.as_str());
        session.dispatch(SessionEvent::ShutdownRequest).unwrap();
        assert_eq!(session.state(), SessionState::Terminating.as_str());
        session.dispatch(SessionEvent::RegionsTerminated).unwrap();
        assert_eq!(session.state(), SessionState::Terminated.as_str());
    }

    #[test]
    fn every_region_is_entered() {
        let session = session();
        for name in REGION_NAMES {
            assert!(
                session.region_state(name).is_some(),
                "region {name} missing"
            );
        }
        assert_eq!(session.region_state("proactive"), Some("idle"));
        assert_eq!(session.region_state("voice"), Some("idle"));
        assert_eq!(session.region_state("memory"), Some("episodic"));
        assert_eq!(session.region_state("preferences"), Some("cold_start"));
        assert_eq!(session.region_state("plugins"), Some("loading"));
        assert_eq!(session.region_state("improvement"), Some("idle"));
    }

    #[test]
    fn turns_only_start_while_active() {
        let mut session = session();
        assert!(session.begin_turn("hi").is_err());
        session.dispatch(SessionEvent::InitComplete).unwrap();
        assert!(session.begin_turn("hi").is_ok());
        assert!(session.begin_turn("again").is_err());
    }
}
