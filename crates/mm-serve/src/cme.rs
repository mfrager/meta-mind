//! The Phase 7 conversation-engine runtime for the console.
//!
//! `mm-cme` models one session per [`SessionDriver`]. The console keeps a map
//! of session identifiers to the driver's message/outcome channels, so a chat
//! request can address its session while the driver owns the state machine.
//! Each session has its own observability hub: snapshots, transition events and
//! Prometheus text are isolated by design, which is what the per-session routes
//! need.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mm_cme::{
    Dashboard, DriverMessage, ImprovementEngine, ImprovementLoop, MemoryBridge, MemoryBridgeConfig,
    Observability, ProactiveConfig, ProactiveEngine, Report, Session, SessionDriver,
    SessionSnapshot, SmTransitionEvent, TurnExecutor, TurnOutcome, ViewingContext,
};
use mm_data::agentstream::{AgentstreamEvent, EventProvenance, EventType, SessionMetadata};
use mm_data::{MemorySystem, SessionId, UserId};
use serde_json::json;
use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex};

use crate::dto::chat::ChatFrame;
use crate::error::AppError;
use crate::feature_flags::FeatureFlags;

/// The stable operator identity used by the console's CME sessions.
pub const CONSOLE_USER: &str = "console-operator";

/// One live session: its driver mailbox, its completed-turn receiver, and its
/// observability hub.
struct CmeSession {
    messages: mpsc::UnboundedSender<DriverMessage>,
    outcomes: AsyncMutex<mpsc::UnboundedReceiver<TurnOutcome>>,
    observability: Arc<Observability>,
}

/// Session manager shared by every CME route.
pub struct CmeRuntime {
    memory: Arc<MemorySystem>,
    features: FeatureFlags,
    sessions: Mutex<HashMap<String, Arc<CmeSession>>>,
}

impl std::fmt::Debug for CmeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CmeRuntime")
            .field("sessions", &self.session_ids())
            .field("features", &self.features)
            .finish()
    }
}

impl CmeRuntime {
    /// Build a runtime over the unified memory system.
    pub fn new(memory: Arc<MemorySystem>, features: FeatureFlags) -> Self {
        Self {
            memory,
            features,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The memory system behind the runtime.
    pub fn memory(&self) -> &Arc<MemorySystem> {
        &self.memory
    }

    /// The flags in force.
    pub fn features(&self) -> FeatureFlags {
        self.features
    }

    /// Session identifiers currently owned by this runtime, sorted.
    pub fn session_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .sessions
            .lock()
            .expect("cme sessions lock")
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    /// The latest snapshot for a session.
    pub fn snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.session(session_id)?
            .observability
            .latest_snapshot(session_id)
    }

    /// Subscribe to one session's transition stream.
    pub fn subscribe(&self, session_id: &str) -> Option<broadcast::Receiver<SmTransitionEvent>> {
        Some(self.session(session_id)?.observability.subscribe())
    }

    /// The session's Prometheus text.
    pub fn metrics(&self, session_id: &str) -> Option<String> {
        Some(self.session(session_id)?.observability.prometheus())
    }

    fn session(&self, session_id: &str) -> Option<Arc<CmeSession>> {
        self.sessions
            .lock()
            .expect("cme sessions lock")
            .get(session_id)
            .cloned()
    }

    /// Run one turn. The first call for a chat session creates its driver; the
    /// per-session outcome mutex serialises turns within that session.
    pub async fn run_turn(
        &self,
        session_id: &str,
        query: String,
        executor: TurnExecutor,
    ) -> Result<TurnOutcome, AppError> {
        let session = self.ensure_session(session_id, executor).await?;
        let mut outcomes = session.outcomes.lock().await;
        session
            .messages
            .send(DriverMessage::StartTurn { query })
            .map_err(|_| AppError::Internal("the CME driver stopped".into()))?;
        outcomes
            .recv()
            .await
            .ok_or_else(|| AppError::Internal("the CME driver stopped before answering".into()))
    }

    /// Write one completed turn and its wire frames to the agentstream log.
    ///
    /// Episodic turns are logged by the driver's memory bridge; this records the
    /// raw conversation frames the UI saw.
    pub async fn log_turn_frames(
        &self,
        session_id: &str,
        request_id: &str,
        query: &str,
        outcome: &TurnOutcome,
        frames: &[(u64, ChatFrame)],
    ) {
        if !self.features.agentstream_logging {
            return;
        }
        let user = AgentstreamEvent::new(
            session_id,
            EventType::UserMessage,
            json!({ "requestId": request_id, "query": query }),
            EventProvenance {
                source: "cme".into(),
                model: None,
            },
        )
        .with_turn(outcome.turn_id.to_string());
        self.log_event(user).await;

        for (seq, frame) in frames {
            let kind = match frame {
                ChatFrame::ToolCall { .. } => EventType::ToolCall,
                ChatFrame::ToolResult { .. } => EventType::ToolResult,
                _ => EventType::AgentMessage,
            };
            let event = AgentstreamEvent::new(
                session_id,
                kind,
                json!({ "requestId": request_id, "seq": seq, "frame": frame }),
                EventProvenance {
                    source: "cme".into(),
                    model: outcome.model.clone(),
                },
            )
            .with_turn(outcome.turn_id.to_string());
            self.log_event(event).await;
        }
    }

    async fn log_event(&self, event: AgentstreamEvent) {
        if let Err(error) = self.memory.log_agentstream_event(event).await {
            eprintln!("mm-serve: cme agentstream logging failed: {error}");
        }
    }

    async fn ensure_session(
        &self,
        session_id: &str,
        executor: TurnExecutor,
    ) -> Result<Arc<CmeSession>, AppError> {
        if let Some(session) = self.session(session_id) {
            return Ok(session);
        }

        let mut session = Session::new(UserId::new(CONSOLE_USER), None)
            .map_err(|error| AppError::Internal(format!("cme session: {error}")))?;
        session.id = SessionId::new(session_id.to_owned());
        // The console is an operator surface: expose the controller's full tool
        // catalog on every CME turn. The driver's `MemoryBridge` may merge learned
        // preferences later, but `or_insert` keeps this opt-in in force.
        session
            .context
            .preferences
            .values
            .insert("tools".into(), 1.0);
        self.memory
            .ensure_session(
                session_id,
                SessionMetadata {
                    user_id: CONSOLE_USER.into(),
                    project_id: None,
                    title: None,
                },
            )
            .await
            .map_err(|error| AppError::Internal(format!("cme memory session: {error}")))?;

        if let Some(tier) = self.features.voice_tier {
            session
                .dispatch_regions("start_voice", None)
                .map_err(|error| AppError::Internal(format!("cme voice: {error}")))?;
            session
                .dispatch_regions("tier_negotiated", Some(json!({ "tier": tier.as_str() })))
                .map_err(|error| AppError::Internal(format!("cme voice: {error}")))?;
        }

        let bridge = Arc::new(MemoryBridge::new(
            Arc::clone(&self.memory),
            MemoryBridgeConfig {
                compression_trigger_turns: if self.features.memory_compression_enabled {
                    self.memory
                        .config()
                        .episodic
                        .compression_trigger_turns
                        .max(1)
                } else {
                    u32::MAX
                },
                auto_aggregate: self.features.memory_compression_enabled,
                auto_sync_user: false,
            },
        ));
        let observability = Arc::new(Observability::new(256));
        observability.publish_snapshot(SessionSnapshot::capture(&session));

        let (driver, messages, outcomes) =
            SessionDriver::spawn_with_executor(session, Duration::from_secs(15), executor);
        let mut driver = driver
            .with_memory(bridge)
            .with_observability(Arc::clone(&observability));
        if self.features.proactive_enabled {
            match ProactiveEngine::with_builtins(ProactiveConfig::default()) {
                Ok(engine) => driver = driver.with_proactive(engine),
                Err(error) => {
                    eprintln!("mm-serve: cme proactive engine unavailable: {error}")
                }
            }
        }
        if self.features.improvement_loop_enabled {
            driver = driver.with_improvement(ImprovementEngine::new(
                Report::new("cme-session-report", "Session report"),
                Dashboard::new(
                    "cme-session-dashboard",
                    "Session dashboard",
                    ViewingContext::Desktop,
                ),
                10,
                ImprovementLoop::new(),
            ));
        }
        tokio::spawn(async move {
            if let Err(error) = driver.run().await {
                eprintln!("mm-serve: cme session driver exited: {error}");
            }
        });

        let handle = Arc::new(CmeSession {
            messages,
            outcomes: AsyncMutex::new(outcomes),
            observability,
        });
        let mut sessions = self.sessions.lock().expect("cme sessions lock");
        if let Some(existing) = sessions.get(session_id) {
            return Ok(Arc::clone(existing));
        }
        sessions.insert(session_id.to_owned(), Arc::clone(&handle));
        Ok(handle)
    }
}
