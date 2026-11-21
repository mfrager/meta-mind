//! Tier 3: project memory, aggregated from compressed segments.
//!
//! Project memory is the durable view a resumed session reads: the decisions
//! still in force, the commitments that are open, the artifacts that matter,
//! the conventions the team follows, and a narrative plus a concrete suggested
//! continuation.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::RdfEngine;
use crate::error::Result;
use crate::rdf::{self, project_iri, project_memory_graph};

use super::compressed::{ArtifactRef, Commitment, DecisionRecord, GoalProgress, OpenQuestion};
use super::model::{
    ArtifactStatus, CommitmentStatus, GoalStatus, ProjectId, ProjectStatus, RuleId, SegmentId,
    Timestamp, UserId,
};

/// A learned convention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConventionRule {
    /// Identifier.
    pub rule_id: RuleId,
    /// Where it applies (`git commit messages`).
    pub context: String,
    /// The rule.
    pub rule: String,
    /// Tool it belongs to.
    pub tool: Option<String>,
    /// 0.0–1.0.
    pub confidence: f32,
    /// Turns it was learned from.
    #[serde(default)]
    pub learned_from_turns: Vec<super::model::TurnId>,
    /// Project.
    pub project_id: ProjectId,
}

/// The project's technology stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TechStack {
    /// Languages.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Frameworks and libraries.
    #[serde(default)]
    pub frameworks: Vec<String>,
    /// Infrastructure.
    #[serde(default)]
    pub infrastructure: Vec<String>,
    /// Command-line tools.
    #[serde(default)]
    pub tools: Vec<String>,
}

/// A deployment target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentTarget {
    /// Name (`prod`, `staging`).
    pub name: String,
    /// Platform (`kubernetes`, `fly.io`).
    pub platform: String,
    /// Region or cluster.
    pub region: Option<String>,
}

/// The aggregated project memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectMemory {
    /// Project.
    pub project_id: ProjectId,
    /// Owning user.
    pub user_id: UserId,
    /// Name.
    pub name: String,
    /// Description.
    pub description: Option<String>,
    /// Created.
    pub created_at: Timestamp,
    /// Updated.
    pub updated_at: Timestamp,
    /// Aggregated decisions.
    #[serde(default)]
    pub aggregated_decisions: Vec<DecisionRecord>,
    /// Active commitments.
    #[serde(default)]
    pub active_commitments: Vec<Commitment>,
    /// Open questions.
    #[serde(default)]
    pub open_questions: Vec<OpenQuestion>,
    /// Key artifacts.
    #[serde(default)]
    pub key_artifacts: Vec<ArtifactRef>,
    /// Active goals.
    #[serde(default)]
    pub active_goals: Vec<GoalProgress>,
    /// Learned conventions.
    #[serde(default)]
    pub learned_conventions: Vec<ConventionRule>,
    /// Tech stack.
    #[serde(default)]
    pub tech_stack: TechStack,
    /// Deployment targets.
    #[serde(default)]
    pub deployment_targets: Vec<DeploymentTarget>,
    /// Segments that contributed.
    #[serde(default)]
    pub segment_ids: Vec<SegmentId>,
    /// Topic → segments.
    #[serde(default)]
    pub topic_index: HashMap<String, Vec<SegmentId>>,
    /// Last session.
    pub last_session_id: Option<super::model::SessionId>,
    /// What to do next.
    pub suggested_continuation: Option<String>,
    /// The project narrative.
    pub project_narrative: String,
    /// Status.
    pub status: ProjectStatus,
    /// Members.
    #[serde(default)]
    pub members: Vec<UserId>,
    /// Datasets.
    #[serde(default)]
    pub datasets: Vec<super::model::DatasetId>,
}

impl ProjectMemory {
    /// An empty memory for a project.
    pub fn new(
        project_id: impl Into<ProjectId>,
        user_id: impl Into<UserId>,
        name: impl Into<String>,
    ) -> Self {
        let now = Timestamp::now();
        let user_id = user_id.into();
        Self {
            project_id: project_id.into(),
            user_id: user_id.clone(),
            name: name.into(),
            description: None,
            created_at: now,
            updated_at: now,
            aggregated_decisions: Vec::new(),
            active_commitments: Vec::new(),
            open_questions: Vec::new(),
            key_artifacts: Vec::new(),
            active_goals: Vec::new(),
            learned_conventions: Vec::new(),
            tech_stack: TechStack::default(),
            deployment_targets: Vec::new(),
            segment_ids: Vec::new(),
            topic_index: HashMap::new(),
            last_session_id: None,
            suggested_continuation: None,
            project_narrative: String::new(),
            status: ProjectStatus::Active,
            members: vec![user_id],
            datasets: Vec::new(),
        }
    }

    /// Commitments that are open or in progress.
    pub fn open_commitments(&self) -> Vec<&Commitment> {
        self.active_commitments
            .iter()
            .filter(|commitment| {
                matches!(
                    commitment.status,
                    CommitmentStatus::Open
                        | CommitmentStatus::InProgress
                        | CommitmentStatus::Blocked
                )
            })
            .collect()
    }

    /// Artifacts that are new or recently changed.
    pub fn current_artifacts(&self) -> Vec<&ArtifactRef> {
        self.key_artifacts
            .iter()
            .filter(|artifact| {
                matches!(
                    artifact.status,
                    ArtifactStatus::Created | ArtifactStatus::Modified
                )
            })
            .collect()
    }

    /// Active goals.
    pub fn active_goals(&self) -> Vec<&GoalProgress> {
        self.active_goals
            .iter()
            .filter(|goal| goal.status == GoalStatus::Active)
            .collect()
    }
}

/// Project-memory graph class.
pub const PROJECT_MEMORY_CLASS: &str = "https://metamind.dev/memory#ProjectMemory";

/// The Tier-3 store.
#[derive(Debug, Clone)]
pub struct ProjectMemoryStore {
    engine: Arc<RdfEngine>,
}

impl ProjectMemoryStore {
    /// Build the store.
    pub fn new(engine: Arc<RdfEngine>) -> Self {
        Self { engine }
    }

    /// Load project memory.
    pub async fn load(&self, project_id: &ProjectId) -> Result<Option<ProjectMemory>> {
        rdf::load_as(
            &self.engine,
            &project_memory_graph(project_id.as_str()),
            &project_iri(project_id.as_str()),
        )
        .await
    }

    /// Save project memory.
    pub async fn save(&self, memory: &ProjectMemory) -> Result<()> {
        rdf::upsert(
            &self.engine,
            &project_memory_graph(memory.project_id.as_str()),
            &project_iri(memory.project_id.as_str()),
            PROJECT_MEMORY_CLASS,
            Some(&memory.name),
            &serde_json::to_value(memory)?,
        )
        .await
    }

    /// Load or create empty memory.
    pub async fn load_or_create(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        name: &str,
    ) -> Result<ProjectMemory> {
        Ok(match self.load(project_id).await? {
            Some(memory) => memory,
            None => ProjectMemory::new(project_id.clone(), user_id.clone(), name),
        })
    }
}
