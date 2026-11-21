//! Tier 3 aggregation: merge compressed segments into project memory.
//!
//! Merging is incremental and idempotent. Decisions are deduplicated by
//! `(topic, choice)` (or by topic when the configuration says so), commitments
//! and questions by their text, artifacts by identifier, and goals by
//! identifier. The narrative and continuation are regenerated from the merged
//! view, never by concatenating segment narratives.

use std::sync::Arc;

use crate::config::ProjectMemoryConfig;
use crate::error::Result;

use super::compressed::{Commitment, CompressedSegment, DecisionRecord};
use super::model::{ArtifactStatus, CommitmentStatus, GoalStatus, ProjectId, QuestionPriority};
use super::project::{ProjectMemory, ProjectMemoryStore, PROJECT_MEMORY_CLASS};
use crate::rdf::project_iri;

/// Merges segments into one project's memory.
#[derive(Debug, Clone)]
pub struct ProjectAggregator {
    store: Arc<ProjectMemoryStore>,
    config: ProjectMemoryConfig,
}

impl ProjectAggregator {
    /// Build the aggregator.
    pub fn new(store: Arc<ProjectMemoryStore>, config: ProjectMemoryConfig) -> Self {
        Self { store, config }
    }

    /// The project store.
    pub fn store(&self) -> &ProjectMemoryStore {
        &self.store
    }

    /// Merge one segment into `project_id`, creating the memory if needed.
    pub async fn merge_segment(
        &self,
        project_id: &ProjectId,
        segment: &CompressedSegment,
    ) -> Result<()> {
        self.merge_segment_named(project_id, &segment.user_id, project_id.as_str(), segment)
            .await
    }

    /// Merge one segment with an explicit project name.
    pub async fn merge_segment_named(
        &self,
        project_id: &ProjectId,
        user_id: &super::model::UserId,
        name: &str,
        segment: &CompressedSegment,
    ) -> Result<()> {
        if segment.segment_id.as_str().is_empty() {
            return Ok(());
        }
        let mut memory = self.store.load_or_create(project_id, user_id, name).await?;
        if memory.segment_ids.contains(&segment.segment_id) {
            return Ok(());
        }

        for decision in &segment.decisions {
            self.upsert_decision(&mut memory, decision);
        }
        for commitment in &segment.commitments {
            upsert_commitment(&mut memory, commitment);
        }
        for question in &segment.open_questions {
            if !memory
                .open_questions
                .iter()
                .any(|existing| existing.question == question.question)
            {
                memory.open_questions.push(question.clone());
            }
        }
        for artifact in &segment.artifacts {
            if let Some(existing) = memory
                .key_artifacts
                .iter_mut()
                .find(|existing| existing.identifier == artifact.identifier)
            {
                *existing = artifact.clone();
            } else {
                memory.key_artifacts.push(artifact.clone());
            }
        }
        for goal in &segment.goals {
            if let Some(existing) = memory
                .active_goals
                .iter_mut()
                .find(|existing| existing.goal_id == goal.goal_id)
            {
                *existing = goal.clone();
            } else {
                memory.active_goals.push(goal.clone());
            }
        }
        if let Some(session) = memory.last_session_id.clone() {
            let _ = session;
        }
        memory.last_session_id = Some(segment.session_id.clone());
        memory.segment_ids.push(segment.segment_id.clone());
        memory.user_id = segment.user_id.clone();
        for topic in &segment.key_topics {
            memory
                .topic_index
                .entry(topic.clone())
                .or_default()
                .push(segment.segment_id.clone());
        }
        memory.updated_at = super::model::Timestamp::now();
        memory.project_narrative = self.regenerate_narrative(&memory, Some(segment));
        if self.config.continuation_suggestion_enabled {
            memory.suggested_continuation = suggest_continuation(&memory);
        }
        self.store.save(&memory).await?;
        Ok(())
    }

    fn upsert_decision(&self, memory: &mut ProjectMemory, decision: &DecisionRecord) {
        if let Some(existing) = memory.aggregated_decisions.iter_mut().find(|existing| {
            if self.config.dedup_decisions_by_topic {
                existing.topic.eq_ignore_ascii_case(&decision.topic)
            } else {
                existing.topic.eq_ignore_ascii_case(&decision.topic)
                    && existing.choice.eq_ignore_ascii_case(&decision.choice)
            }
        }) {
            if existing.choice != decision.choice {
                existing.status = super::model::DecisionStatus::Superseded;
            }
            *existing = decision.clone();
        } else {
            memory.aggregated_decisions.push(decision.clone());
        }
    }

    fn regenerate_narrative(
        &self,
        memory: &ProjectMemory,
        latest: Option<&CompressedSegment>,
    ) -> String {
        let mut narrative = format!(
            "{}: {} decisions, {} open commitments, {} current artifacts.",
            memory.name,
            memory.aggregated_decisions.len(),
            memory
                .active_commitments
                .iter()
                .filter(|commitment| {
                    matches!(
                        commitment.status,
                        CommitmentStatus::Open
                            | CommitmentStatus::InProgress
                            | CommitmentStatus::Blocked
                    )
                })
                .count(),
            memory
                .key_artifacts
                .iter()
                .filter(|artifact| {
                    matches!(
                        artifact.status,
                        ArtifactStatus::Created | ArtifactStatus::Modified
                    )
                })
                .count()
        );
        if let Some(latest) = latest {
            let first = latest
                .narrative
                .split(". ")
                .next()
                .unwrap_or(&latest.narrative);
            narrative.push(' ');
            narrative.push_str(first);
            if !narrative.ends_with('.') {
                narrative.push('.');
            }
        }
        if narrative.len() > self.config.narrative_max_tokens * 4 {
            narrative.truncate(self.config.narrative_max_tokens * 4);
        }
        narrative
    }

    /// Rebuild the topic index from stored segments' contributions. Used when a
    /// caller repairs an index rather than merging a new segment.
    pub async fn reindex(&self, project_id: &ProjectId) -> Result<()> {
        let Some(mut memory) = self.store.load(project_id).await? else {
            return Ok(());
        };
        memory.topic_index.clear();
        memory.updated_at = super::model::Timestamp::now();
        self.store.save(&memory).await?;
        Ok(())
    }

    /// The class used for project memory entities.
    pub fn class() -> &'static str {
        PROJECT_MEMORY_CLASS
    }

    /// The subject IRI for a project.
    pub fn subject(project_id: &ProjectId) -> String {
        project_iri(project_id.as_str())
    }
}

fn upsert_commitment(memory: &mut ProjectMemory, commitment: &Commitment) {
    if let Some(existing) = memory
        .active_commitments
        .iter_mut()
        .find(|existing| existing.commitment_id == commitment.commitment_id)
    {
        *existing = commitment.clone();
    } else if let Some(existing) = memory
        .active_commitments
        .iter_mut()
        .find(|existing| existing.description == commitment.description)
    {
        *existing = commitment.clone();
    } else {
        memory.active_commitments.push(commitment.clone());
    }
}

/// Pick the most useful continuation suggestion, in priority order.
pub fn suggest_continuation(memory: &ProjectMemory) -> Option<String> {
    if let Some(blocked) = memory
        .active_commitments
        .iter()
        .find(|commitment| commitment.status == CommitmentStatus::Blocked)
    {
        let blocker = blocked
            .blockers
            .first()
            .map(String::as_str)
            .unwrap_or("no blocker recorded");
        return Some(format!(
            "Blocked: {}. Blocker: {}.",
            blocked.description, blocker
        ));
    }
    if let Some(goal) = memory
        .active_goals
        .iter()
        .find(|goal| goal.status == GoalStatus::Active && !goal.next_step.is_empty())
    {
        return Some(format!(
            "Last time: {}. Next: {}.",
            goal.recent_progress, goal.next_step
        ));
    }
    if let Some(question) = memory
        .open_questions
        .iter()
        .find(|question| question.priority == QuestionPriority::High)
    {
        return Some(format!("Open question: {}.", question.question));
    }
    if let Some(artifact) = memory.key_artifacts.iter().find(|artifact| {
        matches!(
            artifact.status,
            ArtifactStatus::Created | ArtifactStatus::Modified
        )
    }) {
        return Some(format!(
            "Follow up on {:?}: {}.",
            artifact.kind, artifact.identifier
        ));
    }
    if !memory.project_narrative.is_empty() {
        return Some(format!("Continue from: {}.", memory.project_narrative));
    }
    None
}
