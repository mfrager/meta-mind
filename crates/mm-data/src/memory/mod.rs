//! The four-tier unified memory system.
//!
//! Tier 1 (`episodic`) stores raw turns in the agentstream graph. Tier 2
//! (`compressed`) stores segments and their artifacts. Tier 3 (`project`) is the
//! aggregated project view. Tier 4 (`user`) is the global user view. The
//! compressor moves 1 → 2, the aggregator 2 → 3, and the cross-project learner
//! 3 → 4.
//!
//! `MemorySystem` is the facade the conversation engine will call: it owns one
//! of each store plus the pipeline, and exposes session lifecycle, logging,
//! compression, aggregation, retrieval and continuity.

pub mod aggregator;
pub mod compressed;
pub mod compressor;
pub mod continuity;
pub mod cross_project;
pub mod episodic;
pub mod model;
pub mod project;
pub mod rdf_mapping;
pub mod sync;
pub mod user;

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::agentstream::{
    ensure_session, AgentstreamEvent, AgentstreamStore, SessionFilter, SessionMetadata,
    SessionRecord,
};
use crate::config::MemoryConfig;
use crate::engine::RdfEngine;
use crate::error::{DataError, Result};
use crate::rdf::new_id;

pub use aggregator::ProjectAggregator;
pub use compressed::{
    ArtifactRef, Commitment, CompressedSegment, DecisionRecord, GoalProgress, OpenQuestion,
    SearchFilters, SearchResult, SegmentStore,
};
pub use compressor::{
    Embedder, HashingEmbedder, HeuristicExtractor, MemoryExtractor, NarrativeGenerator,
    TemplateNarrator, UnifiedCompressor,
};
pub use continuity::{
    PreviousSessionContext, ProjectContext, SessionContext, SessionContinuityManager, UserContext,
};
pub use cross_project::CrossProjectLearner;
pub use episodic::{EpisodicStore, EpisodicTurn};
pub use model::*;
pub use project::{ProjectMemory, ProjectMemoryStore};
pub use sync::{MultiDeviceSync, Turn, UiChannel, UiMessage};
pub use user::{UserMemory, UserMemoryStore};

/// The unified memory facade.
pub struct MemorySystem {
    /// Tier 1.
    pub episodic: Arc<EpisodicStore>,
    /// Tier 2.
    pub segments: Arc<SegmentStore>,
    /// Tier 3.
    pub projects: Arc<ProjectMemoryStore>,
    /// Tier 4.
    pub users: Arc<UserMemoryStore>,
    /// Resume context.
    pub continuity: Arc<SessionContinuityManager>,
    /// Presence and broadcast.
    pub sync: Arc<MultiDeviceSync>,
    /// Compression pipeline.
    pub compressor: Arc<UnifiedCompressor>,
    /// Project aggregation.
    pub aggregator: Arc<ProjectAggregator>,
    /// Cross-project learning.
    pub learner: Arc<CrossProjectLearner>,
    /// Agentstream, shared with Tier 1.
    pub agentstream: Arc<AgentstreamStore>,
    embedder: Arc<dyn Embedder>,
    config: MemoryConfig,
    engine: Arc<RdfEngine>,
}

impl std::fmt::Debug for MemorySystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySystem")
            .field("segments", &self.segments)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl MemorySystem {
    /// Build the system over an open engine.
    pub fn new(engine: Arc<RdfEngine>, base_dir: &Path, config: MemoryConfig) -> Result<Self> {
        let agentstream = Arc::new(AgentstreamStore::new(Arc::clone(&engine), base_dir)?);
        let episodic = Arc::new(EpisodicStore::new(
            Arc::clone(&engine),
            Arc::clone(&agentstream),
        ));
        let segments = Arc::new(SegmentStore::new(Arc::clone(&engine)));
        let projects = Arc::new(ProjectMemoryStore::new(Arc::clone(&engine)));
        let users = Arc::new(UserMemoryStore::new(Arc::clone(&engine)));
        let embedder: Arc<dyn Embedder> =
            Arc::new(HashingEmbedder::new(config.vector_index.dimension.max(1)));
        let compressor = Arc::new(UnifiedCompressor::new(
            Arc::clone(&episodic),
            Arc::clone(&segments),
            Arc::new(HeuristicExtractor::default()),
            Arc::new(TemplateNarrator),
            Arc::clone(&embedder),
            config.segments.clone(),
            config.episodic.clone(),
            config.compression.clone(),
        ));
        let aggregator = Arc::new(ProjectAggregator::new(
            Arc::clone(&projects),
            config.project.clone(),
        ));
        let learner = Arc::new(CrossProjectLearner::new(
            Arc::clone(&projects),
            Arc::clone(&users),
            config.user.clone(),
        ));
        let continuity = Arc::new(SessionContinuityManager::new(
            Arc::clone(&projects),
            Arc::clone(&users),
            Arc::clone(&episodic),
            Arc::clone(&agentstream),
        ));
        Ok(Self {
            episodic,
            segments,
            projects,
            users,
            continuity,
            sync: Arc::new(MultiDeviceSync::new()),
            compressor,
            aggregator,
            learner,
            agentstream,
            embedder,
            config,
            engine,
        })
    }

    /// The engine.
    pub fn engine(&self) -> &Arc<RdfEngine> {
        &self.engine
    }

    /// The configuration in force.
    pub fn config(&self) -> &MemoryConfig {
        &self.config
    }

    /// Start a session and return its resume context.
    pub async fn start_session(
        &self,
        user_id: &UserId,
        project_id: Option<&ProjectId>,
        previous_session: Option<&SessionId>,
    ) -> Result<SessionContext> {
        let session_id = SessionId::new(new_id());
        self.agentstream
            .start_session(
                session_id.as_str(),
                SessionMetadata {
                    user_id: user_id.to_string(),
                    project_id: project_id.map(ToString::to_string),
                    title: None,
                },
            )
            .await?;

        let mut user = self.users.load_or_create(user_id, user_id.as_str()).await?;
        user.note_session();
        self.users.save(&user).await?;

        let mut context = self
            .continuity
            .resume_or_create_session(user_id, project_id, previous_session)
            .await?;
        context.session_id = Some(session_id);
        Ok(context)
    }

    /// Resume context for an existing session.
    pub async fn session_context(&self, session_id: &SessionId) -> Result<SessionContext> {
        let record = self
            .agentstream
            .session(session_id.as_str())
            .await?
            .ok_or_else(|| DataError::NotFound {
                kind: "session",
                id: session_id.to_string(),
            })?;
        let project_id = record
            .project_id
            .as_ref()
            .map(|id| ProjectId::from(id.as_str()));
        let user_id = UserId::from(record.user_id.as_str());
        let mut context = self
            .continuity
            .resume_or_create_session(&user_id, project_id.as_ref(), Some(session_id))
            .await?;
        context.session_id = Some(session_id.clone());
        Ok(context)
    }

    /// End a session, writing its final snapshot.
    pub async fn end_session(&self, session_id: &SessionId) -> Result<()> {
        self.agentstream.end_session(session_id.as_str()).await?;
        self.agentstream.write_projections().await?;
        self.sync.close_session(session_id).await;
        Ok(())
    }

    /// Log one episodic turn.
    pub async fn log_turn(&self, turn: EpisodicTurn) -> Result<TurnId> {
        if turn.session_id.as_str().is_empty() {
            return Err(DataError::Invalid {
                kind: "turn",
                detail: "session_id must not be empty".to_owned(),
            });
        }
        self.episodic.log_turn(turn).await
    }

    /// Log one raw agentstream event.
    pub async fn log_agentstream_event(&self, event: AgentstreamEvent) -> Result<AgentstreamEvent> {
        self.agentstream.log_event(event).await
    }

    /// Ensure a session exists and return its record.
    pub async fn ensure_session(
        &self,
        session_id: &str,
        metadata: SessionMetadata,
    ) -> Result<SessionRecord> {
        ensure_session(&self.agentstream, session_id, metadata).await
    }

    /// Sessions, newest first.
    pub async fn sessions(&self, filter: SessionFilter) -> Result<Vec<SessionRecord>> {
        self.agentstream.sessions(filter).await
    }

    /// Compress every uncompressed turn of a session.
    pub async fn compress_session(&self, session_id: &SessionId) -> Result<Vec<CompressedSegment>> {
        self.compressor.compress_session(session_id).await
    }

    /// Merge every unmerged segment of a project into its project memory.
    pub async fn aggregate_project(&self, project_id: &ProjectId) -> Result<()> {
        let existing = self.projects.load(project_id).await?;
        let merged: Vec<SegmentId> = existing
            .as_ref()
            .map(|memory| memory.segment_ids.clone())
            .unwrap_or_default();
        let name = existing
            .as_ref()
            .map(|memory| memory.name.clone())
            .unwrap_or_else(|| project_id.to_string());
        let segments = self.segments.all(None, Some(project_id)).await?;
        for segment in segments {
            if merged.contains(&segment.segment_id) {
                continue;
            }
            self.aggregator
                .merge_segment_named(project_id, &segment.user_id, &name, &segment)
                .await?;
        }
        Ok(())
    }

    /// Transfer every project of a user into their global memory.
    pub async fn sync_user(&self, user_id: &UserId) -> Result<()> {
        let user = self.users.load_or_create(user_id, user_id.as_str()).await?;
        let project_ids: Vec<ProjectId> = user
            .projects
            .keys()
            .map(|key| ProjectId::from(key.as_str()))
            .collect();
        for project_id in project_ids {
            self.learner
                .learn_from_project(user_id, &project_id)
                .await?;
        }
        Ok(())
    }

    /// Decisions about a topic.
    pub async fn find_decisions(
        &self,
        topic: &str,
        user_id: &UserId,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<DecisionRecord>> {
        let needle = topic.to_lowercase();
        let mut out = Vec::new();
        for segment in self.segments.all(Some(user_id), project_id).await? {
            for decision in segment.decisions {
                if decision.topic.to_lowercase().contains(&needle)
                    || decision.choice.to_lowercase().contains(&needle)
                {
                    out.push(decision);
                }
            }
        }
        if let Some(project_id) = project_id {
            if let Some(memory) = self.projects.load(project_id).await? {
                for decision in memory.aggregated_decisions {
                    if decision.topic.to_lowercase().contains(&needle)
                        || decision.choice.to_lowercase().contains(&needle)
                    {
                        out.push(decision);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Commitments by owner and status.
    pub async fn find_commitments(
        &self,
        owner: &EntityId,
        status: CommitmentStatus,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<Commitment>> {
        let mut out = Vec::new();
        if let Some(project_id) = project_id {
            if let Some(memory) = self.projects.load(project_id).await? {
                out.extend(memory.active_commitments.into_iter().filter(|commitment| {
                    &commitment.owner == owner && commitment.status == status
                }));
            }
        }
        Ok(out)
    }

    /// Artifacts by kind and free-text query.
    pub async fn find_artifacts(
        &self,
        kind: ArtifactKind,
        query: &str,
        user_id: &UserId,
    ) -> Result<Vec<ArtifactRef>> {
        let needle = query.to_lowercase();
        let mut out = Vec::new();
        for segment in self.segments.all(Some(user_id), None).await? {
            for artifact in segment.artifacts {
                if artifact.kind == kind
                    && (needle.is_empty()
                        || artifact.identifier.to_lowercase().contains(&needle)
                        || artifact.description.to_lowercase().contains(&needle))
                {
                    out.push(artifact);
                }
            }
        }
        Ok(out)
    }

    /// Goals by status.
    pub async fn find_goals(
        &self,
        user_id: &UserId,
        status: GoalStatus,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<GoalProgress>> {
        let mut out = Vec::new();
        for segment in self.segments.all(Some(user_id), project_id).await? {
            out.extend(
                segment
                    .goals
                    .into_iter()
                    .filter(|goal| goal.status == status),
            );
        }
        Ok(out)
    }

    /// Semantic search over segment embeddings.
    pub async fn semantic_search(
        &self,
        query: &str,
        filters: SearchFilters,
        k: usize,
    ) -> Result<Vec<SearchResult>> {
        let embedding = self.embedder.embed(query).await?;
        self.segments.search(&embedding, &filters, k).await
    }

    /// Project memory.
    pub async fn project_context(&self, project_id: &ProjectId) -> Result<Option<ProjectMemory>> {
        self.projects.load(project_id).await
    }

    /// User memory.
    pub async fn user_context(&self, user_id: &UserId) -> Result<Option<UserMemory>> {
        self.users.load(user_id).await
    }

    /// Subscribe a UI channel to a session.
    pub async fn register_channel(
        &self,
        session_id: &SessionId,
        channel: UiChannel,
    ) -> Result<UnboundedReceiver<UiMessage>> {
        if self
            .agentstream
            .session(session_id.as_str())
            .await?
            .is_none()
        {
            return Err(DataError::NotFound {
                kind: "session",
                id: session_id.to_string(),
            });
        }
        Ok(self.sync.connect(session_id, channel).await)
    }

    /// Broadcast a turn to every other channel.
    pub async fn broadcast_turn(
        &self,
        session_id: &SessionId,
        turn: Turn,
        origin: UiChannel,
    ) -> Result<usize> {
        self.sync.broadcast_turn(session_id, turn, origin).await
    }

    /// Move a session between modalities.
    pub async fn handoff_modality(
        &self,
        session_id: &SessionId,
        from: UiChannel,
        to: UiChannel,
    ) -> Result<usize> {
        self.sync.handoff(session_id, from, to, None).await
    }

    /// Export everything this layer stores for a user as one JSON document.
    pub async fn export_user_data(&self, user_id: &UserId) -> Result<Value> {
        #[derive(Serialize)]
        struct Export {
            user: Option<UserMemory>,
            projects: Vec<ProjectMemory>,
            segments: Vec<CompressedSegment>,
            sessions: Vec<SessionRecord>,
        }
        let user = self.users.load(user_id).await?;
        let mut projects = Vec::new();
        if let Some(user) = &user {
            for key in user.projects.keys() {
                if let Some(memory) = self.projects.load(&ProjectId::from(key.as_str())).await? {
                    projects.push(memory);
                }
            }
        }
        let segments = self.segments.all(Some(user_id), None).await?;
        let sessions = self
            .agentstream
            .sessions(SessionFilter {
                user_id: Some(user_id.to_string()),
                ..SessionFilter::default()
            })
            .await?;
        serde_json::to_value(Export {
            user,
            projects,
            segments,
            sessions,
        })
        .map_err(Into::into)
    }
}
