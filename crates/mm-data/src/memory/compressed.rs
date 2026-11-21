//! Tier 2: compressed segments and their structured artifacts.
//!
//! A segment is the output of the compression pipeline: a short causal
//! narrative, the decisions/commitments/questions/artifacts/goals extracted
//! from the turns, an embedding, and provenance back to the source turns. It is
//! stored once in its own graph and mirrored in a segment index for retrieval.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::RdfEngine;
use crate::error::Result;
use crate::rdf::{self, new_id, segment_graph, segment_iri};

use super::model::{
    AlternativeId, ArtifactId, ArtifactKind, ArtifactStatus, CommitmentId, CommitmentStatus,
    DecisionId, DecisionStatus, EntityId, GoalId, GoalStatus, ProjectId, QuestionId,
    QuestionPriority, SegmentId, SessionId, SubgoalId, Timestamp, TurnId, UserId, Vector,
};

/// A rejected alternative in a decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alternative {
    /// Identifier.
    pub alternative_id: AlternativeId,
    /// What was considered.
    pub description: String,
    /// Why it was not chosen.
    pub reason: String,
}

/// A decision made in the conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRecord {
    /// Identifier.
    pub decision_id: DecisionId,
    /// What the decision was about.
    pub topic: String,
    /// What was chosen.
    pub choice: String,
    /// Why.
    pub rationale: String,
    /// Rejected options.
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
    /// 0.0–1.0.
    pub confidence: f32,
    /// Lifecycle state.
    pub status: DecisionStatus,
    /// Who decided.
    pub decided_by: EntityId,
    /// Source turns.
    pub turn_ids: Vec<TurnId>,
    /// Related artifacts.
    #[serde(default)]
    pub related_artifacts: Vec<ArtifactRef>,
    /// Project.
    pub project_id: Option<ProjectId>,
}

/// A commitment someone made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Commitment {
    /// Identifier.
    pub commitment_id: CommitmentId,
    /// What was promised.
    pub description: String,
    /// Who owns it.
    pub owner: EntityId,
    /// Optional deadline.
    pub deadline: Option<Timestamp>,
    /// Lifecycle state.
    pub status: CommitmentStatus,
    /// 0.0–1.0.
    pub progress: f32,
    /// Blockers.
    #[serde(default)]
    pub blockers: Vec<String>,
    /// Turn that created it.
    pub created_turn: Option<TurnId>,
    /// Turn that last changed it.
    pub updated_turn: Option<TurnId>,
    /// Related decisions.
    #[serde(default)]
    pub related_decisions: Vec<DecisionId>,
    /// Related artifacts.
    #[serde(default)]
    pub related_artifacts: Vec<ArtifactRef>,
    /// Project.
    pub project_id: Option<ProjectId>,
}

/// A question left open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenQuestion {
    /// Identifier.
    pub question_id: QuestionId,
    /// The question.
    pub question: String,
    /// Surrounding context.
    pub context: String,
    /// Who asked it.
    pub asked_by: EntityId,
    /// When.
    pub asked_at: Timestamp,
    /// Source turn.
    pub turn_id: Option<TurnId>,
    /// Priority.
    pub priority: QuestionPriority,
    /// A tentative answer, if one was offered.
    pub tentative_answer: Option<String>,
    /// Related decisions.
    #[serde(default)]
    pub related_decisions: Vec<DecisionId>,
    /// Project.
    pub project_id: Option<ProjectId>,
}

/// A file, PR, commit, document or similar produced by the work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// Identifier.
    pub artifact_id: ArtifactId,
    /// Kind.
    pub kind: ArtifactKind,
    /// Human-readable identifier (`PR #247`, `src/lib.rs`).
    pub identifier: String,
    /// What it is.
    pub description: String,
    /// URL or path.
    pub location: Option<String>,
    /// Source turn.
    pub created_turn: Option<TurnId>,
    /// Lifecycle state.
    pub status: ArtifactStatus,
    /// Project.
    pub project_id: Option<ProjectId>,
}

/// A subgoal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subgoal {
    /// Identifier.
    pub subgoal_id: SubgoalId,
    /// Description.
    pub description: String,
    /// Whether it is complete.
    pub done: bool,
}

/// Progress on a goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalProgress {
    /// Identifier.
    pub goal_id: GoalId,
    /// Description.
    pub description: String,
    /// Lifecycle state.
    pub status: GoalStatus,
    /// 0.0–1.0.
    pub progress: f32,
    /// Subgoals.
    #[serde(default)]
    pub subgoals: Vec<Subgoal>,
    /// Related commitments.
    #[serde(default)]
    pub related_commitments: Vec<CommitmentId>,
    /// Last update.
    pub last_updated: Timestamp,
    /// A concrete next step, when known.
    #[serde(default)]
    pub next_step: String,
    /// What happened most recently.
    #[serde(default)]
    pub recent_progress: String,
    /// Project.
    pub project_id: Option<ProjectId>,
}

/// A preference inferred from behaviour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferredPreference {
    /// Dimension name (`verbosity`, …).
    pub dimension: String,
    /// Value, −1.0 to 1.0.
    pub value: f32,
    /// Confidence, 0.0–1.0.
    pub confidence: f32,
    /// Evidence turns.
    #[serde(default)]
    pub evidence_turns: Vec<TurnId>,
}

/// Quality measurements for a segment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct SegmentQuality {
    /// Decision recall.
    pub decision_recall: f32,
    /// Commitment recall.
    pub commitment_recall: f32,
    /// Artifact recall.
    pub artifact_recall: f32,
    /// Narrative faithfulness.
    pub narrative_faithfulness: f32,
    /// Hallucination rate.
    pub hallucination_rate: f32,
}

/// A compressed segment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompressedSegment {
    /// Identifier.
    pub segment_id: SegmentId,
    /// Session.
    pub session_id: SessionId,
    /// Project.
    pub project_id: Option<ProjectId>,
    /// User.
    pub user_id: UserId,
    /// First and last source turn.
    pub turn_range: (TurnId, TurnId),
    /// Tokens in the source turns.
    pub token_count: usize,
    /// When it was created.
    pub created_at: Timestamp,
    /// Causal narrative.
    pub narrative: String,
    /// Extracted decisions.
    #[serde(default)]
    pub decisions: Vec<DecisionRecord>,
    /// Extracted commitments.
    #[serde(default)]
    pub commitments: Vec<Commitment>,
    /// Extracted questions.
    #[serde(default)]
    pub open_questions: Vec<OpenQuestion>,
    /// Extracted artifacts.
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    /// Extracted goals.
    #[serde(default)]
    pub goals: Vec<GoalProgress>,
    /// Inferred preferences.
    #[serde(default)]
    pub inferred_preferences: Vec<InferredPreference>,
    /// Embedding.
    pub embedding: Vector,
    /// Key topics.
    #[serde(default)]
    pub key_topics: Vec<String>,
    /// Topic weights.
    #[serde(default)]
    pub topic_weights: HashMap<String, f32>,
    /// Compression ratio.
    pub compression_ratio: f32,
    /// Quality metrics.
    #[serde(default)]
    pub quality: SegmentQuality,
    /// Source turns.
    #[serde(default)]
    pub source_turn_ids: Vec<TurnId>,
    /// Models used.
    pub compression_model: String,
    /// Extraction model.
    pub extraction_model: String,
    /// Embedding model.
    pub embedding_model: String,
}

impl CompressedSegment {
    /// A segment skeleton with a fresh id and timestamp.
    pub fn new(
        session_id: impl Into<SessionId>,
        user_id: impl Into<UserId>,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            segment_id: SegmentId::new(new_id()),
            session_id: session_id.into(),
            project_id,
            user_id: user_id.into(),
            turn_range: (TurnId::from(""), TurnId::from("")),
            token_count: 0,
            created_at: Timestamp::now(),
            narrative: String::new(),
            decisions: Vec::new(),
            commitments: Vec::new(),
            open_questions: Vec::new(),
            artifacts: Vec::new(),
            goals: Vec::new(),
            inferred_preferences: Vec::new(),
            embedding: Vec::new(),
            key_topics: Vec::new(),
            topic_weights: HashMap::new(),
            compression_ratio: 1.0,
            quality: SegmentQuality::default(),
            source_turn_ids: Vec::new(),
            compression_model: String::new(),
            extraction_model: String::new(),
            embedding_model: String::new(),
        }
    }
}

/// Filters for semantic search.
#[derive(Debug, Clone, Default)]
pub struct SearchFilters {
    /// Restrict to a user.
    pub user_id: Option<UserId>,
    /// Restrict to a project.
    pub project_id: Option<ProjectId>,
    /// Restrict to a session.
    pub session_id: Option<SessionId>,
    /// Minimum cosine similarity.
    pub min_score: Option<f32>,
}

/// One semantic-search hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    /// The segment.
    pub segment: CompressedSegment,
    /// Cosine similarity to the query.
    pub score: f32,
}

/// Segment graph class.
pub const SEGMENT_CLASS: &str = "https://metamind.dev/memory#CompressedSegment";

/// The Tier-2 store.
#[derive(Debug, Clone)]
pub struct SegmentStore {
    engine: Arc<RdfEngine>,
}

impl SegmentStore {
    /// Build the store.
    pub fn new(engine: Arc<RdfEngine>) -> Self {
        Self { engine }
    }

    /// Persist a segment in its own graph and in the index.
    pub async fn save(&self, segment: &CompressedSegment) -> Result<()> {
        let graph = segment_graph(segment.segment_id.as_str());
        let subject = segment_iri(segment.segment_id.as_str());
        let payload = serde_json::to_value(segment)?;
        rdf::upsert(
            &self.engine,
            &graph,
            &subject,
            SEGMENT_CLASS,
            Some(&segment.narrative),
            &payload,
        )
        .await?;
        rdf::upsert(
            &self.engine,
            rdf::SEGMENT_INDEX_GRAPH,
            &subject,
            SEGMENT_CLASS,
            Some(&segment.narrative),
            &payload,
        )
        .await?;
        Ok(())
    }

    /// One segment by id.
    pub async fn get(&self, segment_id: &SegmentId) -> Result<Option<CompressedSegment>> {
        rdf::load_as(
            &self.engine,
            rdf::SEGMENT_INDEX_GRAPH,
            &segment_iri(segment_id.as_str()),
        )
        .await
    }

    /// Every segment, optionally filtered by user and project.
    pub async fn all(
        &self,
        user_id: Option<&UserId>,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<CompressedSegment>> {
        let mut segments: Vec<CompressedSegment> =
            rdf::list_as(&self.engine, rdf::SEGMENT_INDEX_GRAPH, SEGMENT_CLASS).await?;
        segments.retain(|segment| {
            user_id.is_none_or(|user| &segment.user_id == user)
                && project_id.is_none_or(|project| segment.project_id.as_ref() == Some(project))
        });
        segments.sort_by_key(|segment| segment.created_at);
        Ok(segments)
    }

    /// Segments mentioning `topic`.
    pub async fn find_by_topic(
        &self,
        topic: &str,
        user_id: Option<&UserId>,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<CompressedSegment>> {
        let needle = topic.to_lowercase();
        Ok(self
            .all(user_id, project_id)
            .await?
            .into_iter()
            .filter(|segment| {
                segment
                    .key_topics
                    .iter()
                    .any(|candidate| candidate.to_lowercase().contains(&needle))
                    || segment.narrative.to_lowercase().contains(&needle)
            })
            .collect())
    }

    /// Cosine similarity over the stored embeddings.
    pub async fn search(
        &self,
        query: &[f32],
        filters: &SearchFilters,
        k: usize,
    ) -> Result<Vec<SearchResult>> {
        let mut hits: Vec<SearchResult> = self
            .all(filters.user_id.as_ref(), filters.project_id.as_ref())
            .await?
            .into_iter()
            .filter(|segment| {
                filters
                    .session_id
                    .as_ref()
                    .is_none_or(|session| &segment.session_id == session)
            })
            .map(|segment| SearchResult {
                score: cosine(query, &segment.embedding),
                segment,
            })
            .filter(|hit| filters.min_score.is_none_or(|min| hit.score >= min))
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(k);
        Ok(hits)
    }

    /// The number of stored segments.
    pub async fn count(&self) -> Result<usize> {
        Ok(self.all(None, None).await?.len())
    }
}

/// Cosine similarity. A zero vector scores zero rather than dividing by zero.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}
