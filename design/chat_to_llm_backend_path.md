# Backend Path: Chat Request to the Model Call

> **Status:** reference design (as built) · **Date:** 2026-11-25 · **Scope:** `crates/mm-serve`, `crates/mm-cme`, `crates/mm-llm-agentstream`, `crates/mm-llm`
>
> This document traces one console chat request from the HTTP handler to the provider
> socket and back: which component owns each step, what is passed between them, and
> where the path can terminate. It describes the shipped code, not a proposal; the
> stream-termination contract it relies on is specified in
> [`terminating_loops_design.md`](terminating_loops_design.md), and the pass protocol in
> [`three_pass_turn_sequence.md`](three_pass_turn_sequence.md).

---

## 1. Summary

A chat request does not call a model directly. It admits a run, resolves a per-session
conversation engine, builds one turn's prompt inputs, and hands the turn to a
long-lived session driver. The driver runs a deterministic plan, chooses a model, and
runs three passes — **Interpret → Process → Verify** — with bounded retries. Each pass
renders its own prompt and calls the provider through the LLM substrate. Pass hooks
become SSE frames while the turn runs; the same frames are persisted against the run so
a reconnect replays them.

The path has seven layers:

```
HTTP route → CME runtime → session driver → turn executor
           → three-pass runner → agentstream gateway → provider transport
```

Every layer has one clear return value, and the response body ends on a terminal frame
rather than when the turn object happens to be dropped.

---

## 2. Entry points and path selection

| Route | Handler | Notes |
|---|---|---|
| `POST /api/chat` | `routes::chat::open` (`crates/mm-serve/src/routes/chat.rs`) | Delegates to `routes::cme::open` when `state.features.cme_enabled` (set by `MM_CME_ENABLED`). Otherwise runs the legacy `controller::reply` path, which is not covered here. |
| `POST /api/cme/chat` | `routes::cme::open` (`crates/mm-serve/src/routes/cme.rs:139`) | The conversation-engine route proper. |
| `GET /api/chat/{request_id}/stream?from=<seq>` | `routes::chat::stream` | Replays persisted frames from a run; used for resume. |
| `POST /api/chat/{request_id}/stop` | `routes::chat::stop` | Requests that a run stop. |

Admission (`routes::cme::open`) does the work that must happen before any streaming:

1. Rejects an empty query and invalid attachments.
2. Reuses `sessionId` if the session exists, else creates one; allocates `requestId`.
3. Records the run with `state.chat.begin(request_id, session_id, query)`.
4. Ensures conversation state exists in `state_service` for `CONSOLE_USER`.
5. Ensures the automation timer exists (when the automation feature is on) and marks
   every status section dirty.
6. Loads the conversation bundle and derives `PromptVariables::from_bundle`, including
   the `Standard` budget snapshot and the console user projection.
7. Selects the prompt variant `"pending"` when the bundle carries `next` or `blockers`.
8. Builds a per-request `TurnExecutor` from `state.cme_executor()`, then attaches the
   prompt variables and variant.

If `state.three_pass.turn.stream` is false, the route awaits the turn and returns the
whole frame list as one SSE body. The streaming path (the default, `[turn] stream = true`
in `config/three_pass.toml`) is described below.

---

## 3. Sequence

```
client                mm-serve route              CME runtime              session driver
  │ POST /api/chat           │                          │                         │
  ├─────────────────────────►│ begin run, build vars    │                         │
  │                          ├─────────────────────────►│ run_turn(session,query) │
  │                          │                          ├────────────────────────►│ StartTurnWith
  │                          │                          │                         │
  │                          │                          │        ┌────────────────┴───────────────┐
  │                          │                          │        │ TurnExecutor::execute          │
  │                          │                          │        │  plan → route → 3 passes       │
  │                          │                          │        └────────────────┬───────────────┘
  │   SSE frames (live)      │                          │                         │
  │◄─────────────────────────┼──────────────────────────┼─────────────────────────┤ PassObserver
  │                          │                          │◄──── TurnOutcome ───────┤ outcomes channel
  │                          │◄──── TurnOutcome ────────┤                         │
  │   terminal frames        │                          │                         │
  │◄─────────────────────────┤ finish_stream, close     │                         │
```

Inside `TurnExecutor::execute`:

```
plan (deterministic) → route (model) → Interpret
                                          │
                        fast path ────────┴──► finalize
                                          │ action
                                          ▼
                                  Process (model/tool loop)
                                          │
                                          ▼
                                  Verify (checks + staged writes)
                                     │ approved        │ rejected
                                     ▼                 ▼
                              commit journal      retry Process
                                     │                 (≤ max_retries)
                                     ▼
                                  finalize
```

---

## 4. Stage detail

### 4.1 Live stream and spawn (`routes/cme.rs:213-291`)

The route creates:

- `ChatStream::new(state, request_id)` — the frame writer and `PassObserver`
  (`crates/mm-serve/src/cme_stream.rs`), returning the receiver for SSE.
- Two opening frames: `Started { query }` and `Run { request_id, session_id, query }`.
- `CloseChatStream(Arc<ChatStream>)` — a Drop guard so the response closes on panic.
- A `CallContext::new("chat")` carrying `session`, `run`, `trigger = "user_turn"`,
  `origin = "cme.turn"`, wrapped by `mm_llm::scope`.

The turn runs in `tokio::spawn`; the HTTP handler returns immediately with
`Sse::new(unfold(...))`. Every `ChatStream::emit` appends the frame to the durable run
via `chat.append_frame_live`, which returns the run's sequence number; that number is
the SSE `id`, so `?from=<seq>` resumes without duplicates.

`ChatStream` is both producer and observer:

- `PassObserver` hooks (`pass_started`, `round_started`, `delta`, `tool_call`,
  `tool_result`, `check`, `retry_scheduled`, `pass_finished`, `round_finished`) map to
  `PassStart`, `PassRound`, `PassToken`, `ToolCall`, `ToolResult`, `Verification`,
  `PassRetry`, `PassComplete`, `PassRoundEnd`.
- Control calls (`metamind.approve`, `metamind.verify`) are suppressed as tool cards;
  their outcome is carried by the pass frames.

### 4.2 Session runtime (`crates/mm-serve/src/cme.rs`)

`CmeRuntime::run_turn` is the only entry to a session:

1. `executor.turn_overrides()` captures the current turn's observer, prompt variables,
   and variant.
2. `ensure_session` creates, on first use, a `Session` bound to the request's
   `sessionId`, a `MemoryBridge`, an `Observability` hub, optional proactive and
   improvement engines, and a `SessionDriver` spawned over the mailbox and outcome
   channel. The driver's executor is the first turn's executor; per-turn inputs travel
   as `DriverMessage::StartTurnWith { query, overrides }`.
3. The per-session `outcomes` `AsyncMutex` is held across send-and-receive, serialising
   turns in one session while allowing different sessions to run concurrently.
4. `DriverMessage::StartTurnWith` sets the overrides on the executor and calls
   `start_turn`.

The outcome channel carries `TurnOutcome` back to the route. `ensure_session` uses a
weak reference in `SessionPromptProvider`, so the long-lived driver and executor do not
keep `AppState` alive.

### 4.3 Driver and turn lifecycle (`crates/mm-cme/src/driver.rs`)

`SessionDriver::start_turn`:

1. `run_turn(query)` — clones the session context, begins the turn machine, and calls
   `TurnExecutor::execute(turn, &context, voice)`.
2. `executor.clear_pass_observer()` on both success and error — a finished turn must not
   leave its observer (and therefore the chat stream) attached to the long-lived
   executor.
3. Sends the `TurnOutcome` to the route.
4. Dispatches `SessionEvent::TurnCompleted(outcome)` to the session state machine and
   publishes a snapshot.

Inside `run_turn`, on success, the driver's `MemoryBridge::after_turn` runs the memory
cycle (compression/aggregation) before the outcome is returned. A memory failure is
logged, not fatal to the turn.

The driver loop also owns a 15 s tick for proactive and improvement cycles; those are
session lifetime, not turn lifetime.

### 4.4 Plan and route (`crates/mm-cme/src/executor.rs:428-561`)

`TurnExecutor::execute`:

1. Attaches the turn observer and **takes** the per-turn `pass_observer` so it cannot
   leak into a later turn.
2. Builds a deterministic `TurnPlan` with `TurnPlanner::build_plan` — modality, chunk
   directives, detail level, tool policy, budget class. No model is consulted.
3. Selects the model with `ConversationRouter::choose`:
   - `ConversationRoutingRequest::text(query, plan.modality)` plus cost budget, voice
     tier (if any), proactive flag, and the session preference profile.
   - The router is `ThresholdRouter::from_specs(config.models, default_model)` over
     `config/llm.toml`, with the pinned table from `config/routing_table.toml`.
   - `decision.model` is stored on the turn (`turn.route_selected`) and passed to every
     pass as `ctx.model`.
   - With no router the model is `"stub-model"`; the deterministic path is used when no
     client is configured.
4. Enters `LlmStreamStart`, spawns the turn's chunks, and dispatches either the
   three-pass flow or the legacy single `ProcessPass` (`cm.turn.v1`).

`state.cme_executor()` (`crates/mm-serve/src/state.rs:678`) is where the pieces are
assembled: `resolve_client` decides between `Off`, `Scripted`, and `FromEnv`; a missing
client or empty model list yields `TurnExecutor::deterministic()`, whose reply is
`Echo: <query>`. This is why tests and offline sessions work without a provider.

### 4.5 Interpret (`executor.rs:562-650`, `runner.rs:600-1110`)

`execute_three_pass` loads the clean log from agentstream (user messages + approved
final replies only), takes the turn's prompt variables, and calls
`ThreePassRunner::run_interpret`.

Interpret is a loop over rounds, `0..ledger.limits().rounds.min(MAX_PASS_ROUNDS)` with
`MAX_PASS_ROUNDS = 12` (`runner.rs:47`). Each round:

- Refreshes the prompt context from the session bundle (`refresh_context`).
- Renders `cm.interpret.v1` into a `PromptInstance`; records prompt id and hash.
- Builds `LlmRequest::new(Purpose::Interpret, messages)` with `CacheMode::Use`, the
  Interpret-scoped tools (`PassScope::Interpret`), pass-level temperature 0.2, and
  `max_tokens = 2048`; then applies `ctx.model`.
- Reserves tokens and calls `client.complete_stream` inside
  `mm_llm::scope(pass_context_with(...))`, so the call is attributed to pass/round/run.
- Streams deltas through `ObserverDeltaSink` → observer → `PassToken` frames.

The pass has three accepted outcomes; anything else is a protocol violation:

| Response | Result |
|---|---|
| `metamind.approve` as the only call | `PassFinish::FastPath`; reply is streamed and the turn finalizes without Process/Verify |
| `metamind.verify` / other scoped control calls | `PassFinish::Action` with `VerifyCheck[]` and a `ProcessContext` brief |
| `conversation.*` calls | Executed immediately, result fed back as history, next round |
| Text with no tool call | One repair round (`repair_text`); a second violation is `INTERPRET_PROTOCOL` |

Fast-path determinism matters: an approval with any other call is a protocol error, not
a partial success.

### 4.6 Process (`executor.rs:651-750`, `process.rs:168-339`)

The executor creates a `ProcessPass` per attempt with the Process budget class,
`budgeted` from `three_pass.budget.enabled`, and the turn's observer tagged with the
attempt number. `run_accounted` returns the reply, token totals, tool emissions,
mutations, and a budget snapshot.

Process is the model/tool loop:

1. Render `cm.process.v1` with the Interpret brief, clean log, retry context (if any),
   and Process-scoped tool schemas.
2. Call `client.complete_stream` under a scope that adds `pass = "process"`, the round,
   and a `tool_result` trigger when following up a tool call.
3. If the response has no tool calls, that text is the reply.
4. If tools are requested but `plan.tool_policy == None`, fail the turn.
5. Otherwise enforce **`MAX_TOOL_ROUNDS = 32`** and **`MAX_TOOL_STEPS = 32`**
   (`process.rs:29-31`), truncate the batch to the remaining steps, reserve steps from
   the budget, and dispatch each call through `ToolHandler::call_for_session`.
6. Append `assistant_calls` and the tool results to the transcript and take another
   round.

Caps are unconditional; budgeting adds token/wall-clock refusal but is never the only
exit. An empty final reply is `PROCESS_INCOMPLETE` when `require_reply` is set.

### 4.7 Verify and retry (`executor.rs:750-889`, `runner.rs:1112-1420`)

`PassContext::verify` receives the Interpret checks, the Process result, prior
verification, retry context, prior effects, attempt number, and the current envelope.
`run_verify` runs under the same `MAX_PASS_ROUNDS = 12` ceiling (min with the pass
ledger's allowed rounds) using `cm.verify.v1` and the Verify tool scope.

Verify may run `llm_judge` checks (bounded by `verify.check_timeout_ms`, 20 s in
`config/three_pass.toml`) and stage mutating calls in a `MutationJournal` via
`call_staged_for_session`. Its outcome is one of:

- **approved** — if the journal is empty the commit is a no-op; otherwise
  `ThreePassRunner::commit` replays it through `commit_staged_for_session` and returns
  `CommittedMutation`s. The executor records the pass and returns
  `ThreePassFlow::Approved(reply)`.
- **rejected** — a `RetryContext` is built from failed checks (label, reasoning,
  confidence, prior reply digest, prior effects), a retry is reserved from the envelope,
  and the loop returns to Process with the next attempt. The loop is bounded by
  `verify.max_retries` (5 in `config/three_pass.toml`), so at most six Process+Verify
  attempts run. Exhaustion is a failed turn with the accumulated verification attached.
- **error / other** — the turn fails with the verification and staged writes recorded,
  and any staged writes are discarded (never committed).

The same loop uses `TurnEnvelope`: with budgeting enabled, retries and tokens come from
the envelope; with budgeting off, `TurnEnvelope::unlimited(max_retries)` still bounds
retries. Child passes absorb their spend into the turn envelope.

### 4.8 Finalize (`executor.rs:520-560`)

On approval (or fast path) the executor:

- splits the reply across the plan's text chunks (`stream_text`),
- flushes structured chunks with a JSON payload derived from the reply,
- flushes audio chunks (no-op in a text turn),
- calls `turn.finalize()`, producing `TurnOutcome` (text, model, chunks, tool calls,
  pass history, staged/committed writes, prompt instance/hash/variant, verification).

---

## 5. Model transport layers

The three-pass runner holds an `Arc<dyn LlmClient>`. In the console that client is
`AgentstreamLlm`, even in scripted mode: `resolve_client` injects the scripted client
into the gateway so the gateway's journaling and live feed still see the calls.

```
ThreePassRunner / ProcessPass
        │  complete_stream(&LlmRequest, &mut ObserverDeltaSink)
        ▼
AgentstreamLlm::call_stream            (crates/mm-llm-agentstream/src/lib.rs:507)
        │  execute → at most one reasoning-only escalation
        ▼
AgentstreamLlm::execute_once           (lib.rs:556)
        │  journal request → publish live Request → pool profile → service call
        ▼
LlmService::call_stream                (crates/mm-llm/src/service.rs)
        │  accounting facade: cache, decode, schema validation/repair
        ▼
OpenAiClient::complete_stream          (crates/mm-llm/src/openai.rs:1085)
        │  body with stream=true, tool calls and usage accumulation
        ▼
TcpTransport::post_sse                 (openai.rs)
        │  idle bound: config/llm.toml `idle_timeout_ms = 300000`
        ▼
provider endpoint
```

Key behaviours:

- **Provider selection.** `ProviderPool::profile` reads `req.provider` (default
  `"default"`); the profile supplies endpoint/key env vars, model defaults, timeout,
  idle timeout, structured-output and thinking policy. The console's injected scripted
  client bypasses endpoint resolution.
- **Request normalization.** `normalize_request` fills the provider and model from the
  profile and applies the profile's body policy.
- **Reasoning-only retry.** If a round returns empty text with a `length` finish reason
  because all tokens went to reasoning, `execute` journals the failed round, doubles the
  budget (bounded by `MM_LLM_REASONING_RETRY_MAX_TOKENS`, default 8192), and runs one
  retry under a new call id with `trigger = "budget_retry"` and `parent_call_id`.
  `MM_LLM_REASONING_RETRY=off` makes the failure terminal.
- **Streaming vs structured.** Structured output never streams: `OpenAiClient` falls
  back to a normal call, so the sink receives no deltas. A proxy that ignores `stream`
  is recovered with one ordinary call that reports one delta.
- **Delta path.** `ObserverDeltaSink` forwards tokens to the pass observer; `ChatStream`
  turns them into `PassToken` frames and also appends them to the run, so live and
  replay views agree.

---

## 6. Return path

1. `TurnOutcome` travels from the executor to the driver, is sent on the session's
   outcome channel, and is dispatched to the session state machine.
2. `CmeRuntime::run_turn` returns it to the route's spawned task.
3. On error, the task emits `Failed`, calls `finish_stream(request_id, "failed")`, and
   closes the stream.
4. On success:
   - `settle_after_turn` resumes the automation frame timer (when enabled) and marks
     status sections dirty.
   - `terminal_frames` emits only what was not already live: non-text `Block`s, `Focus`
     documents referenced by tool results, `LlmResponse`, `FinalResponse`, and
     `Completed { subsystems }`.
   - `finish_stream(request_id, "completed" | "failed")` persists the run's state.
   - `ChatStream::close()` drops the sender.
   - `log_turn_frames` writes the prompt record, user message, assistant message, every
     frame, verification registrations, retries, and staged/committed/discarded writes
     to agentstream (when `features.agentstream_logging` is on).
5. The SSE `unfold` terminates on `Completed` or `Failed`, independent of whether the
   sender was closed, and the body ends.

Persistence is per run, not per response: a client that disconnects mid-turn can
reconnect at `GET /api/chat/{request_id}/stream?from=<last_seq>` and receive the missed
frames, including the terminal one.

---

## 7. Ownership and concurrency

| Object | Lifetime | Owner |
|---|---|---|
| `TurnExecutor` | session (driver lifetime) | `SessionDriver` |
| `TurnOverrides` (observer, prompt vars, variant) | one turn | captured by `CmeRuntime::run_turn`, applied by `StartTurnWith` |
| `ChatStream` | one run, closed explicitly | route task, with `CloseChatStream` Drop guard |
| `SessionDriver` | session | spawned task, mailbox `DriverMessage` |
| Turn | one request | created by `driver.run_turn`, finished into `TurnOutcome` |
| `TurnOutcome` | one request | outcome channel, then route |
| `LlmRequest` | one model round | pass loop |

Turns in one session serialise on the outcome mutex; different sessions run
concurrently. The driver mailbox is unbounded, but only one turn per session can be in
flight, so the queue depth is bounded by clients issuing overlapping requests for the
same session.

---

## 8. Termination and failure semantics

Exit points, by stage:

| Stage | Failure / stop | Result |
|---|---|---|
| Admission | empty query, bad attachment, unknown session | `400` / `404`, no run |
| Runtime | driver channel closed | `AppError::Internal`, `Failed` frame |
| Planning / routing | planner or router error | turn failed via `executor.fail` |
| Interpret | protocol violation, model error, cap | `ThreePassFlow::Failed`, turn failed |
| Process | tool policy violation, round/step cap, `PROCESS_INCOMPLETE` | turn failed |
| Verify | rejected and retries exhausted, check error, commit failure | turn failed, staged writes discarded |
| Transport | profile timeout, idle timeout, reasoning-only (retry off) | `LlmError` surfaces as a failed pass |
| Stream | terminal frame, guard drop, explicit close | SSE body ends |

Invariants:

- Every model/tool loop has a hard ceiling independent of `budget.enabled`
  (`MAX_PASS_ROUNDS`, `MAX_TOOL_ROUNDS`, `MAX_TOOL_STEPS`, `verify.max_retries`).
- A failed turn never commits staged writes.
- Terminal frames appear exactly once per run; the terminal frame ends the response.
- A request that finishes always leaves the run in `completed` or `failed`, whether or
  not a client is still connected.

---

## 9. Observability along the path

| Signal | Produced at | Carries |
|---|---|---|
| `ChatFrame` stream | `ChatStream` hooks | start, pass/round, token, tool call/result, verification, retry, terminal |
| Durable run frames | `state.chat.append_frame_live` / `finish_stream` | sequence numbers, resumable replay |
| `CallContext` scope | route spawn, then per pass and per round | session, run, trigger, origin, pass, round, tool |
| Agentstream journal | `AgentstreamLlm::execute_once` | request, response/error, attempt, usage, latency |
| Live LLM feed | `AgentstreamLlm` live events | request, response, error, partial chars, reasoning chars, finish reason |
| Session events | `Observability` hub | state-machine transitions per session |
| Status sections | `open` and `settle_after_turn` | dirty markers for the console inspector |

`mm_llm::scope` is what makes the journal and the activity feed attributable: without
it the gateway still answers, but the call cannot be tied to a session, run, or pass.

---

## 10. Deliberate exclusions

- **Legacy controller path.** `routes::chat::open` with `cme_enabled = false` calls
  `controller::reply` and `controller::turn`, a separate loop with its own hard caps
  (`HARD_MODEL_ROUNDS`, `HARD_TOOL_STEPS`). It is not part of the three-pass path.
- **Proactive, voice, memory, preference, plugin, improvement regions.** The driver
  ticks them, but they do not participate in a chat turn's model call except through
  session context and prompt variables.
- **Recursive Think pass.** When `three_pass.think.three_pass` is enabled, `CmTools`
  exposes a thinking sub-runner fixed to `PassScope::Think`; it is a tool call from
  Process, not a fourth pass of the main sequence.
- **Process engine (`process_v1`).** `with_process` lets tools start process instances;
  the turn path treats those as ordinary tool calls.
- **Provider failover.** `ProviderPool` selects one profile per request. There is no
  cross-provider retry in this path; a failed profile returns its typed error.

---

## 11. Where to change what

| Change | File |
|---|---|
| Admission, frame persistence, terminal frames | `crates/mm-serve/src/routes/cme.rs` |
| Frame taxonomy and observer mapping | `crates/mm-serve/src/cme_stream.rs`, `dto/chat.rs` |
| Session creation, driver options, memory bridge | `crates/mm-serve/src/cme.rs` |
| Executor wiring, client resolution, tool catalog | `crates/mm-serve/src/state.rs`, `controller/mod.rs` |
| Plan, route, pass sequencing, commit | `crates/mm-cme/src/executor.rs` |
| Interpret/Verify prompts, rounds, checks | `crates/mm-cme/src/runner.rs` |
| Process prompt, tool loop, caps | `crates/mm-cme/src/process.rs` |
| Prompt rendering and bundle inputs | `crates/mm-prompt`, `crates/mm-cme/src/prompt.rs` |
| Provider profiles, journal, live feed | `crates/mm-llm-agentstream/src/lib.rs`, `pool.rs` |
| Transport, request body, SSE parsing | `crates/mm-llm/src/openai.rs` |
| Accounting, cache, schema validation | `crates/mm-llm/src/service.rs` |

---

## 12. Verification notes

The path is exercised without a live provider by injecting `Scripted`/`MockClient`
through `resolve_client`; the gateway still journals and emits live events, so frame
order, pass records, and terminal behaviour are testable end to end. The stream
termination tests (`crates/mm-serve/tests/three_pass_stream_terminates.rs`,
`agent_stream_terminates.rs`) and the loop-cap tests (`crates/mm-cme/tests/phase2_runner.rs`,
`phase3_process.rs`, `phase4_verify.rs`) cover the exit contract in §8.

Not covered by scripted tests: real provider latency and retry behaviour, profile
timeouts against a slow endpoint, and the live console rendering of the frame stream.
Those require a configured provider (`OPENAI_BASE_URL` / `OPENAI_API_KEY`) and a manual
run.
