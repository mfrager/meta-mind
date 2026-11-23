//! State service for the conversation tools and inspector.
//!
//! Phase 1 owns the read/write path, not the HTTP surface: it binds the
//! conversation and session stores, creates the lazy session directory, starts
//! the main trace, and exposes typed operations that the Phase 2 tools call.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mm_cme::{FrameEngine, FrameError};
use mm_data::{
    ConversationError, ConversationEvent, ConversationEventKind, ConversationSessionFilter,
    ConversationState, ConversationStore, DeleteMode, FrameStack, FrameUtterance, Presence,
    RdfSessionStore, ResumeMetadata, SessionBundle, SessionDirectoryError, SessionDirectoryService,
    SessionError, SessionLifecycle, SessionMetadata, SessionPage, SessionState, SessionStore,
    SessionSummary, SessionTrace, StateOrigin, StatusValue, TraceKind, TracePage, WorkCatalog,
    WorkItem, WorkStatus,
};
use mm_prompt::PromptStore;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::RwLock;

use crate::session::{SessionLease, SessionRegistry};
use crate::status::{
    ActionsStatusView, AutomationStatusView, BudgetStatusView, DataStatusView, FrameStatusView,
    FramesStatusView, InterjectionsStatusView, PlansStatusView, SessionStatusView, StateStatusView,
    StatusView,
};

/// The Phase 1 state facade.
#[derive(Debug, Clone)]
pub struct StateService {
    conversation: ConversationStore,
    sessions: RdfSessionStore,
    frames: FrameEngine,
    directory: SessionDirectoryService,
    registry: SessionRegistry,
    agentstream: Arc<mm_data::AgentstreamStore>,
    mutations: Arc<RwLock<()>>,
}

impl StateService {
    /// Build over the initialized data layer and the reviewed prompt tree.
    pub fn new(
        engine: Arc<mm_data::RdfEngine>,
        agentstream: Arc<mm_data::AgentstreamStore>,
        prompts: Arc<PromptStore>,
        base_dir: impl Into<PathBuf>,
    ) -> Result<Self, StateServiceError> {
        let base_dir = base_dir.into();
        Ok(Self {
            conversation: ConversationStore::new(Arc::clone(&engine), Arc::clone(&agentstream)),
            sessions: RdfSessionStore::new(engine, Arc::clone(&agentstream), base_dir.clone()),
            frames: FrameEngine::new(prompts),
            directory: SessionDirectoryService::new(base_dir),
            registry: SessionRegistry::default_config(),
            agentstream,
            mutations: Arc::new(RwLock::new(())),
        })
    }

    /// The live registry (a cache, never the source of truth).
    pub fn registry(&self) -> &SessionRegistry {
        &self.registry
    }

    /// Open a fresh session with an empty conversation state and the `intro`
    /// frame. A previous session may be named for continuity, but its state is
    /// deliberately not copied into the current state.
    pub async fn open_session(
        &self,
        user_id: &str,
        previous_session: Option<&str>,
        now_ms: i64,
    ) -> Result<SessionBundle, StateServiceError> {
        self.open_session_with_id(mm_data::rdf::new_id(), user_id, previous_session, now_ms)
            .await
    }

    /// Open a fresh session with a caller-chosen id. This is used by the CME
    /// surface, whose session id is minted by the chat catalog before the first
    /// tool call. Existing sessions are rejected by the store's revision guard.
    pub async fn open_session_with_id(
        &self,
        session_id: impl Into<String>,
        user_id: &str,
        previous_session: Option<&str>,
        now_ms: i64,
    ) -> Result<SessionBundle, StateServiceError> {
        let _guard = self.mutations.write().await;
        let previous = if let Some(previous) = previous_session {
            Some(self.load(previous).await?)
        } else {
            None
        };
        let session_id = session_id.into();
        let conversation = ConversationState::new(&session_id, now_ms);
        let mut session = SessionState::new(&session_id, user_id, now_ms);
        if let Some(previous) = &previous {
            session.resume = ResumeMetadata {
                epoch: 1,
                resume_of: Some(previous.session.session_id.clone()),
                resume_count: 1,
                resumable_until: None,
            };
        }
        let frame = self.frames.frame("frame.intro", "root", now_ms)?;
        let frames = FrameStack::new(frame);

        self.agentstream
            .start_session(
                &session_id,
                SessionMetadata {
                    user_id: user_id.to_owned(),
                    project_id: None,
                    title: None,
                },
            )
            .await?;
        self.conversation.save(&conversation, 0).await?;
        self.sessions.save_session(&session, 0).await?;

        let main = SessionTrace {
            trace_id: "main".into(),
            session_id: session_id.clone(),
            kind: TraceKind::Main,
            agentstream_id: session_id.clone(),
            first_seq: 0,
            last_seq: 0,
            event_count: 0,
            started_at: now_ms,
            ended_at: None,
            size_bytes: 0,
        };
        self.sessions.register_trace(&session_id, &main).await?;
        self.sessions.save_frames(&session_id, &frames, 0).await?;
        let session = self
            .sessions
            .load(&session_id)
            .await?
            .ok_or_else(|| StateServiceError::Missing(session_id.clone()))?;
        self.conversation
            .append_event(ConversationEvent::lifecycle(
                ConversationEventKind::SessionOpened,
                &session_id,
                session.session.revision,
                StateOrigin::System,
            ))
            .await?;
        self.registry
            .insert(SessionLease::new(&session_id, user_id, now_ms))
            .await;
        if let Some(mut previous) = previous {
            previous.session.superseded_by = Some(session_id.clone());
            previous.session.revision = previous.session.revision.saturating_add(1);
            self.sessions
                .save_session(&previous.session, previous.session.revision - 1)
                .await?;
        }
        Ok(session)
    }

    /// Return an existing session or bootstrap it under its assigned id.
    pub async fn ensure_session(
        &self,
        session_id: &str,
        user_id: &str,
        now_ms: i64,
    ) -> Result<SessionBundle, StateServiceError> {
        match self.load(session_id).await {
            Ok(bundle) => Ok(bundle),
            Err(StateServiceError::Missing(_)) => {
                self.open_session_with_id(session_id, user_id, None, now_ms)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    /// Resume an existing session in place, restoring its stack/work and pushing
    /// one `resume` frame.
    pub async fn resume_session(
        &self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<SessionBundle, StateServiceError> {
        let _guard = self.mutations.write().await;
        let mut bundle = self.load(session_id).await?;
        if bundle.session.deleted_at.is_some() {
            return Err(StateServiceError::Deleted(session_id.into()));
        }
        bundle.session.lifecycle = SessionLifecycle::Active;
        bundle.session.presence = Presence::Present;
        bundle.session.ended_at = None;
        bundle.session.end_reason = None;
        bundle.session.expires_at = None;
        bundle.session.last_activity_at = now_ms;
        bundle.session.resume.epoch = bundle.session.resume.epoch.saturating_add(1);
        bundle.session.resume.resume_count = bundle.session.resume.resume_count.saturating_add(1);
        bundle.session.revision = bundle.session.revision.saturating_add(1);

        let resume = self
            .frames
            .frame("frame.resume", mm_data::rdf::new_id(), now_ms)?;
        bundle.frames.push(resume)?;
        self.sessions
            .save_frames(session_id, &bundle.frames, bundle.frames.revision - 1)
            .await?;
        self.sessions
            .save_session(&bundle.session, bundle.session.revision - 1)
            .await?;
        self.conversation
            .append_event(ConversationEvent::lifecycle(
                ConversationEventKind::SessionResumed,
                session_id,
                bundle.session.revision,
                StateOrigin::System,
            ))
            .await?;
        self.registry
            .insert(SessionLease::new(
                session_id,
                &bundle.session.user_id,
                now_ms,
            ))
            .await;
        self.sessions
            .load(session_id)
            .await?
            .ok_or_else(|| StateServiceError::Missing(session_id.into()))
    }

    /// Load a bundle.
    pub async fn load(&self, session_id: &str) -> Result<SessionBundle, StateServiceError> {
        self.sessions
            .load(session_id)
            .await?
            .ok_or_else(|| StateServiceError::Missing(session_id.into()))
    }

    /// Get the materialized conversation state.
    pub async fn conversation(
        &self,
        session_id: &str,
    ) -> Result<ConversationState, StateServiceError> {
        self.conversation
            .load(session_id)
            .await?
            .ok_or_else(|| StateServiceError::Missing(session_id.into()))
    }

    /// The step of the current top frame, for validating `frame_step` writes.
    pub async fn current_frame_step(&self, session_id: &str) -> Result<String, StateServiceError> {
        let bundle = self.load(session_id).await?;
        let frame = bundle
            .frames
            .top()
            .ok_or_else(|| SessionError::FrameInvalid("frame stack is empty".into()))?;
        self.frames
            .template(&frame.template_id)
            .map(|template| template.step.clone())
            .ok_or_else(|| FrameError::UnknownTemplate(frame.template_id.clone()).into())
    }

    /// Rebuild a conversation state from its event log.
    pub async fn replay(&self, session_id: &str) -> Result<ConversationState, StateServiceError> {
        Ok(self.conversation.replay(session_id).await?)
    }

    /// Set the topic with an optimistic revision check.
    pub async fn set_topic(
        &self,
        session_id: &str,
        topic: Option<String>,
        expected_revision: u64,
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<ConversationState, StateServiceError> {
        let _guard = self.mutations.write().await;
        self.ensure_writable(session_id).await?;
        let mut state = self.conversation(session_id).await?;
        state.set_topic(topic, origin, now_ms)?;
        self.conversation.save(&state, expected_revision).await?;
        Ok(state)
    }

    /// Set the depth with an optimistic revision check.
    pub async fn set_depth(
        &self,
        session_id: &str,
        depth: u8,
        expected_revision: u64,
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<ConversationState, StateServiceError> {
        let _guard = self.mutations.write().await;
        self.ensure_writable(session_id).await?;
        let mut state = self.conversation(session_id).await?;
        state.set_depth(depth, origin, now_ms)?;
        self.conversation.save(&state, expected_revision).await?;
        Ok(state)
    }

    /// Set one status variable.
    pub async fn set_status(
        &self,
        session_id: &str,
        key: &str,
        value: StatusValue,
        expected_revision: u64,
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<ConversationState, StateServiceError> {
        let _guard = self.mutations.write().await;
        self.ensure_writable(session_id).await?;
        let mut state = self.conversation(session_id).await?;
        state.set_status(key, value, origin, now_ms)?;
        self.conversation.save(&state, expected_revision).await?;
        Ok(state)
    }

    /// Unset one status variable.
    pub async fn unset_status(
        &self,
        session_id: &str,
        key: &str,
        expected_revision: u64,
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<ConversationState, StateServiceError> {
        let _guard = self.mutations.write().await;
        self.ensure_writable(session_id).await?;
        let mut state = self.conversation(session_id).await?;
        state.unset_status(key, origin, now_ms)?;
        self.conversation.save(&state, expected_revision).await?;
        Ok(state)
    }

    /// Apply several status writes in one state revision.
    pub async fn update_status(
        &self,
        session_id: &str,
        values: BTreeMap<String, StatusValue>,
        unset: &[String],
        expected_revision: u64,
        origin: StateOrigin,
        now_ms: i64,
    ) -> Result<ConversationState, StateServiceError> {
        let _guard = self.mutations.write().await;
        self.ensure_writable(session_id).await?;
        let mut state = self.conversation(session_id).await?;
        state.set_status_values(values, origin, now_ms)?;
        state.unset_status_values(unset, origin, now_ms)?;
        self.conversation.save(&state, expected_revision).await?;
        Ok(state)
    }

    /// Push a frame by template id and seed the four reserved frame keys.
    pub async fn push_frame(
        &self,
        session_id: &str,
        template_id: &str,
        expected_frame_revision: u64,
        now_ms: i64,
    ) -> Result<FrameStack, StateServiceError> {
        let _guard = self.mutations.write().await;
        let mut bundle = self.load(session_id).await?;
        self.ensure_open(&bundle.session)?;
        let frame = self
            .frames
            .frame(template_id, mm_data::rdf::new_id(), now_ms)?;
        let step = self.frame_step(template_id)?;
        let expected = frame_expectation(&bundle, expected_frame_revision);
        bundle.frames.push(frame)?;
        let saved = self
            .sessions
            .save_frames(session_id, &bundle.frames, expected)
            .await?;
        self.write_frame_status(session_id, &step, 0, now_ms)
            .await?;
        Ok(saved)
    }

    /// Replace the top frame, preserving and incrementing `frame_attempt`.
    pub async fn replace_frame(
        &self,
        session_id: &str,
        template_id: &str,
        expected_frame_revision: u64,
        now_ms: i64,
    ) -> Result<FrameStack, StateServiceError> {
        let _guard = self.mutations.write().await;
        let mut bundle = self.load(session_id).await?;
        self.ensure_open(&bundle.session)?;
        let frame = self
            .frames
            .frame(template_id, mm_data::rdf::new_id(), now_ms)?;
        let step = self.frame_step(template_id)?;
        let attempt = frame_attempt(&bundle.conversation).saturating_add(1).min(5);
        let expected = frame_expectation(&bundle, expected_frame_revision);
        bundle.frames.replace_top(frame)?;
        let saved = self
            .sessions
            .save_frames(session_id, &bundle.frames, expected)
            .await?;
        self.write_frame_status(session_id, &step, attempt, now_ms)
            .await?;
        Ok(saved)
    }

    /// Pop the top frame. The root cannot be popped.
    pub async fn pop_frame(
        &self,
        session_id: &str,
        expected_frame_revision: u64,
        now_ms: i64,
    ) -> Result<FrameStack, StateServiceError> {
        let _guard = self.mutations.write().await;
        let mut bundle = self.load(session_id).await?;
        self.ensure_open(&bundle.session)?;
        let expected = frame_expectation(&bundle, expected_frame_revision);
        let _ = self.frames.pop(&mut bundle.frames, now_ms)?;
        let saved = self
            .sessions
            .save_frames(session_id, &bundle.frames, expected)
            .await?;
        let step = saved
            .top()
            .and_then(|frame| self.frames.template(&frame.template_id))
            .map_or("", |template| template.step.as_str());
        self.write_frame_status(session_id, step, 0, now_ms).await?;
        self.cancel_utterance_inner(session_id).await?;
        Ok(saved)
    }

    /// Schedule the next in-character utterance for the top frame.
    pub async fn schedule_utterance(
        &self,
        session_id: &str,
        delay_ms: Option<u64>,
        reason: &str,
        expected_revision: u64,
        now_ms: i64,
    ) -> Result<(SessionState, FrameUtterance), StateServiceError> {
        let _guard = self.mutations.write().await;
        let mut bundle = self.load(session_id).await?;
        self.ensure_open(&bundle.session)?;
        if expected_revision != bundle.conversation.revision
            && expected_revision != bundle.frames.revision
        {
            return Err(StateServiceError::Session(SessionError::Conflict {
                expected: expected_revision,
                current: bundle.conversation.revision,
            }));
        }
        let frame = bundle
            .frames
            .top()
            .ok_or_else(|| SessionError::FrameInvalid("frame stack is empty".into()))?;
        let template = self
            .frames
            .template(&frame.template_id)
            .ok_or_else(|| SessionError::FrameInvalid("unknown frame template".into()))?;
        let delay = delay_ms.unwrap_or(template.utterance.delay_ms);
        let attempt = frame_attempt(&bundle.conversation).min(5);
        let utterance = FrameUtterance {
            id: mm_data::rdf::new_id(),
            frame_id: frame.id.clone(),
            due_at: now_ms.saturating_add(delay.min(i64::MAX as u64) as i64),
            reason: reason.to_owned(),
            attempt,
        };
        let mut conversation = bundle.conversation.clone();
        let step = template.step.clone();
        conversation.set_status_values(
            [
                ("frame_step".to_owned(), StatusValue::Text(step)),
                (
                    "frame_attempt".to_owned(),
                    StatusValue::Integer(i64::from(attempt)),
                ),
                ("awaiting_user".to_owned(), StatusValue::Bool(true)),
                (
                    "next_utterance_at".to_owned(),
                    StatusValue::Integer(utterance.due_at),
                ),
            ],
            StateOrigin::Cm,
            now_ms,
        )?;
        self.conversation
            .save(&conversation, bundle.conversation.revision)
            .await?;
        bundle.session.pending_utterance = Some(utterance.clone());
        bundle.session.revision = bundle.session.revision.saturating_add(1);
        self.sessions
            .save_session(&bundle.session, bundle.session.revision - 1)
            .await?;
        let session = self
            .sessions
            .load(session_id)
            .await?
            .ok_or_else(|| StateServiceError::Missing(session_id.into()))?;
        Ok((session.session, utterance))
    }

    /// Cancel the pending utterance without changing the frame stack.
    pub async fn cancel_utterance(
        &self,
        session_id: &str,
        expected_revision: u64,
        now_ms: i64,
    ) -> Result<SessionState, StateServiceError> {
        let _guard = self.mutations.write().await;
        let bundle = self.load(session_id).await?;
        self.ensure_open(&bundle.session)?;
        if expected_revision != bundle.conversation.revision
            && expected_revision != bundle.frames.revision
        {
            return Err(StateServiceError::Session(SessionError::Conflict {
                expected: expected_revision,
                current: bundle.conversation.revision,
            }));
        }
        let _ = now_ms;
        self.cancel_utterance_inner(session_id).await?;
        self.sessions
            .load(session_id)
            .await?
            .map(|bundle| bundle.session)
            .ok_or_else(|| StateServiceError::Missing(session_id.into()))
    }

    /// Create the session directory on first use and mirror it in RDF.
    pub async fn ensure_session_directory(
        &self,
        session_id: &str,
        _now_ms: i64,
    ) -> Result<PathBuf, StateServiceError> {
        let _guard = self.mutations.write().await;
        let mut bundle = self.load(session_id).await?;
        let path = self.directory.ensure(session_id)?;
        if bundle.session.session_dir.is_none() {
            bundle.session.session_dir = Some(format!("data/sessions/{session_id}"));
            bundle.session.revision = bundle.session.revision.saturating_add(1);
            self.sessions
                .save_session(&bundle.session, bundle.session.revision - 1)
                .await?;
            self.conversation
                .append_event(ConversationEvent::lifecycle(
                    ConversationEventKind::SessionDirectoryCreated,
                    session_id,
                    bundle.session.revision,
                    StateOrigin::System,
                ))
                .await?;
        }
        Ok(path)
    }

    /// Write one session-scoped file, creating the directory lazily.
    pub async fn write_session_file(
        &self,
        session_id: &str,
        relative: impl AsRef<Path>,
        bytes: &[u8],
        now_ms: i64,
    ) -> Result<PathBuf, StateServiceError> {
        self.ensure_session_directory(session_id, now_ms).await?;
        Ok(self.directory.write_file(session_id, relative, bytes)?)
    }

    /// Record a heartbeat and refresh presence.
    pub async fn heartbeat(
        &self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<Presence, StateServiceError> {
        let presence = self
            .registry
            .heartbeat(session_id, now_ms)
            .await
            .ok_or_else(|| StateServiceError::Missing(session_id.into()))?;
        let mut bundle = self.load(session_id).await?;
        bundle.session.last_client_seen_at = Some(now_ms);
        bundle.session.presence = presence;
        bundle.session.last_activity_at = now_ms;
        bundle.session.revision = bundle.session.revision.saturating_add(1);
        self.sessions
            .save_session(&bundle.session, bundle.session.revision - 1)
            .await?;
        Ok(presence)
    }

    /// End a session and keep it resumable.
    pub async fn end_session(
        &self,
        session_id: &str,
        reason: &str,
        now_ms: i64,
    ) -> Result<SessionState, StateServiceError> {
        let _guard = self.mutations.write().await;
        let mut bundle = self.load(session_id).await?;
        bundle.session.end(now_ms, reason);
        bundle.session.revision = bundle.session.revision.saturating_add(1);
        self.sessions
            .save_session(&bundle.session, bundle.session.revision - 1)
            .await?;
        self.registry.end(session_id, now_ms, reason).await;
        Ok(bundle.session)
    }

    /// Soft-delete or purge.
    pub async fn delete(
        &self,
        session_id: &str,
        mode: DeleteMode,
        actor: &str,
    ) -> Result<(), StateServiceError> {
        let _guard = self.mutations.write().await;
        self.sessions.delete(session_id, mode, actor).await?;
        self.registry
            .end(session_id, mm_data::rdf::now_ms(), "deleted")
            .await;
        if mode == DeleteMode::Purge {
            self.directory.remove(session_id)?;
        }
        Ok(())
    }

    /// Sweep idle/work-stall leases and reflect ended leases into RDF.
    pub async fn sweep(&self, now_ms: i64) -> Result<usize, StateServiceError> {
        let expiries = self.registry.sweep(now_ms).await;
        let mut changed = 0usize;
        for expiry in expiries {
            match expiry.action {
                crate::session::ExpiryAction::Ended => {
                    self.end_session(&expiry.session_id, &expiry.reason, now_ms)
                        .await?;
                    changed += 1;
                }
                crate::session::ExpiryAction::WorkStall => {
                    let bundle = self.load(&expiry.session_id).await?;
                    self.make_main_waiting(&expiry.session_id, bundle.session.revision, now_ms)
                        .await?;
                    changed += 1;
                }
                _ => {}
            }
        }
        Ok(changed)
    }

    /// List sessions with cursor pagination.
    pub async fn list_sessions(
        &self,
        user_iri: &str,
        cursor: Option<&str>,
        limit: usize,
        filter: ConversationSessionFilter,
    ) -> Result<SessionPage, StateServiceError> {
        Ok(self
            .sessions
            .list_sessions(user_iri, cursor, limit, filter)
            .await?)
    }

    /// List trace manifests for one session.
    pub async fn list_traces(
        &self,
        session_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<TracePage, StateServiceError> {
        Ok(self.sessions.list_traces(session_id, cursor, limit).await?)
    }

    /// Register one trace manifest.
    pub async fn register_trace(
        &self,
        session_id: &str,
        trace: &SessionTrace,
    ) -> Result<(), StateServiceError> {
        self.sessions.register_trace(session_id, trace).await?;
        Ok(())
    }

    /// The inspector status sources for one session.
    pub async fn status(
        &self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<StatusView, StateServiceError> {
        let bundle = self.load(session_id).await?;
        let mut view = StatusView::empty(now_ms);
        view.state = StateStatusView {
            revision: bundle.conversation.revision,
            topic: bundle.conversation.topic.clone(),
            depth: bundle.conversation.depth,
            status: bundle
                .conversation
                .status
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        serde_json::to_value(value).unwrap_or(Value::Null),
                    )
                })
                .collect(),
            updated_at: bundle.conversation.updated_at,
            updated_by: format!("{:?}", bundle.conversation.updated_by).to_lowercase(),
        };
        view.session = SessionStatusView {
            session_id: bundle.session.session_id.clone(),
            user_id: bundle.session.user_id.clone(),
            lifecycle: bundle.session.lifecycle.as_str().into(),
            presence: format!("{:?}", bundle.session.presence).to_lowercase(),
            working: bundle.session.working(),
            last_activity_at: bundle.session.last_activity_at,
            traces: bundle
                .session
                .traces
                .iter()
                .map(|trace| format!("{:?}", trace.kind).to_lowercase())
                .collect(),
            session_dir: bundle.session.session_dir.clone(),
            resume_epoch: bundle.session.resume.epoch,
        };
        view.frames = FramesStatusView {
            revision: bundle.frames.revision,
            frames: bundle
                .frames
                .frames
                .iter()
                .map(|frame| FrameStatusView {
                    id: frame.id.clone(),
                    template_id: frame.template_id.clone(),
                    template_version: frame.template_version,
                    kind: format!("{:?}", frame.kind).to_lowercase(),
                    status: format!("{:?}", frame.status).to_lowercase(),
                    entered_at: frame.entered_at,
                    updated_at: frame.updated_at,
                    expires_at: frame.expires_at,
                })
                .collect(),
        };
        view.prompt = None;
        view.budget = BudgetStatusView {
            tokens_used: 0,
            token_cap: 0,
            turns_used: 0,
            turn_cap: 0,
        };
        view.data = DataStatusView {
            data_revision: 0,
            index_pending: 0,
            index_lag_ms: 0,
            schemas: 0,
            namespaces: 0,
        };
        view.plans = PlansStatusView { items: Vec::new() };
        view.actions = ActionsStatusView {
            scheduled: Vec::new(),
            recent: Vec::new(),
            next_run_at: None,
        };
        view.automation = AutomationStatusView {
            supervisor: "idle".into(),
            tick_secs: 30,
            frame_timers: 0,
            next_action_at: None,
        };
        view.interjections = InterjectionsStatusView {
            policy_id: "global.default".into(),
            suppressed: false,
            reason: None,
            queue: 0,
        };
        view.updated_at = now_ms;
        Ok(view)
    }

    /// A summary for the current session, useful after bootstrap.
    pub async fn summary(&self, session_id: &str) -> Result<SessionSummary, StateServiceError> {
        let bundle = self.load(session_id).await?;
        Ok(SessionSummary {
            session_id: bundle.session.session_id.clone(),
            user_id: bundle.session.user_id.clone(),
            topic: bundle.conversation.topic,
            lifecycle: bundle.session.lifecycle,
            presence: bundle.session.presence,
            working: bundle.session.working(),
            trace_count: u32::try_from(bundle.session.traces.len()).unwrap_or(u32::MAX),
            trace_kinds: bundle
                .session
                .traces
                .iter()
                .map(|trace| trace.kind)
                .collect(),
            last_activity_at: bundle.session.last_activity_at,
            session_dir: bundle.session.session_dir,
            resumable: bundle.session.lifecycle == SessionLifecycle::Ended,
            deleted: bundle.session.deleted_at.is_some(),
        })
    }

    /// Set work status for a catalog item. Used by the Phase 2 tools and the
    /// Phase 1 work-stall tests.
    pub async fn set_work_status(
        &self,
        session_id: &str,
        work_id: &str,
        status: WorkStatus,
        expected_revision: u64,
        now_ms: i64,
    ) -> Result<SessionState, StateServiceError> {
        let _guard = self.mutations.write().await;
        let mut bundle = self.load(session_id).await?;
        let item = if work_id == "main" {
            &mut bundle.session.work.main
        } else {
            bundle
                .session
                .work
                .parallel
                .get_mut(work_id)
                .ok_or_else(|| SessionError::Invalid(format!("unknown work item {work_id}")))?
        };
        item.status = status;
        item.last_activity_at = now_ms;
        bundle.session.work.validate()?;
        bundle.session.revision = bundle.session.revision.saturating_add(1);
        self.sessions
            .save_session(&bundle.session, expected_revision)
            .await?;
        self.registry.touch_work(session_id, now_ms).await;
        Ok(bundle.session)
    }

    /// Add a parallel work item.
    pub async fn add_work_item(
        &self,
        session_id: &str,
        item: WorkItem,
        expected_revision: u64,
    ) -> Result<SessionState, StateServiceError> {
        let _guard = self.mutations.write().await;
        let mut bundle = self.load(session_id).await?;
        bundle.session.work.upsert(item)?;
        bundle.session.revision = bundle.session.revision.saturating_add(1);
        self.sessions
            .save_session(&bundle.session, expected_revision)
            .await?;
        Ok(bundle.session)
    }

    /// Reset the session's main work item to waiting.
    pub async fn make_main_waiting(
        &self,
        session_id: &str,
        expected_revision: u64,
        now_ms: i64,
    ) -> Result<SessionState, StateServiceError> {
        let _guard = self.mutations.write().await;
        let mut bundle = self.load(session_id).await?;
        let main: &mut WorkItem = &mut bundle.session.work.main;
        main.status = WorkStatus::Waiting;
        main.last_activity_at = now_ms;
        bundle.session.work.validate()?;
        bundle.session.revision = bundle.session.revision.saturating_add(1);
        self.sessions
            .save_session(&bundle.session, expected_revision)
            .await?;
        Ok(bundle.session)
    }

    /// The default work catalog, exposed for bootstrap tests.
    pub fn default_work(now_ms: i64) -> WorkCatalog {
        WorkCatalog::new(now_ms)
    }

    fn frame_step(&self, template_id: &str) -> Result<String, StateServiceError> {
        self.frames
            .template(template_id)
            .map(|template| template.step.clone())
            .ok_or_else(|| FrameError::UnknownTemplate(template_id.into()).into())
    }

    async fn write_frame_status(
        &self,
        session_id: &str,
        step: &str,
        attempt: u8,
        now_ms: i64,
    ) -> Result<ConversationState, StateServiceError> {
        let mut state = self.conversation(session_id).await?;
        let expected = state.revision;
        state.set_status_values(
            [
                ("frame_step".to_owned(), StatusValue::Text(step.to_owned())),
                (
                    "frame_attempt".to_owned(),
                    StatusValue::Integer(i64::from(attempt)),
                ),
                ("awaiting_user".to_owned(), StatusValue::Bool(true)),
                ("next_utterance_at".to_owned(), StatusValue::Integer(0)),
            ],
            StateOrigin::Cm,
            now_ms,
        )?;
        self.conversation.save(&state, expected).await?;
        Ok(state)
    }

    async fn cancel_utterance_inner(&self, session_id: &str) -> Result<(), StateServiceError> {
        let mut bundle = self.load(session_id).await?;
        if bundle.session.pending_utterance.take().is_some() {
            bundle.session.revision = bundle.session.revision.saturating_add(1);
            self.sessions
                .save_session(&bundle.session, bundle.session.revision - 1)
                .await?;
        }
        let mut state = self.conversation(session_id).await?;
        let expected = state.revision;
        state.set_status(
            "next_utterance_at",
            StatusValue::Integer(0),
            StateOrigin::Cm,
            mm_data::rdf::now_ms(),
        )?;
        self.conversation.save(&state, expected).await?;
        Ok(())
    }

    async fn ensure_writable(&self, session_id: &str) -> Result<(), StateServiceError> {
        let bundle = self.load(session_id).await?;
        self.ensure_open(&bundle.session)
    }

    fn ensure_open(&self, session: &SessionState) -> Result<(), StateServiceError> {
        if session.deleted_at.is_some() {
            return Err(StateServiceError::Deleted(session.session_id.clone()));
        }
        if session.lifecycle != SessionLifecycle::Active {
            return Err(StateServiceError::NotOpen(
                session.lifecycle.as_str().into(),
            ));
        }
        Ok(())
    }
}

fn frame_expectation(bundle: &SessionBundle, expected: u64) -> u64 {
    if expected == bundle.frames.revision {
        expected
    } else if expected == bundle.conversation.revision {
        bundle.frames.revision
    } else {
        expected
    }
}

fn frame_attempt(state: &ConversationState) -> u8 {
    match state.status.get("frame_attempt") {
        Some(StatusValue::Integer(value)) => u8::try_from(*value).unwrap_or(0).min(5),
        _ => 0,
    }
}

/// State-service failures.
#[derive(Debug, Error)]
pub enum StateServiceError {
    /// Conversation-store failure.
    #[error(transparent)]
    Conversation(#[from] ConversationError),
    /// Session-store failure.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// Frame-engine failure.
    #[error(transparent)]
    Frame(#[from] FrameError),
    /// Session-directory failure.
    #[error(transparent)]
    Directory(#[from] SessionDirectoryError),
    /// Data-layer failure.
    #[error(transparent)]
    Data(#[from] mm_data::DataError),
    /// Prompt-tree failure.
    #[error(transparent)]
    Prompt(#[from] mm_prompt::PromptError),
    /// Session absent.
    #[error("session not found: {0}")]
    Missing(String),
    /// Session soft-deleted.
    #[error("session deleted: {0}")]
    Deleted(String),
    /// Session is not active.
    #[error("session not open: {0}")]
    NotOpen(String),
}

/// Result alias for the state service.
pub type Result<T, E = StateServiceError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_alias_is_usable() {
        fn accepts(_: Result<()>) {}
        accepts(Ok(()));
    }
}
