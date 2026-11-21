# MetaMind Conversation Engine — Unified Design Plan

**Status**: Design proposal for the next major version of the mm-serve conversation backend
**Scope**: Merge existing `mm-serve` implementation with all 17 conversation designs in `design/conversation/`
**Target Crate**: `crates/mm-serve` (replacing/refactoring the controller, chat, routes, state)

---

## Executive Summary

This design unifies:
- **Existing**: `mm-serve` controller (turn-based, tool-calling, chat sessions, personality, affect)
- **New**: 17 conversation designs covering proactive behavior, voice (3 tiers), deterministic turn planning, speculative execution, compression, preference learning, multi-LLM routing, plugin system, prosody, synthetic testing, cross-session memory, hill-climbing, report generation, dashboard construction, self-improvement loops

The result is a **Conversation Management Engine (CME)** that:
- Owns conversation *structure* (TurnPlan, chunks, modalities), not just content
- Supports voice at 3 tiers with failover
- Runs proactive loops independently of user input
- Learns preferences and improves itself continuously
- Generates hill-climbable reports and dashboards as first-class outputs

---

## I. Architectural Position

### Current (mm-serve)
```
UI → HTTP Routes → controller::reply() → LLM loop (tools → response) → SSE frames
```

### Target (CME)
```
UI (Text/Voice) → CME Session → TurnPlanner → LLM Router → LLM Service
                           ↓
                    ProactiveQueue (plugins, goals, affect)
                           ↓
                    Memory (4-tier: session → segment → project → user)
                           ↓
                    Observability (snapshots, traces, WebSocket, metrics)
```

**Key principle**: CME is the *cognitive controller*. The LLM fills content slots in a TurnPlan; CME decides *what* chunks exist, *when* they emit, *how* they render (modality), and *what* proactive work runs between turns.

---

## II. Core Data Model (Merge of Existing + New)

### II.1 Session (extends existing `chat::Session`)

```rust
pub struct Session {
    // Existing
    pub id: SessionId,
    pub title: String,
    pub created_at: i64,
    pub run_ids: Vec<RunId>,

    // New: cross-session memory
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
    pub memory_context: MemoryContext,      // compressed segments + project/user refs
    pub preference_profile: PreferenceProfile,  // learned from behavior

    // New: voice
    pub voice_tier: VoiceTier,              // RequestResponse | Streaming | FullDuplex
    pub voice_session: Option<VoiceSession>, // audio config, VAD, barge-in state

    // New: proactive
    pub proactive_config: ProactiveConfig,   // enabled plugins, pace, budgets
    pub proactive_queue: ProactiveQueue,     // pending proactive items
}
```

### II.2 Turn & TurnPlan (replaces ad-hoc loop in `controller::reply`)

```rust
/// Deterministic plan constructed BEFORE any LLM call
pub struct TurnPlan {
    pub turn_id: TurnId,
    pub session_id: SessionId,
    pub user_query: Option<String>,          // None for proactive turns
    pub chunks: Vec<ChunkDirective>,         // ordered emission plan
    pub tool_policy: ToolPolicy,             // which groups, caps, confirmations
    pub modality: ResponseModality,          // drill_down | explain | zoom_out | summarize | bullets
    pub detail_level: u8,                    // 1-3
    pub voice_config: Option<VoiceTurnConfig>, // tier, barge-in, prosody hints
    pub speculative_hint: Option<SpeculativeHint>, // partial transcript → pre-warm
    pub budget: TurnBudget,                  // token, time, cost, tool caps
    pub metadata: TurnMetadata,              // parent_turn_id, is_proactive, trigger
}

/// A single emission unit in the stream
pub struct ChunkDirective {
    pub chunk_id: ChunkId,
    pub kind: ChunkKind,                     // Text | StructuredCard | ToolCall | ToolResult | Focus | Status | InputRequest | ProactiveItem
    pub content: ChunkContent,               // text, card schema, tool spec, etc.
    pub streaming: StreamingPolicy,          // Stream | Buffer | HoldUntilComplete
    pub depends_on: Option<ChunkId>,         // for ordering
    pub modality_hint: Option<ResponseModality>, // override for this chunk
}
```

### II.3 ChatFrame (extends existing `dto::chat::ChatFrame`)

```rust
pub enum ChatFrame {
    // Existing
    Started { query: String },
    Run { request_id, session_id, query },
    LlmRequestStarted { turn_id, phase, subsystem, turn_number, visible, voice: VoiceParams },
    LlmResponse { turn_id, phase, text, visible, is_final },
    ToolCall { id, turn_id, phase, name, input },
    ToolResult { id, turn_id, name, success, output, duration_ms },
    Focus { doc: Value },
    FinalResponse { response_id, text, blocks, suggestions, unbacked },
    Completed { subsystems: Vec<String> },
    InputRequest { id, request: Value },
    InputAnswer { id, answer: Value, at: i64 },
    Status { doc_id, status, reason },
    Failed { error, reason },

    // New: modality-aware
    ModalityChunk { turn_id, chunk_id, modality: ResponseModality, content: ModalityContent },
    StructuredSection { turn_id, section_id, section: ReportSection },

    // New: proactive
    ProactiveItem { item_id, kind: ProactiveKind, payload: Value, status: ProactiveStatus },

    // New: voice
    VoiceChunk { turn_id, audio: AudioChunk, is_final: bool },
    BargeInDetected { turn_id, at_ms: u32 },

    // New: observability
    TurnPlanInspection { turn_id, plan: TurnPlan },
    StateSnapshot { session_id, snapshot: SessionSnapshot },
}
```

### II.4 ProactiveQueue (new)

```rust
pub struct ProactiveQueue {
    pub items: VecDeque<ProactiveItem>,
    pub running: Option<ProactiveItem>,
    pub history: Vec<ProactiveItem>,
    pub budget: ProactiveBudget,             // per-turn, per-session caps
}

pub struct ProactiveItem {
    pub id: ProactiveId,
    pub kind: ProactiveKind,                 // Search | Lookup | Joke | Suggestion | Advice | Criticism | Brainstorm | QANarrow | PlanRefine | Plugin
    pub trigger: ProactiveTrigger,           // Time | Event | Pattern | UserSignal | Scheduled
    pub payload: Value,                      // context for the action
    pub status: ProactiveStatus,             // Pending | Running | Completed | Failed | Suppressed
    pub result: Option<ProactiveResult>,     // frames to inject, side effects
    pub plugin_id: Option<PluginId>,         // if from plugin
}
```

### II.5 Voice Types (new — 3 tiers)

```rust
pub enum VoiceTier {
    RequestResponse,   // ASR → LLM → TTS (existing path, new audio I/O)
    StreamingPipeline, // ASR stream → LLM stream → TTS stream (overlap, speculative)
    FullDuplexNative,  // Realtime API / WebRTC (inline tools, barge-in native)
}

pub struct VoiceSession {
    pub tier: VoiceTier,
    pub config: AudioConfig,                 // sample_rate, channels, codec
    pub vad: VadConfig,                      // sensitivity, min_speech_ms, silence_ms
    pub tts: TtsConfig,                      // voice_id, prosody_baseline, ssml_template
    pub asr: AsrConfig,                      // language, partials, punctuation
    pub barge_in: BargeInConfig,             // enabled, threshold, action (interrupt/queue)
    pub state: VoiceSessionState,            // Idle | Listening | Processing | Speaking | BargedIn
}

pub struct VoiceTurnConfig {
    pub tier: VoiceTier,
    pub allow_barge_in: bool,
    pub prosody_hint: Option<ProsodyParams>, // from affect, context, modality
    pub speculative: bool,                   // pre-warm from partials
}
```

---

## III. Runtime Loop (Replaces `controller::reply`)

### III.1 Main Event Loop (per session)

```
┌─────────────────────────────────────────────────────────────────┐
│                    CME SESSION EVENT LOOP                       │
├─────────────────────────────────────────────────────────────────┤
│  while session.active {                                         │
│      select! {                                                  │
│          // 1. User input (text or voice)                       │
│          msg = rx_user.recv() => handle_user_input(msg),        │
│                                                                  │
│          // 2. Proactive tick (independent of user)             │
│          _ = proactive_ticker.tick() => run_proactive_cycle(),  │
│                                                                  │
│          // 3. Voice audio chunks (Tier 2/3)                    │
│          audio = rx_audio.recv() => handle_voice_audio(audio),  │
│                                                                  │
│          // 4. Tool confirmations (Approve/Reject/Defer)        │
│          answer = rx_confirm.recv() => resume_parked_turn(answer),│
│                                                                  │
│          // 5. Plugin events (async completions)                │
│          event = rx_plugins.recv() => handle_plugin_event(event),│
│                                                                  │
│          // 6. Memory compression trigger                       │
│          _ = compression_ticker.tick() => compress_if_needed(), │
│                                                                  │
│          // 7. Self-improvement trigger                         │
│          _ = improvement_ticker.tick() => run_improvement_loop(),│
│      }                                                            │
│  }                                                               │
└─────────────────────────────────────────────────────────────────┘
```

### III.2 Turn Execution (deterministic, replaces ad-hoc LLM loop)

```rust
async fn execute_turn(state: &CmeState, plan: TurnPlan) -> TurnOutcome {
    // 1. Build transcript with compression applied
    let transcript = build_transcript(state, &plan).await;

    // 2. Select model via router (multi-LLM routing)
    let routing = build_routing_request(&plan, &transcript);
    let decision = state.llm_router.choose(&routing)?;

    // 3. If speculative: pre-warm KV cache from partial transcript
    if let Some(hint) = &plan.speculative_hint {
        state.llm_service.speculative_prewarm(hint).await?;
    }

    // 4. Execute chunks in order (streaming where configured)
    for chunk in &plan.chunks {
        match chunk.kind {
            ChunkKind::Text => stream_text_chunk(state, &plan, chunk).await?,
            ChunkKind::StructuredCard => emit_structured_chunk(state, chunk).await?,
            ChunkKind::ToolCall => execute_tool_chunk(state, &plan, chunk).await?,
            ChunkKind::ProactiveItem => inject_proactive_chunk(state, chunk).await?,
            ChunkKind::VoiceChunk => stream_voice_chunk(state, chunk).await?,
            _ => {}
        }
        // Check for barge-in / cancellation between chunks
        if state.is_cancelled(&plan.turn_id) || state.is_barged_in(&plan.turn_id) {
            return handle_interruption(state, &plan).await;
        }
    }

    // 5. Finalize: compress, update memory, log observability
    finalize_turn(state, &plan).await
}
```

### III.3 Proactive Cycle (runs independent of user input)

```rust
async fn run_proactive_cycle(state: &CmeState) {
    // 1. Check budget
    if !state.proactive_budget.try_consume(1) { return; }

    // 2. Score candidates from queue + plugins + goals
    let candidates = gather_proactive_candidates(state).await;

    // 3. Select top-N (diversity-aware)
    let selected = select_proactive_items(candidates, state.proactive_config.max_per_cycle);

    // 4. For each: build mini TurnPlan, execute, inject frames
    for item in selected {
        let mini_plan = build_proactive_plan(state, &item).await;
        let outcome = execute_turn(state, mini_plan).await;
        state.inject_frames(outcome.frames).await;
        item.mark_completed(outcome);
    }

    // 5. Learn from outcomes (preference learning)
    state.preference_learner.observe_proactive_outcomes(&selected).await;
}
```

---

## IV. Voice Architecture (3 Tiers with Failover)

### IV.1 Tier 1: Request/Response (ASR → LLM → TTS)
- **Use case**: Simple voice commands, low latency not critical
- **Implementation**: Wrap existing `controller::reply` with audio I/O
- **Latency**: ~800-1500ms (ASR + LLM + TTS sequential)

### IV.2 Tier 2: Streaming Pipeline (ASR stream → LLM stream → TTS stream)
- **Use case**: Natural conversation, moderate latency
- **Key features**:
  - ASR partials → speculative LLM pre-warm (30-50% latency reduction)
  - LLM streaming → TTS chunking (sentence-boundary)
  - Overlap: TTS starts before LLM finishes
  - Barge-in: interrupt TTS, cancel LLM, new ASR
- **Implementation**: New `VoicePipeline` struct with buffered channels

### IV.3 Tier 3: Full-Duplex Native (Realtime API / WebRTC)
- **Use case**: Human-like conversation, lowest latency
- **Key features**:
  - Single WebSocket/WebRTC connection
  - Inline tool calling (model calls tools mid-stream)
  - Native barge-in (model hears interruption)
  - Prosody in → prosody out (affect-aware)
- **Implementation**: `RealtimeSession` wrapping provider realtime APIs

### IV.4 Tier Negotiation & Failover

```rust
async fn negotiate_tier(requested: VoiceTier, caps: &ProviderCaps) -> VoiceTier {
    match requested {
        VoiceTier::FullDuplexNative if caps.supports_realtime => VoiceTier::FullDuplexNative,
        VoiceTier::FullDuplexNative => negotiate_tier(VoiceTier::StreamingPipeline, caps),
        VoiceTier::StreamingPipeline if caps.supports_streaming => VoiceTier::StreamingPipeline,
        VoiceTier::StreamingPipeline => VoiceTier::RequestResponse,
        VoiceTier::RequestResponse => VoiceTier::RequestResponse,
    }
}
```

---

## V. Memory System (4-Tier Hierarchy)

### V.1 Tier Definitions

| Tier | Scope | Retention | Content |
|------|-------|-----------|---------|
| **Episodic** | Session | Session + 30d | Full frames, turn plans, audio, affect traces |
| **Segment** | Session | 90d | Compressed: narrative + artifacts (decisions, commitments, code_refs, open_questions) |
| **Project** | Project | Years | Aggregated: patterns, conventions, expertise, continuation suggestions |
| **User** | User | Years | Identity, preferences, communication style, cross-project learning |

### V.2 Compression Pipeline (runs async)

```
Raw Turns → Extract Artifacts → Generate Narrative → Embed → Store as Segment
                ↓
         Artifacts: decisions[], commitments[], code_refs[], open_questions[],
                     preferences[], entities[], relationships[]
```

### V.3 Session Resume

```rust
async fn resume_session(session_id: SessionId) -> ResumeContext {
    let segments = load_recent_segments(session_id, 5).await;
    let project_mem = load_project_memory(session_id).await;
    let user_mem = load_user_memory(user_id).await;

    ResumeContext {
        narrative_summary: segments.iter().map(|s| s.narrative).join("\n\n"),
        active_artifacts: merge_artifacts(segments),
        continuation_suggestions: project_mem.continuation_suggestions,
        personalized_greeting: build_greeting(user_mem, project_mem),
    }
}
```

---

## VI. Preference Learning (Implicit, Bayesian)

### VI.1 Behavioral Signals → Preference Dimensions

| Signal | Dimension | Inference |
|--------|-----------|-----------|
| Response length accepted | verbosity | Shorter accepted → prefer concise |
| User interrupts / continues | initiative | Interrupts → prefer proactive; continues → prefer detail |
| Technical terms used | technical_depth | Jargon → prefer technical |
| Formal/informal language | formality | Match user's register |
| Response time tolerance | pace | Fast replies → prefer quick; slow → prefer thorough |

### VI.2 Bayesian Update (per dimension)

```rust
struct PreferencePosterior {
    mean: f32,      // current estimate [0,1]
    variance: f32,  // uncertainty
    n_observations: u32,
}

fn update(posterior: &mut PreferencePosterior, signal: f32, weight: f32) {
    // Conjugate prior: Gaussian with known variance
    let prior_prec = 1.0 / posterior.variance;
    let signal_prec = weight / SIGNAL_VARIANCE;
    posterior.mean = (posterior.mean * prior_prec + signal * signal_prec) / (prior_prec + signal_prec);
    posterior.variance = 1.0 / (prior_prec + signal_prec);
    posterior.n_observations += 1;
}
```

### VI.3 Application

- **TurnPlanner**: Adjusts `detail_level`, `modality`, `chunk_count` from preferences
- **ProactiveQueue**: Tunes frequency, kinds from `initiative` + `pace`
- **VoiceProsody**: Sets baseline from `formality`, `warmth`
- **Report/Dashboard**: Chooses default modality, density from `verbosity`, `technical_depth`

---

## VII. Multi-LLM Routing (extends existing `mm-llm::routing`)

### VII.1 Enhanced RoutingRequest

```rust
pub struct RoutingRequest {
    // Existing
    pub purpose: Purpose,
    pub complexity: Complexity,
    pub stakes: Stakes,
    pub required_precision: Precision,
    pub latency_budget_ms: u32,
    pub cost_budget_micros: i64,
    pub pinned_model: Option<String>,

    // New: conversation-aware
    pub modality: Option<ResponseModality>,     // drill_down → needs reasoning model
    pub voice_tier: Option<VoiceTier>,          // FullDuplex → needs realtime model
    pub is_proactive: bool,                     // background → prefer cheap/fast
    pub session_context: SessionContext,        // long context → needs large window
    pub preference_profile: PreferenceProfile,  // user prefers certain models
}
```

### VII.2 Routing Table (config-driven)

| Task Type | Preferred Model | Fallback |
|-----------|----------------|----------|
| Coding/Tools | Claude 3.5 Sonnet | GPT-4o |
| Reasoning/Planning | o1-preview | Claude 3 Opus |
| Voice Realtime | GPT-4o Realtime | Gemini Live |
| Fast Chat | GPT-4o-mini | Haiku |
| Proactive/Background | Haiku | GPT-4o-mini |
| Structured Output | GPT-4o (grammar) | Claude (tool use) |
| Long Context | Gemini 1.5 Pro | Claude 3.5 Sonnet |

### VII.3 Integration Point

```rust
// In TurnPlanner.build_routing_request()
fn build_routing_request(&self, plan: &TurnPlan, transcript: &Transcript) -> RoutingRequest {
    RoutingRequest {
        purpose: plan.purpose,
        complexity: self.estimate_complexity(transcript),
        stakes: if plan.is_proactive { Stakes::Low } else { Stakes::Medium },
        required_precision: match plan.modality {
            ResponseModality::DrillDown => Precision::High,
            ResponseModality::Summarize => Precision::Normal,
            _ => Precision::Normal,
        },
        latency_budget_ms: plan.voice_config.as_ref()
            .map(|v| v.tier.latency_budget())
            .unwrap_or(0),
        cost_budget_micros: plan.budget.cost_micros,
        modality: Some(plan.modality),
        voice_tier: plan.voice_config.as_ref().map(|v| v.tier),
        is_proactive: plan.metadata.is_proactive,
        session_context: SessionContext::from(transcript),
        preference_profile: self.state.preference_profile.clone(),
        ..Default::default()
    }
}
```

---

## VIII. Proactive Plugin System (WASM-Compatible)

### VIII.1 Plugin Interface

```rust
#[async_trait]
trait ProactivePlugin: Send + Sync {
    fn id(&self) -> PluginId;
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn config_schema(&self) -> Value;          // JSON Schema
    fn triggers(&self) -> Vec<ProactiveTrigger>;
    async fn evaluate(&self, ctx: &PluginContext) -> Vec<ProactiveProposal>;
    async fn execute(&self, proposal: ProactiveProposal, ctx: &PluginContext) -> ProactiveResult;
}

struct PluginContext {
    session: &Session,
    memory: &MemoryContext,
    preferences: &PreferenceProfile,
    tools: &ToolCatalog,
    llm: &LlmService,
    budget: &ProactiveBudget,
}
```

### VIII.2 Built-in Plugins

| Plugin | Triggers | Proposals |
|--------|----------|-----------|
| **SearchLookup** | New entity, technical term, "what is" | Web search, doc lookup, definition |
| **GoalNarrowing** | Vague goal detected | Clarifying questions, option enumeration |
| **PlanRefinement** | Plan created, feedback received | Risk analysis, alternative steps, timeline |
| **PatternWatcher** | Recurring topic, repeated error | Suggestion, automation, template |
| **AffectResponder** | Strong affect detected | Empathy, de-escalation, celebration |
| **KnowledgeGap** | "I don't know", failed recall | Research proposal, learning resource |

### VIII.3 Plugin Lifecycle

```
Load (WASM/so) → Validate Schema → Register Triggers → Hot Reload → Sandbox Execute
                                                              ↓
                                            Resource limits: CPU, memory, network, time
```

---

## IX. Voice Prosody Adaptation (from `voice_prosody.md`)

### IX.1 Affect → Prosody Mapping

```rust
struct ProsodyController {
    baseline: ProsodyParams,           // from personality + user preference
    affect: AffectState,               // from mm-being affect.rs
    context: ProsodyContext,           // modality, stakes, modality
}

fn compute_prosody(controller: &ProsodyController) -> ProsodyParams {
    let mut params = controller.baseline;

    // Affect modulation (bounded)
    params.rate += 0.15 * controller.affect.arousal;           // faster when aroused
    params.pitch += 0.1 * controller.affect.valence;           // higher when positive
    params.volume += 0.1 * controller.affect.energy;           // louder when energetic
    params.warmth += 0.2 * controller.affect.warmth;           // warmer when warm
    params.pause_factor += 0.3 * controller.affect.caution;    // more pauses when cautious

    // Context modulation
    match controller.context.modality {
        ResponseModality::DrillDown => { params.rate *= 0.9; params.detail += 0.2; }
        ResponseModality::Summarize => { params.rate *= 1.1; params.detail -= 0.1; }
        ResponseModality::Explain => { params.pause_factor *= 1.2; }
        _ => {}
    }

    params.clamp()
}
```

### IX.2 SSML Generation

```rust
fn to_ssml(text: &str, prosody: &ProsodyParams) -> String {
    format!(
        r#"<speak><prosody rate="{:.0}%" pitch="{:+.1}st" volume="{:+.1}dB">{}</prosody></speak>"#,
        prosody.rate * 100.0,
        prosody.pitch * 12.0,  // semitones
        prosody.volume * 6.0,  // dB
        escape_xml(text)
    )
}
```

---

## X. Observability (from `conversation_management_engine.md` §14)

### X.1 State Snapshots (REST)

```
GET /api/cme/sessions/{id}/snapshot
→ SessionSnapshot {
    turn: TurnSnapshot,
    memory: MemorySnapshot,
    proactive: ProactiveSnapshot,
    voice: VoiceSnapshot,
    preferences: PreferenceSnapshot,
    timestamp: i64
}
```

### X.2 Real-time Event Stream (WebSocket)

```
WS /api/cme/sessions/{id}/events
→ Events: TurnStarted, ChunkEmitted, ToolCalled, ProactiveInjected,
          BargeInDetected, ModalitySwitched, MemoryCompressed,
          PreferenceUpdated, ImprovementLoopTriggered
```

### X.3 Distributed Tracing (OpenTelemetry)

```
Trace: conversation.turn
  Span: turn.plan (TurnPlanner)
  Span: turn.route (LLMRouter)
  Span: turn.llm (LlmService.call)
  Span: turn.chunks (ChunkExecutor)
  Span: turn.proactive (ProactiveCycle)
  Span: turn.compress (CompressionPipeline)
  Span: turn.learn (PreferenceLearner)
```

### X.4 Prometheus Metrics

| Metric | Type | Labels |
|--------|------|--------|
| `cme_turns_total` | Counter | session_id, modality, outcome |
| `cme_turn_duration_ms` | Histogram | modality, voice_tier |
| `cme_chunk_latency_ms` | Histogram | chunk_kind, streaming |
| `cme_proactive_items_total` | Counter | kind, status, plugin |
| `cme_voice_tier_negotiated` | Counter | requested, actual |
| `cme_barge_in_total` | Counter | tier, outcome |
| `cme_memory_compression_ratio` | Gauge | tier |
| `cme_preference_posterior_variance` | Gauge | dimension |
| `cme_improvement_loops_total` | Counter | domain, outcome |
| `cme_llm_routing_decision` | Counter | model, reason, modality |

---

## XI. Integration with Existing mm-serve

### XI.1 Crate Structure (Refactored)

```
crates/
├── mm-cme/                    # NEW: Conversation Management Engine
│   ├── src/
│   │   ├── session.rs         # Session, TurnPlan, ChunkDirective
│   │   ├── planner.rs         # TurnPlanner, SpeculativeExecutor
│   │   ├── executor.rs        # TurnExecutor, ChunkExecutor
│   │   ├── proactive.rs       # ProactiveQueue, PluginSystem
│   │   ├── voice/             # VoicePipeline, RealtimeSession, Prosody
│   │   ├── memory/            # Compression, 4-tier store
│   │   ├── preferences/       # BayesianLearner, Profile
│   │   ├── routing/           # ConversationRouter (wraps mm-llm)
│   │   ├── observability/     # Snapshots, WebSocket, OTel, Metrics
│   │   ├── improvement/       # SelfImprovementLoop
│   │   ├── report/            # ReportGenerator (hill-climbable)
│   │   ├── dashboard/         # DashboardConstructor (hill-climbable)
│   │   └── lib.rs
│   └── Cargo.toml
│
├── mm-serve/                  # HTTP server, routes, AppState
│   └── src/
│       ├── routes/            # Thin adapters → mm-cme
│       ├── state.rs           # AppState holds CmeEngine
│       └── main.rs
│
├── mm-llm/                    # LLM service, routing, clients (existing)
├── mm-being/                  # Personality, affect, goals (existing)
└── mm-core/                   # Config, IDs, errors (existing)
```

### XI.2 Migration Path

| Phase | Action | Risk |
|-------|--------|------|
| 1 | Create `mm-cme` crate with core types (Session, TurnPlan, Chunk) | Low — new code |
| 2 | Extract `TurnPlanner` from `controller::reply` logic | Medium — core logic |
| 3 | Implement `ProactiveQueue` + 2 plugins (SearchLookup, GoalNarrowing) | Medium — new behavior |
| 4 | Add voice Tier 1 (wrap existing) + audio I/O abstractions | Low — wrapper |
| 5 | Add Tier 2 streaming pipeline + speculative execution | High — new async patterns |
| 6 | Add Tier 3 realtime (provider-dependent) | High — external deps |
| 7 | Implement 4-tier memory + compression pipeline | Medium — storage changes |
| 8 | Add preference learning + apply to planner | Medium — ML-lite |
| 9 | Add observability (snapshots, WS, OTel, metrics) | Low — instrumentation |
| 10 | Add hill-climbing + report/dashboard generators | Medium — new domains |
| 11 | Add self-improvement loop + governance | High — meta-cognitive |
| 12 | Synthetic testing framework + CI integration | Medium — test infra |

---

## XII. Configuration (Unified)

```toml
# config/cme.toml
[cme]
# Session
max_session_age_days = 30
max_turns_per_session = 1000

# Turn Planning
default_modality = "bullets"
default_detail_level = 2
max_chunks_per_turn = 8
turn_token_budget = 8000
turn_time_budget_ms = 30000

# Proactive
[proactive]
enabled = true
max_per_cycle = 2
cycle_interval_ms = 15000
budget_per_turn = 0.15  # fraction of turn budget
plugins = ["search_lookup", "goal_narrowing", "plan_refinement"]

# Voice
[voice]
default_tier = "StreamingPipeline"
fallback_chain = ["FullDuplexNative", "StreamingPipeline", "RequestResponse"]
sample_rate = 24000
vad_sensitivity = 0.5
barge_in_enabled = true
barge_in_threshold = 0.3

# Memory
[memory]
episodic_retention_days = 30
segment_retention_days = 90
compression_trigger_turns = 10
compression_model = "haiku"  # cheap model for compression

# Preferences
[preferences]
learning_rate = 0.1
min_observations_for_apply = 5
dimensions = ["verbosity", "initiative", "technical_depth", "formality", "pace"]

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

# Improvement
[improvement]
enabled = true
budget_time_ms_per_hour = 300000
budget_tokens_per_hour = 500000
human_approval_required = ["report", "dashboard", "agent_cognition"]
```

---

## XIII. Testing Strategy

### XIII.1 Unit Tests (per module)
- TurnPlanner: deterministic plans for given inputs
- ProactiveQueue: budget enforcement, priority ordering
- Memory: compression round-trip, artifact extraction
- Preferences: Bayesian update convergence
- Routing: golden table for routing decisions
- Voice: tier negotiation, failover logic

### XIII.2 Integration Tests
- Full turn: plan → route → LLM → chunks → frames
- Proactive cycle: trigger → proposal → execute → inject
- Voice Tier 1/2: audio in → frames out (mock ASR/TTS)
- Session resume: cold boot → context restored
- Improvement loop: artifact → reflect → plan → act → verify → learn

### XIII.3 Synthetic Testing (from `synthetic_testing.md`)
```yaml
# tests/synthetic/conversation_scenarios.yaml
scenarios:
  - name: "technical_debugging"
    persona: "senior_engineer"
    turns: 8
    checkers: [tool_correctness, modality_fit, proactive_relevance, latency]
  - name: "voice_natural_conversation"
    persona: "casual_user"
    voice_tier: "StreamingPipeline"
    turns: 12
    checkers: [barge_in_handling, prosody_appropriateness, turn_taking]
  - name: "report_generation_refinement"
    persona: "product_manager"
    turns: 6
    checkers: [report_clarity, hill_climb_improvement, export_format]
```

---

## XIV. Rollout & Compatibility

### XIV.1 API Compatibility

| Existing Endpoint | New Behavior | Breaking? |
|-------------------|--------------|-----------|
| `POST /api/chat` | Returns SSE with modality chunks | No (additive frames) |
| `GET /api/chat/stream` | Includes proactive frames | No (new frame types) |
| `POST /api/chat/answer` | Same | No |
| `GET /api/health` | Adds CME services | No |

### XIV.2 Feature Flags

```rust
struct FeatureFlags {
    cme_enabled: bool,              // route to new engine
    proactive_enabled: bool,
    voice_tier: VoiceTier,
    memory_compression_enabled: bool,
    preference_learning_enabled: bool,
    improvement_loop_enabled: bool,
}
```

### XIV.3 Migration Checklist

- [ ] `mm-cme` crate compiles and unit tests pass
- [ ] `mm-serve` routes delegate to `CmeEngine` behind flag
- [ ] Existing chat sessions resume correctly (memory compat)
- [ ] Voice Tier 1 works with existing audio test harness
- [ ] Proactive items appear in SSE stream without breaking UI
- [ ] Observability endpoints return data
- [ ] Synthetic test suite passes ≥90% checkers
- [ ] Load test: 100 concurrent sessions, p99 < 2s
- [ ] Canary deploy with 5% traffic
- [ ] Full rollout

---

## XV. Open Questions & Decisions Needed

| Question | Options | Recommendation |
|----------|---------|----------------|
| **WASM vs native plugins** | WASM (sandbox) vs `.so` (performance) | Start native, WASM for 3rd-party |
| **Realtime provider** | OpenAI Realtime, Gemini Live, custom WebRTC | Abstract behind `RealtimeClient` trait |
| **Memory storage** | SQLite (current) vs dedicated vector DB | Keep SQLite + add `mm-vector` crate later |
| **Preference persistence** | Per-session vs cross-session (user) | Cross-session (user memory tier) |
| **Improvement loop autonomy** | Fully auto vs human-in-the-loop | Human approval for report/dashboard/agent |
| **TurnPlan serialization** | Store in chat DB vs separate store | Chat DB (single source of truth) |

---

## XVI. Appendix: Design Document Cross-Reference

| Design Doc | Sections Implemented Here |
|------------|---------------------------|
| `proactive_conversation.md` | §II.4, §III.3, §VIII, §XII[proactive] |
| `conversation_management_engine.md` | §II, §III, §IV, §X, §XI, §XII |
| `turn_planning.md` | §II.2, §III.2 |
| `speculative_execution.md` | §III.2 step 3, §IV.2 |
| `conversation_compression.md` | §V, §XI.2 step 7 |
| `preference_learning.md` | §VI, §XII[preferences] |
| `multi_llm_routing.md` | §VII, §XII[routing] |
| `proactive_plugin_system.md` | §VIII |
| `voice_prosody.md` | §IX |
| `synthetic_testing.md` | §XIII.3 |
| `cross_session_memory.md` | §V |
| `hill_climbing.md` | §XI.2 step 10, §II.3 (ReportSection) |
| `report_generation.md` | §XI.2 step 10, ReportGenerator |
| `dashboard_construction.md` | §XI.2 step 10, DashboardConstructor |
| `self_improvement_loop.md` | §XI.2 step 11, §III.1 step 7 |
| `conversation_engine_improvements.md` | All (this plan consolidates the 9 improvements) |

---

*This design plan is the single source of truth for the CME implementation. Each section maps to a crate module or implementation phase. Review, approve, then begin Phase 1.*