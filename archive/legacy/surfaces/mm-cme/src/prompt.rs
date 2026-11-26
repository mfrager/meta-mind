//! Domain adapter from CME state to the prompt manager.
//!
//! Prompt prose lives in `config/prompts/`; this module only names the slots and
//! copies bounded domain values into them. Rendering is deterministic and
//! returns an immutable [`mm_prompt::PromptInstance`] whose hash is suitable for
//! the inspector and the turn record.

use std::collections::BTreeMap;
use std::sync::Arc;

use mm_data::{ConversationState, FrameStack, SessionBundle, SessionState};
use mm_prompt::{PromptError, PromptInstance, PromptRequest, PromptStore};

use crate::budget::BudgetSnapshot;
use crate::session::SessionContext;
use crate::state_block::{BudgetProjection, StateView, UserProjection};
use thiserror::Error;

/// The complete slot map for one prompt rendering.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptVariables {
    values: BTreeMap<String, String>,
}

impl PromptVariables {
    /// Empty variables, with the standard bounded slots set to empty strings.
    pub fn new() -> Self {
        let mut values = BTreeMap::new();
        for slot in STANDARD_SLOTS {
            values.insert((*slot).to_owned(), String::new());
        }
        Self { values }
    }

    /// Insert or replace one slot.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.values.insert(key.into(), value.into());
    }

    /// Look up one slot.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Borrow the underlying map.
    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.values
    }

    /// Consume into the underlying map.
    pub fn into_map(self) -> BTreeMap<String, String> {
        self.values
    }

    /// Fill the standard turn slots from durable state. Every value is bounded
    /// by the caller before it reaches the prompt tree.
    pub fn from_turn(
        conversation: &ConversationState,
        session: &SessionState,
        frames: &FrameStack,
        user_input: &str,
        project_name: Option<&str>,
    ) -> Result<Self, PromptingError> {
        let mut variables = Self::new();
        variables.set("session_id", &session.session_id);
        variables.set("user_id", &session.user_id);
        variables.set("state_json", conversation.status_json()?);
        let user = UserProjection {
            iri: "mmd:current-user".into(),
            entity: session.user_id.clone(),
            name: session.user_id.clone(),
            workspace: String::new(),
        };
        let budget = BudgetSnapshot {
            class: "standard",
            rounds: 32,
            steps: 96,
            tokens: 120_000,
            wall_ms: 600_000,
            estimated: false,
        };
        let view = StateView::from_parts(
            conversation,
            session,
            frames,
            user,
            BudgetProjection::from(budget),
        );
        variables.set_view(&view)?;
        variables.set("being_name", "Metamind");
        variables.set("being_version", "1");
        variables.set("being_values", "honesty, continuity, care");
        variables.set(
            "tools_summary",
            "conversation.*, data.*, metamind.*, agent.run",
        );
        variables.set("tone", "warm, direct, honest");
        variables.set("topic", conversation.topic.clone().unwrap_or_default());
        variables.set("depth", conversation.depth.to_string());
        variables.set("revision", conversation.revision.to_string());
        variables.set("user_input", truncate(user_input, 8_000));
        variables.set(
            "project_name",
            project_name
                .map(|value| truncate(value, 200))
                .unwrap_or_default(),
        );
        variables.set(
            "frame_kind",
            frames
                .top()
                .map(|frame| format!("{:?}", frame.kind).to_lowercase())
                .unwrap_or_default(),
        );
        variables.set(
            "frame_step",
            frames
                .top()
                .map(|frame| frame.template_id.clone())
                .unwrap_or_default(),
        );
        variables.set(
            "frame_attempt",
            conversation
                .status
                .get("frame_attempt")
                .map(|value| format!("{value:?}"))
                .unwrap_or_default(),
        );
        Ok(variables)
    }

    /// Fill the frame/tutorial/action slots from bounded strings.
    pub fn with_context(mut self, key: &str, value: impl Into<String>) -> Self {
        self.set(key, value);
        self
    }

    /// Project a bounded state view into prompt slots.
    pub fn set_view(&mut self, view: &StateView) -> Result<(), PromptingError> {
        let block = view
            .render()
            .map_err(|error| PromptingError::State(error.to_string()))?;
        self.set("state_block", block.text);
        self.set(
            "state_json",
            serde_json::to_string(view)
                .map_err(|error| PromptingError::State(error.to_string()))?,
        );
        self.set("revision", view.revision.to_string());
        self.set("topic", view.topic.clone().unwrap_or_default());
        self.set("depth", view.depth.to_string());
        self.set("pending_summary", view.pending_summary());
        self.set("frame_kind", view.frames.top.clone().unwrap_or_default());
        self.set("frame_step", view.frames.step.clone().unwrap_or_default());
        self.set("frame_attempt", view.frames.attempt.to_string());
        self.set("user_name", view.user.name.clone());
        self.set("user_iri", view.user.iri.clone());
        self.set("canonical_user", view.user.entity.clone());
        self.set("workspace_label", view.user.workspace.clone());
        self.set("budget_class", view.budget.class.clone());
        self.set("budget_rounds", view.budget.rounds.to_string());
        self.set("budget_tokens", view.budget.tokens.to_string());
        self.set("budget_wall_ms", view.budget.wall_ms.to_string());
        self.set(
            "memory_summary",
            format!(
                "{} episodic memories, {} segments",
                view.memory.episodic, view.memory.segments
            ),
        );
        Ok(())
    }

    /// A safe set for direct executor tests and callers without a state store.
    /// The state block is explicitly unavailable rather than invented.
    pub fn for_turn(plan: &crate::turn::TurnPlan, session: &SessionContext) -> Self {
        let mut variables = Self::new();
        variables.set("user_input", truncate(&plan.user_query, 8_000));
        variables.set("session_id", plan.session_id.to_string());
        variables.set("user_id", "operator");
        variables.set(
            "project_name",
            session
                .memory
                .project_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        );
        variables.set(
            "previous_summary",
            session.memory.project_summary.clone().unwrap_or_default(),
        );
        variables.set("previous_topics", session.memory.previous_topics.join("; "));
        variables.set(
            "state_block",
            crate::state_block::StateBlock::unavailable(
                "conversation state was not supplied to this executor",
            )
            .text,
        );
        variables.set("being_name", "Metamind");
        variables.set("being_version", "1");
        variables.set("being_values", "honesty, continuity, care");
        variables.set("user_name", "operator");
        variables.set("user_iri", "mmd:current-user");
        variables.set("canonical_user", "mmd:current-user");
        variables.set("user_locale", "en");
        variables.set("user_timezone", "UTC");
        variables.set("workspace_label", "workspace");
        variables.set(
            "tools_summary",
            "conversation.*, data.*, metamind.*, agent.run",
        );
        variables.set("tone", "warm, direct, honest");
        variables.set("pending_summary", "none");
        variables.set("frame_kind", "work");
        variables.set("frame_step", "task");
        variables.set("frame_attempt", "0");
        variables.set("memory_summary", "no recall supplied");
        variables.set("budget_class", "standard");
        variables.set("budget_rounds", "32");
        variables.set("budget_tokens", "120000");
        variables.set("budget_wall_ms", "600000");
        variables
    }

    /// Build a turn variable map from a loaded session bundle and budget.
    pub fn from_bundle(
        bundle: &SessionBundle,
        user: UserProjection,
        budget: BudgetSnapshot,
    ) -> Result<Self, PromptingError> {
        let mut variables = Self::new();
        let view = StateView::from_bundle(bundle, user, BudgetProjection::from(budget));
        variables.set_view(&view)?;
        variables.set("session_id", bundle.session.session_id.clone());
        variables.set("user_id", bundle.session.user_id.clone());
        variables.set("being_name", "Metamind");
        variables.set("being_version", "1");
        variables.set("being_values", "honesty, continuity, care");
        variables.set("user_locale", "en");
        variables.set("user_timezone", "UTC");
        variables.set(
            "tools_summary",
            "conversation.*, data.*, metamind.*, agent.run",
        );
        variables.set("tone", "warm, direct, honest");
        Ok(variables)
    }
}

/// Slots the common prompt tree may reference. A strategy may declare more.
pub const STANDARD_SLOTS: &[&str] = &[
    "session_id",
    "user_id",
    "user_name",
    "user_iri",
    "canonical_user",
    "user_locale",
    "user_timezone",
    "being_name",
    "being_version",
    "being_values",
    "workspace_label",
    "state_block",
    "state_json",
    "topic",
    "depth",
    "revision",
    "transcript_compact",
    "tools_summary",
    "tone",
    "user_input",
    "project_name",
    "previous_summary",
    "previous_topics",
    "last_topic",
    "tutorial_topic",
    "tutorial_step",
    "goodbye_reason",
    "session_summary",
    "frame_kind",
    "frame_step",
    "frame_attempt",
    "frame_goal",
    "entity_summary",
    "action_kind",
    "action_input",
    "recent_log",
    "plan_goal",
    "plan_state",
    "memory_query",
    "memory_context",
    "memory_summary",
    "pending_summary",
    "budget_class",
    "budget_rounds",
    "budget_tokens",
    "budget_wall_ms",
    "repair_reason",
    "thought_question",
    "log_query",
    "log_entries",
    "agent_kind",
    "agent_workspace",
    "agent_input",
];

/// The domain prompt manager.
#[derive(Debug, Clone)]
pub struct CmePromptManager {
    store: Arc<PromptStore>,
}

impl CmePromptManager {
    /// Build over a validated prompt store.
    pub fn new(store: Arc<PromptStore>) -> Self {
        Self { store }
    }

    /// The underlying store.
    pub fn store(&self) -> &PromptStore {
        &self.store
    }

    /// Render a named strategy.
    pub fn render(
        &self,
        strategy: &str,
        variables: &PromptVariables,
    ) -> Result<PromptInstance, PromptingError> {
        Ok(self.store.render(strategy, variables.as_map())?)
    }

    /// Select a strategy and render the complete message envelope.
    pub fn render_request(
        &self,
        request: &PromptRequest,
    ) -> Result<PromptInstance, PromptingError> {
        Ok(self.store.render_request(request)?)
    }

    /// Render with a set of non-standard slots added to the standard map.
    pub fn render_with(
        &self,
        strategy: &str,
        variables: &PromptVariables,
        extra: impl IntoIterator<Item = (String, String)>,
    ) -> Result<PromptInstance, PromptingError> {
        let mut map = variables.as_map().clone();
        for (key, value) in extra {
            map.insert(key, value);
        }
        Ok(self.store.render(strategy, &map)?)
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    value.chars().take(max).collect()
}

/// Prompt adapter failures.
#[derive(Debug, Error)]
pub enum PromptingError {
    /// Prompt tree/render failure.
    #[error("prompt: {0}")]
    Prompt(#[from] PromptError),
    /// Canonical state serialization failed.
    #[error("state: {0}")]
    State(String),
}

impl From<mm_data::ConversationError> for PromptingError {
    fn from(error: mm_data::ConversationError) -> Self {
        Self::State(error.to_string())
    }
}

/// Result alias.
pub type Result<T, E = PromptingError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn manager() -> CmePromptManager {
        let store =
            PromptStore::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"))
                .expect("checked-in tree loads");
        CmePromptManager::new(Arc::new(store))
    }

    #[test]
    fn turn_variables_include_state_and_frame_fields() {
        let conversation = ConversationState::new("s", 1);
        let session = SessionState::new("s", "u", 1);
        let frames = FrameStack::new(mm_data::ConversationFrame {
            id: "root".into(),
            template_id: "frame.intro".into(),
            template_version: 1,
            kind: mm_data::FrameKind::Intro,
            status: mm_data::FrameStatus::Active,
            entered_at: 1,
            updated_at: 1,
            expires_at: None,
        });
        let variables =
            PromptVariables::from_turn(&conversation, &session, &frames, "hello", None).unwrap();
        assert_eq!(variables.get("user_input"), Some("hello"));
        assert_eq!(variables.get("frame_step"), Some("frame.intro"));
        assert!(!variables.get("state_json").unwrap().is_empty());
    }

    #[test]
    fn every_strategy_renders_with_standard_values() {
        let manager = manager();
        let mut variables = PromptVariables::new();
        for slot in STANDARD_SLOTS {
            variables.set(*slot, "fixture");
        }
        for strategy in manager.store().strategies() {
            for slot in &strategy.slots {
                variables.set(slot.clone(), "fixture");
            }
            manager.render(&strategy.id, &variables).unwrap();
        }
    }
}
