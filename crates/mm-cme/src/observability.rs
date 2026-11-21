//! Observability: snapshots, transition events, metrics and trace records.
//!
//! The transport is deliberately not here: Phase 7's HTTP/WebSocket layer mounts
//! [`Observability::snapshot`] on `GET /api/cme/sessions/{id}/snapshot` and
//! subscribes to [`Observability::subscribe`] for `WS /api/cme/sessions/{id}/events`.
//! Transitions are also emitted as `tracing` events, so an OpenTelemetry
//! subscriber sees the state-machine hierarchy as spans and events.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use mm_sm::{SmSnapshot, TransitionRecord};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::chunk::ChunkKind;
use crate::session::Session;
use crate::turn::TurnPlan;

/// One state-machine transition, in wire form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmTransitionEvent {
    /// Unix milliseconds.
    pub timestamp: i64,
    /// Instance identifier.
    pub sm_id: String,
    /// Machine name (`session`, `turn`, `chunk`, `memory`, …).
    pub sm_type: String,
    /// State left.
    pub from: String,
    /// State entered.
    pub to: String,
    /// Event that fired.
    pub event: String,
    /// Composition depth; `0` is the dispatched root.
    pub depth: usize,
}

impl From<&TransitionRecord> for SmTransitionEvent {
    fn from(record: &TransitionRecord) -> Self {
        Self {
            timestamp: now_ms(),
            sm_id: record.instance.clone(),
            sm_type: record.machine.clone(),
            from: record.from.to_string(),
            to: record.to.to_string(),
            event: record.event.clone(),
            depth: record.depth,
        }
    }
}

/// A receiver for state-machine transitions.
pub trait SmObserver: Send + Sync + fmt::Debug {
    /// Record one transition.
    fn on_transition(&self, record: &TransitionRecord);
}

/// One region's current state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionView {
    /// Region name.
    pub name: String,
    /// Current state.
    pub state: String,
    /// Transitions taken.
    pub transitions: u64,
}

impl RegionView {
    fn of(session: &Session, name: &str) -> Self {
        let region = session.sm.region(name);
        Self {
            name: name.to_owned(),
            state: region
                .map(|region| region.state().as_str().to_owned())
                .unwrap_or_else(|| "missing".to_owned()),
            transitions: region.map_or(0, |region| region.transition_count()),
        }
    }
}

/// The voice slice of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceView {
    /// Voice region state.
    pub state: String,
    /// Negotiated tier, when in a tier.
    pub tier: Option<String>,
    /// Barge-ins seen by the session.
    pub barge_in_count: u32,
}

/// The preferences slice of a snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreferencesView {
    /// Region state.
    pub state: String,
    /// Merged values in force.
    pub values: BTreeMap<String, f32>,
}

/// One chunk in a turn snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkSnapshot {
    /// Chunk identifier.
    pub id: String,
    /// What it emits.
    pub kind: ChunkKind,
    /// Current state.
    pub state: String,
    /// Buffered bytes.
    pub buffer_len: usize,
    /// Transitions taken.
    pub transitions: u64,
}

/// The active turn, when there is one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnSnapshot {
    /// Turn identifier.
    pub id: String,
    /// Current state.
    pub state: String,
    /// Transitions taken.
    pub transitions: u64,
    /// The plan, once planning finished.
    pub plan: Option<TurnPlan>,
    /// Planned chunks and their live state.
    pub chunks: Vec<ChunkSnapshot>,
}

/// A full point-in-time view of a session and every region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// The conversation session identifier.
    pub session_id: String,
    /// Unix milliseconds.
    pub timestamp: i64,
    /// The session machine, including its regions and application data.
    pub session: SmSnapshot,
    /// Turns completed in the session.
    pub turns_completed: u32,
    /// Proactive items currently queued for the UI.
    pub proactive_queue_length: usize,
    /// The active turn and chunks, when present.
    pub turn: Option<TurnSnapshot>,
    /// Proactive region.
    pub proactive: RegionView,
    /// Voice region.
    pub voice: VoiceView,
    /// Memory region.
    pub memory: RegionView,
    /// Preference region and merged values.
    pub preferences: PreferencesView,
    /// Plugin region.
    pub plugins: RegionView,
    /// Improvement region.
    pub improvement: RegionView,
}

impl SessionSnapshot {
    /// Build a snapshot of `session`.
    pub fn capture(session: &Session) -> Self {
        let turn = session.active_turn().map(|turn| TurnSnapshot {
            id: turn.turn_id.to_string(),
            state: turn.state().to_owned(),
            transitions: turn.sm.transition_count(),
            plan: turn.plan.clone(),
            chunks: turn
                .chunks
                .iter()
                .map(|chunk| ChunkSnapshot {
                    id: chunk.chunk_id.to_string(),
                    kind: chunk.directive.kind,
                    state: chunk.state().to_owned(),
                    buffer_len: chunk.buffer().len(),
                    transitions: chunk.sm.transition_count(),
                })
                .collect(),
        });
        Self {
            session_id: session.id.to_string(),
            timestamp: now_ms(),
            session: session.sm.snapshot(),
            turns_completed: session.turns_completed,
            proactive_queue_length: session.proactive_items.len(),
            turn,
            proactive: RegionView::of(session, "proactive"),
            voice: VoiceView {
                state: session
                    .region_state("voice")
                    .unwrap_or("missing")
                    .to_owned(),
                tier: session.voice_tier().map(|tier| tier.as_str().to_owned()),
                barge_in_count: session.barge_in_count,
            },
            memory: RegionView::of(session, "memory"),
            preferences: PreferencesView {
                state: session
                    .region_state("preferences")
                    .unwrap_or("missing")
                    .to_owned(),
                values: session.context.preferences.values.clone(),
            },
            plugins: RegionView::of(session, "plugins"),
            improvement: RegionView::of(session, "improvement"),
        }
    }
}

#[derive(Debug, Default)]
struct MetricState {
    transitions: BTreeMap<(String, String, String, String), u64>,
    durations: BTreeMap<(String, String), u64>,
    state_started: BTreeMap<(String, String), Instant>,
    gauges: BTreeMap<(String, String, String), f64>,
}

/// The observability hub: transitions, metrics and snapshots.
#[derive(Debug)]
pub struct Observability {
    events: broadcast::Sender<SmTransitionEvent>,
    metrics: Mutex<MetricState>,
    snapshots: Mutex<BTreeMap<String, SessionSnapshot>>,
    started: Instant,
}

impl Default for Observability {
    fn default() -> Self {
        Self::new(256)
    }
}

impl Observability {
    /// Build a hub with a bounded transition broadcast channel.
    pub fn new(capacity: usize) -> Self {
        let (events, _) = broadcast::channel(capacity.max(1));
        Self {
            events,
            metrics: Mutex::new(MetricState::default()),
            snapshots: Mutex::new(BTreeMap::new()),
            started: Instant::now(),
        }
    }

    /// Subscribe to transitions as they happen.
    pub fn subscribe(&self) -> broadcast::Receiver<SmTransitionEvent> {
        self.events.subscribe()
    }

    /// Record one transition, update metrics and fan it out.
    pub fn record(&self, record: &TransitionRecord) {
        let event = SmTransitionEvent::from(record);
        self.record_event(event);
    }

    /// Record a whole dispatch.
    pub fn record_records(&self, records: &[TransitionRecord]) {
        for record in records {
            self.record(record);
        }
    }

    /// Record an already-built event.
    pub fn record_event(&self, event: SmTransitionEvent) {
        {
            let mut metrics = self.metrics.lock().expect("metrics lock");
            let key = (
                event.sm_type.clone(),
                event.from.clone(),
                event.to.clone(),
                event.event.clone(),
            );
            *metrics.transitions.entry(key).or_insert(0) += 1;
            let state_key = (event.sm_type.clone(), event.from.clone());
            if let Some(started) = metrics
                .state_started
                .insert(state_key.clone(), Instant::now())
            {
                let elapsed = started.elapsed().as_millis() as u64;
                metrics.durations.insert(state_key, elapsed);
            }
        }
        tracing::debug!(
            sm_id = %event.sm_id,
            sm_type = %event.sm_type,
            from = %event.from,
            to = %event.to,
            event = %event.event,
            depth = event.depth,
            "SMTransition"
        );
        let _ = self.events.send(event);
    }

    /// Capture a session snapshot and refresh the snapshot gauges.
    ///
    /// This is the handler behind `GET /api/cme/sessions/{id}/snapshot`.
    pub fn snapshot(&self, session: &Session) -> SessionSnapshot {
        let snapshot = SessionSnapshot::capture(session);
        self.observe_snapshot(&snapshot);
        snapshot
    }

    /// Publish a snapshot captured elsewhere (for example before the driver
    /// starts owning the session).
    pub fn publish_snapshot(&self, snapshot: SessionSnapshot) {
        self.snapshots
            .lock()
            .expect("snapshot lock")
            .insert(snapshot.session_id.clone(), snapshot);
    }

    /// The most recent snapshot for a session.
    pub fn latest_snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.snapshots
            .lock()
            .expect("snapshot lock")
            .get(session_id)
            .cloned()
    }

    /// Update gauges derived from a session snapshot.
    pub fn observe_session(&self, session: &Session) {
        self.observe_snapshot(&SessionSnapshot::capture(session));
    }

    fn observe_snapshot(&self, snapshot: &SessionSnapshot) {
        self.publish_snapshot(snapshot.clone());
        let mut metrics = self.metrics.lock().expect("metrics lock");
        metrics.gauges.retain(|(name, _, _), _| {
            !matches!(
                name.as_str(),
                "cme_sm_active_count"
                    | "cme_session_active"
                    | "cme_turn_chunks"
                    | "cme_preference_value"
            )
        });
        for name in ["proactive", "voice", "memory", "plugins", "improvement"] {
            metrics.gauges.insert(
                (
                    "cme_sm_active_count".to_owned(),
                    "sm_type".to_owned(),
                    name.to_owned(),
                ),
                1.0,
            );
        }
        metrics.gauges.insert(
            (
                "cme_session_active".to_owned(),
                "state".to_owned(),
                snapshot.session.state.to_string(),
            ),
            1.0,
        );
        metrics.gauges.insert(
            (
                "cme_preference_value".to_owned(),
                "samples".to_owned(),
                "total".to_owned(),
            ),
            snapshot.preferences.values.len() as f64,
        );
        if let Some(turn) = &snapshot.turn {
            metrics.gauges.insert(
                (
                    "cme_turn_chunks".to_owned(),
                    "state".to_owned(),
                    turn.state.clone(),
                ),
                turn.chunks.len() as f64,
            );
        }
    }

    /// Uptime in milliseconds.
    pub fn uptime_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Render the current metrics in Prometheus text format.
    pub fn prometheus(&self) -> String {
        let metrics = self.metrics.lock().expect("metrics lock");
        let mut out = String::new();
        out.push_str("# TYPE cme_sm_transitions_total counter\n");
        for ((sm_type, from, to, event), count) in &metrics.transitions {
            out.push_str(&format!(
                "cme_sm_transitions_total{{sm_type=\"{}\",from_state=\"{}\",to_state=\"{}\",event_type=\"{}\"}} {}\n",
                escape(sm_type),
                escape(from),
                escape(to),
                escape(event),
                count
            ));
        }
        out.push_str("# TYPE cme_sm_state_duration_ms gauge\n");
        for ((sm_type, state), duration) in &metrics.durations {
            out.push_str(&format!(
                "cme_sm_state_duration_ms{{sm_type=\"{}\",state=\"{}\"}} {}\n",
                escape(sm_type),
                escape(state),
                duration
            ));
        }
        out.push_str("# TYPE cme_sm_active_count gauge\n");
        for ((name, label, value), number) in &metrics.gauges {
            out.push_str(&format!(
                "{name}{{{label}=\"{}\"}} {number}\n",
                escape(value)
            ));
        }
        out.push_str("# TYPE cme_observability_uptime_ms gauge\n");
        out.push_str(&format!(
            "cme_observability_uptime_ms {}\n",
            self.started.elapsed().as_millis()
        ));
        out
    }
}

impl SmObserver for Observability {
    fn on_transition(&self, record: &TransitionRecord) {
        self.record(record);
    }
}

/// Attach an observer to a turn or chunk.
pub type ObserverHandle = Arc<dyn SmObserver>;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Session, SessionEvent};
    use mm_data::UserId;

    #[test]
    fn snapshots_include_regions_turn_and_chunks() {
        let mut session = Session::new(UserId::new("u1"), None).unwrap();
        session.dispatch(SessionEvent::InitComplete).unwrap();
        let before = SessionSnapshot::capture(&session);
        assert_eq!(before.session.machine, "session");
        assert_eq!(before.proactive.state, "idle");
        assert_eq!(before.memory.state, "episodic");
        assert_eq!(before.voice.barge_in_count, 0);
        assert!(before.turn.is_none());

        let turn = session.begin_turn("hello").unwrap();
        turn.plan_ready().unwrap();
        turn.route_selected(false, "stub").unwrap();
        turn.dispatch(crate::turn::TurnEvent::LlmStreamStart)
            .unwrap();
        turn.spawn_chunks().unwrap();
        let snapshot = SessionSnapshot::capture(&session);
        let turn = snapshot.turn.expect("turn");
        assert_eq!(turn.state, "streaming");
        assert!(!turn.chunks.is_empty());
        assert!(turn.plan.is_some());
    }

    #[test]
    fn transitions_are_streamed_and_metered() {
        let hub = Observability::new(16);
        let mut receiver = hub.subscribe();
        let mut session = Session::new(UserId::new("u1"), None).unwrap();
        let records = session.dispatch(SessionEvent::InitComplete).unwrap();
        hub.record_records(&records);
        let event = receiver.try_recv().expect("transition");
        assert_eq!(event.sm_type, "session");
        assert_eq!(event.from, "initializing");
        assert_eq!(event.to, "active");
        let metrics = hub.prometheus();
        assert!(metrics.contains("cme_sm_transitions_total"));
        assert!(metrics.contains("from_state=\"initializing\""));
    }

    #[test]
    fn observer_trait_records_turn_transitions() {
        let hub = Arc::new(Observability::new(16));
        let mut receiver = hub.subscribe();
        let mut session = Session::new(UserId::new("u1"), None).unwrap();
        session.dispatch(SessionEvent::InitComplete).unwrap();
        let turn = session.begin_turn("hello").unwrap();
        turn.observer = Some(hub.clone());
        turn.plan_ready().unwrap();
        let event = receiver.try_recv().expect("turn transition");
        assert_eq!(event.sm_type, "turn");
        assert_eq!(event.to, "routing");
    }
}
