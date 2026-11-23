//! Turn: the child state machine spawned for each user input or proactive item.
//!
//! `planning → routing → (speculative_warm) → executing → streaming`, with
//! `tool_calling` and `waiting_confirm` as streaming sub-states, then
//! `finalizing → completed`. A barge-in enters `interrupted`, which remembers
//! `streaming`; `resume` restores it through `mm-sm`'s history pseudostate.

use mm_data::{SessionId, TurnId};
use mm_sm::{SmEvent, SmInstance, StateMachine, Transition, TransitionRecord};
use serde::{Deserialize, Serialize};

use crate::chunk::{Chunk, ChunkDirective, ChunkEvent, ChunkKind, ChunkState};
use crate::error::Result;
use crate::planner::{ResponseModality, ToolPolicy, TurnBudget};
use crate::voice::VoiceTurnConfig;

/// Turn lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnState {
    /// Building the plan.
    Planning,
    /// Choosing the model.
    Routing,
    /// Pre-warming for a streaming voice tier.
    SpeculativeWarm,
    /// Waiting for the model's first token.
    Executing,
    /// Receiving tokens.
    Streaming,
    /// Running tools.
    ToolCalling,
    /// Waiting on a confirmation gate.
    WaitingConfirm,
    /// Honesty check and final bookkeeping.
    Finalizing,
    /// Done.
    Completed,
    /// Barged in; history remembers the state to resume.
    Interrupted,
    /// Failed.
    Failed,
}

impl TurnState {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Routing => "routing",
            Self::SpeculativeWarm => "speculative_warm",
            Self::Executing => "executing",
            Self::Streaming => "streaming",
            Self::ToolCalling => "tool_calling",
            Self::WaitingConfirm => "waiting_confirm",
            Self::Finalizing => "finalizing",
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
        }
    }

    /// Whether the turn reached a terminal state.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

/// Events a turn reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnEvent {
    /// The deterministic plan is ready.
    PlanReady(TurnPlan),
    /// A model was selected. `speculative` selects the pre-warm path.
    RouteSelected {
        /// Whether voice Tier 2/3 pre-warming applies.
        speculative: bool,
        /// The selected model, for the record.
        model: String,
    },
    /// Pre-warming finished.
    SpeculativeReady,
    /// The model stream opened.
    LlmStreamStart,
    /// The stream asked for a tool.
    ToolCall,
    /// The tool returned.
    ToolResult,
    /// A tool needs the user's confirmation.
    ConfirmationRequired,
    /// The user answered.
    ConfirmationAnswer,
    /// Every chunk finished.
    AllChunksDone,
    /// Final bookkeeping is complete.
    Finalized,
    /// The user interrupted.
    BargeInDetected,
    /// Resume from history.
    Resume,
    /// The user cancelled.
    CancelRequested,
    /// Failure.
    Error(String),
}

impl TurnEvent {
    /// The wire name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::PlanReady(_) => "plan_ready",
            Self::RouteSelected { .. } => "route_selected",
            Self::SpeculativeReady => "speculative_ready",
            Self::LlmStreamStart => "llm_stream_start",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::ConfirmationRequired => "confirmation_required",
            Self::ConfirmationAnswer => "confirmation_answer",
            Self::AllChunksDone => "all_chunks_done",
            Self::Finalized => "finalized",
            Self::BargeInDetected => "barge_in_detected",
            Self::Resume => "resume",
            Self::CancelRequested => "cancel_requested",
            Self::Error(_) => "error",
        }
    }

    fn into_sm(self) -> SmEvent {
        let name = self.name();
        match self {
            Self::PlanReady(plan) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "plan": plan }))
            }
            Self::RouteSelected { speculative, model } => SmEvent::new(name)
                .with_payload(serde_json::json!({ "speculative": speculative, "model": model })),
            Self::Error(detail) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "detail": detail }))
            }
            _ => SmEvent::new(name),
        }
    }
}

/// The deterministic turn plan. [`TurnPlanner`](crate::planner::TurnPlanner)
/// produces the full form; this is what the turn machine and executor consume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnPlan {
    /// Turn identifier.
    pub turn_id: TurnId,
    /// Session identifier.
    pub session_id: SessionId,
    /// The user's request.
    pub user_query: String,
    /// Response shape.
    pub modality: ResponseModality,
    /// Detail level 1–5.
    pub detail_level: u8,
    /// Planned chunks.
    pub chunks: Vec<ChunkDirective>,
    /// Whether and how tools may run.
    #[serde(default)]
    pub tool_policy: ToolPolicy,
    /// Voice configuration when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_config: Option<VoiceTurnConfig>,
    /// Token and cost allocation.
    #[serde(default)]
    pub budget: TurnBudget,
    /// Provenance for proactive and child turns.
    #[serde(default)]
    pub metadata: TurnPlanMetadata,
}

/// Where a turn came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TurnPlanMetadata {
    /// Parent turn, when this is a proactive or plugin child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<TurnId>,
    /// True for proactive turns.
    pub is_proactive: bool,
    /// `user_input`, `proactive`, `plugin`.
    pub trigger: String,
}

impl TurnPlan {
    /// A single-chunk explanatory plan, for callers that have not run the planner.
    pub fn simple(session_id: SessionId, turn_id: TurnId, query: impl Into<String>) -> Self {
        Self {
            turn_id,
            session_id,
            user_query: query.into(),
            modality: ResponseModality::Explain,
            detail_level: 3,
            chunks: vec![ChunkDirective::text("answer", "answer")],
            tool_policy: ToolPolicy::None,
            voice_config: None,
            budget: TurnBudget::default(),
            metadata: TurnPlanMetadata {
                trigger: "user_input".to_owned(),
                ..TurnPlanMetadata::default()
            },
        }
    }
}

/// One emitted chunk, retained after the turn so transport adapters can render
/// the turn's modalities (not only its concatenated text).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkEmission {
    /// Chunk identifier.
    pub chunk_id: String,
    /// What the chunk emits.
    pub kind: ChunkKind,
    /// Terminal state.
    pub state: String,
    /// Buffer contents (empty for audio in a text turn).
    pub text: String,
}

/// One tool call and its result, retained after the turn so transport adapters
/// can render the tool cards and log the exact arguments and output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallEmission {
    /// The provider's call id.
    pub call_id: String,
    /// The catalog name.
    pub name: String,
    /// The JSON arguments, exactly as the model sent them.
    pub arguments: String,
    /// The handler's result text.
    pub result: String,
    /// Whether the handler reported success.
    pub ok: bool,
}

/// The result of a completed turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnOutcome {
    /// Turn identifier.
    pub turn_id: TurnId,
    /// Final state name.
    pub state: String,
    /// Chunks that completed.
    pub chunks_completed: usize,
    /// Text emitted, concatenated from the text chunks.
    pub text: String,
    /// Failure detail, when the turn failed.
    pub error: Option<String>,
    /// The selected model, when routing completed.
    pub model: Option<String>,
    /// Emitted chunks, including their modality.
    pub chunks: Vec<ChunkEmission>,
    /// Tool calls that ran, in order.
    #[serde(default)]
    pub tool_calls: Vec<ToolCallEmission>,
    /// Immutable prompt instance used for the model call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_instance_id: Option<String>,
    /// Prompt hash pinned for the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hash: Option<String>,
    /// Selected prompt variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_variant: Option<String>,
}

impl TurnOutcome {
    /// Whether the turn completed successfully.
    pub fn is_success(&self) -> bool {
        self.state == TurnState::Completed.as_str()
    }
}

/// The turn state-machine definition.
pub fn turn_machine() -> StateMachine {
    StateMachine::new("turn", TurnState::Planning.as_str())
        .state(TurnState::Routing.as_str())
        .state(TurnState::SpeculativeWarm.as_str())
        .state(TurnState::Executing.as_str())
        .state(TurnState::Streaming.as_str())
        .state(TurnState::ToolCalling.as_str())
        .state(TurnState::WaitingConfirm.as_str())
        .state(TurnState::Finalizing.as_str())
        .state(TurnState::Completed.as_str())
        .state(TurnState::Interrupted.as_str())
        .state(TurnState::Failed.as_str())
        .history(TurnState::Interrupted.as_str())
        .transition(Transition::new(
            TurnState::Planning.as_str(),
            "plan_ready",
            TurnState::Routing.as_str(),
        ))
        .transition(
            Transition::new(
                TurnState::Routing.as_str(),
                "route_selected",
                TurnState::SpeculativeWarm.as_str(),
            )
            .guard(|ctx| ctx.event.payload["speculative"].as_bool().unwrap_or(false)),
        )
        .transition(
            Transition::new(
                TurnState::Routing.as_str(),
                "route_selected",
                TurnState::Executing.as_str(),
            )
            .guard(|ctx| !ctx.event.payload["speculative"].as_bool().unwrap_or(false)),
        )
        .transition(Transition::new(
            TurnState::SpeculativeWarm.as_str(),
            "speculative_ready",
            TurnState::Executing.as_str(),
        ))
        .transition(Transition::new(
            TurnState::Executing.as_str(),
            "llm_stream_start",
            TurnState::Streaming.as_str(),
        ))
        .transition(Transition::new(
            TurnState::Streaming.as_str(),
            "tool_call",
            TurnState::ToolCalling.as_str(),
        ))
        .transition(Transition::new(
            TurnState::ToolCalling.as_str(),
            "tool_result",
            TurnState::Streaming.as_str(),
        ))
        .transition(Transition::new(
            TurnState::Streaming.as_str(),
            "confirmation_required",
            TurnState::WaitingConfirm.as_str(),
        ))
        .transition(Transition::new(
            TurnState::WaitingConfirm.as_str(),
            "confirmation_answer",
            TurnState::Streaming.as_str(),
        ))
        .transition(Transition::new(
            TurnState::Streaming.as_str(),
            "all_chunks_done",
            TurnState::Finalizing.as_str(),
        ))
        .transition(Transition::new(
            TurnState::Finalizing.as_str(),
            "finalized",
            TurnState::Completed.as_str(),
        ))
        .transition(Transition::new(
            TurnState::Streaming.as_str(),
            "barge_in_detected",
            TurnState::Interrupted.as_str(),
        ))
        .transition(Transition::to_history(
            TurnState::Interrupted.as_str(),
            "resume",
        ))
        .transition(Transition::any(
            "cancel_requested",
            TurnState::Interrupted.as_str(),
        ))
        .transition(Transition::any("error", TurnState::Failed.as_str()))
}

/// A live turn and its chunks.
#[derive(Debug)]
pub struct Turn {
    /// The state-machine instance.
    pub sm: SmInstance,
    /// The plan, once `PlanReady` was dispatched.
    pub plan: Option<TurnPlan>,
    /// Chunks spawned for the plan.
    pub chunks: Vec<Chunk>,
    /// Turn identifier.
    pub turn_id: TurnId,
    /// Session identifier.
    pub session_id: SessionId,
    /// Selected model.
    pub model: Option<String>,
    /// Tool results collected during the turn.
    pub tool_results: Vec<String>,
    /// Tool calls retained for the transport layer.
    pub tool_calls: Vec<ToolCallEmission>,
    /// Optional transition observer, inherited by spawned chunks.
    pub observer: Option<crate::observability::ObserverHandle>,
    /// Prompt instance used for this turn's model calls.
    pub prompt_instance_id: Option<String>,
    /// Pinned prompt hash.
    pub prompt_hash: Option<String>,
    /// Selected prompt variant.
    pub prompt_variant: Option<String>,
}

impl Turn {
    /// Spawn a turn in `planning`.
    pub fn new(plan: TurnPlan) -> Result<Self> {
        let sm = SmInstance::start_with_data(
            turn_machine(),
            serde_json::json!({ "plan": plan, "model": null, "error": null }),
        )?;
        Ok(Self {
            sm,
            turn_id: plan.turn_id.clone(),
            session_id: plan.session_id.clone(),
            plan: Some(plan),
            chunks: Vec::new(),
            model: None,
            tool_results: Vec::new(),
            tool_calls: Vec::new(),
            observer: None,
            prompt_instance_id: None,
            prompt_hash: None,
            prompt_variant: None,
        })
    }

    /// Replace the plan before `plan_ready`.
    pub fn set_plan(&mut self, plan: TurnPlan) {
        self.turn_id = plan.turn_id.clone();
        self.session_id = plan.session_id.clone();
        self.plan = Some(plan.clone());
        self.sm.data_mut()["plan"] = serde_json::json!(plan);
    }

    /// Pin the prompt instance used by the model call.
    pub fn set_prompt(&mut self, instance_id: String, hash: String, variant: Option<String>) {
        self.prompt_instance_id = Some(instance_id);
        self.prompt_hash = Some(hash);
        self.prompt_variant = variant;
    }

    /// The current state.
    pub fn state(&self) -> &str {
        self.sm.state().as_str()
    }

    /// Whether the turn reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        TurnState::Completed.as_str() == self.state() || TurnState::Failed.as_str() == self.state()
    }

    /// Dispatch one event.
    pub fn dispatch(&mut self, event: TurnEvent) -> Result<Vec<TransitionRecord>> {
        if let TurnEvent::RouteSelected { model, .. } = &event {
            self.model = Some(model.clone());
        }
        if let TurnEvent::Error(detail) = &event {
            self.sm.data_mut()["error"] = serde_json::Value::String(detail.clone());
        }
        let records = self.sm.dispatch(event.into_sm())?;
        if let Some(observer) = &self.observer {
            for record in &records {
                observer.on_transition(record);
            }
        }
        Ok(records)
    }

    /// Planning → routing.
    pub fn plan_ready(&mut self) -> Result<Vec<TransitionRecord>> {
        let plan = self
            .plan
            .clone()
            .ok_or_else(|| crate::error::CmeError::Driver("turn has no plan".to_owned()))?;
        self.dispatch(TurnEvent::PlanReady(plan))
    }

    /// Routing → executing (or speculative warm).
    pub fn route_selected(
        &mut self,
        speculative: bool,
        model: impl Into<String>,
    ) -> Result<Vec<TransitionRecord>> {
        self.dispatch(TurnEvent::RouteSelected {
            speculative,
            model: model.into(),
        })
    }

    /// Spawn the planned chunks and start those without dependencies.
    pub fn spawn_chunks(&mut self) -> Result<Vec<&mut Chunk>> {
        let directives = self
            .plan
            .as_ref()
            .map(|plan| plan.chunks.clone())
            .unwrap_or_default();
        for directive in directives {
            let mut chunk = Chunk::new(directive, Some(self.turn_id.to_string()))?;
            chunk.observer = self.observer.clone();
            self.chunks.push(chunk);
        }
        for chunk in &mut self.chunks {
            if chunk.directive.depends_on.is_empty() {
                chunk.start()?;
            }
        }
        Ok(self.chunks.iter_mut().collect())
    }

    /// Feed one token to every streaming text chunk.
    pub fn token(&mut self, token: impl Into<String>) -> Result<()> {
        let token = token.into();
        for chunk in &mut self.chunks {
            if chunk.state() == ChunkState::Streaming.as_str()
                && matches!(chunk.directive.kind, crate::chunk::ChunkKind::Text)
            {
                chunk.dispatch(ChunkEvent::LlmToken(token.clone()))?;
            }
        }
        Ok(())
    }

    /// Feed one token to a named chunk.
    pub fn token_to(&mut self, chunk_id: &str, token: impl Into<String>) -> Result<()> {
        let token = token.into();
        let chunk = self
            .chunks
            .iter_mut()
            .find(|chunk| chunk.chunk_id.as_str() == chunk_id)
            .ok_or_else(|| crate::error::CmeError::NotFound {
                kind: "chunk",
                id: chunk_id.to_owned(),
            })?;
        if chunk.state() == ChunkState::Streaming.as_str() {
            chunk.dispatch(ChunkEvent::LlmToken(token))?;
        }
        Ok(())
    }

    /// Finish every streaming chunk and release chunks whose dependencies are
    /// now met. Repeats until no chunk changes state, so a dependency chain
    /// resolves in one call.
    pub fn complete_chunks(&mut self) -> Result<()> {
        loop {
            let mut progressed = false;
            for chunk in &mut self.chunks {
                match chunk.state() {
                    "streaming" => {
                        chunk.dispatch(ChunkEvent::LlmComplete)?;
                        progressed = true;
                    }
                    "buffering" => {
                        chunk.dispatch(ChunkEvent::FlushBuffer)?;
                        progressed = true;
                    }
                    _ => {}
                }
            }
            let completed: Vec<String> = self
                .chunks
                .iter()
                .filter(|chunk| chunk.state() == ChunkState::Completed.as_str())
                .map(|chunk| chunk.chunk_id.to_string())
                .collect();
            for chunk in &mut self.chunks {
                if chunk.state() == ChunkState::Pending.as_str()
                    && chunk
                        .directive
                        .depends_on
                        .iter()
                        .all(|dependency| completed.contains(&dependency.to_string()))
                {
                    chunk.start()?;
                    progressed = true;
                }
            }
            if !progressed {
                break;
            }
        }
        Ok(())
    }

    /// Barge in: interrupt the turn and cancel live chunks.
    pub fn barge_in(&mut self) -> Result<()> {
        self.dispatch(TurnEvent::BargeInDetected)?;
        for chunk in &mut self.chunks {
            if !chunk.is_terminal() {
                chunk.cancel()?;
            }
        }
        Ok(())
    }

    /// Resume from `interrupted` through history.
    pub fn resume(&mut self) -> Result<Vec<TransitionRecord>> {
        self.dispatch(TurnEvent::Resume)
    }

    /// Finish: chunks done, finalize, completed.
    pub fn finalize(&mut self) -> Result<TurnOutcome> {
        self.complete_chunks()?;
        self.dispatch(TurnEvent::AllChunksDone)?;
        self.dispatch(TurnEvent::Finalized)?;
        Ok(self.outcome())
    }

    /// Snapshot the outcome without changing state.
    pub fn outcome(&self) -> TurnOutcome {
        let text = self
            .chunks
            .iter()
            .filter(|chunk| matches!(chunk.directive.kind, crate::chunk::ChunkKind::Text))
            .map(Chunk::buffer)
            .filter(|buffer| !buffer.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        TurnOutcome {
            turn_id: self.turn_id.clone(),
            state: self.state().to_owned(),
            chunks_completed: self
                .chunks
                .iter()
                .filter(|chunk| chunk.state() == ChunkState::Completed.as_str())
                .count(),
            text,
            error: self
                .sm
                .data()
                .get("error")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            model: self.model.clone(),
            chunks: self
                .chunks
                .iter()
                .map(|chunk| ChunkEmission {
                    chunk_id: chunk.chunk_id.to_string(),
                    kind: chunk.directive.kind,
                    state: chunk.state().to_owned(),
                    text: chunk.buffer().to_owned(),
                })
                .collect(),
            tool_calls: self.tool_calls.clone(),
            prompt_instance_id: self.prompt_instance_id.clone(),
            prompt_hash: self.prompt_hash.clone(),
            prompt_variant: self.prompt_variant.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn() -> Turn {
        let plan = TurnPlan::simple(SessionId::new("s1"), TurnId::new("t1"), "hello");
        let mut turn = Turn::new(plan).unwrap();
        turn.plan_ready().unwrap();
        turn
    }

    #[test]
    fn turn_transition_table_matches_the_design() {
        assert_eq!(turn().state(), TurnState::Routing.as_str());

        let mut turn = turn();
        turn.route_selected(false, "model").unwrap();
        assert_eq!(turn.state(), TurnState::Executing.as_str());
        turn.dispatch(TurnEvent::LlmStreamStart).unwrap();
        assert_eq!(turn.state(), TurnState::Streaming.as_str());
        turn.dispatch(TurnEvent::ToolCall).unwrap();
        assert_eq!(turn.state(), TurnState::ToolCalling.as_str());
        turn.dispatch(TurnEvent::ToolResult).unwrap();
        turn.dispatch(TurnEvent::ConfirmationRequired).unwrap();
        assert_eq!(turn.state(), TurnState::WaitingConfirm.as_str());
        turn.dispatch(TurnEvent::ConfirmationAnswer).unwrap();
        turn.dispatch(TurnEvent::AllChunksDone).unwrap();
        assert_eq!(turn.state(), TurnState::Finalizing.as_str());
        turn.dispatch(TurnEvent::Finalized).unwrap();
        assert_eq!(turn.state(), TurnState::Completed.as_str());
    }

    #[test]
    fn speculative_route_takes_the_warm_path() {
        let mut turn = turn();
        turn.route_selected(true, "realtime").unwrap();
        assert_eq!(turn.state(), TurnState::SpeculativeWarm.as_str());
        turn.dispatch(TurnEvent::SpeculativeReady).unwrap();
        assert_eq!(turn.state(), TurnState::Executing.as_str());
    }

    #[test]
    fn barge_in_enters_interrupted_and_resume_restores_streaming() {
        let mut turn = turn();
        turn.route_selected(false, "model").unwrap();
        turn.dispatch(TurnEvent::LlmStreamStart).unwrap();
        turn.dispatch(TurnEvent::BargeInDetected).unwrap();
        assert_eq!(turn.state(), TurnState::Interrupted.as_str());
        assert_eq!(
            turn.sm.resume_target().map(|state| state.as_str()),
            Some(TurnState::Streaming.as_str())
        );
        turn.dispatch(TurnEvent::Resume).unwrap();
        assert_eq!(turn.state(), TurnState::Streaming.as_str());
    }

    #[test]
    fn dependencies_release_and_buffered_chunks_flush() {
        let mut plan = TurnPlan::simple(SessionId::new("s1"), TurnId::new("t1"), "hello");
        let mut dependent = ChunkDirective::text("summary", "summary");
        dependent.depends_on = vec![crate::chunk::ChunkId::new("answer")];
        let mut card = ChunkDirective::text("card", "card");
        card.buffered = true;
        plan.chunks = vec![ChunkDirective::text("answer", "answer"), dependent, card];
        let mut turn = Turn::new(plan).unwrap();
        turn.plan_ready().unwrap();
        turn.route_selected(false, "model").unwrap();
        turn.dispatch(TurnEvent::LlmStreamStart).unwrap();
        turn.spawn_chunks().unwrap();
        assert_eq!(turn.chunks[0].state(), "streaming");
        assert_eq!(turn.chunks[1].state(), "pending");
        assert_eq!(turn.chunks[2].state(), "buffering");
        turn.complete_chunks().unwrap();
        assert_eq!(turn.chunks[1].state(), "completed");
        assert_eq!(turn.chunks[2].state(), "completed");
        assert!(turn.chunks.iter().all(Chunk::is_terminal));
    }

    #[test]
    fn error_from_any_state_fails_the_turn() {
        let mut turn = turn();
        turn.dispatch(TurnEvent::Error("boom".to_owned())).unwrap();
        assert_eq!(turn.state(), TurnState::Failed.as_str());
        assert!(turn.is_terminal());
        assert_eq!(turn.outcome().error.as_deref(), Some("boom"));
    }
}
