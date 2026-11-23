# Three-Pass Chat UI: Live Phase Streaming, Tool Cards, and One-Turn Rendering

> **Status:** proposed implementation plan · **Date:** 2026-11-23 · **Baseline:** `integration/cme-rdf-backend`
> **Scope:** `frontend/src/agentgui`, `frontend/src/lib`, `crates/mm-serve`, `crates/mm-cme`, `crates/mm-llm`, `config/`
> **Flags:** `MM_CME_ENABLED` and `MM_THREE_PASS_V2` (both default `false` today)
> **Related:** [`three_pass_turn_sequence.md`](./three_pass_turn_sequence.md) (normative pass semantics), [`five_phase_turn_process_plan.md`](./five_phase_turn_process_plan.md) (execution plan, §8), [`conversation_manager_agent_inspector_plan.md`](./conversation_manager_agent_inspector_plan.md) §W (console/inspector)

---

## 1. Executive summary

The three-pass turn already runs and records itself. The chat UI does not show it, for four separate reasons (all verified in §2): the path is flag-gated off, the server builds pass frames only after the turn finishes, the browser transport reads the whole response body before dispatching anything, and the chat event model has no pass or verification part.

This plan wires the **live** three-pass turn into the agent chat feed without changing the turn semantics or the one-reply-per-turn contract:

- **One turn is still one assistant message.** Pass data is additive content inside that message, above the final reply.
- **Every pass is visible while it runs**: Interpret, Process, and Verify each get a collapsible phase block with its **own streaming prose, per-LLM-call rounds, tool calls and results, and verification checks**.
- **The reply does not move.** The final reply is still a single `text` part rendered from exactly one terminal frame; phase prose never renders as the answer.
- **Nothing regresses when the flags are off.** The legacy path keeps today's `thinking`/`trace` parts and rendering.

**Vocabulary.** The backend calls these *passes*; the UI labels them *phases* (`Interpret`, `Process`, `Verify`). “Three-pass” and “three-phase” name the same thing in this document.

The work splits into four seams, each independently testable:

1. `mm-llm` gains an opt-in streaming completion with a non-streaming default.
2. `mm-cme` gains a no-op-by-default `PassObserver` that reports pass, round, delta, tool, and check events at the moment they happen.
3. `mm-serve` turns those observations into numbered chat frames, appends them to the run store as they arrive, and streams them instead of returning one buffered SSE body.
4. `agentgui` consumes frames incrementally, models them as `pass` parts on the existing message, and renders each phase with the existing reasoning/trace visual language.

---

## 2. Baseline: what actually exists

### 2.1 Execution path

| Fact | Where |
|---|---|
| `/api/chat` routes to CME only when `MM_CME_ENABLED` is on | `crates/mm-serve/src/routes/chat.rs:96` |
| CME builds the three-pass runner only when `MM_THREE_PASS_V2` is on | `crates/mm-serve/src/state.rs:669` |
| Both flags default to `false` | `crates/mm-serve/src/feature_flags.rs:58,69` |
| Interpret/Process/Verify run through `ThreePassRunner`, one model/tool loop per pass | `crates/mm-cme/src/runner.rs:420+` |
| Each pass ends with `PassRecord { kind, attempt, prompt_hash, budget, outcome, verification, staged, committed, checks }` | `crates/mm-cme/src/turn.rs:270` |
| Tool emissions are flat on `TurnOutcome` with no pass attribution | `crates/mm-cme/src/turn.rs:224,313` |

### 2.2 Frame production today

`routes/cme.rs::open` awaits `run_turn(...)` to completion (`:209`), then builds:

- `started` + `run` (`frames_for`, `:269`),
- `pass_start` / `pass_complete` / `verification` from `outcome.pass_history` (`pass_frames`, `:441`),
- every tool call in `outcome.tool_calls` as `Phase::Act`, with `duration_ms: 0` (`:301`),
- one `llm_response` (`Phase::Final`) and one `final_response` (`:322`),
- `completed` or `failed`.

So the pass transcript is a post-hoc reconstruction. There is no live signal, no round identity, and no pass attribution on tool frames. `pass_frames` also emits `pass_token` only for `fast_path`, so Process/Verify prose never rides the chat stream at all today — only the final reply does.

### 2.3 Wire contract

`ChatFrame` already has the three-pass family (`crates/mm-serve/src/dto/chat.rs:83`):

| Frame | Fields today | Gap |
|---|---|---|
| `pass_start` | `pass, turn_id, attempt` | no model |
| `pass_token` | `pass, token, final` | no attempt, no round, no channel; only fast path uses it |
| `pass_complete` | `pass, finished, budget, promptHash` | **no attempt** — correlation relies on order |
| `verification` | `label, pass, confidence` | no attempt, backend, duration, reasoning |
| `tool_call` / `tool_result` | `id, turn_id, phase, name, input/output, success, durationMs` | `phase` is the legacy plan/act/verify enum; CME always sends `Act` |
| `llm_response` | `turn_id, phase, text, visible, is_final` | no pass, no round |

### 2.4 Chat UI stack

The live shell is `AppShell` → `PaneLayout` → **`@/agentgui/components/chat/chat-pane`** (`frontend/src/agentgui/components/shell/app-shell.tsx`). The separate `frontend/src/components/chat` transcript reducer is not mounted by that shell; this plan targets `agentgui` only and leaves the other stack alone.

Current transport (`frontend/src/agentgui/services/agent-transport.ts`):

- `ServerTransport.send` calls `openChat(...)`.
- `openChat` does `fetch("/api/chat")` → **`await res.text()`** → `parseSse` (`frontend/src/lib/api/chat.ts`). Nothing reaches the UI until the body is complete.
- `eventsForFrame` maps only `llm_response` (non-final, visible), `tool_call`, `tool_result`, `input_request`, `final_response`, `failed`, `completed`. **Every `pass_*` and `verification` frame falls through `default: []`.**
- `AgentEvent` (`frontend/src/agentgui/types/workspace.ts`) has no pass/verification variant, so the reducer has nowhere to put them.
- The `streaming` badge reflects request lifetime, not token arrival.

The inspector is the only place three-pass data is visible: `useTraceStream` + `buildPassTimeline` (`frontend/src/components/inspector/pass-timeline.tsx`) read agentstream pass events. That code is the reference for correlation rules, not the chat renderer.

### 2.5 Gap table

| User expectation | Today |
|---|---|
| One turn, one assistant message | ✅ already true, but only because pass data is absent |
| Streaming while a pass thinks | ❌ buffered twice (server + `res.text()`) |
| Tool calls/results per phase | ❌ dropped by the transport; no pass attribution on the wire |
| All LLM calls per phase | ❌ no round identity at all |
| Verification visible | ❌ frames exist, UI ignores them |
| Retry attempt visible | ❌ only in agentstream, inspector-only |
| Flag-off behavior unchanged | ✅ must stay this way |

---

## 3. Goals and non-goals

### 3.1 Goals

1. Enable the three-pass chat end to end behind `MM_CME_ENABLED` + `MM_THREE_PASS_V2`, with **live** per-pass progress.
2. Render every LLM call of a turn: its pass, attempt, round, streamed prose, tool calls, tool results, and outcome/usage.
3. Keep exactly **one assistant message per user turn**; pass material is attached to it, the final reply stays single and last.
4. Preserve the existing visual language (`Reasoning`, `ChainOfThought`, `Trace`, `Tool`) so pass views feel like richer thinking, not a second UI.
5. Remain additive: no `ChatFrame` field renames, no `wireVersion` bump, unknown frames ignored, legacy path untouched.
6. Make failures honest: a failed pass, a rejected verification, a budget stop, and a cancelled turn each have a distinct, non-fabricated state.

### 3.2 Non-goals

- Changing pass semantics, prompts, tool scopes, budgets, or verification policy.
- Exposing raw prompts, hidden reasoning, or provider chain-of-thought. Only model output that the server already marks `visible` is rendered.
- Replacing or deleting the inspector; the inspector remains the deep forensic view.
- Persisting chat transcripts across reloads (see §10; deferred, current chat is memory-only).
- Reworking the legacy `frontend/src/components/chat` stack.
- Voice/audio pass streaming.

---

## 4. Target architecture

### 4.1 One message, nested phases

```
assistant message (one per turn)
├─ pass part: interpret, attempt 1        ← collapsible headline
│   ├─ round 1  (llm call)  ─ streamed prose, tools
│   └─ round 2  (repair)    ─ only when the protocol needed it
├─ pass part: process, attempt 1
│   ├─ round 1  ─ streamed prose, tool call(s) + results
│   └─ round 2  ─ …
├─ retry divider (only when verify rejected)
├─ pass part: process, attempt 2
├─ pass part: verify, attempt 1
│   └─ checks (label, pass/fail, confidence, backend, duration)
├─ pass part: verify, attempt 2 (only if retried)
└─ text part: the final reply (single)
```

The final `text` part comes from exactly one source: `final_response`. `pass_token(final:true)` is a fallback used only if the run ends without `final_response` (fast path, truncated stream). This prevents the duplicate reply that would otherwise occur because fast-path tokens and `final_response` both carry the same text.

### 4.2 Live frame flow

```
ThreePassRunner ──PassObserver──► FrameSink (mm-serve)
   pass_start                          │  ChatStore::append_frames(frame)      ──► seq n
   round_start                         │                                      ──► broadcast
   delta × n                           │                                      ──► append
   tool_call / tool_result             │
   round_end (usage)                   │
   verification                        │
   pass_complete                       │
   pass_retry                          │
   final_response / completed          ▼
                                  SSE response (id: n, event: type, data: json)
                                          │
                    fetch + ReadableStream ─┘
                                          │
                     parse → dispatch → applyFrame → React (batched per frame)
```

Invariants:

1. Every frame is **appended to the run store before it is written to the stream**, so `GET /api/chat/{requestId}/stream?from=n` replays exactly what the client saw.
2. Sequence numbers are monotonic per run; the client dedupes by `id`.
3. A turn has one `run` frame, one terminal frame (`completed` or `failed`), and one final reply. Nothing else is terminal.
4. Pass frames of one attempt never interleave with another attempt's pass frames; retries are serialized.

### 4.3 Turn lifecycle (client)

```
created ──send──► streaming ──completed──► complete
                      │  ▲
                      │  └── reconnecting ──resume frames──┘
                      ├──failed──► error
                      ├──abort───► cancelled
                      └──pass retry stays streaming
```

The message `status` already owns these states; phase blocks derive from individual pass statuses.

---

## 5. Wire contract (additive, `wireVersion: 1`)

### 5.1 Frame changes

Legend: **E** = existing, extended with optional fields; **N** = new frame.

| Frame | Kind | Fields | Notes |
|---|---|---|---|
| `pass_start` | E | `pass, turn_id, attempt, model?` | `model` is the routed model for this pass |
| `pass_round` | **N** | `pass, turn_id, attempt, round, model?, non_streaming, started_at` | one per model call inside a pass; `round` is 1-based |
| `pass_token` | E | `pass, token, final, attempt?, round?, channel?, call_id?` | `channel: "note" \| "text"`; `text` only for the reply; `note` is phase prose |
| `pass_round_end` | **N** | `pass, turn_id, attempt, round, outcome, tokens_in, tokens_out, latency_ms, error?` | `outcome` = `ok \| protocol_error \| error \| exhausted \| cancelled` |
| `tool_call` | E | `…, pass?, attempt?, round?` | when present, the UI groups the card under that pass/round; `phase` stays for legacy clients |
| `tool_result` | E | `…, pass?, attempt?, round?` | same correlation |
| `verification` | E | `label, pass, confidence, attempt?, backend?, duration_ms?, reasoning?` | `attempt` defaults to the current verify attempt |
| `pass_retry` | **N** | `turn_id, pass, attempt, next_attempt, reason` | emitted after a rejected Verify, before the next Process `pass_start` |
| `pass_complete` | E | `…, attempt?` | must carry `attempt`; correlation by order becomes a fallback only |

`final_response` and `completed`/`failed` are unchanged. `llm_request_started`/`llm_response` remain for the legacy controller; the pass-aware UI ignores them when pass frames exist.

All pass frames accept an optional `parent_tool_id`. Absent means a top-level pass; present means a nested `metamind.think` sub-turn, rendered inside that tool card.

### 5.2 Example stream (abridged)

```json
{"type":"started","query":"audit the config"}
{"type":"run","requestId":"01J…","sessionId":"01J…","query":"audit the config"}
{"type":"pass_start","pass":"interpret","turn_id":"01J…","attempt":1,"model":"…"}
{"type":"pass_round","pass":"interpret","turn_id":"01J…","attempt":1,"round":1,"non_streaming":false,"started_at":1763…}
{"type":"pass_token","pass":"interpret","token":"Reading the request…","final":false,"attempt":1,"round":1,"channel":"note"}
{"type":"tool_call","id":"call_1","turn_id":"01J…","phase":"act","pass":"interpret","attempt":1,"round":1,"name":"metamind.verify","input":{…}}
{"type":"tool_result","id":"call_1","turn_id":"01J…","name":"metamind.verify","success":true,"output":{…},"durationMs":12,"pass":"interpret","attempt":1,"round":1}
{"type":"pass_round_end","pass":"interpret","turn_id":"01J…","attempt":1,"round":1,"outcome":"ok","tokens_in":812,"tokens_out":120,"latency_ms":2410}
{"type":"pass_complete","pass":"interpret","attempt":1,"finished":"action","budget":{…}}
{"type":"pass_start","pass":"process","turn_id":"01J…","attempt":1,"model":"…"}
{"type":"pass_round","pass":"process",…,"round":1}
{"type":"pass_token","pass":"process","token":"The config…","final":false,"channel":"note",…}
{"type":"tool_call","pass":"process","round":1,"name":"data.query",…}
{"type":"tool_result",…}
{"type":"pass_round_end","pass":"process",…,"round":1,"outcome":"ok",…}
{"type":"pass_complete","pass":"process","attempt":1,"finished":"replied","budget":{…}}
{"type":"pass_start","pass":"verify","turn_id":"01J…","attempt":1}
{"type":"verification","label":"action","pass":true,"confidence":0.86,"attempt":1,"backend":"llm_judge","duration_ms":740}
{"type":"pass_complete","pass":"verify","attempt":1,"finished":"approved","budget":{…}}
{"type":"final_response","response_id":"01J…","text":"…","verified":true}
{"type":"completed","subsystems":["cme","memory","tools"]}
```

### 5.3 Compatibility rules

- All additions are optional fields or new tagged variants; old clients ignore unknown fields and unknown `type`s.
- `phase` keeps its legacy meaning. It is never overloaded to mean `pass`.
- `pass_token.final` keeps its exact meaning: only the reply text. Phase prose always carries `final:false, channel:"note"`.
- A server that cannot stream still emits the same frames for the same turn, just batched. The UI cannot tell the difference except by arrival time.

---

## 6. Backend workstreams

### 6.1 `mm-llm`: opt-in streaming completion

Add to `LlmClient` (`crates/mm-llm/src/client.rs:399`):

```rust
async fn complete_stream(
    &self,
    req: &LlmRequest,
    sink: &mut dyn DeltaSink,
) -> Result<LlmResponse> {
    let response = self.complete(req).await?;
    sink.delta(Delta::text(&response.text));
    Ok(response)
}
```

- `DeltaSink` receives `text`, `usage`, and `error` deltas, with a monotonic index.
- `OpenAiClient` overrides it with `"stream": true`, parses `data:` SSE chunks, accumulates the final text/tool calls, and returns the same `LlmResponse` shape. Malformed chunks fail the call; partial text is already delivered and must be marked partial by the caller.
- `CachedClient`, `ReplayLlmClient`, and `MockClient` keep the default (one delta), so cached and test turns are correct, just chunkier.
- **Structured calls do not stream.** When `req.schema.is_some()`, use the default single-delta path; the frame marks `non_streaming: true`.
- Accounting stays in `LlmService`: settle at stream end; if the provider omits usage, estimate and mark `estimated`, never invent tokens.
- Cancellation: the runner's budget/wall clock aborts the stream; the returned error is `LlmError::Timeout`/`Cancelled`, and the round ends with `outcome:"cancelled"`.

### 6.2 `mm-cme`: `PassObserver`

Add a single trait, no-op by default so tests, CLI, and process steps are unchanged:

```rust
pub trait PassObserver: Send + Sync {
    fn pass_started(&self, pass: PassKind, attempt: u32, model: Option<&str>) {}
    fn round_started(&self, pass: PassKind, attempt: u32, round: u32, non_streaming: bool) {}
    fn delta(&self, pass: PassKind, attempt: u32, round: u32, delta: &str) {}
    fn tool_call(&self, pass: PassKind, attempt: u32, round: u32, emission: &ToolCallEmission) {}
    fn tool_result(&self, pass: PassKind, attempt: u32, round: u32, emission: &ToolCallEmission) {}
    fn round_finished(&self, pass: PassKind, attempt: u32, round: u32, outcome: RoundOutcome) {}
    fn check(&self, pass: PassKind, attempt: u32, result: &VerificationResult) {}
    fn retry_scheduled(&self, pass: PassKind, attempt: u32, reason: &str) {}
    fn pass_finished(&self, pass: PassKind, attempt: u32, outcome: &PassOutcome) {}
}
```

- `ThreePassRunner` gains `with_observer(Arc<dyn PassObserver>)`; `TurnExecutor` forwards it from `state.cme_executor()`.
- Emission points: before each `client.complete` / `complete_stream` (round start), on each delta, around every `tools.call_for_session` (tool call/result), after each round (usage/outcome), at the entry and exit of `run_interpret` / `run_process` / `run_verify` (pass start/finish), and where Verify schedules a retry.
- A protocol repair round is `round 2` with `outcome:"protocol_error"`; it is not a new pass attempt.
- The think runner is built in the same `state.cme_executor()`; attach the same observer to it and pass the invoking tool call id as `parent_tool_id`, so nested rounds render under `metamind.think` and never interleave with the parent pass.
- A Verify rejection → Process retry is a new **attempt**, announced by `retry_scheduled`.
- Tool emissions are attributed at the call site. `outcome.tool_calls` stays for compatibility and audit, but `frames_for` must not re-emit it when the observer already streamed those calls (see §6.3).

### 6.3 `mm-serve`: live frames, replay, and stop

1. **Sink**: implement `ChatFrameSink` over `Arc<ChatStore>` + `tokio::sync::broadcast`.
   - `emit(frame)` calls `append_frames(request_id, vec![frame], "processing")`, takes the returned `(seq, frame)`, and broadcasts the encoded SSE frame.
   - `finish(state)` appends the terminal frame and closes the stream.
2. **Handler**: `routes/cme.rs::open` returns an SSE stream immediately, then runs the turn in a spawned task.
   - Emit `started` + `run` before the task starts, so the client can reconnect from the first frame.
   - Spawn `run_turn` with the executor carrying the sink.
   - On completion emit `final_response` + `completed` (or `failed`) and finish.
   - `ChatStore::append_frames` already exists (`src/chat.rs:245`) and re-numbers deterministically; `frames_from` already supports `?from=`.
3. **No duplication**: when the observer is active, `frames_for` emits only the terminal/final frames. The post-hoc `pass_frames` and the flat `outcome.tool_calls` loop are used only when the observer is absent (legacy/fallback). This is a tested invariant, because duplicate tool cards and pass rows are the most likely visual regression.
4. **Fallback**: if streaming cannot be established (no body on the client, proxy buffering, config off), the same frames are collected and returned at the end exactly as today. The run store and replay path are identical.
5. **Stop/regenerate/answer**: `POST /api/chat/{id}/stop` sets the cancelled flag; the sink emits a final `failed` with `reason:"cancelled"` or a `pass_complete` with `finished:"cancelled"` first, so the UI can close open passes. `regenerate` creates a new run id; it does not append to the old one.
6. **Agentstream logging**: keep `log_turn_frames` post-turn for audit consistency. Chat frames are authoritative for the live UI; the inspector continues to read the recorded events. If live logging is added later, it must reuse the same sequence numbers.

### 6.4 Config

Add to `config/three_pass.toml` (not a new env flag):

```toml
[turn]
stream = true          # stream deltas when the provider supports it
frame_heartbeat_ms = 15000
```

`stream = false` gives the batched fallback with the exact same frame set, so a deployment can opt out without changing the UI contract.

---

## 7. Frontend transport

### 7.1 Incremental SSE

Replace the `openChat` body read with a stream reader in `frontend/src/lib/api/chat.ts`:

```ts
export type FrameHandler = (frame: SequencedFrame) => void;

export async function streamChat(
  body: OpenChatBody,
  onFrame: FrameHandler,
  options?: { signal?: AbortSignal; onRun?: (run: { requestId: string; sessionId: string }) => void },
): Promise<void>;
```

- `fetch(..., { signal })`, `response.body.getReader()`, `TextDecoder`, and a small line state machine that handles arbitrary chunk boundaries, multi-line `data:`, CRLF, comments (`: ping`), and `event:` names.
- Reuse `parseSse` for tests and non-streaming responses; extract the parser so both share one implementation.
- If `response.body` is null (test/happy-dom or old proxy), fall back to `await response.text()` and replay through the same parser.
- Keep `openChat` as a thin wrapper (`streamChat` + collect) for call sites that do not need live updates.

### 7.2 Reconnect, resume, dedupe

- On a transport error after `run` was received, call `resumeChat(requestId, lastSeq + 1)` up to 3 times with backoff, feeding frames through the same handler.
- The client keeps `lastSeq`; frames with `id <= lastSeq` are dropped before dispatch.
- A terminal frame stops the loop. `stop`/abort calls `stopChat(requestId)` best-effort and then marks the message cancelled.
- The transport reports the run id to the store so the stop button can act on it (today the store only has a session id).

### 7.3 Frame → event mapping

Extend `eventsForFrame` (`agentgui/services/agent-transport.ts`) with a pass-aware branch. Suggested mapping:

| Frame | AgentEvent |
|---|---|
| `pass_start` | `{type:"pass_start", pass, attempt, model, at}` |
| `pass_round` | `{type:"pass_round", pass, attempt, round, nonStreaming, startedAt}` |
| `pass_token` | `{type:"pass_delta", pass, attempt, round, delta, channel, final}` |
| `pass_round_end` | `{type:"pass_round_end", pass, attempt, round, outcome, usage, latencyMs, error?}` |
| `tool_call` / `tool_result` | the existing `tool` event plus `pass/attempt/round` when present |
| `verification` | `{type:"pass_check", pass, attempt, label, ok, confidence, backend?, durationMs?, reasoning?}` |
| `pass_retry` | `{type:"pass_retry", pass, attempt, nextAttempt, reason}` |
| `pass_complete` | `{type:"pass_complete", pass, attempt, finished, budget?, promptHash?}` |
| `final_response` | `{type:"text", delta, final:true}` (exactly once) |
| `failed` / `completed` | existing `error` / `done` |

`llm_request_started`/`llm_response` are ignored whenever any pass frame has been seen for the turn; they remain the legacy path otherwise.

### 7.4 Delta batching

Token frames are frequent. The transport pushes `pass_delta` into the store through a per-turn buffer flushed on `requestAnimationFrame` (or a 32 ms timer) with concatenated text. This keeps React commits bounded and preserves order. The buffer is flushed synchronously before any non-delta event and on terminal/cancel, so a tool card never appears ahead of the prose that introduced it.

---

## 8. Frontend model

### 8.1 Types

Add to `frontend/src/agentgui/types/workspace.ts` (additive; legacy parts untouched):

```ts
export type PassName = "interpret" | "process" | "verify" | "think";
export type PassPartStatus = "running" | "done" | "error" | "cancelled";

export interface PassCheck {
  label: string;
  ok: boolean;
  confidence: number;
  backend?: string;
  durationMs?: number;
  reasoning?: string;
}

export interface PassUsage {
  tokensIn: number;
  tokensOut: number;
  estimated?: boolean;
}

export interface PassRound {
  round: number;
  status: "running" | "done" | "error";
  text: string;
  tools: TraceStep[];
  startedAt: number;
  endedAt?: number;
  latencyMs?: number;
  usage?: PassUsage;
  nonStreaming: boolean;
  error?: string;
}

export interface PassPart {
  type: "pass";
  pass: PassName;
  attempt: number;
  status: PassPartStatus;
  startedAt: number;
  endedAt?: number;
  model?: string;
  finished?: string;
  promptHash?: string;
  checks: PassCheck[];
  rounds: PassRound[];
  retryReason?: string;
  updatedAt: number;
}

export type MessagePart = /* existing variants */ | PassPart;
```

`AgentEvent` gains the pass variants above. Tool events gain optional `pass`, `attempt`, `round`.

### 8.2 Reducer rules

`applyEvent` in `agentgui/lib/store/chat-store.ts` grows a pass branch with these rules:

1. **Correlate by `(pass, attempt)`**, never by array position. Create the pass part on first `pass_start`; update it on every later frame.
2. **Rounds correlate by `round`**. A missing round is an ordering error, not a reason to guess; start a `round` at the lowest index ≥ 1 that has no start frame and log nothing to the user.
3. **Text is append-only** within a round. Deltas with `final:false` go to round text; `channel:"text"`/`final:true` is used only for the terminal reply fallback.
4. **Tools attach inside the round** identified by the frame; if a tool frame has no pass/round (legacy), keep today's single `trace` part behavior.
5. **Checks attach to the verify pass** by `attempt`; a repeated label replaces the previous confidence (the latest attempt wins).
6. **Only one pass is `running` at a time**; a new `pass_start` marks the previous pass `done` only if it has no `pass_complete` (guards against a lost terminal frame) — and only when the attempt/pass pair differs.
7. **`done`/`error`/`cancelled` on the message** finalizes every open pass and round; no pass stays spinning after the turn ends.
8. **The reply appears once**: `final_response` sets the message's single `text` part. If it arrives twice, the second is ignored. If the run ends with no `final_response`, the last `pass_token(final:true)` (fast path) becomes the reply.
9. **Bounded growth**: keep at most 50 rounds per pass and 64 KiB of prose per round (older text is truncated with a visible marker); drop pass parts beyond 20 attempts. A runaway repair loop must not freeze the tab.
10. **No layout thrash**: pass updates replace only the affected pass and round objects; the message array is rebuilt once per flushed batch.

### 8.3 Store changes

- `ChatMessage` keeps its shape apart from `parts`.
- Add `runIdsBySession` (or a `currentRun` per message) so stop/reconnect can target the request id.
- Add `phasesOpenBySession` (record default), persisted client-side, for the "keep phases open" preference.
- Do not persist messages; see §10.

---

## 9. UI design

### 9.1 Component tree

```
ChatMessageItem
├─ message header (unchanged: Agent, model, time, status)
├─ PassBlock[]                     ← new, in frame order, before the reply
│   ├─ PassHeader
│   ├─ RoundBlock[]
│   │   ├─ RoundHeader (call n, model, timer, usage)
│   │   ├─ PassProse (streaming caret)
│   │   └─ Trace / TraceStepItem[]  (tool call + result)
│   ├─ CheckRow[]                  (verify only)
│   └─ RetryNote                   (only when this attempt follows a rejection)
├─ MessageResponse                 ← the single final reply (unchanged)
└─ footer (tokens, retry, copy — unchanged)
```

New files:

- `frontend/src/agentgui/components/chat/pass-block.tsx` — `PassBlock`, `PassHeader`, `RoundBlock`, `CheckRow`, `RetryNote`.
- `frontend/src/agentgui/components/chat/pass-part.tsx` — pure presentational pieces if `pass-block.tsx` grows past ~250 lines.
- Reuse `@/agentgui/components/ai-elements/trace` for tools and `chain-of-thought`/`collapsible` for the container. Do **not** reuse `Reasoning` for the whole pass: its auto-close timer is wrong for a multi-minute phase. Use `Reasoning` only for a single round's prose if a chevron/caret is desired.

### 9.2 Rendered shape (wireframe)

```
┌ Agent ────────────────────────────────────────────── 3 phases · 9.1 s
│ ▸ Interpret · attempt 1 · 2 checks · 1 call · 2.4 s        ✓
│ ▾ Process   · attempt 1 · 3 calls · 6.2 s                  ● running
│    ▾ call 1 · streamed · 812 → 120 tok
│      The configuration file enables two flags that…
│      ┌ data.query ─────────────────────── ok · 46 ms ┐
│      │ input  { … }                                     │
│      │ result { … }                                     │
│      └──────────────────────────────────────────────────┘
│    ▸ call 2 · streamed · 1.2k → 210 tok
│ ▸ Verify    · attempt 1 · checks 2/2 · 0.5 s              ✓
│
│  The config has two problems: the audit flag is off and…
└  ▸ copy   ▸ regenerate                        1,412 tok
```

Collapsed pass rows are one line; expanded rows own their rounds and tool cards. The final reply is the only Markdown body.

### 9.3 Pass header

Collapsed headline (one line, always visible):

```
[Interpret]  attempt 1   ·  2 checks  ·  2 rounds  ·  4.2 s  ·  1.9k tok   ✓
```

- Badge colors follow the existing phase palette (Interpret violet, Process indigo, Verify green; Think slate), consistent with `Phase` colors in `dto/chat.rs`.
- Running: spinner + elapsed timer + the current round label (`call 2`).
- Done: outcome word (`action`, `replied`, `approved`, `fast_path`), duration, tokens, prompt hash short (`#a1b2c3`) on hover/title only.
- Error/cancelled: destructive/caution treatment with the error text as the second line; no fabricated success.
- `aria-expanded` on the disclosure button; the headline text is the accessible name (`Interpret pass, attempt 1, running`).

### 9.4 Open/closed behavior

| State | Default |
|---|---|
| Running pass | open |
| New pass starts | previous pass collapses, new one opens |
| Turn completes | all passes collapse; header summary remains |
| User toggles a pass | that pass keeps the user's choice for the life of the message |
| "Keep phases open" preference | if on, completed passes stay open |
| Fast path (Interpret only) | collapsed; header shows `fast path` and the reply follows |

Auto-collapse is a render preference, not a data mutation: the pass object stays complete for reopen and copy.

### 9.5 Rounds and prose

- Round header: `call 1` · model · `2.4 s` · `812 → 120 tok` · `streamed` / `buffered`.
- Prose renders as plain text with `whitespace-pre-wrap` (plus optional `Streamdown` behind the same `visible` gate as today), in the muted reasoning style — **not** as `MessageResponse`. This keeps phase notes visually distinct from the answer.
- A blinking caret marks the actively streaming round; it disappears on `pass_round_end`.
- Tool calls appear inline under the prose that produced them, in call order. A parallel batch (future) groups by round, not by call time.
- Empty round (model emitted no text and no tools) shows `no output` in muted text rather than an empty box.

### 9.6 Tool cards

Reuse `Trace` / `TraceStepItem` with two changes:

1. **Pass/round context in the title row**: a tiny `interpret · call 1` label, so identical tool names in different phases are distinguishable.
2. **Result fidelity**: `tool_result` frames carry `success`, `durationMs: 0` today. Once the observer reports real durations, show them; until then show success/failure and the result body without pretending the duration is measured.

- Input and output are collapsible, monospace, copyable, and truncated at 8 KiB with `… truncated (n more chars)`.
- Errors use the destructive tone and keep the tool name visible.
- `metamind.think` results that contain nested pass frames render a nested `PassBlock` (phase 3); otherwise they render the existing `Subagent` card with a link/tooltip pointing to the inspector trace.

### 9.7 Verification

- One row per check: label, ✓/✗, confidence to two decimals, backend, duration.
- Failure rows are amber/red and expanded by default; passing rows collapse into the header count.
- Reviewer rejection text (the retry reason) appears on the `RetryNote` before the next Process attempt, not as an error on the message.
- `overrode`/`approved_override` is shown explicitly (`reviewer override`) if the server reports it; never hidden.

### 9.8 Retry, error, cancel

- **Retry**: after a rejected Verify, render a divider `Process retry · attempt 2 — <reason>`; the new pass header carries `attempt 2`. Earlier attempts stay readable.
- **Failure**: the failing pass stays open with its error; the message-level error appears once; the reply is absent (never a half-answer presented as complete).
- **Cancel**: open passes close with `cancelled`; partial prose remains visible and labeled partial; the message footer says `stopped`.
- **Budget**: a `BUDGET` outcome is a pass-level error with the budget name; no generic "failed".

### 9.9 Legacy path

When no pass frames were seen, `ChatMessageItem` renders exactly as today (`thinking`, `trace`, `text`). The live CSS/aria changes must not alter that DOM.

### 9.10 Accessibility, performance, content limits

- Announce pass transitions once via a polite live region (`Interpret finished, Process running`), never per token.
- Disclosure controls are real buttons with `aria-expanded`/`aria-controls`; focus returns to the header on collapse.
- Respect `prefers-reduced-motion`: no caret animation or shimmer.
- Render collapsed passes as a single header row; only expanded passes render rounds and tool bodies. Memoize `PassBlock` by `(pass, attempt, status, updatedAt)`.
- Virtualize only if measured: a turn with 3 passes and 4 rounds is small; never virtualize the whole feed.
- Hard caps from §8.2 keep a pathological repair loop bounded; the UI shows `earlier output truncated` rather than dropping silently.

---

## 10. Persistence and reload (staged)

Chat messages are memory-only today: `hydrateSessions` restores session rows, not transcripts. This plan does not regress that. Phase 4 adds reconstruction:

- Preferred: `GET /api/chat/sessions/{id}/turns?limit=n` returning stored `(seq, frame)` sets for recent runs, reduced client-side by the same reducer.
- Fallback: replay the main agentstream trace through `buildPassTimeline` and render historical passes read-only, without pretending they are live (no caret, no stop control).

Until then, opening a session shows the persisted conversation history only if the workspace already has it in memory; the plan must state this in the UI docs rather than fabricating a transcript.

---

## 11. Delivery phases

| Phase | Ships | Flag state | Exit gate |
|---|---|---|---|
| **P0. Contract** | Additive `ChatFrame` fields + new frames; DTO export and regenerated `frontend/src/lib/wire/chat.ts`; `PassObserver` trait with no-op default; observer wired from `state.cme_executor()`; `frames_for` dedupe mode | `MM_THREE_PASS_V2=false` | Rust tests: frame round-trip, observer ordering golden, no duplicate pass/tool frames; legacy suites green; generated TS typecheck clean |
| **P1. Transport + renderer on fixtures** | `streamChat` SSE reader with chunk-boundary tests; pass-aware `eventsForFrame`; `PassPart`/`PassBlock`; one-message reducer tests; legacy DOM unchanged | false | A recorded stream fixture renders one assistant message, 3 passes, tool cards, checks, and one reply |
| **P2. Live deltas** | `mm-llm` streaming (`complete_stream`), observer deltas, `config/three_pass.toml` `stream`; per-round usage; cancel closes rounds | true in dev/test | A live turn shows tokens inside each pass before `final_response`; abort closes open passes; `stream=false` still renders identically |
| **P3. Rich states** | Retries/attempts, verification detail, fast path, failure/cancel/budget states, subagent nested passes | true | Fixture matrix covers approve, reject→retry→approve, exhausted, protocol repair, cancel, fast path |
| **P4. Continuity + rollout** | Optional history reconstruction; docs/operator runbook; e2e against the real server with a scripted provider | true | Reload shows recent turns read-only; operator guide documents both flag states |

Each phase lands with the flag off until its gate passes; P2 is the first phase that requires `MM_THREE_PASS_V2=true` to demo.

---

## 12. Test plan

### 12.1 Rust

- `dto/chat.rs`: every new/extended frame serializes with the documented keys; old fixtures still deserialize. Regenerate `frontend/src/lib/wire/chat.ts` with `cargo test -p mm-serve export_types -- --ignored` and commit the generated diff.
- `mm-llm`: OpenAI stream parser handles split chunks, multi-line data, usage-only chunks, mid-stream errors, schema requests (`non_streaming`), and cache hits (single delta).
- `mm-cme`: a `RecordingObserver` asserts exact order for: interpret→process→verify; protocol repair as round 2; reject→retry as attempt 2; fast path (`interpret` only); verify exhausted.
- `mm-serve`: `append_frames` + `frames_from` replay equality; live sink emits each frame once; terminal frame last; `frames_for` emits no duplicate tool/pass frames when the observer is active; stop emits a terminal frame.
- Existing three-pass, process, and chat suites stay green with the flag off.

### 12.2 Frontend unit

- SSE reader: 1-byte chunks, CRLF, comments, `event:` naming, multi-`data:` lines, premature EOF, resume from `from`.
- Reducer: each frame type; duplicate `id`; out-of-order `pass_token`; missing `pass_start`; two `final_response`; `done` with an open round; retry attempt does not overwrite attempt 1.
- Batching: 2,000 deltas coalesce into one commit per animation-frame batch; a tool frame flushes the buffer before it is applied.
- Caps: runaway rounds truncate with the marker and keep the header accurate.

### 12.3 Frontend components

- `PassBlock` collapsed/expanded/running/done/error/cancelled snapshots.
- Tool input/output disclosure, truncation, copy.
- Check rows with pass/fail/confidence and reviewer override.
- Legacy message snapshot is byte-for-byte unchanged (DOM structure test).
- Accessibility: disclosure names, `aria-expanded`, live-region text, reduced motion.

### 12.4 End-to-end

- Playwright route interception for `/api/chat` with a scripted SSE fixture: one user prompt → one assistant bubble containing Interpret/Process/Verify → tools and checks visible → final reply last.
- A second spec with flags on against a scripted deterministic executor (no live provider quota).
- A regression spec with flags off: current behavior, no pass blocks.

---

## 13. Acceptance criteria

- [ ] With `MM_CME_ENABLED=true MM_THREE_PASS_V2=true`, a normal turn renders **one** assistant message with Interpret, Process, and Verify sections in order.
- [ ] The final reply appears once, after the last pass, and is the only Markdown answer block.
- [ ] Every LLM call in a pass appears as a round with streamed prose and its own usage; repair rounds and retries are distinct.
- [ ] Every tool call has a matching result card in the same pass/round, with input, output, success, and real duration when available.
- [ ] Verification checks are visible with label, outcome, and confidence; a rejection produces a visible retry note and a second Process attempt.
- [ ] Fast path renders only Interpret and the reply.
- [ ] Cancel, failure, protocol error, and budget exhaustion are distinguishable and never look like a successful answer.
- [ ] With either flag off, the chat is visually unchanged.
- [ ] Reconnect replays from the last sequence without duplicate text, tool cards, or passes.
- [ ] `npm run typecheck`, `npm test`, `npm run build`, `cargo test -p mm-llm -p mm-cme -p mm-serve`, clippy, and fmt are green.

---

## 14. Risks, decisions, open questions

### 14.1 Decisions

| # | Decision | Why |
|---|---|---|
| D1 | Extend existing frames instead of a parallel pass protocol | Keeps `wireVersion: 1` and legacy clients working; no second source of truth |
| D2 | `pass` + `attempt` + `round` are the correlation keys; `phase` stays legacy | The legacy enum cannot express interpret/process/verify or retries |
| D3 | One assistant message; pass parts before the reply | Satisfies "one turn" without inventing a second bubble or a timeline page |
| D4 | Phase prose is plain/muted, never `MessageResponse` | The answer must be unambiguous; pass notes are supporting evidence |
| D5 | Structured/schema calls are non-streaming | Partial JSON cannot be validated; a single delta is honest |
| D6 | Observer active ⇒ `frames_for` emits no post-hoc pass/tool frames | Duplicate cards are the most likely regression |
| D7 | Reply source is `final_response` first, fast-path token as fallback | Prevents double text when both exist |
| D8 | History reconstruction deferred to P4 | Current chat is memory-only; streaming is the requested change |

### 14.2 Risks

| Risk | Mitigation |
|---|---|
| Providers/proxies buffer SSE | Heartbeats; `stream=false` fallback produces the same frame set; test with nginx-style buffering if deployed |
| Token volume and React churn | rAF batching, collapsed rendering, per-round caps, memoized pass blocks |
| Budget/wall-clock interaction | Stream aborts settle the ledger; round ends with `cancelled`; no partial reply is promoted |
| Duplicate pass/reply frames during migration | Dedupe mode in `frames_for` is a P0 tested invariant; client dedupes by seq and by `(pass, attempt)` |
| Hidden reasoning exposure | Only `visible` model text; no raw prompts; pass prose is already model output |
| Very large tool payloads | 8 KiB render cap with explicit truncation; server keeps full data for the inspector |
| Long-lived turns | Completed passes collapse; elapsed timers are per pass, not one global spinner |

### 14.3 Open questions

1. Should pass prose be Markdown-rendered (`Streamdown`) or deliberately plain? The plan starts plain to avoid answer confusion; revisit after dogfooding.
2. Do we need a compact "phases" strip above the composer (Interpret/Process/Verify lit as they run), in addition to the in-message blocks? Deferred; the collapsed pass headers already provide it.
3. Should `metamind.think` sub-passes stream into the parent message, or remain a nested expandable trace? P3 decides based on frame volume; the default is nested and collapsed.
4. Is `pass_round_end` enough for accounting, or does the UI need live token counts while streaming? Providers often report usage only at the end; do not fabricate interim counts.
5. Should the agentgui chat and the legacy transcript stack converge later? Out of scope here; the reducer boundary (`applyEvent`) is the place to reuse if it happens.

---

## 15. Traceability

| Requirement | Workstream | Test |
|---|---|---|
| UI uses the three-pass path | §6.3, §6.4, §11 P2 | e2e with flags on |
| Streaming for every LLM call | §6.1, §6.2, §7.1, §7.4, §9.5 | SSE + reducer + live e2e |
| Tool calls and results per phase | §5.1, §6.2, §8.2, §9.6 | Rust observer golden + component test |
| Verification visible | §5.1, §9.7 | fixture matrix |
| One turn on the UI | §4.1, §8.2 rule 1, §9.1 | one-message snapshot |
| "Thinking"/"tools" visual language | §9.3–9.6 | component snapshots |
| Legacy behavior preserved | §5.3, §9.9 | legacy DOM test + flag-off e2e |
| Honest failures | §9.8 | failure/cancel/budget fixtures |

---

## 16. Documentation updates when implemented

- `infodocs/operator-guide.md`: the two flags, `stream` config, what the chat shows versus the inspector.
- `infodocs/components-and-status.md`: chat transport, `PassObserver`, frame additions.
- `design/DESIGN_INDEX.md`: link this plan under W.
- `frontend/README.md` (if it documents the shell): the one-message phase model and the legacy fallback.
