# LLM Activity Log — Backend Tap, Real-Time Feed, and Right-Pane Dock

> **Status:** proposed design · **Date:** 2026-11-23 · **Baseline:** `integration/cme-rdf-backend`
> **Scope:** `mm-llm`, `mm-serve`, `frontend/src/components/chat`, `frontend/src/lib/{api,store,wire}`,
> `frontend/src/hooks`, `config/`
> **Flags:** `MM_LLM_ACTIVITY` (default `true`)
>
> This document is a plan, not an implementation. It defines the contracts, the exact seams, the
> UI, and the build order. No code below ships until its phase is implemented and verified.
>
> **Implementation status (2026-11-23):** P0–P3 are implemented. `mm-llm::activity` provides
> `CallContext`, `ActivitySink`, `ObservedClient`, `TeeSink`, `DeltaPolicy`, and `scope`.
> `mm-serve` owns `LlmActivityFeed` (bounded ring + broadcast), wraps every provider construction
> and scripted client, scopes chat/controller/process calls, and serves
> `GET /api/llm-activity` and `GET /api/llm-activity/stream`. The frontend has the generated wire
> types, snapshot/SSE hook with reconnect and gap recovery, a bounded delta-merging store, and the
> `LlmLogDock` at the bottom of the active agent workspace pane. P3 adds filters, clear, pause/resume,
> collapse, persisted height, row expansion, jump-to-latest, the `Ctrl+\`` toggle, and a scripted
> live e2e.
>
> **Superseded for provider access (2026-11-24):** `ObservedClient`, `ActivitySink`, `TeeSink`,
> and the decorator-owned delta policy were retired by Phase 5 of
> `agentstream_llm_gateway_plan.md`. `CallContext`/`scope` now live in `mm_llm::call`, the gateway
> live bus owns delta coalescing, and `LlmActivityFeed` projects gateway events only. The dock,
> wire shape, filters, pause/clear/resize, and reconnect behavior below are unchanged.

---

## 0. Reading this document

| If you want to know… | Read |
|---|---|
| What already exists and why it is not enough | §2 |
| The target architecture in one page | §3 |
| The event model and wire contract | §4 |
| Where the backend tap lives and how context is attached | §5 |
| How the feed, snapshot, and SSE route work | §6 |
| How the frontend consumes the feed | §7 |
| What the dock looks like and where it sits | §8 |
| What is redacted, truncated, and never sent | §9 |
| Retention and restart behavior | §10 |
| What to build, in what order | §11 |
| How it is tested and accepted | §12–§13 |
| Risks and decisions | §14 |

**Vocabulary.**

- **LLM activity** — one observable moment in a model call: request opened, stream fragment,
  reply finished, or call failed. Activity is *diagnostic*: it is not the chat transcript and not
  the audit ledger.
- **Feed** — the in-process, bounded, sequenced buffer plus broadcast that carries activity to
  the browser. Ephemeral by design (§10).
- **Dock** — the logging section at the bottom of the right pane.
- **Call scope** — the `(source, sessionId, runId, pass, round)` tuple attached to every call so a
  row can be correlated with the chat turn that caused it.

---

## 1. Executive summary

The backend already makes every model call through one trait (`LlmClient`) and already records
accounting in `llm_calls`, audited `llm.*` log records, and chat `pass_*` frames. None of those is
a live, readable, all-calls view:

- the audit log records the request but not the streamed text or the reply, and it is durable
  rather than live;
- chat pass frames only exist inside a chat SSE response, only for CME turns, and never cover
  summarizer, agent, automation, process, or startup calls;
- the inspector's logs view is cursor-paginated history, not a tail.

This plan adds a thin observation layer at the **only door to a model**:

1. **`mm-llm` grows an opt-in `ActivitySink` and a decorator** that wraps any `LlmClient` and emits
   `request`, `delta`, `response`, and `error` activity with no change to call semantics.
2. **`mm-serve` owns a bounded `LlmActivityFeed`** (ring buffer + `broadcast`), wraps every client
   it constructs, and exposes a snapshot endpoint plus a resumable SSE stream.
3. **The right pane gains a bottom dock** (`LlmLogDock`) that hydrates from the snapshot, tails the
   stream, merges deltas into their request row, and renders a compact, filterable, pausable log.

The whole feature is local-only, best-effort, and non-blocking: an absent or slow observer can
never fail, delay, or reorder a model call.

---

## 2. Baseline: what actually exists

| Fact | Where |
|---|---|
| Every provider call is behind `LlmClient` (`complete`, `complete_stream`) | `crates/mm-llm/src/client.rs` |
| `LlmService` accounts and audits calls after the fact | `crates/mm-llm/src/service.rs` |
| The service emits `llm.route.select`, `llm.request`, `llm.accounting.commit` records | `service.rs` (`call_routed_with`, `emit_commit`) |
| Ledger rows carry call id, purpose, model, tokens, latency, status | `llm_calls`, `mm-llm/src/accounting.rs` |
| Chat pass/round/`pass_token` frames exist for CME turns | `crates/mm-serve/src/cme_stream.rs`, `dto/chat.rs` |
| SSE frames are `id/event/data` with resumable sequence | `crates/mm-serve/src/sse.rs` |
| A per-session broadcast + SSE + rAF-batched client already exists | `status_service.rs`, `frontend/src/hooks/use-status-stream.ts` |
| The right pane is `ChatPane`; the split is document-left/chat-right | `frontend/src/routes/workspace.tsx`, `components/shell/workspace-split.tsx` |
| The chat pane ends with transcript → suggestions → composer | `frontend/src/components/chat/chat-pane.tsx` |
| Clients are constructed in several places: `resolve_client`, summarizer, mutation tools, agent runtime | `controller/mod.rs:75`, `state.rs:235`, `controller/tools/mutate.rs:237` |

### 2.1 Gaps this plan closes

1. **No cross-cutting live view.** Audit records are deliberately structured and durable, not a
   tail; chat frames only exist while a chat response is open.
2. **No reply/stream in the records.** `llm.request` carries routing metadata and a prompt hash,
   not the text a streaming call emitted.
3. **No correlation to the UI turn.** `trace_id` is optional and often unset; there is no
   `sessionId`/`runId` on a call.
4. **No UI surface.** The inspector has history, but the operator's right pane has no live
   model-activity area.

### 2.2 What must not change

- `LlmClient` call semantics, caching, routing, repair, accounting, and ledger rows.
- `wireVersion: 1` and every existing `ChatFrame`.
- The audit log's content or ordering.
- The chat stream's token pacing (the dock is a second consumer, never the source of reply text).

---

## 3. Target architecture

```
                      ┌──────────────────────────── mm-serve ────────────────────────────┐
 controller / CME ───►│ ObservedClient (decorator) ──► LlmService ──► OpenAiClient       │
 agent / automation ─►│        │                                                          │
 summarizer ─────────►│        ├─ request / delta / response / error events                │
                      │        ▼                                                          │
                      │  LlmActivityFeed  (ring buffer, seq, broadcast)                   │
                      │        │                                                          │
                      │        ├─► GET /api/llm-activity            (snapshot, cursor)    │
                      │        └─► GET /api/llm-activity/stream     (SSE, resumable)      │
                      └────────┼──────────────────────────────────────────────────────────┘
                               │
                    frontend   ▼
                      use-llm-activity-stream (snapshot → EventSource, rAF batch, dedupe)
                               │
                          llm-log store (bounded, delta merge, filters, paused buffer)
                               │
                     LlmLogDock  — bottom of ChatPane, above nothing, below Composer
```

Three invariants:

1. **Observation cannot affect the call.** The decorator forwards the response object it received;
   the sink is synchronous and best-effort. No sink error is propagated.
2. **The feed is a projection, not a second source of truth.** Dropping, capping, or evicting
   activity never changes chat, accounting, or audit.
3. **One sequence per feed.** The browser dedupes and resumes by `seq`, exactly as chat and status
   do.

---

## 4. Wire contract (additive, `wireVersion: 1`)

### 4.1 `LlmActivityEvent`

One flat, JSON-friendly shape. Discriminated by `kind`; absent fields are omitted.

```rust
// crates/mm-serve/src/dto/llm_activity.rs (ts-rs generated to lib/wire/llm_activity.ts)
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmActivityEvent {
    Request {
        seq: u64,
        at: i64,                    // epoch ms
        call_id: String,            // ULID; held across the call's events
        trace_id: Option<String>,
        source: String,             // "chat" | "agent" | "automation" | "process" | "summarize" | "other"
        session_id: Option<String>,
        run_id: Option<String>,     // chat request id
        pass: Option<String>,       // interpret | process | verify | think
        round: Option<u32>,
        purpose: String,            // Purpose::as_str
        provider: String,
        model: Option<String>,      // requested or routed model, whichever is known
        streamed: bool,
        requested: bool,            // false for cache replay
        message_count: u32,
        tool_count: u32,
        preview: String,            // redacted, truncated user excerpt (§9)
    },
    Delta {
        seq: u64,
        at: i64,
        call_id: String,
        text: String,               // coalesced fragment, already capped
        chars: u32,                 // total chars emitted so far for this call
        truncated: bool,            // true once the preview cap was reached
    },
    Response {
        seq: u64,
        at: i64,
        call_id: String,
        model: String,
        status: String,             // "ok" | "cached" | "schema_rejected"
        cached: bool,
        tokens_in: u32,
        tokens_out: u32,
        latency_ms: u64,
        text: String,               // redacted, truncated reply preview
        tool_calls: u32,
    },
    Error {
        seq: u64,
        at: i64,
        call_id: String,
        status: String,             // error kind, e.g. "provider", "timeout", "schema_rejected"
        error: String,              // redacted message
        latency_ms: u64,
        partial_chars: u32,         // delta text emitted before the failure
    },
}
```

Notes:

- `request` is emitted before routing resolution when only the requested model is known; the
  `response` carries the answering model. The dock prefers `response.model`.
- A cache hit emits `request` (`requested: false`) then `response` with `cached: true`; no deltas.
- `delta` events are diagnostic. The authoritative reply text remains the chat final frame.

### 4.2 SSE encoding

Reuse `crates/mm-serve/src/sse.rs` unchanged.

- `id: <seq>`
- `event: activity` for an event; `event: resync` when the cursor predates the ring; `: ping`
  heartbeat every 15 s.
- `GET /api/llm-activity/stream?from=<seq+1>` resumes; `Last-Event-ID` is honored as a fallback.
- A `resync` payload is `{ "oldestSeq": n, "events": LlmActivityEvent[] }` and is rendered by the
  same ingest path, after clearing rows older than `oldestSeq`.
- `GET /api/llm-activity?from=&limit=&kind=&sessionId=` returns
  `{ "oldestSeq": n, "nextSeq": m, "events": [...] }` for initial hydration. `limit` clamps to
  1..=500.

### 4.3 Endpoint placement

Add both routes in `crates/mm-serve/src/routes/mod.rs` beside the other streams:

```rust
.route("/api/llm-activity", get(llm_activity::snapshot))
.route("/api/llm-activity/stream", get(llm_activity::stream))
```

They inherit the localhost CORS allowlist, security headers, and access logging middleware. No new
CORS headers are needed: `EventSource` sends no custom headers, and `Last-Event-ID` is already in
the allow-list.

---

## 5. Backend seam A — `mm-llm` observation

### 5.1 New module `crates/mm-llm/src/activity.rs`

```rust
/// Why and where a call was made. Attached by the caller, never guessed by the adapter.
#[derive(Debug, Clone, Default)]
pub struct CallContext {
    pub source: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub pass: Option<String>,
    pub round: Option<u32>,
}

/// Receives activity for display. Implementations must be cheap and non-blocking.
pub trait ActivitySink: Send + Sync {
    fn request(&self, call_id: Ulid, ctx: &CallContext, req: &LlmRequest);
    fn delta(&self, call_id: Ulid, text: &str, total_chars: u32, truncated: bool);
    fn response(&self, call_id: Ulid, latency_ms: u64, res: &LlmResponse);
    fn error(&self, call_id: Ulid, latency_ms: u64, partial_chars: u32, err: &LlmError);
}
```

`ObservedClient`:

```rust
pub struct ObservedClient {
    inner: Arc<dyn LlmClient>,
    sink: Arc<dyn ActivitySink>,
    context: CallContext, // default when no scoped context is present
}

#[async_trait]
impl LlmClient for ObservedClient {
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        let call_id = accounting::new_call_id();
        let ctx = current_context().unwrap_or_else(|| self.context.clone());
        let started = Instant::now();
        self.sink.request(call_id, &ctx, req);
        match self.inner.complete(req).await {
            Ok(res) => { self.sink.response(call_id, ms(started), &res); Ok(res) }
            Err(err) => { self.sink.error(call_id, ms(started), 0, &err); Err(err) }
        }
    }

    async fn complete_stream(&self, req: &LlmRequest, sink: &mut dyn DeltaSink)
        -> Result<LlmResponse>
    {
        // Same skeleton, but wraps `sink` in a `TeeSink` that forwards every delta to the
        // caller first and then reports a coalesced copy (§5.3). The final response is
        // reported exactly as in `complete`.
    }
}
```

- `provider()` and `decoder()` delegate to the inner client.
- `complete_stream`'s default implementation on `LlmClient` would report the whole answer as one
  delta; the decorator calls `inner.complete_stream`, so adapters that stream stay streaming.
- The decorator never retries, caches, or validates. It is a pass-through.

### 5.2 Call-scope propagation

`mm-llm` exposes a `tokio::task_local!` named `CALL_CONTEXT` with:

```rust
pub fn scope<F>(ctx: CallContext, future: F) -> impl Future<Output = F::Output>
pub fn current_context() -> Option<CallContext>
```

`CallContext` is `Clone`; the scope is entered where the work is owned:

| Caller | Scope site |
|---|---|
| CME chat turn | `routes/cme.rs`, around the spawned `run_turn` (the `tokio::spawn` at ~`:225`) |
| Legacy controller turn | `controller/turn.rs` at the per-request entry |
| Process step | `process_runner.rs`, around one step execution |
| Agent / automation | around the runtime call site; `source` names the subsystem |
| Summarizer / startup / one-off | no scope; the wrapper is built with a default `CallContext { source: "summarize" }` |

Rules:

- A nested `metamind.think` pass sets `pass = "think"` and inherits `sessionId`/`runId`.
- A scope that is absent yields the wrapper's default context, so no call is ever unattributed.
- Spawned work must be scoped explicitly. Where a subsystem spawns and loses the scope, its client
  is constructed with the correct default source; both paths are correct, one is richer.

### 5.3 Delta coalescing

`TeeSink` keeps `total_chars` and a small `String` buffer. It forwards every delta synchronously to
the real sink, then:

- appends to its buffer;
- when the buffer reaches **256 chars** or **50 ms** have elapsed since the last flush, calls
  `sink.delta(call_id, buffer, total_chars, truncated)` and clears it;
- stops reporting at **2,000 chars**, setting `truncated = true` on the last fragment and ignoring
  the rest.

The final response always reports full token usage and latency. The dock's preview cap is
independent of the model's output length; the full text remains in the chat and, where applicable,
the ledger/agentstream.

### 5.4 `LlmService` interaction

`ObservedClient` sits **outside** `LlmService` so it sees cache hits, schema rejections, and
repairs exactly once per provider-facing call. It emits its own `call_id`; `LlmService` keeps its
own internal id for the ledger. The two are correlated by the `response` event's target fields
(purpose/model/latency) and, in a later phase, by passing the observer's call id through
`LlmRequest.trace_id` when that field is empty. Do **not** reuse the ledger id as the activity id in
P0: the service generates it internally after routing.

---

## 6. Backend seam B — `mm-serve` feed and routes

### 6.1 `crates/mm-serve/src/llm_feed.rs`

```rust
pub struct LlmActivityFeed {
    capacity: usize,                    // MM_LLM_ACTIVITY_MAX_EVENTS, default 2_000
    next_seq: AtomicU64,
    ring: Mutex<VecDeque<LlmActivityEvent>>,
    tx: broadcast::Sender<(u64, LlmActivityEvent)>, // capacity 256
    preview: PreviewPolicy,             // request/reply caps, full-prompt opt-in
}

impl LlmActivityFeed {
    pub fn publish(&self, event: LlmActivityEvent);          // assign seq, push, broadcast
    pub fn snapshot(&self, from: Option<u64>, limit: usize) -> Snapshot;
    pub fn subscribe(&self) -> broadcast::Receiver<(u64, LlmActivityEvent)>;
}
```

- `publish` is synchronous, takes the ring lock briefly, and calls `broadcast::Sender::send`
  without awaiting. `Err(SendError)` (no subscribers) is ignored.
- Ring eviction is by count, not time. `oldest_seq` is reported so clients can detect a gap.
- `impl ActivitySink for LlmActivityFeed` maps the `mm-llm` callbacks into `LlmActivityEvent`s,
  applies redaction/truncation (§9), and publishes.
- No `await` and no `Logger` call inside the sink: a full audit queue must not stall a model call,
  and feed -> logger recursion is impossible by construction.

### 6.2 `AppState` wiring

- New field: `pub llm_activity: Arc<LlmActivityFeed>`; built in `boot_with_flags` before any client.
- New helper:

  ```rust
  pub fn observed(self: &Arc<Self>, client: Arc<dyn LlmClient>, ctx: CallContext)
      -> Arc<dyn LlmClient>
  ```
- Replace every direct client construction used by the server:
  - `controller::resolve_client` (`controller/mod.rs:75`) — controller, CME executor, process
    engine;
  - `summarizer_client` (`state.rs:235`);
  - `controller/tools/mutate.rs:237`;
  - the agent runtime's client (pass the decorated client into `AgentRuntime`).
- Add a compile-time guard test: `OpenAiClient::new` may appear only inside the observed-client
  construction path in `mm-serve`. This is the same style as the existing "endpoint is never in
  source" source-scan test in `mm-llm`.

### 6.3 Feature flag

Add to `feature_flags.rs`:

| Variable | Default | Effect |
|---|---|---|
| `MM_LLM_ACTIVITY` | `true` | build the feed and wrap clients |
| `MM_LLM_ACTIVITY_MAX_EVENTS` | `2000` | ring capacity |
| `MM_LLM_ACTIVITY_PREVIEW_CHARS` | `4000` | reply preview cap |
| `MM_LLM_ACTIVITY_DELTA_MS` | `50` | delta coalescing window |

When disabled, `AppState::observed` returns the inner client unchanged, the routes answer
`503 unavailable` with a reason, and no feed is built. One `if` at boot, no per-call branch.

---

## 7. Frontend transport and state

### 7.1 Generated wire types

- Source of truth: `crates/mm-serve/src/dto/llm_activity.rs`, re-exported from `dto/mod.rs`.
- Regenerate with `cargo test -p mm-serve export_types -- --ignored` (same as `chat.ts`).
- Output: `frontend/src/lib/wire/llm_activity.ts`. Never edit by hand.

### 7.2 `frontend/src/lib/api/llm-activity.ts`

```ts
export async function fetchLlmActivity(params: {
  from?: number; limit?: number; kind?: string; sessionId?: string;
}): Promise<LlmActivitySnapshot>;

export function llmActivityStreamUrl(from: number): string {
  return `/api/llm-activity/stream?from=${from}`;
}
```

### 7.3 `frontend/src/lib/store/llm-log.ts` (zustand, mirrors `store/status.ts`)

State:

```ts
type LlmLogState = {
  events: LlmActivityRow[];          // bounded, newest last; max 500
  oldestSeq: number | null;
  lastSeq: number;
  connection: "idle" | "connecting" | "live" | "reconnecting" | "error";
  error: string | null;
  paused: boolean;
  pausedCount: number;
  filter: "all" | "requests" | "streams" | "responses" | "errors";
  collapsed: boolean;                // persisted in `prefs`
  height: number;                    // persisted in `prefs`
};
```

Actions:

- `ingest(seq, event)` — ignore `seq <= lastSeq`; for `delta`, merge into the open row for
  `callId` (append text up to the cap, update `chars`, set `truncated`); otherwise append a row and
  evict the oldest beyond 500.
- `resync(snapshot)` — replace rows at or below `oldestSeq`, then ingest.
- `clear()` — drop rows, keep `lastSeq` so the stream does not replay.
- `setPaused`, `setFilter`, `toggleCollapsed`, `setHeight`.

### 7.4 `frontend/src/hooks/use-llm-activity-stream.ts`

Same shape as `use-status-stream.ts`, with these differences:

1. **Hydrate first.** `fetchLlmActivity({ limit: 200 })` -> `resync`.
2. **Tail.** `EventSource(llmActivityStreamUrl(lastSeq + 1))`; listen to `activity`, `resync`,
   and `error`.
3. **Batch.** Enqueue frames and flush once per animation frame (fallback `setTimeout 0`), so a
   token-heavy turn cannot re-render per delta.
4. **Gap recovery.** If an incoming `seq > lastSeq + 1`, fetch a snapshot and `resync` rather than
   showing a hole.
5. **Visibility.** Pause the connection while the tab is hidden; on resume, reconnect from
   `lastSeq + 1`.
6. **Paused UI.** While `paused`, ingest still advances `lastSeq` but only into a bounded buffer
   (e.g. 200 rows); the visible list is frozen and `pausedCount` increments. Resume swaps the
   buffer in. This keeps the cursor correct.
7. **Reconnect.** `onerror` closes the source, sets `reconnecting`, retries after 1.8 s with the
   current cursor. A malformed frame is ignored.

The hook is mounted once for the whole workspace, not once per session, so activity from agent and
automation calls is not lost while the user is looking at another pane.

---

## 8. UI — the dock at the bottom of the right pane

### 8.1 Placement

`ChatPane` today is:

```
<ChatHeader />
[notice]
<transcript flex-1 />
<SuggestionChips />
<Composer />
```

Add the dock **after `<Composer />`**, inside the same `<section data-chat-pane>`:

```
<ChatHeader />
[notice]
<transcript flex-1 />
<SuggestionChips />
<Composer />
<LlmLogDock />   ← bottom of the right pane
```

This is literally the bottom of the right pane, keeps the composer adjacent to the transcript, and
does not require changes to `WorkspaceSplit` or the pane store. The dock is a fixed-height
`shrink-0` sibling; it never participates in the transcript's scroll area.

Narrow/monitor tiers: the dock stays inside the chat pane. When the chat tab is hidden the pane is
`hidden`, so the dock disappears with it; the stream hook lives at workspace level and keeps the
cursor current.

Fullscreen chat: the dock remains at the bottom of the chat panel.

### 8.2 Anatomy

```
┌ LLM activity ── ● live ─────────────────────────────────────────────────────────┐
│ [All ▾] [Pause] [Clear] [⌃]                                                     │
├─────────────────────────────────────────────────────────────────────────────────┤
│ 14:32:07  REQ   interpret  gpt-4o-mini  "summarize the migration order…"        │
│ 14:32:07  REQ   process    claude-…     4 msgs · 31 tools · stream              │
│ 14:32:08  STR   process    claude-…     "The migration order is fixed…" 1.2k    │
│ 14:32:10  RSP   process    claude-…     812 in / 204 out · 2.4s · ok            │
│ 14:32:11  ERR   verify     —            schema_rejected · 1.9s                  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

Header (30 px, always visible):

- title `LLM activity`;
- connection dot + word (`live`, `reconnecting`, `offline`); `title` carries the error;
- filter `<select>`: All / Requests / Streams / Responses / Errors;
- Pause/Resume, Clear, Collapse/Expand;
- optional count `1,204 events` (suppressed on narrow).

Body (default 168 px, min 96 px, max 40% of the pane):

- rows in a monospace, `text-xs` list, one line each, `overflow-hidden`, newest at the bottom;
- columns: `HH:MM:SS`, kind badge (REQ/STR/RSP/ERR), purpose (or source for non-chat), model
  (`—` when unknown), summary;
- request summary: redacted first-line preview + `n msgs · n tools`, plus `cache` / `stream`
  markers;
- delta summary: the coalesced text, single line, with a character count; the row mutates in place
  until its `response`/`error` arrives;
- response summary: `tokens in / tokens out · latency · status`; expand shows the reply preview;
- error summary: status + message; the row is tinted with the danger token;
- clicking a row expands it inline (details: ids, model, purpose, latency, full capped preview,
  tool-call count, error). No navigation, no modal;
- `role="log"`, `aria-live="off"` (a token stream must not be announced); the header announces
  connection changes via `aria-live="polite"`;
- auto-scroll only while the viewport is pinned to the bottom; scrolling up pauses auto-scroll and
  shows a "jump to latest" control;
- escape all text; render as plain text, never Markdown or HTML.

Resize: a 4 px drag handle on the dock's top edge writes `height` to the prefs store, clamped and
debounced. A double-click on the header toggles collapse.

Collapsed state: only the 30 px header remains, with an unread/error count badge.

### 8.3 Files

| File | Purpose |
|---|---|
| `frontend/src/components/chat/llm-log-dock.tsx` | Dock shell, header, resize, collapse, empty state |
| `frontend/src/components/chat/llm-log-row.tsx` | One row, expand/collapse, kind styling |
| `frontend/src/components/chat/llm-log-filters.tsx` | Filter select and kind counts (optional split) |
| `frontend/src/hooks/use-llm-activity-stream.ts` | Snapshot + SSE + rAF batching |
| `frontend/src/lib/api/llm-activity.ts` | Fetch and URL helpers |
| `frontend/src/lib/store/llm-log.ts` | Bounded event store and merge logic |
| `frontend/src/lib/wire/llm_activity.ts` | Generated types |
| `frontend/src/agentgui/components/workspace/workspace-pane.tsx` | Mount `<LlmLogDock />` |
| `frontend/src/routes/workspace.tsx` | Call `useLlmActivityStream()` once, so the cursor survives pane switches and the chat pane unmounting |
| `frontend/src/lib/store/prefs.ts` | Persist `collapsed` and `height` |
| `frontend/src/components/shell/keyboard-map.tsx` | `Ctrl+\`` toggles the dock |

### 8.4 Visual language

- Reuse existing semantic tokens; add no palette:
  - REQ — `accent` / info tint;
  - STR — `muted-foreground`;
  - RSP — success tint;
  - ERR — destructive tint.
- Micro-label styling for the kind badges; Geist Mono for timestamps, models, and counts.
- No shadows, 1 px divider above the header, existing radii. Follows `DESIGN_INDEX` §XI.A.
- Motion: none beyond the existing 120–240 ms collapse; honor `prefers-reduced-motion`.

---

## 9. Data policy: redaction, truncation, retention

The dock is local-only, but prompts can still contain secrets, personal data, and provider keys.
The feed applies policy **server-side** before publish; the browser never receives more than the
policy allows.

1. **Never send credentials.** `OpenAiClient` already redacts its env in `Debug`; the feed must not
   serialize headers, URLs, or keys. `mm-llm/src/redact.rs` and `mm-log/src/redact.rs` are reused
   for patterns (bearer tokens, `sk-…`, keyed query params).
2. **Request preview = last user message excerpt**, capped at 240 chars by default. The system
   prompt and full message list are available only with
   `MM_LLM_ACTIVITY_PROMPTS=full` (documented as local-only, never a default). Counts
   (`message_count`, `tool_count`) are always available.
3. **Reply preview** capped at `MM_LLM_ACTIVITY_PREVIEW_CHARS` (default 4,000), with an explicit
   `truncated` marker. Delta text stops at 2,000 chars per call.
4. **Tool arguments** are never included in the feed; only the tool-call count. Tool results remain
   in the chat/trace surfaces where they are already governed.
5. **Control characters and newlines** are escaped/normalized to one line in the row; expansion
   preserves line breaks but stays escaped text.
6. **Errors** carry the redacted `LlmError` display string, not a provider body. Provider bodies can
   echo the prompt; they do not reach the feed.
7. **Retention:** in memory only, ring-bounded, discarded on process exit. No new file, no
   telemetry, no cross-process sharing.
8. **Access:** the existing localhost bind and CORS allowlist are the boundary. No new auth is
   introduced; the endpoint is exactly as reachable as `/api/chat`.

---

## 10. Retention and restart behavior

The feed is **ephemeral by design**. It is a live tail, bounded by the ring and discarded on
process exit. After a restart the dock starts empty; there is no history endpoint, no backfill from
`llm_calls`, and no on-disk activity log.

Deltas and prompt previews are never persisted. Durable inspection already belongs to the audit
log, `llm_calls`, and agentstream, each with its own retention policy; the dock does not duplicate
or resurface them.

---

## 11. Delivery phases

### P0 — `mm-llm` observation primitive

- Add `activity.rs`: `CallContext`, `ActivitySink`, `ObservedClient`, `TeeSink`, `scope`.
- Export from `lib.rs`. No call site changes yet; `ObservedClient` is unused and behavior is
  identical.
- Tests: ordering (`request` → deltas → `response`), error path, cache-hit path, delta
  coalescing/cap, context fallback, provider/decoder delegation.

**Exit:** `cargo test -p mm-llm`; no production behavior changes.

### P1 — `mm-serve` feed, wiring, routes

- Add `llm_feed.rs`, `dto/llm_activity.rs`, `routes/llm_activity.rs`; register routes.
- Add `AppState.llm_activity` and `AppState::observed`; route every client construction through it.
- Add flag and env knobs.
- Scope chat/controller/process/agent call sites.
- Tests: feed sequence/eviction/snapshot; SSE resume and resync; flag-off 503; scripted-client
  request produces a `request` row; streaming mock produces deltas and a response.

**Exit:** `curl -N /api/llm-activity/stream` shows live rows during a chat turn.

### P2 — frontend transport and dock

- Generate wire types; add api/store/hook.
- Add `LlmLogDock` and row components; mount in `ChatPane`.
- Empty, loading, live, reconnecting, and error states.
- Tests: store merge/dedupe/bound; hook snapshot + reconnect with a fake `EventSource`; dock render
  from fixtures; paused buffering.

**Exit:** a chat turn appends rows in real time; a disconnect leaves the panel honest and resumes.

### P3 — polish

- Filters, clear, pause/resume, collapse, keyboard toggle, resize persistence, jump-to-latest.
- Row expansion with ids and capped previews.
- Reduced-motion and narrow-tier checks.
- E2E: one scripted run, assert REQ/STR/RSP order and no layout shift in the transcript.

**Exit:** §13 acceptance criteria pass.

---

## 12. Test plan

### 12.1 Rust

| Test | What it proves |
|---|---|
| `observed_client_reports_request_then_deltas_then_response` | Ordering and pass-through equality |
| `observed_client_reports_error_and_keeps_the_error` | Failure is observed, not swallowed or wrapped |
| `stream_deltas_are_coalesced_and_capped` | Volume bound and `truncated` correctness |
| `cache_hit_emits_request_and_cached_response_without_deltas` | Cache semantics preserved |
| `context_falls_back_when_no_scope_is_entered` | Every call is attributed |
| `feed_evicts_oldest_and_reports_oldest_seq` | Ring semantics |
| `stream_resumes_from_cursor_and_resyncs_on_gap` | SSE `from`/`resync` |
| `feed_disabled_returns_503_and_never_wraps` | Kill switch |
| `preview_redacts_patterns_and_caps_length` | §9 policy |
| `no_direct_openai_client_construction_in_mm_serve` | The observed path is the only path |

Commands: `cargo test -p mm-llm`, `cargo test -p mm-serve llm_activity`,
`cargo test -p mm-serve export_types -- --ignored`.

### 12.2 Frontend

| Test | What it proves |
|---|---|
| `llm-log store dedupes by seq` | Reconnect cannot duplicate rows |
| `delta merges into its open request row` | Streaming row behavior |
| `ring bound evicts oldest row` | Memory bound |
| `paused buffering keeps lastSeq advancing` | Pause does not lose the cursor |
| `hook hydrates then tails, batching per frame` | Snapshot + SSE + rAF |
| `dock renders kinds and expands a row` | UI contract, escaping |
| `no layout shift when rows arrive` | Transcript is untouched |

Commands: `npm test -- --run llm-log`, `npm run test:e2e -- llm-activity`.

### 12.3 Manual performance check

Run one three-pass turn with a streaming model and record:

- events/sec at the feed and at the dock;
- main-thread frame budget during streaming;
- chat turn latency with the feed on vs. off (target: within noise; the sink is synchronous and
  lock-brief).

Do not claim an overhead number without this measurement.

---

## 13. Acceptance criteria

1. During any backend model call — chat, agent, automation, process, or summarizer — a `request`
   row appears in the dock within one animation frame of the call starting.
2. A streaming call shows one row that grows in place; a non-streaming call shows request then
   response.
3. A failure shows an `ERR` row with a redacted reason; the chat's own error handling is unchanged.
4. `from=` resume and `Last-Event-ID` never duplicate or reorder rows.
5. The dock is at the bottom of the right pane in wide, medium, narrow, fullscreen, and swapped
   layouts, and the transcript does not resize or scroll when rows arrive.
6. Collapse, pause, clear, filter, and resize survive a reload (prefs) and do not break the stream
   cursor.
7. No prompt, key, header, tool argument, or provider body reaches the browser beyond §9.
8. With `MM_LLM_ACTIVITY=false`, the server works exactly as today and the routes explain why they
   are unavailable.
9. `wireVersion` stays `1`; existing chat frames and the status stream are untouched.
10. The chat reply text still comes only from the existing final frame; the dock is never a source
    of reply text.

---

## 14. Risks, decisions, open questions

### 14.1 Decisions

| # | Decision | Why |
|---|---|---|
| D1 | Tap at `LlmClient`, not at the logger or `llm_calls` | Only the client sees request, deltas, and reply; the logger has no reply text and is durable/async |
| D2 | Decorator outside `LlmService` | Sees cache hits and final outcomes once per call; leaves routing/repair untouched |
| D3 | Separate `/api/llm-activity` stream, not chat frames | Agent/automation/summarizer calls have no chat response; the dock must outlive one chat run |
| D4 | In-memory ring, no disk, no backfill | A live tail does not justify a second durable log; durable inspection stays with audit/`llm_calls`/agentstream |
| D5 | Flat event type, additive to `wireVersion: 1` | One parser, one store; no chat contract changes |
| D6 | Server-side redaction and caps | The browser should never be the place policy is enforced |
| D7 | Delta coalescing server-side *and* rAF batching client-side | Two independent bounds on render churn |
| D8 | Dock inside `ChatPane` after `Composer` | Literal "bottom of the right pane", no workspace-split changes |

### 14.2 Risks

| Risk | Mitigation |
|---|---|
| Token volume overwhelms the UI | 50 ms / 256-char server coalescing, 2,000-char call cap, 500-row client ring, rAF batching |
| Broadcast lag under load | `broadcast` + drop-on-lag; activity is lossy by design, and `oldestSeq` makes gaps explicit |
| Prompt/PII leakage | §9 defaults: excerpt only, no full prompt, no tool args, redacted errors |
| Observer slows a call | Synchronous, non-awaiting sink with a brief ring lock; never propagate sink failures |
| Duplicate rows with chat pass frames | Different surface and sequence; the dock never renders into the transcript |
| Missing context on spawned work | `CallContext` fallback per wrapper plus explicit scoping of chat/process tasks |
| Prompt previews change cache keys | Activity reads the request; it never mutates or re-serializes it |
| Endpoint reachable beyond localhost | Same bind/CORS boundary as `/api/chat`; no new exposure |

### 14.3 Open questions

1. Should `request` include the full message list when `MM_LLM_ACTIVITY_PROMPTS=full`, or is the
   user excerpt plus counts enough for debugging? Start with excerpt; revisit after dogfooding.
2. Is 2,000 events / 500 rows the right default? Keep configurable; tune with the §12.3
   measurement.
3. Should the dock filter by the current chat session by default, or show all backend calls? The
   plan shows all, with an optional session filter; agent/summarizer traffic is often the thing
   being debugged.
4. Do we need a "copy row as JSON" action for bug reports? Deferred until the row shape stabilizes.
5. Should the dock be available outside chat (e.g. inspector)? The store and hook are
   workspace-level, so a second mount is cheap later; not in this plan.

---

## 15. Traceability

| Requirement | Workstream | Test |
|---|---|---|
| Log every backend LLM request | §5, §6.2, P1 | scripted client -> `request` row |
| Log streams in real time | §5.1, §5.3, §7.4 | streaming mock + SSE + dock growth |
| Log replies | §5.1, §4.1 | `response` row with tokens/latency/preview |
| Real-time feed from backend | §4.2, §6.1, §7.4 | resume/resync tests |
| Section at bottom of right pane | §8.1 | wide/narrow/fullscreen layout checks |
| All calls, not only chat | §2.1, §5.2, §6.2 | agent/summarizer rows with `source` |
| Safe to ship off | §6.3 | flag-off 503 + unchanged chat |
| Local-only, redacted | §9 | redaction/cap tests |

---

## 16. Documentation updates when implemented

- `design/DESIGN_INDEX.md`: link this plan under XI.F and the source-document map.
- `infodocs/operator-guide.md`: what the dock shows, the flags, and that activity is ephemeral.
- `infodocs/components-and-status.md`: `ActivitySink`, `ObservedClient`, the feed, and the route.
- `README.md` / `frontend/README.md`: the bottom dock and the `Ctrl+\`` shortcut, if shortcuts are
  documented there.
