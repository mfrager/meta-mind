//! The process supervisor.
//!
//! One engine owns every live instance. It claims queued instances under a
//! lease, executes one dependency-ready group at a time (a parallel group runs
//! its steps concurrently), persists each transition, and stops at a human,
//! user-input, timer or child-process wait. A second worker cannot claim a
//! leased instance, so a process has a single supervisor.
//!
//! Recovery is explicit: [`ProcessEngine::recover`] reads every non-terminal
//! instance, resets interrupted steps, rebuilds the state machine from its
//! snapshot and starts it again. A wait or pause survives a restart; a running
//! step is retried once, never twice in the same pass.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mm_data::process::{
    instance_uri, ProcessError, ProcessInstance, ProcessStatus, ProcessStore, ProcessTemplate,
    QueueState, StepKind, StepStatus, TemplateStatus, WaitCondition, WaitKind,
};
use mm_data::PlanStore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::task::JoinSet;

use crate::backend::{BackendRegistry, BackendRequest};
use crate::config::ProcessConfig;
use crate::machine;
use crate::queue;
use crate::template::{self, Group, Layout};

/// Wall clock, injectable for queue and expiry tests.
pub trait Clock: Send + Sync {
    /// Unix milliseconds.
    fn now_ms(&self) -> i64;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        mm_data::rdf::now_ms()
    }
}

/// Everything a step runner needs. The engine renders bindings before calling
/// the runner, so a runner never interprets template syntax.
#[derive(Debug, Clone)]
pub struct StepRequest {
    /// Process instance.
    pub instance_id: String,
    /// Owning session.
    pub session_id: String,
    /// Template URI.
    pub template_uri: String,
    /// Pinned template version.
    pub template_version: u32,
    /// Step id.
    pub step_id: String,
    /// Step title.
    pub title: String,
    /// Rendered instructions.
    pub instructions: String,
    /// The process task, from inputs or the template name.
    pub task: String,
    /// Outputs of completed steps.
    pub prior: BTreeMap<String, String>,
    /// Process inputs.
    pub inputs: BTreeMap<String, serde_json::Value>,
    /// The template's PROCESS.md, when it has one.
    pub doc: Option<String>,
    /// Tools this step may use; the runner intersects them with its pass scope.
    pub tool_scope: Vec<String>,
    /// Optional check the runner's verifier must satisfy.
    pub check: Option<String>,
    /// Nesting depth.
    pub depth: u32,
    /// Timeout in milliseconds.
    pub timeout_ms: u64,
}

/// What a step produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOutcome {
    /// The step's result text.
    pub reply: String,
    /// Prompt tokens.
    #[serde(default)]
    pub tokens_in: u32,
    /// Completion tokens.
    #[serde(default)]
    pub tokens_out: u32,
    /// Prompt instance used, for tracing.
    #[serde(default)]
    pub prompt_instance_id: Option<String>,
}

impl StepOutcome {
    /// A text-only outcome.
    pub fn text(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
            tokens_in: 0,
            tokens_out: 0,
            prompt_instance_id: None,
        }
    }
}

/// Executes one CM/tool step.
#[async_trait]
pub trait StepRunner: Send + Sync {
    /// Run a step to completion or fail with a typed error.
    async fn run_step(&self, request: StepRequest) -> Result<StepOutcome, ProcessError>;
}

/// The runner used when the pipeline is enabled without a model runtime.
pub struct UnavailableStepRunner;

#[async_trait]
impl StepRunner for UnavailableStepRunner {
    async fn run_step(&self, _request: StepRequest) -> Result<StepOutcome, ProcessError> {
        Err(ProcessError::BackendUnavailable(
            "no process step runner is configured".into(),
        ))
    }
}

/// One aggregate row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateAnalytics {
    /// Instances seen.
    pub total: usize,
    /// Completed.
    pub completed: usize,
    /// Failed.
    pub failed: usize,
    /// Waiting or paused.
    pub waiting: usize,
    /// Mean wall time of terminal instances.
    pub average_duration_ms: u64,
}

/// Deterministic analytics over the instance store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessAnalytics {
    /// Instances seen.
    pub total: usize,
    /// Still live.
    pub live: usize,
    /// Completed.
    pub completed: usize,
    /// Failed.
    pub failed: usize,
    /// Cancelled.
    pub cancelled: usize,
    /// Per-template rows.
    pub by_template: BTreeMap<String, TemplateAnalytics>,
}

/// The process supervisor.
pub struct ProcessEngine {
    store: Arc<dyn ProcessStore>,
    runner: Arc<dyn StepRunner>,
    config: ProcessConfig,
    clock: Arc<dyn Clock>,
    backends: BackendRegistry,
    workspace: PathBuf,
    known_tools: Vec<String>,
    live: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for ProcessEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessEngine")
            .field("config", &self.config)
            .field("live", &self.live.lock().map(|live| live.len()))
            .finish_non_exhaustive()
    }
}

impl ProcessEngine {
    /// Build the engine.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<dyn ProcessStore>,
        runner: Arc<dyn StepRunner>,
        config: ProcessConfig,
        clock: Arc<dyn Clock>,
        backends: BackendRegistry,
        workspace: impl Into<PathBuf>,
        known_tools: Vec<String>,
    ) -> Result<Self, ProcessError> {
        config.validate()?;
        Ok(Self {
            store,
            runner,
            config,
            clock,
            backends,
            workspace: workspace.into(),
            known_tools,
            live: Mutex::new(HashSet::new()),
        })
    }

    /// The reviewed configuration.
    pub fn config(&self) -> &ProcessConfig {
        &self.config
    }

    /// The store.
    pub fn store(&self) -> Arc<dyn ProcessStore> {
        Arc::clone(&self.store)
    }

    /// The registered backend kinds (normally empty).
    pub fn backend_kinds(&self) -> Vec<String> {
        self.backends.kinds()
    }

    /// Save a new or changed template. The caller owns version bumps.
    pub async fn save_template(
        &self,
        mut template: ProcessTemplate,
        expected_revision: u64,
    ) -> Result<ProcessTemplate, ProcessError> {
        if template.doc_path.is_some() {
            template::load_doc(&mut template, &self.workspace, &self.config.limits)?;
        }
        template::validate_template(&template, &self.config.limits, &self.known_tools)?;
        self.store
            .save_template_version(&template, expected_revision)
            .await
    }

    /// Soft-delete a template.
    pub async fn deprecate_template(&self, uri: &str) -> Result<ProcessTemplate, ProcessError> {
        let mut template = self.template(uri).await?;
        template.status = TemplateStatus::Deprecated;
        template.deleted = true;
        let revision = template.revision;
        self.store.save_template(&template, revision).await
    }

    /// Load a template.
    pub async fn template(&self, uri: &str) -> Result<ProcessTemplate, ProcessError> {
        self.store
            .load_template(uri)
            .await?
            .ok_or_else(|| ProcessError::NotFound {
                kind: "template",
                id: uri.to_owned(),
            })
    }

    /// List templates.
    pub async fn templates(
        &self,
        include_deleted: bool,
    ) -> Result<Vec<ProcessTemplate>, ProcessError> {
        self.store.list_templates(include_deleted).await
    }

    /// Create and optionally start an instance.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_instance(
        self: &Arc<Self>,
        template_uri: &str,
        session_id: &str,
        inputs: BTreeMap<String, serde_json::Value>,
        priority: i64,
        run: bool,
        dedupe_key: Option<String>,
    ) -> Result<ProcessInstance, ProcessError> {
        let template = self.template(template_uri).await?;
        if template.deleted || template.status != TemplateStatus::Active {
            return Err(ProcessError::Invalid(format!(
                "template {template_uri} is not active"
            )));
        }
        template::layout(&template, self.config.limits.max_parallel)?;
        if let Some(key) = dedupe_key.as_deref() {
            let existing = self.store.list_instances_for_session(session_id).await?;
            if let Some(found) = queue::dedupe_conflict(existing.iter(), key) {
                return Ok(found.clone());
            }
        }
        let now = self.clock.now_ms();
        let mut instance = ProcessInstance::new(
            instance_uri(&mm_data::rdf::new_id()),
            &template,
            session_id,
            now,
        );
        instance.inputs = inputs;
        instance.queue.priority = priority;
        instance.queue.dedupe_key = dedupe_key;
        instance.push_event(now, "queued", None, template.name.clone());
        let saved = self.store.save_instance(&instance, 0).await?;
        if run {
            let _ = self.start_instance(&saved.id)?;
        }
        Ok(saved)
    }

    /// Start a queued instance in the background.
    ///
    /// Returns `false` when the concurrency cap is reached or the instance is
    /// already live; the instance stays queued and a later drain retries it.
    pub fn start_instance(self: &Arc<Self>, id: &str) -> Result<bool, ProcessError> {
        {
            let mut live = self
                .live
                .lock()
                .map_err(|_| ProcessError::Unavailable("engine lock poisoned".into()))?;
            if live.len() >= self.config.engine.max_live_instances {
                return Ok(false);
            }
            if !live.insert(id.to_owned()) {
                return Ok(false);
            }
        }
        let engine = Arc::clone(self);
        let id = id.to_owned();
        tokio::spawn(async move {
            engine.run(id).await;
        });
        Ok(true)
    }

    /// Claim due queued instances and start them.
    pub async fn drain_queue(self: &Arc<Self>, owner: &str) -> Result<Vec<String>, ProcessError> {
        let now = self.clock.now_ms();
        let mut instances = self.store.list_instances().await?;
        let mut reclaimed: HashSet<String> = HashSet::new();
        for instance in &mut instances {
            if queue::reclaim_expired(instance, now) {
                reclaimed.insert(instance.id.clone());
            }
        }
        let slots = self
            .config
            .engine
            .max_live_instances
            .saturating_sub(self.live.lock().map(|live| live.len()).unwrap_or(0))
            .min(self.config.engine.batch_size);
        let batch = queue::claim_batch(
            &mut instances,
            now,
            owner,
            slots,
            self.config.engine.batch_size,
            self.config.engine.priority_aging_ms,
            self.config.engine.lease_ms,
        )?;
        for instance in &instances {
            if batch.ids.contains(&instance.id) || reclaimed.contains(&instance.id) {
                self.store
                    .save_instance(instance, instance.revision)
                    .await?;
            }
        }
        for id in &batch.ids {
            if !self.start_instance(id)? {
                // The instance stayed queued at the cap. Release the claim so the
                // next drain can retry it instead of waiting out a lease it never
                // used.
                if let Some(mut instance) = self.store.load_instance(id).await? {
                    if instance.status == ProcessStatus::Queued {
                        instance.queue.lease_owner = None;
                        instance.queue.leased_until = None;
                        self.store
                            .save_instance(&instance, instance.revision)
                            .await?;
                    }
                }
                tracing::debug!(process = %id, "process stayed queued at the concurrency cap");
            }
        }
        Ok(batch.ids)
    }

    async fn run(self: Arc<Self>, id: String) {
        let outcome = self.run_loop(&id).await;
        if let Err(error) = outcome {
            if let Err(fail_error) = self.fail(&id, &error.to_string()).await {
                tracing::error!(process = %id, error = %fail_error, "process failure could not be recorded");
            }
        }
        if let Ok(mut live) = self.live.lock() {
            live.remove(&id);
        }
    }

    async fn run_loop(&self, id: &str) -> Result<(), ProcessError> {
        let mut instance = self.load_instance(id).await?;
        if instance.status.is_terminal() {
            return Ok(());
        }
        if instance.status == ProcessStatus::Queued {
            instance.status = ProcessStatus::Running;
        } else if instance.status != ProcessStatus::Running {
            return Ok(());
        }
        let template = self
            .pinned_template(&instance.template_uri, instance.template_version)
            .await?;
        let layout = template::layout(&template, self.config.limits.max_parallel)?;
        let mut snapshot = self.snapshot(&instance, &template, &layout)?;
        if snapshot.state.as_str() == "created" {
            snapshot = machine::dispatch(&template, &layout, &snapshot, "start")?;
            instance.machine = to_value(&snapshot)?;
        }
        instance.push_event(
            self.clock.now_ms(),
            "started",
            None,
            format!("template {}", template.uri),
        );
        let mut instance = self.persist(instance).await?;

        loop {
            if instance.status != ProcessStatus::Running {
                return Ok(());
            }
            let Some(group_index) = next_group(&instance, &template, &layout) else {
                let _ = machine::dispatch(&template, &layout, &snapshot, "advance");
                instance.finish(
                    ProcessStatus::Completed,
                    self.clock.now_ms(),
                    "all steps completed",
                );
                let _ = self.persist(instance).await?;
                return Ok(());
            };
            let group = layout.groups[group_index].clone();
            instance.current_group = Some(group.key.clone());
            instance.queue.lease_owner = None;
            instance.queue.leased_until = None;
            for index in &group.steps {
                let step = &template.steps[*index];
                let state = instance.steps.entry(step.id.clone()).or_default();
                if state.status == StepStatus::Pending {
                    state.status = StepStatus::Running;
                    state.attempts = state.attempts.saturating_add(1);
                    state.started_at = Some(self.clock.now_ms());
                    state.error = None;
                }
            }
            instance.updated_at = self.clock.now_ms();
            instance.machine = to_value(&snapshot)?;
            instance = self.persist(instance.clone()).await?;
            let outputs: BTreeMap<String, String> = instance
                .steps
                .iter()
                .filter_map(|(id, state)| state.output.clone().map(|output| (id.clone(), output)))
                .collect();

            // A timer gate is resolved before any work in the group starts.
            for index in &group.steps {
                let step = &template.steps[*index];
                if let Some(wait) = &step.wait {
                    if wait.kind == WaitKind::Timer
                        && wait.wake_at.is_some_and(|at| at > self.clock.now_ms())
                    {
                        return self
                            .enter_wait(
                                instance,
                                &template,
                                &layout,
                                &snapshot,
                                &step.id.clone(),
                                wait.clone(),
                            )
                            .await;
                    }
                }
            }

            // Run CM/tool/backend members concurrently.
            let mut set = JoinSet::new();
            for index in &group.steps {
                let step = template.steps[*index].clone();
                if matches!(
                    step.kind,
                    StepKind::Human
                        | StepKind::UserInput
                        | StepKind::SubProcess
                        | StepKind::Backend
                ) {
                    continue;
                }
                let runner = Arc::clone(&self.runner);
                let instance_id = instance.id.clone();
                let session_id = instance.session_id.clone();
                let inputs = instance.inputs.clone();
                let prior = outputs.clone();
                let doc = template.instructions.clone();
                let template_uri = template.uri.clone();
                let template_version = template.version;
                let depth = instance.depth;
                let timeout_ms = step
                    .timeout_ms
                    .unwrap_or(self.config.engine.step_timeout_ms);
                set.spawn(async move {
                    let timeout_ms = timeout_ms.min(u64::MAX / 2);
                    let rendered = template::render_bindings(&step.instructions, &inputs, &prior)?;
                    let request = StepRequest {
                        instance_id,
                        session_id,
                        template_uri,
                        template_version,
                        step_id: step.id.clone(),
                        title: step.title.clone(),
                        instructions: rendered,
                        task: inputs
                            .get("task")
                            .and_then(|value| value.as_str())
                            .map(str::to_owned)
                            .unwrap_or_else(|| step.title.clone()),
                        prior,
                        inputs,
                        doc,
                        tool_scope: step.tool_scope.clone(),
                        check: step.check.clone(),
                        depth,
                        timeout_ms,
                    };
                    let request_id = request.step_id.clone();
                    let outcome = tokio::time::timeout(
                        Duration::from_millis(timeout_ms),
                        runner.run_step(request),
                    )
                    .await
                    .map_err(|_| {
                        ProcessError::Invalid(format!("step {request_id:?} exceeded its timeout"))
                    })?;
                    Ok::<_, ProcessError>((request_id, outcome))
                });
            }
            let mut failure: Option<(String, String)> = None;
            while let Some(joined) = set.join_next().await {
                match joined {
                    Ok(Ok((step_id, Ok(outcome)))) => {
                        if let Some(state) = instance.steps.get_mut(&step_id) {
                            state.status = StepStatus::Done;
                            state.output = Some(truncate(
                                &outcome.reply,
                                self.config.limits.max_step_output_chars,
                            ));
                            state.finished_at = Some(self.clock.now_ms());
                            state.wait = None;
                        }
                        instance.tokens_in = instance.tokens_in.saturating_add(outcome.tokens_in);
                        instance.tokens_out =
                            instance.tokens_out.saturating_add(outcome.tokens_out);
                        instance.push_event(
                            self.clock.now_ms(),
                            "step_done",
                            Some(&step_id),
                            outcome.prompt_instance_id.unwrap_or_default(),
                        );
                    }
                    Ok(Ok((step_id, Err(error)))) => {
                        let message = error.to_string();
                        if let Some(state) = instance.steps.get_mut(&step_id) {
                            state.status = StepStatus::Failed;
                            state.error = Some(message.clone());
                            state.finished_at = Some(self.clock.now_ms());
                        }
                        failure = Some((step_id, message));
                    }
                    Ok(Err(error)) => failure = Some((String::new(), error.to_string())),
                    Err(join_error) => {
                        failure = Some((String::new(), format!("step task failed: {join_error}")));
                    }
                }
            }

            // Backend steps, after the CM members so a borrow of the registry is
            // never held across a model call.
            for index in &group.steps {
                let step = template.steps[*index].clone();
                if step.kind != StepKind::Backend {
                    continue;
                }
                if instance
                    .steps
                    .get(&step.id)
                    .is_some_and(|state| state.status == StepStatus::Done)
                {
                    continue;
                }
                let service = step.service.clone().ok_or_else(|| {
                    ProcessError::Invalid(format!("backend step {:?} has no service", step.id))
                })?;
                let outcome = self
                    .backends
                    .execute(BackendRequest {
                        session_id: instance.session_id.clone(),
                        process_id: instance.id.clone(),
                        step_id: step.id.clone(),
                        service,
                        input: json!({
                            "inputs": instance.inputs,
                            "instructions": step.instructions,
                        }),
                    })
                    .await;
                match outcome {
                    Ok(response) if response.ok && response.handle.is_none() => {
                        if let Some(state) = instance.steps.get_mut(&step.id) {
                            state.status = StepStatus::Done;
                            state.output = Some(truncate(
                                &response.output.to_string(),
                                self.config.limits.max_step_output_chars,
                            ));
                            state.finished_at = Some(self.clock.now_ms());
                        }
                    }
                    Ok(response) if response.ok => {
                        return self
                            .enter_wait(
                                instance,
                                &template,
                                &layout,
                                &snapshot,
                                &step.id,
                                WaitCondition {
                                    kind: WaitKind::External,
                                    detail: format!(
                                        "handle:{}",
                                        response.handle.unwrap_or_default()
                                    ),
                                    wake_at: None,
                                },
                            )
                            .await;
                    }
                    Ok(response) => {
                        failure = Some((
                            step.id.clone(),
                            if response.detail.is_empty() {
                                "backend reported failure".to_owned()
                            } else {
                                response.detail
                            },
                        ));
                    }
                    Err(ProcessError::BackendUnavailable(message)) => {
                        failure =
                            Some((step.id.clone(), format!("backend_unavailable: {message}")));
                    }
                    Err(error) => failure = Some((step.id.clone(), error.to_string())),
                }
            }

            // Sub-process steps run inline; a child wait becomes the parent wait.
            for index in &group.steps {
                let step = template.steps[*index].clone();
                if step.kind != StepKind::SubProcess {
                    continue;
                }
                if instance
                    .steps
                    .get(&step.id)
                    .is_some_and(|state| state.status == StepStatus::Done)
                {
                    continue;
                }
                match self.run_child(&instance, &template, &step).await {
                    Ok(ChildOutcome::Done { reply, child_id }) => {
                        if let Some(state) = instance.steps.get_mut(&step.id) {
                            state.status = StepStatus::Done;
                            state.output =
                                Some(truncate(&reply, self.config.limits.max_step_output_chars));
                            state.finished_at = Some(self.clock.now_ms());
                        }
                        instance.push_event(
                            self.clock.now_ms(),
                            "step_done",
                            Some(&step.id),
                            format!("child {child_id}"),
                        );
                    }
                    Ok(ChildOutcome::Waiting { child_id, wait }) => {
                        return self
                            .enter_wait(
                                instance,
                                &template,
                                &layout,
                                &snapshot,
                                &step.id,
                                WaitCondition {
                                    detail: format!("child:{child_id} {}", wait.detail),
                                    ..wait
                                },
                            )
                            .await;
                    }
                    Err(error) => failure = Some((step.id.clone(), error.to_string())),
                }
            }

            if let Some((step_id, message)) = failure {
                if let Some(state) = instance.steps.get_mut(&step_id) {
                    state.status = StepStatus::Failed;
                    state.error = Some(message.clone());
                }
                if !machine::is_terminal(&snapshot) {
                    if let Ok(next) = machine::dispatch(&template, &layout, &snapshot, "fail") {
                        snapshot = next;
                    }
                }
                instance.machine = to_value(&snapshot)?;
                instance.finish(ProcessStatus::Failed, self.clock.now_ms(), message);
                let _ = self.persist(instance).await?;
                return Ok(());
            }

            // Human and user-input steps wait after the work members finish.
            for index in &group.steps {
                let step = template.steps[*index].clone();
                if !matches!(step.kind, StepKind::Human | StepKind::UserInput) {
                    continue;
                }
                if instance
                    .steps
                    .get(&step.id)
                    .is_some_and(|state| state.status == StepStatus::Done)
                {
                    continue;
                }
                let wait = match step.kind {
                    StepKind::Human => WaitCondition {
                        kind: WaitKind::Approval,
                        detail: step
                            .wait
                            .as_ref()
                            .map(|wait| wait.detail.clone())
                            .unwrap_or_else(|| format!("approve step {}", step.title)),
                        wake_at: None,
                    },
                    _ => WaitCondition {
                        kind: WaitKind::UserInput,
                        detail: step
                            .wait
                            .as_ref()
                            .map(|wait| wait.detail.clone())
                            .unwrap_or_else(|| format!("input for step {}", step.title)),
                        wake_at: None,
                    },
                };
                return self
                    .enter_wait(instance, &template, &layout, &snapshot, &step.id, wait)
                    .await;
            }

            // The instance may have been paused or cancelled while a step ran.
            let latest = self.load_instance(&instance.id).await?;
            if latest.revision != instance.revision || latest.status != ProcessStatus::Running {
                // A pause or resume may have written while a step was running.
                // Merge its control fields and revision, and keep this worker's
                // step results; the optimistic write then applies to the latest
                // document instead of conflicting with the operator.
                instance.revision = latest.revision;
                instance.status = latest.status;
                instance.waiting = latest.waiting.clone();
                instance.machine = latest.machine.clone();
                instance.queue = latest.queue.clone();
            }
            if latest.status != ProcessStatus::Running {
                let _ = self.persist(instance).await?;
                return Ok(());
            }
            snapshot = machine::dispatch(&template, &layout, &snapshot, "advance")?;
            instance.machine = to_value(&snapshot)?;
            instance.current_group = None;
            instance = self.persist(instance).await?;
            if snapshot.state.as_str() == "completed" {
                if instance.status == ProcessStatus::Running {
                    instance.finish(
                        ProcessStatus::Completed,
                        self.clock.now_ms(),
                        "all steps completed",
                    );
                    let _ = self.persist(instance).await?;
                }
                return Ok(());
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn enter_wait(
        &self,
        mut instance: ProcessInstance,
        template: &ProcessTemplate,
        layout: &Layout,
        snapshot: &mm_sm::SmSnapshot,
        step_id: &str,
        wait: WaitCondition,
    ) -> Result<(), ProcessError> {
        if let Some(state) = instance.steps.get_mut(step_id) {
            state.status = StepStatus::Waiting;
            state.wait = Some(wait.clone());
            state.attempts = state.attempts.saturating_sub(1);
        }
        instance.status = ProcessStatus::Waiting;
        instance.waiting = Some(wait.clone());
        instance.push_event(
            self.clock.now_ms(),
            "waiting",
            Some(step_id),
            wait.detail.clone(),
        );
        let next = machine::dispatch(template, layout, snapshot, "wait")?;
        instance.machine = to_value(&next)?;
        let _ = self.persist(instance).await?;
        Ok(())
    }

    async fn run_child(
        &self,
        parent: &ProcessInstance,
        parent_template: &ProcessTemplate,
        step: &mm_data::process::TemplateStep,
    ) -> Result<ChildOutcome, ProcessError> {
        let Some(child_uri) = step.template_uri.as_deref() else {
            return Err(ProcessError::SubProcessRefused(
                "sub-process step has no template".into(),
            ));
        };
        let depth = parent.depth.saturating_add(1);
        if depth > self.config.limits.max_depth {
            return Err(ProcessError::SubProcessRefused(format!(
                "nesting depth {depth} exceeds {}",
                self.config.limits.max_depth
            )));
        }
        let mut cursor = Some(parent.id.clone());
        while let Some(id) = cursor {
            let ancestor = self.load_instance(&id).await?;
            if ancestor.template_uri == child_uri {
                return Err(ProcessError::SubProcessRefused(format!(
                    "template {child_uri} already appears in the ancestor chain"
                )));
            }
            cursor = ancestor.parent;
        }
        let child_template = self.template(child_uri).await?;
        let now = self.clock.now_ms();
        let mut child = ProcessInstance::new(
            instance_uri(&mm_data::rdf::new_id()),
            &child_template,
            parent.session_id.clone(),
            now,
        );
        child.parent = Some(parent.id.clone());
        child.depth = depth;
        child.inputs = parent.inputs.clone();
        child.queue.priority = parent.queue.priority;
        child.queue.dedupe_key = Some(format!("child:{}:{}", parent.id, step.id));
        child.push_event(now, "queued", None, format!("child of {}", parent.id));
        {
            let mut live = self
                .live
                .lock()
                .map_err(|_| ProcessError::Unavailable("engine lock poisoned".into()))?;
            live.insert(child.id.clone());
        }
        let child = self.store.save_instance(&child, 0).await?;
        let result = Box::pin(self.run_loop(&child.id)).await;
        if let Ok(mut live) = self.live.lock() {
            live.remove(&child.id);
        }
        result?;
        let finished = self.load_instance(&child.id).await?;
        match finished.status {
            ProcessStatus::Completed => Ok(ChildOutcome::Done {
                reply: finished
                    .result
                    .unwrap_or_else(|| "child completed".to_owned()),
                child_id: finished.id,
            }),
            ProcessStatus::Waiting | ProcessStatus::Paused => Ok(ChildOutcome::Waiting {
                child_id: finished.id,
                wait: finished.waiting.unwrap_or(WaitCondition {
                    kind: WaitKind::External,
                    detail: format!("child of {}", parent_template.uri),
                    wake_at: None,
                }),
            }),
            ProcessStatus::Failed => Err(ProcessError::Invalid(format!(
                "child process failed: {}",
                finished.error.unwrap_or_else(|| "unknown".into())
            ))),
            other => Err(ProcessError::Invalid(format!(
                "child process ended in unexpected state {}",
                other.as_str()
            ))),
        }
    }

    /// Resume a waiting or paused instance.
    pub async fn resume(
        self: &Arc<Self>,
        id: &str,
        input: Option<&str>,
    ) -> Result<ProcessInstance, ProcessError> {
        let mut instance = self.load_instance(id).await?;
        if instance.status.is_terminal() {
            return Err(ProcessError::Invalid(format!(
                "process {id} is already {}",
                instance.status.as_str()
            )));
        }
        if instance.status != ProcessStatus::Waiting && instance.status != ProcessStatus::Paused {
            return Err(ProcessError::Invalid(format!(
                "process {id} is not waiting or paused"
            )));
        }
        let template = self
            .pinned_template(&instance.template_uri, instance.template_version)
            .await?;
        let layout = template::layout(&template, self.config.limits.max_parallel)?;
        let mut snapshot = self.snapshot(&instance, &template, &layout)?;
        let waiting = instance
            .steps
            .iter()
            .find(|(_, state)| state.status == StepStatus::Waiting)
            .map(|(step_id, state)| (step_id.clone(), state.wait.clone()));
        if let Some((step_id, Some(wait))) = waiting {
            if let Some(rest) = wait.detail.strip_prefix("child:") {
                let child_id = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                if !child_id.is_empty() {
                    let child = self.load_instance(&child_id).await?;
                    if child.status == ProcessStatus::Waiting
                        || child.status == ProcessStatus::Paused
                    {
                        Box::pin(self.resume(&child_id, input)).await?;
                    }
                    let child = self.load_instance(&child_id).await?;
                    if !child.status.is_terminal() {
                        return Ok(instance);
                    }
                    if let Some(state) = instance.steps.get_mut(&step_id) {
                        state.status = StepStatus::Done;
                        state.output = child.result.clone().or(child.error.clone());
                        state.finished_at = Some(self.clock.now_ms());
                        state.wait = None;
                    }
                }
            } else {
                let timer_gate = wait.kind == WaitKind::Timer
                    && template.step(&step_id).is_some_and(|step| {
                        matches!(step.kind, StepKind::Cm | StepKind::Backend | StepKind::Tool)
                    });
                if let Some(state) = instance.steps.get_mut(&step_id) {
                    if timer_gate {
                        // A timer gates the work; it does not replace it.
                        state.status = StepStatus::Pending;
                    } else {
                        state.status = StepStatus::Done;
                        state.output = Some(
                            input
                                .filter(|value| !value.trim().is_empty())
                                .unwrap_or("approved")
                                .to_owned(),
                        );
                        state.finished_at = Some(self.clock.now_ms());
                    }
                    state.wait = None;
                }
            }
            if matches!(snapshot.state.as_str(), "waiting" | "paused") {
                if let Some(index) = index_of_step(&template, &step_id) {
                    if let Some((_, group)) = layout.group_of(index) {
                        let event = format!("resume:{}", leader_state(&template, group));
                        snapshot = machine::dispatch(&template, &layout, &snapshot, &event)?;
                    }
                }
            }
        } else {
            for state in instance.steps.values_mut() {
                if state.status == StepStatus::Running {
                    state.status = StepStatus::Pending;
                }
            }
            let event = instance
                .current_group
                .as_deref()
                .and_then(|key| layout.groups.iter().find(|group| group.key == key))
                .map(|group| format!("resume:{}", leader_state(&template, group)))
                .or_else(|| {
                    next_group(&instance, &template, &layout).map(|index| {
                        format!("resume:{}", leader_state(&template, &layout.groups[index]))
                    })
                });
            if let Some(event) = event {
                if matches!(snapshot.state.as_str(), "waiting" | "paused") {
                    snapshot = machine::dispatch(&template, &layout, &snapshot, &event)?;
                }
            }
        }
        instance.status = ProcessStatus::Running;
        instance.waiting = None;
        instance.machine = to_value(&snapshot)?;
        instance.push_event(self.clock.now_ms(), "resumed", None, "operator resume");
        let saved = self.persist(instance).await?;
        let _ = self.start_instance(&saved.id)?;
        Ok(saved)
    }

    /// Pause a running or waiting instance.
    pub async fn pause(&self, id: &str) -> Result<ProcessInstance, ProcessError> {
        let mut instance = self.load_instance(id).await?;
        if !instance.status.is_live() || instance.status == ProcessStatus::Paused {
            return Err(ProcessError::Invalid(format!(
                "process {id} cannot be paused from {}",
                instance.status.as_str()
            )));
        }
        let template = self
            .pinned_template(&instance.template_uri, instance.template_version)
            .await?;
        let layout = template::layout(&template, self.config.limits.max_parallel)?;
        let snapshot = self.snapshot(&instance, &template, &layout)?;
        if !machine::is_waiting(&snapshot) && snapshot.state.as_str().starts_with("step:") {
            instance.machine =
                to_value(&machine::dispatch(&template, &layout, &snapshot, "pause")?)?;
        }
        instance.status = ProcessStatus::Paused;
        instance.push_event(self.clock.now_ms(), "paused", None, "operator pause");
        self.persist(instance).await
    }

    /// Cancel an instance; terminal instances are unchanged.
    pub async fn cancel(&self, id: &str) -> Result<ProcessInstance, ProcessError> {
        let mut instance = self.load_instance(id).await?;
        if instance.status.is_terminal() {
            return Ok(instance);
        }
        let template = self
            .pinned_template(&instance.template_uri, instance.template_version)
            .await?;
        let layout = template::layout(&template, self.config.limits.max_parallel)?;
        let snapshot = self.snapshot(&instance, &template, &layout)?;
        if !machine::is_terminal(&snapshot) {
            if let Ok(next) = machine::dispatch(&template, &layout, &snapshot, "cancel") {
                instance.machine = to_value(&next)?;
            }
        }
        instance.finish(
            ProcessStatus::Cancelled,
            self.clock.now_ms(),
            "cancelled by operator",
        );
        self.persist(instance).await
    }

    /// Reset one step and resume from its group.
    pub async fn retry_step(
        self: &Arc<Self>,
        id: &str,
        step_id: &str,
    ) -> Result<ProcessInstance, ProcessError> {
        let mut instance = self.load_instance(id).await?;
        let template = self
            .pinned_template(&instance.template_uri, instance.template_version)
            .await?;
        let index = index_of_step(&template, step_id).ok_or_else(|| ProcessError::NotFound {
            kind: "step",
            id: step_id.to_owned(),
        })?;
        let layout = template::layout(&template, self.config.limits.max_parallel)?;
        let (_, group) = layout
            .group_of(index)
            .ok_or_else(|| ProcessError::Invalid("step has no group".into()))?;
        for state in instance.steps.values_mut() {
            if state.status == StepStatus::Running {
                state.status = StepStatus::Pending;
            }
        }
        if let Some(state) = instance.steps.get_mut(step_id) {
            state.status = StepStatus::Pending;
            state.error = None;
            state.output = None;
            state.wait = None;
        }
        instance.current_group = Some(group.key.clone());
        instance.status = ProcessStatus::Running;
        instance.waiting = None;
        instance.error = None;
        instance.machine = to_value(&machine::at_state(&template, &layout, group, &instance)?)?;
        instance.push_event(
            self.clock.now_ms(),
            "retried",
            Some(step_id),
            "operator retry",
        );
        let saved = self.persist(instance).await?;
        let _ = self.start_instance(&saved.id)?;
        Ok(saved)
    }

    /// Recover non-terminal instances after a restart.
    pub async fn recover(self: &Arc<Self>) -> Result<Vec<String>, ProcessError> {
        let mut recovered = Vec::new();
        for mut instance in self.store.list_instances().await? {
            if instance.status != ProcessStatus::Running {
                continue;
            }
            for state in instance.steps.values_mut() {
                if state.status == StepStatus::Running {
                    state.status = StepStatus::Pending;
                    state.error = None;
                }
            }
            instance.push_event(self.clock.now_ms(), "recovered", None, "restart recovery");
            let saved = self.persist(instance).await?;
            if self.start_instance(&saved.id)? {
                recovered.push(saved.id);
            }
        }
        Ok(recovered)
    }

    /// Deterministic analytics.
    pub async fn analytics(
        &self,
        session_id: Option<&str>,
    ) -> Result<ProcessAnalytics, ProcessError> {
        let instances = match session_id {
            Some(session) => self.store.list_instances_for_session(session).await?,
            None => self.store.list_instances().await?,
        };
        let mut analytics = ProcessAnalytics {
            total: instances.len(),
            live: 0,
            completed: 0,
            failed: 0,
            cancelled: 0,
            by_template: BTreeMap::new(),
        };
        let mut durations: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for instance in &instances {
            let row = analytics
                .by_template
                .entry(instance.template_uri.clone())
                .or_insert(TemplateAnalytics {
                    total: 0,
                    completed: 0,
                    failed: 0,
                    waiting: 0,
                    average_duration_ms: 0,
                });
            row.total += 1;
            match instance.status {
                ProcessStatus::Completed => {
                    analytics.completed += 1;
                    row.completed += 1;
                }
                ProcessStatus::Failed => {
                    analytics.failed += 1;
                    row.failed += 1;
                }
                ProcessStatus::Cancelled => analytics.cancelled += 1,
                ProcessStatus::Waiting | ProcessStatus::Paused => {
                    analytics.live += 1;
                    row.waiting += 1;
                }
                ProcessStatus::Running | ProcessStatus::Queued => analytics.live += 1,
            }
            if instance.status.is_terminal() {
                let duration = instance
                    .updated_at
                    .saturating_sub(instance.created_at)
                    .max(0) as u64;
                let entry = durations
                    .entry(instance.template_uri.clone())
                    .or_insert((0, 0));
                entry.0 += duration;
                entry.1 += 1;
            }
        }
        for (uri, (sum, count)) in durations {
            if let Some(row) = analytics.by_template.get_mut(&uri) {
                row.average_duration_ms = sum.checked_div(count).unwrap_or(0);
            }
        }
        Ok(analytics)
    }

    /// Export an instance and its pinned template.
    pub async fn export(&self, id: &str, rdf: bool) -> Result<String, ProcessError> {
        let instance = self.load_instance(id).await?;
        let template = self
            .pinned_template(&instance.template_uri, instance.template_version)
            .await?;
        if rdf {
            let mut out = String::new();
            out.push_str(&format!(
                "@prefix mm: <{}> .\n@prefix mmd: <{}> .\n\n",
                mm_data::rdf::MM_NS,
                mm_data::rdf::MMD_NS
            ));
            out.push_str(&format!(
                "<{}> a mm:ProcessTemplate ; mm:name \"{}\" ; mm:version {} .\n",
                escape_turtle(&template.uri),
                escape_turtle(&template.name),
                template.version
            ));
            out.push_str(&format!(
                "<{}> a mm:ProcessInstance ; mm:template <{}> ; mm:status \"{}\" .\n",
                escape_turtle(&instance.id),
                escape_turtle(&template.uri),
                instance.status.as_str()
            ));
            out.push_str(&format!(
                "<{}> mm:payload \"{}\" .\n",
                escape_turtle(&instance.id),
                escape_turtle(
                    &serde_json::to_string(&instance)
                        .map_err(|error| ProcessError::Invalid(error.to_string()))?
                )
            ));
            Ok(out)
        } else {
            serde_json::to_string_pretty(&json!({
                "schema": "mm.process.export.v1",
                "template": template,
                "instance": instance,
            }))
            .map_err(|error| ProcessError::Invalid(error.to_string()))
        }
    }

    /// Import a template, refusing a changed version under the same number.
    pub async fn import_template(
        &self,
        mut template: ProcessTemplate,
    ) -> Result<ProcessTemplate, ProcessError> {
        if template.doc_path.is_some() {
            template::load_doc(&mut template, &self.workspace, &self.config.limits)?;
        }
        template::validate_template(&template, &self.config.limits, &self.known_tools)?;
        let current = self.store.load_template(&template.uri).await?;
        let revision = current.as_ref().map_or(0, |value| value.revision);
        if let Some(existing) = current {
            if existing.version == template.version {
                return Err(ProcessError::Invalid(
                    "imported template reuses an existing version; bump its version".into(),
                ));
            }
        }
        self.store.save_template_version(&template, revision).await
    }

    /// Load one instance.
    pub async fn load_instance(&self, id: &str) -> Result<ProcessInstance, ProcessError> {
        self.store
            .load_instance(id)
            .await?
            .ok_or_else(|| ProcessError::NotFound {
                kind: "process",
                id: id.to_owned(),
            })
    }

    /// Instances for a session.
    pub async fn session_instances(
        &self,
        session_id: &str,
    ) -> Result<Vec<ProcessInstance>, ProcessError> {
        self.store.list_instances_for_session(session_id).await
    }

    /// Expire plans whose deadline passed.
    pub async fn expire_plans(&self, plans: &dyn PlanStore) -> Result<usize, ProcessError> {
        crate::plan_builder::expire_due(plans, self.clock.now_ms()).await
    }

    async fn pinned_template(
        &self,
        uri: &str,
        version: u32,
    ) -> Result<ProcessTemplate, ProcessError> {
        if let Some(version) = self.store.load_template_version(uri, version).await? {
            return Ok(version.template);
        }
        self.template(uri).await
    }

    fn snapshot(
        &self,
        instance: &ProcessInstance,
        template: &ProcessTemplate,
        layout: &Layout,
    ) -> Result<mm_sm::SmSnapshot, ProcessError> {
        if instance.machine.is_null() {
            return machine::start(template, layout, instance);
        }
        serde_json::from_value(instance.machine.clone()).map_err(|error| {
            ProcessError::Invalid(format!("process machine snapshot is invalid: {error}"))
        })
    }

    async fn persist(&self, instance: ProcessInstance) -> Result<ProcessInstance, ProcessError> {
        let revision = instance.revision;
        self.store.save_instance(&instance, revision).await
    }

    async fn fail(&self, id: &str, message: &str) -> Result<(), ProcessError> {
        let mut instance = self.load_instance(id).await?;
        if instance.status.is_terminal() {
            return Ok(());
        }
        instance.finish(ProcessStatus::Failed, self.clock.now_ms(), message);
        self.persist(instance).await?;
        Ok(())
    }
}

enum ChildOutcome {
    Done {
        reply: String,
        child_id: String,
    },
    Waiting {
        child_id: String,
        wait: WaitCondition,
    },
}

fn next_group(
    instance: &ProcessInstance,
    template: &ProcessTemplate,
    layout: &Layout,
) -> Option<usize> {
    layout.groups.iter().position(|group| {
        group.steps.iter().any(|index| {
            let id = &template.steps[*index].id;
            instance
                .steps
                .get(id)
                .is_none_or(|state| !state.status.is_satisfied())
        })
    })
}

fn index_of_step(template: &ProcessTemplate, step_id: &str) -> Option<usize> {
    template.steps.iter().position(|step| step.id == step_id)
}

fn leader_state(template: &ProcessTemplate, group: &Group) -> String {
    machine::step_state(&template.steps[group.steps[0]].id)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn to_value(snapshot: &mm_sm::SmSnapshot) -> Result<serde_json::Value, ProcessError> {
    serde_json::to_value(snapshot).map_err(|error| ProcessError::Invalid(error.to_string()))
}

fn escape_turtle(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Reset an instance's queue fields for a re-enqueue.
pub fn reset_queue(instance: &mut ProcessInstance, at: i64, priority: i64) {
    instance.queue = QueueState {
        enqueued_at: at,
        priority,
        ..QueueState::default()
    };
}
