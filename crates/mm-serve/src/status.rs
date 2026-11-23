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
    /// Trace kinds.
    pub traces: Vec<String>,
    /// Session directory.
    pub session_dir: Option<String>,
    /// Resume epoch.
    pub resume_epoch: u32,
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
}

/// Budget projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatusView {
    /// Tokens used.
    pub tokens_used: u64,
    /// Token cap.
    pub token_cap: u64,
    /// Turns used.
    pub turns_used: u32,
    /// Turn cap.
    pub turn_cap: u32,
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

/// The whole inspector status payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusView {
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
    /// Update instant.
    pub updated_at: i64,
}

impl StatusView {
    /// A minimal empty status, useful for fixtures.
    pub fn empty(now_ms: i64) -> Self {
        Self {
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
                traces: Vec::new(),
                session_dir: None,
                resume_epoch: 0,
            },
            frames: FramesStatusView {
                revision: 1,
                frames: Vec::new(),
            },
            prompt: None,
            budget: BudgetStatusView {
                tokens_used: 0,
                token_cap: 0,
                turns_used: 0,
                turn_cap: 0,
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
            },
            interjections: InterjectionsStatusView {
                policy_id: "global.default".into(),
                suppressed: false,
                reason: None,
                queue: 0,
            },
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
