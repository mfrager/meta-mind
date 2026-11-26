# Streamlined Core: Four-Phase Simplification Plan

> **Status:** superseded for turn processing by
> `design/simple_core_five_phase_plan.md` (2026-11-26); archival, JSONL, and
> frontend decisions below still apply · **Date:** 2026-11-25 · **Scope:** whole
> workspace, server surface, agentstream logging, and frontend log
>
> **Goal:** reduce MetaMind to one thing — the three-pass turn processor —
> with one append-only event log that the frontend renders directly. Everything
> else is frozen in `archive/legacy/` and stops building, testing, and running.

---

## 1. Executive summary

The current backend contains several overlapping systems: a legacy controller
(`mm-serve/src/controller/`), the conversation engine (`mm-cme` driver, regions,
voice, memory, plugins, improvement), the durable process pipeline
(`mm-process`), an external agent runtime (`mm-agent`, `mm-pi`), the RDF/SQLite
data layer (`mm-data`, `mm-store-*`), and a separate LLM activity feed
(`mm-serve/src/llm_feed.rs`). The three-pass turn is already implemented inside
that stack (`mm-cme/src/executor.rs::execute_three_pass`,
`mm-cme/src/runner.rs`, `mm-cme/src/process.rs`,
`mm-cme/src/verification/`), but it is coupled to all of it.

This plan keeps only the three-pass turn processing system and the minimum
transport needed to run and observe it:

- **Six crates**: `mm-core`, `mm-prompt`, `mm-llm`, `mm-agentstream`,
  `mm-cme`, `mm-serve`.
- **One turn path**: Interpret → Process → Verify, with retries, no controller,
  no driver, no regions, no voice, no process pipeline, no agent delegation.
- **Six tools**: four conversation-state tools plus `metamind.verify` and
  `metamind.approve`. All `data.*`, `agent.run`, `metamind.think`,
  `metamind.plan`, `metamind.memory`, `metamind.log`, and `metamind.act` are
  removed.
- **One log**: `data/core/agentstream/<sessionId>.jsonl`. Every line is one
  event, served unchanged to the frontend over snapshot and SSE. LLM requests
  carry the complete rendered messages and the complete tool schemas.
- **One frontend log**: the LLM activity dock is replaced by an Agentstream
  dock that renders those JSON lines, with a full prompt and tools viewer.
- **One test scope**: only the six kept crates and the kept frontend. Retired
  tests move into the archive with their code.

The work is split into four phases: archive and contract freeze, backend
cutover, transparent event log, then frontend wiring and test consolidation.

| Phase | Outcome | Exit condition |
|---|---|---|
| 1. Freeze | Archive skeleton, manifest, freeze tag, golden fixtures, event-schema test | Legacy is reproducible; nothing has moved yet |
| 2. Backend cutover | Six kept crates; one `TurnPipeline`; six tools; everything else `git mv`'d | `cargo check --workspace` green; mock turns log the full pipeline |
| 3. Transparent log | JSONL event store, full prompt/tools capture, snapshot + SSE, old LLM feed removed | File bytes equal wire bytes; full prompt visible by curl |
| 4. Frontend + tests | Event-driven transcript, Agentstream dock with prompt viewer, core-only CI | Only core tests run; live turn is transparent end to end |

---

## 2. Current state and why it is being replaced

| Area | Today | Problem |
|---|---|---|
| Turn execution | `mm-cme` `SessionDriver` + `TurnExecutor` + `Turn`/`Chunk` state machines, plus legacy `mm-serve/src/controller/turn.rs` | Several orchestration layers for one pipeline; controller can be selected independently, so behavior and logging differ by path |
| LLM logging | `llm_calls` SQLite ledger, `mm-serve/src/llm_feed.rs` in-memory ring, agentstream `llm_*` RDF events, chat `pass_*` frames | Four partial views of the same call; none has the full prompt; the dock sees redacted previews only |
| Agentstream storage | `mm-data::agentstream` upserts each event into RDF and appends an N-Triples line | Full-graph reads and JSON sorting dominate CPU; the on-disk `.nt` shape is not the wire shape |
| Prompt state | `StateService` loads an RDF `SessionBundle` with conversation, frames, memory, data health, plans, actions, agents | Prompt templating depends on systems that are being removed; loading it is expensive |
| Tools | 19-tool catalog across conversation, data, and metamind families | Most tools are unrelated to the turn loop and carry their own stores and permissions |
| UI | Chat shell, inspector, trees, files, docs, catalog, map, agentgui | Most surfaces have no backend once the core is kept |

The measured symptom that motivated this work — a chat turn spending ~59 s in
status recomputation and ~700% CPU in full-graph reads — is a consequence of the
data/agent surface, not of the three passes. Removing that surface removes the
cause.

---

## 3. Target architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ mm-serve                                                                    │
│  POST /api/turns        → start one turn, return { sessionId, turnId }      │
│  GET  /api/agentstream/sessions/{id}/events?from=<seq>                      │
│  GET  /api/agentstream/sessions/{id}/stream?from=<seq>   (SSE)              │
│  GET  /api/agentstream/sessions                                             │
│  GET  /api/health                                                           │
└───────────────┬─────────────────────────────────────────────────────────────┘
                │ run_turn(session, text)
┌───────────────▼─────────────────────────────────────────────────────────────┐
│ mm-cme::TurnPipeline                                                        │
│                                                                             │
│  1. Interpret   prompt cm.interpret.v1                                      │
│                 tools: metamind.verify, metamind.approve                    │
│                 result: fast-path reply  |  checks + process brief           │
│                                                                             │
│  2. Process     prompt cm.process.v1                                        │
│                 tools: conversation.topic/depth/status/frame                │
│                 loop until final text (no agent.run, no data.*)             │
│                                                                             │
│  3. Verify      Phase A: run each VerifyCheck (llm_judge)                   │
│                 Phase B: prompt cm.verify.v1                                │
│                 tools: conversation.* + metamind.approve                    │
│                 approve → commit staged state, emit agent.message           │
│                 reject  → retry Process (max 5)                             │
└───────────────┬─────────────────────────────────────────────────────────────┘
                │ append every event
┌───────────────▼─────────────────────────────────────────────────────────────┐
│ mm-agentstream                                                              │
│  JSONL store per session (seq, event)                                       │
│  projections: CleanLog, SessionState, TurnSummary, SessionIndex            │
│  LLM gateway: full request/response capture                                 │
│  live bus: broadcast<Event> for SSE                                         │
└───────────────┬─────────────────────────────────────────────────────────────┘
                │ same JSON objects, unchanged
┌───────────────▼─────────────────────────────────────────────────────────────┐
│ Frontend                                                                    │
│  transcript  = projection of turn.started / agent.message / turn.failed    │
│  Agentstream dock = one row per JSON line, grouped Turn → Pass → Call      │
│  prompt viewer = llm.request.data.messages + data.tools (complete)         │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Kept crates

| Crate | Kept modules | Archived / removed |
|---|---|---|
| `mm-core` | `config` (reduced), `error`, `hash`, `id`, `time`, `lib` | `activation`, `codex`, `iri` (unless needed by `id`), `store` |
| `mm-prompt` | all | none (prompt tree pruned in `config/`) |
| `mm-llm` | `client`, `openai`, `mock`, `config`, `error`, `call`, `redact`, `schema`, `accounting` (usage) | `backend_llguidance`, `backend_xgrammar`, `grammar` (unless `schema` needs it), `cached`, `provenance`, `replay`, `semantic_cache`, `routing`, `service` (replaced by a small transport) |
| `mm-agentstream` | **new**: `event`, `store`, `bus`, `projection`, `state`, `gateway`, `lib` | supersedes `mm-llm-agentstream` and `mm-data::agentstream` |
| `mm-cme` | `config`, `error`, `pipeline`, `interpret`, `process`, `verify`, `prompt`, `state_block`, `tools`, `verification/`, `budget`, `observer`, `lib` | `chunk`, `dashboard`, `driver`, `executor`, `frames`, `hill_climb`, `improvement`, `memory`, `observability`, `planner`, `plugins`, `preferences`, `regions`, `report`, `routing`, `runner` (logic absorbed), `session`, `subagent`, `turn`, `voice` |
| `mm-serve` | `main`, `lib`, `state`, `error`, `sse`, `metrics` (minimal), `dto/agentstream`, `routes/turns`, `routes/agentstream`, `routes/sessions`, `routes/health` | every other module in `src/` (see the manifest) |

`mm-sm` is intentionally not kept. The new `TurnPipeline` is a sequential
function with explicit retry control; it does not need a general state-machine
runtime. `mm-log`, `mm-eventlog`, `mm-store-graph`, and `mm-store-sqlite` are
also not kept: `tracing` writes operational output to stderr, and agentstream
JSONL is the only durable event log.

### 3.2 Turn request and result

The planner, router, voice tiers, chunks, and speculative warm-up are removed.
A turn needs only its identifiers, the user text, the session projection, and
the configured pass budgets.

```rust
pub struct TurnRequest {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub user_input: String,
}

pub struct TurnResult {
    pub turn_id: TurnId,
    pub reply: String,
    pub model: String,
    pub attempts: u32,
    pub verified: bool,
    pub usage: Usage,
    pub duration_ms: u64,
}
```

### 3.3 The six tools

| Tool | Pass | Effect |
|---|---|---|
| `conversation.topic` | Process, Verify | Stage a topic update |
| `conversation.depth` | Process, Verify | Stage a depth update |
| `conversation.status` | Process, Verify | Stage a typed status update |
| `conversation.frame` | Process, Verify | Stage a frame-stack change |
| `metamind.verify` | Interpret | Register verification checks |
| `metamind.approve` | Interpret (fast path), Verify | End the turn, or reject and retry |

**Removed and archived:** `agent.run`, `data.create`, `data.update`,
`data.query`, `data.delete`, `data.search`, `data.schema`, `data.namespace`,
`metamind.plan`, `metamind.think`, `metamind.memory`, `metamind.log`,
`metamind.act`. There is no shell, file, HTTP, process, or external-agent tool
in the kept core. With no side-effecting tools, the confirmation gate and the
ToolPolicy machinery are removed.

Why `conversation.*` stays: it is the only writer of the state that templates
the next prompt. It is turn-processing state, not a general data or agent tool.

---

## 4. The only event log

### 4.1 File layout

```
data/core/
  server.lock                         # single-writer guard
  agentstream/
    sessions.jsonl                    # session index; one record per line
    <sessionId>.jsonl                 # this session's events, seq 1..N
```

Rules:

- One JSON object per line, compact UTF-8, `\n` terminated.
- Append-only. A record is never rewritten. A crash may leave a partial last
  line; readers skip it and the next append starts on a fresh line.
- `seq` is monotonic **within a session**, starting at 1. The server is the
  only writer (enforced by `server.lock`), so allocation is an in-memory
  counter initialized from the last valid line.
- `sessions.jsonl` is the only cross-session index: `session.started` /
  `session.ended` / title updates. It is also append-only and folded on read.
- Old `data/` is not touched. The kept server defaults to `data/core/`; the
  legacy worktree keeps `data/` at the freeze tag.
- Rotation: at 64 MiB, `<sessionId>.jsonl` is closed and a
  `<sessionId>.1.jsonl` segment is started, with `seq` continuing. The reader
  concatenates segments in order. Rotation is an implementation detail; the
  wire contract is still one ordered stream per session.

### 4.2 Envelope

Every line has the same envelope. `data` is type-specific.

```json
{
  "v": 2,
  "seq": 17,
  "id": "01JC9W4T7Q9K5S2V8M3N6P1R0X",
  "sessionId": "01JC9W4T7Q9K5S2V8M3N6P1R0X",
  "turnId": "01JC9W4T7Q9K5S2V8M3N6P1R0Y",
  "at": 1764115256123,
  "type": "llm.request",
  "source": "pass.process",
  "model": "nvidia/nemotron-3.5-lightning-30b-a3b",
  "data": { }
}
```

| Field | Meaning |
|---|---|
| `v` | Schema version. This plan defines `2`. |
| `seq` | Per-session cursor used by snapshot, SSE `id:`, dedupe, and resume. |
| `id` | Event ULID. Stable across files and logs. |
| `sessionId` | Owning session. |
| `turnId` | Owning turn, when applicable. |
| `at` | Epoch milliseconds. |
| `type` | Dot-separated event type. |
| `source` | Emitting component (`turn.pipeline`, `pass.interpret`, `llm.gateway`, `tool.conversation.status`). |
| `model` | Model involved, when applicable. |
| `data` | Full payload. No previews, no truncation by default. |

### 4.3 Event types

**Session and turn**

| Type | Data |
|---|---|
| `session.started` | `{ userId, title? }` |
| `session.ended` | `{ reason }` |
| `turn.started` | `{ userInput, text }` |
| `turn.completed` | `{ reply, model, attempts, verified, usage, durationMs }` |
| `turn.failed` | `{ error, kind, attempts }` |

**Passes and prompts**

| Type | Data |
|---|---|
| `pass.started` | `{ pass, attempt, model }` |
| `pass.completed` | `{ pass, attempt, finished, text?, checks?, usage, durationMs }` |
| `pass.failed` | `{ pass, attempt, error, durationMs }` |
| `pass.retry` | `{ pass, attempt, nextAttempt, reason, verification }` |
| `prompt.rendered` | `{ pass, strategy, instanceId, hash, variables, messageCount }` |

`prompt.rendered` is the internal-action record that proves what the template
received. `llm.request` then carries the exact messages produced from it.

**Model calls** — produced by the `mm-agentstream` gateway for every call,
including the LLM judge.

| Type | Data |
|---|---|
| `llm.request` | `{ callId, pass, attempt, round, provider, purpose, strategy, promptInstanceId, promptHash, params, messageCount, toolCount, messages, tools }` |
| `llm.attempt` | `{ callId, attemptNo, status, model, latencyMs, usage }` |
| `llm.delta` | `{ callId, index, text }` — **live only, not persisted** |
| `llm.response` | `{ callId, status, model, text, toolCalls, usage, latencyMs, finishReason, reasoningChars, cached, streamed }` |
| `llm.error` | `{ callId, kind, message, latencyMs, partialChars }` |

`llm.request.data.messages` is the complete rendered array, exactly as sent:
system instructions, clean-log turns, the user input, prior assistant tool
calls, and tool results. `llm.request.data.tools` is the complete array of
`ToolSchema` objects offered for that call, including `parameters`. No prompt or
tool schema is replaced by a hash. A hash is additional, not a substitute.

`llm.delta` is the one deliberate exception to "each JSON line is persisted":
tokens would make the log enormous. Deltas are broadcast live so the dock can
stream, and the final `llm.response` persists the full text. The frontend log's
rows are built from persisted records; live deltas update the open row only.

**Tools, state, verification, internals**

| Type | Data |
|---|---|
| `tool.call` | `{ callId, name, arguments, pass, attempt, round }` |
| `tool.result` | `{ callId, name, ok, output, durationMs, staged }` |
| `state.updated` | `{ revision, before, after }` |
| `verification.registered` | `{ checks }` |
| `verification.result` | `{ label, backend, pass, confidence, reasoning, durationMs, attempt }` |
| `internal.action` | `{ component, action, ok, durationMs, input, output, error? }` |
| `internal.error` | `{ component, action, message, detail? }` |
| `agent.message` | `{ text, final: true, verified, replyTo }` |
| `user.message` | `{ text, attachments? }` |

`internal.action` is the generalization requested for internal actions and
function calls: prompt rendering, state load, budget reservation, state staging,
verification dispatch, file append, and any future step. `tool.call` /
`tool.result` are the function-call records. Every one is a JSON line and every
one is rendered by the frontend log.

### 4.4 Example: a complete LLM request record

```json
{
  "v": 2,
  "seq": 23,
  "id": "01JC9W4T7Q9K5S2V8M3N6P1R0X",
  "sessionId": "01JC9W4T7Q9K5S2V8M3N6P1R0A",
  "turnId": "01JC9X4T7Q9K5S2V8M3N6P1R0B",
  "at": 1764115256123,
  "type": "llm.request",
  "source": "llm.gateway",
  "model": "nvidia/nemotron-3.5-lightning-30b-a3b",
  "data": {
    "callId": "01JC9Y4T7Q9K5S2V8M3N6P1R0C",
    "pass": "process",
    "attempt": 1,
    "round": 1,
    "provider": "default",
    "purpose": "converse",
    "strategy": "cm.process.v1",
    "promptInstanceId": "cm.process.v1@sha256:7f3c...",
    "promptHash": "sha256:7f3c...",
    "params": { "temperature": 0.2, "maxTokens": 4096, "streamed": true },
    "messageCount": 4,
    "toolCount": 4,
    "messages": [
      { "role": "system", "content": "You are Metamind...\n[STATE]..." },
      { "role": "user", "content": "Earlier question" },
      { "role": "assistant", "content": "Earlier answer" },
      { "role": "user", "content": "What did we decide?" }
    ],
    "tools": [
      {
        "name": "conversation.topic",
        "description": "Set the conversation topic.",
        "parameters": {
          "type": "object",
          "properties": { "topic": { "type": "string" } },
          "required": ["topic"]
        }
      }
    ]
  }
}
```

### 4.5 Projections used for prompt templating

The log is authoritative; every projection is rebuildable from it.

| Projection | Source events | Used for |
|---|---|---|
| `CleanLog` | `user.message`, `agent.message` (`final: true`) | Conversation history in every pass |
| `SessionState` | `session.started`, `state.updated` | `[STATE]`: topic, depth, status, frames, revision |
| `TurnSummary` | `turn.started`, `pass.*`, `llm.*`, `verification.*`, `turn.*` | Inspector, retry context, metrics |
| `SessionIndex` | `session.*` | Session list and title |

`PromptContext` is assembled before each pass:

```rust
pub struct PromptContext {
    pub session_id: SessionId,
    pub user_input: String,
    pub clean_log: CleanLog,
    pub state: SessionState,
    pub budget: BudgetSnapshot,
    pub pass: PassInput,
}
```

`PassInput` is `Interpret { }`, `Process { brief, checks, retry }`, or
`Verify { process_reply, checks, results, retry }`. The prompt store renders
`cm.interpret.v1`, `cm.process.v1`, or `cm.verify.v1` from this context. No
removed subsystem is referenced by the kept prompt tree.

### 4.6 Append semantics

- `turn.started`, `pass.started`, `prompt.rendered`, `llm.request`,
  `llm.response`/`llm.error`, `verification.*`, `agent.message`,
  `turn.completed`/`turn.failed` are **required**: the turn fails closed if the
  record cannot be appended.
- `tool.call`, `tool.result`, `state.updated`, `internal.action`,
  `internal.error`, and `llm.attempt` are **best-effort**: a write failure is
  reported on stderr and as an `internal.error` if that also succeeds; it does
  not abort a turn that is already correct.
- `llm.delta` is live-only and never blocks.
- Writes go through a single append lock per session. The record is serialized
  once and the same bytes are written to disk and published on the live bus, so
  the frontend cannot disagree with the file.

### 4.7 Retention

- Session files are kept by default. `MM_AGENTSTREAM_RETENTION_DAYS` may prune
  whole session files; pruning is never applied to the active session.
- Timestamps, `seq`, and `id` are never rewritten.
- Export is a file copy. There is no database, migration, or backup format.

---

## 5. Keep / archive matrix

The full path-level inventory is in
[`archive/legacy/MANIFEST.md`](../archive/legacy/MANIFEST.md). The binding
rule: a crate is kept only if the three-pass turn cannot run without it.

### 5.1 Workspace

**Kept members (6):** `mm-core`, `mm-prompt`, `mm-llm`, `mm-agentstream`,
`mm-cme`, `mm-serve`.

**Archived members:** `mm-agent`, `mm-being`, `mm-cli`, `mm-codex`, `mm-data`,
`mm-decision`, `mm-epistemic`, `mm-eventlog`, `mm-firewall`, `mm-library`,
`mm-log`, `mm-memory`, `mm-metaanalysis`, `mm-metacog`, `mm-mistakes`, `mm-pi`,
`mm-process`, `mm-runtime`, `mm-search`, `mm-selfeng`, `mm-sm`,
`mm-store-graph`, `mm-store-sqlite`, `mm-tools`, `mm-llm-agentstream`, the ten
`modules/*` members, the seven `vendor/rust_symbolic/*` members, and
`tests/e2e`.

The root `Cargo.toml` gets `exclude = ["archive/**"]`; the member list contains
exactly the six kept crates. `Cargo.lock` is regenerated.

### 5.2 `mm-serve`

Keep and rewrite: `main.rs`, `lib.rs`, `state.rs`, `error.rs`, `sse.rs`,
`metrics.rs` (minimal), `dto/agentstream.rs`, `routes/turns.rs`,
`routes/agentstream.rs`, `routes/sessions.rs`, `routes/health.rs`.

Archive every other module listed in the manifest, including `llm_feed.rs`,
`dto/llm_activity.rs`, `routes/llm_activity.rs`, the controller, `chat.rs`,
`cme.rs`, `cm_tools.rs`, `data_tools.rs`, `metamind_tools.rs`, automation,
status/status_service, focus, trees, docs, interjection, plan monitor, process
runner, parity, feature flags, and the legacy route files.

### 5.3 Frontend

Keep: `App.tsx`, `main.tsx`, `index.css`, the chat pane/composer, the
transcript view, the Agentstream dock and its prompt/tools viewers, a minimal
shell and session picker, `lib/api/{client,turns,agentstream}.ts`,
`lib/store/{agentstream,session,prefs}.ts`, `lib/transcript/*`,
`lib/wire/agentstream.ts`, and only the UI primitives those files import.

Archive: `agentgui/`, `catalog/`, `docs/`, `files/`, `inspector/`, `mindmap/`,
`trees/`, `state/`, the old `sessions/` list, the old `llm-log-*` components,
`use-llm-activity-stream.ts`, `use-trace-stream.ts`, `use-status-stream.ts`,
the old API clients for data/files/focus/inspector/state/trees, and the old
stores for documents/focus/inspector/llm-log/pane/side/status. Every retired
component's test moves with it.

### 5.4 Config and prompts

Keep: `config/llm.toml` (one default model plus the judge model),
`config/three_pass.toml` (three passes, retries, budgets, streaming),
`config/persona.toml` (new, static identity values),
`config/server.toml` (new, bind address and data dir), and the pruned prompt
tree.

Keep prompt applications: `cm.interpret.v1.toml`, `cm.process.v1.toml`,
`cm.verify.v1.toml`. Keep the fragments those strategies use (identity,
character, style, being, state, state ownership/frames/pending, tools contract,
tool selection, interpret contract, fast-path criteria, verification guide and
review, verify contract, retry guidance, process execution, budget contract,
default action check, honesty contract, safety contract, output character).

Archive the rest of `config/prompts/applications/` (agent, frame, resume,
tutorial, repair, step, think, plan builder, data, metamind, turn variants) and
the fragments only they use (data, memory, plan, frame, agent, think, repair,
resume, tutorial, step, source map). Archive `config/agents.toml`,
`automation.toml`, `interjections.toml`, `logging.toml`, `metamind.toml`,
`process.toml`, and `routing_table.toml`.

### 5.5 Tests

Every `#[cfg(test)]` module and `tests/` directory of an archived crate moves
with that crate. Archived frontend tests move with their components.
`tests/e2e`, `bench/`, and the `pi/` test trees are archived. The kept test
matrix is defined in §9.

---

## 6. Phase 1 — Freeze, inventory, and capture (no behavior change)

**Intent:** make the archive and the contracts real before deleting anything,
and keep a reproducible legacy reference.

### Work

1. Create `archive/legacy/` with the README and manifest in this change.
2. Record the current `main` commit and create the annotated tag
   `legacy-freeze-2026-11-25`. The tag is the last commit where the full
   workspace builds and its tests pass.
3. Fill `MANIFEST.md`: every workspace member, `mm-serve` module, frontend
   directory, config file, and test tree gets a keep/archive decision and a
   replacement (or "removed"). Record the measured baseline:
   `cargo test --workspace` duration and result, `cargo build --workspace`
   result, frontend `typecheck`/`test`/`build` result.
4. Capture golden fixtures from the current three-pass path with the mock
   client: a fast-path turn, an action turn that approves, an action turn that
   fails once then approves. Store the rendered prompts, tool schemas, pass
   events, verification results, and final reply under
   `crates/mm-cme/tests/fixtures/golden/` (new fixtures are part of the kept
   test scope; legacy fixtures stay in the archive).
5. Write the event-schema contract test first (an `mm-agentstream` type test
   that asserts the envelope, `seq`, and every event name). This is the
   contract Phase 2 and Phase 4 implement against.
6. Decide and document the new data root (`data/core/`). Do not move or delete
   existing `data/`.

### Done criteria

- `archive/legacy/` exists with a complete manifest and freeze tag.
- Baseline build/test timings and results are recorded.
- Golden prompt/event fixtures exist.
- The event-schema test compiles (it may be `#[ignore]` until Phase 2).
- No runtime code has been removed yet.

### Rollback

Delete the tag and the archive skeleton. Nothing in the running system changed.

---

## 7. Phase 2 — Backend cutover to the six-crate core

**Intent:** the server runs only Interpret → Process → Verify, writes only the
new event log, and the rest of the backend is in the archive.

### Work

**Reset dependency edges first.** Before adding new modules, remove the old
edges that would drag archived crates back into the build: `mm-llm` drops
`mm-store-sqlite`, `mm-store-graph`, and `mm-log`; `mm-cme` drops
`mm-data`, `mm-sm`, and `mm-being`; `mm-serve` drops every archived crate and
keeps only the five kept libraries plus its HTTP dependencies. The exact
cache, replay, ledger, and RDF modules that used those edges are archived, not
re-exported.

1. **Create `mm-agentstream`.** Implement the envelope, the JSONL store,
   per-session `seq`, segment rotation, partial-line recovery, the live
   broadcast, and the projections (`CleanLog`, `SessionState`, `TurnSummary`,
   `SessionIndex`). Move `CleanLog` logic from `mm-data::agentstream`; do not
   carry the RDF upsert or the N-Triples writer.
2. **Create the LLM gateway in `mm-agentstream`.** Wrap any `LlmClient` so
   `complete` and `complete_stream` append `llm.request` with full `messages`
   and `tools`, publish `llm.delta` live, and append `llm.response`/`llm.error`.
   Keep the fail-closed request append. This replaces `mm-llm-agentstream`.
3. **Slim `mm-cme` to the pipeline.** Port the orchestration logic from
   `executor.rs::execute_three_pass`, `runner.rs`, `process.rs`, and
   `verification/` into `pipeline`, `interpret`, `process`, `verify`. Replace
   the driver, turn/chunk state machines, planner, router, voice, regions,
   memory bridge, and plugins with one `TurnPipeline::run(TurnRequest)`.
   Keep the retry loop, staged conversation-state writes, the LLM judge, and
   per-pass budgets. `SessionId`/`TurnId` become plain newtypes in `mm-core`,
   `CleanLog` moves to `mm-agentstream`, and the RDF `SessionBundle` is replaced
   by the `SessionState` projection.
4. **Reduce the tools to six.** Move the handlers from `mm-serve/cm_tools.rs`
   into `mm-cme/src/tools.rs` for `conversation.*`, `metamind.verify`, and
   `metamind.approve`. Delete the data, agent, plan, think, memory, log, and act
   handlers. Enforce visibility per pass in `pass_scope` (now a small const
   table, not the 19-tool catalog).
5. **Reduce prompt variables.** Remove data health, memory summary, plans,
   actions, agents, project, and tutorial/frame slots. Keep user input, clean
   log, state block (topic/depth/status/frames/revision), budget, persona, and
   pass context. Edit the prompt tree to match; no kept strategy may reference a
   removed slot.
6. **Rewrite `mm-serve`.** Boot with `mm-core` config, one `mm-agentstream`
   store, one `TurnPipeline`, and one **gateway-wrapped** `LlmClient` (the
   pipeline never receives a raw provider client, so no call can bypass the
   log). Implement `POST /api/turns` (202 with ids), the session/event/stream/
   health routes, and delete the legacy route tree. Remove the audit logger,
   metrics registry, feature flags, chat catalog, and lockfile format; use a
   single lock file.
7. **Archive mechanically.** Use `git mv` (not copy) for every retired crate,
   `mm-serve` module, config file, and backend test tree into the paths recorded
   in the manifest. Frontend directories move in Phase 4, after the new routes
   are stable. Remove archived members from the workspace, add
   `exclude = ["archive/**"]`, regenerate `Cargo.lock`, and update the
   manifest's move log with the commit.
8. **Cut data over.** The new server uses `data/core/`. Legacy `data/` is left
   intact for the tagged worktree. Do not write a migration in this phase; a
   fresh event log is the intended start.

### Done criteria

- `cargo metadata` lists exactly the six kept crates (plus no modules/vendor/
  e2e members).
- `cargo tree` for each kept crate names no archived crate.
- `cargo check --workspace` passes.
- A scripted mock fast-path turn and a scripted action+verify turn complete
  through `TurnPipeline`; `state.updated`, `tool.call/result`, `pass.*`,
  `llm.request/response`, `verification.*`, `agent.message`, and
  `turn.completed` appear in the JSONL.
- A `llm.request` line contains the full system/user messages and all offered
  tool schemas.
- No kept crate references an archived crate (`rg` guard).
- The legacy tag is unchanged and `archive/legacy/MANIFEST.md` records the move.

### Rollback

Return to the freeze tag. The archive is additive until the cutover commit;
the cutover itself is one revertable commit series.

---

## 8. Phase 3 — Transparent log and frontend log contract

**Intent:** agentstream is the only LLM/internal log, and the frontend log is a
direct render of its JSON lines.

### Work

1. **Finalize the event taxonomy** from §4.3 in `mm-agentstream::event`, with
   serde rename rules and a versioned envelope. Export TypeScript with `ts-rs`
   from `mm-serve/src/dto/agentstream.rs` to
   `frontend/src/lib/wire/agentstream.ts`.
2. **Add the internal-action records.** Emit `prompt.rendered` before every
   model call; `internal.action` for state load, stage, commit, discard, budget
   reserve, and verification dispatch; `internal.error` for recovered
   failures. Emit `state.updated` only when staged state commits.
3. **Implement snapshot and tail.**
   - `GET /api/agentstream/sessions/{id}/events?from=<seq>&limit=<n>` returns
     `{ sessionId, from, next, events }` straight from the JSONL.
   - `GET /api/agentstream/sessions/{id}/stream?from=<seq>` sends
     `id: <seq>`, `event: <type>`, `data: <record>`; it replays the file from
     the cursor, then forwards the live bus. Duplicate/late seqs are dropped.
   - If the cursor is below the retained segment start, send a `resync` event
     with the current snapshot.
4. **Remove the old LLM log.** Delete `llm_feed.rs`, `dto/llm_activity.rs`,
   `routes/llm_activity.rs`, the `MM_LLM_ACTIVITY` flag, and any client wrapper
   that redacts or previews prompts. The `llm_calls` ledger retires with
   `mm-store-sqlite`/`mm-data`. Agentstream is the only source.
5. **Wire turn creation to the event stream.** `POST /api/turns` validates
   text, creates a session when absent, allocates `turnId`, starts
   `TurnPipeline::run` in a spawned task, and returns
   `202 { sessionId, turnId, seq }` before the first event is appended. The
   client then opens the session stream from that `seq`; a response received
   before the first line is normal.
6. **Define the frontend projection.** A persisted record is a log row; a live
   `llm.delta` updates the open response row. `turn.started` creates the user
   bubble; `agent.message` creates the assistant bubble; `turn.completed` marks
   the turn done; `turn.failed` shows the error. The transcript and the log are
   two views of one stream.
7. **Tests at the contract boundary.** A test asserts that the JSON bytes sent
   over SSE equal the bytes in the file for the same `seq`; another asserts
   that `llm.request.data.messages` equals the `LlmRequest.messages` the mock
   client received, including tool schemas.

### Done criteria

- The old `/api/llm-activity` routes and `llm_feed` no longer exist.
- A mock turn produces a contiguous `seq` range; `events?from=N` and
  `stream?from=N` return identical records.
- `llm.request` includes the full prompt and tools; no field is a preview.
- `prompt.rendered`, `internal.action`, `tool.call/result`, `state.updated`,
  and `verification.result` are present for the action turn.
- Generated TS types match the Rust DTOs.

### Rollback

Keep the old `llm_feed` files in the archive. Reverting this phase restores
the previous routes and dock without touching the turn pipeline.

---

## 9. Phase 4 — Frontend wiring and core-only testing

**Intent:** the kept UI talks only to the new routes, renders the full prompt,
and every remaining test targets the streamlined core.

### Work

1. **Prune the frontend** to the kept files in §5.3 and archive the rest with
   `git mv` (including their tests).
2. **Replace the transport and store.**
   - `lib/api/turns.ts`: `startTurn({ sessionId?, text })`.
   - `lib/api/agentstream.ts`: snapshot, stream URL, session list.
   - `hooks/use-agentstream.ts`: initial snapshot, `EventSource` with
     `?from=lastSeq+1`, dedupe by `seq`, reconnect with backoff, `resync` on a
     gap.
   - `lib/store/agentstream.ts`: bounded records, `lastSeq`, connection state,
     filters, pause/clear; derived `turns` and `calls`.
3. **Derive the transcript from events.** Replace the `ChatFrame` reducer with
   an event reducer: `turn.started` → user item; `pass.*` → progress; `llm.delta`
   → streamed pass text; `llm.response` → final pass text; `agent.message` →
   assistant item; `turn.completed` → verified/done; `turn.failed` → error item.
   No separate chat SSE.
4. **Build the Agentstream dock.** Replace `LlmLogDock` with
   `components/agentstream/AgentstreamDock`:
   - one row per persisted record, grouped Turn → Pass → LLM call;
   - filters: all, turns, passes, calls, tools, verification, internals, errors;
   - pause/clear/collapse, drag/keyboard resize, copy-as-text;
   - connection state from the hook;
   - a live `llm.delta` updates the open call row without adding a row.
5. **Full prompt and tools viewer.** The expanded `llm.request` row has tabs:
   - **Prompt**: every message with role and complete content; tool calls on
     assistant messages; tool results; a copy-JSON button.
   - **Tools**: every offered tool's name, description, and complete
     `parameters` JSON, plus which tools the response actually called.
   - **Response**: full text, finish reason, usage, latency, attempt.
   - **Internals**: `prompt.rendered` variables/hash, `state.updated` before/
     after, `verification.result`, `internal.action`.
   The viewer reads `record.data` directly; it does not reconstruct or
   re-truncate the prompt.
6. **Wire the workspace.** One route: session picker, transcript, composer,
   Agentstream dock. Replace the old API/store imports; keep only the UI
   primitives actually used. Update `vite.config.ts` only if the dev proxy
   targets changed.
7. **Test consolidation.**
   - Move retired tests with their code. Delete duplicate fixtures that refer
     to archived modules.
   - Keep `crates/mm-prompt/tests` as-is, plus the new tests below.
   - Add `scripts/check-core-only.sh`:
     ```bash
     actual=$(cargo metadata --no-deps --format-version 1 \
       | jq -r '.packages[].name' | sort)
     expected=$(printf '%s\n' \
       mm-agentstream mm-cme mm-core mm-llm mm-prompt mm-serve | sort)
     [ "$actual" = "$expected" ] || {
       echo "unexpected workspace members:"; diff <(echo "$expected") <(echo "$actual")
       exit 1
     }
     ```
     and an `rg` guard that kept sources do not import archived crate names.
   - CI runs only `cargo test --workspace`, `cargo clippy`, `cargo fmt`,
     frontend `typecheck`, `test`, and `build`.
8. **Update operations.** Rewrite `start-mm-serve.sh` and `start-console.sh` to
   the new binary/data root; update `README.md` to the six-crate architecture
   and the archive tag; remove scripts for retired systems.

### Done criteria

- `cargo test --workspace` runs only the kept crates and passes.
- `cargo metadata` proves the member set.
- `npm run typecheck`, `npm test`, and `npm run build` pass with only kept
  frontend files.
- A mock action turn shows, live: user message, interpret pass, process call
  with the full prompt and four tool schemas, verify with the judge result,
  assistant message, and the same rows after reload from JSONL.
- SSE reconnect from a mid-turn `seq` duplicates nothing.
- No kept source references an archived crate, route, or config file.
- The server is idle after a turn (no full-graph scan, no recompute loop).

### Rollback

The frontend is the last cutover. Revert this phase to return to the prior UI;
the backend event routes remain usable by curl and the Rust tests.

---

## 10. Test scope after the plan is built

Only this matrix is kept and run.

### Rust unit tests

| Crate | Tests |
|---|---|
| `mm-core` | config resolution, ID/ULID formatting, hash stability |
| `mm-prompt` | existing renderer/hash tests; reduced-tree strategies render |
| `mm-llm` | request/response mapping, redaction, mock scripting, tool schema serialization |
| `mm-agentstream` | envelope/version, `seq` monotonicity, append/replay, partial-line recovery, segment rotation, clean-log projection, state projection, session index, live bus, gateway full-prompt capture |
| `mm-cme` | Interpret fast path, action path, Process loop, staged state commit/discard, Verify approve/reject, retry limit and exhaustion, tool scope per pass, prompt context construction, budget enforcement, no removed tool is callable |
| `mm-serve` | `POST /api/turns`, session creation, events snapshot paging, SSE replay/cursor/dedupe, health, retired routes return 404 |

### Rust integration tests

1. **Fast path:** pure conversation exits after Interpret with one
   `agent.message` and no Process call.
2. **Approved action:** Interpret registers a check, Process stages state,
   Verify approves, `state.updated` and `agent.message` appear, and the JSONL
   is replayable.
3. **Retry:** Verify rejects once, `pass.retry` is recorded, Process receives
   the error and verification context, the second attempt approves.
4. **Exhaustion:** five rejections produce `turn.failed` with
   `VERIFY_EXHAUSTED`, never a silent success.
5. **Prompt fidelity:** for every `llm.request`, its `messages` and `tools`
   equal the objects the mock client received; the full prompt contains the
   clean log and `[STATE]`; no removed slot is referenced.
6. **Log/wire fidelity:** SSE bytes equal file bytes per `seq`; `from=N`
   resumes exactly; no duplicate after reconnect.
7. **Crash recovery:** a partial final line is skipped; the next append starts
   a new valid line; the session projection still loads.

### Frontend tests

- Event reducer: user/assistant/progress/error/final derivation; duplicate seq
  is a no-op.
- Agentstream store: grouping, filters, pause/clear, bounded rows, delta merge.
- Prompt viewer: renders every message and tool schema from a fixture record;
  copy JSON preserves the payload.
- Stream hook: snapshot → live, gap → resync, reconnect → no duplicates.
- One Playwright smoke test with the mock server: start a turn, see the
  transcript reply, open the process call, see the full system prompt and all
  offered tool schemas.

### Explicitly not tested

Anything in `archive/legacy/`, the legacy controller/CME driver/process
pipeline/agent runtime/RDF data layer, retired routes, retired config, and
retired frontend surfaces. Their last green run is the freeze tag.

---

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Prompt quality drops when memory/data/plan context is removed | Golden prompt fixtures before/after; keep identity, state, clean log, honesty, and verification; review one real turn per pass before cutover |
| The archive drifts or gets imported | `exclude` in the workspace, member-set CI guard, `rg` import guard, README rules, freeze tag |
| Losing legacy behavior/data | Old `data/` untouched; `data/core/` is a new root; the tag is the last buildable legacy commit |
| Full prompts expose secrets | Local-only store with `0700`; redactor for key-shaped strings at the gateway; no remote shipping; explicit note that prompts may contain user data |
| Event volume and disk growth | Per-session files, 64 MiB segments, optional day retention; no per-token persistence (deltas live-only); compression is a filesystem concern, not a format change |
| A slow journal write stalls a turn | Required events are small and appended once; response/delta writes are off the provider path; a write failure on an optional event never aborts an approved reply |
| Crash leaves a partial line | Reader skips an invalid trailing line; append starts a fresh line; `internal.error` records the recovery |
| Frontend rewrite breaks the chat | Keep the old dock/transcript in the archive; land the event reducer with fixture tests before switching `App.tsx`; one-route fallback is the prior commit |
| SSE reconnect duplicates or drops | Stable per-session `seq`; `from=lastSeq+1`; dedupe by `seq`; `resync` below the retained start; byte-equality test |
| One process assumption breaks | `server.lock` is acquired at boot; a second server refuses to start; seq allocation is in-process only |

---

## 12. Acceptance traceability

| Requirement | Where it is met |
|---|---|
| Four-phase plan in `design/` | §6–§9 |
| New directory for legacy code | `archive/legacy/` + README/MANIFEST |
| Eliminate everything except the 3-pass system | §3.1, §5.1, §7 |
| Keep only directly related tools; remove data/agent tools | §3.3, §5.2, §7.4 |
| Each agentstream JSON line is the frontend log data | §4, §8.3, §8.6 |
| Additional logs for internal actions and function calls | `internal.action`, `internal.error`, `tool.call/result`, `prompt.rendered` in §4.3 |
| Replace all current LLM logging | §2, §8.4; removes `llm_feed`, `/api/llm-activity`, `llm_calls`, `llm_activity` wire/dock |
| Turn processing still has prompt data | §4.5, §5.4, §7.5 |
| Frontend shows full prompt and available tools | §8.5, §9.5 |
| Archive all other backend code and tests | §5, §7.7, §10 |
| Tests only cover the streamlined core | §10, §9.7 |
| Wire the new backend to the frontend | §3 diagram, §8, §9 |

---

## 13. Open decisions

1. **Session list retention.** Keep the minimal session picker, or always open
   the newest session? The plan keeps the picker because it is one endpoint and
   one component.
2. **`llm.delta` persistence.** The plan keeps deltas live-only and persists
   the full response. If post-hoc token replay is required, persist coalesced
   deltas as `llm.delta.batch` instead of one line per token.
3. **Judge prompt visibility.** The LLM judge is a normal `llm.request` with
   `pass: "verify.judge"`, so its full prompt is visible in the same viewer.
   Confirm this is acceptable; it may contain the process reply.
4. **Persona source.** The plan freezes identity/character values into
   `config/persona.toml` and prompt fragments. If those values should remain
   dynamic, a tiny static loader is the only replacement for `mm-being`.
5. **Retention default.** Keep all sessions until an operator prunes, or default
   to 30 days. The plan defaults to keep-all for a local single-user server.
6. **Legacy data import.** The plan starts `data/core/` empty. Importing the
   old RDF agentstream into JSONL would require a one-off tool built from the
   legacy tag; it is out of scope unless continuity is required.

---

## 14. Appendix: minimal `mm-cme` pipeline shape

```rust
pub async fn run(&self, req: TurnRequest) -> Result<TurnResult> {
    let log = self.events.open(&req.session_id).await?;
    log.append(Event::user_message(&req)).await?;
    log.append(Event::turn_started(&req)).await?;

    let ctx = self.context(&req).await?;
    let interpret = self.interpret(ctx.clone()).await?;

    if let Interpret::FastPath { reply } = interpret {
        log.append(Event::agent_message(&req, &reply, true)).await?;
        return self.finish(&log, &req, reply, 1, true).await;
    }

    let mut retry = None;
    for attempt in 1..=self.config.verify.max_retries + 1 {
        let process = self.process(ctx.clone(), &interpret, retry.clone()).await?;
        let results = self.verify_checks(&interpret.checks, &process).await?;
        let verdict = self.verify_llm(ctx.clone(), &process, &results, retry.clone()).await?;

        match verdict {
            Verdict::Approve => {
                self.commit_state(&log, &req).await?;
                log.append(Event::agent_message(&req, &process.reply, true)).await?;
                return self.finish(&log, &req, process.reply, attempt, true).await;
            }
            Verdict::Reject { reason } => {
                log.append(Event::pass_retry(&req, attempt, &reason, &results)).await?;
                retry = Some(RetryContext { reason, results });
            }
        }
    }

    log.append(Event::turn_failed(&req, "VERIFY_EXHAUSTED")).await?;
    Err(CmeError::VerifyExhausted)
}
```

This is the entire controllable flow. Everything in §7.7 that does not appear
in this function is archived.
