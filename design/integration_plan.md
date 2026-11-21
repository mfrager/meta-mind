# MetaMind Integration Plan — Conversation Engine + RDF Backend (7-Phase Build)

**Status**: Implementation roadmap merging `conversation_engine_design.md` (state machine CME) + `rdf_backend_integration.md` (cubecast) into existing `mm-serve`
**Target**: Rebuild `mm-serve` on new architecture in 7 phases
**Prerequisites**: Designs approved, `cubecast` builds locally, `mm-serve` tests pass

---

## Phase Overview

| Phase | Name | Core Deliverable | Depends On |
|-------|------|------------------|------------|
| 0 | Foundation | Baseline verified, integration branch, crate scaffolding | — |
| 1 | Core State Machine Engine | `mm-sm` crate with hierarchical/orthogonal SM primitives | 0 |
| 2 | RDF Data Layer + Unified Memory | `mm-data` crate with cubecast + 4-tier memory | 1 |
| 3 | Conversation Engine Core | `mm-cme` with Session/Turn/Chunk SMs + 6 orthogonal regions | 2 |
| 4 | Planner, Executor, Routing & Voice | Deterministic TurnPlanner, TurnExecutor, ConversationRouter, Voice regions | 3 |
| 5 | Unified Memory, Preferences, Plugins | Memory/Preference/Plugin regions delegating to `mm-data` | 3, 2 |
| 6 | Observability, Improvement, Report/Dashboard | Snapshots, WS, OTel, Prometheus, Improvement loop, generators | 4, 5 |
| 7 | mm-serve Integration, Migration, Production | Full integration, dual-run, canary, cleanup, v2.0.0-cme | 6 |

**Critical Path**: 0 → 1 → 2 → 3 → 4 → 6 → 7 (Phase 5 runs parallel with 4)

---

## Phase 0: Foundation — Prepare & Baseline

### 0.1 Verify Current State
```bash
# Run existing test suite - must pass before changes
cd /home/mfrager/Build/metamind
cargo test --workspace 2>&1 | tail -50

# Verify cubecast builds
cd /home/mfrager/Build/cubecast
cargo build --workspace --features application 2>&1 | tail -20
```

### 0.2 Create Integration Branch
```bash
cd /home/mfrager/Build/metamind
git checkout -b integration/cme-rdf-backend
git push -u origin integration/cme-rdf-backend
```

### 0.3 Baseline Metrics
| Metric | Current | Target |
|--------|---------|--------|
| `cargo test --workspace` | Pass | Pass |
| `mm-serve` boot time | ~2s | <3s |
| Chat turn latency (p99) | ~1.5s | <2s |
| Memory usage (idle) | ~200MB | <300MB |
| Concurrent sessions | 50 | 200 |

### 0.4 Directory Setup for New Crates
```bash
mkdir -p crates/mm-sm/src
mkdir -p crates/mm-cme/src/{regions,driver,planner,executor,routing,observability,report,dashboard}
mkdir -p crates/mm-data/src
```

### 0.5 Exit Criteria
- [ ] All existing tests pass
- [ ] Cubecast builds successfully
- [ ] Integration branch created and pushed
- [ ] Baseline metrics recorded
- [ ] Crate directories scaffolded

---

## Phase 1: Core State Machine Engine — `mm-sm`

### 1.1 Create `mm-sm` Crate
**File**: `crates/mm-sm/Cargo.toml`
```toml
[package]
name = "mm-sm"
version.workspace = true
edition.workspace = true

[dependencies]
ulid = { workspace = true }
tokio = { workspace = true, features = ["sync", "time"] }
tracing = { workspace = true }
serde = { workspace = true }
thiserror = { workspace = true }
```

**Files to create**:
- `src/lib.rs` — exports
- `src/machine.rs` — `StateMachine`, `SmInstance`, `SmEvent`, `SmState`, `SmAction`, `SmGuard`, `Transition`
- `src/hierarchical.rs` — `spawn_child`, `parent`, `history` pseudostates
- `src/orthogonal.rs` — `enter_region`, `orthogonal_current`, parallel dispatch
- `src/snapshot.rs` — `SmSnapshot`, serialization
- `src/tracing.rs` — OpenTelemetry span per SM instance

### 1.2 Unit Tests (TDD)
```rust
// tests/basic_sm.rs
#[tokio::test] async fn simple_transition() { ... }
#[tokio::test] async fn guard_blocks_transition() { ... }
#[tokio::test] async fn action_on_transition() { ... }
#[tokio::test] async fn hierarchical_spawn_child() { ... }
#[tokio::test] async fn orthogonal_regions_parallel() { ... }
#[tokio::test] async fn history_pseudostate_restore() { ... }
```

### 1.3 Exit Criteria
- [ ] `mm-sm` compiles, all tests pass
- [ ] Published to local registry (`cargo publish --local` or path dep)
- [ ] Documentation: `SM_INTEGRATION_GUIDE.md`

---

## Phase 2: RDF Data Layer + Unified Memory — `mm-data`

### 2.1 Create `mm-data` Crate
**File**: `crates/mm-data/Cargo.toml`
```toml
[package]
name = "mm-data"
version.workspace = true
edition.workspace = true

[dependencies]
cubecast = { path = "/home/mfrager/Build/cubecast", features = ["application"] }
mm-core = { path = "../mm-core" }
mm-sm = { path = "../mm-sm" }
tokio = { workspace = true }
tracing = { workspace = true }
serde = { workspace = true }
```

### 2.2 Implement Core Modules (Including Unified Memory)
| File | Responsibility |
|------|----------------|
| `src/engine.rs` | `RdfEngine` wrapper over `Cubecast`, config mapping |
| `src/config.rs` | `EngineConfig`, `build_cubecast_config()`, unified TOML |
| `src/init.rs` | `initialize_data_layer()` — 9-step bootstrap |
| `src/agentstream.rs` | `AgentstreamStore` — session graphs, event log, projections (Tier 1) |
| `src/users.rs` | `UserRegistry` — CRUD, FOAF/SKOS profiles, symlinks (Tier 4) |
| `src/projects.rs` | `ProjectRegistry` — DOAP manifests, members, datasets (Tier 3) |
| `src/apps.rs` | `AppRegistry` — manifests, plugins, data |
| `src/datasets.rs` | `DatasetCatalog` — DCAT, versions, PROV-O provenance |
| `src/graphs.rs` | Graph aliases, routing to cubecast multi-graph |
| `src/ontology.rs` | Load MetaMind ontology files (mm.ttl + 11 shapes) |
| `src/sqlite.rs` | SQLite pool for non-RDF data (chat, audit, ULID) |
| **`src/memory/episodic.rs`** | **Tier 1: EpisodicTurn, AgentstreamEvent schemas** |
| **`src/memory/compressed.rs`** | **Tier 2: CompressedSegment, DecisionRecord, Commitment, etc.** |
| **`src/memory/project.rs`** | **Tier 3: ProjectMemory, ConventionRule, TechStack** |
| **`src/memory/user.rs`** | **Tier 4: UserMemory, UserPreferenceModel, CrossProjectLearner** |
| **`src/memory/compressor.rs`** | **UnifiedCompressor: LLM extraction → narrative → embed → quality** |
| **`src/memory/aggregator.rs`** | **ProjectAggregator: merges segments → ProjectMemory** |
| **`src/memory/cross_project.rs`** | **CrossProjectLearner: transfers expertise/conventions to user** |
| **`src/memory/continuity.rs`** | **SessionContinuityManager: resume with context, greetings** |
| **`src/memory/sync.rs`** | **MultiDeviceSync: presence, broadcast, voice↔text handoff** |
| **`src/memory/rdf_mapping.rs`** | **RDF graph IRIs, SPARQL patterns per tier** |
| **`src/memory/mod.rs`** | **MemorySystem: unified API (EpisodicStore, SegmentStore, ProjectMemoryStore, UserMemoryStore)** |

### 2.3 Directory Structure Creation (Idempotent)
```rust
fn create_directory_structure(base: &Path) -> Result<()> {
    for dir in [
        "cubecast/main", "cubecast/graphs/stores", "cubecast/registry",
        "cubecast/projects", "cubecast/extractions", "cubecast/cache",
        "agentstream/sessions", "agentstream/events", "agentstream/projections",
        "users", "projects", "apps", "datasets", "sqlite", "logs", "temp/uploads"
    ] {
        std::fs::create_dir_all(base.join(dir))?;
    }
    Ok(())
}
```

### 2.4 Integration Test
```rust
#[tokio::test]
async fn full_init_sequence() {
    let temp = tempfile::tempdir()?;
    let result = initialize_data_layer(InitConfig { base_dir: temp.path(), ... }).await?;
    assert!(result.engine.store().is_ready());
    assert!(result.agentstream.start_session(...).await.is_ok());
    assert!(result.users.create_user(...).await.is_ok());
    
    // Unified memory tests
    let memory = result.memory_system;
    let session_ctx = memory.start_session(user_id, project_id, None).await?;
    memory.log_turn(episodic_turn).await?;
    let segments = memory.compress_session(session_id).await?;
    assert!(!segments.is_empty());
}
```

### 2.5 Exit Criteria
- [ ] `mm-data` compiles, integration test passes
- [ ] Can open cubecast engine, run SPARQL query
- [ ] Agentstream logs event, queryable via SPARQL (Tier 1)
- [ ] User/Project/App/Dataset CRUD works
- [ ] **Unified memory**: compression pipeline runs, segments stored in RDF + vector index
- [ ] **Project aggregation** merges segments into ProjectMemory
- [ ] **User sync** transfers cross-project learning
- [ ] **Session continuity** provides resume context with greetings
- [ ] **Multi-device sync** presence/broadcast/handoff functional

---

## Phase 3: Conversation Engine Core — `mm-cme`

### 3.1 Session & Turn State Machines

**Files**:
- `src/session.rs` — `Session`, `SessionState`, `SessionEvent`, `SessionContext`, 6 orthogonal regions
- `src/turn.rs` — `Turn`, `TurnState`, `TurnEvent`, `TurnContext`, `TurnPlan`
- `src/chunk.rs` — `ChunkSm`, `ChunkState`, `ChunkEvent`, `ChunkDirective`
- `src/driver.rs` — `SessionDriver` main loop with `tokio::select!`

**State Machine Definitions** (using `mm-sm`):

```rust
// Session SM
let session_sm = StateMachine {
    initial: SessionState::Initializing,
    states: vec![Initializing, Active, Paused, Suspended, Resuming, Terminating, Terminated],
    transitions: vec![
        Transition { from: Initializing, to: Active, event: InitComplete },
        Transition { from: Active, to: Paused, event: UserAway },
        Transition { from: Paused, to: Active, event: UserReturn },
        Transition { from: Active, to: Suspended, event: SuspendRequest },
        Transition { from: Suspended, to: Resuming, event: ResumeRequest },
        Transition { from: Resuming, to: Active, event: ResumeComplete },
        Transition { from: Active, to: Terminating, event: ShutdownRequest },
        Transition { from: Terminating, to: Terminated, event: RegionsTerminated },
    ],
    orthogonal_regions: vec![turn_region, proactive_region, voice_region, memory_region, pref_region, plugin_region, improvement_region],
    history_states: vec![],
};

// Turn SM (child)
let turn_sm = StateMachine {
    initial: TurnState::Planning,
    states: vec![Planning, Routing, SpeculativeWarm, Executing, Streaming, ToolCalling, WaitingConfirm, Finalizing, Completed, Interrupted, Failed],
    transitions: vec![
        Transition { from: Planning, to: Routing, event: PlanReady },
        Transition { from: Routing, to: SpeculativeWarm, event: RouteSelected, guard: has_voice_tier_2_3 },
        Transition { from: Routing, to: Executing, event: RouteSelected, guard: not_voice_tier_2_3 },
        Transition { from: SpeculativeWarm, to: Executing, event: SpeculativeReady },
        Transition { from: Executing, to: Streaming, event: LlmStreamStart },
        Transition { from: Streaming, to: ToolCalling, event: LlmToolCall },
        Transition { from: ToolCalling, to: Streaming, event: ToolResult },
        Transition { from: Streaming, to: WaitingConfirm, event: ConfirmationRequired },
        Transition { from: WaitingConfirm, to: Streaming, event: ConfirmationAnswer },
        Transition { from: Streaming, to: Finalizing, event: AllChunksDone },
        Transition { from: Finalizing, to: Completed, event: Finalized },
        Transition { from: Streaming, to: Interrupted, event: BargeInDetected },
        Transition { from: *, to: Failed, event: Error },
    ],
    orthogonal_regions: vec![],
    history_states: vec![Interrupted], // deep history to Streaming
};
```

### 3.2 Orthogonal Regions

| Region | File | Key States |
|--------|------|------------|
| Proactive | `src/regions/proactive.rs` | Idle, Evaluating, Selecting, Planning, Executing, Injecting, Suppressed |
| Voice | `src/regions/voice.rs` | Negotiating, Tier1/2/3, Streaming, BargedIn, Recovering, Degraded |
| Memory | `src/regions/memory.rs` | Episodic, Compressing, Segmented, ProjectAggregating, UserSyncing |
| Preferences | `src/regions/preferences.rs` | ColdStart, Observing, Converging, Stable, DriftDetected, Relearning (per dimension) |
| Plugins | `src/regions/plugins.rs` | Loading, Validating, Registered, Running, Suspended, Updating, Unloading |
| Improvement | `src/regions/improvement.rs` | Idle, Reflecting, Planning, Acting, Verifying, Learning, Completed |

### 3.3 Exit Criteria
- [ ] `mm-cme` compiles
- [ ] `SessionDriver` runs: spawns turn SM, drives regions
- [ ] Unit tests for each SM transition table
- [ ] Integration test: full session → turn → chunk → completion

---

## Phase 4: Planner, Executor, Routing & Voice

### 4.1 TurnPlanner (in `Turn.Planning` state)
**File**: `src/planner.rs`
```rust
impl TurnPlanner {
    pub fn build_plan(&self, ctx: &TurnContext, session: &SessionContext) -> TurnPlan { ... }
    fn plan_chunks(&self, modality: ResponseModality, detail: u8) -> Vec<ChunkDirective> { ... }
    fn plan_tool_policy(&self, session: &SessionContext) -> ToolPolicy { ... }
    fn allocate_budget(&self, budgets: &SessionBudgets, chunk_count: usize) -> TurnBudget { ... }
}
```

### 4.2 TurnExecutor (drives Turn SM)
**File**: `src/executor.rs`
```rust
impl TurnExecutor {
    pub async fn execute(&mut self, turn: &mut Turn) -> Result<TurnOutcome> {
        turn.sm.dispatch(TurnEvent::PlanReady(turn.plan.clone())).await?;
        
        let decision = self.router.choose(&turn.context.routing).await?;
        turn.sm.dispatch(TurnEvent::RouteSelected(decision)).await?;
        
        if turn.context.voice_config.speculative {
            self.llm_service.speculative_prewarm(&turn.context.speculative_hint).await?;
            turn.sm.dispatch(TurnEvent::SpeculativeReady).await?;
        }
        
        let stream = self.llm_service.call_streaming(request).await?;
        turn.sm.dispatch(TurnEvent::LlmStreamStart).await?;
        
        while let Some(chunk) = stream.next().await {
            for chunk_sm in &mut turn.chunks {
                chunk_sm.dispatch(ChunkEvent::LlmToken(chunk.text)).await?;
            }
        }
        
        turn.sm.dispatch(TurnEvent::AllChunksDone).await?;
        self.finalize(turn).await
    }
}
```

### 4.3 ConversationRouter
**File**: `src/routing.rs`
- Wraps `mm-llm::routing::ThresholdRouter`
- Adds conversation-aware fields: `modality`, `voice_tier`, `is_proactive`, `preference_profile`
- Routing table config: `config/routing_table.toml`

### 4.4 Voice Region Implementation
**File**: `src/regions/voice.rs`
- Tier negotiation with provider caps
- Tier 1: ASR → LLM → TTS (sequential)
- Tier 2: Streaming pipeline with overlap + speculative
- Tier 3: Realtime API (OpenAI/Gemini) — abstract behind `RealtimeClient` trait
- Barge-in detector, prosody controller

### 4.5 Audio I/O Abstractions
**File**: `src/voice/audio.rs`
- `AudioInput` trait (microphone, file, network)
- `AudioOutput` trait (speaker, file, network)
- `Vad` trait (silero, webrtc-vad)
- `AsrClient` trait (whisper.cpp, cloud)
- `TtsClient` trait (piper, cloud)

### 4.6 Exit Criteria
- [ ] Deterministic TurnPlan from planner
- [ ] TurnExecutor drives Turn SM through all states
- [ ] Chunk SMs stream text/structured/tool calls
- [ ] Barge-in transitions to `Interrupted` history state
- [ ] Routing selects correct model for modality/voice tier
- [ ] Tier 1 voice works (mock ASR/TTS)
- [ ] Tier 2 streaming pipeline works (mock)
- [ ] Tier 3 trait defined, mock implementation

---

## Phase 5: Unified Memory, Preferences, Plugins

> **Implementation follows**: [`unified_memory_system.md`](unified_memory_system.md) — single source of truth for 4-tier hierarchy, compression pipeline, RDF storage mapping, session continuity, and multi-device sync.

### 5.1 Memory Region (Delegates to `mm-data::MemorySystem`)
**File**: `src/regions/memory.rs` — **Thin SM wrapper** over `mm-data::MemorySystem`

```rust
pub struct MemoryRegion {
    memory_system: Arc<MemorySystem>,
}

impl MemoryRegion {
    async fn handle_event(&mut self, event: MemoryEvent) -> Result<()> {
        match event {
            MemoryEvent::TurnCompleted(turn) => {
                self.memory_system.log_turn(turn.into()).await?;
                if self.should_compress() {
                    self.dispatch(MemoryEvent::CompressionTriggered).await?;
                }
            }
            MemoryEvent::CompressionTriggered => {
                let segments = self.memory_system.compress_session(session_id).await?;
                self.dispatch(MemoryEvent::CompressionDone(segments)).await?;
            }
            MemoryEvent::CompressionDone(segments) => {
                for seg in segments {
                    if let Some(pid) = seg.project_id {
                        self.memory_system.aggregate_project(pid).await?;
                    }
                }
                self.dispatch(MemoryEvent::ProjectAggregationTriggered).await?;
            }
            MemoryEvent::RetentionTick => {
                self.memory_system.run_retention_cleanup().await?;
            }
            _ => {}
        }
        Ok(())
    }
}
```

**mm-data implements** (see `unified_memory_system.md` §III):
- **Episodic buffer** → Agentstream + SQLite
- **UnifiedCompressor** → LLM extraction → narrative → embeddings → quality validation → RDF + vector index
- **ProjectAggregator** → merges segments into ProjectMemory (cubecast project layer)
- **CrossProjectLearner** → updates UserMemory (cubecast user layer)
- **SessionContinuityManager** → resume with context, personalized greetings
- **MultiDeviceSync** → presence, broadcast, voice↔text handoff

### 5.2 Preference Region (Delegates to `mm-data::MemorySystem`)
**File**: `src/regions/preferences.rs` — **Thin SM wrapper**
- 5 dimensions: verbosity, initiative, technical_depth, formality, pace
- Bayesian update per signal → `MemorySystem.update_preference()`
- Convergence detection (variance < threshold) → `PlannerHooks.update()`
- Drift detection (signal > 3σ from posterior) → `PreferenceState::DriftDetected`
- Planner hooks: `update(dimension, mean)`

### 5.3 Plugin Region
**File**: `src/regions/plugins.rs`
- WASM sandbox (wasmtime) or native `.so`
- Plugin trait: `evaluate()`, `execute()`, `config_schema()`, `triggers()`
- Built-in plugins: SearchLookup, GoalNarrowing, PlanRefinement, PatternWatcher, AffectResponder, KnowledgeGap
- Trigger registry, budget enforcement

### 5.4 Exit Criteria
- [ ] Memory compression runs via `mm-data`, segments queryable via SPARQL + vector search
- [ ] Preferences converge after ~10 observations, applied to planner via hooks
- [ ] Plugins load, evaluate, execute within sandbox
- [ ] Proactive items injected into session stream
- [ ] Session resume works with project/user context
- [ ] Multi-device sync: presence, broadcast, handoff functional

---

## Phase 6: Observability, Improvement, Report/Dashboard

### 6.1 Observability
**File**: `src/observability.rs`
- REST snapshots: `GET /api/cme/sessions/{id}/snapshot`
- WebSocket events: `WS /api/cme/sessions/{id}/events` (SMTransition events)
- OpenTelemetry: one span per SM instance, transitions as span events
- Prometheus metrics: `cme_sm_transitions_total`, `cme_sm_state_duration_ms`, etc.

### 6.2 Improvement Region
**File**: `src/regions/improvement.rs`
- 5-phase loop: Reflect → Plan → Act → Verify → Learn
- Critics: Correctness, Clarity, Actionability, Completeness, Consistency, Tone, Efficiency, Safety
- Verifiers: UnitTest, Lint, Schema, LLMJudge, HumanReview, Simulation
- HillClimbingEngine integration (from `hill_climbing.md`)
- Governance: budgets, human approval gates, auto-rollback

### 6.3 Report & Dashboard Generators
**File**: `src/report.rs`, `src/dashboard.rs`
- ReportGenerator: modality-aware sections, hill-climbable, multi-format export
- DashboardConstructor: cost-aware panels, context-aware layouts, hill-climbable

### 6.4 Exit Criteria
- [ ] Snapshots return full SM state + orthogonal regions
- [ ] WebSocket streams SMTransition events in real-time
- [ ] OTel traces show hierarchical spans
- [ ] Improvement loop runs on schedule, improves report/dashboard

---

## Phase 7: mm-serve Integration, Migration, Production

### 7.1 Update AppState
**File**: `crates/mm-serve/src/state.rs`
```rust
use mm_cme::{SessionDriver, SessionConfig};
use mm_data::{initialize_data_layer, InitResult, InitConfig, MemorySystem};

pub struct AppState {
    // ... existing fields (sqlite, events, logger, ids, lock, focus, trees, chat) ...
    
    // NEW: Unified data layer (includes MemorySystem)
    pub data: InitResult,
    pub memory_system: Arc<MemorySystem>,  // unified memory API
    pub rdf_engine: Arc<RdfEngine>,
    pub agentstream: Arc<AgentstreamStore>,
    
    // NEW: Conversation engine
    pub cme_driver: Arc<SessionDriver>,
}
```

### 7.2 Boot Sequence Integration
**File**: `crates/mm-serve/src/state.rs` — `AppState::boot()`
```rust
pub async fn boot(cfg: Config, lock: ConsoleLock) -> Result<Self, MmError> {
    // ... existing: sqlite, events, logger, ids, lock ...
    
    // 1. Initialize mm-data (RDF + agentstream + registries + unified memory)
    let init_config = InitConfig {
        base_dir: cfg.store.data_dir.clone(),
        enable_multi_graph: true,
        enable_registry: true,
        enable_projects: true,
        enable_extractions: false,
        enable_inference: true,
    };
    let data = initialize_data_layer(init_config).await?;
    
    // 2. Extract unified memory system
    let memory_system = data.memory_system.clone();
    
    // 3. Initialize CME driver with memory system
    let cme_config = SessionConfig::from_toml(&cfg.cme_config)?;
    let cme_driver = Arc::new(SessionDriver::new(
        cme_config, 
        data.rdf_engine.clone(),
        memory_system.clone(),
    ).await?);
    
    // 4. Start driver (background task)
    tokio::spawn(cme_driver.clone().run());
    
    // 5. Existing: graph, being, focus, trees, chat
    // ... but chat now uses CME for turns ...
    
    Ok(Self { ..., data, memory_system, rdf_engine: data.engine, agentstream: data.agentstream, cme_driver })
}
```

### 7.3 Controller Integration (Agentstream + Memory Logging)
**File**: `crates/mm-serve/src/controller/turn.rs`
```rust
async fn reply(state: &AppState, ...) -> TurnOutcome {
    // ... existing logic ...
    
    // Log turn to unified memory system (episodic + agentstream)
    let episodic_turn = turn_to_episodic(&outcome, session_id, user_id, project_id);
    state.memory_system.log_turn(episodic_turn).await?;
    
    // Log each frame to agentstream (also in memory system)
    for frame in &frames {
        let event = frame_to_agentstream_event(frame, session_id, turn_id);
        state.memory_system.log_agentstream_event(event).await?;
    }
}
```

### 7.4 Route Adapters
**File**: `crates/mm-serve/src/routes/chat.rs`
```rust
pub async fn open(
    State(state): State<Arc<AppState>>,
    Json(body): Json<OpenChat>,
) -> Result<Response, AppError> {
    let session = state.cme_driver.get_or_create_session(body.session_id).await?;
    let turn_id = session.submit_user_input(body.query, body.pane, body.propose).await?;
    let frames = session.stream_frames(turn_id).await?;
    Ok(sse_response(&frames)?.into_response())
}
```

### 7.5 Migration & Compatibility
**File**: `crates/mm-data/src/migrate.rs`
```rust
pub async fn migrate_from_mm_serve(old_data_dir: &Path, new_base_dir: &Path) -> Result<()> {
    copy_sqlite(old_data_dir, new_base_dir.join("sqlite")).await?;
    for graph in ["being", "memory", "epistemic", "world", "library", "code", "decision", "tools", "selfeng", "self", "provenance"] {
        export_graph(old_data_dir.join("graph"), graph).await?;
        import_graph(new_base_dir, graph).await?;
    }
    for session in load_old_sessions(old_data_dir).await? {
        create_agentstream_session(&session).await?;
    }
    migrate_users_projects_apps(old_data_dir, new_base_dir).await?;
    Ok(())
}
```

**Feature Flags** — `crates/mm-serve/src/feature_flags.rs`:
```rust
pub struct FeatureFlags {
    pub cme_enabled: bool,
    pub proactive_enabled: bool,
    pub voice_tier: VoiceTier,
    pub memory_compression_enabled: bool,
    pub preference_learning_enabled: bool,
    pub improvement_loop_enabled: bool,
    pub agentstream_logging: bool,
}
```

- Old `controller::reply` behind `cme_enabled = false`
- New CME behind `cme_enabled = true`
- Canary: 5% → 25% → 50% → 100% traffic

### 7.6 Synthetic Testing & Load
- YAML DSL scenarios, 5 personas, 8 checkers
- CI integration with regression detection
- Load test: 100 concurrent sessions, 10 turns each, 300s
- Target: Turn p50 <500ms, p99 <2s, 200+ concurrent sessions

### 7.7 Documentation & Debug UI
- `ARCHITECTURE.md`, `DEVELOPER_GUIDE.md`, `OPERATIONS.md`, `MIGRATION_GUIDE.md`
- Web UI at `/debug/cme` showing SM states, regions, queues, metrics
- `mm-serve config check` command

### 7.8 Exit Criteria (Definition of Done)
- [ ] `mm-serve` boots with mm-data + mm-cme + unified memory
- [ ] `POST /api/chat` works via CME
- [ ] SSE stream includes modality chunks, proactive frames
- [ ] Agentstream logs conversation events via unified memory
- [ ] MemorySystem API accessible for queries (decisions, commitments, artifacts, goals)
- [ ] Session resume works with project/user context
- [ ] Multi-device sync functional
- [ ] Migration script runs, verifies data integrity
- [ ] Feature flags control CME vs old path
- [ ] Canary deploy works (5% → 100%)
- [ ] Synthetic test suite passes ≥90% checkers
- [ ] Load test meets targets
- [ ] All docs published, debug UI functional
- [ ] Legacy code removed (`controller/`, `mm-store-graph/`)
- [ ] Release tagged: `v2.0.0-cme`

---

## Dependency Graph

```
Phase 0: Foundation
    │
    ├─ Phase 1: mm-sm (core SM engine) ◄──────────────────┐
    │                                                      │
    ├─ Phase 2: mm-data (RDF + Unified Memory) ◄──────────┤
    │                                                      │
    ├─ Phase 3: mm-cme core SMs (Session, Turn, Chunk)   │
    │         │                                            │
    │         ├─ Phase 4: Planner + Executor + Routing + Voice  │
    │         │                                            │
    │         ├─ Phase 5: Memory + Preferences + Plugins  │
    │         │         │                                  │
    │         └─ Phase 6: Observability + Improvement + Report/Dashboard  │
    │                   │                                  │
    └───────────────────┴──────────────────────────────────┘
                              │
                    Phase 7: mm-serve Integration + Migration + Production
```

---

## Risk Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Cubecast RocksDB build fails | Medium | High | Pre-built binaries, Docker build env |
| SM engine performance overhead | Low | Medium | Benchmark early, optimize dispatch |
| Voice Tier 3 provider API changes | Medium | Low | Abstract behind trait, mock for tests |
| Migration data loss | Low | Critical | Dry-run migration, checksum verification |
| Existing tests break | High | Medium | Dual-run mode, feature flags, gradual cutover |
| Synthetic test flakiness | Medium | Low | Deterministic seeds, retry logic |
| Phase slip | Medium | Medium | Clear exit criteria per phase, early integration |

---

## Success Criteria (Definition of Done)

1. **Functional**: All existing `mm-serve` features work via CME
2. **Performance**: Turn p99 < 2s, 200+ concurrent sessions
3. **Reliability**: 99.9% uptime, <0.1% error rate
4. **Observability**: Full SM introspection, OTel traces, metrics
5. **Extensibility**: New modality/plugin/region in <1 day
6. **Data Integrity**: Migration verified, agentstream queryable
7. **Tests**: Unit >90%, Integration >80%, Synthetic >90% checkers
8. **Documentation**: Complete, examples runnable

---

*This plan is the single source of truth for the integration. Each phase has clear exit criteria and dependencies. Maintain the critical path: mm-sm → mm-data → mm-cme core → (Planner/Executor/Routing/Voice) + (Memory/Prefs/Plugins) → Observability/Improvement → mm-serve integration.*