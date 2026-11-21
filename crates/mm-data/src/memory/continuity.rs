//! Session continuity: resume with the context a person expects.
//!
//! The continuity manager reads the three lower tiers and the previous session
//! and assembles the context a session starts with, including a greeting that
//! prefers a concrete continuation over a generic welcome.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::agentstream::AgentstreamStore;
use crate::error::Result;
use crate::rdf::now_ms;

use super::compressed::{ArtifactRef, Commitment, GoalProgress, OpenQuestion};
use super::episodic::EpisodicStore;
use super::model::{ProjectId, SessionId, TurnId, UserId};
use super::project::ConventionRule;
use super::project::ProjectMemoryStore;
use super::user::UserMemoryStore;

/// The project slice of a resume context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProjectContext {
    /// The project narrative.
    pub project_summary: String,
    /// Active goals.
    #[serde(default)]
    pub active_goals: Vec<GoalProgress>,
    /// Open or in-progress commitments.
    #[serde(default)]
    pub open_commitments: Vec<Commitment>,
    /// Recent artifacts.
    #[serde(default)]
    pub recent_artifacts: Vec<ArtifactRef>,
    /// Learned conventions.
    #[serde(default)]
    pub learned_conventions: Vec<ConventionRule>,
    /// Open questions.
    #[serde(default)]
    pub open_questions: Vec<OpenQuestion>,
    /// Suggested next step.
    pub suggested_continuation: Option<String>,
}

/// The user slice of a resume context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UserContext {
    /// Merged preferences.
    #[serde(default)]
    pub preferences: std::collections::HashMap<String, f32>,
    /// Communication style description.
    pub communication_style: String,
    /// Relationship familiarity.
    pub familiarity: String,
    /// Expertise by domain.
    #[serde(default)]
    pub expertise_domains: std::collections::HashMap<String, u8>,
    /// Global narrative.
    pub global_narrative: String,
}

/// The previous session's salient facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PreviousSessionContext {
    /// Session identifier.
    pub session_id: String,
    /// Turns in that session.
    pub turn_count: u64,
    /// Topics last discussed.
    #[serde(default)]
    pub last_topics: Vec<String>,
    /// Recently referenced artifacts.
    #[serde(default)]
    pub last_artifacts: Vec<ArtifactRef>,
    /// When it ended.
    pub ended_at: i64,
}

/// Everything a new session can start from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionContext {
    /// New session, when one was created.
    pub session_id: Option<SessionId>,
    /// User.
    pub user_id: UserId,
    /// Project.
    pub project_id: Option<ProjectId>,
    /// Project context.
    pub project_context: Option<ProjectContext>,
    /// User context.
    pub user_context: Option<UserContext>,
    /// Previous session.
    pub previous_session: Option<PreviousSessionContext>,
    /// A greeting for the user.
    pub greeting: String,
}

impl SessionContext {
    /// An empty context for a brand-new user.
    pub fn new(user_id: UserId, project_id: Option<ProjectId>) -> Self {
        Self {
            session_id: None,
            user_id,
            project_id,
            project_context: None,
            user_context: None,
            previous_session: None,
            greeting: "I'm Metamind. What should I call you?".to_owned(),
        }
    }
}

/// Loads resume context from the memory tiers.
#[derive(Debug, Clone)]
pub struct SessionContinuityManager {
    projects: Arc<ProjectMemoryStore>,
    users: Arc<UserMemoryStore>,
    episodic: Arc<EpisodicStore>,
    agentstream: Arc<AgentstreamStore>,
}

impl SessionContinuityManager {
    /// Build the manager.
    pub fn new(
        projects: Arc<ProjectMemoryStore>,
        users: Arc<UserMemoryStore>,
        episodic: Arc<EpisodicStore>,
        agentstream: Arc<AgentstreamStore>,
    ) -> Self {
        Self {
            projects,
            users,
            episodic,
            agentstream,
        }
    }

    /// Assemble the context for a new or resumed session.
    pub async fn resume_or_create_session(
        &self,
        user_id: &UserId,
        project_id: Option<&ProjectId>,
        previous_session_id: Option<&SessionId>,
    ) -> Result<SessionContext> {
        let mut context = SessionContext::new(user_id.clone(), project_id.cloned());

        if let Some(project_id) = project_id {
            if let Some(project) = self.projects.load(project_id).await? {
                context.project_context = Some(ProjectContext {
                    project_summary: project.project_narrative.clone(),
                    active_goals: project.active_goals().into_iter().cloned().collect(),
                    open_commitments: project.open_commitments().into_iter().cloned().collect(),
                    recent_artifacts: project.key_artifacts.iter().take(10).cloned().collect(),
                    learned_conventions: project.learned_conventions.clone(),
                    open_questions: project.open_questions.clone(),
                    suggested_continuation: project.suggested_continuation.clone(),
                });
            }
        }

        if let Some(user) = self.users.load(user_id).await? {
            context.user_context = Some(UserContext {
                preferences: user.preferences.merged.clone(),
                communication_style: user.communication_style.description.clone(),
                familiarity: user.relationship.familiarity.clone(),
                expertise_domains: user
                    .expertise_domains
                    .iter()
                    .map(|(domain, level)| (domain.clone(), level.0))
                    .collect(),
                global_narrative: user.global_narrative.clone(),
            });
        }

        if let Some(previous) = previous_session_id {
            if let Some(record) = self.agentstream.session(previous.as_str()).await? {
                let turns = self.episodic.turns(previous).await?;
                let mut topics: Vec<String> = Vec::new();
                for turn in &turns {
                    for word in super::compressor::key_topics(&turn.transcript(), 3).0 {
                        if !topics.contains(&word) {
                            topics.push(word);
                        }
                    }
                }
                topics.truncate(5);
                context.previous_session = Some(PreviousSessionContext {
                    session_id: previous.to_string(),
                    turn_count: record.event_count,
                    last_topics: topics,
                    last_artifacts: Vec::new(),
                    ended_at: record.last_event_at,
                });
            }
        }

        context.greeting = generate_greeting(&context);
        Ok(context)
    }

    /// The topics discussed in a session.
    pub async fn session_topics(&self, session_id: &SessionId) -> Result<Vec<String>> {
        let turns = self.episodic.turns(session_id).await?;
        let mut topics = Vec::new();
        for turn in turns {
            for topic in super::compressor::key_topics(&turn.transcript(), 3).0 {
                if !topics.contains(&topic) {
                    topics.push(topic);
                }
            }
        }
        Ok(topics)
    }

    /// Turn ids of a session, for provenance displays.
    pub async fn turn_ids(&self, session_id: &SessionId) -> Result<Vec<TurnId>> {
        Ok(self
            .episodic
            .turns(session_id)
            .await?
            .into_iter()
            .map(|turn| turn.turn_id)
            .collect())
    }
}

/// The greeting policy from the design: concrete continuation first, then a
/// project welcome, then a returning-user welcome, then an introduction.
pub fn generate_greeting(context: &SessionContext) -> String {
    if let Some(project) = &context.project_context {
        if let Some(continuation) = &project.suggested_continuation {
            return format!("Back at it. {continuation}");
        }
        if let Some(previous) = &context.previous_session {
            let hours = (now_ms() - previous.ended_at).max(0) / 3_600_000;
            if hours < 24 {
                let first = first_sentence(&project.project_summary);
                return format!("Welcome back. Still on {first}?");
            }
        }
        let first = first_sentence(&project.project_summary);
        return format!("Back to {first}. What's next?");
    }
    if context.user_context.is_some() {
        return "Back at it. What are we working on?".to_owned();
    }
    "I'm Metamind. What should I call you?".to_owned()
}

fn first_sentence(text: &str) -> &str {
    text.split(['.', '!', '?', ':'])
        .next()
        .map(str::trim)
        .filter(|sentence| !sentence.is_empty())
        .unwrap_or("the project")
}
