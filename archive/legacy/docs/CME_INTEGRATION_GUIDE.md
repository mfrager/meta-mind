# `mm-cme` Integration Guide

`mm-cme` is the conversation engine core: the Session, Turn and Chunk state
machines, the six orthogonal regions, deterministic planning, conversation-aware
routing, the turn executor, the three voice tiers and the driver loop. It uses
`mm-sm` with runtime-named states and events, so the machines compose and
snapshot without monomorphizing over the context type. Phase 5 connects the
regions to real work: `mm-data`'s unified memory, preference learning, and the
proactive plugin host. Phase 6 adds transport-neutral observability, the
reflection-to-learning improvement loop, and hill-climbable report and dashboard
artifacts.

## Shape

```
Session (mm-sm instance)
├── region: proactive
├── region: voice
├── region: memory
├── region: preferences
├── region: plugins
├── region: improvement
└── active_turn: Option<Turn>
     └── chunks: Vec<Chunk>
```

The session machine validates the whole tree at startup: every region's states
and transitions are checked when `Session::new` runs.

## Running a session

```rust
use std::time::Duration;
use mm_cme::{DriverMessage, Session, SessionDriver};

let session = Session::new("user-1", None)?;
let (driver, messages, mut outcomes) =
    SessionDriver::spawn(session, Duration::from_secs(15));
let handle = tokio::spawn(driver.run());

messages.send(DriverMessage::StartTurn { query: "hello".into() })?;
let outcome = outcomes.recv().await.unwrap();
assert!(outcome.is_success());

messages.send(DriverMessage::Shutdown)?;
let session = handle.await??;
assert_eq!(session.state(), "terminated");
```

`SessionDriver::run` dispatches `init_complete`, then loops over `tokio::select!`:
external messages, the periodic tick and turn work. Shutdown pauses the regions
and dispatches `regions_terminated`, leaving the session in `terminated`.

`spawn` installs the deterministic executor (`Echo:` replies, no model). To use a
provider, build an executor and pass it to `spawn_with_executor`:

```rust
use std::sync::Arc;
use mm_cme::{ConversationRouter, RoutingTable, TurnExecutor};
use mm_llm::{OpenAiClient, ThresholdRouter};

let config = mm_llm::LlmConfig::load_or_default("config/llm.toml")?;
let table = RoutingTable::load("config/routing_table.toml".as_ref())?;
let router = ConversationRouter::with_table(
    Arc::new(ThresholdRouter::new(&config)),
    table,
);
let client = Arc::new(OpenAiClient::new(&config)?);
let executor = TurnExecutor::with_llm(client, router);
let (driver, messages, outcomes) =
    SessionDriver::spawn_with_executor(session, Duration::from_secs(15), executor);
```

## Session transitions

| From | Event | To |
|---|---|---|
| initializing | init_complete | active |
| active | user_away | paused |
| paused | user_return | active |
| active | suspend_request | suspended |
| suspended | resume_request | resuming |
| resuming | resume_complete | active |
| any | shutdown_request | terminating |
| terminating | regions_terminated | terminated |

`SessionEvent::UserInput`, `TurnCompleted`, `Tick` and `BargeIn` are forwarded to
every region; the session machine has no transition for them, so only the
regions react.

## Turn transitions

`planning → routing → (speculative_warm) → executing → streaming`, with
`tool_calling` and `waiting_confirm` as streaming sub-states, then
`finalizing → completed`.

- `route_selected` carries `speculative`; the guard sends voice Tier 2/3 turns to
  `speculative_warm`, everything else straight to `executing`.
- `streaming --barge_in_detected--> interrupted` records `streaming` as the
  history target. `resume` restores it through `mm-sm`'s history pseudostate.
- `error` from any state fails the turn.

## Chunks

A chunk starts `pending` (or `buffering` for a structured payload), and moves
`streaming → completed`. Tool chunks cycle through
`tool_executing ⇄ tool_waiting_confirm`. `cancel` and `error` are terminal from
any state.

`Turn::spawn_chunks` starts every chunk with no dependencies. `complete_chunks`
finishes streaming/buffering chunks and repeatedly releases `pending` chunks
whose dependencies have all completed, so a dependency chain resolves in one
call.

## Regions

| Region | States |
|---|---|
| proactive | idle, evaluating, selecting, planning, executing, injecting, suppressed, completed |
| voice | idle, negotiating, tier1_request_response, tier2_streaming, tier3_full_duplex, barged_in, recovering, degraded |
| memory | episodic, compressing, segmented, project_aggregating, project_synced, user_syncing, user_synced, purging |
| preferences | cold_start, observing, converging, stable, drift_detected, relearning |
| plugins | loading, validating, registered, running, suspended, updating, unloading, failed |
| improvement | idle, reflecting, planning, acting, verifying, learning, completed |

Each region has typed events (`ProactiveEvent`, `VoiceEvent`, …) and a `state()`
accessor. The session forwards raw event names; a region with no matching
transition ignores the event. Region-only traffic such as starting voice uses
`session.dispatch_regions("start_voice", None)`.

## Planning

`TurnPlanner::build_plan` is deterministic and never calls a model. It reads the
query's wording first, then preferences:

| Query signal | Modality | Chunks |
|---|---|---|
| summarize / tl;dr | `summarize` | `bullets` text |
| big picture / implications | `zoom_out` | `summary` text + buffered `implications` |
| detail / drill / sources | `drill_down` | `overview` text + buffered `details` + `sources` |
| bullet / list / steps, or `format > 0.4` | `bullets` | `bullets` text |
| anything else | `explain` | `concept` + `analogy` text |

The `detail` preference maps −1..1 to level 1..5 (neutral 3). A `tools > 0.5`
preference sets `ToolPolicy::Allowed` and appends a tool chunk; a
`confirm_tools > 0.5` preference takes precedence and sets `ToolPolicy::Confirm`
instead. Proactive turns never use tools. Budget is split evenly with a
128-token floor per chunk, and a voice tier appends an audio chunk.

## Routing

`ConversationRouter` wraps any `mm_llm::Router` (production uses
`ThresholdRouter`) and adds modality, voice tier, proactive flag, learned
preferences and a cost ceiling. Mapping:

- complexity: deterministic from query length; `drill_down` or `quality > 0.5`
  raises it to high
- stakes: low for proactive turns, medium otherwise
- precision: high for `drill_down`
- latency: 1500 ms Tier 1, 700 ms Tier 2, 300 ms Tier 3; `speed > 0.5` gives text
  turns an 800 ms ceiling

`config/routing_table.toml` optionally pins a model, in precedence order voice
tier → proactive → modality → `default_model`. Without a pin the cheapest
sufficient configured model wins. The table is a preference, not a bypass: the
substrate still validates the model and records the decision.

## Executing

`TurnExecutor::execute` drives one turn:

1. planner builds the plan and `PlanReady` moves the turn to routing
2. the router picks a model and `RouteSelected` records it; voice Tier 2/3 enters
   `speculative_warm` and exits through `SpeculativeReady`
3. `LlmStreamStart` spawns chunks and opens `streaming`
4. the complete reply is split across streaming text chunks in order; buffered
   structured chunks receive a JSON payload; provider tool calls run through the
   `ToolHandler` under the turn's policy: `allowed` goes straight to
   `tool_call → tool_executing → tool_result`, while `confirm` pauses the turn in
   `waiting_confirm` (chunk in `tool_waiting_confirm`) until the
   `ConfirmationHandler` answers. A refusal records the denial in
   `turn.tool_results` and the turn finishes; the default handler denies. The
   caller's `ToolHandler::schemas` is the advertised catalog, and each result is
   appended to the model transcript for another bounded round (four rounds and
   eight tool steps per turn). Each call is retained in `TurnOutcome.tool_calls`
5. `AllChunksDone` and `Finalized` complete the turn

With no client the executor produces `Echo: <query>`, so session/driver tests do
not need a provider. A model or tool failure marks the turn `failed` and returns
the outcome with the error; state-machine violations return `Err`.

## Voice

`VoiceTier` reports the negotiated tier; the voice region's state drives
`Session::voice_tier` and carries a `VoiceTurnConfig` on the plan.

`VoiceCapabilities` describes what a deployment can run. `VoiceRegion::negotiate`
starts the region if needed, enters the requested tier when the deployment
supports it, and otherwise enters the best-supported tier and returns
`VoiceNegotiation { degraded: true }`. With no supported tier the region stays in
`negotiating` and returns `CmeError::Voice`, leaving the caller free to fall back
to text.

| Tier | Behaviour | Latency ceiling |
|---|---|---|
| Tier 1 | ASR → responder → TTS, sequential | 1500 ms |
| Tier 2 | respond on the first final while input continues; barge-in stops playback | 700 ms |
| Tier 3 | delegate to a `RealtimeClient` | 300 ms |

`voice::audio` defines `AudioInput`, `AudioOutput`, `Vad`, `AsrClient`,
`TtsClient` and `RealtimeClient` plus deterministic mocks (`MockAudioInput`,
`MockAsr`, `MockTts`, `MockRealtimeClient`, `EnergyVad`).
`VoicePipeline::run_tier1/2/3` run the tiers over those traits. Prosody maps
`mm-being` affect and response modality to rate, pitch, volume, warmth, pauses
and detail, then clamps to synthesizer-safe ranges.

## Memory

`MemoryBridge` is the engine's handle on `mm_data::MemorySystem`; the memory
region's state machine records where the pipeline is, while the records live in
the data layer.

- `start_session` / `resume_session` adopt the memory session id and apply the
  resume context: greeting, project narrative, previous-session topics and the
  user's learned preferences (with planner hooks applied).
- `log_turn` writes an `EpisodicTurn` with the query, response, detail level,
  model, applied preferences, affect and any proactive items.
- `after_turn` logs, then once `compression_trigger_turns` is reached dispatches
  `compression_triggered` → `compress_session` → `compression_done`, aggregates
  the project (`auto_aggregate`) and optionally syncs user memory
  (`auto_sync_user`). The region ends in `segmented`, `project_synced` or
  `user_synced`; the machine can compress again from each.
- `search`, `presence`, `broadcast_turn` and `handoff_modality` expose the vector
  index and the multi-device surface.

`SessionDriver::with_memory(bridge)` opens the memory session at startup, logs
and compresses each successful turn, and writes the final snapshot on shutdown.
A memory failure is logged and the conversation continues.

```rust
use std::sync::Arc;
use mm_cme::{MemoryBridge, MemoryBridgeConfig, SessionDriver};

let bridge = Arc::new(MemoryBridge::new(
    data.memory_system.clone(),
    MemoryBridgeConfig { compression_trigger_turns: 10, ..Default::default() },
));
let (driver, messages, outcomes) = SessionDriver::spawn(session, tick);
let handle = tokio::spawn(driver.with_memory(bridge).run());
```

## Preferences

`PreferenceLearner` owns one posterior per `PreferenceDimension`. `observe`
folds in a weighted signal; a posterior becomes `Stable` after 10 observations or
when its variance falls to the threshold. A stable posterior contradicted by a
signal more than three standard deviations away enters `DriftDetected` and
relearns. `apply_to` writes applicable values into `PreferenceProfile`, and
`PlannerHooks` derives the keys the planner reads (`detail` from technical depth
and verbosity, `format` from formality) without overwriting explicit values.
`persist` saves the learned posteriors into the user's Tier 4 memory, and
`apply_to_session` updates a live session.

## Plugins and proactive work

`PluginHost` loads compiled `Plugin` implementations after validating their
manifest (identifier, name, at least one trigger, the `propose` capability). It
registers triggers, caps proposals per plugin and per cycle, and caps executions
per cycle. Six built-ins ship behind `BuiltIn`: search lookup, goal narrowing,
plan refinement, pattern watching, affect response and knowledge gaps. Their
`evaluate` methods read the query, preferences, affect and memory hits; `execute`
returns a `PluginResult`.

`ProactiveEngine::run` starts a cycle from `idle` (resetting `completed` or
`suppressed`), evaluates the host, applies `max_items` and `min_score`, executes
the winners within the sandbox budget, injects `ProactiveItem`s into
`Session::proactive_items` (read with `drain_proactive`) and finishes the region
in `completed`. Repeated titles are suppressed for `repeat_window` cycles, and a
plugin failure injects what succeeded and still completes the cycle.

`SessionDriver::with_proactive(engine)` runs one cycle per tick using the most
recent query. The driver's tick still advances the region when no engine is
attached.

## Observability

`Observability` is the transport-neutral Phase 6 surface. Phase 7's HTTP layer
mounts it at `GET /api/cme/sessions/{id}/snapshot` and
`WS /api/cme/sessions/{id}/events`.

- `Observability::snapshot(&session)` returns the full `mm-sm` snapshot
  (including all six orthogonal regions and application data), the active turn
  and its chunks, plus typed proactive, voice, memory, preference, plugin and
  improvement views. It is the handler behind `GET .../snapshot`; a bare
  `SessionSnapshot::capture(&session)` is available when no metric refresh is
  wanted.
- `Observability::subscribe()` returns a `broadcast::Receiver<SmTransitionEvent>`.
  Every transition carries `timestamp`, `sm_id`, `sm_type`, `from`, `to`, `event`
  and composition `depth`. The session, its regions, the turn and every chunk
  publish to it.
- `Observability::prometheus()` renders `cme_sm_transitions_total`,
  `cme_sm_state_duration_ms`, `cme_sm_active_count`, `cme_session_active`,
  `cme_turn_chunks` and `cme_preference_value`.
- Transitions are also emitted as `tracing::debug!` events with structured
  fields, and `mm-sm` wraps each instance and transition in spans, so an
  OpenTelemetry subscriber sees the machine hierarchy.

`SessionDriver::with_observability(hub)` installs the observer on the session
and executor, takes snapshots at startup, after each turn and at shutdown, and
streams every dispatch. A bare `Session` can also carry an observer directly
(`Session.observer`).

## Improvement, reports and dashboards

`Report` is a title, audience and ordered modality-aware sections. It exports
Markdown, JSON and plain text, exposes clarity, brevity, actionability,
completeness, source coverage, consistency, tone, safety and correctness metrics,
and implements `Optimizable` with mutations for modality, detail, trimming,
sources, action items and ordering.

`Dashboard` is a grid of panels with refresh costs and a viewing context. Its
neighbors add thresholds, reduce cost, compact secondary panels, promote critical
panels and switch to a single column.

`HillClimbingEngine` (the `hill_climb` module) is a generic steepest-ascent
optimizer: it evaluates every neighbor, accepts only improvements above
`min_improvement`, and returns the best artifact plus its score trajectory.

`ImprovementLoop::run` drives the region through
`reflect → plan → act → verify → learn`:

1. Critics turn weak metrics into `Issue`s.
2. The plan summarises the issues; governance can stop for human approval.
3. The hill climber produces a candidate.
4. Six verifiers check regression, consistency, completeness, composite score,
   human approval and simulated cost. A failure dispatches
   `rollback_triggered → rolling_back → rollback_done` and restores the baseline
   when `auto_rollback` is set.
5. A successful cycle records a `LearnedPattern` and completes the region.

`ImprovementEngine` owns a report and dashboard, runs both loops when its
`ImprovementScheduler` is due, and is attached to the driver with
`SessionDriver::with_improvement(engine)`, so improvement runs on the driver's
tick schedule.

## Remaining seams

- `ToolHandler` is the tool seam; `mm-serve` implements it over the full console
  catalog. `ConfirmationHandler` is the user gate for `ToolPolicy::Confirm`; the
  default `DenyUnconfirmed` fails closed until a UI or plugin answers.
- Plugins are in-process Rust trait objects. A WASM or native host implements
  the same `Plugin` trait; the sandbox contract (manifest validation, proposal
  and execution caps) does not change.
- The LLM substrate currently exposes `complete`, not a token stream, so the
  executor feeds the completed reply into the chunk stream. A streaming client
  replaces only that step; the Turn/Chunk transitions do not change.
- Speculative pre-warm enters and exits the state but the substrate has no warm
  endpoint yet.
- The voice pipeline is provider-neutral and mock-backed; microphone/WebRTC
  adapters and wiring audio into live turns are later phases.
- `MemorySystem` has no retention API, so the memory region's `purging` state is
  driven by explicit events rather than a scheduled sweep.
- REST and WebSocket transport is Phase 7; Phase 6 exposes the snapshot,
  subscription and Prometheus text without mounting routes.
- Report and dashboard construction is deterministic and metric-based. A
  model-backed generator or judge can replace the metric critics and verifiers
  behind the same traits.
- OpenTelemetry export depends on the installed `tracing` subscriber; `mm-sm`
  emits the spans and the observability hub emits the structured transition
  events.

## Design deviations

- `mm-sm` is runtime-typed, so each enum has an `as_str()` and events carry JSON
  payloads. This keeps the session's six regions and per-turn children in one
  homogeneous instance tree.
- The active turn is `Session.active_turn`, not an `mm-sm` child instance. It is
  spawned and dropped per turn and appears in `SessionSnapshot.turn` without
  changing the turn API.
- Chunk state starts at `buffering` when the directive is marked buffered; no
  separate "enter buffering" event exists.
- Session regions are runtime-typed `mm-sm` instances, so they cannot own
  `Arc<MemorySystem>` or the plugin host. `MemoryBridge`, `PreferenceLearner`,
  `PluginHost` and `ProactiveEngine` own the work; the region machines record
  the lifecycle around it.
- Proactive items are exposed on the session (`proactive_items` and
  `drain_proactive`) and summarized by `SessionSnapshot.proactive_queue_length`;
  the items themselves stay in the session so the transport can drain them.

## Verification

`crates/mm-cme/tests/engine.rs` covers each region's transition table, session
event forwarding, turn/chunk completion, dependency release, and a full driver
run: session init → turn → chunk → outcome → shutdown. `tests/phase4.rs` covers
deterministic planning, tool-policy selection, modality/voice routing, the
committed routing table, the executor with a mock LLM, allowed and
confirmation-gated tool calls, model failure, voice capability negotiation, mock
Tier 1 voice, prosody, and a driver turn at negotiated Tier 2. `tests/phase5.rs`
runs the real data layer: a turn compresses into a segment searchable by vector
and SPARQL, project memory and resume context carry forward, learned preferences
persist and move the planner, plugins load/evaluate/execute under the sandbox
caps, proactive items are injected, the driver schedules proactive ticks,
presence/broadcast/handoff work, and a driver with a memory bridge compresses a
completed turn. `tests/phase6.rs` captures full snapshots, streams session,
turn and chunk transitions, renders Prometheus metrics, improves reports and
dashboards, runs the improvement schedule through the driver, and proves a
verification failure rolls the artifact back. Unit tests in
`session.rs`, `turn.rs`, `chunk.rs`, `planner.rs`, `routing.rs`, `preferences.rs`,
`plugins.rs`, `observability.rs`, `hill_climb.rs`, `report.rs`, `dashboard.rs`,
`improvement.rs`, `voice/*` and `regions/voice.rs` cover lifecycle, speculative
routing, barge-in history restore, sampling, pinned routing, convergence,
drift, manifest validation, transition metering, snapshots, hill-climb
acceptance, report/dashboard metrics, critic and verifier behavior, capability
downgrade and all three tier pipelines.
