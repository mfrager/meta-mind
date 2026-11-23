//! Phase 9: one supervisor per session, generic action suppression/delivery,
//! frame timing, and plan-monitor advancement through `ExternalWorkPort`.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const NOON: i64 = 12 * 3_600_000;

use async_trait::async_trait;
use mm_agent::{AgentCall, AgentRun, AgentRunStatus, AgentTurnResult, ExternalWorkPort};
use mm_cme::FrameEngine;
use mm_data::{
    ActionStore, FormValues, InternalActionRegistry, Plan, PlanOwner, PlanStatus, PlanStep,
    PlanStepStatus, PlanStore, RdfLogService, RdfPlanStore, Recurrence, ScheduleSpec,
    ScheduledAction, ScheduledActionSpec, ScheduledActionStatus, SessionBundle, WorkStatus,
};
use mm_prompt::PromptStore;
use mm_serve::automation::{
    ActionExecutor, AutomationConfig, AutomationDeps, AutomationManager, FrameExecutor,
};
use mm_serve::interjection::InterjectionRouter;
use mm_serve::plan_monitor::PlanMonitor;
use mm_serve::state_service::StateService;

struct Fixture {
    state: Arc<StateService>,
    actions: Arc<ActionStore>,
    engine: Arc<mm_data::RdfEngine>,
    prompts: Arc<PromptStore>,
    _temp: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let data = mm_data::initialize_data_layer(mm_data::InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..mm_data::InitConfig::default()
    })
    .await
    .unwrap();
    let prompts = Arc::new(
        PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("prompt tree"),
    );
    let engine = Arc::clone(&data.engine);
    let state = Arc::new(
        StateService::new(
            Arc::clone(&engine),
            Arc::clone(&data.agentstream),
            Arc::clone(&prompts),
            data.base_dir.clone(),
        )
        .unwrap(),
    );
    let bundle = state
        .open_session_with_id("s", "u", None, 1_000)
        .await
        .unwrap();
    state
        .push_frame("s", "frame.work", bundle.frames.revision, 1_000)
        .await
        .unwrap();
    let actions = Arc::new(ActionStore::new(
        Arc::clone(&engine),
        Arc::new(InternalActionRegistry::builtin()),
    ));
    Fixture {
        state,
        actions,
        engine,
        prompts,
        _temp: temp,
    }
}

struct FakeAction {
    calls: AtomicUsize,
    text: &'static str,
}

#[async_trait]
impl ActionExecutor for FakeAction {
    async fn execute(&self, _action: &ScheduledAction) -> Result<String, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.text.into())
    }
}

struct FakeFrame;

#[async_trait]
impl FrameExecutor for FakeFrame {
    async fn execute(
        &self,
        _session_id: &str,
        _frame: &mm_data::FrameUtterance,
        _bundle: &SessionBundle,
    ) -> Result<String, String> {
        Ok("frame line".into())
    }
}

fn deps(fixture: &Fixture, action: Arc<FakeAction>, frames: bool) -> AutomationDeps {
    AutomationDeps {
        state: Arc::clone(&fixture.state),
        actions: Arc::clone(&fixture.actions),
        logs: Arc::new(RdfLogService::new(Arc::clone(&fixture.engine))),
        frames: FrameEngine::new(Arc::clone(&fixture.prompts)),
        action_executor: action,
        frame_executor: Arc::new(FakeFrame),
        interjections: Arc::new(InterjectionRouter::defaults().unwrap()),
        config: AutomationConfig {
            frames: mm_serve::automation::FramesConfig {
                enabled: frames,
                ..Default::default()
            },
            ..Default::default()
        },
        disabled_kinds: BTreeSet::new(),
    }
}

async fn schedule_due(fixture: &Fixture, now: i64) {
    fixture
        .actions
        .schedule(
            ScheduledActionSpec {
                session_id: "s".into(),
                kind: "joke".into(),
                input: Some("a short aside".into()),
                parameters: FormValues::new(),
                schedule: ScheduleSpec::Delay { delay_ms: 1 },
                idempotency_key: format!("due-{now}"),
                status: ScheduledActionStatus::Scheduled,
                created_at: now,
                last_run_at: None,
                runs: 0,
            },
            Recurrence::Once,
            None,
            now,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn repeated_ensure_returns_the_same_supervisor() {
    let fixture = fixture().await;
    let action = Arc::new(FakeAction {
        calls: AtomicUsize::new(0),
        text: "aside",
    });
    let manager = Arc::new(AutomationManager::new(deps(&fixture, action, false)));
    assert!(manager.ensure("s").await);
    assert!(!manager.ensure("s").await);
    assert_eq!(manager.session_ids(), vec!["s".to_owned()]);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(fixture
        .state
        .load("s")
        .await
        .unwrap()
        .session
        .pending_utterance
        .is_some());
    manager.stop_all();
    assert!(manager.session_ids().is_empty());
}

#[tokio::test]
async fn working_suppresses_the_occurrence_without_executing_it() {
    let fixture = fixture().await;
    let action = Arc::new(FakeAction {
        calls: AtomicUsize::new(0),
        text: "aside",
    });
    let manager = AutomationManager::new(deps(&fixture, Arc::clone(&action), false));
    let bundle = fixture.state.load("s").await.unwrap();
    fixture
        .state
        .set_work_status(
            "s",
            "main",
            WorkStatus::Working,
            bundle.session.revision,
            NOON + 1,
        )
        .await
        .unwrap();
    schedule_due(&fixture, NOON + 1).await;
    let events = manager.tick("s", NOON + 10).await.unwrap();
    assert!(events
        .iter()
        .any(|event| event.kind == "suppressed" && event.detail.contains("working")));
    assert_eq!(action.calls.load(Ordering::SeqCst), 0);
    let actions = fixture.actions.list("s").await.unwrap();
    let due = actions
        .iter()
        .find(|row| row.spec.idempotency_key == format!("due-{}", NOON + 1))
        .unwrap();
    assert_eq!(due.spec.status, ScheduledActionStatus::Done);
}

#[tokio::test]
async fn a_due_frame_utterance_is_delivered_and_cleared() {
    let fixture = fixture().await;
    let action = Arc::new(FakeAction {
        calls: AtomicUsize::new(0),
        text: "unused",
    });
    let manager = AutomationManager::new(deps(&fixture, action, true));
    manager.seed("s", NOON).await.unwrap();
    let events = manager.tick("s", NOON + 10_000).await.unwrap();
    assert!(events
        .iter()
        .any(|event| event.kind == "frame" && event.text == "frame line"));
    let pending = fixture
        .state
        .load("s")
        .await
        .unwrap()
        .session
        .pending_utterance;
    assert!(pending.is_none_or(|utterance| utterance.due_at > NOON + 10_000));
}

#[tokio::test]
async fn idle_delivers_one_action_and_logs_it() {
    let fixture = fixture().await;
    let action = Arc::new(FakeAction {
        calls: AtomicUsize::new(0),
        text: "A short, original aside.",
    });
    let manager = AutomationManager::new(deps(&fixture, Arc::clone(&action), false));
    schedule_due(&fixture, NOON + 1).await;
    let events = manager.tick("s", NOON + 10).await.unwrap();
    assert!(events
        .iter()
        .any(|event| event.kind == "action" && event.text.contains("original")));
    assert_eq!(action.calls.load(Ordering::SeqCst), 1);
    let logs = RdfLogService::new(Arc::clone(&fixture.engine))
        .entries("s")
        .await
        .unwrap();
    assert!(logs.iter().any(|entry| entry.message.contains("original")));
}

struct DoneWork {
    calls: AtomicUsize,
}

#[async_trait]
impl ExternalWorkPort for DoneWork {
    async fn run(&self, call: AgentCall) -> mm_agent::Result<AgentTurnResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut result = AgentTurnResult::running("run-1");
        result.status = AgentRunStatus::Done;
        result.summary = format!("finished {}", call.input);
        result.message = "done".into();
        Ok(result)
    }

    async fn continue_run(
        &self,
        _run_id: &str,
        _input: &str,
        _parent_session: &str,
        _user_id: &str,
    ) -> mm_agent::Result<AgentTurnResult> {
        let mut result = AgentTurnResult::running("run-1");
        result.status = AgentRunStatus::Done;
        result.summary = "continued".into();
        Ok(result)
    }

    async fn status(&self, _run_id: &str) -> mm_agent::Result<Option<AgentRun>> {
        Ok(None)
    }
}

#[tokio::test]
async fn plan_monitor_runs_an_agent_step_and_completes_the_plan() {
    let fixture = fixture().await;
    let plans = Arc::new(RdfPlanStore::new(Arc::clone(&fixture.engine)));
    let mut plan = Plan::new("p1", "s", "fix the flake", 1_000);
    plan.status = PlanStatus::Active;
    let mut step = PlanStep::new("s1", "reproduce");
    step.owner = PlanOwner::Agent;
    step.detail = Some("reproduce the failing test".into());
    plan.steps.push(step);
    plans.save(&plan, 0).await.unwrap();
    let work = Arc::new(DoneWork {
        calls: AtomicUsize::new(0),
    });
    let monitor = PlanMonitor::new(Arc::clone(&plans), Arc::clone(&fixture.state), work.clone());
    let decisions = monitor.tick(2_000).await;
    assert!(decisions
        .iter()
        .any(|decision| decision.action == "completed"));
    assert_eq!(work.calls.load(Ordering::SeqCst), 1);
    let loaded = plans.load("p1").await.unwrap().unwrap();
    assert_eq!(loaded.status, PlanStatus::Completed);
    assert_eq!(loaded.steps[0].status, PlanStepStatus::Done);
    assert_eq!(loaded.steps[0].agent_run_id.as_deref(), Some("run-1"));
}

struct FlakyWork {
    calls: AtomicUsize,
}

#[async_trait]
impl ExternalWorkPort for FlakyWork {
    async fn run(&self, _call: AgentCall) -> mm_agent::Result<AgentTurnResult> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let mut result = AgentTurnResult::running("run-flaky");
        if call == 0 {
            result.status = AgentRunStatus::Failed;
            result.error = Some("verification failed".into());
        } else {
            result.status = AgentRunStatus::Done;
            result.summary = "verified".into();
        }
        Ok(result)
    }

    async fn continue_run(
        &self,
        _run_id: &str,
        _input: &str,
        _parent_session: &str,
        _user_id: &str,
    ) -> mm_agent::Result<AgentTurnResult> {
        let mut result = AgentTurnResult::running("run-flaky");
        result.status = AgentRunStatus::Done;
        Ok(result)
    }

    async fn status(&self, _run_id: &str) -> mm_agent::Result<Option<AgentRun>> {
        Ok(None)
    }
}

#[tokio::test]
async fn a_failed_step_retries_after_backoff_then_completes() {
    let fixture = fixture().await;
    let plans = Arc::new(RdfPlanStore::new(Arc::clone(&fixture.engine)));
    let mut plan = Plan::new("p3", "s", "retry", 1_000);
    plan.status = PlanStatus::Active;
    let mut step = PlanStep::new("s1", "verify");
    step.owner = PlanOwner::Agent;
    plan.steps.push(step);
    plans.save(&plan, 0).await.unwrap();
    let work = Arc::new(FlakyWork {
        calls: AtomicUsize::new(0),
    });
    let monitor = PlanMonitor::new(Arc::clone(&plans), Arc::clone(&fixture.state), work.clone())
        .with_retry(mm_serve::plan_monitor::RetryPolicy {
            max_attempts: 3,
            base_backoff_ms: 100,
        });
    let decisions = monitor.tick(2_000).await;
    assert!(decisions
        .iter()
        .any(|decision| decision.action == "retried"));
    let after_failure = plans.load("p3").await.unwrap().unwrap();
    assert_eq!(after_failure.status, PlanStatus::Active);
    let retry_at = after_failure.steps[0].next_attempt_at.unwrap();
    assert!(retry_at > 2_000);
    let decisions = monitor.tick(retry_at).await;
    assert!(decisions
        .iter()
        .any(|decision| decision.action == "completed"));
    let loaded = plans.load("p3").await.unwrap().unwrap();
    assert_eq!(loaded.status, PlanStatus::Completed);
    assert_eq!(work.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn plan_monitor_waits_for_cm_owned_steps() {
    let fixture = fixture().await;
    let plans = Arc::new(RdfPlanStore::new(Arc::clone(&fixture.engine)));
    let mut plan = Plan::new("p2", "s", "ask the operator", 1_000);
    plan.status = PlanStatus::Active;
    let step = PlanStep::new("s1", "answer the question");
    plan.steps.push(step);
    plans.save(&plan, 0).await.unwrap();
    let work = Arc::new(DoneWork {
        calls: AtomicUsize::new(0),
    });
    let monitor = PlanMonitor::new(Arc::clone(&plans), Arc::clone(&fixture.state), work.clone());
    let decisions = monitor.tick(2_000).await;
    assert!(decisions
        .iter()
        .any(|decision| decision.action == "waiting_cm"));
    assert_eq!(work.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn user_workspaces_are_created_and_escapes_are_refused() {
    use mm_agent::{FilesystemWorkspaces, UserWorkspaceResolver};
    let temp = tempfile::tempdir().unwrap();
    let workspaces = FilesystemWorkspaces::new(temp.path());
    let root = workspaces.resolve("u1").unwrap();
    assert!(root.ends_with("workspace/u1"));
    assert!(root.is_dir());
    assert!(workspaces
        .confine(&root, std::path::Path::new("../outside"))
        .is_err());
    assert!(workspaces
        .resolve_call("u1", Some(std::path::Path::new("/tmp/outside")))
        .is_err());
}

#[test]
fn automation_config_parses_the_checked_in_file() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/automation.toml");
    let config = AutomationConfig::load(&path);
    assert!(config.supervisor.enabled);
    assert!(config.actions.enabled);
    assert!(config.frames.enabled);
}
