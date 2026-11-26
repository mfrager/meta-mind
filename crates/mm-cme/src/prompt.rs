//! The domain adapter from turn state to the prompt tree.
//!
//! Prose lives in `config/prompts/`; this module names the retained slots and
//! copies bounded values into them. Rendering is deterministic and returns an
//! immutable [`mm_prompt::PromptInstance`].

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use mm_agentstream::SessionState;
use mm_prompt::{PromptInstance, PromptRequest, PromptStore};
use serde::{Deserialize, Serialize};

use crate::budget::BudgetSnapshot;
use crate::error::{CmeError, Result};
use crate::state_block;

/// Static identity values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Persona {
    /// Display name.
    pub name: String,
    /// Identity version.
    pub version: u32,
    /// Stated values.
    pub values: String,
    /// Speaking tone.
    pub tone: String,
}

impl Default for Persona {
    fn default() -> Self {
        Self {
            name: "Metamind".into(),
            version: 1,
            values: "honesty, continuity, care".into(),
            tone: "warm, direct, honest".into(),
        }
    }
}

impl Persona {
    /// Load a persona file. A missing file is the default.
    pub fn load_or_default(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|error| CmeError::Config(format!("{}: {error}", path.display())))?;
        toml::from_str(&text).map_err(|error| CmeError::Config(format!("persona: {error}")))
    }
}

/// A slot map for one prompt rendering.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptVariables {
    values: BTreeMap<String, String>,
}

impl PromptVariables {
    /// Empty variables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace one slot.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.values.insert(key.into(), value.into());
    }

    /// Look up one slot.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// The underlying map.
    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.values
    }

    /// The base variables every pass shares.
    pub fn base(
        persona: &Persona,
        session_id: &str,
        state: &SessionState,
        budget: &BudgetSnapshot,
        user_input: &str,
        tools_summary: &str,
    ) -> Self {
        let mut variables = Self::new();
        variables.set("session_id", session_id);
        variables.set("user_id", "operator");
        variables.set("user_name", "operator");
        variables.set("workspace_label", "workspace");
        variables.set("being_name", &persona.name);
        variables.set("being_version", persona.version.to_string());
        variables.set("being_values", &persona.values);
        variables.set("tone", &persona.tone);
        variables.set("state_block", state_block::render("operator", state));
        variables.set(
            "state_json",
            serde_json::to_string(state).unwrap_or_else(|_| "{}".into()),
        );
        variables.set("revision", state.revision.to_string());
        variables.set("topic", state.topic.clone().unwrap_or_default());
        variables.set("depth", state.depth.to_string());
        variables.set("user_input", truncate(user_input, 8_000));
        variables.set("tools_summary", tools_summary);
        variables.set("budget_class", &budget.class);
        variables.set("budget_rounds", budget.rounds.to_string());
        variables.set("budget_tokens", budget.tokens.to_string());
        variables.set("budget_wall_ms", budget.wall_ms.to_string());
        variables
    }
}

/// Slots the retained prompt tree may reference.
pub const STANDARD_SLOTS: &[&str] = &[
    "session_id",
    "user_id",
    "user_name",
    "workspace_label",
    "being_name",
    "being_version",
    "being_values",
    "state_block",
    "state_json",
    "topic",
    "depth",
    "revision",
    "tools_summary",
    "tone",
    "user_input",
    "budget_class",
    "budget_rounds",
    "budget_tokens",
    "budget_wall_ms",
    "process_task",
    "process_constraints",
    "process_suggested_tools",
    "retry_context",
    "verification_results",
    "checks_registered",
    "retry_count",
    "process_reply",
    "staged_intents",
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
    pub fn render(&self, strategy: &str, variables: &PromptVariables) -> Result<PromptInstance> {
        Ok(self.store.render(strategy, variables.as_map())?)
    }

    /// Select and render a strategy with the ordered message envelope.
    pub fn render_request(&self, request: &PromptRequest) -> Result<PromptInstance> {
        Ok(self.store.render_request(request)?)
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::BudgetClass;

    #[test]
    fn base_variables_have_every_shared_slot() {
        let budget = BudgetSnapshot::for_class(BudgetClass::Standard, false);
        let variables = PromptVariables::base(
            &Persona::default(),
            "s",
            &SessionState::default(),
            &budget,
            "hello",
            "conversation.topic",
        );
        for slot in [
            "state_block",
            "being_name",
            "user_input",
            "budget_class",
            "tools_summary",
            "revision",
        ] {
            assert!(variables.get(slot).is_some(), "{slot}");
        }
        assert_eq!(variables.get("user_input"), Some("hello"));
    }

    #[test]
    fn every_retained_strategy_renders() {
        let store =
            PromptStore::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"))
                .unwrap();
        let manager = CmePromptManager::new(Arc::new(store));
        let budget = BudgetSnapshot::for_class(BudgetClass::Standard, false);
        let mut variables = PromptVariables::base(
            &Persona::default(),
            "s",
            &SessionState::default(),
            &budget,
            "hello",
            "conversation.topic",
        );
        for slot in STANDARD_SLOTS {
            if variables.get(slot).is_none() {
                variables.set(*slot, "fixture");
            }
        }
        for strategy in ["cm.interpret.v1", "cm.process.v1", "cm.verify.v1"] {
            manager.render(strategy, &variables).unwrap();
        }
    }
}
