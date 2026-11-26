//! Durable process templates, instances and versions.
//!
//! A process template is a reviewable recipe: named steps, each with
//! instructions, a tool scope, dependencies and an optional parallel group. A
//! process instance is one running copy with its state-machine snapshot, per-step
//! state, queue position and event log. Both are stored as JSON payloads in the
//! `process` graph, exactly like plans, so the RDF surface stays stable while the
//! Rust type evolves.
//!
//! Templates and instances are append-only: every write stamps a revision, and
//! readers take the newest payload. Template versions are separate subjects so
//! an instance can pin the version it started under.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::engine::RdfEngine;
use crate::rdf::{self, MMD_NS, MM_NS};

/// The `process` graph alias.
pub const PROCESS_GRAPH: &str = "process";
/// `mm:ProcessTemplate`.
pub const MM_PROCESS_TEMPLATE: &str = "https://metamind.dev/ontology#ProcessTemplate";
/// `mm:ProcessTemplateVersion`.
pub const MM_PROCESS_TEMPLATE_VERSION: &str =
    "https://metamind.dev/ontology#ProcessTemplateVersion";
/// `mm:ProcessInstance`.
pub const MM_PROCESS_INSTANCE: &str = "https://metamind.dev/ontology#ProcessInstance";
/// `mm:StepExecution`.
pub const MM_STEP_EXECUTION: &str = "https://metamind.dev/ontology#StepExecution";

/// Maximum steps in a template.
pub const MAX_TEMPLATE_STEPS: usize = 50;
/// Maximum stored output for one step.
pub const MAX_STEP_OUTPUT_CHARS: usize = 8_000;
/// Maximum template document size.
pub const MAX_DOC_CHARS: usize = 65_536;
/// Maximum template instruction length.
pub const MAX_INSTRUCTION_CHARS: usize = 4_000;
/// Event log retention per instance.
pub const MAX_INSTANCE_EVENTS: usize = 200;

/// Process failures with stable tool-facing codes.
#[derive(Debug, Error)]
pub enum ProcessError {
    /// A field or invariant failed.
    #[error("{0}")]
    Invalid(String),
    /// The template or instance does not exist.
    #[error("unknown {kind} {id}")]
    NotFound {
        /// `template` or `process`.
        kind: &'static str,
        /// The identifier.
        id: String,
    },
    /// Optimistic revision mismatch.
    #[error("conflict: expected revision {expected}, current revision {current}")]
    Conflict {
        /// Expected revision.
        expected: u64,
        /// Current revision.
        current: u64,
    },
    /// A step kind has no configured runtime.
    #[error("backend_unavailable: {0}")]
    BackendUnavailable(String),
    /// Sub-process nesting or a cycle was refused.
    #[error("sub_process_refused: {0}")]
    SubProcessRefused(String),
    /// The store is unavailable.
    #[error("process store unavailable: {0}")]
    Unavailable(String),
}

/// Result alias for process operations.
pub type Result<T, E = ProcessError> = std::result::Result<T, E>;

/// How a template came to exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemplateSource {
    /// Shipped with the kernel.
    #[default]
    BuiltIn,
    /// Derived from a plan.
    Plan,
    /// Authored by an operator.
    User,
    /// Imported into a project.
    Project,
}

/// Template lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemplateStatus {
    /// Usable.
    #[default]
    Active,
    /// Retained for pinned instances, not offered.
    Deprecated,
}

/// What executes a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// A CM model/tool pass.
    #[default]
    Cm,
    /// A registered backend service; stubbed.
    Backend,
    /// A human approval gate.
    Human,
    /// The user's next message.
    UserInput,
    /// A child process.
    SubProcess,
    /// A direct tool call.
    Tool,
}

/// A wait condition attached to a step or reported by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    /// An operator approval.
    Approval,
    /// The user's next message.
    UserInput,
    /// A backend/external event.
    External,
    /// A clock deadline.
    Timer,
}

/// A declared or active wait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitCondition {
    /// What resolves the wait.
    pub kind: WaitKind,
    /// Human-readable description.
    #[serde(default)]
    pub detail: String,
    /// Wake instant for `Timer`, Unix milliseconds.
    #[serde(default)]
    pub wake_at: Option<i64>,
}

/// One template step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateStep {
    /// Stable step id, unique in the template.
    pub id: String,
    /// Short title.
    pub title: String,
    /// Natural-language instructions.
    pub instructions: String,
    /// Steps that must be done before this one.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Consecutive steps sharing a group run concurrently.
    #[serde(default)]
    pub parallel_group: Option<String>,
    /// Tools this step may call, intersected with the pass scope.
    #[serde(default)]
    pub tool_scope: Vec<String>,
    /// Execution kind.
    #[serde(default)]
    pub kind: StepKind,
    /// Optional check the verifier must satisfy.
    #[serde(default)]
    pub check: Option<String>,
    /// Step timeout override.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Wait condition for human/user_input/external steps.
    #[serde(default)]
    pub wait: Option<WaitCondition>,
    /// Child template URI for `SubProcess`.
    #[serde(default)]
    pub template_uri: Option<String>,
    /// Registered backend service for `Backend`.
    #[serde(default)]
    pub service: Option<String>,
}

impl TemplateStep {
    /// A CM step with no dependencies.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            instructions: instructions.into(),
            depends_on: Vec::new(),
            parallel_group: None,
            tool_scope: Vec::new(),
            kind: StepKind::Cm,
            check: None,
            timeout_ms: None,
            wait: None,
            template_uri: None,
            service: None,
        }
    }
}

/// A reviewable process template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessTemplate {
    /// Schema marker.
    #[serde(default = "process_schema")]
    pub schema: String,
    /// Template IRI, `urn:metamind:process/template/<id>`.
    pub uri: String,
    /// Human name.
    pub name: String,
    /// What it is for.
    #[serde(default)]
    pub description: String,
    /// Monotonic version, starting at 1.
    #[serde(default = "one")]
    pub version: u32,
    /// Where it came from.
    #[serde(default)]
    pub source: TemplateSource,
    /// Owning session, when derived from a plan.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Ordered steps.
    pub steps: Vec<TemplateStep>,
    /// Declared input names; only these bind.
    #[serde(default)]
    pub variables: Vec<String>,
    /// Workspace-relative PROCESS.md path.
    #[serde(default)]
    pub doc_path: Option<String>,
    /// Injected PROCESS.md text.
    #[serde(default)]
    pub instructions: Option<String>,
    /// Optimistic revision.
    #[serde(default = "one_u64")]
    pub revision: u64,
    /// Creation instant.
    pub created_at: i64,
    /// Last update instant.
    pub updated_at: i64,
    /// Lifecycle.
    #[serde(default)]
    pub status: TemplateStatus,
    /// Soft-deleted; never offered, retained for history.
    #[serde(default)]
    pub deleted: bool,
    /// The template this one was derived from.
    #[serde(default)]
    pub derived_from: Option<String>,
}

fn process_schema() -> String {
    "mm.process.v1".to_owned()
}

fn one() -> u32 {
    1
}

fn one_u64() -> u64 {
    1
}

impl ProcessTemplate {
    /// A one-step template.
    pub fn new(
        uri: impl Into<String>,
        name: impl Into<String>,
        steps: Vec<TemplateStep>,
        now_ms: i64,
    ) -> Self {
        Self {
            schema: process_schema(),
            uri: uri.into(),
            name: name.into(),
            description: String::new(),
            version: 1,
            source: TemplateSource::BuiltIn,
            session_id: None,
            steps,
            variables: Vec::new(),
            doc_path: None,
            instructions: None,
            revision: 1,
            created_at: now_ms,
            updated_at: now_ms,
            status: TemplateStatus::Active,
            deleted: false,
            derived_from: None,
        }
    }

    /// The step with this id.
    pub fn step(&self, id: &str) -> Option<&TemplateStep> {
        self.steps.iter().find(|step| step.id == id)
    }
}

/// One immutable template version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessTemplateVersion {
    /// Version subject IRI.
    pub uri: String,
    /// Template IRI.
    pub template_uri: String,
    /// Version number.
    pub version: u32,
    /// The full template snapshot.
    pub template: ProcessTemplate,
    /// Creation instant.
    pub created_at: i64,
}

/// Instance lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    /// Waiting in the queue.
    Queued,
    /// Executing.
    Running,
    /// Blocked on a wait condition.
    Waiting,
    /// Operator-paused.
    Paused,
    /// Every step is done.
    Completed,
    /// A step failed or the budget ended.
    Failed,
    /// Cancelled by the operator or session.
    Cancelled,
}

impl ProcessStatus {
    /// Whether the instance still occupies a slot.
    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Waiting | Self::Paused
        )
    }

    /// Whether no further transition is expected.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Per-step lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Not started.
    Pending,
    /// Executing.
    Running,
    /// Blocked on a wait.
    Waiting,
    /// Finished successfully.
    Done,
    /// Failed.
    Failed,
    /// Skipped because a dependency did not satisfy.
    Skipped,
}

impl StepStatus {
    /// Whether the step satisfies dependencies.
    pub fn is_satisfied(self) -> bool {
        matches!(self, Self::Done | Self::Skipped)
    }
}

/// Stored step state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepState {
    /// Lifecycle.
    pub status: StepStatus,
    /// Attempts started.
    #[serde(default)]
    pub attempts: u32,
    /// Stored output, bounded.
    #[serde(default)]
    pub output: Option<String>,
    /// Last error.
    #[serde(default)]
    pub error: Option<String>,
    /// Start instant.
    #[serde(default)]
    pub started_at: Option<i64>,
    /// Finish instant.
    #[serde(default)]
    pub finished_at: Option<i64>,
    /// The active or resolved wait.
    #[serde(default)]
    pub wait: Option<WaitCondition>,
}

impl Default for StepState {
    fn default() -> Self {
        Self {
            status: StepStatus::Pending,
            attempts: 0,
            output: None,
            error: None,
            started_at: None,
            finished_at: None,
            wait: None,
        }
    }
}

/// Queue fields persisted on the instance document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct QueueState {
    /// Enqueue instant.
    #[serde(default)]
    pub enqueued_at: i64,
    /// Operator/plan priority; higher runs first.
    #[serde(default)]
    pub priority: i64,
    /// Dedupe key; one live instance per key.
    #[serde(default)]
    pub dedupe_key: Option<String>,
    /// Batch key; steps with the same key may share an action batch.
    #[serde(default)]
    pub batch_key: Option<String>,
    /// Lease owner.
    #[serde(default)]
    pub lease_owner: Option<String>,
    /// Lease expiry, Unix milliseconds.
    #[serde(default)]
    pub leased_until: Option<i64>,
    /// Times the instance was claimed.
    #[serde(default)]
    pub claims: u32,
}

/// One ordered process event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessEvent {
    /// Monotonic sequence inside the instance.
    pub seq: u64,
    /// Instant.
    pub at: i64,
    /// Stable kind, e.g. `started`, `step_done`, `waiting`, `failed`.
    pub kind: String,
    /// Step id, when the event names one.
    #[serde(default)]
    pub step_id: Option<String>,
    /// Short detail.
    #[serde(default)]
    pub detail: String,
}

/// A durable process instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessInstance {
    /// Schema marker.
    #[serde(default = "process_schema")]
    pub schema: String,
    /// Instance id, `urn:metamind:process/instance/<id>`.
    pub id: String,
    /// Pinned template URI.
    pub template_uri: String,
    /// Pinned template version.
    pub template_version: u32,
    /// Owning session.
    pub session_id: String,
    /// Lifecycle.
    pub status: ProcessStatus,
    /// The serialized `mm_sm::SmSnapshot`; empty before the machine starts.
    #[serde(default)]
    pub machine: serde_json::Value,
    /// Declared inputs.
    #[serde(default)]
    pub inputs: BTreeMap<String, serde_json::Value>,
    /// Per-step state, keyed by template step id.
    #[serde(default)]
    pub steps: BTreeMap<String, StepState>,
    /// Queue metadata.
    #[serde(default)]
    pub queue: QueueState,
    /// The group currently executing.
    #[serde(default)]
    pub current_group: Option<String>,
    /// Active wait, when waiting.
    #[serde(default)]
    pub waiting: Option<WaitCondition>,
    /// Parent instance for a sub-process.
    #[serde(default)]
    pub parent: Option<String>,
    /// Nesting depth; root is 0.
    #[serde(default)]
    pub depth: u32,
    /// Wall-clock deadline.
    #[serde(default)]
    pub deadline_ms: Option<i64>,
    /// Creation instant.
    pub created_at: i64,
    /// Last update instant.
    pub updated_at: i64,
    /// Optimistic revision.
    #[serde(default = "one_u64")]
    pub revision: u64,
    /// Final summary.
    #[serde(default)]
    pub result: Option<String>,
    /// Terminal error.
    #[serde(default)]
    pub error: Option<String>,
    /// Cancellation instant.
    #[serde(default)]
    pub cancelled_at: Option<i64>,
    /// Tokens consumed by all steps.
    #[serde(default)]
    pub tokens_in: u32,
    /// Completion tokens produced by all steps.
    #[serde(default)]
    pub tokens_out: u32,
    /// Ordered events, bounded.
    #[serde(default)]
    pub events: Vec<ProcessEvent>,
    /// Next event sequence.
    #[serde(default)]
    pub next_event: u64,
}

impl ProcessInstance {
    /// A queued instance pinned to one template version.
    pub fn new(
        id: impl Into<String>,
        template: &ProcessTemplate,
        session_id: impl Into<String>,
        now_ms: i64,
    ) -> Self {
        Self {
            schema: process_schema(),
            id: id.into(),
            template_uri: template.uri.clone(),
            template_version: template.version,
            session_id: session_id.into(),
            status: ProcessStatus::Queued,
            machine: serde_json::Value::Null,
            inputs: BTreeMap::new(),
            steps: template
                .steps
                .iter()
                .map(|step| (step.id.clone(), StepState::default()))
                .collect(),
            queue: QueueState {
                enqueued_at: now_ms,
                ..QueueState::default()
            },
            current_group: None,
            waiting: None,
            parent: None,
            depth: 0,
            deadline_ms: None,
            created_at: now_ms,
            updated_at: now_ms,
            revision: 1,
            result: None,
            error: None,
            cancelled_at: None,
            tokens_in: 0,
            tokens_out: 0,
            events: Vec::new(),
            next_event: 0,
        }
    }

    /// Append one bounded event.
    pub fn push_event(
        &mut self,
        at: i64,
        kind: impl Into<String>,
        step_id: Option<&str>,
        detail: impl Into<String>,
    ) {
        self.next_event = self.next_event.saturating_add(1);
        self.events.push(ProcessEvent {
            seq: self.next_event,
            at,
            kind: kind.into(),
            step_id: step_id.map(str::to_owned),
            detail: detail.into(),
        });
        if self.events.len() > MAX_INSTANCE_EVENTS {
            let excess = self.events.len() - MAX_INSTANCE_EVENTS;
            self.events.drain(0..excess);
        }
    }

    /// Whether the queue lease is still held at `now`.
    pub fn leased(&self, now: i64) -> bool {
        self.queue.leased_until.is_some_and(|until| until > now)
    }

    /// Set the terminal status, refusing to overwrite a terminal one.
    pub fn finish(&mut self, status: ProcessStatus, at: i64, detail: impl Into<String>) {
        if self.status.is_terminal() {
            return;
        }
        self.status = status;
        self.updated_at = at;
        self.waiting = None;
        self.queue.lease_owner = None;
        self.queue.leased_until = None;
        let detail = detail.into();
        match status {
            ProcessStatus::Completed => self.result = Some(detail.clone()),
            ProcessStatus::Failed => self.error = Some(detail.clone()),
            ProcessStatus::Cancelled => self.cancelled_at = Some(at),
            _ => {}
        }
        self.push_event(at, status.as_str(), None, detail);
    }
}

/// Template and instance persistence.
#[async_trait::async_trait]
pub trait ProcessStore: Send + Sync {
    /// Load one template.
    async fn load_template(&self, uri: &str) -> Result<Option<ProcessTemplate>>;
    /// Save a template with optimistic concurrency; stamps `revision`.
    async fn save_template(
        &self,
        template: &ProcessTemplate,
        expected_revision: u64,
    ) -> Result<ProcessTemplate>;
    /// Every template, including soft-deleted when asked.
    async fn list_templates(&self, include_deleted: bool) -> Result<Vec<ProcessTemplate>>;
    /// Save a template and freeze one immutable version atomically.
    async fn save_template_version(
        &self,
        template: &ProcessTemplate,
        expected_revision: u64,
    ) -> Result<ProcessTemplate>;
    /// Load one frozen version.
    async fn load_template_version(
        &self,
        uri: &str,
        version: u32,
    ) -> Result<Option<ProcessTemplateVersion>>;
    /// Every frozen version, oldest first.
    async fn versions(&self, uri: &str) -> Result<Vec<ProcessTemplateVersion>>;
    /// Load one instance.
    async fn load_instance(&self, id: &str) -> Result<Option<ProcessInstance>>;
    /// Save one instance with optimistic concurrency.
    async fn save_instance(
        &self,
        instance: &ProcessInstance,
        expected_revision: u64,
    ) -> Result<ProcessInstance>;
    /// Every instance, newest first.
    async fn list_instances(&self) -> Result<Vec<ProcessInstance>>;
    /// Instances for one session, newest first.
    async fn list_instances_for_session(&self, session_id: &str) -> Result<Vec<ProcessInstance>>;
}

/// The RDF-backed process store.
#[derive(Debug, Clone)]
pub struct RdfProcessStore {
    engine: Arc<RdfEngine>,
    /// Last known instance revisions. A save normally checks the revision it
    /// just read; caching it avoids re-reading the whole graph for every step
    /// transition while keeping the optimistic check exact in this process.
    revisions: Arc<std::sync::Mutex<BTreeMap<String, u64>>>,
}

impl RdfProcessStore {
    /// Build over the data layer's engine.
    pub fn new(engine: Arc<RdfEngine>) -> Self {
        Self {
            engine,
            revisions: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        }
    }

    /// Decode every payload of `class` in one graph pass.
    ///
    /// The per-document `load_as` helper re-reads the whole graph, which is
    /// fine for one instance but quadratic for a queue listing. This reads the
    /// triples once, groups payloads by subject and keeps the highest revision.
    async fn payloads(&self, class: &str) -> Result<Vec<serde_json::Value>> {
        let triples = self
            .engine
            .triples(PROCESS_GRAPH)
            .await
            .map_err(store_error)?;
        let mut typed = std::collections::BTreeSet::new();
        for row in &triples {
            if row.get("p").and_then(rdf::term_value) == Some(rdf::RDF_TYPE)
                && row.get("o").and_then(rdf::term_value) == Some(class)
            {
                if let Some(subject) = row.get("s").and_then(rdf::term_value) {
                    typed.insert(subject.to_owned());
                }
            }
        }
        let mut best: BTreeMap<String, (u64, serde_json::Value)> = BTreeMap::new();
        for row in &triples {
            let Some(subject) = row.get("s").and_then(rdf::term_value) else {
                continue;
            };
            if !typed.contains(subject)
                || row.get("p").and_then(rdf::term_value) != Some(rdf::PAYLOAD)
            {
                continue;
            }
            let Some(text) = row.get("o").and_then(rdf::term_value) else {
                continue;
            };
            let Ok(mut value) = serde_json::from_str::<serde_json::Value>(text) else {
                continue;
            };
            let revision = value
                .get("_rev")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            if let serde_json::Value::Object(map) = &mut value {
                map.remove("_rev");
            }
            match best.get(subject) {
                Some((current, _)) if *current > revision => {}
                _ => {
                    best.insert(subject.to_owned(), (revision, value));
                }
            }
        }
        Ok(best.into_values().map(|(_, value)| value).collect())
    }

    async fn write_template(
        &self,
        template: &ProcessTemplate,
        expected_revision: u64,
        freeze_version: bool,
    ) -> Result<ProcessTemplate> {
        let current = self.load_template(&template.uri).await?;
        let current_revision = current.as_ref().map_or(0, |value| value.revision);
        if expected_revision != current_revision {
            return Err(ProcessError::Conflict {
                expected: expected_revision,
                current: current_revision,
            });
        }
        let mut stored = template.clone();
        stored.revision = current_revision.saturating_add(1).max(1);
        stored.updated_at = rdf::now_ms().max(stored.updated_at);
        let graph = crate::graphs::resolve_graph(PROCESS_GRAPH);
        rdf::upsert(
            &self.engine,
            &graph,
            &template_subject(&stored.uri),
            MM_PROCESS_TEMPLATE,
            Some(&stored.name),
            &serde_json::to_value(&stored)
                .map_err(|error| ProcessError::Invalid(error.to_string()))?,
        )
        .await
        .map_err(store_error)?;
        if freeze_version {
            let version = ProcessTemplateVersion {
                uri: template_version_subject(&stored.uri, stored.version),
                template_uri: stored.uri.clone(),
                version: stored.version,
                template: stored.clone(),
                created_at: rdf::now_ms(),
            };
            rdf::upsert(
                &self.engine,
                &graph,
                &version.uri,
                MM_PROCESS_TEMPLATE_VERSION,
                Some(&format!("{} v{}", stored.name, stored.version)),
                &serde_json::to_value(&version)
                    .map_err(|error| ProcessError::Invalid(error.to_string()))?,
            )
            .await
            .map_err(store_error)?;
        }
        Ok(stored)
    }
}

#[async_trait::async_trait]
impl ProcessStore for RdfProcessStore {
    async fn load_template(&self, uri: &str) -> Result<Option<ProcessTemplate>> {
        rdf::load_as(&self.engine, PROCESS_GRAPH, &template_subject(uri))
            .await
            .map_err(store_error)
    }

    async fn save_template(
        &self,
        template: &ProcessTemplate,
        expected_revision: u64,
    ) -> Result<ProcessTemplate> {
        self.write_template(template, expected_revision, false)
            .await
    }

    async fn list_templates(&self, include_deleted: bool) -> Result<Vec<ProcessTemplate>> {
        let mut templates: Vec<ProcessTemplate> = self
            .payloads(MM_PROCESS_TEMPLATE)
            .await?
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| ProcessError::Invalid(format!("template payload: {error}")))?;
        templates.retain(|template| include_deleted || !template.deleted);
        templates.sort_by(|a, b| a.uri.cmp(&b.uri).then(b.version.cmp(&a.version)));
        Ok(templates)
    }

    async fn save_template_version(
        &self,
        template: &ProcessTemplate,
        expected_revision: u64,
    ) -> Result<ProcessTemplate> {
        self.write_template(template, expected_revision, true).await
    }

    async fn load_template_version(
        &self,
        uri: &str,
        version: u32,
    ) -> Result<Option<ProcessTemplateVersion>> {
        rdf::load_as(
            &self.engine,
            PROCESS_GRAPH,
            &template_version_subject(uri, version),
        )
        .await
        .map_err(store_error)
    }

    async fn versions(&self, uri: &str) -> Result<Vec<ProcessTemplateVersion>> {
        let mut versions: Vec<ProcessTemplateVersion> =
            rdf::list_as(&self.engine, PROCESS_GRAPH, MM_PROCESS_TEMPLATE_VERSION)
                .await
                .map_err(store_error)?;
        versions.retain(|version| version.template_uri == uri);
        versions.sort_by_key(|version| version.version);
        Ok(versions)
    }

    async fn load_instance(&self, id: &str) -> Result<Option<ProcessInstance>> {
        let loaded: Option<ProcessInstance> =
            rdf::load_as(&self.engine, PROCESS_GRAPH, &instance_subject(id))
                .await
                .map_err(store_error)?;
        if let Some(instance) = &loaded {
            if let Ok(mut revisions) = self.revisions.lock() {
                revisions.insert(instance.id.clone(), instance.revision);
            }
        }
        Ok(loaded)
    }

    async fn save_instance(
        &self,
        instance: &ProcessInstance,
        expected_revision: u64,
    ) -> Result<ProcessInstance> {
        let cached = self
            .revisions
            .lock()
            .ok()
            .and_then(|revisions| revisions.get(&instance.id).copied());
        let current_revision = match cached {
            Some(revision) => revision,
            None => self
                .load_instance(&instance.id)
                .await?
                .map_or(0, |value| value.revision),
        };
        if expected_revision != current_revision {
            return Err(ProcessError::Conflict {
                expected: expected_revision,
                current: current_revision,
            });
        }
        let mut stored = instance.clone();
        stored.revision = current_revision.saturating_add(1).max(1);
        stored.updated_at = rdf::now_ms().max(stored.updated_at);
        let graph = crate::graphs::resolve_graph(PROCESS_GRAPH);
        rdf::upsert(
            &self.engine,
            &graph,
            &instance_subject(&stored.id),
            MM_PROCESS_INSTANCE,
            Some(&format!(
                "{} {}",
                stored.template_uri,
                stored.status.as_str()
            )),
            &serde_json::to_value(&stored)
                .map_err(|error| ProcessError::Invalid(error.to_string()))?,
        )
        .await
        .map_err(store_error)?;
        if let Ok(mut revisions) = self.revisions.lock() {
            revisions.insert(stored.id.clone(), stored.revision);
        }
        // Step executions are projections, one subject per step, so an inspector
        // can query them without decoding the whole instance.
        for (step_id, step) in &stored.steps {
            let subject = format!("{}/step/{}", instance_subject(&stored.id), step_id);
            let payload = serde_json::json!({
                "processId": stored.id,
                "stepId": step_id,
                "status": step.status,
                "attempts": step.attempts,
                "output": step.output,
                "error": step.error,
                "startedAt": step.started_at,
                "finishedAt": step.finished_at,
            });
            rdf::upsert(
                &self.engine,
                &graph,
                &subject,
                MM_STEP_EXECUTION,
                Some(step_id),
                &payload,
            )
            .await
            .map_err(store_error)?;
        }
        Ok(stored)
    }

    async fn list_instances(&self) -> Result<Vec<ProcessInstance>> {
        let mut instances: Vec<ProcessInstance> = self
            .payloads(MM_PROCESS_INSTANCE)
            .await?
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| ProcessError::Invalid(format!("instance payload: {error}")))?;
        instances.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        if let Ok(mut revisions) = self.revisions.lock() {
            for instance in &instances {
                revisions.insert(instance.id.clone(), instance.revision);
            }
        }
        Ok(instances)
    }

    async fn list_instances_for_session(&self, session_id: &str) -> Result<Vec<ProcessInstance>> {
        let mut instances = self.list_instances().await?;
        instances.retain(|instance| instance.session_id == session_id);
        Ok(instances)
    }
}

fn store_error(error: impl std::fmt::Display) -> ProcessError {
    ProcessError::Unavailable(error.to_string())
}

/// A template IRI from a caller-supplied id or IRI.
pub fn template_uri(id: &str) -> String {
    if id.starts_with("http") || id.starts_with("urn:") {
        id.to_owned()
    } else {
        format!("{MMD_NS}process/template/{id}")
    }
}

/// An instance IRI from a caller-supplied id or IRI.
pub fn instance_uri(id: &str) -> String {
    if id.starts_with("http") || id.starts_with("urn:") {
        id.to_owned()
    } else {
        format!("{MMD_NS}process/instance/{id}")
    }
}

fn template_subject(uri: &str) -> String {
    template_uri(uri)
}

fn instance_subject(id: &str) -> String {
    instance_uri(id)
}

fn template_version_subject(uri: &str, version: u32) -> String {
    format!("{}/version/{version}", template_uri(uri))
}

/// The class IRI for a template step execution.
pub fn step_class() -> String {
    MM_STEP_EXECUTION.to_owned()
}

/// The `mm:` namespace, re-exported for callers building predicates.
pub fn mm_namespace() -> &'static str {
    MM_NS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InitConfig;
    use crate::init::initialize_data_layer;

    async fn store() -> (RdfProcessStore, Arc<RdfEngine>) {
        let data = initialize_data_layer(InitConfig {
            in_memory: true,
            ..InitConfig::default()
        })
        .await
        .unwrap();
        (RdfProcessStore::new(Arc::clone(&data.engine)), data.engine)
    }

    #[tokio::test]
    async fn template_versions_are_frozen_and_revisioned() {
        let (store, _) = store().await;
        let mut template = ProcessTemplate::new(
            template_uri("t"),
            "T",
            vec![TemplateStep::new("a", "A", "do a")],
            1,
        );
        let saved = store.save_template(&template, 0).await.unwrap();
        assert_eq!(saved.revision, 1);
        template.version = 2;
        let saved = store.save_template_version(&template, 1).await.unwrap();
        assert_eq!(saved.version, 2);
        let versions = store.versions(&template.uri).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 2);
        assert_eq!(versions[0].template.version, 2);
        assert!(matches!(
            store.save_template(&template, 1).await,
            Err(ProcessError::Conflict { .. })
        ));
    }

    #[tokio::test]
    async fn instance_round_trips_and_projects_steps() {
        let (store, engine) = store().await;
        let template = ProcessTemplate::new(
            template_uri("t"),
            "T",
            vec![TemplateStep::new("a", "A", "do a")],
            1,
        );
        store.save_template(&template, 0).await.unwrap();
        let mut instance = ProcessInstance::new(instance_uri("p"), &template, "s", 5);
        instance.status = ProcessStatus::Running;
        instance.steps.get_mut("a").unwrap().status = StepStatus::Done;
        instance.steps.get_mut("a").unwrap().output = Some("ok".into());
        let saved = store.save_instance(&instance, 0).await.unwrap();
        assert_eq!(saved.revision, 1);
        let loaded = store.load_instance(&instance.id).await.unwrap().unwrap();
        assert_eq!(loaded.steps["a"].output.as_deref(), Some("ok"));
        assert_eq!(
            store.list_instances_for_session("s").await.unwrap().len(),
            1
        );
        let projected = crate::rdf::list_as::<serde_json::Value>(
            engine.as_ref(),
            PROCESS_GRAPH,
            MM_STEP_EXECUTION,
        )
        .await
        .unwrap();
        assert_eq!(projected.len(), 1);
    }
}
