# MetaMind Conversation Engine — Unified Design Plan (State Machine Architecture)

**Status**: Design proposal for the next major version of the mm-serve conversation backend
**Scope**: Merge existing `mm-serve` implementation with all 17 conversation designs in `design/conversation/` around a **core state machine engine**
**Target Crate**: `crates/mm-cme` (new), `crates/mm-serve` (thin HTTP wrapper)

---

## Executive Summary

This design unifies:
- **Existing**: `mm-serve` controller (turn-based, tool-calling, chat sessions, personality, affect)
- **New**: 17 conversation designs covering proactive behavior, voice (3 tiers), deterministic turn planning, speculative execution, compression, preference learning, multi-LLM routing, plugin system, prosody, synthetic testing, cross-session memory, hill-climbing, report generation, dashboard construction, self-improvement loops

**Core architectural principle**: Everything is a **state machine**. The Conversation Management Engine (CME) is a **hierarchical state machine** where:

- **Session** = top-level state machine
- **Turn** = child state machine (spawned per turn)
- **Chunk** = leaf state machine (streaming emission unit)
- **Proactive**, **Voice**, **Memory**, **Preferences**, **Routing**, **Plugins**, **Improvement** = orthogonal state machines that compose via **state machine composition** (parallel, hierarchical, history)

The LLM fills content slots in a TurnPlan; the state machine engine decides *what* states exist, *when* transitions fire, *how* they emit, and *what* orthogonal regions run in parallel.

---

## I. Core State Machine Engine

### I.1 Why State Machines?

| Problem | State Machine Solution |
|---------|------------------------|
| Turn planning + execution + interruption | Hierarchical states: `Turn.Planning → Turn.Executing → Turn.Streaming → Turn.Completed` with `Interruption` history state |
| Proactive items running parallel to user turns | Orthogonal regions: `Session.Proactive` region runs independent of `Session.Turn` region |
| Voice tiers with failover | State machine with `TierNegotiation → ActiveTier → Failover → DegradedTier` |
| Memory compression tiers | State machine: `Episodic → Compressing → Segment → Project → User` with timed transitions |
| Preference learning convergence | State machine: `ColdStart → Observing → Converging → Stable` per dimension |
| Plugin lifecycle | State machine: `Loaded → Validated → Registered → Running → Suspended → Unloaded` |
| Self-improvement loop | State machine: `Reflect → Plan → Act → Verify → Learn` with budget guards |

### I.2 State Machine Engine API (Core)

```rust
// mm-cme/src/state_machine/mod.rs

/// Unique identifier for a state machine instance
pub type SmId = Ulid;

/// Event that can trigger transitions
pub trait SmEvent: Send + Sync + 'static {
    fn event_type(&self) -> &'static str;
}

/// State in a state machine
pub trait SmState: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn is_final(&self) -> bool { false }
    fn is_history(&self) -> bool { false }
}

/// Action executed on transition (entry/exit/transition)
pub trait SmAction<Ctx>: Send + Sync {
    fn execute(&self, ctx: &mut Ctx, event: &dyn SmEvent) -> Pin<Box<dyn Future<Output = Result<(), SmError>> + Send>>;
}

/// Guard condition for transition
pub trait SmGuard<Ctx>: Send + Sync {
    fn check(&self, ctx: &Ctx, event: &dyn SmEvent) -> bool;
}

/// Transition definition
pub struct Transition<S, E, Ctx> {
    pub from: S,
    pub to: S,
    pub event: E,
    pub guard: Option<Box<dyn SmGuard<Ctx>>>,
    pub action: Option<Box<dyn SmAction<Ctx>>>,
}

/// State machine definition (compile-time checked via typestate)
pub struct StateMachine<S, E, Ctx> {
    pub initial: S,
    pub states: Vec<S>,
    pub transitions: Vec<Transition<S, E, Ctx>>,
    pub orthogonal_regions: Vec<StateMachine<S, E, Ctx>>,  // parallel regions
    pub history_states: Vec<S>,  // deep/shallow history
}

/// Runtime instance of a state machine
pub struct SmInstance<S, E, Ctx> {
    pub id: SmId,
    pub definition: Arc<StateMachine<S, E, Ctx>>,
    pub current_state: S,
    pub history: Vec<(S, Instant)>,  // for history pseudostates
    pub context: Ctx,
    pub parent: Option<SmId>,        // for hierarchical composition
    pub children: Vec<SmId>,         // spawned child machines
    pub orthogonal_current: HashMap<String, S>,  // per region
}

impl<S, E, Ctx> SmInstance<S, E, Ctx> {
    /// Process an event, return list of emitted events (for parent/observers)
    pub async fn dispatch(&mut self, event: E) -> Result<Vec<Box<dyn SmEvent>>, SmError> { ... }
    
    /// Spawn a child state machine (hierarchical)
    pub fn spawn_child<CS, CE, CCtx>(&mut self, def: StateMachine<CS, CE, CCtx>) -> SmId { ... }
    
    /// Enter orthogonal region
    pub fn enter_region(&mut self, region_name: &str, initial_state: S) { ... }
    
    /// Get current state (including orthogonal regions)
    pub fn snapshot(&self) -> SmSnapshot<S> { ... }
}

/// Snapshot for observability
pub struct SmSnapshot<S> {
    pub id: SmId,
    pub current: S,
    pub orthogonal: HashMap<String, S>,
    pub history_depth: usize,
    pub timestamp: Instant,
}
```

### I.3 Composition Patterns

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         SESSION STATE MACHINE                               │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                    TURN REGION (hierarchical)                       │   │
│  │  Idle → Planning → Executing → Streaming → Completed                │   │
│  │                    ↓                    ↓                            │   │
│  │               [Interruption]*      [Chunk]* (parallel chunks)       │   │
│  │                    ↓                    ↓                            │   │
│  │               Interrupted          Finalizing                       │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                  PROACTIVE REGION (orthogonal)                      │   │
│  │  Idle → Evaluating → Selecting → Executing → Injecting → Idle       │   │
│  │                    ↓                    ↓                            │   │
│  │               Suppressed            Completed                       │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                    VOICE REGION (orthogonal)                        │   │
│  │  Negotiating → Tier1/2/3 → Streaming → BargedIn? → Recovering → ... │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                   MEMORY REGION (orthogonal)                        │   │
│  │  Episodic → Compressing → Segmented → ProjectAggregated → UserSync  │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                 PREFERENCE REGION (orthogonal, per dimension)       │   │
│  │  ColdStart → Observing → Converging → Stable → DriftDetected → ...  │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## II. Core Data Model (State Machine Centric)

### II.1 Session = Top-Level State Machine Instance

```rust
// mm-cme/src/session.rs

/// Session is a state machine instance with orthogonal regions
pub struct Session {
    pub sm: SmInstance<SessionState, SessionEvent, SessionContext>,
    
    // Persistent identity
    pub id: SessionId,
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
    pub created_at: i64,
    
    // Orthogonal region handles (accessed via sm.orthogonal_current)
    pub turn_region: Option<SmId>,
    pub proactive_region: Option<SmId>,
    pub voice_region: Option<SmId>,
    pub memory_region: Option<SmId>,
    pub preference_region: Option<SmId>,
    
    // Cross-region shared context
    pub context: SessionContext,
}

/// Session states (top-level)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SessionState {
    Initializing,
    Active,
    Paused,           // user away, voice idle
    Suspended,        // explicit pause
    Resuming,         // restoring from memory
    Terminating,
    Terminated,
}

/// Events that drive session lifecycle
#[derive(Debug, Clone)]
pub enum SessionEvent {
    UserInput(UserInputEvent),
    VoiceAudio(AudioChunk),
    TurnCompleted(TurnOutcome),
    ProactiveTrigger(ProactiveTrigger),
    MemoryPressure(MemoryPressure),
    PreferenceUpdate(PreferenceDimension, f32),
    VoiceTierChange(VoiceTier),
    ShutdownRequest,
    ResumeRequest(ResumeContext),
}

/// Shared context across all regions
pub struct SessionContext {
    pub memory: MemoryContext,
    pub preferences: PreferenceProfile,
    pub affect: AffectState,
    pub budgets: SessionBudgets,
    pub config: SessionConfig,
}
```

### II.2 Turn = Child State Machine (Spawned per Turn)

```rust
// mm-cme/src/turn.rs

/// Turn state machine - spawned by Session for each user input or proactive item
pub struct Turn {
    pub sm: SmInstance<TurnState, TurnEvent, TurnContext>,
    pub plan: TurnPlan,           // deterministic plan (immutable after Planning)
    pub chunks: Vec<ChunkSm>,     // child chunk state machines
    pub turn_id: TurnId,
    pub parent_session: SmId,
}

/// Turn states
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TurnState {
    Planning,          // building TurnPlan (deterministic, no LLM yet)
    Routing,           // selecting model via router
    SpeculativeWarm,   // pre-warming KV cache from partials (Tier 2/3)
    Executing,         // LLM call in progress
    Streaming,         // emitting chunks (parallel chunk SMs)
    ToolCalling,       // executing tool calls (sub-state)
    WaitingConfirm,    // paused on confirmation gate
    Finalizing,        // honesty check, compression, memory update
    Completed,
    Interrupted,       // history state for barge-in/cancel
    Failed(ErrorKind),
}

/// Turn events
#[derive(Debug, Clone)]
pub enum TurnEvent {
    PlanReady(TurnPlan),
    RouteSelected(RoutingDecision),
    SpeculativeReady,
    LlmChunk(LlmStreamChunk),
    ToolResult(ToolResult),
    ConfirmationAnswer(ConfirmAnswer),
    ChunkCompleted(ChunkId),
    AllChunksDone,
    BargeInDetected(BargeInInfo),
    CancelRequested,
    Error(ErrorKind),
}

/// Turn context (mutable during execution)
pub struct TurnContext {
    pub transcript: Transcript,
    pub routing: RoutingRequest,
    pub budget: TurnBudget,
    pub emitted_chunks: Vec<EmittedChunk>,
    pub tool_results: Vec<ToolResult>,
    pub speculative_hint: Option<SpeculativeHint>,
    pub voice_config: Option<VoiceTurnConfig>,
}
```

### II.3 Chunk = Leaf State Machine (Streaming Emission Unit)

```rust
// mm-cme/src/chunk.rs

/// Chunk state machine - one per ChunkDirective in TurnPlan
pub struct ChunkSm {
    pub sm: SmInstance<ChunkState, ChunkEvent, ChunkContext>,
    pub directive: ChunkDirective,
    pub chunk_id: ChunkId,
    pub parent_turn: SmId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChunkState {
    Pending,           // waiting for depends_on
    Streaming,         // actively emitting (text/audio/structured)
    Buffering,         // holding until complete (StructuredCard)
    ToolExecuting,     // tool call in progress
    ToolWaitingConfirm,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum ChunkEvent {
    DependencyMet,
    LlmToken(String),
    LlmComplete,
    ToolResult(ToolResult),
    ConfirmationAnswer,
    FlushBuffer,
    Cancel,
    Error(String),
}

pub struct ChunkContext {
    pub buffer: String,
    pub stream_sender: Option<mpsc::Sender<ChatFrame>>,
    pub tool_calls: Vec<ToolCall>,
}
```

---

## III. Orthogonal Regions (Subsystems as State Machines)

### III.1 Proactive Region

```rust
// mm-cme/src/proactive/region.rs

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProactiveState {
    Idle,
    Evaluating,        // scoring candidates from queue + plugins + goals
    Selecting,         // diversity-aware top-N selection
    Planning,          // building mini TurnPlan per item
    Executing,         // spawning child Turn SMs
    Injecting,         // merging frames into session stream
    Suppressed,        // budget exhausted or user busy
    Completed,
}

#[derive(Debug, Clone)]
pub enum ProactiveEvent {
    Tick,
    BudgetAvailable,
    CandidateScored(Vec<ScoredProposal>),
    SelectionMade(Vec<ProactiveItem>),
    TurnSpawned(SmId),
    TurnCompleted(TurnOutcome),
    FramesInjected,
    BudgetExhausted,
    UserActivityDetected,  // suppress if user typing
}

pub struct ProactiveContext {
    pub queue: ProactiveQueue,
    pub plugins: Vec<Box<dyn ProactivePlugin>>,
    pub budget: ProactiveBudget,
    pub config: ProactiveConfig,
    pub last_user_activity: Instant,
}
```

### III.2 Voice Region

```rust
// mm-cme/src/voice/region.rs

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VoiceState {
    Negotiating,       // tier negotiation with provider caps
    Tier1_ReqResp,     // ASR → LLM → TTS sequential
    Tier2_Streaming,   // ASR stream → LLM stream → TTS stream (overlap)
    Tier3_FullDuplex,  // Realtime API / WebRTC
    BargedIn,          // interruption detected
    Recovering,        // cancelling old, starting new
    Degraded,          // fallback tier active
    Idle,
}

#[derive(Debug, Clone)]
pub enum VoiceEvent {
    AudioChunk(AudioChunk),
    AsrPartial(AsrPartial),
    AsrFinal(AsrFinal),
    TierNegotiated(VoiceTier),
    LlmStreamStart,
    LlmToken(String),
    LlmToolCall(ToolCall),
    TtsChunk(AudioChunk),
    BargeInDetected(BargeInInfo),
    TierFailover(VoiceTier),
    SessionPause,
    SessionResume,
}

pub struct VoiceContext {
    pub tier: VoiceTier,
    pub session: VoiceSession,
    pub asr: AsrHandle,
    pub tts: TtsHandle,
    pub llm_stream: Option<LlmStreamHandle>,
    pub realtime: Option<RealtimeHandle>,
    pub prosody: ProsodyController,
    pub barge_in: BargeInDetector,
}
```

### III.3 Memory Region (Unified — see `unified_memory_system.md`)

The Memory Region SM drives the **unified 4-tier memory hierarchy** defined in `unified_memory_system.md`. This replaces the separate designs in `cross_session_memory.md` and `conversation_compression.md`.

```rust
// mm-cme/src/memory/region.rs
// DELEGATES TO: mm-data::MemorySystem (unified implementation)

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MemoryState {
    Episodic,           // Tier 1: raw turns buffered in Agentstream + SQLite
    Compressing,        // Tier 1→2: UnifiedCompressor running (LLM extraction + narrative + embed)
    Segmented,          // Tier 2: CompressedSegment stored in RDF + vector index
    ProjectAggregating, // Tier 2→3: ProjectAggregator merging into ProjectMemory
    ProjectSynced,      // Tier 3: ProjectMemory updated in cubecast project layer
    UserSyncing,        // Tier 3→4: CrossProjectLearner updating UserMemory
    UserSynced,         // Tier 4: UserMemory updated in cubecast user layer
    Purging,            // Retention cleanup across all tiers
}

#[derive(Debug, Clone)]
pub enum MemoryEvent {
    TurnCompleted(TurnOutcome),           // → buffer in episodic
    CompressionTriggered,                 // → Compressing
    CompressionDone(Vec<CompressedSegment>), // → Segmented
    ProjectAggregationTriggered,          // → ProjectAggregating
    ProjectAggregationDone(ProjectMemory),   // → ProjectSynced
    UserSyncTriggered,                    // → UserSyncing
    UserSyncDone(UserMemory),             // → UserSynced
    RetentionTick,                        // → Purging
    PurgeCompleted,
}

/// Thin context — real state lives in mm-data::MemorySystem
pub struct MemoryContext {
    pub memory_system: Arc<MemorySystem>,  // delegate to mm-data
    pub config: MemoryConfig,
}
```

**Key Unification Points:**
- **Single compression pipeline**: `UnifiedCompressor` in `mm-data` handles LLM extraction, narrative, embeddings, quality validation
- **RDF storage per tier**: Episodic→Agentstream graphs, Segments→main store+vector index, Projects→cubecast project layer, Users→cubecast user layer
- **Session continuity**: `SessionContinuityManager` provides resume context with project/user memory
- **Multi-device sync**: `MultiDeviceSync` handles presence, broadcast, voice↔text handoff
- **Cross-project learning**: `CrossProjectLearner` transfers expertise/conventions to user memory

**Configuration**: Single `memory.toml` (see `unified_memory_system.md` §VII) controls all tiers.

### III.4 Preference Region (Per Dimension)

```rust
// mm-cme/src/preferences/region.rs

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PreferenceState {
    ColdStart,          // no observations
    Observing,          // collecting signals
    Converging,         // variance decreasing
    Stable,             // variance < threshold, applied to planner
    DriftDetected,      // new signals contradict posterior
    Relearning,         // adapting to drift
}

#[derive(Debug, Clone)]
pub enum PreferenceEvent {
    SignalObserved(PreferenceDimension, f32, f32),  // dimension, signal, weight
    PosteriorUpdated(PreferencePosterior),
    ConvergenceReached,
    DriftDetected(PreferenceDimension),
    AppliedToPlanner(PlannerConfig),
    ResetRequested,
}

pub struct PreferenceContext {
    pub dimensions: HashMap<PreferenceDimension, PreferencePosterior>,
    pub config: PreferenceConfig,
    pub planner_hooks: PlannerHooks,
}
```

### III.5 Plugin Region

```rust
// mm-cme/src/plugins/region.rs

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PluginState {
    Loading,            // WASM/.so load
    Validating,         // schema + capability check
    Registered,         // triggers registered
    Running,            // evaluating proposals
    Suspended,          // budget/resource pressure
    Updating,           // hot reload
    Unloading,
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum PluginEvent {
    LoadRequest(PluginId, PluginSource),
    ValidationResult(Result<PluginManifest, ValidationError>),
    TriggerFired(ProactiveTrigger, PluginContext),
    ProposalReceived(Vec<ProactiveProposal>),
    ExecutionComplete(ProactiveResult),
    BudgetPressure,
    ReloadRequest(PluginId),
    UnloadRequest(PluginId),
}

pub struct PluginContext {
    pub plugins: HashMap<PluginId, LoadedPlugin>,
    pub trigger_registry: TriggerRegistry,
    pub sandbox: Sandbox,
    pub budget: PluginBudget,
}
```

---

## IV. Runtime: State Machine Engine Driver

### IV.1 Session Driver (Main Loop)

```rust
// mm-cme/src/driver.rs

pub struct SessionDriver {
    pub session: Session,
    pub event_bus: EventBus,
    pub tick_interval: Duration,
}

impl SessionDriver {
    pub async fn run(mut self) -> Result<(), DriverError> {
        // Start all orthogonal regions
        self.session.sm.enter_region("turn", TurnState::Idle);
        self.session.sm.enter_region("proactive", ProactiveState::Idle);
        self.session.sm.enter_region("voice", VoiceState::Negotiating);
        self.session.sm.enter_region("memory", MemoryState::Episodic);
        self.session.sm.enter_region("preferences", PreferenceState::ColdStart);
        self.session.sm.enter_region("plugins", PluginState::Loading);
        
        // Load plugins
        self.dispatch(SessionEvent::PluginLoadAll).await?;

        let mut ticker = interval(self.tick_interval);
        
        loop {
            tokio::select! {
                // 1. External events (user input, voice audio, HTTP)
                event = self.event_bus.recv() => {
                    self.dispatch(event).await?;
                }
                
                // 2. Internal tick (proactive, memory, preferences)
                _ = ticker.tick() => {
                    self.dispatch(SessionEvent::Tick).await?;
                }
                
                // 3. Child turn completion
                outcome = self.wait_for_turn_completion() => {
                    self.dispatch(SessionEvent::TurnCompleted(outcome)).await?;
                }
                
                // 4. Shutdown signal
                _ = shutdown_signal() => {
                    self.dispatch(SessionEvent::ShutdownRequest).await?;
                    break;
                }
            }
        }
        
        // Graceful termination: wait for regions to reach final states
        self.terminate_gracefully().await
    }
    
    async fn dispatch(&mut self, event: SessionEvent) -> Result<(), DriverError> {
        // Dispatch to session SM
        let emitted = self.session.sm.dispatch(event.clone()).await?;
        
        // Dispatch to orthogonal regions (they handle relevant events)
        for region_name in ["turn", "proactive", "voice", "memory", "preferences", "plugins"] {
            if let Some(region_sm) = self.session.get_region(region_name) {
                region_sm.dispatch(event.clone()).await?;
            }
        }
        
        // Handle emitted events (parent notifications, observability)
        for e in emitted {
            self.handle_emitted(e).await?;
        }
        
        Ok(())
    }
}
```

### IV.2 Turn Driver (Spawned by Session)

```rust
pub struct TurnDriver {
    pub turn: Turn,
    pub session_tx: mpsc::Sender<SessionEvent>,
}

impl TurnDriver {
    pub async fn run(mut self) -> Result<TurnOutcome, DriverError> {
        // 1. PLANNING: Build TurnPlan (deterministic, no LLM)
        self.turn.sm.dispatch(TurnEvent::PlanReady(self.build_plan())).await?;
        
        // 2. ROUTING: Select model
        let routing = self.build_routing_request();
        let decision = self.llm_router.choose(&routing).await?;
        self.turn.sm.dispatch(TurnEvent::RouteSelected(decision)).await?;
        
        // 3. SPECULATIVE: Pre-warm if voice Tier 2/3
        if self.turn.context.voice_config.as_ref().map(|v| v.speculative).unwrap_or(false) {
            self.llm_service.speculative_prewarm(&self.turn.context.speculative_hint).await?;
            self.turn.sm.dispatch(TurnEvent::SpeculativeReady).await?;
        }
        
        // 4. EXECUTING: Call LLM, spawn chunk SMs
        let llm_stream = self.llm_service.call_streaming(self.build_llm_request()).await?;
        self.turn.sm.dispatch(TurnEvent::LlmStreamStart).await?;
        
        // Spawn chunk SMs for each ChunkDirective
        for chunk_directive in &self.turn.plan.chunks {
            let chunk_sm = ChunkSm::new(chunk_directive.clone(), self.turn.turn_id);
            let chunk_id = self.turn.sm.spawn_child(chunk_sm);
            self.turn.chunks.push(chunk_sm);
        }
        
        // 5. STREAMING: Drive chunk SMs from LLM stream
        while let Some(chunk) = llm_stream.next().await {
            for chunk_sm in &mut self.turn.chunks {
                chunk_sm.dispatch(ChunkEvent::LlmToken(chunk.text.clone())).await?;
            }
            
            // Check for tool calls
            if let Some(tool_calls) = chunk.tool_calls {
                self.turn.sm.dispatch(TurnEvent::ToolCalling).await?;
                for tc in tool_calls {
                    let result = self.execute_tool(tc).await?;
                    self.turn.context.tool_results.push(result.clone());
                    // Find relevant chunk SM and dispatch
                    self.dispatch_to_tool_chunk(tc.id, result).await?;
                }
            }
            
            // Check for barge-in
            if self.voice_context.barge_in_detected() {
                self.turn.sm.dispatch(TurnEvent::BargeInDetected(...)).await?;
                break;
            }
        }
        
        // 6. FINALIZING
        self.turn.sm.dispatch(TurnEvent::AllChunksDone).await?;
        let outcome = self.finalize().await?;
        
        // Notify session
        self.session_tx.send(SessionEvent::TurnCompleted(outcome.clone())).await?;
        
        Ok(outcome)
    }
}
```

---

## V. Subsystems as State Machine Transitions

### V.1 Turn Planning (in Turn.Planning state)

```rust
// mm-cme/src/planner.rs

pub struct TurnPlanner;

impl TurnPlanner {
    pub fn build_plan(&self, ctx: &TurnContext, session: &SessionContext) -> TurnPlan {
        // 1. Determine modality from preferences + context
        let modality = session.preferences.choose_modality(ctx);
        let detail_level = session.preferences.choose_detail_level(ctx);
        
        // 2. Build chunk directives (deterministic structure)
        let chunks = self.plan_chunks(ctx, modality, detail_level);
        
        // 3. Tool policy from session config + proactive queue
        let tool_policy = self.plan_tool_policy(session);
        
        // 4. Budget allocation
        let budget = self.allocate_budget(session.budgets, chunks.len());
        
        // 5. Voice config if applicable
        let voice_config = session.voice_region.as_ref().map(|v| v.build_turn_config());
        
        TurnPlan {
            turn_id: ctx.turn_id,
            session_id: ctx.session_id,
            user_query: ctx.user_query.clone(),
            chunks,
            tool_policy,
            modality,
            detail_level,
            voice_config,
            speculative_hint: ctx.speculative_hint.clone(),
            budget,
            metadata: TurnMetadata {
                parent_turn_id: ctx.parent_turn_id,
                is_proactive: ctx.is_proactive,
                trigger: ctx.trigger.clone(),
            },
        }
    }
    
    fn plan_chunks(&self, ctx: &TurnContext, modality: ResponseModality, detail: u8) -> Vec<ChunkDirective> {
        match modality {
            ResponseModality::DrillDown => vec![
                ChunkDirective::text("overview", detail),
                ChunkDirective::structured("details", DetailLevel::Deep),
                ChunkDirective::structured("sources", SourceList),
            ],
            ResponseModality::Explain => vec![
                ChunkDirective::text("concept", detail),
                ChunkDirective::structured("examples", DetailLevel::Medium),
                ChunkDirective::text("analogy", detail),
            ],
            ResponseModality::ZoomOut => vec![
                ChunkDirective::structured("summary", DetailLevel::High),
                ChunkDirective::structured("implications", DetailLevel::Medium),
            ],
            ResponseModality::Summarize => vec![
                ChunkDirective::structured("bullets", DetailLevel::Low),
            ],
            ResponseModality::Bullets => vec![
                ChunkDirective::bullets(detail, ctx.max_bullets, ctx.heading_style, ctx.sources_per_bullet),
            ],
        }
    }
}
```

### V.2 Multi-LLM Routing (in Turn.Routing state)

```rust
// mm-cme/src/routing/region.rs

pub struct ConversationRouter {
    inner: Arc<dyn Router>,  // wraps mm-llm::routing::ThresholdRouter
}

impl ConversationRouter {
    pub async fn choose(&self, turn: &TurnContext, session: &SessionContext) -> RoutingDecision {
        let req = RoutingRequest {
            purpose: turn.plan.purpose,
            complexity: self.estimate_complexity(&turn.context.transcript),
            stakes: if turn.plan.metadata.is_proactive { Stakes::Low } else { Stakes::Medium },
            required_precision: match turn.plan.modality {
                ResponseModality::DrillDown => Precision::High,
                _ => Precision::Normal,
            },
            latency_budget_ms: turn.plan.voice_config.as_ref()
                .map(|v| v.tier.latency_budget())
                .unwrap_or(0),
            cost_budget_micros: turn.plan.budget.cost_micros,
            pinned_model: None,
            // Conversation-aware fields
            modality: Some(turn.plan.modality),
            voice_tier: turn.plan.voice_config.as_ref().map(|v| v.tier),
            is_proactive: turn.plan.metadata.is_proactive,
            session_context: SessionContext::from(&turn.context.transcript),
            preference_profile: session.preferences.clone(),
        };
        self.inner.choose(&req).await
    }
}
```

### V.3 Voice Prosody (in Voice region states)

```rust
// mm-cme/src/voice/prosody.rs

impl VoiceRegion {
    fn compute_prosody(&self, ctx: &VoiceContext) -> ProsodyParams {
        let mut params = ctx.prosody.baseline;
        
        // Affect modulation (from mm-being)
        params.rate += 0.15 * ctx.prosody.affect.arousal;
        params.pitch += 0.1 * ctx.prosody.affect.valence;
        params.volume += 0.1 * ctx.prosody.affect.energy;
        params.warmth += 0.2 * ctx.prosody.affect.warmth;
        params.pause_factor += 0.3 * ctx.prosody.affect.caution;
        
        // Modality modulation (from turn plan)
        if let Some(turn_modality) = ctx.current_turn_modality {
            match turn_modality {
                ResponseModality::DrillDown => { params.rate *= 0.9; params.detail += 0.2; }
                ResponseModality::Summarize => { params.rate *= 1.1; params.detail -= 0.1; }
                ResponseModality::Explain => { params.pause_factor *= 1.2; }
                _ => {}
            }
        }
        
        params.clamp()
    }
}
```

### V.4 Preference Learning (in Preference region)

```rust
// mm-cme/src/preferences/learner.rs

impl PreferenceRegion {
    fn observe_signal(&mut self, dimension: PreferenceDimension, signal: f32, weight: f32) {
        let posterior = self.context.dimensions.entry(dimension).or_default();
        
        // Bayesian update
        let prior_prec = 1.0 / posterior.variance;
        let signal_prec = weight / SIGNAL_VARIANCE;
        posterior.mean = (posterior.mean * prior_prec + signal * signal_prec) / (prior_prec + signal_prec);
        posterior.variance = 1.0 / (prior_prec + signal_prec);
        posterior.n_observations += 1;
        
        // Check convergence
        if posterior.variance < self.config.convergence_threshold && posterior.n_observations >= self.config.min_observations {
            self.dispatch(PreferenceEvent::ConvergenceReached(dimension)).await?;
        }
        
        // Check drift (new signal far from posterior)
        if (signal - posterior.mean).abs() > 3.0 * posterior.variance.sqrt() {
            self.dispatch(PreferenceEvent::DriftDetected(dimension)).await?;
        }
        
        // Apply to planner hooks
        self.context.planner_hooks.update(dimension, posterior.mean);
    }
}
```

### V.5 Self-Improvement Loop (Orthogonal Region)

```rust
// mm-cme/src/improvement/region.rs

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImprovementState {
    Idle,
    Reflecting,      // critics analyze artifacts
    Planning,        // generate improvement proposals
    Acting,          // mutate artifacts (hill-climb)
    Verifying,       // run verifiers (tests, LLM judge, human)
    Learning,        // extract patterns, update rules
    RollingBack,     // verification failed
    Completed,
}

#[derive(Debug, Clone)]
pub enum ImprovementEvent {
    Trigger(ImprovementTrigger),  // Scheduled | OnChange | OnRegression | OnFeedback
    ReflectionDone(Vec<Issue>),
    PlanReady(Vec<ImprovementProposal>),
    MutationApplied(HillClimbStep),
    VerificationResult(VerificationResult),
    PatternLearned(LearnedPattern),
    RollbackTriggered,
    BudgetExhausted,
}

pub struct ImprovementContext {
    pub critics: Vec<Box<dyn Critic>>,
    pub verifiers: Vec<Box<dyn Verifier>>,
    pub hill_climber: HillClimbingEngine,
    pub learning_store: LearningStore,
    pub budgets: ImprovementBudgets,
    pub governance: GovernanceConfig,
}
```

---

## VI. Observability as State Machine Introspection

### VI.1 State Snapshots (REST)

```
GET /api/cme/sessions/{id}/snapshot
→ {
    session: { sm_id, current_state, orthogonal_states, history_depth },
    turn: { sm_id, current_state, plan, chunks: [{id, state, progress}] },
    proactive: { sm_id, current_state, queue_length, budget_remaining },
    voice: { sm_id, current_state, tier, barge_in_count },
    memory: { sm_id, current_state, episodic_count, segment_count },
    preferences: { sm_id, states: {dimension: {state, mean, variance}} },
    plugins: { sm_id, states: {plugin_id: state} },
    improvement: { sm_id, current_state, last_run, patterns_learned },
    timestamp: i64
}
```

### VI.2 Real-time Event Stream (WebSocket)

```
WS /api/cme/sessions/{id}/events
→ Events with SM context:
  { type: "SMTransition", sm_id, sm_type: "Turn", from: "Streaming", to: "Interrupted", event: "BargeInDetected" }
  { type: "SMTransition", sm_id, sm_type: "Proactive", from: "Idle", to: "Evaluating", event: "Tick" }
  { type: "SMTransition", sm_id, sm_type: "Voice", from: "Tier2_Streaming", to: "Tier1_ReqResp", event: "TierFailover" }
  { type: "SMTransition", sm_id, sm_type: "Memory", from: "Episodic", to: "Compressing", event: "CompressionTriggered" }
  { type: "SMTransition", sm_id, sm_type: "Preference.verbosity", from: "Converging", to: "Stable", event: "ConvergenceReached" }
```

### VI.3 Distributed Tracing (OpenTelemetry)

Each state machine instance = trace span; transitions = span events; orthogonal regions = parallel spans.

```
Trace: session.{id}
  Span: session (Session SM)
    Event: SessionState: Active
    Span: turn.{turn_id} (Turn SM - child)
      Event: TurnState: Planning
      Event: TurnState: Routing
      Event: TurnState: Executing
      Span: chunk.{chunk_id} (Chunk SM - child)
        Event: ChunkState: Streaming
        Event: ChunkState: Completed
    Span: proactive (Orthogonal region)
      Event: ProactiveState: Evaluating → Selecting → Executing
    Span: voice (Orthogonal region)
      Event: VoiceState: Tier2_Streaming → BargedIn → Recovering
    Span: memory (Orthogonal region)
      Event: MemoryState: Episodic → Compressing → Segmented
    Span: preferences (Orthogonal region, per dimension)
      Event: PreferenceState.verbosity: Observing → Converging → Stable
    Span: improvement (Orthogonal region)
      Event: ImprovementState: Reflecting → Planning → Acting → Verifying → Learning
```

### VI.4 Prometheus Metrics (State Machine Derived)

| Metric | Type | Labels |
|--------|------|--------|
| `cme_sm_transitions_total` | Counter | sm_type, from_state, to_state, event_type |
| `cme_sm_state_duration_ms` | Histogram | sm_type, state |
| `cme_sm_active_count` | Gauge | sm_type |
| `cme_turn_sm_duration_ms` | Histogram | outcome, modality |
| `cme_proactive_sm_cycle_ms` | Histogram | status |
| `cme_voice_sm_tier_changes` | Counter | from_tier, to_tier, reason |
| `cme_memory_sm_compression_ratio` | Gauge | tier |
| `cme_preference_sm_convergence` | Gauge | dimension, posterior_variance |
| `cme_improvement_sm_cycles` | Counter | trigger, outcome |

---

## VII. Crate Structure (State Machine Centric)

```
crates/
├── mm-sm/                      # NEW: Core State Machine Engine (no deps on metamind)
│   ├── src/
│   │   ├── machine.rs          # StateMachine, SmInstance, SmEvent, SmState, SmAction, SmGuard
│   │   ├── hierarchical.rs     # Parent/child spawning, history pseudostates
│   │   ├── orthogonal.rs       # Region management, parallel composition
│   │   ├── snapshot.rs         # SmSnapshot, serialization
│   │   ├── tracing.rs          # OpenTelemetry integration
│   │   └── lib.rs
│   └── Cargo.toml
│
├── mm-cme/                     # Conversation Management Engine
│   ├── src/
│   │   ├── session.rs          # Session SM + regions
│   │   ├── turn.rs             # Turn SM (child of Session)
│   │   ├── chunk.rs            # Chunk SM (child of Turn)
│   │   ├── planner.rs          # TurnPlanning logic (in Turn.Planning)
│   │   ├── executor.rs         # TurnExecutor (drives Turn SM)
│   │   ├── regions/
│   │   │   ├── proactive.rs    # ProactiveRegion SM
│   │   │   ├── voice.rs        # VoiceRegion SM
│   │   │   ├── memory.rs       # MemoryRegion SM
│   │   │   ├── preferences.rs  # PreferenceRegion SM (per dimension)
│   │   │   ├── plugins.rs      # PluginRegion SM
│   │   │   └── improvement.rs  # ImprovementRegion SM
│   │   ├── routing.rs          # ConversationRouter (used in Turn.Routing)
│   │   ├── observability.rs    # Snapshots, WS, OTel, Metrics
│   │   ├── driver.rs           # SessionDriver (main loop)
│   │   ├── report.rs           # ReportGenerator (uses hill_climbing SM)
│   │   ├── dashboard.rs        # DashboardConstructor (uses hill_climbing SM)
│   │   └── lib.rs
│   └── Cargo.toml              # deps: mm-sm, mm-llm, mm-being, mm-data
│
├── mm-serve/                   # HTTP server, routes, AppState
│   └── src/
│       ├── routes/             # Thin adapters → mm-cme
│       ├── state.rs            # AppState holds SessionDriver
│       └── main.rs
│
├── mm-llm/                     # LLM service, routing, clients (existing)
├── mm-being/                   # Personality, affect, goals (existing)
├── mm-data/                    # RDF layer (cubecast) (from rdf_backend_integration.md)
└── mm-core/                    # Config, IDs, errors (existing)
```

---

## VIII. Configuration (State Machine Centric)

```toml
# config/cme.toml

[cme]
# Session SM
max_session_age_days = 30
max_turns_per_session = 1000
tick_interval_ms = 100

# Turn SM
[turn]
default_modality = "bullets"
default_detail_level = 2
max_chunks_per_turn = 8
turn_token_budget = 8000
turn_time_budget_ms = 30000
planning_timeout_ms = 5000
routing_timeout_ms = 2000
speculative_timeout_ms = 1000

# Proactive Region SM
[proactive]
enabled = true
max_per_cycle = 2
cycle_interval_ms = 15000
budget_per_turn = 0.15
suppress_on_user_activity = true
suppress_threshold_ms = 500
plugins = ["search_lookup", "goal_narrowing", "plan_refinement"]

# Voice Region SM
[voice]
default_tier = "StreamingPipeline"
fallback_chain = ["FullDuplexNative", "StreamingPipeline", "RequestResponse"]
sample_rate = 24000
vad_sensitivity = 0.5
barge_in_enabled = true
barge_in_threshold = 0.3
tier_negotiation_timeout_ms = 1000

# Memory Region SM
[memory]
episodic_retention_days = 30
segment_retention_days = 90
compression_trigger_turns = 10
compression_model = "haiku"
project_aggregation_interval_hours = 6
user_sync_interval_hours = 24

# Preference Region SM (per dimension)
[preferences]
learning_rate = 0.1
min_observations_for_apply = 5
convergence_threshold = 0.02
drift_threshold_sigma = 3.0
dimensions = ["verbosity", "initiative", "technical_depth", "formality", "pace"]

# Plugin Region SM
[plugins]
sandbox = "wasmtime"  # wasmtime | native
max_cpu_ms_per_eval = 100
max_memory_mb = 64
network_allowed = false
hot_reload_enabled = true

# Improvement Region SM
[improvement]
enabled = true
budget_time_ms_per_hour = 300000
budget_tokens_per_hour = 500000
human_approval_required = ["report", "dashboard", "agent_cognition"]
critics = ["correctness", "clarity", "actionability", "completeness", "consistency", "tone", "efficiency", "safety"]
verifiers = ["unit_test", "lint", "schema", "llm_judge", "human_review", "simulation"]

# Routing
[routing]
enable_multi_llm = true
routing_table_path = "config/routing_table.toml"

# Observability
[observability]
snapshots_enabled = true
websocket_events_enabled = true
otel_enabled = true
prometheus_enabled = true
debug_ui_enabled = true
```

---

## IX. Migration Path (State Machine Refactor)

| Phase | Action | State Machine Focus |
|-------|--------|---------------------|
| 1 | Create `mm-sm` crate with core engine | Core SM primitives |
| 2 | Create `mm-cme` with `Session`, `Turn`, `Chunk` SMs | Hierarchical composition |
| 3 | Implement `TurnPlanner` as `Turn.Planning` state logic | Deterministic planning |
| 4 | Implement `ProactiveRegion` as orthogonal region | Parallel composition |
| 5 | Implement `VoiceRegion` with 3-tier SM | Failover transitions |
| 6 | Implement `MemoryRegion` with 4-tier SM | Timed transitions |
| 7 | Implement `PreferenceRegion` per dimension | Convergence guards |
| 8 | Implement `PluginRegion` lifecycle | Sandbox guards |
| 9 | Implement `ImprovementRegion` 5-phase loop | Budget guards |
| 10 | Add observability (snapshots, WS, OTel, metrics) | SM introspection |
| 11 | Integrate `mm-serve` routes → `SessionDriver` | Driver pattern |
| 12 | Migrate existing `mm-serve` sessions | History state resume |
| 13 | Synthetic testing + load testing | SM coverage |
| 14 | Canary deploy | Feature flags per region |

---

## X. Testing Strategy (State Machine Based)

### X.1 Unit Tests (Per SM)

```rust
// Test Turn SM transitions
#[test]
fn turn_sm_planning_to_routing_on_plan_ready() {
    let mut turn = TurnSm::new(TurnState::Planning);
    turn.dispatch(TurnEvent::PlanReady(plan)).await;
    assert_eq!(turn.current_state, TurnState::Routing);
}

#[test]
fn turn_sm_barge_in_goes_to_interrupted_history() {
    let mut turn = TurnSm::new(TurnState::Streaming);
    turn.dispatch(TurnEvent::BargeInDetected(...)).await;
    assert_eq!(turn.current_state, TurnState::Interrupted);
    assert!(turn.history.contains(&TurnState::Streaming));
}

#[test]
fn turn_sm_resume_from_interrupted() {
    let mut turn = TurnSm::new_with_history(TurnState::Interrupted, vec![TurnState::Streaming]);
    turn.dispatch(TurnEvent::Resume).await;
    assert_eq!(turn.current_state, TurnState::Streaming);  // history restore
}
```

### X.2 Integration Tests (Cross-Region)

```rust
#[tokio::test]
async fn proactive_region_suppressed_during_user_typing() {
    let mut session = Session::new();
    session.dispatch(SessionEvent::UserInput(...)).await;
    
    // Proactive tick should go to Suppressed
    session.dispatch(SessionEvent::Tick).await;
    assert_eq!(session.proactive_region.current_state, ProactiveState::Suppressed);
    
    // After user idle, should return to Evaluating
    advance_time(Duration::from_secs(2));
    session.dispatch(SessionEvent::Tick).await;
    assert_eq!(session.proactive_region.current_state, ProactiveState::Evaluating);
}

#[tokio::test]
async fn voice_tier_failover_on_provider_error() {
    let mut session = Session::new();
    session.voice_region.current_state = VoiceState::Tier3_FullDuplex;
    session.dispatch(VoiceEvent::TierFailover(VoiceTier::StreamingPipeline)).await;
    assert_eq!(session.voice_region.current_state, VoiceState::Tier2_Streaming);
}
```

### X.3 Synthetic Testing (YAML DSL with SM Assertions)

```yaml
# tests/synthetic/conversation_scenarios.yaml
scenarios:
  - name: "voice_barge_in_recovery"
    persona: "interruptive_user"
    voice_tier: "StreamingPipeline"
    turns: 8
    sm_assertions:
      - turn_1: [Planning, Routing, Executing, Streaming, BargedIn, Recovering, Streaming, Completed]
      - proactive: [Idle, Suppressed, Idle, Evaluating, Executing]
      - voice: [Negotiating, Tier2_Streaming, BargedIn, Recovering, Tier2_Streaming]
    checkers: [barge_in_handling, prosody_appropriateness, turn_taking, sm_coverage]
  
  - name: "preference_convergence"
    persona: "concise_engineer"
    turns: 20
    sm_assertions:
      - preference.verbosity: [ColdStart, Observing, Converging, Stable]
      - preference.initiative: [ColdStart, Observing, Converging, Stable]
    checkers: [preference_accuracy, sm_convergence_timing]
```

---

## XI. Open Questions

| Question | Options | Recommendation |
|----------|---------|----------------|
| **SM engine: library vs embedded** | `mm-sm` as separate crate vs inline in `mm-cme` | Separate crate (reusable for agent loops, workflows) |
| **Async vs sync transitions** | All async (current) vs sync for simple guards | Async throughout; sync guards via `SmGuard` trait |
| **Persistence of SM state** | Event sourcing (replay) vs snapshots | Snapshots + event log for audit |
| **WASM plugins as SMs** | Each plugin = SM instance | Yes, `PluginRegion` spawns child SM per plugin |
| **Distributed SMs** | Single process vs multi-node | Single process first; CRDT for multi-node later |
| **SM visualization** | Mermaid/PlantUML export from definitions | Add `mm-sm` dev feature for diagram generation |

---

## XII. Appendix: Design Document Cross-Reference

| Design Doc | SM Region / State |
|------------|-------------------|
| `proactive_conversation.md` | `ProactiveRegion` SM (§III.1, §VIII) |
| `conversation_management_engine.md` | `Session`/`Turn`/`Chunk` SMs (§II, §III, §IV) |
| `turn_planning.md` | `Turn.Planning` state logic |
| `speculative_execution.md` | `Turn.SpeculativeWarm` state |
| `conversation_compression.md` | `MemoryRegion` SM (§III.3) |
| `preference_learning.md` | `PreferenceRegion` SM (§III.4) |
| `multi_llm_routing.md` | `Turn.Routing` state logic |
| `proactive_plugin_system.md` | `PluginRegion` SM (§III.5) |
| `voice_prosody.md` | `VoiceRegion` prosody computation |
| `synthetic_testing.md` | SM assertions in test DSL |
| `cross_session_memory.md` | `MemoryRegion` tier transitions |
| `hill_climbing.md` | `ImprovementRegion.Acting` state (hill-climb SM) |
| `report_generation.md` | Report as artifact mutated by hill-climb SM |
| `dashboard_construction.md` | Dashboard as artifact mutated by hill-climb SM |
| `self_improvement_loop.md` | `ImprovementRegion` 5-phase SM |
| `conversation_engine_improvements.md` | All consolidated into SM architecture |

---

*This design plan establishes the **core state machine engine** as the single architectural foundation. Every subsystem is a state machine or state machine region; composition is via hierarchical and orthogonal patterns. Review, approve, then begin Phase 1 (`mm-sm` crate).*