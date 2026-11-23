# Three-Pass Turn Sequence for the Conversation Manager

> **Status:** proposed design · **Date:** 2026-11-23 · **Scope:** `mm-cme`, `mm-serve`, `mm-data`, `mm-prompt`
>
> This document defines a restructuring of the conversation manager's core turn execution from a monolithic loop into three explicit, testable passes. It builds on the architecture in `conversation_manager_agent_inspector_plan.md` (§10.3) and the existing `TurnExecutor` in `mm-cme`.

---

## 1. Motivation

The current `TurnExecutor::execute()` (≈170 lines) performs all turn work in a single async function:

1. Build `TurnPlan` via `TurnPlanner`
2. Route to a model via `ConversationRouter`
3. (Optional) speculative pre-warm
4. Enter the model/tool loop: render prompt → call model → run tools → reload state → re-render → check budget
5. Pre-final checks (pending work, active agent runs, index health)
6. Stream final text
7. Persist prompt instance, turn outcome, state/session/frame/data/plan/action revisions
8. Emit status events

**Problems with the monolithic structure:**

| Problem | Impact |
|---------|--------|
| **No clear phase boundaries** | Hard to test planning vs. execution vs. commit independently; bugs in one phase leak into another |
| **State reloads implicit** | `refresh_variables()` called mid-loop but not visibly tied to a phase; observers can't distinguish "pre-tool" vs "post-tool" state |
| **Budget enforcement scattered** | Reservations happen before model calls, settlements after, caps checked in the loop — no single budget accounting point |
| **Prompt instance lifecycle unclear** | New `PromptInstance` rendered each round but not explicitly versioned per pass; audit trail is implicit |
| **Error recovery opaque** | A tool failure, budget exhaustion, or prompt failure all exit through `fail()` without a structured phase result |
| **Observer granularity** | `SmObserver` sees state-machine transitions but not "pass complete" boundaries; inspector cannot show pass-level progress |
| **Concurrency model fragile** | The entire turn holds the session's outcome mutex; a long model call blocks the session from barge-in or status queries |

**The three-pass design makes each phase:**

- **Explicit** — a named function with a typed input/output
- **Testable** — each pass can be unit-tested with mocked dependencies
- **Observable** — each pass emits a `PassEvent` the inspector can display
- **Recoverable** — a failure in Pass 2 can return a structured `Pass2Result::Paused` that Pass 3 commits honestly
- **Budget-accountable** — Pass 1 reserves the budget; Pass 2 spends it; Pass 3 settles and reports

---

## 2. Pass Definitions

### Pass 1: OBSERVE — Plan & Prepare

**Input:** `TurnExecutor`, `Turn` (in `Planning` state), `SessionContext`, optional `VoiceTurnConfig`

**Output:** `ObserveResult`

```rust
pub struct ObserveResult {
    pub turn_plan: TurnPlan,
    pub state_view: StateView,
    pub prompt_instance: PromptInstance,
    pub model: String,
    pub budget_ledger: BudgetLedger,
    pub tool_schemas: Vec<ToolSchema>,
}
```

**Steps:**

1. **Planning** — Call `TurnPlanner::build_plan()` with the user query and `SessionContext`. Produces `TurnPlan` with modality, detail level, chunks, tool policy, budget class, and metadata.
2. **StateView assembly** — Load the complete `StateView` from stores:
   - Conversation state (topic, depth, status variables, revision)
   - Session state (lifecycle, presence, work catalog, frame stack, expiry)
   - Current user (canonical IRI, preferences, workspace, relationship)
   - Active agent runs (summaries, last turn results)
   - Data health (revision, entity count, index lag, schemas)
   - Plan registry (active plan header, monitor state)
   - Action service (scheduled/running/cron counts)
   - Memory service (counts, last recall, project sync)
   - Budget ledger snapshot (class, rounds/steps/tokens/wall remaining)
   - Interjection preferences (delivery policy, quiet hours, rate state)
3. **Prompt rendering** — `PromptManager` selects `cm.turn.v1` (variant `standard`/`pending`/`resume`/`repair` per planner) and renders a `PromptInstance` with the `StateView` as the `[STATE]` system message.
4. **Model routing** — `ConversationRouter::choose()` selects the model; record in `TurnPlan`.
5. **Budget reservation** — `BudgetLedger::reserve_initial()` for the selected budget class (rounds, steps, tokens, wall time).
6. **Tool schemas** — Collect exactly 17 schemas from the `ToolHandler` (conversation 5 + data 7 + metamind 5).
7. **Turn state advance** — `turn.plan_ready()` → `turn.route_selected()`.

**Failure modes:**

| Error | Handling |
|-------|----------|
| Planner error | `ObserveError::Planning` — turn fails immediately |
| State store unavailable | `ObserveError::StateUnavailable(reason)` — render `[STATE unavailable: reason]` block, continue |
| Prompt unavailable | `ObserveError::PromptUnavailable(reason)` — turn fails closed (`PROMPT_UNAVAILABLE`) |
| No model available | `ObserveError::NoModel` — use deterministic executor path |
| Budget reservation denied | `ObserveError::BudgetExhausted` — turn fails with `BUDGET` |

**Observability:** Emits `PassEvent::ObserveComplete { turn_id, plan, model, budget_class, prompt_hash }`.

---

### Pass 2: DELIBERATE — Execute & Iterate

**Input:** `ObserveResult`, mutable `Turn` (in `Streaming` state), `TurnExecutor` (with `LlmClient`, `ToolHandler`, `ConfirmationHandler`)

**Output:** `DeliberateResult`

```rust
pub enum DeliberateResult {
    Completed {
        final_text: String,
        tool_calls: Vec<ToolCallEmission>,
        prompt_instances: Vec<PromptInstance>, // one per round
        budget_ledger: BudgetLedger,
    },
    Paused {
        reason: PauseReason,
        partial_text: String,
        tool_calls: Vec<ToolCallEmission>,
        prompt_instances: Vec<PromptInstance>,
        budget_ledger: BudgetLedger,
    },
    Failed(String),
}

pub enum PauseReason {
    BudgetExhausted,
    MaxToolRoundsReached,
    MaxToolStepsReached,
    AgentRunStillActive,
    BlockingPendingWork,
    IndexPending,
}
```

**Steps:**

1. **Enter streaming** — `turn.dispatch(TurnEvent::LlmStreamStart)` → `turn.spawn_chunks()`.
2. **Loop** (while budget allows):
   a. **Reserve round** — `budget_ledger.reserve_round()`.
   b. **Refresh variables** — `refresh_variables()` reloads `StateView` from stores (critical for tool-side writes).
   c. **Re-render prompt** — `PromptManager::render()` with refreshed variables → new `PromptInstance`.
   d. **Update turn prompt** — `turn.set_prompt()` with new instance.
   e. **Call model** — `LlmClient::complete()` with messages + 17 tool schemas.
   f. **Settle tokens** — `budget_ledger.settle_tokens(provider_usage)`.
   g. **If no tool calls** — break with `final_text = response.text`.
   h. **If tool calls** — `run_tool_calls()`:
      - Validate strict JSON; unknown fields → `INVALID_ARGUMENTS`
      - For each call: `confirm()` if `ToolPolicy::Confirm`, then `tool_handler.call_for_session()`
      - Append `ToolCallEmission` to turn; build `Message::tool()` results
      - `budget_ledger.reserve_steps()` per call
      - Check per-family quotas (data, metamind, agent)
   i. **Append to transcript** — typed envelope for each tool result.
   j. **Check caps** — `MAX_TOOL_ROUNDS` (32), `MAX_TOOL_STEPS` (32), budget rounds/steps/tokens/wall.
3. **Post-loop pre-commit checks** (honesty rules):
   - If an `agent.run` started this turn is still running → `PauseReason::AgentRunStillActive`
   - If blocking `pending` work remains and no tool was called this round → force one extra round (max 2)
   - If a data write returned `indexPending` → `PauseReason::IndexPending`
4. **Return result** — `Completed` if loop exited naturally; `Paused` with reason if checks triggered.

**Failure modes:**

| Error | Handling |
|-------|----------|
| Model call error | `DeliberateResult::Failed(detail)` |
| Tool validation error | Record `INVALID_ARGUMENTS` emission, continue loop |
| Tool execution error | Record failed emission with error text, continue loop |
| Confirmation denied | Record denied emission, continue loop |
| Budget cap hit | Return `Paused { reason: BudgetExhausted }` |
| Prompt render error | Return `Failed("PROMPT_UNAVAILABLE: ...")` |

**Observability:** Emits `PassEvent::DeliberateRound { turn_id, round, model, tokens_used, tool_calls_count, budget_remaining }` per round; `PassEvent::DeliberateComplete { turn_id, outcome: Completed|Paused, rounds, total_tokens }` on exit.

---

### Pass 3: COMMIT — Finalize & Persist

**Input:** `DeliberateResult`, mutable `Turn`, `SessionContext`, `AppState` (for persistence services)

**Output:** `CommitResult` (the final `TurnOutcome` returned to the caller)

```rust
pub struct CommitResult {
    pub outcome: TurnOutcome,
    pub persisted_revisions: PersistedRevisions,
}

pub struct PersistedRevisions {
    pub conversation_state_rev: u64,
    pub session_rev: u64,
    pub frame_rev: u64,
    pub data_revision: Option<u64>,
    pub plan_revision: Option<u64>,
    pub action_revision: Option<u64>,
    pub prompt_instance_ids: Vec<String>,
}
```

**Steps:**

1. **Final text handling**
   - If `DeliberateResult::Completed { final_text }` → stream via `stream_text()` + `flush_structured()` + `flush_audio()`.
   - If `DeliberateResult::Paused { partial_text, reason }` → stream `partial_text` + honest pause notice (e.g., *"I've used my budget for this turn. Remaining: [pending items]. Shall I continue?"*).
   - If `DeliberateResult::Failed(detail)` → stream error notice.
2. **Finalize turn state machine** — `turn.finalize()` (dispatches `AllChunksDone` → `Finalized` → `Completed`).
3. **Persist all revisions** (transactional where possible):
   - Conversation state (topic, depth, status, `pending` projection) → `ConversationStore::save()` with expected revision
   - Session (lifecycle, presence, work catalog, frame stack) → `SessionStore::save()`
   - Frame stack → `FrameEngine::persist()`
   - Data writes already committed in Pass 2; record `data_revision`
   - Plan versions → `PlanStore::save()`
   - Action records → `ActionStore::save()`
   - Prompt instances → `PromptInstanceAudit::write()` (redacted, 0600)
   - Agentstream events for the turn (already logged during Pass 2; ensure completion)
4. **Emit status events** — `StatusService::mark_dirty()` for all affected sections; SSE broadcasts `ConversationStatusView` revision.
5. **Budget settlement** — Final `BudgetLedger` snapshot attached to `TurnOutcome`.
6. **Return `TurnOutcome`** — includes all chunks, tool calls, prompt instance IDs, model, budget snapshot.

**Failure modes:**

| Error | Handling |
|-------|----------|
| Persistence conflict (revision mismatch) | Retry once with fresh read; on second failure, `TurnOutcome::Failed` with `CONCURRENCY_CONFLICT` |
| Prompt audit write error | Log warning, continue (audit is best-effort) |
| Status broadcast error | Log warning, continue (SSE is best-effort) |

**Observability:** Emits `PassEvent::CommitComplete { turn_id, outcome_state, revisions, prompt_instances_count }`.

---

## 3. TurnExecutor Restructuring

### 3.1 New Public API

```rust
impl TurnExecutor {
    /// Execute Pass 1: OBSERVE
    pub async fn pass1_observe(
        &mut self,
        turn: &mut Turn,
        session: &SessionContext,
        voice: Option<VoiceTurnConfig>,
    ) -> Result<ObserveResult>;

    /// Execute Pass 2: DELIBERATE
    pub async fn pass2_deliberate(
        &mut self,
        observe: ObserveResult,
        turn: &mut Turn,
    ) -> Result<DeliberateResult>;

    /// Execute Pass 3: COMMIT
    pub async fn pass3_commit(
        &mut self,
        deliberate: DeliberateResult,
        turn: &mut Turn,
        session: &SessionContext,
        state: &Arc<AppState>, // for persistence services
    ) -> Result<CommitResult>;

    /// Convenience: run all three passes in sequence (preserves existing execute() behavior)
    pub async fn execute(
        &mut self,
        turn: &mut Turn,
        session: &SessionContext,
        voice: Option<VoiceTurnConfig>,
        state: &Arc<AppState>,
    ) -> Result<TurnOutcome> {
        let observe = self.pass1_observe(turn, session, voice).await?;
        let deliberate = self.pass2_deliberate(observe, turn).await?;
        let commit = self.pass3_commit(deliberate, turn, session, state).await?;
        Ok(commit.outcome)
    }
}
```

### 3.2 Internal Method Changes

| Current Method | New Location | Notes |
|----------------|--------------|-------|
| `planner.build_plan()` | `pass1_observe` | Unchanged |
| `router.choose()` | `pass1_observe` | Unchanged |
| `refresh_variables()` | `pass2_deliberate` (loop top) | Called every round |
| `render_prompt()` | `pass1_observe` (initial) + `pass2_deliberate` (each round) | Now returns `PromptInstance` explicitly |
| `llm_request()` / `client.complete()` | `pass2_deliberate` | Unchanged |
| `run_tool_calls()` | `pass2_deliberate` | Unchanged |
| `stream_text()` / `flush_structured()` / `flush_audio()` | `pass3_commit` | Unchanged |
| `turn.finalize()` | `pass3_commit` | Unchanged |
| Persistence (agentstream, state, session, etc.) | `pass3_commit` | Moved from `CmeRuntime::run_turn` / `log_turn_frames` |

### 3.3 State Machine Integration

The turn state machine (`turn_machine()`) already has the right states. The passes map to state transitions:

| Pass | Entry State | Exit State | Key Transitions |
|------|-------------|------------|-----------------|
| OBSERVE | `Planning` | `Streaming` | `PlanReady` → `RouteSelected` → (optional `SpeculativeWarm`) → `LlmStreamStart` → `spawn_chunks()` |
| DELIBERATE | `Streaming` | `Streaming` / `Finalizing` | Loop: `ToolCall` ↔ `ToolResult` → `AllChunksDone` |
| COMMIT | `Finalizing` | `Completed` | `Finalized` |

**No state machine changes required.** The passes are an execution-layer decomposition; the SM remains the source of truth for legality.

---

## 4. Budget Accounting per Pass

| Pass | Budget Action |
|------|---------------|
| OBSERVE | `reserve_initial(class)` — reserves rounds, steps, tokens, wall time for the whole turn |
| DELIBERATE | `reserve_round()` before each model call; `settle_tokens(usage)` after; `reserve_steps(n)` per tool call; check caps |
| COMMIT | `snapshot()` — attach final ledger to `TurnOutcome`; no new reservations |

**Key invariant:** Pass 1 reserves the *entire* turn budget up front. Pass 2 spends against it. Pass 3 only reports. This prevents mid-turn budget top-ups and makes "budget exhausted" a single, well-defined transition.

---

## 5. Prompt Instance Versioning

Each pass produces prompt instances that are explicitly tracked:

| Pass | Instances Produced | Variant |
|------|-------------------|---------|
| OBSERVE | 1 (initial) | `standard` / `pending` / `resume` / `repair` |
| DELIBERATE | 1 per model round | Same as initial (re-rendered with refreshed state) |
| COMMIT | 0 | — |

All instances are collected in `DeliberateResult.prompt_instances` and persisted in `CommitResult.persisted_revisions.prompt_instance_ids`. The agentstream `PromptBuilt` event is emitted for each.

---

## 6. Error Handling & Recovery

### 6.1 Structured Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum Pass1Error {
    #[error("planning failed: {0}")]
    Planning(String),
    #[error("state unavailable: {0}")]
    StateUnavailable(String),
    #[error("prompt unavailable: {0}")]
    PromptUnavailable(String),
    #[error("no model available")]
    NoModel,
    #[error("budget exhausted: {0}")]
    BudgetExhausted(String),
}

#[derive(Debug, thiserror::Error)]
pub enum Pass2Error {
    #[error("model call failed: {0}")]
    ModelCall(String),
    #[error("prompt unavailable: {0}")]
    PromptUnavailable(String),
    #[error("budget cap: {0}")]
    BudgetCap(String),
}

#[derive(Debug, thiserror::Error)]
pub enum Pass3Error {
    #[error("persistence conflict: {0}")]
    ConcurrencyConflict(String),
    #[error("prompt audit write failed: {0}")]
    AuditWrite(String),
}
```

### 6.2 Recovery Strategies

| Failure Point | Recovery |
|---------------|----------|
| Pass 1 (Observe) | Turn fails immediately; `TurnOutcome::Failed` with error; no model called |
| Pass 2 (Deliberate) model error | Return `DeliberateResult::Failed`; Pass 3 streams error, persists nothing |
| Pass 2 (Deliberate) budget cap | Return `DeliberateResult::Paused { reason: BudgetExhausted }`; Pass 3 commits honest pause |
| Pass 2 (Deliberate) tool error | Record emission, continue loop (per-tool failure ≠ turn failure) |
| Pass 3 (Commit) persistence conflict | Retry once with fresh read; on second failure, `TurnOutcome::Failed` with `CONCURRENCY_CONFLICT` |
| Pass 3 (Commit) audit/SSE error | Log warning, continue (best-effort) |

---

## 7. Observability & Inspector Integration

### 7.1 Pass Events

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PassEvent {
    ObserveComplete {
        turn_id: TurnId,
        plan: TurnPlan,
        model: String,
        budget_class: BudgetClass,
        prompt_hash: String,
    },
    DeliberateRound {
        turn_id: TurnId,
        round: usize,
        model: String,
        tokens_used: u32,
        tool_calls_count: usize,
        budget_remaining: BudgetSnapshot,
    },
    DeliberateComplete {
        turn_id: TurnId,
        outcome: DeliberateOutcome, // Completed | Paused(PauseReason) | Failed
        rounds: usize,
        total_tokens: u32,
    },
    CommitComplete {
        turn_id: TurnId,
        outcome_state: String, // "completed" | "paused" | "failed"
        revisions: PersistedRevisions,
        prompt_instances_count: usize,
    },
}
```

### 7.2 Inspector Status View Extensions

Add to `ConversationView` (§11.1 of main plan):

```rust
pub struct ConversationView {
    // ... existing fields ...
    pub pass: Option<PassView>, // current/last pass
}

pub struct PassView {
    pub name: String,           // "observe" | "deliberate" | "commit"
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub rounds: Option<usize>,  // for deliberate
    pub pause_reason: Option<String>,
}
```

The SSE stream emits a `PassEvent` on each pass boundary; the inspector shows a pass timeline.

---

## 8. Testing Strategy

### 8.1 Unit Tests per Pass

| Pass | Test Cases |
|------|------------|
| OBSERVE | • Planner produces correct plan for each modality<br>• StateView assembles all projections<br>• PromptInstance hash stable for same inputs<br>• Model routing respects cost budget<br>• Budget reservation matches class table<br>• Prompt unavailable → fail closed |
| DELIBERATE | • Single round, no tools → Completed<br>• Multiple tool rounds → Continues until model answers<br>• Tool validation error → INVALID_ARGUMENTS emission, loop continues<br>• Tool execution error → failed emission, loop continues<br>• Confirmation denied → denied emission, loop continues<br>• Budget round cap → Paused(BudgetExhausted)<br>• Budget step cap → Paused(BudgetExhausted)<br>• Budget token cap → Paused(BudgetExhausted)<br>• Budget wall cap → Paused(BudgetExhausted)<br>• Agent run still active → Paused(AgentRunStillActive)<br>• Blocking pending work, no tool called → extra round (max 2)<br>• Index pending → Paused(IndexPending)<br>• Prompt render error → Failed |
| COMMIT | • Completed → streams text, persists all revisions, emits status<br>• Paused → streams partial + notice, persists revisions, emits status<br>• Failed → streams error, persists nothing<br>• Revision conflict → retry once, then Failed(CONCURRENCY_CONFLICT)<br>• Prompt audit write error → logs warning, continues |

### 8.2 Integration Tests

- **Full three-pass flow** with deterministic executor (no model) — verifies pass boundaries, state transitions, persistence.
- **Full three-pass flow** with stub model that returns tool calls — verifies loop, budget, tool execution, prompt re-rendering.
- **Barge-in during Pass 2** — verifies `Turn::barge_in()` interrupts loop, Pass 3 commits interrupted state.
- **Concurrent turns on same session** — verifies outcome mutex serializes, revision conflicts handled.

### 8.3 Property Tests

- **Prompt instance hash determinism** — same inputs → same hash across passes.
- **Budget monotonicity** — rounds/steps/tokens/wall only decrease across Pass 2 rounds.
- **State revision monotonicity** — conversation/session/frame revisions only increase across Pass 3 commits.

---

## 9. Migration Plan

This is a **refactoring** of `TurnExecutor::execute()` — no external contract changes. The existing `execute()` method becomes a thin wrapper calling the three passes in sequence.

### 9.1 Phase 1: Extract Pass Methods (no behavior change)

1. Extract `pass1_observe()` from lines 267–312 of `executor.rs` (planning through `LlmStreamStart` + `spawn_chunks`).
2. Extract `pass2_deliberate()` from lines 314–415 (the model/tool loop).
3. Extract `pass3_commit()` from lines 417–432 (streaming, finalize, return outcome).
4. Make `execute()` call the three passes sequentially.
5. All existing tests pass.

### 9.2 Phase 2: Add Structured Results & Error Types

1. Define `ObserveResult`, `DeliberateResult`, `CommitResult`, `PassEvent`.
2. Update pass signatures to return structured results.
3. Emit `PassEvent` at each boundary (via observer).
4. Add unit tests for each pass (see §8.1).

### 9.3 Phase 3: Persistence & Status Integration

1. Move persistence logic from `CmeRuntime::log_turn_frames()` and `StateService` into `pass3_commit()`.
2. Ensure `ConversationStore`, `SessionStore`, `FrameEngine`, `PlanStore`, `ActionStore`, `PromptInstanceAudit` are called from Pass 3.
3. `StatusService::mark_dirty()` called from Pass 3 with all affected sections.
4. Integration tests (§8.2) pass.

### 9.4 Phase 4: Inspector Pass Timeline

1. Add `PassView` to `ConversationStatusView`.
2. Frontend inspector renders pass timeline.
3. Manual verification: watch a multi-round turn show Observe → Deliberate (round 1) → Deliberate (round 2) → Commit.

---

## 10. Affected Files

| File | Change |
|------|--------|
| `crates/mm-cme/src/executor.rs` | Core restructuring: three pass methods, new result types, `execute()` wrapper |
| `crates/mm-cme/src/turn.rs` | No SM changes; `Turn` may gain `pass` field for observability |
| `crates/mm-cme/src/budget.rs` | `BudgetLedger::reserve_initial()`, `snapshot()` |
| `crates/mm-cme/src/lib.rs` | Re-export new pass types |
| `crates/mm-serve/src/cme.rs` | `CmeRuntime::run_turn()` calls `executor.execute()` (unchanged); `log_turn_frames()` logic moves to Pass 3 |
| `crates/mm-serve/src/state.rs` | `AppState::cme_executor()` unchanged; persistence services accessible for Pass 3 |
| `crates/mm-serve/src/status_service.rs` | `mark_dirty()` called from Pass 3 with full section list |
| `crates/mm-prompt/src/lib.rs` | `PromptManager::render()` used by Pass 1 + Pass 2 (no change) |
| `crates/mm-data/src/conversation.rs` | `ConversationStore::save()` with expected revision (Pass 3) |
| `crates/mm-data/src/session.rs` | `SessionStore::save()` (Pass 3) |
| `crates/mm-data/src/frame.rs` | `FrameEngine::persist()` (Pass 3) |
| `crates/mm-data/src/plan.rs` | `PlanStore::save()` (Pass 3) |
| `crates/mm-data/src/action.rs` | `ActionStore::save()` (Pass 3) |
| `crates/mm-data/src/prompt_audit.rs` (new) | `PromptInstanceAudit::write()` (Pass 3) |
| `frontend/src/components/inspector/` | Pass timeline display |

---

## 11. Rollback Plan

If issues arise:

1. **Phase 1 only** — The three-pass structure is internal; `execute()` signature unchanged. Revert to monolithic `execute()` by inlining the three methods.
2. **Phase 2+** — Feature flag `MM_THREE_PASS_TURN` (default `false`) guards the new path. When `false`, `execute()` uses the old monolithic code (kept as `execute_legacy()`). When `true`, uses three-pass.

```rust
// In executor.rs
const THREE_PASS_ENABLED: bool = cfg!(feature = "three_pass_turn"); // or env var

pub async fn execute(...) -> Result<TurnOutcome> {
    if THREE_PASS_ENABLED {
        self.execute_three_pass(...).await
    } else {
        self.execute_legacy(...).await
    }
}
```

---

## 12. Open Questions

1. **Should Pass 1 include speculative pre-warm?** Currently yes (it's part of routing). If pre-warm becomes a real model call, it may belong in Pass 2.
2. **Should `agent.run` delegation be a separate pass?** Currently it's a tool call within Pass 2. If delegation grows complex (multi-turn agent sessions), a Pass 2.5 "Delegate" might emerge.
3. **Is the `PauseReason::IndexPending` check in Pass 2 or Pass 3?** Currently Pass 2 (pre-commit checks). Moving to Pass 3 would make it a commit-time gate.
4. **Should `PromptInstanceAudit` be a separate crate or part of `mm-prompt`?** Currently `mm-data` owns agentstream; prompt audit is a new concern.

---

## 13. References

- `conversation_manager_agent_inspector_plan.md` §10.3 (turn algorithm), §10.4 (budgets), §11.1 (status view)
- `crates/mm-cme/src/executor.rs` (current `TurnExecutor::execute`)
- `crates/mm-cme/src/turn.rs` (turn state machine)
- `crates/mm-cme/src/budget.rs` (`BudgetLedger`)
- `crates/mm-prompt/src/lib.rs` (`PromptManager`, `PromptInstance`)
- `crates/mm-serve/src/cme.rs` (`CmeRuntime::run_turn`, `log_turn_frames`)
- `crates/mm-serve/src/state_service.rs` (`StateService`)
- `crates/mm-serve/src/status_service.rs` (`StatusService`)