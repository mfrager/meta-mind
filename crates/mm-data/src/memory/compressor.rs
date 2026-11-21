//! The compression pipeline: episodic turns → compressed segment.
//!
//! Four stages mirror the design: structured extraction, narrative generation,
//! embedding + topics, and optional quality validation. The default extractor,
//! narrator and embedder are deterministic and offline, so the pipeline runs in
//! tests and on a machine with no model access. `MemoryExtractor`,
//! `NarrativeGenerator` and `Embedder` are the seams Phase 5 plugs an LLM into;
//! no caller changes when that happens.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::config::{CompressionConfig, EpisodicConfig, SegmentConfig};
use crate::error::{DataError, Result};

use super::compressed::{
    ArtifactRef, Commitment, CompressedSegment, DecisionRecord, GoalProgress, InferredPreference,
    OpenQuestion, SegmentQuality, SegmentStore,
};
use super::episodic::{EpisodicStore, EpisodicTurn};
use super::model::{
    ArtifactId, ArtifactKind, ArtifactStatus, CommitmentId, CommitmentStatus, DecisionId,
    DecisionStatus, EntityId, GoalId, GoalStatus, ProjectId, QuestionId, QuestionPriority,
    SegmentId, SessionId, Timestamp, TurnId, UserId, Vector,
};
use crate::rdf::new_id;

/// The structured output of extraction.
#[derive(Debug, Clone, Default)]
pub struct Extraction {
    /// Decisions.
    pub decisions: Vec<DecisionRecord>,
    /// Commitments.
    pub commitments: Vec<Commitment>,
    /// Open questions.
    pub open_questions: Vec<OpenQuestion>,
    /// Artifacts.
    pub artifacts: Vec<ArtifactRef>,
    /// Goals.
    pub goals: Vec<GoalProgress>,
    /// Inferred preferences.
    pub inferred_preferences: Vec<InferredPreference>,
}

/// Structured extraction from a turn transcript.
#[async_trait]
pub trait MemoryExtractor: Send + Sync {
    /// Extract artifacts from the turns in one segment.
    async fn extract(&self, turns: &[EpisodicTurn]) -> Result<Extraction>;
}

/// Narrative generation from turns and extraction.
#[async_trait]
pub trait NarrativeGenerator: Send + Sync {
    /// Write the causal narrative.
    async fn narrate(&self, turns: &[EpisodicTurn], extraction: &Extraction) -> Result<String>;
}

/// Text embedding.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed one string.
    async fn embed(&self, text: &str) -> Result<Vector>;

    /// The model name.
    fn model(&self) -> &str;

    /// The vector dimension.
    fn dimension(&self) -> usize;
}

/// The deterministic extractor used when no model is configured.
///
/// It reads explicit phrasing (`decided`, `will`, `?`, paths, `PR #n`) rather
/// than guessing. That is deliberately conservative: a missed artifact costs
/// less than an invented one, and the segment's quality metrics make the gap
/// visible when an LLM extractor replaces it.
#[derive(Debug, Clone, Default)]
pub struct HeuristicExtractor {
    user_id: Option<UserId>,
    project_id: Option<ProjectId>,
}

impl HeuristicExtractor {
    /// An extractor that attributes records to the given user and project.
    pub fn new(user_id: Option<UserId>, project_id: Option<ProjectId>) -> Self {
        Self {
            user_id,
            project_id,
        }
    }
}

#[async_trait]
impl MemoryExtractor for HeuristicExtractor {
    async fn extract(&self, turns: &[EpisodicTurn]) -> Result<Extraction> {
        let mut extraction = Extraction::default();
        for turn in turns {
            let transcript = turn.transcript();
            for line in transcript.lines() {
                let lower = line.to_lowercase();
                if let Some(rest) = after_any(&lower, &["decided ", "decision: "]) {
                    extraction.decisions.push(DecisionRecord {
                        decision_id: DecisionId::new(new_id()),
                        topic: summarize_topic(&lower),
                        choice: rest.trim().to_owned(),
                        rationale: format!("From turn {}.", turn.turn_number),
                        alternatives: Vec::new(),
                        confidence: 0.75,
                        status: DecisionStatus::Decided,
                        decided_by: EntityId::agent(),
                        turn_ids: vec![turn.turn_id.clone()],
                        related_artifacts: Vec::new(),
                        project_id: self.project_id.clone(),
                    });
                }
                if lower.contains("will ") || lower.contains("todo") || lower.contains("next step")
                {
                    extraction.commitments.push(Commitment {
                        commitment_id: CommitmentId::new(new_id()),
                        description: line.trim().to_owned(),
                        owner: EntityId::agent(),
                        deadline: None,
                        status: CommitmentStatus::Open,
                        progress: 0.0,
                        blockers: Vec::new(),
                        created_turn: Some(turn.turn_id.clone()),
                        updated_turn: None,
                        related_decisions: Vec::new(),
                        related_artifacts: Vec::new(),
                        project_id: self.project_id.clone(),
                    });
                }
                if line.trim_end().ends_with('?') {
                    extraction.open_questions.push(OpenQuestion {
                        question_id: QuestionId::new(new_id()),
                        question: line.trim().to_owned(),
                        context: format!("Turn {}.", turn.turn_number),
                        asked_by: self
                            .user_id
                            .as_ref()
                            .map(EntityId::user)
                            .unwrap_or_else(EntityId::agent),
                        asked_at: turn.timestamp,
                        turn_id: Some(turn.turn_id.clone()),
                        priority: QuestionPriority::Medium,
                        tentative_answer: None,
                        related_decisions: Vec::new(),
                        project_id: self.project_id.clone(),
                    });
                }
            }
            extraction
                .artifacts
                .extend(extract_artifacts(turn, self.project_id.clone()));
            if let Some(goal) = extract_goal(turn, self.project_id.clone()) {
                extraction.goals.push(goal);
            }
            if let Some(preference) = extract_preference(turn) {
                extraction.inferred_preferences.push(preference);
            }
        }
        Ok(extraction)
    }
}

fn after_any<'a>(line: &'a str, needles: &[&str]) -> Option<&'a str> {
    needles
        .iter()
        .find_map(|needle| line.find(needle).map(|index| &line[index + needle.len()..]))
}

fn summarize_topic(line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace().take(6).collect();
    words.join(" ")
}

fn extract_artifacts(turn: &EpisodicTurn, project_id: Option<ProjectId>) -> Vec<ArtifactRef> {
    let text = turn.transcript();
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| {
            matches!(c, '(' | ')' | ',' | '.' | ':' | ';' | '"' | '\'' | '`')
        });
        let (kind, identifier) = if cleaned.starts_with("PR#") || cleaned.starts_with("PR") {
            (ArtifactKind::PullRequest, cleaned.to_owned())
        } else if cleaned.contains('/') && cleaned.contains('.') {
            (ArtifactKind::File, cleaned.to_owned())
        } else if cleaned.starts_with('#') && cleaned.len() > 1 {
            (ArtifactKind::Commit, cleaned.to_owned())
        } else {
            continue;
        };
        out.push(ArtifactRef {
            artifact_id: ArtifactId::new(new_id()),
            kind,
            identifier: identifier.clone(),
            description: format!("Mentioned in turn {}.", turn.turn_number),
            location: None,
            created_turn: Some(turn.turn_id.clone()),
            status: ArtifactStatus::Modified,
            project_id: project_id.clone(),
        });
    }
    out
}

fn extract_goal(turn: &EpisodicTurn, project_id: Option<ProjectId>) -> Option<GoalProgress> {
    let text = turn.transcript();
    let line = text
        .lines()
        .find(|line| line.to_lowercase().contains("goal:"))?;
    let description = line
        .split_once("goal:")
        .map(|(_, rest)| rest.trim())?
        .to_owned();
    if description.is_empty() {
        return None;
    }
    Some(GoalProgress {
        goal_id: GoalId::new(new_id()),
        description,
        status: GoalStatus::Active,
        progress: 0.0,
        subgoals: Vec::new(),
        related_commitments: Vec::new(),
        last_updated: turn.timestamp,
        next_step: String::new(),
        recent_progress: format!("Turn {}.", turn.turn_number),
        project_id,
    })
}

fn extract_preference(turn: &EpisodicTurn) -> Option<InferredPreference> {
    let text = turn.transcript().to_lowercase();
    let (dimension, value) = if text.contains("be brief") || text.contains("shorter") {
        ("verbosity", -0.6)
    } else if text.contains("more detail") || text.contains("explain more") {
        ("verbosity", 0.6)
    } else {
        return None;
    };
    Some(InferredPreference {
        dimension: dimension.to_owned(),
        value,
        confidence: 0.5,
        evidence_turns: vec![turn.turn_id.clone()],
    })
}

/// A deterministic narrator: one sentence per stage of the segment.
#[derive(Debug, Clone, Default)]
pub struct TemplateNarrator;

#[async_trait]
impl NarrativeGenerator for TemplateNarrator {
    async fn narrate(&self, turns: &[EpisodicTurn], extraction: &Extraction) -> Result<String> {
        let first = turns.first().map(|turn| turn.turn_number).unwrap_or(0);
        let last = turns.last().map(|turn| turn.turn_number).unwrap_or(0);
        let user = turns
            .iter()
            .find_map(|turn| turn.user_query.as_deref())
            .unwrap_or("the user's request");
        let response = turns
            .iter()
            .map(|turn| turn.agent_response.text.as_str())
            .find(|text| !text.is_empty())
            .unwrap_or("the agent responded");
        let mut narrative =
            format!("Turns {first}–{last}: user asked about {user}. Agent: {response}.");
        if let Some(decision) = extraction.decisions.first() {
            narrative.push_str(&format!(
                " Decision on {}: {}.",
                decision.topic, decision.choice
            ));
        }
        if let Some(commitment) = extraction.commitments.first() {
            narrative.push_str(&format!(" Next: {}.", commitment.description));
        }
        if let Some(question) = extraction.open_questions.first() {
            narrative.push_str(&format!(" Open: {}.", question.question));
        }
        Ok(narrative)
    }
}

/// A 384-dimensional hashed bag-of-words embedding.
///
/// Deterministic, dependency-free and good enough to exercise retrieval; it is
/// not a semantic model. The `HashingEmbedder` name in the segment says exactly
/// which embedding produced a vector, so a later re-embedding pass can find the
/// old ones.
#[derive(Debug, Clone)]
pub struct HashingEmbedder {
    dimension: usize,
}

impl Default for HashingEmbedder {
    fn default() -> Self {
        Self { dimension: 384 }
    }
}

impl HashingEmbedder {
    /// An embedder with a custom dimension.
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension: dimension.max(1),
        }
    }
}

#[async_trait]
impl Embedder for HashingEmbedder {
    async fn embed(&self, text: &str) -> Result<Vector> {
        let mut vector = vec![0.0f32; self.dimension];
        for word in tokenize(text) {
            let hash = fnv1a(word.as_bytes());
            let index = (hash as usize) % self.dimension;
            let sign = if hash & (1 << 63) == 0 { 1.0 } else { -1.0 };
            vector[index] += sign;
        }
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if norm > 0.0 {
            for value in &mut vector {
                *value /= norm;
            }
        }
        Ok(vector)
    }

    fn model(&self) -> &str {
        "hashing-384"
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Lowercase alphanumeric tokens.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "to", "of", "in", "on", "for", "with", "is", "are",
    "was", "were", "be", "been", "it", "this", "that", "we", "i", "you", "he", "she", "they", "as",
    "at", "by", "from", "will", "can", "should", "would", "not", "do", "did", "does", "if", "then",
    "than", "so", "about", "into", "over", "after", "before", "more", "most", "some", "any", "all",
    "no", "yes", "has", "have", "had", "our", "your", "their",
];

/// The most frequent meaningful words, with weights.
pub fn key_topics(text: &str, limit: usize) -> (Vec<String>, HashMap<String, f32>) {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for word in tokenize(text) {
        if word.len() < 3 || STOPWORDS.contains(&word.as_str()) {
            continue;
        }
        *counts.entry(word).or_default() += 1;
    }
    let total: u32 = counts.values().sum();
    let mut topics: Vec<(String, u32)> = counts.into_iter().collect();
    topics.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    topics.truncate(limit);
    let weights = topics
        .iter()
        .map(|(topic, count)| {
            (
                topic.clone(),
                if total == 0 {
                    0.0
                } else {
                    *count as f32 / total as f32
                },
            )
        })
        .collect();
    (
        topics.into_iter().map(|(topic, _)| topic).collect(),
        weights,
    )
}

/// The pipeline.
pub struct UnifiedCompressor {
    episodic: Arc<EpisodicStore>,
    segments: Arc<SegmentStore>,
    extractor: Arc<dyn MemoryExtractor>,
    narrator: Arc<dyn NarrativeGenerator>,
    embedder: Arc<dyn Embedder>,
    config: SegmentConfig,
    triggers: EpisodicConfig,
    models: CompressionConfig,
}

impl UnifiedCompressor {
    /// Assemble the pipeline.
    pub fn new(
        episodic: Arc<EpisodicStore>,
        segments: Arc<SegmentStore>,
        extractor: Arc<dyn MemoryExtractor>,
        narrator: Arc<dyn NarrativeGenerator>,
        embedder: Arc<dyn Embedder>,
        config: SegmentConfig,
        triggers: EpisodicConfig,
        models: CompressionConfig,
    ) -> Self {
        Self {
            episodic,
            segments,
            extractor,
            narrator,
            embedder,
            config,
            triggers,
            models,
        }
    }

    /// The default offline pipeline.
    pub fn heuristic(
        episodic: Arc<EpisodicStore>,
        segments: Arc<SegmentStore>,
        config: SegmentConfig,
        triggers: EpisodicConfig,
        models: CompressionConfig,
        user_id: UserId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self::new(
            episodic,
            segments,
            Arc::new(HeuristicExtractor::new(Some(user_id), project_id)),
            Arc::new(TemplateNarrator),
            Arc::new(HashingEmbedder::default()),
            config,
            triggers,
            models,
        )
    }

    /// Whether the uncompressed tail has reached a configured trigger.
    pub async fn should_compress(&self, session_id: &SessionId) -> Result<bool> {
        let uncompressed = self.episodic.uncompressed(session_id).await?;
        let tokens: usize = uncompressed
            .iter()
            .map(EpisodicTurn::estimated_tokens)
            .sum();
        Ok(
            uncompressed.len() as u32 >= self.triggers.compression_trigger_turns
                || tokens >= self.triggers.compression_trigger_tokens,
        )
    }

    /// Compress every uncompressed turn of a session.
    pub async fn compress_session(&self, session_id: &SessionId) -> Result<Vec<CompressedSegment>> {
        let turns = self.episodic.uncompressed(session_id).await?;
        if turns.is_empty() {
            return Ok(Vec::new());
        }
        let chunks = chunk_turns(
            &turns,
            self.config.segment_tokens,
            self.config.overlap_turns,
        );
        let mut segments = Vec::with_capacity(chunks.len());
        for (index, chunk) in chunks.iter().enumerate() {
            let extraction = self.extractor.extract(chunk).await?;
            let narrative = self.narrator.narrate(chunk, &extraction).await?;
            let embedding = self.embedder.embed(&narrative).await?;
            let (key_topics, topic_weights) = key_topics(&narrative, 8);
            let token_count: usize = chunk.iter().map(EpisodicTurn::estimated_tokens).sum();
            let narrative_tokens = narrative.len().div_ceil(4).max(1);
            let quality = if self.models.quality_validation_enabled
                && (index as u32 + 1).is_multiple_of(self.config.validate_every_n.max(1))
            {
                validate_quality(chunk, &extraction, &narrative)
            } else {
                SegmentQuality::default()
            };
            let Some(first) = chunk.first() else { continue };
            let Some(last) = chunk.last() else { continue };
            let mut segment = CompressedSegment::new(
                session_id.clone(),
                first.user_id.clone(),
                first.project_id.clone(),
            );
            segment.turn_range = (first.turn_id.clone(), last.turn_id.clone());
            segment.token_count = token_count;
            segment.narrative = narrative;
            segment.decisions = extraction.decisions;
            segment.commitments = extraction.commitments;
            segment.open_questions = extraction.open_questions;
            segment.artifacts = extraction.artifacts;
            segment.goals = extraction.goals;
            segment.inferred_preferences = extraction.inferred_preferences;
            segment.embedding = embedding;
            segment.key_topics = key_topics;
            segment.topic_weights = topic_weights;
            segment.compression_ratio = token_count as f32 / narrative_tokens as f32;
            segment.quality = quality;
            segment.source_turn_ids = chunk.iter().map(|turn| turn.turn_id.clone()).collect();
            segment.compression_model = self.models.narrative_model.clone();
            segment.extraction_model = self.models.extraction_model.clone();
            segment.embedding_model = self.embedder.model().to_owned();
            self.segments.save(&segment).await?;
            segments.push(segment);
        }
        let turn_ids: Vec<TurnId> = turns.iter().map(|turn| turn.turn_id.clone()).collect();
        self.episodic.mark_compressed(session_id, &turn_ids).await?;
        Ok(segments)
    }

    /// The segment store.
    pub fn segments(&self) -> &SegmentStore {
        &self.segments
    }
}

fn chunk_turns(
    turns: &[EpisodicTurn],
    segment_tokens: usize,
    overlap: usize,
) -> Vec<Vec<EpisodicTurn>> {
    let budget = segment_tokens.max(1);
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < turns.len() {
        let mut end = start;
        let mut tokens = 0;
        while end < turns.len() {
            let cost = turns[end].estimated_tokens().max(1);
            if tokens + cost > budget && end > start {
                break;
            }
            tokens += cost;
            end += 1;
        }
        chunks.push(turns[start..end].to_vec());
        if end >= turns.len() {
            break;
        }
        let next = end.saturating_sub(overlap).max(start + 1);
        start = next;
    }
    chunks
}

fn validate_quality(
    turns: &[EpisodicTurn],
    extraction: &Extraction,
    narrative: &str,
) -> SegmentQuality {
    let transcript = turns
        .iter()
        .map(EpisodicTurn::transcript)
        .collect::<String>();
    let lower = transcript.to_lowercase();
    let narrative_lower = narrative.to_lowercase();
    let mentions_decision_phrase = lower.contains("decided");
    let decision_recall = if mentions_decision_phrase {
        if extraction.decisions.is_empty() {
            0.0
        } else {
            1.0
        }
    } else {
        1.0
    };
    let mentions_commitment = lower.contains("will ") || lower.contains("todo");
    let commitment_recall = if mentions_commitment {
        if extraction.commitments.is_empty() {
            0.0
        } else {
            1.0
        }
    } else {
        1.0
    };
    let artifact_recall = if extraction.artifacts.is_empty() {
        1.0
    } else {
        extraction
            .artifacts
            .iter()
            .filter(|artifact| narrative_lower.contains(&artifact.identifier.to_lowercase()))
            .count() as f32
            / extraction.artifacts.len() as f32
    };
    SegmentQuality {
        decision_recall,
        commitment_recall,
        artifact_recall,
        narrative_faithfulness: 1.0,
        hallucination_rate: 0.0,
    }
}

/// Convenience: parse a JSON extraction payload, used by LLM-backed extractors
/// in later phases and by tests.
pub fn extraction_from_json(value: Value) -> Result<Extraction> {
    serde_json::from_value(value).map_err(|error| DataError::Invalid {
        kind: "extraction",
        detail: error.to_string(),
    })
}

impl serde::Serialize for Extraction {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Extraction", 6)?;
        state.serialize_field("decisions", &self.decisions)?;
        state.serialize_field("commitments", &self.commitments)?;
        state.serialize_field("open_questions", &self.open_questions)?;
        state.serialize_field("artifacts", &self.artifacts)?;
        state.serialize_field("goals", &self.goals)?;
        state.serialize_field("inferred_preferences", &self.inferred_preferences)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for Extraction {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Raw {
            #[serde(default)]
            decisions: Vec<DecisionRecord>,
            #[serde(default)]
            commitments: Vec<Commitment>,
            #[serde(default)]
            open_questions: Vec<OpenQuestion>,
            #[serde(default)]
            artifacts: Vec<ArtifactRef>,
            #[serde(default)]
            goals: Vec<GoalProgress>,
            #[serde(default)]
            inferred_preferences: Vec<InferredPreference>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            decisions: raw.decisions,
            commitments: raw.commitments,
            open_questions: raw.open_questions,
            artifacts: raw.artifacts,
            goals: raw.goals,
            inferred_preferences: raw.inferred_preferences,
        })
    }
}

/// A timestamp helper used when a caller builds records outside the pipeline.
pub fn stamp() -> Timestamp {
    Timestamp::now()
}

/// Unused identifier helper kept for extractor implementations.
pub fn segment_id() -> SegmentId {
    SegmentId::new(new_id())
}
