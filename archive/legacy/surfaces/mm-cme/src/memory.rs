//! Phase 5 memory integration: the region's work, delegated to `mm-data`.
//!
//! [`MemoryBridge`] is the conversation engine's handle on the unified four-tier
//! memory system. It logs completed turns into the episodic tier, triggers
//! compression through `mm_data::MemorySystem`, aggregates project memory,
//! optionally transfers learning into user memory, and restores resume context
//! (greeting, project narrative, previous topics and learned preferences).
//!
//! It also exposes the multi-device surface: channel presence, turn broadcast and
//! voice ↔ text handoff. The region state machine still records where the engine
//! is in the pipeline; the records themselves live in `mm-data`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use mm_data::memory::episodic::{AffectState as DataAffectState, ProactiveItemRecord};
use mm_data::memory::{InputModality, ResponseModality, SessionContext as DataSessionContext};
use mm_data::{
    CompressedSegment, EpisodicTurn, MemorySystem, ProjectId, SearchFilters, SearchResult,
    SessionId, Turn as SyncTurn, UiChannel, UiMessage, UserId,
};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::error::{CmeError, Result};
use crate::preferences::PlannerHooks;
use crate::session::Session;
use crate::turn::TurnOutcome;

/// When the bridge compresses and what it does afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBridgeConfig {
    /// Uncompressed turns that trigger compression.
    pub compression_trigger_turns: u32,
    /// Merge new segments into project memory after compression.
    pub auto_aggregate: bool,
    /// Transfer project learning into user memory after aggregation.
    pub auto_sync_user: bool,
}

impl Default for MemoryBridgeConfig {
    fn default() -> Self {
        Self {
            compression_trigger_turns: 10,
            auto_aggregate: true,
            auto_sync_user: false,
        }
    }
}

impl MemoryBridgeConfig {
    /// Take the trigger threshold from the data layer's own configuration.
    pub fn from_memory_config(config: &mm_data::MemoryConfig) -> Self {
        Self {
            compression_trigger_turns: config.episodic.compression_trigger_turns.max(1),
            ..Self::default()
        }
    }
}

/// What a session can resume from.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionResume {
    /// The memory session identifier.
    pub session_id: SessionId,
    /// The project, when the session has one.
    pub project_id: Option<ProjectId>,
    /// Greeting generated from continuity.
    pub greeting: String,
    /// Merged user preferences.
    pub preferences: BTreeMap<String, f32>,
    /// Project narrative, when a project exists.
    pub project_summary: Option<String>,
    /// Suggested next step from project memory.
    pub suggested_continuation: Option<String>,
    /// Topics from the previous session.
    pub previous_topics: Vec<String>,
    /// Previous session identifier.
    pub previous_session: Option<SessionId>,
}

impl SessionResume {
    fn from_data(context: DataSessionContext, previous: Option<&SessionId>) -> Result<Self> {
        let session_id = context
            .session_id
            .clone()
            .ok_or_else(|| CmeError::Driver("memory session has no identifier".to_owned()))?;
        let previous_session = context
            .previous_session
            .as_ref()
            .map(|previous| SessionId::new(previous.session_id.clone()))
            .or_else(|| previous.cloned());
        Ok(Self {
            session_id,
            project_id: context.project_id.clone(),
            greeting: context.greeting,
            preferences: context
                .user_context
                .as_ref()
                .map(|user| {
                    user.preferences
                        .iter()
                        .map(|(k, v)| (k.clone(), *v))
                        .collect()
                })
                .unwrap_or_default(),
            project_summary: context
                .project_context
                .as_ref()
                .map(|project| project.project_summary.clone()),
            suggested_continuation: context
                .project_context
                .as_ref()
                .and_then(|project| project.suggested_continuation.clone()),
            previous_topics: context
                .previous_session
                .as_ref()
                .map(|previous| previous.last_topics.clone())
                .unwrap_or_default(),
            previous_session,
        })
    }

    /// Apply the resume context to a session.
    pub fn apply(&self, session: &mut Session) {
        session.id = self.session_id.clone();
        session.project_id = self.project_id.clone();
        session.context.memory.project_id = self.project_id.clone();
        session.context.memory.previous_session_id = self.previous_session.clone();
        session.context.memory.greeting = Some(self.greeting.clone());
        session.context.memory.project_summary = self.project_summary.clone();
        session.context.memory.previous_topics = self.previous_topics.clone();
        for (dimension, value) in &self.preferences {
            session
                .context
                .preferences
                .values
                .entry(dimension.clone())
                .or_insert(*value);
        }
        PlannerHooks::apply(&mut session.context.preferences);
    }
}

/// The result of one memory cycle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryCycle {
    /// Segments produced by compression.
    pub segments: Vec<CompressedSegment>,
    /// Whether project memory absorbed them.
    pub aggregated: bool,
    /// Whether user memory absorbed the project learning.
    pub user_synced: bool,
}

/// The conversation engine's memory handle.
#[derive(Debug)]
pub struct MemoryBridge {
    system: Arc<MemorySystem>,
    config: MemoryBridgeConfig,
    uncompressed: AtomicU32,
}

impl MemoryBridge {
    /// Build a bridge over an existing memory system.
    pub fn new(system: Arc<MemorySystem>, config: MemoryBridgeConfig) -> Self {
        Self {
            system,
            config,
            uncompressed: AtomicU32::new(0),
        }
    }

    /// The underlying memory system.
    pub fn system(&self) -> &Arc<MemorySystem> {
        &self.system
    }

    /// The bridge configuration.
    pub fn config(&self) -> MemoryBridgeConfig {
        self.config
    }

    /// Uncompressed turns recorded since the last cycle.
    pub fn uncompressed_turns(&self) -> u32 {
        self.uncompressed.load(Ordering::SeqCst)
    }

    /// Start a memory session for `session`, applying resume context.
    ///
    /// The memory layer assigns the session identifier, so the conversation
    /// session adopts it; subsequent turns and sync calls then share one id.
    pub async fn start_session(
        &self,
        session: &mut Session,
        previous: Option<&SessionId>,
    ) -> Result<SessionResume> {
        // A caller may already have resumed this session through the bridge (for
        // example before handing it to the driver); reuse it instead of creating
        // a second memory session under a new id.
        let existing_id = session.id.clone();
        if self.system.session_context(&existing_id).await.is_ok() {
            return self.resume_session(session, &existing_id).await;
        }
        let context = self
            .system
            .start_session(&session.user_id, session.project_id.as_ref(), previous)
            .await?;
        let resume = SessionResume::from_data(context, previous)?;
        resume.apply(session);
        self.uncompressed.store(0, Ordering::SeqCst);
        Ok(resume)
    }

    /// Restore context for an existing memory session.
    pub async fn resume_session(
        &self,
        session: &mut Session,
        session_id: &SessionId,
    ) -> Result<SessionResume> {
        let context = self.system.session_context(session_id).await?;
        let resume = SessionResume::from_data(context, None)?;
        resume.apply(session);
        Ok(resume)
    }

    /// Log one completed turn into episodic memory.
    pub async fn log_turn(
        &self,
        session: &Session,
        query: &str,
        detail_level: u8,
        outcome: &TurnOutcome,
    ) -> Result<EpisodicTurn> {
        let mut turn = EpisodicTurn::new(
            session.id.clone(),
            session.user_id.clone(),
            session.turns_completed.max(1),
            query,
            outcome.text.clone(),
        );
        turn.turn_id = outcome.turn_id.clone();
        turn.project_id = session.project_id.clone();
        turn.detail_level = detail_level;
        turn.user_modality = if session.voice_tier().is_some() {
            InputModality::Voice
        } else {
            InputModality::Text
        };
        turn.modality = if session.voice_tier().is_some() {
            ResponseModality::Voice
        } else {
            ResponseModality::Text
        };
        turn.agent_response.model = outcome.model.clone();
        turn.preferences_applied.values = session.context.preferences.values.clone();
        turn.affect_snapshot = DataAffectState {
            valence: session.context.affect.valence,
            arousal: session.context.affect.arousal,
            confidence: session.context.affect.confidence,
        };
        turn.proactive_items = session
            .proactive_items
            .iter()
            .map(|item| ProactiveItemRecord {
                kind: item.plugin.clone(),
                content: item.body.clone(),
                accepted: None,
            })
            .collect();
        self.system.log_turn(turn.clone()).await?;
        self.uncompressed.fetch_add(1, Ordering::SeqCst);
        Ok(turn)
    }

    /// Log a successful turn and run the pipeline when the trigger is reached.
    pub async fn after_turn(
        &self,
        session: &mut Session,
        query: &str,
        detail_level: u8,
        outcome: &TurnOutcome,
    ) -> Result<MemoryCycle> {
        if !outcome.is_success() {
            return Ok(MemoryCycle::default());
        }
        self.log_turn(session, query, detail_level, outcome).await?;
        if self.uncompressed.load(Ordering::SeqCst) < self.config.compression_trigger_turns {
            return Ok(MemoryCycle::default());
        }
        self.compress(session).await
    }

    /// Run compression, aggregation and optional user sync for a session.
    pub async fn compress(&self, session: &mut Session) -> Result<MemoryCycle> {
        session.dispatch_regions("compression_triggered", None)?;
        let segments = self.system.compress_session(&session.id).await?;
        let ids: Vec<String> = segments
            .iter()
            .map(|segment| segment.segment_id.to_string())
            .collect();
        session.dispatch_regions("compression_done", Some(json!({ "segments": ids })))?;

        let mut aggregated = false;
        if self.config.auto_aggregate && !segments.is_empty() {
            if let Some(project_id) = session.project_id.clone() {
                session.dispatch_regions("project_aggregation_triggered", None)?;
                self.system.aggregate_project(&project_id).await?;
                session.dispatch_regions("project_aggregation_done", None)?;
                aggregated = true;
            }
        }

        let mut user_synced = false;
        if self.config.auto_sync_user {
            session.dispatch_regions("user_sync_triggered", None)?;
            self.system.sync_user(&session.user_id).await?;
            session.dispatch_regions("user_sync_done", None)?;
            user_synced = true;
        }

        self.uncompressed.store(0, Ordering::SeqCst);
        Ok(MemoryCycle {
            segments,
            aggregated,
            user_synced,
        })
    }

    /// Transfer a user's project learning into user memory.
    pub async fn sync_user(&self, user_id: &UserId) -> Result<()> {
        self.system.sync_user(user_id).await?;
        Ok(())
    }

    /// End the memory session and write its snapshot.
    pub async fn end_session(&self, session: &Session) -> Result<()> {
        self.system.end_session(&session.id).await?;
        Ok(())
    }

    /// Semantic search over compressed segments.
    pub async fn search(
        &self,
        query: &str,
        filters: SearchFilters,
        k: usize,
    ) -> Result<Vec<SearchResult>> {
        self.system
            .semantic_search(query, filters, k)
            .await
            .map_err(Into::into)
    }

    /// Subscribe a UI channel to a session.
    pub async fn register_channel(
        &self,
        session_id: &SessionId,
        channel: UiChannel,
    ) -> Result<UnboundedReceiver<UiMessage>> {
        self.system
            .register_channel(session_id, channel)
            .await
            .map_err(Into::into)
    }

    /// Channels currently connected to a session.
    pub async fn presence(&self, session_id: &SessionId) -> Result<Vec<UiChannel>> {
        Ok(self.system.sync.presence(session_id).await)
    }

    /// Broadcast a turn to every channel except the origin.
    pub async fn broadcast_turn(
        &self,
        session_id: &SessionId,
        turn: SyncTurn,
        origin: UiChannel,
    ) -> Result<usize> {
        self.system
            .broadcast_turn(session_id, turn, origin)
            .await
            .map_err(Into::into)
    }

    /// Hand a session from one modality to another.
    pub async fn handoff_modality(
        &self,
        session_id: &SessionId,
        from: UiChannel,
        to: UiChannel,
    ) -> Result<usize> {
        self.system
            .handoff_modality(session_id, from, to)
            .await
            .map_err(Into::into)
    }
}
