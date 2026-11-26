//! Tier 1: raw episodic turns.
//!
//! A turn is the unit the compressor reads: the user's query, the agent's
//! response, the tools it used, proactive items and the affect/preference
//! snapshot in force. Turns live in the session's agentstream graph so a
//! session and its memory are one query away from each other.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agentstream::AgentstreamStore;
use crate::engine::RdfEngine;
use crate::error::Result;
use crate::rdf::{self, new_id, session_graph, turn_iri};

use super::model::{InputModality, ResponseModality, SessionId, Timestamp, TurnId, UserId};

/// A chunk the agent emitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmittedChunk {
    /// Chunk text.
    pub text: String,
    /// Chunk index within the response.
    pub index: u32,
    /// Whether the user heard/read this chunk before barge-in.
    #[serde(default)]
    pub delivered: bool,
}

/// A tool invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// Call identifier.
    pub call_id: String,
    /// Tool name.
    pub tool: String,
    /// Arguments.
    pub arguments: Value,
    /// When the call started.
    pub started_at: Timestamp,
    /// Duration in milliseconds.
    #[serde(default)]
    pub duration_ms: u64,
}

/// A tool result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultRecord {
    /// Call identifier.
    pub call_id: String,
    /// Whether the tool succeeded.
    pub ok: bool,
    /// Summary of the result.
    pub summary: String,
    /// Raw result, when small enough to keep.
    #[serde(default)]
    pub payload: Option<Value>,
}

/// A proactive item that was surfaced during the turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProactiveItemRecord {
    /// Item kind, e.g. `reminder` or `suggestion`.
    pub kind: String,
    /// What was surfaced.
    pub content: String,
    /// Whether the user accepted it.
    #[serde(default)]
    pub accepted: Option<bool>,
}

/// Voice context captured with the turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VoiceContext {
    /// Provider tier (`tier1` … `tier3`).
    pub tier: String,
    /// Prosody profile name.
    pub prosody_profile: Option<String>,
    /// Last observed affect label.
    pub last_affect: Option<String>,
}

/// The affect snapshot in force at the turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffectState {
    /// Valence, −1.0 to 1.0.
    pub valence: f32,
    /// Arousal, 0.0 to 1.0.
    pub arousal: f32,
    /// Confidence, 0.0 to 1.0.
    pub confidence: f32,
}

impl Default for AffectState {
    fn default() -> Self {
        Self {
            valence: 0.0,
            arousal: 0.5,
            confidence: 0.5,
        }
    }
}

/// The preferences that were applied to this turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PreferenceSnapshot {
    /// Merged preference values in force.
    pub values: std::collections::BTreeMap<String, f32>,
}

/// The agent's response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AgentResponse {
    /// Full response text.
    pub text: String,
    /// Model that produced it.
    pub model: Option<String>,
}

/// One raw turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodicTurn {
    /// Turn identifier.
    pub turn_id: TurnId,
    /// Session identifier.
    pub session_id: SessionId,
    /// Project, when the session has one.
    pub project_id: Option<super::model::ProjectId>,
    /// User identifier.
    pub user_id: UserId,
    /// One-based turn number within the session.
    pub turn_number: u32,
    /// When the turn finished.
    pub timestamp: Timestamp,
    /// The user's query.
    pub user_query: Option<String>,
    /// Input modality.
    pub user_modality: InputModality,
    /// Voice context.
    pub voice_context: Option<VoiceContext>,
    /// The agent's response.
    pub agent_response: AgentResponse,
    /// Response modality.
    pub modality: ResponseModality,
    /// Requested detail level, 1–5.
    pub detail_level: u8,
    /// Emitted chunks.
    pub chunks: Vec<EmittedChunk>,
    /// Tool calls.
    pub tool_calls: Vec<ToolCallRecord>,
    /// Tool results.
    pub tool_results: Vec<ToolResultRecord>,
    /// Proactive items.
    pub proactive_items: Vec<ProactiveItemRecord>,
    /// Affect snapshot.
    pub affect_snapshot: AffectState,
    /// Applied preferences.
    pub preferences_applied: PreferenceSnapshot,
    /// Whether a segment has consumed this turn.
    #[serde(default)]
    pub compressed: bool,
}

impl EpisodicTurn {
    /// A minimal text turn.
    pub fn new(
        session_id: impl Into<SessionId>,
        user_id: impl Into<UserId>,
        turn_number: u32,
        query: impl Into<String>,
        response: impl Into<String>,
    ) -> Self {
        Self {
            turn_id: TurnId::new(new_id()),
            session_id: session_id.into(),
            project_id: None,
            user_id: user_id.into(),
            turn_number,
            timestamp: Timestamp::now(),
            user_query: Some(query.into()),
            user_modality: InputModality::Text,
            voice_context: None,
            agent_response: AgentResponse {
                text: response.into(),
                model: None,
            },
            modality: ResponseModality::Text,
            detail_level: 3,
            chunks: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            proactive_items: Vec::new(),
            affect_snapshot: AffectState::default(),
            preferences_applied: PreferenceSnapshot::default(),
            compressed: false,
        }
    }

    /// A rough token estimate: four characters per token.
    pub fn estimated_tokens(&self) -> usize {
        let query = self.user_query.as_deref().unwrap_or_default().len();
        let response = self.agent_response.text.len();
        let tools: usize = self
            .tool_calls
            .iter()
            .map(|call| call.arguments.to_string().len() + call.tool.len())
            .sum();
        let results: usize = self
            .tool_results
            .iter()
            .map(|result| result.summary.len())
            .sum();
        (query + response + tools + results).div_ceil(4)
    }

    /// The text the compressor reads: query, response, tools and proactive
    /// items, in a stable order.
    pub fn transcript(&self) -> String {
        let mut out = String::new();
        if let Some(query) = &self.user_query {
            out.push_str("User: ");
            out.push_str(query);
            out.push('\n');
        }
        if !self.agent_response.text.is_empty() {
            out.push_str("Agent: ");
            out.push_str(&self.agent_response.text);
            out.push('\n');
        }
        for call in &self.tool_calls {
            out.push_str(&format!("Tool {}: {}\n", call.tool, call.arguments));
        }
        for result in &self.tool_results {
            out.push_str(&format!("Result {}: {}\n", result.call_id, result.summary));
        }
        for item in &self.proactive_items {
            out.push_str(&format!("Proactive {}: {}\n", item.kind, item.content));
        }
        out
    }
}

/// Turn graph class.
pub const TURN_CLASS: &str = "https://metamind.dev/memory#EpisodicTurn";

/// The Tier-1 store.
#[derive(Debug, Clone)]
pub struct EpisodicStore {
    engine: Arc<RdfEngine>,
    agentstream: Arc<AgentstreamStore>,
}

impl EpisodicStore {
    /// Build the store.
    pub fn new(engine: Arc<RdfEngine>, agentstream: Arc<AgentstreamStore>) -> Self {
        Self {
            engine,
            agentstream,
        }
    }

    /// The agentstream store beside this tier.
    pub fn agentstream(&self) -> &AgentstreamStore {
        &self.agentstream
    }

    /// Append a turn to its session graph.
    pub async fn log_turn(&self, turn: EpisodicTurn) -> Result<TurnId> {
        let graph = session_graph(turn.session_id.as_str());
        rdf::upsert(
            &self.engine,
            &graph,
            &turn_iri(turn.turn_id.as_str()),
            TURN_CLASS,
            Some(&format!("#{}", turn.turn_number)),
            &serde_json::to_value(&turn)?,
        )
        .await?;
        Ok(turn.turn_id)
    }

    /// Every turn in a session, ordered by turn number.
    pub async fn turns(&self, session_id: &SessionId) -> Result<Vec<EpisodicTurn>> {
        let graph = session_graph(session_id.as_str());
        let mut turns: Vec<EpisodicTurn> = rdf::list_as(&self.engine, &graph, TURN_CLASS).await?;
        turns.retain(|turn| &turn.session_id == session_id);
        turns.sort_by_key(|turn| (turn.turn_number, turn.timestamp));
        Ok(turns)
    }

    /// Turns that no segment has consumed.
    pub async fn uncompressed(&self, session_id: &SessionId) -> Result<Vec<EpisodicTurn>> {
        Ok(self
            .turns(session_id)
            .await?
            .into_iter()
            .filter(|turn| !turn.compressed)
            .collect())
    }

    /// Mark a group of turns compressed.
    pub async fn mark_compressed(&self, session_id: &SessionId, turn_ids: &[TurnId]) -> Result<()> {
        let mut turns = self.turns(session_id).await?;
        let mut changed = 0;
        for turn in &mut turns {
            if turn_ids.contains(&turn.turn_id) && !turn.compressed {
                turn.compressed = true;
                changed += 1;
            }
        }
        if changed == 0 {
            return Ok(());
        }
        let graph = session_graph(session_id.as_str());
        for turn in turns.iter().filter(|turn| turn_ids.contains(&turn.turn_id)) {
            rdf::upsert(
                &self.engine,
                &graph,
                &turn_iri(turn.turn_id.as_str()),
                TURN_CLASS,
                Some(&format!("#{}", turn.turn_number)),
                &serde_json::to_value(turn)?,
            )
            .await?;
        }
        Ok(())
    }

    /// The next turn number for a session.
    pub async fn next_turn_number(&self, session_id: &SessionId) -> Result<u32> {
        Ok(self
            .turns(session_id)
            .await?
            .iter()
            .map(|turn| turn.turn_number)
            .max()
            .unwrap_or(0)
            + 1)
    }

    /// Rough token count of the uncompressed tail of a session.
    pub async fn uncompressed_tokens(&self, session_id: &SessionId) -> Result<usize> {
        Ok(self
            .uncompressed(session_id)
            .await?
            .iter()
            .map(EpisodicTurn::estimated_tokens)
            .sum())
    }
}
