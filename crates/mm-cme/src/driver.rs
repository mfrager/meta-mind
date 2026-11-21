//! The session driver: one `tokio::select!` loop over external events, ticks
//! and turn work.
//!
//! Turns are planned, routed and streamed by [`TurnExecutor`]; this loop only
//! owns session events, ticks and the outcome channel. A deterministic executor
//! (`Echo:` output, no model) is the default; callers that have a client use
//! [`SessionDriver::spawn_with_executor`].

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

use crate::error::{CmeError, Result};
use crate::executor::TurnExecutor;
use crate::improvement::ImprovementEngine;
use crate::memory::MemoryBridge;
use crate::observability::{Observability, SmObserver};
use crate::plugins::PluginContext;
use crate::regions::proactive::ProactiveEngine;
use crate::session::{Session, SessionEvent};
use crate::turn::TurnOutcome;

/// Messages the driver accepts.
#[derive(Debug, Clone)]
pub enum DriverMessage {
    /// A session lifecycle event.
    Event(SessionEvent),
    /// Start and run one turn for this query.
    StartTurn {
        /// The user's request.
        query: String,
    },
    /// Stop the loop.
    Shutdown,
}

/// The session driver.
pub struct SessionDriver {
    session: Session,
    executor: TurnExecutor,
    messages: mpsc::UnboundedReceiver<DriverMessage>,
    outcomes: mpsc::UnboundedSender<TurnOutcome>,
    tick_interval: Duration,
    memory: Option<Arc<MemoryBridge>>,
    proactive: Option<ProactiveEngine>,
    last_query: String,
    observability: Option<Arc<Observability>>,
    improvement: Option<ImprovementEngine>,
}

impl std::fmt::Debug for SessionDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionDriver")
            .field("session", &self.session.id)
            .field("state", &self.session.state())
            .field("tick_interval", &self.tick_interval)
            .finish_non_exhaustive()
    }
}

impl SessionDriver {
    /// Create the driver and its two channels.
    ///
    /// The first channel feeds the driver; the second yields completed turn
    /// outcomes to an HTTP/UI layer. A small tick interval makes proactive work
    /// responsive; tests use a long one.
    pub fn spawn(
        session: Session,
        tick_interval: Duration,
    ) -> (
        Self,
        mpsc::UnboundedSender<DriverMessage>,
        mpsc::UnboundedReceiver<TurnOutcome>,
    ) {
        Self::spawn_with_executor(session, tick_interval, TurnExecutor::deterministic())
    }

    /// Create a driver with a specific executor (a model client, router and
    /// tools), sharing the same channels as [`SessionDriver::spawn`].
    pub fn spawn_with_executor(
        session: Session,
        tick_interval: Duration,
        executor: TurnExecutor,
    ) -> (
        Self,
        mpsc::UnboundedSender<DriverMessage>,
        mpsc::UnboundedReceiver<TurnOutcome>,
    ) {
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let (outcome_tx, outcome_rx) = mpsc::unbounded_channel();
        (
            Self {
                session,
                executor,
                messages: message_rx,
                outcomes: outcome_tx,
                tick_interval,
                memory: None,
                proactive: None,
                last_query: String::new(),
                observability: None,
                improvement: None,
            },
            message_tx,
            outcome_rx,
        )
    }

    /// The session behind the driver.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Attach the memory bridge. The driver then opens a memory session at
    /// startup, logs each successful turn, compresses on the bridge's trigger and
    /// writes the final snapshot on shutdown.
    pub fn with_memory(mut self, memory: Arc<MemoryBridge>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Attach a proactive engine. The driver then runs one plugin cycle per tick
    /// using the most recent query, and injects the selected items into the
    /// session stream.
    pub fn with_proactive(mut self, proactive: ProactiveEngine) -> Self {
        self.proactive = Some(proactive);
        self
    }

    /// Attach the observability hub. Session dispatch records and turn/chunk
    /// transitions are streamed as `SMTransition` events, and a snapshot is
    /// taken at startup, after each turn and at shutdown.
    pub fn with_observability(mut self, observability: Arc<Observability>) -> Self {
        self.observability = Some(observability);
        self
    }

    /// Attach the scheduled improvement engine. Each due tick improves the
    /// report and dashboard artifacts and advances the improvement region.
    pub fn with_improvement(mut self, improvement: ImprovementEngine) -> Self {
        self.improvement = Some(improvement);
        self
    }

    /// Drive the session until shutdown or until every sender is dropped.
    pub async fn run(mut self) -> Result<Session> {
        if let Some(memory) = self.memory.clone() {
            if let Err(error) = memory.start_session(&mut self.session, None).await {
                tracing::warn!(%error, "memory session could not start");
            }
        }
        if let Some(observability) = &self.observability {
            let observer = observability.clone() as Arc<dyn SmObserver>;
            self.executor.set_observer(Arc::clone(&observer));
            self.session.observer = Some(observer);
            observability.observe_session(&self.session);
        }
        self.dispatch_session(SessionEvent::InitComplete)?;
        let mut ticker = interval(self.tick_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;
                message = self.messages.recv() => {
                    let Some(message) = message else {
                        break;
                    };
                    if self.handle(message).await? {
                        break;
                    }
                }
                _ = ticker.tick() => {
                    self.proactive_tick().await?;
                    self.improvement_tick()?;
                }
            }
        }

        self.shutdown().await?;
        if let Some(observability) = &self.observability {
            observability.observe_session(&self.session);
        }
        Ok(self.session)
    }

    /// Dispatch a session event, forwarding the resulting transitions to the
    /// observability hub when one is attached.
    fn dispatch_session(&mut self, event: SessionEvent) -> Result<()> {
        self.session.dispatch(event)?;
        Ok(())
    }

    /// Handle one message. Returns `true` when the loop should stop.
    async fn handle(&mut self, message: DriverMessage) -> Result<bool> {
        match message {
            DriverMessage::Event(event) => {
                if matches!(&event, SessionEvent::Tick) && self.proactive.is_some() {
                    self.proactive_tick().await?;
                    return Ok(false);
                }
                let is_barge_in = matches!(&event, SessionEvent::BargeIn);
                self.dispatch_session(event)?;
                if is_barge_in {
                    self.barge_in()?;
                }
            }
            DriverMessage::StartTurn { query } => {
                let outcome = self.run_turn(query).await?;
                self.outcomes
                    .send(outcome.clone())
                    .map_err(|_| CmeError::ChannelClosed)?;
                self.dispatch_session(SessionEvent::TurnCompleted(outcome))?;
                if let Some(observability) = &self.observability {
                    observability.observe_session(&self.session);
                }
            }
            DriverMessage::Shutdown => return Ok(true),
        }
        Ok(false)
    }

    /// Run one proactive cycle, or a bare region tick when no engine is attached.
    async fn proactive_tick(&mut self) -> Result<()> {
        let profile = self.session.context.preferences.clone();
        let affect = self.session.context.affect;
        let query = self.last_query.clone();
        let Some(engine) = self.proactive.as_mut() else {
            return self.dispatch_session(SessionEvent::Tick);
        };
        let ctx = PluginContext {
            query: &query,
            preferences: &profile,
            affect,
            memory_hits: &[],
        };
        if let Err(error) = engine.run(&mut self.session, &ctx) {
            tracing::warn!(%error, "proactive cycle failed");
        }
        Ok(())
    }

    /// Run a scheduled improvement cycle when one is due.
    fn improvement_tick(&mut self) -> Result<()> {
        let Some(engine) = self.improvement.as_mut() else {
            return Ok(());
        };
        if let Err(error) = engine.on_tick(&mut self.session) {
            tracing::warn!(%error, "improvement cycle failed");
        }
        Ok(())
    }

    /// Run one turn end to end through the executor and the Turn/Chunk machines.
    async fn run_turn(&mut self, query: String) -> Result<TurnOutcome> {
        // Clone what the executor needs before the mutable turn borrow starts.
        let context = self.session.context.clone();
        let voice = self.session.voice_turn_config();
        let query_for_memory = query.clone();
        self.last_query = query.clone();
        let turn = self.session.begin_turn(query)?;
        let detail_level = turn.plan.as_ref().map_or(3, |plan| plan.detail_level);
        let result = self.executor.execute(turn, &context, voice).await;
        let outcome = self.session.finish_turn();
        let outcome = match result {
            Ok(_) => outcome,
            Err(error) => return Err(error),
        }?;
        if let Some(memory) = self.memory.clone() {
            if outcome.is_success() {
                if let Err(error) = memory
                    .after_turn(&mut self.session, &query_for_memory, detail_level, &outcome)
                    .await
                {
                    tracing::warn!(%error, "memory cycle failed after turn");
                }
            }
        }
        Ok(outcome)
    }

    /// Interrupt the active turn and the voice pipeline.
    fn barge_in(&mut self) -> Result<()> {
        self.session.note_barge_in();
        if let Some(turn) = self.session.active_turn_mut() {
            turn.barge_in()?;
        }
        self.session.dispatch_regions("barge_in_detected", None)?;
        Ok(())
    }

    /// Send the regions to their shutdown states and terminate the session.
    async fn shutdown(&mut self) -> Result<()> {
        if self.session.state() != "terminating" {
            self.dispatch_session(SessionEvent::ShutdownRequest)?;
        }
        self.session.dispatch_regions("session_pause", None)?;
        self.dispatch_session(SessionEvent::RegionsTerminated)?;
        if let Some(memory) = self.memory.clone() {
            if let Err(error) = memory.end_session(&self.session).await {
                tracing::warn!(%error, "memory session could not be closed");
            }
        }
        Ok(())
    }
}
