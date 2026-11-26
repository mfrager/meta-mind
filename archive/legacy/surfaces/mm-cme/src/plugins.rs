//! Phase 5 plugin host: load, validate, evaluate and execute proactive plugins.
//!
//! Plugins are compiled Rust trait objects. The host is the sandbox boundary: it
//! validates manifests, caps the proposals and executions a cycle may consume,
//! and only evaluates plugins registered for a trigger. Dynamic loading (WASM or
//! a native shared object) is a transport adapter behind [`Plugin`]; it does not
//! change the host contract.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{CmeError, Result};
use crate::session::{AffectState, PreferenceProfile};

/// What a plugin is allowed to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    /// Read the merged preference profile.
    ReadPreferences,
    /// Read memory search hits.
    ReadMemory,
    /// Propose proactive items.
    Propose,
}

/// A plugin's declared identity, triggers and capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Stable identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Semantic version.
    pub version: String,
    /// Triggers the plugin wants to run on.
    pub triggers: Vec<String>,
    /// Capabilities it needs.
    pub capabilities: Vec<PluginCapability>,
}

/// The inputs a plugin may read when it evaluates.
#[derive(Debug, Clone, Copy)]
pub struct PluginContext<'a> {
    /// The user's current request.
    pub query: &'a str,
    /// Merged preferences in force.
    pub preferences: &'a PreferenceProfile,
    /// Current affect.
    pub affect: AffectState,
    /// Memory hits already retrieved for this turn.
    pub memory_hits: &'a [String],
}

/// One proposal a plugin makes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProactiveProposal {
    /// Plugin that made it.
    pub plugin: String,
    /// Trigger that fired.
    pub trigger: String,
    /// Short heading.
    pub title: String,
    /// The suggestion itself.
    pub body: String,
    /// 0.0–1.0 relevance.
    pub score: f32,
}

/// The result of executing one proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginResult {
    /// Plugin that ran.
    pub plugin: String,
    /// Action taken.
    pub action: String,
    /// Human-readable detail.
    pub detail: String,
}

/// A proactive plugin.
pub trait Plugin: Send + Sync {
    /// Declared identity.
    fn manifest(&self) -> &PluginManifest;
    /// JSON schema for its configuration.
    fn config_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "min_score": { "type": "number", "minimum": 0.0, "maximum": 1.0 } }
        })
    }
    /// Score proposals for this context.
    fn evaluate(&self, ctx: &PluginContext<'_>) -> Vec<ProactiveProposal>;
    /// Carry out a selected proposal.
    fn execute(&self, proposal: &ProactiveProposal) -> Result<PluginResult>;
}

/// Resource limits enforced by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginSandbox {
    /// Proposals kept per plugin per cycle.
    pub max_proposals_per_plugin: usize,
    /// Proposals kept across all plugins per cycle.
    pub max_proposals_per_cycle: usize,
    /// Executions allowed per cycle.
    pub max_executions_per_cycle: usize,
}

impl Default for PluginSandbox {
    fn default() -> Self {
        Self {
            max_proposals_per_plugin: 2,
            max_proposals_per_cycle: 8,
            max_executions_per_cycle: 8,
        }
    }
}

/// The plugin host.
pub struct PluginHost {
    plugins: BTreeMap<String, Box<dyn Plugin>>,
    triggers: BTreeMap<String, Vec<String>>,
    sandbox: PluginSandbox,
    executions: usize,
}

impl std::fmt::Debug for PluginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginHost")
            .field("plugins", &self.plugins.keys().collect::<Vec<_>>())
            .field("sandbox", &self.sandbox)
            .field("executions", &self.executions)
            .finish()
    }
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new(PluginSandbox::default())
    }
}

impl PluginHost {
    /// An empty host with explicit limits.
    pub fn new(sandbox: PluginSandbox) -> Self {
        Self {
            plugins: BTreeMap::new(),
            triggers: BTreeMap::new(),
            sandbox,
            executions: 0,
        }
    }

    /// A host with all six built-in plugins loaded.
    pub fn with_builtins(sandbox: PluginSandbox) -> Result<Self> {
        let mut host = Self::new(sandbox);
        for plugin in BuiltIn::all().iter().copied() {
            host.load(plugin.plugin())?;
        }
        Ok(host)
    }

    /// Load and register a plugin after validating its manifest.
    pub fn load(&mut self, plugin: Box<dyn Plugin>) -> Result<PluginManifest> {
        let manifest = plugin.manifest().clone();
        if manifest.id.trim().is_empty() {
            return Err(CmeError::Plugin("plugin id must not be empty".to_owned()));
        }
        if manifest.name.trim().is_empty() {
            return Err(CmeError::Plugin(format!(
                "plugin `{}` has no name",
                manifest.id
            )));
        }
        if manifest.triggers.is_empty() {
            return Err(CmeError::Plugin(format!(
                "plugin `{}` registers no triggers",
                manifest.id
            )));
        }
        if !manifest.capabilities.contains(&PluginCapability::Propose) {
            return Err(CmeError::Plugin(format!(
                "plugin `{}` does not declare the propose capability",
                manifest.id
            )));
        }
        if self.plugins.contains_key(&manifest.id) {
            return Err(CmeError::Plugin(format!(
                "plugin `{}` is already loaded",
                manifest.id
            )));
        }
        for trigger in &manifest.triggers {
            self.triggers
                .entry(trigger.clone())
                .or_default()
                .push(manifest.id.clone());
        }
        self.plugins.insert(manifest.id.clone(), plugin);
        Ok(manifest)
    }

    /// Unload a plugin and its triggers.
    pub fn unload(&mut self, id: &str) -> Result<()> {
        self.plugins
            .remove(id)
            .ok_or_else(|| CmeError::Plugin(format!("plugin `{id}` is not loaded")))?;
        for ids in self.triggers.values_mut() {
            ids.retain(|loaded| loaded != id);
        }
        self.triggers.retain(|_, ids| !ids.is_empty());
        Ok(())
    }

    /// Loaded plugin ids.
    pub fn ids(&self) -> Vec<&str> {
        self.plugins.keys().map(String::as_str).collect()
    }

    /// Whether a plugin is loaded.
    pub fn contains(&self, id: &str) -> bool {
        self.plugins.contains_key(id)
    }

    /// A plugin's configuration schema.
    pub fn config_schema(&self, id: &str) -> Option<Value> {
        self.plugins.get(id).map(|plugin| plugin.config_schema())
    }

    /// Evaluate every loaded plugin under the sandbox's proposal caps.
    pub fn evaluate(&self, ctx: &PluginContext<'_>) -> Vec<ProactiveProposal> {
        let plugins: Vec<&dyn Plugin> = self
            .plugins
            .values()
            .map(|plugin| plugin.as_ref())
            .collect();
        self.collect(plugins, ctx)
    }

    /// Evaluate only plugins registered for `trigger`.
    pub fn dispatch(&self, trigger: &str, ctx: &PluginContext<'_>) -> Vec<ProactiveProposal> {
        let plugins: Vec<&dyn Plugin> = self
            .triggers
            .get(trigger)
            .into_iter()
            .flatten()
            .filter_map(|id| self.plugins.get(id))
            .map(|plugin| plugin.as_ref())
            .collect();
        self.collect(plugins, ctx)
    }

    fn collect(
        &self,
        plugins: Vec<&dyn Plugin>,
        ctx: &PluginContext<'_>,
    ) -> Vec<ProactiveProposal> {
        let mut proposals = Vec::new();
        for plugin in plugins {
            let id = &plugin.manifest().id;
            let mut produced: Vec<ProactiveProposal> = plugin
                .evaluate(ctx)
                .into_iter()
                .filter(|proposal| proposal.score.is_finite())
                .map(|mut proposal| {
                    proposal.plugin.clone_from(id);
                    proposal.score = proposal.score.clamp(0.0, 1.0);
                    proposal
                })
                .collect();
            produced.truncate(self.sandbox.max_proposals_per_plugin);
            proposals.extend(produced);
        }
        proposals.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.plugin.cmp(&right.plugin))
                .then_with(|| left.title.cmp(&right.title))
        });
        proposals.truncate(self.sandbox.max_proposals_per_cycle);
        proposals
    }

    /// Execute a selected proposal, within the cycle's execution budget.
    pub fn execute(&mut self, proposal: &ProactiveProposal) -> Result<PluginResult> {
        if self.executions >= self.sandbox.max_executions_per_cycle {
            return Err(CmeError::Plugin(
                "sandbox execution budget exhausted".to_owned(),
            ));
        }
        let plugin = self.plugins.get(&proposal.plugin).ok_or_else(|| {
            CmeError::Plugin(format!("plugin `{}` is not loaded", proposal.plugin))
        })?;
        self.executions = self.executions.saturating_add(1);
        plugin.execute(proposal)
    }

    /// Start a new cycle, resetting the execution budget.
    pub fn reset_cycle(&mut self) {
        self.executions = 0;
    }

    /// Executions consumed this cycle.
    pub fn executions(&self) -> usize {
        self.executions
    }

    /// Executions still available this cycle.
    pub fn remaining_executions(&self) -> usize {
        self.sandbox
            .max_executions_per_cycle
            .saturating_sub(self.executions)
    }
}

/// The six built-in proactive plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltIn {
    /// Look up an external or memory source.
    SearchLookup,
    /// Narrow a broad goal.
    GoalNarrowing,
    /// Refine an existing plan.
    PlanRefinement,
    /// Notice a repeated pattern.
    PatternWatcher,
    /// Respond to the user's affect.
    AffectResponder,
    /// Surface a question the memory cannot answer.
    KnowledgeGap,
}

impl BuiltIn {
    /// Every built-in.
    pub const fn all() -> [Self; 6] {
        [
            Self::SearchLookup,
            Self::GoalNarrowing,
            Self::PlanRefinement,
            Self::PatternWatcher,
            Self::AffectResponder,
            Self::KnowledgeGap,
        ]
    }

    /// The plugin id.
    pub const fn id(self) -> &'static str {
        match self {
            Self::SearchLookup => "search_lookup",
            Self::GoalNarrowing => "goal_narrowing",
            Self::PlanRefinement => "plan_refinement",
            Self::PatternWatcher => "pattern_watcher",
            Self::AffectResponder => "affect_responder",
            Self::KnowledgeGap => "knowledge_gap",
        }
    }

    /// The manifest.
    pub fn manifest(self) -> PluginManifest {
        let (name, triggers, capabilities) = match self {
            Self::SearchLookup => (
                "Search lookup",
                vec!["search".to_owned()],
                vec![PluginCapability::ReadMemory, PluginCapability::Propose],
            ),
            Self::GoalNarrowing => (
                "Goal narrowing",
                vec!["goal".to_owned()],
                vec![PluginCapability::ReadPreferences, PluginCapability::Propose],
            ),
            Self::PlanRefinement => (
                "Plan refinement",
                vec!["plan".to_owned()],
                vec![PluginCapability::ReadMemory, PluginCapability::Propose],
            ),
            Self::PatternWatcher => (
                "Pattern watcher",
                vec!["repeat".to_owned()],
                vec![PluginCapability::ReadPreferences, PluginCapability::Propose],
            ),
            Self::AffectResponder => (
                "Affect responder",
                vec!["affect".to_owned()],
                vec![PluginCapability::ReadPreferences, PluginCapability::Propose],
            ),
            Self::KnowledgeGap => (
                "Knowledge gap",
                vec!["question".to_owned()],
                vec![PluginCapability::ReadMemory, PluginCapability::Propose],
            ),
        };
        PluginManifest {
            id: self.id().to_owned(),
            name: name.to_owned(),
            version: "0.1.0".to_owned(),
            triggers,
            capabilities,
        }
    }

    /// Build the plugin.
    pub fn plugin(self) -> Box<dyn Plugin> {
        Box::new(BuiltInPlugin {
            manifest: self.manifest(),
            kind: self,
        })
    }
}

/// A built-in plugin instance.
#[derive(Debug)]
pub struct BuiltInPlugin {
    manifest: PluginManifest,
    kind: BuiltIn,
}

impl Plugin for BuiltInPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn evaluate(&self, ctx: &PluginContext<'_>) -> Vec<ProactiveProposal> {
        let query = ctx.query.to_lowercase();
        let (trigger, title, body, score) = match self.kind {
            BuiltIn::SearchLookup
                if ["search", "find", "look up", "lookup"]
                    .iter()
                    .any(|needle| query.contains(needle)) =>
            {
                (
                    "search",
                    "Search for this",
                    format!("Look up the sources for: {}", ctx.query.trim()),
                    0.70,
                )
            }
            BuiltIn::GoalNarrowing if query.contains("goal") || query.contains("priorit") => (
                "goal",
                "Narrow the goal",
                "Split this into the one goal that matters now.".to_owned(),
                0.55,
            ),
            BuiltIn::PlanRefinement if query.contains("plan") || query.contains("next step") => (
                "plan",
                "Refine the plan",
                "Turn the plan into ordered, verifiable steps.".to_owned(),
                0.60,
            ),
            BuiltIn::PatternWatcher if query.contains("again") || query.contains("still") => (
                "repeat",
                "Repeated pattern",
                "This came up before; reuse the earlier resolution.".to_owned(),
                0.45,
            ),
            BuiltIn::AffectResponder if ctx.affect.valence <= -0.3 => (
                "affect",
                "Acknowledge the frustration",
                "Name the friction before proposing the fix.".to_owned(),
                0.80,
            ),
            BuiltIn::KnowledgeGap if query.contains('?') && ctx.memory_hits.is_empty() => (
                "question",
                "Answer from first principles",
                "No memory hit; answer explicitly and flag the uncertainty.".to_owned(),
                0.65,
            ),
            _ => return Vec::new(),
        };
        vec![ProactiveProposal {
            plugin: self.manifest.id.clone(),
            trigger: trigger.to_owned(),
            title: title.to_owned(),
            body,
            score,
        }]
    }

    fn execute(&self, proposal: &ProactiveProposal) -> Result<PluginResult> {
        if proposal.plugin != self.manifest.id {
            return Err(CmeError::Plugin(format!(
                "proposal for `{}` was routed to `{}`",
                proposal.plugin, self.manifest.id
            )));
        }
        Ok(PluginResult {
            plugin: self.manifest.id.clone(),
            action: proposal.trigger.clone(),
            detail: format!("{}: {}", proposal.title, proposal.body),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(query: &'a str, hits: &'a [String]) -> PluginContext<'a> {
        static PROFILE: std::sync::OnceLock<PreferenceProfile> = std::sync::OnceLock::new();
        PluginContext {
            query,
            preferences: PROFILE.get_or_init(PreferenceProfile::default),
            affect: AffectState::default(),
            memory_hits: hits,
        }
    }

    #[test]
    fn builtins_are_discovered_by_trigger() {
        let host = PluginHost::with_builtins(PluginSandbox::default()).unwrap();
        assert_eq!(host.ids().len(), 6);
        assert!(host.contains("search_lookup"));
        let proposals = host.dispatch("search", &context("find the design", &[]));
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].plugin, "search_lookup");
        assert!(host
            .dispatch("unknown", &context("find the design", &[]))
            .is_empty());
    }

    #[test]
    fn questions_without_memory_propose_a_knowledge_gap() {
        let host = PluginHost::with_builtins(PluginSandbox::default()).unwrap();
        let empty = host.dispatch("question", &context("why is the cache cold?", &[]));
        assert_eq!(empty[0].plugin, "knowledge_gap");
        let hits = vec!["cache.md".to_owned()];
        assert!(host
            .dispatch("question", &context("why is the cache cold?", &hits))
            .is_empty());
    }

    #[test]
    fn sandbox_caps_proposals_and_executions() {
        let sandbox = PluginSandbox {
            max_proposals_per_plugin: 1,
            max_proposals_per_cycle: 2,
            max_executions_per_cycle: 1,
        };
        let mut host = PluginHost::with_builtins(sandbox).unwrap();
        let proposals = host.evaluate(&context("plan and search the goal?", &[]));
        assert_eq!(proposals.len(), 2);
        host.execute(&proposals[0]).unwrap();
        assert!(host.execute(&proposals[1]).is_err());
        host.reset_cycle();
        host.execute(&proposals[1]).unwrap();
    }

    #[test]
    fn duplicate_manifests_are_refused() {
        let mut host = PluginHost::default();
        host.load(BuiltIn::SearchLookup.plugin()).unwrap();
        assert!(host.load(BuiltIn::SearchLookup.plugin()).is_err());
        assert_eq!(host.ids(), vec!["search_lookup"]);
    }

    #[test]
    fn unloaded_plugins_stop_receiving_triggers() {
        let mut host = PluginHost::with_builtins(PluginSandbox::default()).unwrap();
        host.unload("search_lookup").unwrap();
        assert!(!host.contains("search_lookup"));
        assert!(host.dispatch("search", &context("find it", &[])).is_empty());
        assert!(host.unload("search_lookup").is_err());
    }
}
