//! Durable plan contract for `metamind.plan`.
//!
//! A plan is an ordered, acyclic list of steps. Exactly one step is current at
//! a time, dependencies must be satisfied before a step can run, and every
//! update is revision-checked. The Phase 0 type is storage-neutral; the graph
//! representation and planner loop arrive later.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum steps in one plan.
pub const MAX_PLAN_STEPS: usize = 64;
/// Maximum direct dependencies per step.
pub const MAX_STEP_DEPENDENCIES: usize = 8;
/// Maximum goal length.
pub const MAX_GOAL_CHARS: usize = 500;

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
    /// Operator/system cancellation.
    Cancelled,
    /// A step failed.
    Failed,
}

impl PlanStatus {
    /// Whether the plan accepts step execution.
    pub fn is_runnable(self) -> bool {
        self == Self::Active
    }

    /// Whether the plan is finished.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::Failed)
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
                if !matches!(plan.status, PlanStatus::Draft | PlanStatus::Paused) {
                    return Err(PlanError::Invalid(
                        "plan cannot start from its current status".into(),
                    ));
                }
                plan.status = PlanStatus::Active;
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
            PlanControl::Complete => {
                if !plan.all_steps_satisfied() {
                    return Err(PlanError::Invalid("plan has unsatisfied steps".into()));
                }
                plan.status = PlanStatus::Done;
                plan.current_step = None;
                plan.result = request.note.clone();
            }
            PlanControl::Advance => {
                if plan.status != PlanStatus::Active {
                    return Err(PlanError::Invalid("plan is not active".into()));
                }
                if let Some(step) = plan.ready_steps().first().map(|step| step.id.clone()) {
                    plan.start_step(&step, now_ms)?;
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
                plan.status = PlanStatus::Failed;
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

/// Persistence seam for plans. Phase 1 implements the RDF-backed store.
#[async_trait::async_trait]
pub trait PlanStore: Send + Sync {
    /// Load one plan.
    async fn load(&self, plan_id: &str) -> Result<Option<Plan>, PlanError>;
    /// Save one plan with optimistic concurrency.
    async fn save(&self, plan: &Plan, expected_revision: u64) -> Result<Plan, PlanError>;
    /// List plans for a session.
    async fn list(&self, session_id: &str) -> Result<Vec<Plan>, PlanError>;
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
