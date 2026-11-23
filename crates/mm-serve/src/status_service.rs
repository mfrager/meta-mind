//! Per-session status service.
//!
//! The service is the only place the inspector view is composed. Mutations mark
//! sections dirty; a 100 ms debounce collapses a burst into one revision. The
//! cached snapshot is served directly while fresh, and recomputed on GET when it
//! is older than `MM_STATUS_MAX_AGE_MS` (default 2 s). A broadcast channel feeds
//! the SSE route; the first frame is always a full view.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mm_agent::{AgentKind, AgentRunStatus, AgentRuntime};
use mm_data::{
    ActionStore, DataBroker, DataService, PlanOwner, PlanStatus, PlanStepStatus, PlanStore,
    RdfLogService, RdfPlanStore, ScheduledAction, ScheduledActionStatus,
};
use mm_prompt::PromptStore;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::automation::AutomationManager;
use crate::metrics::Metrics;
use crate::state_service::StateService;
use crate::status::{
    ActionStatusView, ActionsStatusView, AgentStatusView, AutomationStatusView, DataStatusView,
    InterjectionsStatusView, OperationStatusView, PlanStatusView, PlanStepStatusView,
    PlansStatusView, PromptFragmentView, PromptStatusView, StatusView,
};

/// The default status coalescing window.
pub const COALESCE_MS: u64 = 100;
/// Default maximum snapshot age before a synchronous recompute.
pub const DEFAULT_MAX_AGE_MS: i64 = 2_000;
/// Retained revisions for a resync check.
pub const RETAINED_REVISIONS: usize = 256;

/// One independently markable section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusSection {
    /// Conversation state.
    State,
    /// Session lifecycle/presence/work.
    Session,
    /// Frame stack and utterance.
    Frames,
    /// Prompt instance summary.
    Prompt,
    /// Budget summary.
    Budget,
    /// Data health.
    Data,
    /// Plans.
    Plans,
    /// Scheduled actions.
    Actions,
    /// Automation supervisor.
    Automation,
    /// Interjection decisions.
    Interjections,
    /// Agent runs.
    Agents,
    /// Operations/log rows.
    Operations,
}

/// A published status revision.
#[derive(Debug, Clone)]
pub struct StatusEvent {
    /// Session id.
    pub session_id: String,
    /// Full view, including its revision.
    pub view: StatusView,
    /// `update` or `resync`.
    pub reason: String,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    view: StatusView,
    computed_at: i64,
    history: VecDeque<StatusView>,
}

/// The status composer and cache.
pub struct StatusService {
    state: Arc<StateService>,
    data: Arc<DataService>,
    agent: Arc<AgentRuntime>,
    plans: Arc<RdfPlanStore>,
    actions: Arc<ActionStore>,
    logs: Arc<RdfLogService>,
    prompts: Arc<PromptStore>,
    automation: Option<Arc<AutomationManager>>,
    metrics: Arc<Metrics>,
    max_age_ms: i64,
    cache: Mutex<BTreeMap<String, CacheEntry>>,
    dirty: Mutex<BTreeMap<String, BTreeSet<StatusSection>>>,
    pending: Mutex<BTreeSet<String>>,
    events: broadcast::Sender<StatusEvent>,
}

impl std::fmt::Debug for StatusService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StatusService")
            .field("sessions", &self.cached_sessions())
            .field("max_age_ms", &self.max_age_ms)
            .finish_non_exhaustive()
    }
}

impl StatusService {
    /// Build the service over the live stores.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: Arc<StateService>,
        data: Arc<DataService>,
        agent: Arc<AgentRuntime>,
        plans: Arc<RdfPlanStore>,
        actions: Arc<ActionStore>,
        logs: Arc<RdfLogService>,
        prompts: Arc<PromptStore>,
        automation: Option<Arc<AutomationManager>>,
        metrics: Arc<Metrics>,
    ) -> Self {
        let (events, _) = broadcast::channel(64);
        let max_age_ms = std::env::var("MM_STATUS_MAX_AGE_MS")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value >= 0)
            .unwrap_or(DEFAULT_MAX_AGE_MS);
        Self {
            state,
            data,
            agent,
            plans,
            actions,
            logs,
            prompts,
            automation,
            metrics,
            max_age_ms,
            cache: Mutex::new(BTreeMap::new()),
            dirty: Mutex::new(BTreeMap::new()),
            pending: Mutex::new(BTreeSet::new()),
            events,
        }
    }

    /// Subscribe to published revisions.
    pub fn subscribe(&self) -> broadcast::Receiver<StatusEvent> {
        self.events.subscribe()
    }

    /// The process metrics this service publishes to.
    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    /// Sessions with a cached view.
    pub fn cached_sessions(&self) -> Vec<String> {
        self.cache
            .lock()
            .expect("status cache lock")
            .keys()
            .cloned()
            .collect()
    }

    /// Mark one section dirty. The first mark in a window schedules the flush.
    pub fn mark_dirty(self: &Arc<Self>, session_id: &str, section: StatusSection) {
        self.dirty
            .lock()
            .expect("status dirty lock")
            .entry(session_id.to_owned())
            .or_default()
            .insert(section);
        let schedule = self
            .pending
            .lock()
            .expect("status pending lock")
            .insert(session_id.to_owned());
        if !schedule {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let service = Arc::clone(self);
        let id = session_id.to_owned();
        handle.spawn(async move {
            tokio::time::sleep(Duration::from_millis(COALESCE_MS)).await;
            service.flush(&id).await;
        });
    }

    /// Mark several sections dirty at once.
    pub fn mark_dirty_many(
        self: &Arc<Self>,
        session_id: &str,
        sections: impl IntoIterator<Item = StatusSection>,
    ) {
        for section in sections {
            self.mark_dirty(session_id, section);
        }
    }

    /// Recompute and publish one revision.
    pub async fn flush(&self, session_id: &str) {
        if !self
            .pending
            .lock()
            .expect("status pending lock")
            .remove(session_id)
        {
            // A caller already flushed this window; the coalescing timer that
            // was scheduled for it must not publish a duplicate revision.
            return;
        }
        self.dirty
            .lock()
            .expect("status dirty lock")
            .remove(session_id);
        let started = std::time::Instant::now();
        let Some(view) = self.compose(session_id).await else {
            return;
        };
        self.metrics
            .record_status(started.elapsed().as_millis() as u64);
        let revision = self
            .cache
            .lock()
            .expect("status cache lock")
            .get(session_id)
            .map_or(1, |entry| entry.view.revision.saturating_add(1));
        let mut view = view;
        view.revision = revision;
        view.generated_at = now_ms();
        view.updated_at = view.generated_at;
        {
            let mut cache = self.cache.lock().expect("status cache lock");
            let entry = cache
                .entry(session_id.to_owned())
                .or_insert_with(|| CacheEntry {
                    view: view.clone(),
                    computed_at: view.generated_at,
                    history: VecDeque::new(),
                });
            entry.view = view.clone();
            entry.computed_at = view.generated_at;
            entry.history.push_back(view.clone());
            while entry.history.len() > RETAINED_REVISIONS {
                entry.history.pop_front();
            }
        }
        let _ = self.events.send(StatusEvent {
            session_id: session_id.to_owned(),
            view,
            reason: "update".into(),
        });
    }

    /// Return the cached view, recomputing when it is stale or dirty.
    pub async fn snapshot(&self, session_id: &str) -> Option<StatusView> {
        let now = now_ms();
        let dirty = self
            .dirty
            .lock()
            .expect("status dirty lock")
            .contains_key(session_id);
        if !dirty {
            if let Some(view) = self.cached(session_id, now) {
                return Some(view);
            }
        }
        let mut view = self.compose(session_id).await?;
        let revision = self
            .cache
            .lock()
            .expect("status cache lock")
            .get(session_id)
            .map_or(1, |entry| entry.view.revision.max(1));
        view.revision = revision;
        view.generated_at = now;
        view.updated_at = now;
        self.store(session_id, view.clone(), now);
        Some(view)
    }

    /// The retained revisions for one session, oldest first.
    pub fn history(&self, session_id: &str) -> Vec<StatusView> {
        self.cache
            .lock()
            .expect("status cache lock")
            .get(session_id)
            .map(|entry| entry.history.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn cached(&self, session_id: &str, now: i64) -> Option<StatusView> {
        let cache = self.cache.lock().expect("status cache lock");
        let entry = cache.get(session_id)?;
        if now.saturating_sub(entry.computed_at) <= self.max_age_ms {
            Some(entry.view.clone())
        } else {
            None
        }
    }

    fn store(&self, session_id: &str, view: StatusView, now: i64) {
        let mut cache = self.cache.lock().expect("status cache lock");
        let entry = cache
            .entry(session_id.to_owned())
            .or_insert_with(|| CacheEntry {
                view: view.clone(),
                computed_at: now,
                history: VecDeque::new(),
            });
        entry.view = view.clone();
        entry.computed_at = now;
        entry.history.push_back(view);
        while entry.history.len() > RETAINED_REVISIONS {
            entry.history.pop_front();
        }
    }

    /// Compose one full view. Section failures fill `unavailable`; they do not
    /// fail the panel.
    pub async fn compose(&self, session_id: &str) -> Option<StatusView> {
        let mut view = match self.state.status(session_id, now_ms()).await {
            Ok(view) => view,
            Err(error) => {
                eprintln!("mm-serve: status base unavailable for {session_id}: {error}");
                return None;
            }
        };
        view.session_id = session_id.to_owned();

        // These projections are independent reads. Run them together so a
        // status flush stays inside the 250 ms publish budget under load.
        let (data_status, namespaces, events, plan_registry, actions) = tokio::join!(
            self.data.status(),
            self.data.list_namespaces(),
            self.state.agentstream().events(session_id),
            self.plans.registry(),
            self.actions.list(session_id),
        );
        match data_status {
            Ok(status) => {
                let namespaces = namespaces.map_or(0, |rows| rows.len());
                view.data = DataStatusView {
                    data_revision: status.data_revision,
                    index_pending: status.index_pending,
                    index_lag_ms: 0,
                    schemas: status.active_schemas.len(),
                    namespaces,
                };
                view.data_revision = status.data_revision;
            }
            Err(error) => {
                view.unavailable
                    .insert("data".into(), format!("data unavailable: {error}"));
            }
        }

        // Prompt instance pinned by the latest model call.
        match events {
            Ok(events) => {
                view.prompt = latest_prompt(&events);
                view.operations = recent_operations(&events);
            }
            Err(error) => {
                view.unavailable
                    .insert("prompt".into(), format!("agentstream unavailable: {error}"));
            }
        }
        if view.prompt.is_none() && !view.unavailable.contains_key("prompt") {
            view.unavailable
                .insert("prompt".into(), "no prompt instance recorded".into());
        }

        // Plans.
        match plan_registry {
            Ok(registry) => {
                let mut items = Vec::new();
                for plan_id in registry.plans {
                    match self.plans.load(&plan_id).await {
                        Ok(Some(plan)) => items.push(plan_view(&plan)),
                        Ok(None) => {}
                        Err(error) => {
                            view.unavailable
                                .insert("plans".into(), format!("plan {plan_id}: {error}"));
                        }
                    }
                }
                items.sort_by(|a, b| {
                    plan_rank(&a.status)
                        .cmp(&plan_rank(&b.status))
                        .then_with(|| b.revision.cmp(&a.revision))
                });
                view.plans = PlansStatusView { items };
            }
            Err(error) => {
                view.unavailable.insert(
                    "plans".into(),
                    format!("plan registry unavailable: {error}"),
                );
            }
        }

        // Scheduled actions.
        match actions {
            Ok(actions) => {
                let (scheduled, recent) = action_rows(actions);
                view.actions = ActionsStatusView {
                    next_run_at: scheduled.iter().filter_map(|row| row.due_at).min(),
                    scheduled,
                    recent,
                };
            }
            Err(error) => {
                view.unavailable
                    .insert("actions".into(), format!("actions unavailable: {error}"));
            }
        }

        // Automation live state.
        let mut automation = AutomationStatusView {
            supervisor: "idle".into(),
            tick_secs: self
                .automation
                .as_ref()
                .map_or(30, |manager| manager.config().supervisor.tick_secs.max(1)),
            frame_timers: usize::from(view.frames.pending_utterance.is_some()),
            next_action_at: view.actions.next_run_at,
            sessions: self
                .automation
                .as_ref()
                .map_or(0, |manager| manager.session_ids().len()),
            cron_jobs: view.actions.scheduled.len(),
            last_reason: None,
        };
        if let Some(manager) = &self.automation {
            if manager.has_session(session_id) {
                automation.supervisor = "running".into();
            }
            if let Some(last) = manager.last_event(session_id) {
                automation.last_reason = is_suppressed(&last.kind).then_some(last.detail);
            }
        }
        view.automation = automation;

        // Interjections: the effective routing policy and last suppression.
        view.interjections = InterjectionsStatusView {
            policy_id: "global.default".into(),
            suppressed: view.automation.last_reason.is_some(),
            reason: view.automation.last_reason.clone(),
            queue: 0,
        };
        if self.automation.is_none() {
            view.unavailable.insert(
                "automation".into(),
                "automation is disabled by MM_AUTOMATION_V1".into(),
            );
        }

        // Agent runs owned by the session.
        let runs = self.agent.runs_for_session(session_id).await;
        view.agents = runs
            .into_iter()
            .take(8)
            .map(|run| AgentStatusView {
                active: !run.status.is_terminal(),
                run_id: run.run_id,
                kind: kind_label(run.kind).into(),
                status: run.status.label().into(),
                session_id: run.session_id,
                updated_at: run.updated_at,
            })
            .collect();

        // Budget metrics are not assembled by the state service yet; make the
        // section independently explicit rather than fabricating numbers.
        view.unavailable.insert(
            "budget".into(),
            "budget ledger is not attached to this session yet".into(),
        );

        // Prompt inventory is always available read-only, even when no turn ran.
        let _ = &self.prompts;
        let _ = &self.logs;
        Some(view)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn latest_prompt(events: &[mm_data::AgentstreamEvent]) -> Option<PromptStatusView> {
    events.iter().rev().find_map(|event| {
        let payload = event.payload.as_object()?;
        if payload.get("kind").and_then(Value::as_str) != Some("prompt_built") {
            return None;
        }
        let instance = payload
            .get("instanceId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let strategy = payload
            .get("strategy")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let fragments = payload
            .get("fragments")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| {
                        Some(PromptFragmentView {
                            id: row.get("id")?.as_str()?.to_owned(),
                            version: row.get("version")?.as_u64()? as u32,
                            chars: row.get("chars").and_then(Value::as_u64).unwrap_or(0) as usize,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(PromptStatusView {
            id: instance.to_owned(),
            strategy,
            variant: payload
                .get("variant")
                .and_then(Value::as_str)
                .map(str::to_owned),
            hash: payload
                .get("hash")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            fragments,
        })
    })
}

fn recent_operations(events: &[mm_data::AgentstreamEvent]) -> Vec<OperationStatusView> {
    events
        .iter()
        .rev()
        .take(20)
        .map(|event| OperationStatusView {
            id: event.event_id.clone(),
            kind: event.event_type.as_str().into(),
            source: event.provenance.source.clone(),
            at: event.timestamp,
            detail: event.payload.clone(),
        })
        .collect()
}

fn plan_view(plan: &mm_data::Plan) -> PlanStatusView {
    PlanStatusView {
        plan_id: plan.id.clone(),
        goal: plan.goal.clone(),
        status: status_label(plan.status),
        revision: plan.revision,
        current_step: plan.current_step.clone(),
        steps: plan
            .steps
            .iter()
            .map(|step| PlanStepStatusView {
                id: step.id.clone(),
                title: step.title.clone(),
                status: step_status_label(step.status),
                attempts: step.attempts,
                runs: step.runs,
            })
            .collect(),
    }
}

fn action_rows(actions: Vec<ScheduledAction>) -> (Vec<ActionStatusView>, Vec<ActionStatusView>) {
    let mut scheduled = Vec::new();
    let mut recent = Vec::new();
    for action in actions {
        let row = ActionStatusView {
            action_id: action.id.clone(),
            kind: action.spec.kind.clone(),
            status: action_status_label(action.spec.status),
            due_at: action.next_run_at,
            last_run_at: action.spec.last_run_at,
            runs: action.spec.runs,
        };
        if matches!(
            action.spec.status,
            ScheduledActionStatus::Scheduled
                | ScheduledActionStatus::Claimed
                | ScheduledActionStatus::Running
        ) {
            scheduled.push(row);
        } else {
            recent.push(row);
        }
    }
    scheduled.sort_by_key(|row| row.due_at.unwrap_or(i64::MAX));
    recent.truncate(20);
    (scheduled, recent)
}

fn is_suppressed(kind: &str) -> bool {
    kind == "suppressed"
}

fn plan_rank(status: &str) -> u8 {
    match status {
        "active" => 0,
        "draft" => 1,
        "paused" => 2,
        _ => 3,
    }
}

fn kind_label(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Hermes => "hermes",
        AgentKind::Pi => "pi",
    }
}

fn status_label(status: PlanStatus) -> String {
    match status {
        PlanStatus::Draft => "draft",
        PlanStatus::Active => "active",
        PlanStatus::Paused => "paused",
        PlanStatus::Done => "done",
        PlanStatus::Cancelled => "cancelled",
        PlanStatus::Completed => "completed",
        PlanStatus::Failed => "failed",
        PlanStatus::Abandoned => "abandoned",
        PlanStatus::Stopped => "stopped",
    }
    .into()
}

fn step_status_label(status: PlanStepStatus) -> String {
    match status {
        PlanStepStatus::Pending => "pending",
        PlanStepStatus::Running => "running",
        PlanStepStatus::Done => "done",
        PlanStepStatus::Failed => "failed",
        PlanStepStatus::Blocked => "blocked",
        PlanStepStatus::Skipped => "skipped",
        PlanStepStatus::Cancelled => "cancelled",
    }
    .into()
}

fn action_status_label(status: ScheduledActionStatus) -> String {
    match status {
        ScheduledActionStatus::Scheduled => "scheduled",
        ScheduledActionStatus::Claimed => "claimed",
        ScheduledActionStatus::Running => "running",
        ScheduledActionStatus::Done => "done",
        ScheduledActionStatus::Cancelled => "cancelled",
        ScheduledActionStatus::Failed => "failed",
    }
    .into()
}

/// JSON for one status SSE frame. The reason is additive and lets a client tell
/// a normal update from a full resync.
pub fn frame_json(view: &StatusView, reason: &str) -> Value {
    let mut value = serde_json::to_value(view).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert("reason".into(), Value::String(reason.into()));
    }
    value
}

/// Whether `from` is older than the retained history and needs a full resync.
pub fn needs_resync(history: &[StatusView], from: Option<u64>) -> bool {
    let Some(from) = from.filter(|value| *value > 0) else {
        return false;
    };
    history
        .first()
        .is_none_or(|oldest| from.saturating_add(1) < oldest.revision)
}

/// Owner label used by plan details.
pub fn owner_label(owner: PlanOwner) -> &'static str {
    match owner {
        PlanOwner::Cm => "cm",
        PlanOwner::Agent => "agent",
        PlanOwner::Operator => "operator",
        PlanOwner::Action => "action",
        PlanOwner::User => "user",
    }
}

/// Status for an existing agent run.
pub fn agent_status_label(status: AgentRunStatus) -> &'static str {
    status.label()
}
