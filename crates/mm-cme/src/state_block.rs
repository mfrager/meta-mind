//! Canonical `[STATE]` construction.
//!
//! The block is a deterministic projection, not a second state store. It reads
//! the conversation/session/frame bundle plus bounded service projections,
//! serializes one minified JSON object, and refuses to truncate: a block that
//! exceeds the cap is a bug and returns an error instead of a plausible but
//! incomplete prompt.

use std::collections::BTreeMap;

use mm_data::{ConversationState, FrameStack, SessionBundle, SessionState, StatusValue};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum canonical `[STATE]` size, including the header line.
pub const MAX_STATE_BLOCK_BYTES: usize = 8 * 1024;
/// Maximum work items shown.
pub const MAX_WORK_ITEMS: usize = 8;
/// Maximum pending items shown.
pub const MAX_PENDING_ITEMS: usize = 8;
/// Maximum active agents shown.
pub const MAX_ACTIVE_AGENTS: usize = 3;
/// Maximum recent operations shown.
pub const MAX_OPS: usize = 10;
/// Maximum active schema prefixes shown.
pub const MAX_SCHEMAS: usize = 8;
/// Maximum frame labels shown.
pub const MAX_FRAME_LABELS: usize = 8;

/// The current user projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserProjection {
    /// Alias or stable IRI (`mmd:current-user`).
    pub iri: String,
    /// Canonical entity IRI.
    pub entity: String,
    /// Preferred name.
    pub name: String,
    /// Workspace label.
    pub workspace: String,
}

/// One work item in the session projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkProjection {
    /// Stable id.
    pub id: String,
    /// Status name.
    pub status: String,
    /// Operator-facing title.
    pub title: String,
}

/// Session lifecycle, presence, and bounded work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionProjection {
    /// Lifecycle name.
    pub lifecycle: String,
    /// Presence name.
    pub presence: String,
    /// Derived from the work catalog, never model-set.
    pub working: bool,
    /// Explicit expiry.
    pub expires_at: Option<i64>,
    /// At most eight work items, main first.
    pub work: Vec<WorkProjection>,
}

/// Frame stack projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FramesProjection {
    /// Frame-stack revision.
    pub rev: u64,
    /// Top template id.
    pub top: Option<String>,
    /// Current step.
    pub step: Option<String>,
    /// Current attempt.
    pub attempt: u8,
    /// Next utterance instant.
    pub next_at: Option<i64>,
    /// Frame labels, root first.
    pub stack: Vec<String>,
}

/// One pending-work item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingItem {
    /// `commitment`, `blocker`, `agent`, `plan_step`, or `action`.
    pub kind: String,
    /// Stable id.
    pub id: String,
    /// Bounded summary.
    pub summary: String,
    /// `tool`, `operator`, or `wait`.
    pub actionable: String,
}

/// Budget projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BudgetProjection {
    /// Class name.
    pub class: String,
    /// Model rounds remaining.
    pub rounds: u64,
    /// Tool steps remaining.
    pub steps: u64,
    /// Tokens remaining.
    pub tokens: u64,
    /// Wall time remaining.
    pub wall_ms: u64,
    /// Whether the remaining values include an estimate.
    pub estimated: bool,
}

impl From<crate::budget::BudgetSnapshot> for BudgetProjection {
    fn from(snapshot: crate::budget::BudgetSnapshot) -> Self {
        Self {
            class: snapshot.class.into(),
            rounds: u64::from(snapshot.rounds),
            steps: u64::from(snapshot.steps),
            tokens: snapshot.tokens,
            wall_ms: snapshot.wall_ms,
            estimated: snapshot.estimated,
        }
    }
}

/// One downstream agent run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProjection {
    /// Run id.
    pub run_id: String,
    /// Backend kind.
    pub kind: String,
    /// Run status.
    pub status: String,
    /// Completed turns.
    pub turn: u32,
    /// Concrete next step.
    pub next: Option<String>,
    /// Last compact result.
    pub last: Option<String>,
}

/// Knowledge-health projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataProjection {
    /// Data revision.
    pub rev: u64,
    /// Entity count.
    pub entities: u64,
    /// Search document count.
    pub docs: u64,
    /// Index lag.
    pub lag_ms: u64,
    /// Active schema prefixes.
    pub schemas: Vec<String>,
    /// Last mutation summary.
    pub last: Option<String>,
}

/// Memory counts, never contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProjection {
    /// Episodic count.
    pub episodic: u64,
    /// Segment count.
    pub segments: u64,
    /// Last recall summary.
    pub last_recall: Option<String>,
}

/// Current plan header, not the plan body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlansProjection {
    /// Current plan id.
    pub current: Option<String>,
    /// Active plan count.
    pub active: u64,
    /// Current version.
    pub version: u64,
    /// Current status.
    pub status: Option<String>,
    /// Step count.
    pub steps: u64,
}

/// Action counts, not the action list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActionsProjection {
    /// Scheduled actions.
    pub scheduled: u64,
    /// Cron actions.
    pub cron: u64,
    /// Running actions.
    pub running: u64,
}

/// The complete bounded source for one `[STATE]` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateView {
    /// Current user.
    pub user: UserProjection,
    /// Session projection.
    pub session: SessionProjection,
    /// Frame projection.
    pub frames: FramesProjection,
    /// Topic, if the CM has named one.
    pub topic: Option<String>,
    /// Drill level.
    pub depth: u8,
    /// Every status variable, never truncated.
    pub status: BTreeMap<String, StatusValue>,
    /// Conversation revision.
    pub revision: u64,
    /// Last state write instant.
    pub updated_at: i64,
    /// Last state writer.
    pub updated_by: String,
    /// Derived pending work.
    pub pending: Vec<PendingItem>,
    /// Budget projection.
    pub budget: BudgetProjection,
    /// Active agent runs.
    pub agents: Vec<AgentProjection>,
    /// Most recent terminal run.
    pub last_agent: Option<AgentProjection>,
    /// Knowledge health.
    pub data: DataProjection,
    /// Memory counts.
    pub memory: MemoryProjection,
    /// Plan header.
    pub plans: PlansProjection,
    /// Action counts.
    pub actions: ActionsProjection,
    /// Compact service health.
    pub health: BTreeMap<String, String>,
    /// In-flight operations plus the last five completed.
    pub ops: Vec<String>,
}

impl StateView {
    /// Build from a loaded session bundle.
    pub fn from_bundle(
        bundle: &SessionBundle,
        user: UserProjection,
        budget: BudgetProjection,
    ) -> Self {
        Self::from_parts(
            &bundle.conversation,
            &bundle.session,
            &bundle.frames,
            user,
            budget,
        )
    }

    /// Build from its three durable components.
    pub fn from_parts(
        conversation: &ConversationState,
        session: &SessionState,
        frames: &FrameStack,
        user: UserProjection,
        budget: BudgetProjection,
    ) -> Self {
        let status = conversation.status.clone();
        let work = session
            .work
            .items()
            .into_iter()
            .take(MAX_WORK_ITEMS)
            .map(|item| WorkProjection {
                id: item.id.clone(),
                status: work_status(item.status),
                title: item.title.clone(),
            })
            .collect();
        let top = frames.top();
        let frame_step = status
            .get("frame_step")
            .and_then(status_text)
            .or_else(|| top.map(|frame| frame.template_id.clone()));
        let frame_attempt = status
            .get("frame_attempt")
            .and_then(status_integer)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(0);
        let next_at = status
            .get("next_utterance_at")
            .and_then(status_integer)
            .filter(|value| *value > 0);
        let stack = frames
            .frames
            .iter()
            .rev()
            .take(MAX_FRAME_LABELS)
            .map(|frame| frame.template_id.clone())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let mut pending = Vec::new();
        for (key, actionable, kind) in [
            ("next", "tool", "commitment"),
            ("blockers", "operator", "blocker"),
            ("questions", "operator", "question"),
        ] {
            if let Some(StatusValue::List(values)) = status.get(key) {
                for (index, value) in values.iter().enumerate() {
                    if pending.len() >= MAX_PENDING_ITEMS {
                        break;
                    }
                    pending.push(PendingItem {
                        kind: kind.into(),
                        id: format!("{key}:{index}"),
                        summary: value.clone(),
                        actionable: actionable.into(),
                    });
                }
            }
        }
        Self {
            user,
            session: SessionProjection {
                lifecycle: session.lifecycle.as_str().into(),
                presence: format!("{:?}", session.presence).to_lowercase(),
                working: session.work.working(),
                expires_at: session.expires_at,
                work,
            },
            frames: FramesProjection {
                rev: frames.revision,
                top: top.map(|frame| frame.template_id.clone()),
                step: frame_step,
                attempt: frame_attempt,
                next_at,
                stack,
            },
            topic: conversation.topic.clone(),
            depth: conversation.depth,
            status,
            revision: conversation.revision,
            updated_at: conversation.updated_at,
            updated_by: conversation.updated_by.as_str().into(),
            pending,
            budget,
            agents: Vec::new(),
            last_agent: None,
            data: DataProjection::default(),
            memory: MemoryProjection::default(),
            plans: PlansProjection::default(),
            actions: ActionsProjection::default(),
            health: BTreeMap::new(),
            ops: Vec::new(),
        }
    }

    /// Bound optional service projections without changing conversation state.
    pub fn bounded(mut self) -> Self {
        self.pending.truncate(MAX_PENDING_ITEMS);
        self.agents.truncate(MAX_ACTIVE_AGENTS);
        self.data.schemas.truncate(MAX_SCHEMAS);
        self.ops.truncate(MAX_OPS);
        self
    }

    /// Render the canonical `[STATE]` envelope.
    pub fn render(&self) -> Result<StateBlock, StateBlockError> {
        let json = serde_json::to_string(&self.clone().bounded())?;
        let header = format!(
            "[STATE user={} rev={} as_of={}]",
            if self.user.iri.is_empty() {
                "unknown"
            } else {
                self.user.iri.as_str()
            },
            self.revision,
            self.updated_at
        );
        let text = format!("{header}\n{json}");
        if text.len() > MAX_STATE_BLOCK_BYTES {
            return Err(StateBlockError::TooLarge {
                bytes: text.len(),
                cap: MAX_STATE_BLOCK_BYTES,
            });
        }
        Ok(StateBlock { header, json, text })
    }

    /// The bounded pending summary rendered into prompt variables.
    pub fn pending_summary(&self) -> String {
        if self.pending.is_empty() {
            return "none".into();
        }
        self.pending
            .iter()
            .map(|item| format!("[{}] {} ({})", item.kind, item.summary, item.actionable))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// A rendered state block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateBlock {
    /// Header line.
    pub header: String,
    /// Minified JSON payload.
    pub json: String,
    /// Header, newline, and payload.
    pub text: String,
}

impl StateBlock {
    /// The honest unavailable block. It is never silently omitted.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let header = format!("[STATE unavailable: {reason}]");
        Self {
            header: header.clone(),
            json: String::new(),
            text: header,
        }
    }
}

fn status_text(value: &StatusValue) -> Option<String> {
    match value {
        StatusValue::Text(value) => Some(value.clone()),
        _ => None,
    }
}

fn status_integer(value: &StatusValue) -> Option<i64> {
    match value {
        StatusValue::Integer(value) => Some(*value),
        _ => None,
    }
}

fn work_status(status: mm_data::WorkStatus) -> String {
    format!("{status:?}").to_lowercase()
}

/// State-block failures.
#[derive(Debug, Error)]
pub enum StateBlockError {
    /// Serialization failed.
    #[error("state block json: {0}")]
    Json(#[from] serde_json::Error),
    /// The canonical block exceeded its cap; never truncate.
    #[error("state block is {bytes} bytes, over the {cap}-byte cap")]
    TooLarge {
        /// Actual size.
        bytes: usize,
        /// Allowed size.
        cap: usize,
    },
}

/// Result alias.
pub type Result<T, E = StateBlockError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;
    use mm_data::{ConversationFrame, FrameKind, FrameStatus, SessionState};

    fn bundle() -> SessionBundle {
        let mut conversation = ConversationState::new("s", 1_000);
        conversation.topic = Some("auth refresh flake".into());
        conversation.depth = 2;
        conversation.status.insert(
            "next".into(),
            StatusValue::List(vec![
                "reproduce the flake".into(),
                "run auth test 20x".into(),
            ]),
        );
        conversation
            .status
            .insert("frame_step".into(), StatusValue::Text("connect".into()));
        let session = SessionState::new("s", "u", 1_000);
        let frames = FrameStack::new(ConversationFrame {
            id: "root".into(),
            template_id: "frame.intro".into(),
            template_version: 1,
            kind: FrameKind::Intro,
            status: FrameStatus::Active,
            entered_at: 1_000,
            updated_at: 1_000,
            expires_at: None,
        });
        SessionBundle {
            conversation,
            session,
            frames,
        }
    }

    fn user() -> UserProjection {
        UserProjection {
            iri: "mmd:current-user".into(),
            entity: "mmd:user/01U".into(),
            name: "Ada".into(),
            workspace: "workspace/01U".into(),
        }
    }

    #[test]
    fn every_conversation_field_and_pending_item_is_present() {
        let view = StateView::from_bundle(
            &bundle(),
            user(),
            BudgetProjection {
                class: "standard".into(),
                rounds: 24,
                steps: 80,
                tokens: 108_000,
                wall_ms: 520_000,
                estimated: false,
            },
        );
        let block = view.render().unwrap();
        assert!(block
            .header
            .starts_with("[STATE user=mmd:current-user rev=1"));
        assert!(block.json.contains("\"topic\":\"auth refresh flake\""));
        assert!(block.json.contains("\"depth\":2"));
        assert!(block.json.contains("\"status\""));
        assert!(block.json.contains("\"frame_step\":\"connect\""));
        assert!(block.json.contains("\"pending\""));
        assert!(block.json.contains("\"budget\""));
        assert!(block.json.contains("\"updatedAt\""));
        assert!(!view.pending_summary().is_empty());
    }

    #[test]
    fn unavailable_is_honest_and_never_empty() {
        let block = StateBlock::unavailable("store closed");
        assert_eq!(block.text, "[STATE unavailable: store closed]");
        assert!(block.json.is_empty());
    }

    #[test]
    fn over_cap_is_an_error_not_a_truncation() {
        let mut view = StateView::from_bundle(&bundle(), user(), BudgetProjection::default());
        view.ops = (0..MAX_OPS)
            .map(|index| "x".repeat(1_000) + &index.to_string())
            .collect();
        assert!(matches!(
            view.render(),
            Err(StateBlockError::TooLarge { .. })
        ));
    }

    #[test]
    fn optional_projections_are_bounded() {
        let mut view = StateView::from_bundle(&bundle(), user(), BudgetProjection::default());
        view.pending = (0..20)
            .map(|index| PendingItem {
                kind: "commitment".into(),
                id: index.to_string(),
                summary: "x".into(),
                actionable: "tool".into(),
            })
            .collect();
        view.agents = (0..10)
            .map(|index| AgentProjection {
                run_id: index.to_string(),
                kind: "pi".into(),
                status: "running".into(),
                turn: 1,
                next: None,
                last: None,
            })
            .collect();
        let bounded = view.bounded();
        assert_eq!(bounded.pending.len(), MAX_PENDING_ITEMS);
        assert_eq!(bounded.agents.len(), MAX_ACTIVE_AGENTS);
    }
}
