//! Tier 4 transfer: learn user-level facts from a project's memory.
//!
//! The learner only lifts high-confidence, durable facts: the tech stack (as
//! expertise), conventions above the confidence threshold (as tool preferences),
//! and a project summary. It never copies project-specific decisions into the
//! user's global memory.

use std::sync::Arc;

use crate::config::UserMemoryConfig;
use crate::error::{DataError, Result};

use super::model::{ExpertiseLevel, ProjectId, ProjectStatus, Timestamp, UserId};
use super::project::ProjectMemoryStore;
use super::user::{convention_to_preference, ProjectSummary, UserMemory, UserMemoryStore};

/// Transfers project memory into user memory.
#[derive(Debug, Clone)]
pub struct CrossProjectLearner {
    projects: Arc<ProjectMemoryStore>,
    users: Arc<UserMemoryStore>,
    config: UserMemoryConfig,
}

impl CrossProjectLearner {
    /// Build the learner.
    pub fn new(
        projects: Arc<ProjectMemoryStore>,
        users: Arc<UserMemoryStore>,
        config: UserMemoryConfig,
    ) -> Self {
        Self {
            projects,
            users,
            config,
        }
    }

    /// Sync one project into its user's global memory.
    pub async fn learn_from_project(
        &self,
        user_id: &UserId,
        project_id: &ProjectId,
    ) -> Result<UserMemory> {
        let project = self
            .projects
            .load(project_id)
            .await?
            .ok_or_else(|| DataError::NotFound {
                kind: "project memory",
                id: project_id.to_string(),
            })?;
        let mut user = self.users.load_or_create(user_id, &project.name).await?;
        if !user.privacy_policies.allow_cross_project_learning {
            return Ok(user);
        }

        if self.config.cross_project_learning_enabled {
            for language in &project.tech_stack.languages {
                raise(&mut user, language, 10);
            }
            for framework in &project.tech_stack.frameworks {
                raise(&mut user, framework, 5);
            }
            for infrastructure in &project.tech_stack.infrastructure {
                raise(&mut user, infrastructure, 3);
            }
            for tool in &project.tech_stack.tools {
                raise(&mut user, tool, 2);
            }

            for convention in &project.learned_conventions {
                if convention.confidence > 0.8 {
                    let preference = convention_to_preference(convention, project_id);
                    if let Some(existing) = user.preferred_tools.iter_mut().find(|existing| {
                        existing.tool == preference.tool && existing.context == preference.context
                    }) {
                        if preference.confidence >= existing.confidence {
                            *existing = preference;
                        }
                    } else {
                        user.preferred_tools.push(preference);
                    }
                }
            }
        }

        user.projects.insert(
            project_id.to_string(),
            ProjectSummary {
                name: project.name.clone(),
                description: project.description.clone(),
                status: project.status,
                last_activity: project.updated_at,
                key_topics: project.topic_index.keys().cloned().collect(),
            },
        );
        if self.config.global_narrative_enabled {
            user.global_narrative = regenerate_global_narrative(&user);
        }
        user.updated_at = Timestamp::now();
        self.users.save(&user).await?;
        Ok(user)
    }

    /// The user store.
    pub fn users(&self) -> &UserMemoryStore {
        &self.users
    }
}

fn raise(user: &mut UserMemory, domain: &str, by: u32) {
    let entry = user
        .expertise_domains
        .entry(domain.to_owned())
        .or_insert(ExpertiseLevel(0));
    *entry = entry.raised(by);
}

/// Compose the one-line global narrative from expertise, style and recency.
pub fn regenerate_global_narrative(user: &UserMemory) -> String {
    let top_expertise = user
        .top_expertise(1)
        .first()
        .map(|(domain, _)| domain.clone())
        .unwrap_or_else(|| "generalist".to_owned());
    let style = match user.preferences.merged.get("verbosity") {
        Some(value) if *value < -0.5 => "terse",
        Some(value) if *value > 0.5 => "verbose",
        _ => "balanced",
    };
    let current_project = user
        .projects
        .values()
        .filter(|project| project.status == ProjectStatus::Active)
        .max_by_key(|project| project.last_activity)
        .map(|project| project.name.clone())
        .unwrap_or_else(|| "none".to_owned());
    let role = user
        .identity
        .role
        .clone()
        .unwrap_or_else(|| "Software".to_owned());
    format!(
        "{role} focused on {top_expertise}, prefers {style} communication. Current project: {current_project}."
    )
}
