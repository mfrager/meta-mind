//! Tier 4: user memory, transferred from every project the user works on.
//!
//! This is the global layer: identity, explicit and learned preferences,
//! expertise, communication patterns, preferred tools, a registry of projects,
//! and the narrative a new session can greet the user with.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::RdfEngine;
use crate::error::Result;
use crate::rdf::{self, user_iri, user_memory_graph};

use super::model::{
    ExpertiseLevel, PreferenceDimension, PreferenceState, ProjectId, ProjectStatus, Timestamp,
    UserId,
};
use super::project::ConventionRule;

/// Who the user is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UserIdentity {
    /// Display name.
    pub name: String,
    /// Email.
    pub email: Option<String>,
    /// Role (`Senior Rust engineer`).
    pub role: Option<String>,
}

/// An explicit preference value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreferenceValue {
    /// −1.0 to 1.0.
    pub value: f32,
    /// Where it was set.
    pub set_at: Timestamp,
}

/// A learned preference posterior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedPreference {
    /// Posterior mean, −1.0 to 1.0.
    pub mean: f32,
    /// Posterior variance.
    pub variance: f32,
    /// Observations.
    pub n_observations: u32,
    /// Last update.
    pub last_updated: Timestamp,
    /// Lifecycle state.
    pub state: PreferenceState,
}

impl LearnedPreference {
    /// A cold-start posterior centred on zero.
    #[must_use]
    pub fn cold_start() -> Self {
        Self {
            mean: 0.0,
            variance: 1.0,
            n_observations: 0,
            last_updated: Timestamp::now(),
            state: PreferenceState::ColdStart,
        }
    }

    /// Fold one observation into the posterior. A simple running mean with a
    /// shrinking variance; Phase 5 can replace it with the full Bayesian model.
    pub fn observe(&mut self, value: f32) {
        let n = self.n_observations.saturating_add(1);
        self.mean += (value - self.mean) / f32::from(n as u16).max(1.0);
        self.variance = (self.variance * f32::from(n.saturating_sub(1) as u16)
            / f32::from(n as u16).max(1.0))
        .max(0.0001);
        self.n_observations = n;
        self.state = match n {
            0..=2 => PreferenceState::Observing,
            3..=6 => PreferenceState::Converging,
            7..=19 => PreferenceState::Stable,
            _ => PreferenceState::Stable,
        };
        self.last_updated = Timestamp::now();
    }
}

/// The merged preference model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UserPreferenceModel {
    /// Values the user set explicitly.
    #[serde(default)]
    pub explicit: HashMap<String, PreferenceValue>,
    /// Values inferred from behaviour.
    #[serde(default)]
    pub learned: HashMap<String, LearnedPreference>,
    /// What the planner uses: explicit values win, otherwise the learned mean.
    #[serde(default)]
    pub merged: HashMap<String, f32>,
}

impl UserPreferenceModel {
    /// Recompute `merged` from the explicit and learned maps.
    pub fn merge(&mut self) {
        self.merged.clear();
        for dimension in PreferenceDimension::ALL {
            let key = dimension.as_str();
            let value = self
                .explicit
                .get(key)
                .map(|explicit| explicit.value)
                .or_else(|| self.learned.get(key).map(|learned| learned.mean));
            if let Some(value) = value {
                self.merged.insert(key.to_owned(), value);
            }
        }
        for (key, learned) in &self.learned {
            self.merged.entry(key.clone()).or_insert(learned.mean);
        }
    }
}

/// How the user likes to communicate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CommunicationStyle {
    /// Free-form description (`terse, technical, direct`).
    pub description: String,
    /// Preferred response length (`short`, `balanced`, `long`).
    pub length: Option<String>,
    /// Preferred tone.
    pub tone: Option<String>,
}

/// The state of the agent–user relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RelationshipState {
    /// How familiar the agent is with the user.
    pub familiarity: String,
    /// Last interaction.
    pub last_seen: Option<Timestamp>,
    /// Sessions so far.
    pub session_count: u32,
}

/// A known fact about the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    /// The fact.
    pub value: String,
    /// 0.0–1.0.
    pub confidence: f32,
    /// Where it came from.
    pub source: String,
    /// When it was learned.
    pub learned_at: Timestamp,
}

/// Learned communication patterns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CommunicationPatterns {
    /// Median query length in characters.
    pub median_query_chars: Option<u32>,
    /// Whether the user tends to interrupt.
    pub interrupts: bool,
    /// Preferred modality.
    pub preferred_modality: Option<String>,
}

/// A preferred tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPreference {
    /// Tool name.
    pub tool: String,
    /// When it is preferred.
    pub context: String,
    /// 0.0–1.0.
    pub confidence: f32,
    /// Project it was learned in.
    pub learned_from_project: Option<ProjectId>,
}

/// A project summary in the user's registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    /// Project name.
    pub name: String,
    /// Description.
    pub description: Option<String>,
    /// Status.
    pub status: ProjectStatus,
    /// Last activity.
    pub last_activity: Timestamp,
    /// Key topics.
    #[serde(default)]
    pub key_topics: Vec<String>,
}

/// Privacy controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyPolicies {
    /// Allow learning across projects.
    pub allow_cross_project_learning: bool,
    /// Allow preference learning.
    pub allow_preference_learning: bool,
    /// Days of inactivity before deletion; zero never deletes.
    pub auto_delete_on_inactivity_days: u32,
}

impl Default for PrivacyPolicies {
    fn default() -> Self {
        Self {
            allow_cross_project_learning: true,
            allow_preference_learning: true,
            auto_delete_on_inactivity_days: 0,
        }
    }
}

/// The global user memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMemory {
    /// User.
    pub user_id: UserId,
    /// Identity.
    pub identity: UserIdentity,
    /// Created.
    pub created_at: Timestamp,
    /// Updated.
    pub updated_at: Timestamp,
    /// Preferences.
    pub preferences: UserPreferenceModel,
    /// Communication style.
    pub communication_style: CommunicationStyle,
    /// Relationship.
    pub relationship: RelationshipState,
    /// Known facts.
    #[serde(default)]
    pub known_facts: HashMap<String, Fact>,
    /// Expertise by domain, 0–100.
    #[serde(default)]
    pub expertise_domains: HashMap<String, ExpertiseLevel>,
    /// Communication patterns.
    pub communication_patterns: CommunicationPatterns,
    /// Preferred tools.
    #[serde(default)]
    pub preferred_tools: Vec<ToolPreference>,
    /// Projects, keyed by id.
    #[serde(default)]
    pub projects: HashMap<String, ProjectSummary>,
    /// The global narrative.
    pub global_narrative: String,
    /// Privacy.
    pub privacy_policies: PrivacyPolicies,
}

impl UserMemory {
    /// A new user memory.
    pub fn new(user_id: impl Into<UserId>, name: impl Into<String>) -> Self {
        let now = Timestamp::now();
        Self {
            user_id: user_id.into(),
            identity: UserIdentity {
                name: name.into(),
                email: None,
                role: None,
            },
            created_at: now,
            updated_at: now,
            preferences: UserPreferenceModel::default(),
            communication_style: CommunicationStyle::default(),
            relationship: RelationshipState {
                familiarity: "new".to_owned(),
                last_seen: Some(now),
                session_count: 0,
            },
            known_facts: HashMap::new(),
            expertise_domains: HashMap::new(),
            communication_patterns: CommunicationPatterns::default(),
            preferred_tools: Vec::new(),
            projects: HashMap::new(),
            global_narrative: String::new(),
            privacy_policies: PrivacyPolicies::default(),
        }
    }

    /// Record that a session started.
    pub fn note_session(&mut self) {
        self.relationship.session_count = self.relationship.session_count.saturating_add(1);
        self.relationship.last_seen = Some(Timestamp::now());
        if self.relationship.session_count > 1 {
            self.relationship.familiarity = "returning".to_owned();
        }
        self.updated_at = Timestamp::now();
    }

    /// The strongest expertise domains, highest first.
    pub fn top_expertise(&self, limit: usize) -> Vec<(String, ExpertiseLevel)> {
        let mut domains: Vec<(String, ExpertiseLevel)> = self
            .expertise_domains
            .iter()
            .map(|(domain, level)| (domain.clone(), *level))
            .collect();
        domains.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        domains.truncate(limit);
        domains
    }
}

/// Convert a project convention into a user tool preference.
pub fn convention_to_preference(
    convention: &ConventionRule,
    project_id: &ProjectId,
) -> ToolPreference {
    ToolPreference {
        tool: convention
            .tool
            .clone()
            .unwrap_or_else(|| convention.context.clone()),
        context: convention.rule.clone(),
        confidence: convention.confidence,
        learned_from_project: Some(project_id.clone()),
    }
}

/// User-memory graph class.
pub const USER_MEMORY_CLASS: &str = "https://metamind.dev/memory#UserMemory";

/// The Tier-4 store.
#[derive(Debug, Clone)]
pub struct UserMemoryStore {
    engine: Arc<RdfEngine>,
}

impl UserMemoryStore {
    /// Build the store.
    pub fn new(engine: Arc<RdfEngine>) -> Self {
        Self { engine }
    }

    /// Load user memory.
    pub async fn load(&self, user_id: &UserId) -> Result<Option<UserMemory>> {
        rdf::load_as(
            &self.engine,
            &user_memory_graph(user_id.as_str()),
            &user_iri(user_id.as_str()),
        )
        .await
    }

    /// Save user memory.
    pub async fn save(&self, memory: &UserMemory) -> Result<()> {
        rdf::upsert(
            &self.engine,
            &user_memory_graph(memory.user_id.as_str()),
            &user_iri(memory.user_id.as_str()),
            USER_MEMORY_CLASS,
            Some(&memory.identity.name),
            &serde_json::to_value(memory)?,
        )
        .await
    }

    /// Load or create user memory.
    pub async fn load_or_create(&self, user_id: &UserId, name: &str) -> Result<UserMemory> {
        Ok(match self.load(user_id).await? {
            Some(memory) => memory,
            None => UserMemory::new(user_id.clone(), name),
        })
    }
}
