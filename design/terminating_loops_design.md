# Terminating Loops: Stream Closure and Bounded Work

> **Status:** design + implementation · **Date:** 2026-11-25 · **Scope:** `crates/mm-serve`, `crates/mm-cme`, `frontend/src/lib/api`
> **Trigger:** a normal three-pass chat request leaves `mm-serve` holding the response body open forever.
> **Phases:** exactly two — Phase 1 and Phase 2. There is no Phase 0; see §5.

---

## 1. Executive summary

The hang is not a CPU spin. The turn completes and every frame, including `completed`,
is written. The response body stays open because the SSE stream ends only when every
`mpsc::UnboundedSender` behind it drops, and the long-lived session `ThreePassRunner`
keeps the turn's `ChatStream` (and its `Arc<AppState>`) alive after the turn ends. The
browser then blocks in `reader.read()` with nothing left to read.

Two defects share one shape: **a loop whose exit condition is owned by someone else**.

- Streams exit when a sender drops — but the sender belongs to a component that outlives the work.
- Model/tool loops exit when a budget refuses work — but the budget is explicitly disabled in `config/three_pass.toml`, so the limits are `u32::MAX`.

This design fixes both in two phases:

1. **Phase 1 — Terminate every stream that has a finish line.** The producer closes its
   channel on every exit path, the SSE stream also stops on a terminal frame, and the
   browser stops reading at a terminal frame. A finished turn cannot leave a body open.
2. **Phase 2 — Bound every model/tool loop.** Hard round and step ceilings apply
   regardless of `budget.enabled`; budgets remain a cost policy, not the only reason a
   loop stops.

The phases are independent: Phase 1 alone removes the reported hang; Phase 2 alone
removes the unbounded-work risk. Neither depends on a preparatory phase.

---

## 2. Inventory: loops that can wait forever

### 2.1 Defects in scope

| # | Site | Exit condition today | Why it does not exit |
|---|---|---|---|
| L1 | `crates/mm-serve/src/routes/cme.rs` chat SSE `stream::unfold` over `ChatStream`'s receiver | all senders drop | the session `ThreePassRunner` retains `ChatStream` after the turn; `unfold` never sees `None` |
| L2 | `crates/mm-serve/src/cme_stream.rs` `ChatStream` | sender drops when the last `Arc` drops | `Arc` outlives the request by design; no explicit close |
| L3 | `frontend/src/lib/api/chat.ts` `openChatStream` `for (;;) reader.read()` | body EOF | the body never ends (L1); the client has no frame-level exit |
| L4 | `crates/mm-serve/src/routes/console.rs::agent_stream` | never (poll `loop`) | polls a completed run every 500 ms forever |
| L5 | `crates/mm-cme/src/runner.rs` `run_interpret_inner` / `run_verify_inner` `for _round in 0..ledger.limits().rounds` | round counter reaches limit | with budgeting off the limit is `u32::MAX` |
| L6 | `crates/mm-cme/src/process.rs` tool loop | round/step cap **when `budgeted`** | caps are skipped entirely when budgeting is off; `remaining` becomes unbounded |
| L7 | `crates/mm-serve/src/controller/turn.rs` controller loop | caps **when `budgeted`** | same gating; a model that always returns tool calls never reaches the summary round |

### 2.2 Long-lived streams that are correct as-is

These are tails with no finish line. They end when the client disconnects or the
process-wide channel closes, which is correct; they are not changed.

- `GET /api/cme/sessions/{id}/events` (state transitions)
- `GET /api/console/sessions/{id}/status/stream` (status revisions)
- `GET /api/llm-activity/stream` (activity tail)
- `GET /api/agentstream/sessions/{id}/stream` (durable log tail; a session may be appended to again)
- `SessionDriver` message loop and the process supervisor tick in `state.rs` (process lifetime, not turn lifetime)

`GET /api/agents/{run_id}/stream` (L4) is **not** in this group: a run has a terminal
status, so its stream has a finish line and must close.

---

## 3. Exit contract

Four invariants. Every change in both phases enforces one of them.

1. **A stream with a finish line ends at its terminal frame.** For the chat stream the
   terminal frame is `completed` or `failed`; for a run stream it is the run reaching
   `done`, `failed`, or `cancelled` after its remaining events have been sent.
2. **Closing is explicit, idempotent, and runs on every exit path** — success, error,
   cancellation, and panic (Drop guard). It does not rely on reference counting.
3. **A consumer stops at the terminal frame even if the body stays open.** The client is
   never the only thing standing between a finished turn and a released connection.
4. **Every model/tool loop has a hard ceiling independent of configuration.** Budgets
   may stop work earlier; they are never the only stop.

Terminal frames remain exactly once per run. Phase 1 must not introduce a second
`failed`/`completed` frame or truncate frames that were already appended before the
terminal frame.

---

## 4. Phase 1 — Terminate streams with a finish line

### 4.1 Producer: explicit close (`cme_stream.rs`)

`ChatStream` owns the sender behind a `Mutex<Option<…>>`:

- `emit` still appends the frame to the durable run first, then sends only if the
  channel is still open (a closed stream must not resurrect itself).
- `close()` takes the sender out of the option and drops it. Idempotent; safe from any
  thread; never blocks on I/O.
- `Drop for ChatStream` calls `close()` as a local backstop.

A `CloseStream` guard in the spawned turn task holds an `Arc<ChatStream>` and closes it
when the task ends. Because it is a `Drop` guard it also runs when `run_turn` panics, so
no exit path can leave the channel open.

### 4.2 SSE: end on the terminal frame (`routes/cme.rs`)

The `unfold` state becomes `(receiver, done)`. It yields frames as before, sets
`done = true` after yielding a terminal frame, and returns `None` on the next poll. This
is defense in depth: even if some future observer retains the sender, the body ends.

The existing ordering is unchanged: `started` and `run` are emitted before the turn
spawns; pass/tool frames stream live; `final_response`/`completed` or `failed` are last.

### 4.3 Client: stop at the terminal frame (`frontend/src/lib/api/chat.ts`)

`openChatStream` cancels the reader and returns as soon as it dispatches a `completed`
or `failed` frame. It no longer depends on the server closing the body before the UI can
settle; a body that stays open costs nothing.

### 4.4 Run streams: close after the terminal status (`routes/console.rs`)

`agent_stream` keeps its 500 ms poll while the run is live. When the poll finds no new
events and the run's status `is_terminal()`, it drains once more and returns `None`.
A missing run record is also terminal. The durable-log tail
(`agentstream_stream`) stays open by design (§2.2).

### 4.5 Release the turn's observer (`mm-cme`)

`SessionDriver::start_turn` calls `TurnExecutor::clear_pass_observer()` after every turn
(including errors) so the session runner drops the turn's `ChatStream` instead of holding
it until the next turn. This is the memory-side half of the same fix: with `close()` the
stream ends, and with the clear the object is released.

### 4.6 Phase 1 tests

- `ChatStream::close()` makes the receiver return `None` after draining.
- The SSE stream ends immediately after a terminal frame even when the sender is still
  retained (the regression probe for the reported hang).
- `openChatStream` resolves on `completed` with a body that never closes.
- `agent_stream` ends after a run reaches `done`.
- A driver turn leaves no observer installed on the runner.

---

## 5. Phase 2 — Bound every model/tool loop

### 5.1 Pass loops (`crates/mm-cme/src/runner.rs`)

`MAX_PASS_ROUNDS: u32 = 12` is a code-level ceiling. Interpret and Verify iterate
`0..min(ledger.limits().rounds, MAX_PASS_ROUNDS)`. A budgeted turn keeps its smaller
class limit; an unbudgeted turn stops after 12 model calls. Reaching the ceiling is
reported through the existing `PassFinish` error path, so the outcome stays honest.

### 5.2 Process tool loop (`crates/mm-cme/src/process.rs`)

`MAX_TOOL_ROUNDS` (32) and `MAX_TOOL_STEPS` (32) become unconditional:

- The round and step checks run whether or not `budgeted`.
- The per-response tool batch is always truncated to the remaining step allowance, so a
  single response with hundreds of calls cannot exceed the cap.

Budgeting still adds token and wall-clock refusal; it no longer controls whether the
loop can terminate.

### 5.3 Controller loop (`crates/mm-serve/src/controller/turn.rs`)

Add `HARD_MODEL_ROUNDS = 32` and `HARD_TOOL_STEPS = 64` in `controller/limits.rs`.
The summarize trigger and the step-denial check use `MAX_*` when budgeting is on and
`HARD_*` when it is off. Because `summarizing` removes tools and ends the loop on the
next response, the loop has a finite bound in both configurations. The existing
"budget off keeps going past six steps" behavior is preserved (7 < 64); only the absence
of any ceiling is removed.

### 5.4 Phase 2 tests

- Interpret/Verify stop at `MAX_PASS_ROUNDS` when the envelope is unlimited.
- Process stops at the tool step cap with budgeting disabled and truncates an oversized
  batch.
- The controller stops at `HARD_TOOL_STEPS`/`HARD_MODEL_ROUNDS` with budgeting disabled.
- Existing budgeted behavior is unchanged (same limits, same errors).

---

## 6. Why there is no Phase 0

A Phase 0 would be inventory/plumbing that ships no behavior. The inventory is §2 and
the two fixes are independently shippable: Phase 1 is the live-hang fix and touches only
stream ownership, while Phase 2 is a pure loop-bound change. Adding a Phase 0 would
delay the hang fix without reducing either phase's risk. Numbering starts at Phase 1.

---

## 7. Acceptance criteria

- [ ] A chat turn with `[turn] stream = true` closes its SSE body within one poll after
      `completed`, even when the session driver is still alive.
- [ ] A failed turn closes the body the same way; exactly one terminal frame is sent.
- [ ] The browser settles the turn on `completed`/`failed` without waiting for EOF.
- [ ] A completed agent run's stream closes instead of polling forever.
- [ ] With `[budget] enabled = false`, no model/tool loop can exceed its hard ceiling.
- [ ] With budgeting enabled, limits and errors are unchanged.
- [ ] `cargo test -p mm-cme -p mm-serve`, frontend chat/stream tests, `tsc`, fmt and
      clippy pass.

---

## 8. Traceability

| Defect | Phase | Change | Test |
|---|---|---|---|
| L1, L2 | 1 | `ChatStream::close`, `CloseStream` guard, terminal-aware `unfold` | stream ends after `completed` with sender retained |
| L3 | 1 | `openChatStream` terminal-frame exit | reader resolves on open body |
| L4 | 1 | `agent_stream` terminal-status exit | run stream ends after `done` |
| observer leak | 1 | `clear_pass_observer()` in `start_turn` | runner observer is `Noop` after a turn |
| L5 | 2 | `MAX_PASS_ROUNDS` | unlimited envelope stops at 12 |
| L6 | 2 | unconditional tool caps + truncation | budget-off step cap |
| L7 | 2 | `HARD_MODEL_ROUNDS`, `HARD_TOOL_STEPS` | budget-off controller cap |
