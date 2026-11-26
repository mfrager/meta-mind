//! Inspector status view models.
//!
//! These are wire projections, not runtime state. They are deliberately
//! independent of route handlers so the Phase 0 shapes can be serde-tested and
//! reused by the UI and the status endpoint in later phases.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Prompt instance summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptStatusView {
    /// Instance id.
    pub id: String,
    /// Strategy.
    pub strategy: String,
    /// Variant.
    pub variant: Option<String>,
    /// Hash.
    pub hash: String,
    /// Fragment ids and versions.
    pub fragments: Vec<PromptFragmentView>,
}

/// One rendered fragment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptFragmentView {
    /// Fragment id.
    pub id: String,
    /// Version.
    pub version: u32,
    /// Character count, not prose repeated in the status panel.
    pub chars: usize,
}

/// Conversation state projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateStatusView {
    /// Revision.
    pub revision: u64,
    /// Topic.
    pub topic: Option<String>,
    /// Depth.
    pub depth: u8,
    /// Typed status.
    pub status: BTreeMap<String, Value>,
    /// Last update.
    pub updated_at: i64,
    /// Last writer.
    pub updated_by: String,
}

/// Session projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatusView {
    /// Session id.
    pub session_id: String,
    /// User id.
    pub user_id: String,
    /// Lifecycle.
    pub lifecycle: String,
    /// Presence.
    pub presence: String,
    /// Derived working.
    pub working: bool,
    /// Last activity.
    pub last_activity_at: i64,
    /// Last user input.
    #[serde(default)]
    pub last_user_input_at: Option<i64>,
    /// Last work activity.
    #[serde(default)]
    pub last_work_activity_at: Option<i64>,
    /// Explicit expiry.
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// Trace kinds.
    pub traces: Vec<String>,
    /// Number of trace manifests.
    #[serde(default)]
    pub trace_count: u32,
    /// Main trace id.
    #[serde(default)]
    pub main_trace_id: Option<String>,
    /// Session directory.
    pub session_dir: Option<String>,
    /// Resume epoch.
    pub resume_epoch: u32,
    /// Soft-delete instant.
    #[serde(default)]
    pub deleted_at: Option<i64>,
    /// Bounded work catalog, main first.
    #[serde(default)]
    pub work: Vec<WorkStatusView>,
}

/// One work-catalog row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkStatusView {
    /// Stable item id; `main` for the foreground item.
    pub id: String,
    /// Operator-facing title.
    pub title: String,
    /// Work kind.
    pub kind: String,
    /// Work status.
    pub status: String,
    /// Start instant.
    pub started_at: i64,
    /// Last activity instant.
    pub last_activity_at: i64,
}

/// Frame projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameStatusView {
    /// Frame id.
    pub id: String,
    /// Template id.
    pub template_id: String,
    /// Template version.
    pub template_version: u32,
    /// Kind.
    pub kind: String,
    /// Status.
    pub status: String,
    /// Entered at.
    pub entered_at: i64,
    /// Updated at.
    pub updated_at: i64,
    /// Expiry.
    pub expires_at: Option<i64>,
}

/// Frames projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FramesStatusView {
    /// Shared revision.
    pub revision: u64,
    /// Root first, top last.
    pub frames: Vec<FrameStatusView>,
    /// Top frame id, when the stack is non-empty.
    #[serde(default)]
    pub top_frame_id: Option<String>,
    /// Pending frame utterance, when one is scheduled.
    #[serde(default)]
    pub pending_utterance: Option<Value>,
}

/// Budget projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusView {
    /// Budget class.
    #[serde(default)]
    pub class: String,
    /// Tokens used.
    pub tokens_used: u64,
    /// Token cap.
    pub token_cap: u64,
    /// Turns used.
    pub turns_used: u32,
    /// Turn cap.
    pub turn_cap: u32,
    /// Whether the remaining values are estimates.
    #[serde(default)]
    pub estimated: bool,
}

/// Data/index projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataStatusView {
    /// Current data revision.
    pub data_revision: u64,
    /// Pending index docs.
    pub index_pending: u64,
    /// Index lag.
    pub index_lag_ms: u64,
    /// Active schemas.
    pub schemas: usize,
    /// Namespace bindings.
    pub namespaces: usize,
}

/// Plan row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStatusView {
    /// Plan id.
    pub plan_id: String,
    /// Goal.
    pub goal: String,
    /// Status.
    pub status: String,
    /// Revision.
    pub revision: u64,
    /// Current step.
    pub current_step: Option<String>,
    /// Step rows.
    pub steps: Vec<PlanStepStatusView>,
}

/// Plan step row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStepStatusView {
    /// Step id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Status.
    pub status: String,
    /// Attempts.
    pub attempts: u32,
    /// Runs.
    pub runs: u32,
}

/// Plans projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlansStatusView {
    /// Plans.
    pub items: Vec<PlanStatusView>,
}

/// Action row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionStatusView {
    /// Action id.
    pub action_id: String,
    /// Kind.
    pub kind: String,
    /// Status.
    pub status: String,
    /// Next due.
    pub due_at: Option<i64>,
    /// Last run.
    pub last_run_at: Option<i64>,
    /// Runs.
    pub runs: u32,
}

/// Actions projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionsStatusView {
    /// Scheduled rows.
    pub scheduled: Vec<ActionStatusView>,
    /// Recent receipts.
    pub recent: Vec<ActionStatusView>,
    /// Next due instant.
    pub next_run_at: Option<i64>,
}

/// Automation projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationStatusView {
    /// Supervisor state.
    pub supervisor: String,
    /// Tick seconds.
    pub tick_secs: u64,
    /// Active frame timers.
    pub frame_timers: usize,
    /// Next action.
    pub next_action_at: Option<i64>,
    /// Live supervisors in this process.
    #[serde(default)]
    pub sessions: usize,
    /// Scheduled/cron action rows.
    #[serde(default)]
    pub cron_jobs: usize,
    /// Last suppression reason, when the most recent decision suppressed.
    #[serde(default)]
    pub last_reason: Option<String>,
}

/// Interjections projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterjectionsStatusView {
    /// Effective policy id.
    pub policy_id: String,
    /// Whether the last candidate was suppressed.
    pub suppressed: bool,
    /// Last reason.
    pub reason: Option<String>,
    /// Queued items.
    pub queue: usize,
}

/// One agent run row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatusView {
    /// Run id.
    pub run_id: String,
    /// Backend kind.
    pub kind: String,
    /// Run status.
    pub status: String,
    /// Owning session.
    pub session_id: String,
    /// Last update.
    pub updated_at: i64,
    /// Whether the run is still active.
    pub active: bool,
}

/// One recent operation/log row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationStatusView {
    /// Stable row id.
    pub id: String,
    /// Kind/event type.
    pub kind: String,
    /// Source.
    pub source: String,
    /// Instant.
    pub at: i64,
    /// Compact payload.
    pub detail: Value,
}

/// The whole inspector status payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusView {
    /// Status-service view revision.
    #[serde(default)]
    pub revision: u64,
    /// Session id.
    #[serde(default)]
    pub session_id: String,
    /// Generation instant.
    #[serde(default)]
    pub generated_at: i64,
    /// Conversation-state revision.
    #[serde(default)]
    pub state_revision: u64,
    /// Data-layer revision.
    #[serde(default)]
    pub data_revision: u64,
    /// Conversation state.
    pub state: StateStatusView,
    /// Session.
    pub session: SessionStatusView,
    /// Frames.
    pub frames: FramesStatusView,
    /// Prompt.
    pub prompt: Option<PromptStatusView>,
    /// Budget.
    pub budget: BudgetStatusView,
    /// Data.
    pub data: DataStatusView,
    /// Plans.
    pub plans: PlansStatusView,
    /// Actions.
    pub actions: ActionsStatusView,
    /// Automation.
    pub automation: AutomationStatusView,
    /// Interjections.
    pub interjections: InterjectionsStatusView,
    /// Recent agent runs for this session.
    #[serde(default)]
    pub agents: Vec<AgentStatusView>,
    /// Recent operations for this session.
    #[serde(default)]
    pub operations: Vec<OperationStatusView>,
    /// Sections that could not be read; the rest of the panel still renders.
    #[serde(default)]
    pub unavailable: BTreeMap<String, String>,
    /// Update instant.
    pub updated_at: i64,
}

impl StatusView {
    /// A minimal empty status, useful for fixtures.
    pub fn empty(now_ms: i64) -> Self {
        Self {
            revision: 1,
            session_id: String::new(),
            generated_at: now_ms,
            state_revision: 1,
            data_revision: 0,
            state: StateStatusView {
                revision: 1,
                topic: None,
                depth: 0,
                status: BTreeMap::new(),
                updated_at: now_ms,
                updated_by: "system".into(),
            },
            session: SessionStatusView {
                session_id: String::new(),
                user_id: String::new(),
                lifecycle: "active".into(),
                presence: "absent".into(),
                working: false,
                last_activity_at: now_ms,
                last_user_input_at: None,
                last_work_activity_at: None,
                expires_at: None,
                traces: Vec::new(),
                trace_count: 0,
                main_trace_id: None,
                session_dir: None,
                resume_epoch: 0,
                deleted_at: None,
                work: Vec::new(),
            },
            frames: FramesStatusView {
                revision: 1,
                frames: Vec::new(),
                top_frame_id: None,
                pending_utterance: None,
            },
            prompt: None,
            budget: BudgetStatusView {
                class: "standard".into(),
                tokens_used: 0,
                token_cap: 0,
                turns_used: 0,
                turn_cap: 0,
                estimated: false,
            },
            data: DataStatusView {
                data_revision: 0,
                index_pending: 0,
                index_lag_ms: 0,
                schemas: 0,
                namespaces: 0,
            },
            plans: PlansStatusView { items: Vec::new() },
            actions: ActionsStatusView {
                scheduled: Vec::new(),
                recent: Vec::new(),
                next_run_at: None,
            },
            automation: AutomationStatusView {
                supervisor: "idle".into(),
                tick_secs: 30,
                frame_timers: 0,
                next_action_at: None,
                sessions: 0,
                cron_jobs: 0,
                last_reason: None,
            },
            interjections: InterjectionsStatusView {
                policy_id: "global.default".into(),
                suppressed: false,
                reason: None,
                queue: 0,
            },
            agents: Vec::new(),
            operations: Vec::new(),
            unavailable: BTreeMap::new(),
            updated_at: now_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_view_round_trips_camel_case() {
        let view = StatusView::empty(42);
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["session"]["sessionId"], "");
        assert_eq!(json["frames"]["revision"], 1);
        assert_eq!(json["automation"]["tickSecs"], 30);
        let back: StatusView = serde_json::from_value(json).unwrap();
        assert_eq!(back, view);
    }

    #[test]
    fn status_does_not_repeat_prompt_prose() {
        let view = StatusView::empty(1);
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("You are"));
        assert!(json.contains("\"state\""));
    }
}
