//! Per-session automation: one supervisor, generic due actions, and frame
//! utterance timing.
//!
//! There is no per-kind timer or branch. Every action is an ordinary
//! `mm:ScheduledAction` row; the supervisor claims due rows, asks the one
//! interjection router, executes through an injected bounded executor, and
//! records the result. Frame utterances use the durable pending utterance on
//! the session, not a second timer store.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mm_cme::FrameEngine;
use mm_data::{
    ActionStore, FrameUtterance, InternalActionRegistry, LogFilter, LogLevel, RdfLogService,
    ScheduledAction, ScheduledActionSpec, ScheduledActionStatus, SessionBundle, StateOrigin,
    StatusValue,
};
use mm_llm::{LlmClient, LlmRequest, Message, Purpose};
use mm_prompt::PromptStore;
use serde::Deserialize;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use crate::interjection::{Delivery, InterjectionRequest, InterjectionRouter, InterjectionSource};
use crate::state_service::StateService;

/// The built-in kind is one reviewed spec, not a special branch.
const BUILTIN_KIND: &str = "joke";
const BUILTIN_CRON: &str = "*/5 * * * *";

/// Automation defaults from `config/automation.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AutomationConfig {
    /// Supervisor cadence and enablement.
    pub supervisor: SupervisorConfig,
    /// Action scheduling defaults.
    pub actions: ActionsConfig,
    /// Frame timing defaults.
    pub frames: FramesConfig,
}

/// Supervisor configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SupervisorConfig {
    /// Enable the supervisor.
    pub enabled: bool,
    /// Idle tick in seconds.
    pub tick_secs: u64,
    /// Maximum concurrent sessions.
    pub max_sessions: usize,
    /// Shutdown grace.
    pub grace_ms: u64,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            tick_secs: 1,
            max_sessions: 32,
            grace_ms: 5_000,
        }
    }
}

/// Action configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ActionsConfig {
    /// Enable action execution.
    pub enabled: bool,
}

impl Default for ActionsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Frame timing configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FramesConfig {
    /// Enable frame utterances.
    pub enabled: bool,
    /// Default delay when a template does not supply one.
    pub default_delay_ms: u64,
    /// Goodbye hold before ending.
    pub goodbye_hold_ms: u64,
}

impl Default for FramesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_delay_ms: 4_500,
            goodbye_hold_ms: 3_000,
        }
    }
}

impl AutomationConfig {
    /// Load from the repository config, falling back to safe defaults.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Kinds disabled through `MM_INTERNAL_ACTIONS_DISABLED`.
    pub fn disabled_kinds() -> BTreeSet<String> {
        std::env::var("MM_INTERNAL_ACTIONS_DISABLED")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// One automation output or suppression, suitable for status/SSE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationEvent {
    /// Session.
    pub session_id: String,
    /// `action`, `frame`, `suppressed`, or `stopped`.
    pub kind: String,
    /// Operator-facing text, empty when suppressed.
    pub text: String,
    /// Stable reason.
    pub detail: String,
    /// Decision instant.
    pub at: i64,
}

/// Executes one due action after policy approval.
#[async_trait]
pub trait ActionExecutor: Send + Sync {
    /// Run one action; the manager logs and advances its schedule.
    async fn execute(&self, action: &ScheduledAction) -> Result<String, String>;
}

/// Executes one frame utterance after policy approval.
#[async_trait]
pub trait FrameExecutor: Send + Sync {
    /// Render and run the frame's prompt strategy.
    async fn execute(
        &self,
        session_id: &str,
        frame: &FrameUtterance,
        bundle: &SessionBundle,
    ) -> Result<String, String>;
}

/// Prompt-backed built-in action executor. Reads the last five outputs for
/// originality, renders the registered strategy, and asks the configured model.
pub struct PromptActionExecutor {
    state: Arc<StateService>,
    logs: Arc<RdfLogService>,
    prompts: Arc<PromptStore>,
    registry: Arc<InternalActionRegistry>,
    client: Option<Arc<dyn LlmClient>>,
    model: Option<String>,
}

impl PromptActionExecutor {
    /// Build the executor.
    pub fn new(
        state: Arc<StateService>,
        logs: Arc<RdfLogService>,
        prompts: Arc<PromptStore>,
        registry: Arc<InternalActionRegistry>,
        client: Option<Arc<dyn LlmClient>>,
        model: Option<String>,
    ) -> Self {
        Self {
            state,
            logs,
            prompts,
            registry,
            client,
            model,
        }
    }
}

#[async_trait]
impl ActionExecutor for PromptActionExecutor {
    async fn execute(&self, action: &ScheduledAction) -> Result<String, String> {
        let bundle = self
            .state
            .load(&action.spec.session_id)
            .await
            .map_err(|error| error.to_string())?;
        let recent = self
            .logs
            .latest(
                &action.spec.session_id,
                &LogFilter {
                    kinds: vec![action.spec.kind.clone()],
                    ..LogFilter::default()
                },
                5,
            )
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|entry| entry.message)
            .collect::<Vec<_>>()
            .join(" | ");
        let action_spec = self
            .registry
            .require(&action.spec.kind)
            .map_err(|error| error.to_string())?;
        let strategy_id = action_spec
            .prompt_strategy
            .clone()
            .ok_or_else(|| format!("kind {} has no prompt strategy", action.spec.kind))?;
        let strategy = self
            .prompts
            .strategy(&strategy_id)
            .ok_or_else(|| format!("missing prompt strategy {strategy_id}"))?;
        let mut variables: BTreeMap<String, String> = strategy
            .slots
            .iter()
            .map(|slot| (slot.clone(), String::new()))
            .collect();
        variables.insert("being_name".into(), "Metamind".into());
        variables.insert("user_name".into(), bundle.session.user_id.clone());
        variables.insert(
            "workspace_label".into(),
            format!("workspace/{}", action.spec.session_id),
        );
        variables.insert("action_kind".into(), action.spec.kind.clone());
        variables.insert(
            "action_input".into(),
            action.spec.input.clone().unwrap_or_default(),
        );
        variables.insert("recent_log".into(), recent);
        variables.insert("tone".into(), "warm, direct, honest".into());
        let prompt = self
            .prompts
            .render(&strategy_id, &variables)
            .map_err(|error| error.to_string())?;
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "no model is configured for internal actions".to_owned())?;
        let mut request = LlmRequest::new(
            Purpose::Converse,
            vec![Message::system(prompt.text), Message::user("Say it now.")],
        );
        if let Some(model) = &self.model {
            request = request.with_model(model.clone());
        }
        let response = client
            .complete(&request)
            .await
            .map_err(|error| error.to_string())?;
        let text = response.text.trim().to_owned();
        if text.is_empty() {
            return Err("the action model returned no text".into());
        }
        Ok(text)
    }
}

/// Prompt-backed frame executor.
pub struct PromptFrameExecutor {
    engine: FrameEngine,
    client: Option<Arc<dyn LlmClient>>,
    model: Option<String>,
}

impl PromptFrameExecutor {
    /// Build the executor.
    pub fn new(
        engine: FrameEngine,
        client: Option<Arc<dyn LlmClient>>,
        model: Option<String>,
    ) -> Self {
        Self {
            engine,
            client,
            model,
        }
    }
}

#[async_trait]
impl FrameExecutor for PromptFrameExecutor {
    async fn execute(
        &self,
        _session_id: &str,
        frame: &FrameUtterance,
        bundle: &SessionBundle,
    ) -> Result<String, String> {
        let template_id = bundle
            .frames
            .top()
            .map(|top| top.template_id.clone())
            .ok_or_else(|| "frame stack is empty".to_owned())?;
        let template = self
            .engine
            .template(&template_id)
            .ok_or_else(|| format!("unknown frame template {template_id}"))?;
        let mut variables = BTreeMap::new();
        variables.insert("being_name".into(), "Metamind".into());
        variables.insert("being_version".into(), "1".into());
        variables.insert("being_values".into(), "honesty, continuity, care".into());
        variables.insert("frame_kind".into(), template.kind.clone());
        variables.insert("frame_step".into(), template.step.clone());
        variables.insert("frame_attempt".into(), frame.attempt.to_string());
        variables.insert("budget_class".into(), "frame".into());
        variables.insert("budget_rounds".into(), "2".into());
        variables.insert("budget_tokens".into(), "8000".into());
        variables.insert("budget_wall_ms".into(), "45000".into());
        variables.insert("tone".into(), "warm, direct, honest".into());
        let prompt = self
            .engine
            .prompts()
            .render("frame.utterance.v1", &variables)
            .map_err(|error| error.to_string())?;
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "no model is configured for frame utterances".to_owned())?;
        let mut request = LlmRequest::new(
            Purpose::Converse,
            vec![Message::system(prompt.text), Message::user("Say it now.")],
        );
        if let Some(model) = &self.model {
            request = request.with_model(model.clone());
        }
        let response = client
            .complete(&request)
            .await
            .map_err(|error| error.to_string())?;
        let text = response.text.trim().to_owned();
        if text.is_empty() {
            return Err("the frame model returned no text".into());
        }
        Ok(text)
    }
}

/// Shared automation dependencies.
pub struct AutomationDeps {
    /// Conversation state and durable frame timers.
    pub state: Arc<StateService>,
    /// Generic scheduled-action store.
    pub actions: Arc<ActionStore>,
    /// RDF log for action/frame outputs.
    pub logs: Arc<RdfLogService>,
    /// Frame templates and timing.
    pub frames: FrameEngine,
    /// Action executor.
    pub action_executor: Arc<dyn ActionExecutor>,
    /// Frame executor.
    pub frame_executor: Arc<dyn FrameExecutor>,
    /// The single interjection gate.
    pub interjections: Arc<InterjectionRouter>,
    /// Config.
    pub config: AutomationConfig,
    /// Disabled internal-action kinds.
    pub disabled_kinds: BTreeSet<String>,
}

/// One live supervisor.
struct SessionHandle {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

/// The per-process automation manager.
pub struct AutomationManager {
    deps: Arc<AutomationDeps>,
    sessions: Mutex<HashMap<String, SessionHandle>>,
    last_events: Mutex<HashMap<String, AutomationEvent>>,
    events: broadcast::Sender<AutomationEvent>,
}

impl std::fmt::Debug for AutomationManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AutomationManager")
            .field("sessions", &self.session_ids())
            .finish_non_exhaustive()
    }
}

impl AutomationManager {
    /// Build the manager.
    pub fn new(deps: AutomationDeps) -> Self {
        let (events, _) = broadcast::channel(128);
        Self {
            deps: Arc::new(deps),
            sessions: Mutex::new(HashMap::new()),
            last_events: Mutex::new(HashMap::new()),
            events,
        }
    }

    /// The loaded automation configuration.
    pub fn config(&self) -> &AutomationConfig {
        &self.deps.config
    }

    /// The scheduled-action store shared with the supervisor.
    pub fn actions(&self) -> &Arc<ActionStore> {
        &self.deps.actions
    }

    /// Whether a live supervisor exists for the session.
    pub fn has_session(&self, session_id: &str) -> bool {
        self.sessions
            .lock()
            .expect("automation sessions lock")
            .contains_key(session_id)
    }

    /// The most recent action/frame decision for a session.
    pub fn last_event(&self, session_id: &str) -> Option<AutomationEvent> {
        self.last_events
            .lock()
            .expect("automation events lock")
            .get(session_id)
            .cloned()
    }

    /// Subscribe to approved/suppressed automation events.
    pub fn subscribe(&self) -> broadcast::Receiver<AutomationEvent> {
        self.events.subscribe()
    }

    /// Live session ids, sorted.
    pub fn session_ids(&self) -> Vec<String> {
        let mut ids = self
            .sessions
            .lock()
            .expect("automation sessions lock")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    /// Ensure exactly one supervisor. Returns `false` when one already existed.
    pub async fn ensure(self: &Arc<Self>, session_id: &str) -> bool {
        if !self.deps.config.supervisor.enabled {
            return false;
        }
        let mut sessions = self.sessions.lock().expect("automation sessions lock");
        if sessions.contains_key(session_id) {
            return false;
        }
        if sessions.len() >= self.deps.config.supervisor.max_sessions {
            return false;
        }
        let (stop, mut stopped) = watch::channel(false);
        let manager = Arc::clone(self);
        let id = session_id.to_owned();
        let task = tokio::spawn(async move {
            if let Err(error) = manager.seed(&id, mm_data::rdf::now_ms()).await {
                eprintln!("mm-serve: automation seed failed for {id}: {error}");
            }
            // Frame timers are real-time; one second also bounds action claim
            // latency. `supervisor.tick_secs` is the proactive policy cadence,
            // not the frame timer's resolution.
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        match manager.tick(&id, mm_data::rdf::now_ms()).await {
                            Ok(events) if events.iter().any(|event| event.kind == "stopped") => break,
                            Ok(_) => {}
                            Err(error) => {
                                eprintln!("mm-serve: automation tick failed for {id}: {error}");
                            }
                        }
                    }
                    changed = stopped.changed() => {
                        if changed.is_err() || *stopped.borrow() {
                            break;
                        }
                    }
                }
            }
            manager
                .sessions
                .lock()
                .expect("automation sessions lock")
                .remove(&id);
        });
        sessions.insert(session_id.to_owned(), SessionHandle { stop, task });
        true
    }

    /// Stop one supervisor.
    pub fn stop(&self, session_id: &str) -> bool {
        let Some(handle) = self
            .sessions
            .lock()
            .expect("automation sessions lock")
            .remove(session_id)
        else {
            return false;
        };
        let _ = handle.stop.send(true);
        handle.task.abort();
        self.last_events
            .lock()
            .expect("automation events lock")
            .remove(session_id);
        true
    }

    /// Stop every supervisor.
    pub fn stop_all(&self) {
        let handles = self
            .sessions
            .lock()
            .expect("automation sessions lock")
            .drain()
            .map(|(_, handle)| handle)
            .collect::<Vec<_>>();
        for handle in handles {
            let _ = handle.stop.send(true);
            handle.task.abort();
        }
        self.last_events
            .lock()
            .expect("automation events lock")
            .clear();
    }

    /// Seed the built-in action as an ordinary idempotent row.
    pub async fn seed(&self, session_id: &str, now: i64) -> Result<(), String> {
        self.deps
            .actions
            .schedule(
                ScheduledActionSpec {
                    session_id: session_id.to_owned(),
                    kind: BUILTIN_KIND.to_owned(),
                    input: Some("a short, original aside".into()),
                    parameters: Default::default(),
                    schedule: mm_data::ScheduleSpec::Cron {
                        expression: BUILTIN_CRON.into(),
                        timezone: Some("UTC".into()),
                    },
                    idempotency_key: format!("automation:{BUILTIN_KIND}:{BUILTIN_CRON}"),
                    status: ScheduledActionStatus::Scheduled,
                    created_at: now,
                    last_run_at: None,
                    runs: 0,
                },
                mm_data::Recurrence::Cron,
                None,
                now,
            )
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())?;
        let bundle = self
            .deps
            .state
            .load(session_id)
            .await
            .map_err(|error| error.to_string())?;
        if bundle.session.pending_utterance.is_none() && bundle.frames.top().is_some() {
            self.deps
                .state
                .schedule_utterance(session_id, None, "intro", bundle.frames.revision, now)
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// One supervisor tick. Public for deterministic tests and operator probes.
    pub async fn tick(&self, session_id: &str, now: i64) -> Result<Vec<AutomationEvent>, String> {
        let bundle = self
            .deps
            .state
            .load(session_id)
            .await
            .map_err(|error| error.to_string())?;
        if bundle.session.lifecycle != mm_data::SessionLifecycle::Active {
            return Ok(vec![AutomationEvent {
                session_id: session_id.into(),
                kind: "stopped".into(),
                text: String::new(),
                detail: format!("session is {}", bundle.session.lifecycle.as_str()),
                at: now,
            }]);
        }
        let mut events = Vec::new();
        if self.deps.config.actions.enabled {
            events.extend(self.tick_actions(session_id, &bundle, now).await?);
        }
        if self.deps.config.frames.enabled {
            events.extend(self.tick_frame(session_id, &bundle, now).await?);
        }
        if let Some(last) = events.last() {
            self.last_events
                .lock()
                .expect("automation events lock")
                .insert(session_id.to_owned(), last.clone());
        }
        for event in &events {
            let _ = self.events.send(event.clone());
        }
        Ok(events)
    }

    async fn tick_actions(
        &self,
        session_id: &str,
        bundle: &SessionBundle,
        now: i64,
    ) -> Result<Vec<AutomationEvent>, String> {
        let working = bundle.session.work.working();
        let frame = bundle.frames.top().map(|frame| frame.kind);
        let actions = self
            .deps
            .actions
            .claim_due_for_session(session_id, now)
            .await
            .map_err(|error| error.to_string())?;
        let mut events = Vec::new();
        for action in actions {
            let spec = match self.deps.actions.registry().require(&action.spec.kind) {
                Ok(spec) => spec,
                Err(error) => {
                    let _ = self.deps.actions.fail_run(&action.id, now).await;
                    events.push(AutomationEvent {
                        session_id: session_id.into(),
                        kind: "suppressed".into(),
                        text: String::new(),
                        detail: error.to_string(),
                        at: now,
                    });
                    continue;
                }
            };
            if self.deps.disabled_kinds.contains(&spec.kind) {
                let _ = self.deps.actions.finish_run(&action.id, now).await;
                events.push(AutomationEvent {
                    session_id: session_id.into(),
                    kind: "suppressed".into(),
                    text: String::new(),
                    detail: format!("kind {} is disabled", spec.kind),
                    at: now,
                });
                continue;
            }
            let request = InterjectionRequest {
                id: action.id.clone(),
                session_id: session_id.into(),
                source: InterjectionSource::InternalAction,
                kind_policy: Some(spec.interjection_policy.clone()),
                user_policy: None,
                instance_policy: None,
                frame,
                working,
                presence: bundle.session.presence,
                at: now,
                expires_at: now.saturating_add(900_000),
            };
            let decision = self
                .deps
                .interjections
                .route(&request)
                .map_err(|error| error.to_string())?;
            if decision.decision != Delivery::Deliver {
                let _ = self.deps.actions.finish_run(&action.id, now).await;
                events.push(AutomationEvent {
                    session_id: session_id.into(),
                    kind: "suppressed".into(),
                    text: String::new(),
                    detail: format!("{} ({})", decision.reason, decision.policy_id),
                    at: now,
                });
                continue;
            }
            match self.deps.action_executor.execute(&action).await {
                Ok(text) => {
                    let _ = self.deps.actions.finish_run(&action.id, now).await;
                    self.deps
                        .logs
                        .append(
                            session_id,
                            None,
                            LogLevel::Info,
                            &action.spec.kind,
                            &text,
                            vec!["automation".into()],
                            vec![action.id.clone()],
                            "action",
                        )
                        .await
                        .ok();
                    events.push(AutomationEvent {
                        session_id: session_id.into(),
                        kind: "action".into(),
                        text,
                        detail: format!("{} delivered", action.spec.kind),
                        at: now,
                    });
                }
                Err(error) => {
                    let _ = self.deps.actions.fail_run(&action.id, now).await;
                    events.push(AutomationEvent {
                        session_id: session_id.into(),
                        kind: "suppressed".into(),
                        text: String::new(),
                        detail: error,
                        at: now,
                    });
                }
            }
        }
        Ok(events)
    }

    async fn tick_frame(
        &self,
        session_id: &str,
        bundle: &SessionBundle,
        now: i64,
    ) -> Result<Vec<AutomationEvent>, String> {
        let Some(pending) = &bundle.session.pending_utterance else {
            return Ok(Vec::new());
        };
        if pending.due_at > now {
            return Ok(Vec::new());
        }
        if !matches!(
            bundle.conversation.status.get("awaiting_user"),
            Some(StatusValue::Bool(true))
        ) {
            return Ok(Vec::new());
        }
        let working = bundle.session.work.working();
        let frame = bundle.frames.top().map(|frame| frame.kind);
        if working || frame.is_none() {
            return Ok(vec![AutomationEvent {
                session_id: session_id.into(),
                kind: "suppressed".into(),
                text: String::new(),
                detail: "frame utterance while working".into(),
                at: now,
            }]);
        }
        let request = InterjectionRequest {
            id: pending.id.clone(),
            session_id: session_id.into(),
            source: InterjectionSource::FrameUtterance,
            kind_policy: None,
            user_policy: None,
            instance_policy: None,
            frame,
            working,
            presence: bundle.session.presence,
            at: now,
            expires_at: now.saturating_add(900_000),
        };
        let decision = self
            .deps
            .interjections
            .route(&request)
            .map_err(|error| error.to_string())?;
        if decision.decision != Delivery::Deliver {
            return Ok(vec![AutomationEvent {
                session_id: session_id.into(),
                kind: "suppressed".into(),
                text: String::new(),
                detail: format!("{} ({})", decision.reason, decision.policy_id),
                at: now,
            }]);
        }
        let template = bundle
            .frames
            .top()
            .and_then(|frame| self.deps.frames.template(&frame.template_id));
        let text = self
            .deps
            .frame_executor
            .execute(session_id, pending, bundle)
            .await
            .map_err(|error| format!("frame utterance: {error}"))?;
        self.deps
            .state
            .cancel_utterance(session_id, bundle.conversation.revision, now)
            .await
            .map_err(|error| error.to_string())?;
        self.deps
            .logs
            .append(
                session_id,
                None,
                LogLevel::Info,
                "frame",
                &text,
                vec!["automation".into()],
                vec![pending.id.clone()],
                "frame",
            )
            .await
            .ok();
        if let Some(template) = template {
            if template.kind == "goodbye" {
                let state = Arc::clone(&self.deps.state);
                let session = session_id.to_owned();
                let hold = self.deps.config.frames.goodbye_hold_ms;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(hold)).await;
                    let _ = state
                        .end_session(&session, "goodbye", mm_data::rdf::now_ms())
                        .await;
                });
            }
            let next_attempt = pending.attempt.saturating_add(1);
            if template.kind != "goodbye" && next_attempt < template.utterance.max_attempts {
                let delay = template
                    .utterance
                    .delay_ms
                    .saturating_mul(u64::from(template.utterance.backoff).pow(next_attempt.into()));
                self.deps
                    .state
                    .schedule_utterance(
                        session_id,
                        Some(delay),
                        "unanswered backoff",
                        bundle.frames.revision,
                        now,
                    )
                    .await
                    .ok();
            }
        }
        Ok(vec![AutomationEvent {
            session_id: session_id.into(),
            kind: "frame".into(),
            text,
            detail: "frame utterance delivered".into(),
            at: now,
        }])
    }
}

/// Start a fresh frame timer after a user reply.
pub async fn resume_frame_timer(
    state: &StateService,
    session_id: &str,
    now: i64,
) -> Result<(), String> {
    let bundle = state
        .load(session_id)
        .await
        .map_err(|error| error.to_string())?;
    if bundle.session.pending_utterance.is_some() {
        state
            .cancel_utterance(session_id, bundle.conversation.revision, now)
            .await
            .map_err(|error| error.to_string())?;
    }
    if bundle.frames.top().is_some() {
        state
            .schedule_utterance(
                session_id,
                Some(4_500),
                "user-reply",
                bundle.frames.revision,
                now,
            )
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Mark a frame timer as answered. This keeps status variables coherent for the
/// inspector; it does not invent a new frame stack.
pub async fn mark_frame_answered(
    state: &StateService,
    session_id: &str,
    now: i64,
) -> Result<(), String> {
    let bundle = state
        .load(session_id)
        .await
        .map_err(|error| error.to_string())?;
    state
        .update_status(
            session_id,
            [
                ("awaiting_user".to_owned(), StatusValue::Bool(false)),
                ("next_utterance_at".to_owned(), StatusValue::Integer(0)),
            ]
            .into_iter()
            .collect(),
            &[],
            bundle.conversation.revision,
            StateOrigin::Cm,
            now,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}
