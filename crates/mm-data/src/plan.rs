//! Durable plan contract for `metamind.plan`.
//!
//! A plan is an ordered, acyclic list of steps. Exactly one step is current at
//! a time, dependencies must be satisfied before a step can run, and every
//! update is revision-checked. The Phase 0 type is storage-neutral; the graph
//! representation and planner loop arrive later.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::engine::RdfEngine;
use crate::rdf::{self, iri, lit, next_revision, now_ms, typed_lit, MMD_NS, MM_NS, XSD_NS};

/// Maximum steps in one plan.
pub const MAX_PLAN_STEPS: usize = 64;
/// Maximum direct dependencies per step.
pub const MAX_STEP_DEPENDENCIES: usize = 8;
/// Maximum goal length.
pub const MAX_GOAL_CHARS: usize = 4_000;
/// Maximum plan body length.
pub const MAX_PLAN_BODY_CHARS: usize = 128_000;

/// The `plan` graph alias.
pub const PLAN_GRAPH: &str = "plan";
/// `mm:Plan`.
pub const MM_PLAN: &str = "https://metamind.dev/ontology#Plan";
/// `mm:PlanVersion`.
pub const MM_PLAN_VERSION: &str = "https://metamind.dev/ontology#PlanVersion";
/// `mm:PlanRegistry`.
pub const MM_PLAN_REGISTRY: &str = "https://metamind.dev/ontology#PlanRegistry";
/// The singleton registry IRI.
pub const PLAN_REGISTRY_IRI: &str = "https://metamind.dev/data/plan-registry";
/// The `mm:hasPlan` predicate.
pub const MM_HAS_PLAN: &str = "https://metamind.dev/ontology#hasPlan";
/// The `mm:currentVersion` predicate.
pub const MM_CURRENT_VERSION: &str = "https://metamind.dev/ontology#currentVersion";

/// How a plan is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanFormat {
    /// A markdown body.
    #[default]
    Markdown,
    /// An `mm.plan.v1` object.
    Structured,
}

impl PlanFormat {
    /// Parse the tool's `format` value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "markdown" => Some(Self::Markdown),
            "structured" => Some(Self::Structured),
            _ => None,
        }
    }
}

/// Who owns a plan or step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanOwner {
    /// The conversation manager.
    #[default]
    Cm,
    /// A delegated agent run.
    Agent,
    /// The operator.
    Operator,
    /// A scheduled internal action.
    Action,
    /// The user.
    User,
}

impl PlanOwner {
    /// Parse the tool value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "cm" => Some(Self::Cm),
            "agent" => Some(Self::Agent),
            "operator" => Some(Self::Operator),
            "action" => Some(Self::Action),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

/// Plan status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Editable, not started.
    Draft,
    /// Executing.
    Active,
    /// Waiting for a dependency or input.
    Paused,
    /// All steps complete.
    Done,
    /// Past its expiry without becoming durable.
    Expired,
    /// Archived by the operator; retained, not listed by default.
    Archived,
    /// Operator/system cancellation.
    Cancelled,
    /// A step failed.
    Failed,
    /// Stopped by a plan control; resumable.
    Stopped,
    /// Completed successfully.
    Completed,
    /// Abandoned without success.
    Abandoned,
}

impl PlanStatus {
    /// Whether the plan accepts step execution.
    pub fn is_runnable(self) -> bool {
        self == Self::Active
    }

    /// Whether the plan is finished.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done
                | Self::Cancelled
                | Self::Failed
                | Self::Completed
                | Self::Abandoned
                | Self::Expired
                | Self::Archived
        )
    }
}

/// Step status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    /// Not started.
    Pending,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Done,
    /// Failed.
    Failed,
    /// Skipped intentionally.
    Skipped,
    /// Waiting on a dependency/input.
    Blocked,
    /// Cancelled by a stop control.
    Cancelled,
}

impl PlanStepStatus {
    /// Whether the step satisfies a dependency.
    pub fn is_satisfied(self) -> bool {
        matches!(self, Self::Done | Self::Skipped)
    }

    /// Whether the step can be selected.
    pub fn is_runnable(self) -> bool {
        self == Self::Pending
    }
}

/// One plan step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    /// Stable step id.
    pub id: String,
    /// Short operator-facing title.
    pub title: String,
    /// Optional detail.
    pub detail: Option<String>,
    /// Current status.
    pub status: PlanStepStatus,
    /// Step ids that must be satisfied first.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Optional generic action name.
    pub action: Option<String>,
    /// Result/error summary.
    pub result: Option<String>,
    /// Execution attempts.
    #[serde(default)]
    pub attempts: u32,
    /// Completed runs.
    #[serde(default)]
    pub runs: u32,
    /// Last run instant.
    #[serde(default)]
    pub last_run_at: Option<i64>,
    /// Who must execute the step.
    #[serde(default)]
    pub owner: PlanOwner,
    /// How completion is verified.
    #[serde(default)]
    pub verification: Option<String>,
    /// Last error, preserved across retries.
    #[serde(default)]
    pub last_error: Option<String>,
    /// Agent run currently executing the step.
    #[serde(default)]
    pub agent_run_id: Option<String>,
    /// Step start instant.
    #[serde(default)]
    pub started_at: Option<i64>,
    /// Step finish instant.
    #[serde(default)]
    pub finished_at: Option<i64>,
    /// Earliest retry instant after a failed attempt.
    #[serde(default)]
    pub next_attempt_at: Option<i64>,
    /// Last monitor decision instant.
    #[serde(default)]
    pub last_monitor_at: Option<i64>,
}

impl PlanStep {
    /// A new pending step.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            detail: None,
            status: PlanStepStatus::Pending,
            depends_on: Vec::new(),
            action: None,
            result: None,
            attempts: 0,
            runs: 0,
            last_run_at: None,
            owner: PlanOwner::Cm,
            verification: None,
            last_error: None,
            agent_run_id: None,
            started_at: None,
            finished_at: None,
            next_attempt_at: None,
            last_monitor_at: None,
        }
    }
}

/// Cancellation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlanCancellation {
    /// Whether cancellation was requested.
    pub requested: bool,
    /// Who requested it.
    pub requested_by: Option<String>,
    /// Why.
    pub reason: Option<String>,
    /// Request instant.
    pub requested_at: Option<i64>,
}

/// A durable plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Plan id.
    pub id: String,
    /// Owning session.
    pub session_id: String,
    /// Goal statement.
    pub goal: String,
    /// Status.
    pub status: PlanStatus,
    /// Steps in intended order.
    pub steps: Vec<PlanStep>,
    /// Current step id.
    pub current_step: Option<String>,
    /// Optimistic revision.
    pub revision: u64,
    /// Creation instant.
    pub created_at: i64,
    /// Last update instant.
    pub updated_at: i64,
    /// Cancellation marker.
    #[serde(default)]
    pub cancellation: PlanCancellation,
    /// Final result summary.
    pub result: Option<String>,
    /// Markdown or structured.
    #[serde(default)]
    pub format: PlanFormat,
    /// The current version's body (markdown or canonical JSON).
    #[serde(default)]
    pub body: String,
    /// Current immutable version number.
    #[serde(default = "one")]
    pub current_version: u32,
    /// Plan owner.
    #[serde(default)]
    pub owner: PlanOwner,
    /// Whether the monitor may advance it.
    #[serde(default = "yes")]
    pub monitor: bool,
    /// Last plan-monitor decision instant.
    #[serde(default)]
    pub last_monitor_at: Option<i64>,
    /// One-line description shown in listings.
    #[serde(default)]
    pub description: Option<String>,
    /// The builder's `mm.plan.structure.v1` output.
    #[serde(default)]
    pub structure: Option<serde_json::Value>,
    /// Template URI compiled from this plan.
    #[serde(default)]
    pub process_uri: Option<String>,
    /// Expiry instant for a non-durable plan.
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// Whether the plan is exempt from expiry.
    #[serde(default)]
    pub never_expires: bool,
    /// Workspace-relative PROCESS.md attached to the compiled template.
    #[serde(default)]
    pub doc_path: Option<String>,
    /// Template this plan was derived from, when it was launched from one.
    #[serde(default)]
    pub derived_template_uri: Option<String>,
}

fn one() -> u32 {
    1
}

fn yes() -> bool {
    true
}

impl Plan {
    /// A draft plan with no steps.
    pub fn new(
        id: impl Into<String>,
        session_id: impl Into<String>,
        goal: impl Into<String>,
        now_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            session_id: session_id.into(),
            goal: goal.into(),
            status: PlanStatus::Draft,
            steps: Vec::new(),
            current_step: None,
            revision: 1,
            created_at: now_ms,
            updated_at: now_ms,
            cancellation: PlanCancellation::default(),
            result: None,
            format: PlanFormat::Markdown,
            body: String::new(),
            current_version: 1,
            owner: PlanOwner::Cm,
            monitor: true,
            last_monitor_at: None,
            description: None,
            structure: None,
            process_uri: None,
            expires_at: None,
            never_expires: false,
            doc_path: None,
            derived_template_uri: None,
        }
    }

    /// The current step, if any.
    pub fn current(&self) -> Option<&PlanStep> {
        let id = self.current_step.as_deref()?;
        self.steps.iter().find(|step| step.id == id)
    }

    /// Steps whose dependencies are satisfied and which are pending.
    pub fn ready_steps(&self) -> Vec<&PlanStep> {
        let done: BTreeSet<&str> = self
            .steps
            .iter()
            .filter(|step| step.status.is_satisfied())
            .map(|step| step.id.as_str())
            .collect();
        self.steps
            .iter()
            .filter(|step| {
                step.status.is_runnable()
                    && step
                        .depends_on
                        .iter()
                        .all(|dependency| done.contains(dependency.as_str()))
            })
            .collect()
    }

    /// Validate ids, statuses, dependencies, and single-current invariant.
    pub fn validate(&self) -> Result<(), PlanError> {
        if self.id.trim().is_empty() || self.session_id.trim().is_empty() {
            return Err(PlanError::Invalid(
                "plan id and session id must not be empty".into(),
            ));
        }
        if self.goal.trim().is_empty() || self.goal.chars().count() > MAX_GOAL_CHARS {
            return Err(PlanError::Invalid(format!(
                "goal must be 1–{MAX_GOAL_CHARS} characters"
            )));
        }
        if self.body.chars().count() > MAX_PLAN_BODY_CHARS {
            return Err(PlanError::Invalid("plan body exceeds its limit".into()));
        }
        if self.current_version == 0 {
            return Err(PlanError::Invalid("version must be positive".into()));
        }
        if self.steps.len() > MAX_PLAN_STEPS {
            return Err(PlanError::TooManySteps);
        }
        if self.revision == 0 {
            return Err(PlanError::Invalid("revision must be positive".into()));
        }
        let mut ids = BTreeSet::new();
        for step in &self.steps {
            if step.id.trim().is_empty() || !ids.insert(step.id.as_str()) {
                return Err(PlanError::Invalid(format!(
                    "step id {:?} is empty or duplicated",
                    step.id
                )));
            }
            if step.title.trim().is_empty() {
                return Err(PlanError::Invalid(format!(
                    "step {} has an empty title",
                    step.id
                )));
            }
            if step.depends_on.len() > MAX_STEP_DEPENDENCIES {
                return Err(PlanError::TooManyDependencies);
            }
            if step
                .depends_on
                .iter()
                .any(|dependency| dependency == &step.id)
            {
                return Err(PlanError::Cycle(step.id.clone()));
            }
            if step
                .depends_on
                .iter()
                .any(|dependency| !ids.contains(dependency.as_str()))
            {
                // This check can only see earlier ids; the second pass below
                // catches forward references and cycles.
                continue;
            }
        }
        let mut by_id = BTreeMap::new();
        for step in &self.steps {
            by_id.insert(step.id.as_str(), step);
        }
        for step in &self.steps {
            for dependency in &step.depends_on {
                if !by_id.contains_key(dependency.as_str()) {
                    return Err(PlanError::UnknownDependency {
                        step: step.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }
        if has_cycle(self) {
            return Err(PlanError::Cycle(self.id.clone()));
        }
        if self
            .current_step
            .as_deref()
            .is_some_and(|id| !by_id.contains_key(id))
        {
            return Err(PlanError::Invalid(
                "current_step does not name a step".into(),
            ));
        }
        if self.status.is_terminal() && self.current_step.is_some() {
            return Err(PlanError::Invalid(
                "terminal plan must not have a current step".into(),
            ));
        }
        Ok(())
    }

    /// Mark a step running and make it current.
    pub fn start_step(&mut self, step_id: &str, now_ms: i64) -> Result<(), PlanError> {
        if self.status != PlanStatus::Active {
            return Err(PlanError::Invalid("plan is not active".into()));
        }
        let ready = self
            .ready_steps()
            .into_iter()
            .any(|step| step.id == step_id);
        if !ready {
            return Err(PlanError::NotReady(step_id.to_owned()));
        }
        if let Some(previous) = self.current_step.as_deref() {
            if let Some(step) = self.steps.iter_mut().find(|step| step.id == previous) {
                if step.status == PlanStepStatus::Running {
                    step.status = PlanStepStatus::Pending;
                }
            }
        }
        let step = self
            .steps
            .iter_mut()
            .find(|step| step.id == step_id)
            .ok_or_else(|| PlanError::UnknownStep(step_id.to_owned()))?;
        step.status = PlanStepStatus::Running;
        step.started_at = Some(now_ms);
        self.current_step = Some(step_id.to_owned());
        self.touch(now_ms);
        Ok(())
    }

    /// Record a step result.
    pub fn finish_step(
        &mut self,
        step_id: &str,
        status: PlanStepStatus,
        result: Option<String>,
        now_ms: i64,
    ) -> Result<(), PlanError> {
        if !matches!(
            status,
            PlanStepStatus::Done
                | PlanStepStatus::Failed
                | PlanStepStatus::Skipped
                | PlanStepStatus::Blocked
        ) {
            return Err(PlanError::Invalid(
                "finish status must be done, failed, skipped, or blocked".into(),
            ));
        }
        let step = self
            .steps
            .iter_mut()
            .find(|step| step.id == step_id)
            .ok_or_else(|| PlanError::UnknownStep(step_id.to_owned()))?;
        step.status = status;
        step.result = result;
        if status == PlanStepStatus::Failed {
            step.last_error = step.result.clone();
        }
        step.finished_at = Some(now_ms);
        if self.current_step.as_deref() == Some(step_id) {
            self.current_step = None;
        }
        if status == PlanStepStatus::Failed {
            self.status = PlanStatus::Failed;
        }
        self.touch(now_ms);
        Ok(())
    }

    /// Whether every step is satisfied.
    pub fn all_steps_satisfied(&self) -> bool {
        self.steps.iter().all(|step| step.status.is_satisfied())
    }

    fn touch(&mut self, now_ms: i64) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = now_ms;
    }
}

fn has_cycle(plan: &Plan) -> bool {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit<'a>(id: &'a str, plan: &'a Plan, marks: &mut BTreeMap<&'a str, Mark>) -> bool {
        match marks.get(id) {
            Some(Mark::Visiting) => true,
            Some(Mark::Done) => false,
            None => {
                marks.insert(id, Mark::Visiting);
                let Some(step) = plan.steps.iter().find(|step| step.id == id) else {
                    return false;
                };
                for dependency in &step.depends_on {
                    if visit(dependency, plan, marks) {
                        return true;
                    }
                }
                marks.insert(id, Mark::Done);
                false
            }
        }
    }
    let mut marks = BTreeMap::new();
    plan.steps
        .iter()
        .any(|step| visit(step.id.as_str(), plan, &mut marks))
}

/// One plan-control operation, shared by `metamind.act` and the inspector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PlanControl {
    /// Start a draft/ready plan.
    Start,
    /// Pause a running plan.
    Pause,
    /// Resume a paused plan.
    Resume,
    /// Mark the current/plan complete.
    Complete,
    /// Advance to the next ready step.
    Advance,
    /// Retry a failed/blocked step.
    Retry {
        /// Step id.
        step_id: String,
    },
    /// Stop the plan; resumable.
    Stop,
    /// Cancel the plan.
    Cancel,
    /// Abandon the plan without success.
    Abandon,
}

/// A control request with optimistic concurrency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanControlRequest {
    /// Plan id.
    pub plan_id: String,
    /// Control.
    pub control: PlanControl,
    /// Expected plan revision.
    pub expected_revision: u64,
    /// Operator/audit note.
    pub note: Option<String>,
}

/// A control receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanControlReceipt {
    /// Plan id.
    pub plan_id: String,
    /// Applied control.
    pub control: PlanControl,
    /// Resulting status.
    pub status: PlanStatus,
    /// Resulting revision.
    pub revision: u64,
    /// Compact operator message.
    pub message: String,
}

/// In-memory registry contract used by later persistence-backed implementations.
#[derive(Debug, Clone, Default)]
pub struct PlanRegistry {
    plans: BTreeMap<String, Plan>,
}

impl PlanRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a plan after validation.
    pub fn upsert(&mut self, plan: Plan) -> Result<(), PlanError> {
        plan.validate()?;
        self.plans.insert(plan.id.clone(), plan);
        Ok(())
    }

    /// Look up a plan.
    pub fn get(&self, plan_id: &str) -> Option<&Plan> {
        self.plans.get(plan_id)
    }

    /// All plans sorted by id.
    pub fn plans(&self) -> impl Iterator<Item = &Plan> {
        self.plans.values()
    }

    /// Apply one control with a revision check.
    pub fn control(
        &mut self,
        request: &PlanControlRequest,
        now_ms: i64,
    ) -> Result<PlanControlReceipt, PlanError> {
        let plan = self
            .plans
            .get_mut(&request.plan_id)
            .ok_or_else(|| PlanError::Invalid(format!("unknown plan {}", request.plan_id)))?;
        if plan.revision != request.expected_revision {
            return Err(PlanError::Conflict {
                expected: request.expected_revision,
                current: plan.revision,
            });
        }
        match &request.control {
            PlanControl::Start => {
                if !matches!(
                    plan.status,
                    PlanStatus::Draft | PlanStatus::Paused | PlanStatus::Stopped
                ) {
                    return Err(PlanError::Invalid(
                        "plan cannot start from its current status".into(),
                    ));
                }
                plan.status = PlanStatus::Active;
                for step in &mut plan.steps {
                    if step.status == PlanStepStatus::Blocked {
                        step.status = PlanStepStatus::Pending;
                    }
                }
            }
            PlanControl::Pause => {
                if plan.status != PlanStatus::Active {
                    return Err(PlanError::Invalid("only an active plan can pause".into()));
                }
                plan.status = PlanStatus::Paused;
            }
            PlanControl::Resume => {
                if plan.status != PlanStatus::Paused {
                    return Err(PlanError::Invalid("only a paused plan can resume".into()));
                }
                plan.status = PlanStatus::Active;
            }
            PlanControl::Stop => {
                if !matches!(plan.status, PlanStatus::Active | PlanStatus::Paused) {
                    return Err(PlanError::Invalid(
                        "only an active or paused plan can stop".into(),
                    ));
                }
                plan.status = PlanStatus::Stopped;
                for step in &mut plan.steps {
                    if step.status == PlanStepStatus::Running {
                        step.status = PlanStepStatus::Cancelled;
                    }
                }
                plan.current_step = None;
            }
            PlanControl::Complete => {
                if !plan.all_steps_satisfied() {
                    return Err(PlanError::Invalid("plan has unsatisfied steps".into()));
                }
                plan.status = PlanStatus::Completed;
                plan.current_step = None;
                plan.result = request.note.clone();
            }
            PlanControl::Advance => {
                if plan.status != PlanStatus::Active {
                    return Err(PlanError::Invalid("plan is not active".into()));
                }
                let step = plan
                    .current_step
                    .clone()
                    .or_else(|| plan.ready_steps().first().map(|step| step.id.clone()));
                if let Some(step_id) = step {
                    let step = plan
                        .steps
                        .iter_mut()
                        .find(|step| step.id == step_id)
                        .ok_or_else(|| PlanError::UnknownStep(step_id.clone()))?;
                    step.status = PlanStepStatus::Done;
                    step.result = request.note.clone();
                    step.finished_at = Some(now_ms);
                    plan.current_step = None;
                    if plan.all_steps_satisfied() {
                        plan.status = PlanStatus::Done;
                    }
                } else if plan.all_steps_satisfied() {
                    plan.status = PlanStatus::Done;
                } else {
                    plan.status = PlanStatus::Paused;
                }
            }
            PlanControl::Retry { step_id } => {
                let step = plan
                    .steps
                    .iter_mut()
                    .find(|step| &step.id == step_id)
                    .ok_or_else(|| PlanError::UnknownStep(step_id.clone()))?;
                if !matches!(
                    step.status,
                    PlanStepStatus::Failed | PlanStepStatus::Blocked
                ) {
                    return Err(PlanError::Invalid(
                        "only a failed/blocked step can retry".into(),
                    ));
                }
                step.status = PlanStepStatus::Pending;
                step.attempts = step.attempts.saturating_add(1);
                step.finished_at = None;
                plan.status = PlanStatus::Active;
                plan.current_step = None;
            }
            PlanControl::Cancel => {
                plan.status = PlanStatus::Cancelled;
                plan.current_step = None;
                plan.cancellation.requested = true;
                plan.cancellation.requested_at = Some(now_ms);
                plan.cancellation.reason = request.note.clone();
            }
            PlanControl::Abandon => {
                if plan.status.is_terminal() {
                    return Err(PlanError::Invalid("plan is already terminal".into()));
                }
                plan.status = PlanStatus::Abandoned;
                plan.current_step = None;
                plan.result = request.note.clone();
            }
        }
        plan.revision = plan.revision.saturating_add(1);
        plan.updated_at = now_ms;
        let status = plan.status;
        let revision = plan.revision;
        Ok(PlanControlReceipt {
            plan_id: plan.id.clone(),
            control: request.control.clone(),
            status,
            revision,
            message: format!("plan {status:?} at revision {revision}"),
        })
    }
}

/// One immutable plan version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanVersion {
    /// Owning plan.
    pub plan_id: String,
    /// One-based version.
    pub version: u32,
    /// Markdown or structured.
    pub format: PlanFormat,
    /// Markdown body or canonical `mm.plan.v1` JSON.
    pub body: String,
    /// Structured steps (empty for markdown).
    #[serde(default)]
    pub steps: Vec<PlanStep>,
    /// Creation instant.
    pub created_at: i64,
    /// `cm`, `user`, or `action`.
    pub created_by: String,
}

impl PlanVersion {
    /// Validate the version envelope.
    pub fn validate(&self) -> Result<(), PlanError> {
        if self.plan_id.trim().is_empty() {
            return Err(PlanError::Invalid("version must name a plan".into()));
        }
        if self.version == 0 {
            return Err(PlanError::Invalid("version must be positive".into()));
        }
        if self.body.chars().count() > MAX_PLAN_BODY_CHARS {
            return Err(PlanError::Invalid("plan body exceeds its limit".into()));
        }
        if self.format == PlanFormat::Structured {
            parse_structured_plan(&self.body, &self.plan_id, 1)?;
        }
        Ok(())
    }
}

/// A parsed `mm.plan.v1` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredPlan {
    /// Schema id; must be `mm.plan.v1`.
    pub schema: String,
    /// Goal statement.
    pub goal: String,
    /// Assumptions.
    #[serde(default)]
    pub assumptions: Vec<String>,
    /// Steps.
    pub steps: Vec<StructuredStep>,
    /// Risks.
    #[serde(default)]
    pub risks: Vec<String>,
    /// Open questions.
    #[serde(default)]
    pub open_questions: Vec<String>,
    /// Success criteria.
    #[serde(default)]
    pub success_criteria: Vec<String>,
}

/// One step in a structured plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredStep {
    /// Stable id.
    pub id: String,
    /// Short title.
    pub title: String,
    /// Optional detail.
    #[serde(default)]
    pub detail: Option<String>,
    /// Dependencies.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Verification text.
    #[serde(default)]
    pub verification: Option<String>,
    /// Owner (`cm`, `agent`, `operator`).
    #[serde(default)]
    pub owner: Option<String>,
}

/// Parse and validate an `mm.plan.v1` body into executable steps.
pub fn parse_structured_plan(body: &str, plan_id: &str, revision: u64) -> Result<Plan, PlanError> {
    let structured: StructuredPlan = serde_json::from_str(body)
        .map_err(|error| PlanError::Invalid(format!("invalid structured plan: {error}")))?;
    if structured.schema != "mm.plan.v1" {
        return Err(PlanError::Invalid(
            "structured plan schema must be mm.plan.v1".into(),
        ));
    }
    let mut plan = Plan::new(plan_id, "unknown", structured.goal.clone(), now_ms());
    plan.format = PlanFormat::Structured;
    plan.body = body.to_owned();
    plan.revision = revision.max(1);
    plan.steps = structured
        .steps
        .into_iter()
        .map(|step| {
            let mut out = PlanStep::new(step.id, step.title);
            out.detail = step.detail;
            out.depends_on = step.depends_on;
            out.verification = step.verification;
            if let Some(owner) = step.owner.as_deref() {
                out.owner = PlanOwner::parse(owner)
                    .ok_or_else(|| PlanError::Invalid(format!("unknown step owner {owner:?}")))?;
            }
            Ok(out)
        })
        .collect::<Result<Vec<_>, PlanError>>()?;
    plan.validate()?;
    Ok(plan)
}

/// A validated plan write ready for storage.
pub struct PlanWrite {
    /// The plan header.
    pub plan: Plan,
    /// The new immutable version.
    pub version: PlanVersion,
    /// Revision the caller read before building this write.
    pub expected_revision: u64,
}

/// Persistence seam for plans. Phase 1 implements the RDF-backed store.
#[async_trait::async_trait]
pub trait PlanStore: Send + Sync {
    /// Load one plan.
    async fn load(&self, plan_id: &str) -> Result<Option<Plan>, PlanError>;
    /// Save one plan with optimistic concurrency.
    async fn save(&self, plan: &Plan, expected_revision: u64) -> Result<Plan, PlanError>;
    /// List plans for a session.
    async fn list(&self, session_id: &str) -> Result<Vec<Plan>, PlanError>;
    /// Every plan in the store, for lifecycle sweeps.
    async fn list_all(&self) -> Result<Vec<Plan>, PlanError>;
    /// Save a plan and one immutable version atomically.
    async fn save_version(&self, write: &PlanWrite) -> Result<Plan, PlanError>;
    /// Load one version.
    async fn load_version(
        &self,
        plan_id: &str,
        version: u32,
    ) -> Result<Option<PlanVersion>, PlanError>;
    /// Every version, oldest first.
    async fn versions(&self, plan_id: &str) -> Result<Vec<PlanVersion>, PlanError>;
    /// Apply one lifecycle control with revision checking.
    async fn control(&self, request: &PlanControlRequest) -> Result<PlanControlReceipt, PlanError>;
}

/// The RDF-backed plan store and registry writer.
#[derive(Debug, Clone)]
pub struct RdfPlanStore {
    engine: Arc<RdfEngine>,
}

impl RdfPlanStore {
    /// Build over the data layer's engine.
    pub fn new(engine: Arc<RdfEngine>) -> Self {
        Self { engine }
    }

    /// Load the singleton registry projection.
    pub async fn registry(&self) -> Result<PlanRegistryView, PlanError> {
        if let Some(mut payload) =
            rdf::load_as::<PlanRegistryView>(&self.engine, PLAN_GRAPH, PLAN_REGISTRY_IRI)
                .await
                .map_err(store_error)?
        {
            payload.plans.sort();
            return Ok(payload);
        }
        let mut plans: Vec<String> = self
            .list_plans()
            .await?
            .into_iter()
            .filter(|plan| !plan.status.is_terminal())
            .map(|plan| plan.id)
            .collect();
        plans.sort();
        Ok(PlanRegistryView { revision: 1, plans })
    }

    async fn write_plan(
        &self,
        plan: &Plan,
        expected_revision: u64,
        version: Option<&PlanVersion>,
    ) -> Result<Plan, PlanError> {
        plan.validate()?;
        let current = self.load(&plan.id).await?;
        let current_revision = current.as_ref().map_or(0, |value| value.revision);
        if expected_revision != current_revision {
            return Err(PlanError::Conflict {
                expected: expected_revision,
                current: current_revision,
            });
        }
        if let Some(version) = version {
            version.validate()?;
            if version.version != plan.current_version || version.plan_id != plan.id {
                return Err(PlanError::Invalid(
                    "version does not match the plan header".into(),
                ));
            }
        }

        let plan_graph = crate::graphs::resolve_graph(PLAN_GRAPH);
        let subject = plan_subject(&plan.id);
        let mut inserts = entity_quads(
            &plan_graph,
            &subject,
            MM_PLAN,
            Some(&plan.goal),
            &serde_json::to_value(plan).map_err(|error| PlanError::Invalid(error.to_string()))?,
        );
        inserts.push(RdfEngine::quad(
            &plan_graph,
            &subject,
            MM_CURRENT_VERSION,
            iri(plan_version_subject(&plan.id, plan.current_version)),
        ));
        let mut deletes = self
            .engine
            .store()
            .quads_for_subject(&plan_graph, &subject)
            .await
            .map_err(store_error)?;

        if let Some(version) = version {
            inserts.extend(entity_quads(
                &plan_graph,
                &plan_version_subject(&plan.id, version.version),
                MM_PLAN_VERSION,
                Some(&format!("{} v{}", plan.goal, version.version)),
                &serde_json::to_value(version)
                    .map_err(|error| PlanError::Invalid(error.to_string()))?,
            ));
        }

        let mut plans: BTreeSet<String> = self
            .list_plans()
            .await?
            .into_iter()
            .map(|existing| existing.id)
            .collect();
        if plan.status.is_terminal() && plan.status != PlanStatus::Stopped {
            plans.remove(&plan.id);
        } else {
            plans.insert(plan.id.clone());
        }
        let registry_revision = self
            .registry()
            .await
            .map(|registry| registry.revision.saturating_add(1))
            .unwrap_or(1);
        let registry = PlanRegistryView {
            revision: registry_revision,
            plans: plans.into_iter().collect(),
        };
        let registry_graph = crate::graphs::resolve_graph(PLAN_GRAPH);
        deletes.extend(
            self.engine
                .store()
                .quads_for_subject(&registry_graph, PLAN_REGISTRY_IRI)
                .await
                .map_err(store_error)?,
        );
        inserts.extend(entity_quads(
            &registry_graph,
            PLAN_REGISTRY_IRI,
            MM_PLAN_REGISTRY,
            Some("Plan registry"),
            &serde_json::to_value(&registry)
                .map_err(|error| PlanError::Invalid(error.to_string()))?,
        ));
        for plan_id in &registry.plans {
            inserts.push(RdfEngine::quad(
                &registry_graph,
                PLAN_REGISTRY_IRI,
                MM_HAS_PLAN,
                iri(plan_subject(plan_id)),
            ));
        }

        self.engine
            .store()
            .handle()
            .apply_batch(mm_store_graph::GraphBatch { deletes, inserts })
            .await
            .map_err(store_error)?;
        Ok(plan.clone())
    }

    async fn list_plans(&self) -> Result<Vec<Plan>, PlanError> {
        let mut plans: Vec<Plan> = rdf::list_as(&self.engine, PLAN_GRAPH, MM_PLAN)
            .await
            .map_err(store_error)?;
        plans.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(plans)
    }
}

/// The registry projection stored alongside the plan graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRegistryView {
    /// Registry revision.
    pub revision: u64,
    /// Current non-terminal plan ids.
    pub plans: Vec<String>,
}

pub(crate) fn plan_subject(plan_id: &str) -> String {
    if plan_id.starts_with("http") || plan_id.starts_with("urn:") {
        plan_id.to_owned()
    } else {
        format!("{MMD_NS}plan/{plan_id}")
    }
}

fn plan_version_subject(plan_id: &str, version: u32) -> String {
    format!("{}/version/{version}", plan_subject(plan_id))
}

pub(crate) fn entity_quads(
    graph: &str,
    subject: &str,
    class: &str,
    name: Option<&str>,
    payload: &serde_json::Value,
) -> Vec<oxrdf::Quad> {
    let mut stamped = payload.clone();
    if let serde_json::Value::Object(map) = &mut stamped {
        map.insert("_rev".to_owned(), serde_json::Value::from(next_revision()));
    }
    let mut quads = vec![
        RdfEngine::quad(graph, subject, rdf::RDF_TYPE, iri(class)),
        RdfEngine::quad(
            graph,
            subject,
            rdf::PAYLOAD,
            lit(serde_json::to_string(&stamped).unwrap_or_default()),
        ),
        RdfEngine::quad(
            graph,
            subject,
            rdf::REVISION,
            typed_lit(
                next_revision().to_string(),
                &format!("{XSD_NS}unsignedLong"),
            ),
        ),
        RdfEngine::quad(
            graph,
            subject,
            rdf::UPDATED_AT,
            typed_lit(now_ms().to_string(), &format!("{XSD_NS}dateTime")),
        ),
    ];
    if let Some(name) = name {
        quads.push(RdfEngine::quad(
            graph,
            subject,
            &format!("{MM_NS}name"),
            lit(name),
        ));
    }
    quads
}

fn store_error(error: impl std::fmt::Display) -> PlanError {
    PlanError::Unavailable(error.to_string())
}

#[async_trait::async_trait]
impl PlanStore for RdfPlanStore {
    async fn load(&self, plan_id: &str) -> Result<Option<Plan>, PlanError> {
        rdf::load_as(&self.engine, PLAN_GRAPH, &plan_subject(plan_id))
            .await
            .map_err(store_error)
    }

    async fn save(&self, plan: &Plan, expected_revision: u64) -> Result<Plan, PlanError> {
        self.write_plan(plan, expected_revision, None).await
    }

    async fn list(&self, session_id: &str) -> Result<Vec<Plan>, PlanError> {
        let mut plans = self.list_plans().await?;
        plans.retain(|plan| plan.session_id == session_id);
        Ok(plans)
    }

    async fn list_all(&self) -> Result<Vec<Plan>, PlanError> {
        self.list_plans().await
    }

    async fn save_version(&self, write: &PlanWrite) -> Result<Plan, PlanError> {
        self.write_plan(&write.plan, write.expected_revision, Some(&write.version))
            .await
    }

    async fn load_version(
        &self,
        plan_id: &str,
        version: u32,
    ) -> Result<Option<PlanVersion>, PlanError> {
        rdf::load_as(
            &self.engine,
            PLAN_GRAPH,
            &plan_version_subject(plan_id, version),
        )
        .await
        .map_err(store_error)
    }

    async fn versions(&self, plan_id: &str) -> Result<Vec<PlanVersion>, PlanError> {
        let mut versions: Vec<PlanVersion> =
            rdf::list_as(&self.engine, PLAN_GRAPH, MM_PLAN_VERSION)
                .await
                .map_err(store_error)?;
        versions.retain(|version| version.plan_id == plan_id);
        versions.sort_by_key(|version| version.version);
        Ok(versions)
    }

    async fn control(&self, request: &PlanControlRequest) -> Result<PlanControlReceipt, PlanError> {
        let plan = self
            .load(&request.plan_id)
            .await?
            .ok_or_else(|| PlanError::NotFound(request.plan_id.clone()))?;
        let mut registry = PlanRegistry::new();
        registry.upsert(plan)?;
        let receipt = registry.control(request, now_ms())?;
        let updated = registry
            .get(&request.plan_id)
            .cloned()
            .ok_or_else(|| PlanError::Invalid("plan disappeared from the registry".into()))?;
        self.write_plan(&updated, request.expected_revision, None)
            .await?;
        Ok(receipt)
    }
}

/// Plan failures.
#[derive(Debug, Error)]
pub enum PlanError {
    /// Invalid field/invariant.
    #[error("invalid plan: {0}")]
    Invalid(String),
    /// Step or plan confusion.
    #[error("unknown step {0}")]
    UnknownStep(String),
    /// Plan missing.
    #[error("unknown plan {0}")]
    NotFound(String),
    /// A dependency is missing.
    #[error("step {step} depends on unknown step {dependency}")]
    UnknownDependency {
        /// Dependent step.
        step: String,
        /// Missing dependency.
        dependency: String,
    },
    /// A step cannot run yet.
    #[error("step {0} is not ready")]
    NotReady(String),
    /// Cycle detected.
    #[error("plan graph contains a cycle involving {0}")]
    Cycle(String),
    /// Step count cap.
    #[error("plan exceeds the step limit")]
    TooManySteps,
    /// Dependency cap.
    #[error("step exceeds the dependency limit")]
    TooManyDependencies,
    /// Optimistic revision mismatch.
    #[error("conflict: expected revision {expected}, current revision {current}")]
    Conflict {
        /// Expected revision.
        expected: u64,
        /// Current revision.
        current: u64,
    },
    /// Persistence unavailable.
    #[error("plan store unavailable: {0}")]
    Unavailable(String),
}

/// Result alias for plans.
pub type Result<T, E = PlanError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        let mut plan = Plan::new("p", "s", "fix refresh", 1);
        plan.status = PlanStatus::Active;
        plan.steps.push(PlanStep::new("repro", "Reproduce"));
        let mut fix = PlanStep::new("fix", "Fix");
        fix.depends_on.push("repro".into());
        plan.steps.push(fix);
        plan
    }

    #[test]
    fn dependencies_gate_step_start() {
        let mut plan = plan();
        assert!(plan.start_step("fix", 2).is_err());
        plan.start_step("repro", 2).unwrap();
        plan.finish_step("repro", PlanStepStatus::Done, None, 3)
            .unwrap();
        plan.start_step("fix", 4).unwrap();
        assert_eq!(plan.current_step.as_deref(), Some("fix"));
    }

    #[test]
    fn cycle_is_rejected() {
        let mut plan = plan();
        plan.steps[0].depends_on.push("fix".into());
        assert!(matches!(plan.validate(), Err(PlanError::Cycle(_))));
    }

    #[test]
    fn unknown_dependency_is_rejected() {
        let mut plan = plan();
        plan.steps[1].depends_on.push("missing".into());
        assert!(matches!(
            plan.validate(),
            Err(PlanError::UnknownDependency { .. })
        ));
    }

    #[test]
    fn failed_step_fails_the_plan() {
        let mut plan = plan();
        plan.start_step("repro", 2).unwrap();
        plan.finish_step(
            "repro",
            PlanStepStatus::Failed,
            Some("cannot reproduce".into()),
            3,
        )
        .unwrap();
        assert_eq!(plan.status, PlanStatus::Failed);
        assert_eq!(plan.current_step, None);
    }
}
