//! The global plan monitor.
//!
//! One loop reads the RDF plan registry, advances active plans whose
//! `mm:monitor` flag is true, and uses only [`ExternalWorkPort`] for external
//! work. Plan state is written back to the same plan graph; there is no side
//! table. CM and operator steps are surfaced, never executed unsolicited.

use std::collections::BTreeSet;
use std::sync::Arc;

use mm_agent::{
    AgentCall, AgentKind, AgentOrigin, AgentRunStatus, AgentTurnResult, ExternalWorkPort,
};
use mm_data::{Plan, PlanOwner, PlanStatus, PlanStep, PlanStepStatus, PlanStore, RdfPlanStore};
use serde::{Deserialize, Serialize};

use crate::state_service::StateService;

/// Bounded automatic retry policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum attempts per step.
    pub max_attempts: u32,
    /// First backoff in milliseconds; doubles per attempt.
    pub base_backoff_ms: i64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_backoff_ms: 15_000,
        }
    }
}

/// What the monitor did on one tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorDecision {
    /// Plan.
    pub plan_id: String,
    /// Step, when the decision names one.
    pub step_id: Option<String>,
    /// Stable action: `started`, `continued`, `completed`, `failed`, `retried`,
    /// `waiting_cm`, `waiting_operator`, or `skipped`.
    pub action: String,
    /// Operator-facing reason.
    pub detail: String,
    /// Decision instant.
    pub at: i64,
}

impl MonitorDecision {
    fn new(
        plan_id: &str,
        step_id: Option<&str>,
        action: &str,
        detail: impl Into<String>,
        at: i64,
    ) -> Self {
        Self {
            plan_id: plan_id.into(),
            step_id: step_id.map(str::to_owned),
            action: action.into(),
            detail: detail.into(),
            at,
        }
    }
}

/// The global monitor.
pub struct PlanMonitor {
    plans: Arc<RdfPlanStore>,
    state: Arc<StateService>,
    agent: Arc<dyn ExternalWorkPort>,
    retry: RetryPolicy,
}

impl std::fmt::Debug for PlanMonitor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlanMonitor")
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl PlanMonitor {
    /// Build the monitor.
    pub fn new(
        plans: Arc<RdfPlanStore>,
        state: Arc<StateService>,
        agent: Arc<dyn ExternalWorkPort>,
    ) -> Self {
        Self {
            plans,
            state,
            agent,
            retry: RetryPolicy::default(),
        }
    }

    /// Override retry bounds.
    #[must_use]
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// One global tick. Returns every decision, whether or not state changed.
    pub async fn tick(&self, now: i64) -> Vec<MonitorDecision> {
        let registry = match self.plans.registry().await {
            Ok(registry) => registry,
            Err(error) => {
                return vec![MonitorDecision::new(
                    "",
                    None,
                    "skipped",
                    format!("plan registry unavailable: {error}"),
                    now,
                )]
            }
        };
        let mut decisions = Vec::new();
        let mut seen = BTreeSet::new();
        for plan_id in registry.plans {
            if !seen.insert(plan_id.clone()) {
                continue;
            }
            match self.tick_plan(&plan_id, now).await {
                Ok(mut plan_decisions) => decisions.append(&mut plan_decisions),
                Err(error) => {
                    decisions.push(MonitorDecision::new(&plan_id, None, "skipped", error, now))
                }
            }
        }
        decisions
    }

    async fn tick_plan(&self, plan_id: &str, now: i64) -> Result<Vec<MonitorDecision>, String> {
        let Some(mut plan) = self
            .plans
            .load(plan_id)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(Vec::new());
        };
        if plan.status != PlanStatus::Active || !plan.monitor || plan.steps.is_empty() {
            return Ok(Vec::new());
        }
        if plan.all_steps_satisfied() {
            plan.status = PlanStatus::Completed;
            plan.current_step = None;
            plan.last_monitor_at = Some(now);
            plan.revision = plan.revision.saturating_add(1);
            plan.updated_at = now;
            self.save(&plan, plan_id).await?;
            return Ok(vec![MonitorDecision::new(
                plan_id,
                None,
                "completed",
                "all steps satisfied",
                now,
            )]);
        }
        let ready = plan
            .ready_steps()
            .into_iter()
            .find(|step| step.next_attempt_at.is_none_or(|at| at <= now))
            .map(|step| step.id.clone());
        let Some(step_id) = ready else {
            return Ok(Vec::new());
        };
        let Some(step) = plan.steps.iter().find(|step| step.id == step_id).cloned() else {
            return Ok(Vec::new());
        };
        match step.owner {
            PlanOwner::Cm => {
                plan.last_monitor_at = Some(now);
                plan.revision = plan.revision.saturating_add(1);
                plan.updated_at = now;
                if let Some(step) = plan.steps.iter_mut().find(|item| item.id == step_id) {
                    step.last_monitor_at = Some(now);
                    step.next_attempt_at = Some(now.saturating_add(self.retry.base_backoff_ms));
                }
                self.save(&plan, plan_id).await?;
                Ok(vec![MonitorDecision::new(
                    plan_id,
                    Some(&step_id),
                    "waiting_cm",
                    "CM-owned step waits for the next turn",
                    now,
                )])
            }
            PlanOwner::Operator => {
                plan.last_monitor_at = Some(now);
                plan.revision = plan.revision.saturating_add(1);
                plan.updated_at = now;
                if let Some(step) = plan.steps.iter_mut().find(|item| item.id == step_id) {
                    step.status = PlanStepStatus::Blocked;
                    step.last_error = Some("waiting for operator".into());
                    step.last_monitor_at = Some(now);
                }
                plan.current_step = None;
                self.save(&plan, plan_id).await?;
                Ok(vec![MonitorDecision::new(
                    plan_id,
                    Some(&step_id),
                    "waiting_operator",
                    "step needs an operator",
                    now,
                )])
            }
            PlanOwner::Action => Ok(vec![MonitorDecision::new(
                plan_id,
                Some(&step_id),
                "waiting_cm",
                "action-owned step waits for an action run",
                now,
            )]),
            PlanOwner::User => {
                plan.last_monitor_at = Some(now);
                plan.revision = plan.revision.saturating_add(1);
                plan.updated_at = now;
                if let Some(step) = plan.steps.iter_mut().find(|item| item.id == step_id) {
                    step.status = PlanStepStatus::Blocked;
                    step.last_error = Some("waiting for user".into());
                    step.last_monitor_at = Some(now);
                }
                plan.current_step = None;
                self.save(&plan, plan_id).await?;
                Ok(vec![MonitorDecision::new(
                    plan_id,
                    Some(&step_id),
                    "waiting_operator",
                    "step is owned by the user",
                    now,
                )])
            }
            PlanOwner::Agent => self.tick_agent_step(plan, step, now).await,
        }
    }

    async fn tick_agent_step(
        &self,
        mut plan: Plan,
        step: PlanStep,
        now: i64,
    ) -> Result<Vec<MonitorDecision>, String> {
        let bundle = self
            .state
            .load(&plan.session_id)
            .await
            .map_err(|error| format!("session bundle: {error}"))?;
        let plan_id = plan.id.clone();
        let step_id = step.id.clone();
        if let Some(run_id) = &step.agent_run_id {
            let status = self
                .agent
                .status(run_id)
                .await
                .map_err(|error| error.to_string())?;
            let Some(run) = status else {
                return self
                    .retry_or_fail(plan, &step, now, "agent run disappeared")
                    .await;
            };
            match run.status {
                AgentRunStatus::Done => {
                    let mut plan = plan;
                    plan.last_monitor_at = Some(now);
                    if let Some(step) = plan.steps.iter_mut().find(|item| item.id == step_id) {
                        step.status = PlanStepStatus::Done;
                        step.result = run.last_summary.clone();
                        step.finished_at = Some(now);
                        step.last_monitor_at = Some(now);
                    }
                    plan.current_step = None;
                    plan.revision = plan.revision.saturating_add(1);
                    plan.updated_at = now;
                    if plan.all_steps_satisfied() {
                        plan.status = PlanStatus::Completed;
                    }
                    self.save(&plan, &plan_id).await?;
                    Ok(vec![MonitorDecision::new(
                        &plan_id,
                        Some(&step_id),
                        "completed",
                        "agent run completed the step",
                        now,
                    )])
                }
                AgentRunStatus::Failed | AgentRunStatus::Cancelled => {
                    let reason = run
                        .last_error
                        .clone()
                        .unwrap_or_else(|| format!("agent run {}", run.status.label()));
                    self.retry_or_fail(plan, &step, now, &reason).await
                }
                AgentRunStatus::WaitingInput => {
                    plan.last_monitor_at = Some(now);
                    if let Some(step) = plan.steps.iter_mut().find(|item| item.id == step_id) {
                        step.status = PlanStepStatus::Blocked;
                        step.last_error = Some("waiting for operator input".into());
                        step.last_monitor_at = Some(now);
                    }
                    plan.current_step = None;
                    plan.revision = plan.revision.saturating_add(1);
                    plan.updated_at = now;
                    self.save(&plan, &plan_id).await?;
                    Ok(vec![MonitorDecision::new(
                        &plan_id,
                        Some(&step_id),
                        "waiting_operator",
                        "agent run is waiting for input",
                        now,
                    )])
                }
                AgentRunStatus::Waiting | AgentRunStatus::Running | AgentRunStatus::Queued => {
                    if run.status == AgentRunStatus::Waiting {
                        let result = self
                            .agent
                            .continue_run(
                                run_id,
                                "continue",
                                &bundle.session.session_id,
                                &bundle.session.user_id,
                            )
                            .await
                            .map_err(|error| error.to_string())?;
                        return self.apply_turn(plan, &step, result, now, "continued").await;
                    }
                    Ok(Vec::new())
                }
            }
        } else {
            let Some(mut started) = plan.steps.iter().find(|item| item.id == step_id).cloned()
            else {
                return Ok(Vec::new());
            };
            plan.start_step(&step_id, now)
                .map_err(|error| error.to_string())?;
            if let Some(step) = plan.steps.iter_mut().find(|item| item.id == step_id) {
                step.attempts = step.attempts.saturating_add(1);
                step.last_run_at = Some(now);
                step.last_monitor_at = Some(now);
                step.next_attempt_at = None;
                started = step.clone();
            }
            plan.last_monitor_at = Some(now);
            self.save(&plan, &plan_id).await?;
            let input = started
                .detail
                .clone()
                .unwrap_or_else(|| started.title.clone());
            let result = self
                .agent
                .run(AgentCall {
                    input,
                    run_id: None,
                    kind: AgentKind::Hermes,
                    workspace: None,
                    parent_session: plan.session_id.clone(),
                    user_id: bundle.session.user_id.clone(),
                    origin: AgentOrigin::PlanMonitor,
                })
                .await
                .map_err(|error| error.to_string())?;
            self.apply_turn(plan, &started, result, now, "started")
                .await
        }
    }

    async fn apply_turn(
        &self,
        mut plan: Plan,
        step: &PlanStep,
        result: AgentTurnResult,
        now: i64,
        action: &str,
    ) -> Result<Vec<MonitorDecision>, String> {
        plan.last_monitor_at = Some(now);
        match result.status {
            AgentRunStatus::Done => {
                if let Some(item) = plan.steps.iter_mut().find(|item| item.id == step.id) {
                    item.status = PlanStepStatus::Done;
                    item.result = Some(result.summary.clone());
                    item.finished_at = Some(now);
                    item.agent_run_id = Some(result.run_id.clone());
                    item.last_monitor_at = Some(now);
                }
                plan.current_step = None;
                plan.revision = plan.revision.saturating_add(1);
                plan.updated_at = now;
                if plan.all_steps_satisfied() {
                    plan.status = PlanStatus::Completed;
                }
                self.save(&plan, &plan.id).await?;
                Ok(vec![MonitorDecision::new(
                    &plan.id,
                    Some(&step.id),
                    "completed",
                    format!("{action}: {}", result.summary),
                    now,
                )])
            }
            AgentRunStatus::Failed | AgentRunStatus::Cancelled => {
                let reason = result
                    .error
                    .clone()
                    .unwrap_or_else(|| "agent turn failed".into());
                self.retry_or_fail(plan, step, now, &reason).await
            }
            AgentRunStatus::WaitingInput => {
                if let Some(item) = plan.steps.iter_mut().find(|item| item.id == step.id) {
                    item.status = PlanStepStatus::Blocked;
                    item.agent_run_id = Some(result.run_id.clone());
                    item.last_error = Some("waiting for operator input".into());
                    item.last_monitor_at = Some(now);
                }
                plan.current_step = None;
                plan.revision = plan.revision.saturating_add(1);
                plan.updated_at = now;
                self.save(&plan, &plan.id).await?;
                Ok(vec![MonitorDecision::new(
                    &plan.id,
                    Some(&step.id),
                    "waiting_operator",
                    result.message,
                    now,
                )])
            }
            AgentRunStatus::Waiting | AgentRunStatus::Running | AgentRunStatus::Queued => {
                if let Some(item) = plan.steps.iter_mut().find(|item| item.id == step.id) {
                    item.status = PlanStepStatus::Running;
                    item.agent_run_id = Some(result.run_id.clone());
                    item.runs = item.runs.saturating_add(1);
                    item.last_run_at = Some(now);
                    item.last_monitor_at = Some(now);
                }
                plan.revision = plan.revision.saturating_add(1);
                plan.updated_at = now;
                self.save(&plan, &plan.id).await?;
                Ok(vec![MonitorDecision::new(
                    &plan.id,
                    Some(&step.id),
                    action,
                    result.summary,
                    now,
                )])
            }
        }
    }

    async fn retry_or_fail(
        &self,
        mut plan: Plan,
        step: &PlanStep,
        now: i64,
        reason: &str,
    ) -> Result<Vec<MonitorDecision>, String> {
        let policy_error = is_policy_error(reason);
        let attempts = step.attempts.max(1);
        let retry = !policy_error && attempts < self.retry.max_attempts;
        plan.last_monitor_at = Some(now);
        if let Some(item) = plan.steps.iter_mut().find(|item| item.id == step.id) {
            item.last_error = Some(reason.to_owned());
            item.status = if retry {
                PlanStepStatus::Pending
            } else {
                PlanStepStatus::Failed
            };
            item.agent_run_id = None;
            item.finished_at = Some(now);
            item.last_monitor_at = Some(now);
            item.next_attempt_at = retry.then(|| {
                let factor = 1_i64 << attempts.saturating_sub(1).min(8);
                now.saturating_add(self.retry.base_backoff_ms.saturating_mul(factor))
            });
        }
        plan.current_step = None;
        if !retry {
            plan.status = PlanStatus::Failed;
        }
        plan.revision = plan.revision.saturating_add(1);
        plan.updated_at = now;
        self.save(&plan, &plan.id).await?;
        Ok(vec![MonitorDecision::new(
            &plan.id,
            Some(&step.id),
            if retry { "retried" } else { "failed" },
            reason,
            now,
        )])
    }

    async fn save(&self, plan: &Plan, expected: &str) -> Result<(), String> {
        self.plans
            .save(plan, plan.revision.saturating_sub(1))
            .await
            .map(|_| ())
            .map_err(|error| format!("save {expected}: {error}"))
    }
}

fn is_policy_error(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    reason.contains("confirm") || reason.contains("policy") || reason.contains("forbidden")
}
