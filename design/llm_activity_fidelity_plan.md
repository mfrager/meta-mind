# LLM Call Fidelity & Activity Dock v2 — Two-Phase Plan

> **Status:** proposed design · **Date:** 2026-11-24 · **Baseline:** `integration/cme-rdf-backend`
> **Scope:** `mm-llm`, `mm-llm-agentstream`, `mm-serve`, `mm-cme`, `mm-agent`, `mm-process`,
> `config/`, `frontend/src/components/chat`, `frontend/src/lib/{store,wire}`
> **Predecessors:** `llm_activity_log_plan.md` (dock and feed), `agentstream_llm_gateway_plan.md`
> (single provider door)
>
> This is a plan, not an implementation. It names the exact seams, the wire changes, the UI, and
> the order of work. Nothing below ships until its phase is implemented and verified.

---

## 0. Reading this document

| If you want to know… | Read |
|---|---|
| What the reported failure actually is | §2.1 |
| Why streaming does not already avoid it | §2.2 |
| Every 30-second ceiling that can cut a model call | §2.3 |
| What the dock currently hides | §2.4 |
| The target state in one page | §3 |
| Phase 1 (backend: budgets, retry, timeouts, attribution, roll-up) | §4 |
| Phase 2 (dock: grouped rows, RSP roll-up, text tab) | §5 |
| Additive wire changes | §6 |
| What is never sent to the browser | §7 |
| Acceptance criteria | §8 |
| Risks and decisions | §9 |
| Requirement → workstream → test | §10 |

**Vocabulary.**

- **Reasoning-only answer** — a provider completion whose visible `content` is empty but whose
  `reasoning_content` is not, usually with `finish_reason: "length"`.
- **Budget** — the `max_tokens` room granted to a call. Not to be confused with `mm-cme`'s
  per-turn `BudgetClass` wall clock.
- **Call group** — one request plus its outcome (and, for a retry, one nested child group).
- **Roll-up** — the full streamed body attached to the terminal `response`/`error` event instead
  of being left as a trail of capped `delta` rows.

---

## 1. Executive summary

The reported failure is a **budget** failure, not a transport failure: the endpoint spent the
whole 1,024-token completion budget on reasoning, emitted no visible text, and the adapter
reported it as `LlmError::Transport`, which is why the operator sees
`llm transport error: … (finish_reason: length)`. Because the streaming path also ignores
`reasoning_content`, it treats the empty stream as "the proxy ignored `stream`" and re-issues the
call as a second, non-streaming provider round — doubling the cost and producing the same error.

Three independent fixes are needed, and they belong together because each one hides the others:

1. **Give the call a real budget** per purpose, and make a reasoning-only outcome a typed,
   retryable event instead of a generic transport error.
2. **Remove the 30-second wall clock** from the model path. A call should be bounded by progress
   (an idle timeout), not by a fixed ceiling that a thinking model cannot meet.
3. **Make the activity dock tell the truth**: the RSP line must carry the whole streamed body,
   outcomes must sit *under* their request, and the request must say what triggered it.

Phase 1 is entirely backend and ships the diagnosis into the existing wire shape. Phase 2 is
entirely frontend and changes how rows are grouped, rolled up, and read — including a second,
text-only tab with exactly three lines per block.

---

## 2. The failure, in detail

### 2.1 Where the message comes from

`crates/mm-llm/src/openai.rs:781-799` (`OpenAiClient::parse_response`):

```rust
if text.trim().is_empty() && tool_calls.is_empty() {
    let finish = choice.get("finish_reason")...unwrap_or("unknown");
    let reasoning_chars = message.get("reasoning_content")
        .or_else(|| message.get("reasoning"))...count();
    let detail = if reasoning_chars > 0 {
        format!("the endpoint returned {reasoning_chars} reasoning characters but no visible text; disable thinking or raise max_tokens")
    } else { ... };
    return Err(LlmError::Transport(format!("{detail} (finish_reason: {finish})")));
}
```

Three facts follow:

| Fact | Consequence |
|---|---|
| The error is `LlmError::Transport` | `kind()` is `"transport"`, `status()` is `ProviderError`; the ledger and the dock cannot distinguish "the endpoint is down" from "the model thought until it ran out of room" |
| The budget was `config/llm.toml` `[defaults] max_tokens = 1024` | 3,838 reasoning characters is roughly 950–1,000 tokens: the model had no room left for the answer |
| The message is a *hint* inside a string | Nothing downstream can act on it: no retry, no thinking-off switch, no dashboard signal |

`crates/mm-llm/src/service.rs:259` applies `cfg.defaults.max_tokens` only when the request still
carries the `DEFAULT_MAX_TOKENS` sentinel, so **every** caller that does not set a budget
explicitly gets 1,024 tokens — including `interpret`, `converse`, and agent turns.

### 2.2 Why streaming does not already avoid it

`crates/mm-llm/src/openai.rs:452-523` (`StreamAccumulator`) reads only `delta.content` and
`delta.tool_calls`. It never looks at `reasoning_content`, so a reasoning-only stream leaves
`text` empty and `tool_calls` empty. Then `complete_stream` (`:1049-1064`) does:

```rust
if text.is_empty() && calls.is_empty() {
    let response = self.complete(req).await?;   // second provider round
    ...
}
```

So the operator pays for the reasoning twice, and the second round fails the same way. This is a
correctness bug independent of the budget: a stream that *did* deliver SSE frames must never be
replayed as a non-streaming call. The fallback is only honest when the transport reported
`streamed == false`.

### 2.3 Every 30-second ceiling on the model path

The reported error is not caused by a timeout, but the ceiling is a second, compounding defect:
a thinking model that needs 90 seconds is killed at 30. All of these are in play.

| # | Location | Current | What it bounds |
|---|---|---|---|
| T1 | `crates/mm-llm-agentstream/src/lib.rs:137` | `timeout_ms: 30_000` | default profile: whole call (pool + transport) |
| T2 | `crates/mm-llm-agentstream/src/lib.rs:150` | `timeout_ms: 120_000` | hermes profile: whole call |
| T3 | `crates/mm-llm-agentstream/src/pool.rs:135-140` | clamp to profile | `req.timeout_ms`, so a caller cannot ask for more than the profile |
| T4 | `crates/mm-llm/src/config.rs:161`, `config/llm.toml:19` | `timeout_ms: 30_000` | transport budget for direct constructions (CLI, tests) |
| T5 | `crates/mm-llm/src/openai.rs:142,188,213-240` | per-connect/write/read | transport: the *read* timeout is also the ceiling for a non-streaming body |
| T6 | `crates/mm-cme/src/budget.rs:66,69` | `wall_ms: 30_000` | Memory and Summarize turn budgets |
| T7 | `crates/mm-agent/src/summarizer.rs:22` | `SUMMARY_TIMEOUT = 30s` | one summarizer call |
| T8 | `crates/mm-serve/src/cm_tools.rs:1000` | `Duration::from_secs(30)` | `metamind.act` default (some actions call a model) |
| T9 | `config/automation.toml:31` | `immediate_timeout_ms = 30_000` | one automation action (may call a model) |
| T10 | `crates/mm-process/src/config.rs:35` | `lease_ms: 30_000` | step lease, not the call — but a 5-minute call can outlive it |

Not in scope: `crates/mm-serve/src/controller/exec.rs:79` (a generic tool budget) and
`crates/mm-serve/src/cm_tools.rs` read/write tool budgets. Those are tool ceilings, not model
ceilings; changing them is a separate decision.

### 2.4 What the dock shows today, and why it hides the cause

| Observation | Cause |
|---|---|
| The RSP line is empty for a streaming turn | `llm_feed.rs:294` publishes `preview.reply(&response.text)`; when the stream carried only reasoning, `response.text` is empty. Nothing accumulates deltas into the outcome |
| Deltas stop at 2,000 characters | `live.rs:18` `DELTA_CAP_CHARS = 2_000` — a display cap, but it is the *only* stream text the dock ever sees |
| Rows are flat | `llm-log.ts` `applyActivityEvent` appends every event; a response is a sibling of its request, not a child |
| No trigger/origin | `CallContext` (`mm-llm/src/call.rs`) carries only `source`, `session_id`, `run_id`, `pass`, `round` — and `pass`/`round` are **never set**: no scope in `mm-cme` enters them |
| No retry visibility | The gateway journals attempts, but the live bus has no attempt event and the wire has no `attempt` field |

---

## 3. Target state

```
thinking model, 3.8k reasoning chars, no visible text
        │
        ▼
OpenAiClient ──► LlmError::ReasoningOnly { reasoning_chars, finish_reason }
        │                    │
        │                    ├─► feed: error event with reasoningChars + finishReason
        │                    └─► gateway: ONE escalation retry (new call, parentCallId set)
        │                                 └─► thinking disabled if MM_LLM_THINKING=disabled
        ▼
dock group:
  REQ  14:32:07 chat/process r2 · converse · trigger=tool_result · tool=memory.add
  │ gpt-4o-mini · 4 msgs · 31 tools · stream
  └ ERR 14:32:10 reasoning_only · length · 3,838 reasoning chars
     REQ 14:32:11 chat/process r2 · converse · trigger=budget_retry parent=01J9…
     └ RSP 14:32:19 gpt-4o-mini · ok · 1,004/612 tok · 8.1s · 2.4k chars "The migration…"

Text tab:
  14:32:07 REQ 01J9… · chat/process r2 · converse · trigger=tool_result tool=memory.add
  gpt-4o-mini · requested · 812/204 tok · 2400 ms · 1204 chars
  "summarize the migration order…"
    14:32:10 ↳ERR 01J9… · chat/process r2 · converse
    — · reasoning_only · finish=length · 2400 ms · 0 chars · 3838 reasoning chars
    "llm reasoning-only answer: 3838 reasoning chars, no visible text (finish_reason: length)"
    14:32:11 ↳REQ 01J9… · chat/process r2 · converse · trigger=budget_retry parent=01J9…
    gpt-4o-mini · requested · 8100 ms · 2410 chars
    "summarize the migration order…"
      14:32:19 ↳RSP 01J9… · chat/process r2 · converse
      gpt-4o-mini · ok · finish=stop · 1004/612 tok · 8100 ms · 2410 chars
      "The migration order is fixed by the schema; apply 003 then 004."
```

Invariants:

1. **No model call is cut while tokens are still arriving.** Progress, not wall clock, is the
   bound.
2. **A failed round is a first-class event, not a string.** The dock, the ledger, and the retry
   policy all read the same typed outcome.
3. **The dock is a projection.** Grouping, caps, and the text tab never change chat, accounting,
   or the journal.

---

## 4. Phase 1 — Call fidelity and attribution (backend)

### 4.1 Per-purpose output budget

`max_tokens = 1024` for every purpose is the root cause. Budget is a property of the work, not of
the deployment.

**Config** (`crates/mm-llm/src/config.rs`, `config/llm.toml`):

```toml
[defaults]
# Fallback when a purpose has no entry below.
max_tokens = 2048

[defaults.max_tokens_by_purpose]
interpret   = 4096
plan        = 4096
extract     = 2048
critique    = 2048
summarize   = 2048
code_review = 4096
classify    = 512
diagnose    = 2048
converse    = 4096
agent       = 8192
```

- Parse into `BTreeMap<String, u32>` keyed by `Purpose::as_str()`; an unknown key is a config
  error (the config loader already validates this way).
- `LlmService::call_routed_with` (`service.rs:259`) replaces
  `resolved.max_tokens = self.cfg.defaults.max_tokens` with a purpose lookup, then the fallback.
  An explicit caller budget still wins — the sentinel check stays.
- `agent` only matters for profiles that send `max_tokens`; hermes sets
  `omit_max_tokens = true`, so its value is documentation until that changes.
- Environment override for operators: `MM_LLM_MAX_TOKENS` sets the fallback; no per-purpose env
  knob (keep the surface small).

**Exit:** `cargo test -p mm-llm` proves a `Converse` request gets 4096, an explicit budget is
untouched, and an unknown purpose key is rejected at load.

### 4.2 A typed reasoning-only outcome

Add one variant to `crates/mm-llm/src/error.rs`:

```rust
/// The endpoint spent its completion budget on reasoning and emitted no visible
/// answer. This is a budget outcome, not a transport failure.
#[error("llm reasoning-only answer: {reasoning_chars} reasoning chars, no visible text (finish_reason: {finish_reason})")]
ReasoningOnly {
    reasoning_chars: u32,
    finish_reason: String,
},
```

- `kind()` → `"reasoning_only"`; `status()` → `CallStatus::ProviderError` (unchanged ledger
  semantics; the `error_kind` column becomes informative).
- `parse_response` returns it when `reasoning_chars > 0`; the no-reasoning empty case keeps
  `Transport` (that really is a broken endpoint).
- Add `reasoning_chars` and `finish_reason` to the error's display, and carry the same values on
  the live/feed error event (§4.6).
- Do **not** make it `Timeout` or `SchemaRejected`: retry policy and the dock treat each
  differently.

### 4.3 One bounded escalation retry

The operator's error should not be terminal when a larger budget would have answered.

**Where:** `AgentstreamLlm::execute` (`mm-llm-agentstream/src/lib.rs:509-575`), after the first
`service.call*` returns `Err(LlmError::ReasoningOnly { .. })`.

**Policy:**

- Exactly **one** retry per logical call. Conditions: `kind == "reasoning_only"`,
  `finish_reason == "length"`, visible text empty, no tool calls, and
  `MM_LLM_REASONING_RETRY != "off"`.
- New budget: `min(max(current * 2, 4096), MM_LLM_REASONING_RETRY_MAX_TOKENS)` (default 8192).
- When `MM_LLM_THINKING=disabled`, the retry also asks the provider to skip reasoning (§4.3.1).
- The retry is a **new call** with its own `call_id`, `parent_call_id` = the failed call, and
  `trigger = "budget_retry"`. It is journaled, accounted, and published like any other call, so
  the ledger never hides the extra spend and the dock nests it under the original request.
- No retry for `Timeout`, `Transport`, `SchemaRejected`, or `ProviderNotConfigured`; those have
  their own existing handling and a blind retry would double a real outage.
- If the retry also fails, return the retry's error (which carries its own reasoning count).

**4.3.1 Thinking switch**

`ProviderProfile` (`mm-llm-agentstream/src/lib.rs:104-127`) gains:

```rust
/// How this profile asks the provider to skip reasoning, when asked to.
pub thinking: ThinkingPolicy, // ProviderDefault | Disabled
```

`OpenAiClient::request_body` emits, only when the policy is `Disabled`:

| Provider shape | Body field |
|---|---|
| OpenAI reasoning models | `"reasoning_effort": "none"` |
| vLLM / NIM chat templates | `"chat_template_kwargs": { "enable_thinking": false }` |
| Anything else | no field (the profile stays on its default) |

Which shape is chosen comes from a `thinking_style` profile field (`auto` default), so a proxy
that rejects the field is a config fix, not a code change. `MM_LLM_THINKING=auto|disabled`
sets the policy for `default` and `hermes` in `ProviderProfiles::standard()`.

### 4.4 Streaming parity

`StreamAccumulator` gains a `reasoning_chars: u32` counter, fed by
`delta.reasoning_content` / `delta.reasoning`, and `complete_stream` changes its fallback:

- **Never** call `complete()` when the SSE stream produced frames. If `text` is empty and
  `tool_calls` is empty and `reasoning_chars > 0`, return
  `LlmError::ReasoningOnly { reasoning_chars, finish_reason }` — the same typed outcome as the
  non-streaming path, so §4.3 applies to both.
- Keep the `!streamed` fallback (the proxy ignored `stream`): that is the only honest replay.
- Reasoning deltas are **not** forwarded to the caller's `DeltaSink`; the caller still receives
  only visible text. They are counted, and the count rides on the terminal event (§4.7), which is
  what the dock needs to explain the failure.

**Exit:** a streamed reasoning-only fixture produces exactly one provider round, one typed error,
and no `complete()` call.

### 4.5 Remove the 30-second ceiling

Replace "one wall clock for the whole call" with two independent bounds:

| Bound | Field | Default | Purpose |
|---|---|---|---|
| Total ceiling | `timeout_ms` | `0` (none) | an optional hard stop; `0` means unbounded |
| Idle bound | `idle_timeout_ms` | `300_000` | connect/write/read must make progress inside this window |

- `ProviderProfile` gains `idle_timeout_ms`; `ProviderPool::provider_client`
  (`pool.rs:111`) passes it to `TcpTransport` as the per-I/O budget instead of `timeout_ms`.
- `ProviderPool::budget_ms` returns `Option<Duration>`: `None` when the ceiling is `0`.
  `complete`/`complete_stream` wrap in `tokio::time::timeout` only when a ceiling exists.
- `normalize_request` (`lib.rs:627`) clamps `req.timeout_ms` only when a ceiling exists, so a
  caller can no longer be silently capped at the profile's value.
- `TcpTransport` keeps per-connect/write/read timeouts (idle semantics). A non-streaming body is
  one read window; 300 s covers a long reasoning completion.
- Defaults: `default` profile `timeout_ms = 0`, `idle_timeout_ms = 300_000`; `hermes` the same.
  `LlmConfig::defaults.timeout_ms` and `config/llm.toml` move to `0` with
  `idle_timeout_ms = 300_000`.
- Operator knobs: `MM_LLM_TIMEOUT_MS` (0 = none), `MM_LLM_IDLE_TIMEOUT_MS`.

**The other 30-second bounds** (T6–T10 in §2.3) are raised in the same change so the model path
has one story:

| Location | New value |
|---|---|
| `mm-cme/src/budget.rs` Memory, Summarize | `wall_ms: 120_000` |
| `mm-cme/src/budget.rs` Think | `wall_ms: 180_000` (from 90_000) |
| `mm-agent/src/summarizer.rs` | `SUMMARY_TIMEOUT = 120s` |
| `mm-serve/src/cm_tools.rs` `metamind.act` default | `120s`; `metamind.plan`/`think` 90 → 300 |
| `config/automation.toml` `immediate_timeout_ms` | `120_000` |
| `mm-process/src/config.rs` `lease_ms` | verify renewal during a step; if the lease is not renewed, raise to `300_000` |

**Exit:** a scripted provider that waits 90 s before its first byte completes normally; a
provider that opens a socket and then goes silent is cut by the idle bound with
`LlmError::Timeout(300_000)`.

### 4.6 Attribution: what triggered this call

Extend `CallContext` (`mm-llm/src/call.rs`) — all fields optional, all cheap to clone:

```rust
pub struct CallContext {
    pub source: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub pass: Option<String>,
    pub round: Option<u32>,
    /// What caused this call: `user_turn`, `tool_result`, `repair`, `budget_retry`,
    /// `agent_step`, `scheduler`, `boot`, `summary`.
    pub trigger: Option<String>,
    /// The call whose outcome caused this one (retry, repair, tool-driven follow-up).
    pub parent_call_id: Option<String>,
    /// The plan step or agent step being served.
    pub step: Option<String>,
    /// The tool whose result prompted this call.
    pub tool: Option<String>,
    /// Stable call-site label, e.g. `cme.think`, `controller.turn`, `agent.hermes`.
    pub origin: Option<String>,
}
```

Builders: `with_trigger`, `with_parent`, `with_step`, `with_tool`, `with_origin`. `scope` is
unchanged, so nesting works: a pass scope refines the turn's context via
`current_context().unwrap_or_default().with_pass(..)`.

**Scope sites to fill (existing):**

| Site | `source` | Add |
|---|---|---|
| `mm-serve/src/routes/cme.rs:226,273` | `chat` | `trigger=user_turn`, `origin=cme.turn` |
| `mm-serve/src/controller/turn.rs:50` | `chat` | `trigger=user_turn`, `origin=controller.turn` |
| `mm-serve/src/process_runner.rs:103` | `process` | `step=request.step_id`, `tool` from `tool_scope`, `trigger=process_step` |
| `mm-serve/src/automation.rs:264,343` | `automation` | `trigger=scheduler`, `origin=automation.action` |
| `mm-agent/src/hermes.rs:322` | `agent` | `trigger=agent_step`, `step=turn index` |
| `mm-agent/src/summarizer.rs:114` | `summarize` | `trigger=summary` |

**New scope sites:**

- `mm-cme/src/executor.rs` at the pass constructions (`:439`, `:606`) and
  `mm-cme/src/subagent.rs:576` (`think`): wrap each pass in a nested scope with `pass` and
  `round`. Today `pass` is always empty for chat traffic, which is the single biggest gap in
  "where did this come from".
- Tool-result follow-ups: where a tool result is fed back into a round, set `trigger=tool_result`
  and `tool=<name>`.

**Wire** (`mm-serve/src/dto/llm_activity.rs`): `Request` gains `trigger`, `parent_call_id`,
`step`, `tool`, `origin`, `attempt: u32`, and `timeout_ms: Option<u64>` (the budget actually
applied — the value that explains a `timeout` row). Regenerate
`frontend/src/lib/wire/llm_activity.ts` with `cargo test -p mm-serve export_types -- --ignored`.

**Journal parity:** `journal::request_payload` (`journal.rs:410`) carries the same fields so the
durable record and the live dock agree.

### 4.7 Roll the stream into the terminal event

`LlmResponse.text` already holds the complete streamed body — the feed simply never sends it.

- `LlmActivityEvent::Response` gains `stream_text`, `stream_chars`, `stream_truncated`,
  `reasoning_chars`, and `attempt`. `Error` gains `reasoning_chars`, `finish_reason`, `attempt`,
  and `stream_chars`.
- `LlmActivityFeed::response` fills `stream_text` from `response.text` with a new
  `MM_LLM_ACTIVITY_STREAM_CHARS` cap (default 20,000) and its own truncation flag. The existing
  reply preview stays for the one-line row summary.
- Keep `DELTA_CAP_CHARS = 2_000`: deltas are a bounded live preview; the terminal event is the
  authoritative roll-up. This keeps bandwidth flat while making the RSP line complete.
- The gateway sets `attempt` on the live request/response/error events
  (`LiveLlmEvent`, `live.rs`); the feed copies it through. `attempt` is the pool's attempt number
  (`CallScope::next_attempt`), which already exists for the journal.

### 4.8 Files touched in Phase 1

| File | Change |
|---|---|
| `crates/mm-llm/src/config.rs`, `config/llm.toml` | purpose budgets, `timeout_ms`, `idle_timeout_ms` |
| `crates/mm-llm/src/error.rs` | `ReasoningOnly` variant, `kind()` |
| `crates/mm-llm/src/openai.rs` | typed error, reasoning counter, stream fallback fix, thinking knob, idle timeouts |
| `crates/mm-llm/src/service.rs` | purpose budget lookup |
| `crates/mm-llm/src/call.rs` | `CallContext` fields + builders |
| `crates/mm-llm-agentstream/src/{lib,pool,live,journal}.rs` | retry, thinking policy, `attempt`, idle timeout, payload fields |
| `crates/mm-serve/src/{dto/llm_activity,llm_feed}.rs` | wire fields, roll-up, `attempt` |
| `crates/mm-serve/src/{routes/cme,controller/turn,process_runner,automation}.rs` | richer scopes |
| `crates/mm-cme/src/{executor,subagent,budget}.rs` | pass scopes, wall times |
| `crates/mm-agent/src/{hermes,summarizer}.rs` | triggers, timeout |
| `crates/mm-process/src/config.rs` | lease check |
| `config/automation.toml` | automation timeout |

No new dependencies.

### 4.9 Tests and exit for Phase 1

Rust:

| Test | Proves |
|---|---|
| `a_purpose_budget_replaces_the_default` | 4096 for `Converse`; explicit budgets untouched |
| `reasoning_only_is_a_typed_error` | `kind() == "reasoning_only"`, counts preserved |
| `a_reasoning_only_stream_makes_one_provider_round` | no `complete()` fallback, one typed error |
| `a_proxy_that_ignores_stream_still_falls_back_once` | the honest fallback is preserved |
| `reasoning_only_is_retried_once_with_a_larger_budget` | new call id, `parent_call_id`, doubled budget, journaled attempt |
| `a_timeout_is_never_retried` | retry policy is narrow |
| `thinking_disabled_sends_the_provider_knob` | body shape per `thinking_style` |
| `an_unbounded_profile_has_no_total_timeout_but_keeps_the_idle_bound` | 90 s call succeeds; silent socket times out |
| `request_event_carries_trigger_parent_step_tool_origin` | attribution reaches the wire |
| `response_event_carries_the_full_stream_text` | roll-up, cap, truncation flag |
| `a_cme_pass_scope_sets_pass_and_round` | the pass gap is closed |

Exit:

- `cargo test -p mm-llm -p mm-llm-agentstream -p mm-cme -p mm-serve`
- `cargo test -p mm-serve export_types -- --ignored` regenerates the TS wire file.
- Manual: one thinking-model turn with `max_tokens_by_purpose.converse = 512` reproduces the
  original error as `reasoning_only`, then recovers on the retry; `curl -N /api/llm-activity/stream`
  shows `trigger`, `attempt`, `reasoningChars`, and `streamText`.

---

## 5. Phase 2 — The dock: groups, rolled-up RSP, text tab

Phase 1 makes the data available. Phase 2 changes what the operator reads.

### 5.1 Grouping model

Replace the flat row list with call groups, computed from the same bounded `rows` (no new
storage, no new endpoint):

```ts
type CallGroup = {
  callId: string;
  request: RequestRow;                 // always present
  outcome: ResponseEvent | ErrorEvent | null;
  children: CallGroup[];               // retries (parentCallId === this callId)
  orphan?: boolean;                    // parent evicted from the ring
};

export function groupRows(rows: LlmLogRow[]): CallGroup[];
```

- One pass over `rows`: index requests by `callId`, attach each `response`/`error` to its
  request, then attach requests that name a `parentCallId` under that parent.
- A retry group is nested one level; deeper nesting is flattened with an `↳ retry of …` label.
  Cap group nesting at two levels (a parent and one retry); an outcome indents one step from its
  own request. Guard against cycles (a malformed `parentCallId` becomes a root group with
  `orphan: true`).
- `delta` events never become rows in the structured tab: they fold into their request's
  in-flight text (today's behavior) and are superseded by `streamText` when the outcome arrives.
- Ordering is by request `seq`; an outcome always renders under its request even though its
  `seq` is later. That is the point of the change.

### 5.2 The RSP line

One line, newest at the bottom, monospace, single-line with `overflow-hidden`:

```
14:32:10  RSP  process · claude-…  ok · 812/204 tok · 2.4s · 1.2k chars · "The migration order…"
```

- Text: `response.streamText` when present, else the folded delta text, else
  `(empty reply)`.
- Meta: model · status · cached · tokens in/out · latency · char count
  (`truncated` marker when `streamTruncated`) · reasoning char count when nonzero.
- Expanding the row shows the **whole** rolled-up body in a bounded, scrollable
  `whitespace-pre-wrap break-words` block (not truncated to one line), plus the ids:
  call, parent, trigger, origin, step, tool, attempt, `timeoutMs`, finish reason.
- A reasoning-only failure is an `ERR` child with `reasoning_only · length · 3,838 reasoning
  chars`, so the operator reads the cause without opening the chat error.
- The in-flight request row keeps growing from deltas; on outcome, its preview stays and the
  child carries the full text. No duplicate text in the same group.

### 5.3 Nesting and indentation

```
▾ REQ  14:32:07  chat/process r2 · converse · trigger=tool_result · tool=memory.add
   │  gpt-4o-mini · 4 msgs · 31 tools · stream
   └ ERR  14:32:10  reasoning_only · length · 3,838 reasoning chars
   ▾ REQ  14:32:11  chat/process r2 · converse · trigger=budget_retry · parent=01J9…
      └ RSP 14:32:19  gpt-4o-mini · ok · 1,004/612 tok · 8.1s · 2.4k chars · "The migration…"
```

A call has exactly one terminal event, so a group is `REQ` + (`RSP` | `ERR`) or `REQ` + a
nested retry group. The retry is indented under the request that failed, and its own outcome is
indented one step further.

- Indentation: `pl-6` on the child list, a 1 px `border-l` guide, a small `↳`/`├`/`└` glyph in
  the muted token. Text stays monospace and `text-xs`.
- Semantics: the outer element is `role="tree"`, each group is a `role="treeitem"` with
  `aria-level={1|2}` and `aria-expanded`; the request row toggles its children. The existing
  `role="log"`/`aria-live="off"` on the scroll container is kept so streaming is never announced.
- The request row is the header: clicking it expands/collapses the children (and still reveals
  its own details). Children are never collapsible individually — they are one outcome each.
- Pinned auto-scroll, jump-to-latest, resize, collapse, pause, and clear behave exactly as today,
  measured on groups.

### 5.4 Filters with groups

`filterRows` becomes `filterGroups(groups, filter)`:

| Filter | Shows |
|---|---|
| All | every group |
| Requests | request rows only, outcomes hidden |
| Streams | groups whose request was `streamed` |
| Responses | groups with an outcome, request header kept as context (dimmed, not hidden) |
| Errors | groups whose outcome is an error, request header kept as context |

Counts in the header count groups, not events (`12 calls / 34 events`), so a filter cannot show
an outcome without its cause.

### 5.5 Two tabs: Log and Text

The header gains a two-item tablist before the filter:

```tsx
<div role="tablist" aria-label="LLM activity view">
  <button role="tab" aria-selected={tab === "log"}>Log</button>
  <button role="tab" aria-selected={tab === "text"}>Text</button>
</div>
```

- Persisted as `llmLogTab: "log" | "text"` in `prefs.ts` (`partialize` updated), so a reload
  keeps the operator's choice.
- `Log` is today's structured view, with §5.1–5.3 applied.
- `Text` is a **plain-text rendering of the same groups**: no badges, no chevrons, no per-row
  buttons, no expand — only selectable text, so it can be copied into a bug report. One
  `Copy` button in the header copies the whole tab as text.
- `Ctrl+\`` still toggles the dock; `Ctrl+Shift+\`` switches tab (optional, only if the existing
  keyboard map has room — otherwise skip it).

### 5.6 The text tab: exactly three lines per block

Each block is three lines of fixed-role text, one block per request and one per outcome
(indented with two spaces and `↳` when nested). Deltas never produce a block.

```
HH:MM:SS KIND <call> · <source>[/<pass> r<round>] · <purpose> · trigger=<trigger>[ tool=<tool>][ parent=<parent>]
<model> · <status>[ · cached] · <tokensIn>/<tokensOut> tok · <latency> ms · <chars> chars[ truncated][ · <reasoning> reasoning chars]
<one-line normalized text: preview | reply | error>
```

Rules:

- Line 1 always names the kind (`REQ` / `RSP` / `ERR`), the short call id (first 8 chars of the
  ULID), the origin tuple, the purpose, and the trigger. Missing fields are omitted, not
  rendered as `null`.
- Line 2 always carries the numbers that explain cost and time; `finish=<reason>` is appended
  for a response or error.
- Line 3 is one line: newlines and control characters are collapsed to spaces, runs of spaces
  are squeezed, and the text is clipped with `…` at the viewport width. The full text stays
  available in the Log tab and by copying (the copy action emits untruncated line 3).
- Blocks are separated by a 1 px border, not a blank line, so "three lines per block" is exact
  and testable.
- `font-mono text-[11px] leading-[1.35]`, `whitespace-pre`, `overflow-hidden`; no `title`
  tooltips (they are not copyable) — the Log tab is the place for detail.
- Accessibility: `role="log"`, `aria-live="off"`, and `aria-label="LLM activity text log"`.
  Because the tab is text-only, it is also the print/screen-reader-friendly view.

Example (two calls, one retry):

```
14:32:07 REQ 01J9K2M0 · chat/process r2 · converse · trigger=tool_result tool=memory.add
gpt-4o-mini · requested · 812/204 tok · 2400 ms · 1204 chars
"summarize the migration order and the schema version it needs"
  14:32:10 ↳ERR 01J9K2M0 · chat/process r2 · converse
  — · reasoning_only · finish=length · 2400 ms · 0 chars · 3838 reasoning chars
  "llm reasoning-only answer: 3838 reasoning chars, no visible text (finish_reason: length)"
  14:32:11 ↳REQ 01J9K2M1 · chat/process r2 · converse · trigger=budget_retry parent=01J9K2M0
  gpt-4o-mini · requested · 8100 ms · 2410 chars
  "summarize the migration order and the schema version it needs"
    14:32:19 ↳RSP 01J9K2M1 · chat/process r2 · converse
    gpt-4o-mini · ok · finish=stop · 1004/612 tok · 8100 ms · 2410 chars
    "The migration order is fixed by the schema; apply 003 then 004."
```

An outcome block inherits `source`/`pass`/`round`/`purpose` from its request, because the wire
outcome event does not repeat them.

### 5.7 Files touched in Phase 2

| File | Change |
|---|---|
| `frontend/src/lib/store/llm-log.ts` | `groupRows`, `filterGroups`, keep `rows` as the flat event ring |
| `frontend/src/lib/store/llm-log.test.ts` | grouping, nesting, filters, bounds |
| `frontend/src/components/chat/llm-log-dock.tsx` | tabs, group rendering, rolled-up RSP detail, text view |
| `frontend/src/components/chat/llm-log-row.tsx` | new: request header, outcome child, indent guides (split out of the dock) |
| `frontend/src/components/chat/llm-log-text.tsx` | new: the three-line text renderer and copy action |
| `frontend/src/components/chat/llm-log-dock.test.tsx` | tab switch, nesting, text format, copy |
| `frontend/src/lib/store/prefs.ts` | `llmLogTab` persisted |
| `frontend/src/lib/wire/llm_activity.ts` | regenerated (Phase 1 output) |
| `frontend/src/components/shell/keyboard-map.tsx` | only if the tab shortcut is added |

No new dependencies; reuse `components/kit/tabs.tsx` underline style and the existing tokens.

### 5.8 Tests and exit for Phase 2

| Test | Proves |
|---|---|
| `groupRows nests an outcome under its request` | structure, not just order |
| `a retry group nests under its parent request` | `parentCallId` grouping, two levels max |
| `an evicted parent renders the child as an orphan root` | ring eviction cannot lose rows silently |
| `the rsp line rolls up the full stream text` | `streamText` wins over folded deltas |
| `a reasoning-only error shows its reasoning count` | the reported failure is readable |
| `filters keep the request as context` | no orphaned outcomes |
| `the text tab renders exactly three lines per block` | line count is a contract |
| `copy emits the untruncated text log` | bug-report path |
| `delta frames never create their own block` | roll-up invariant |
| `the dock keeps its height, pause, and cursor across a tab switch` | no state loss |

Exit: `npm test -- --run llm-log`, `npm run typecheck`, and one manual turn that produces a
reasoning-only failure plus a successful retry, read in both tabs.

---

## 6. Wire changes (additive, `wireVersion: 1` unchanged)

```rust
Request {
    // existing…
    trigger: Option<String>,
    parent_call_id: Option<String>,
    step: Option<String>,
    tool: Option<String>,
    origin: Option<String>,
    attempt: u32,
    timeout_ms: Option<u64>,
}
Response {
    // existing…
    stream_text: String,       // full rolled-up body, capped by MM_LLM_ACTIVITY_STREAM_CHARS
    stream_chars: u32,
    stream_truncated: bool,
    reasoning_chars: u32,      // reasoning the model did before answering
    attempt: u32,
}
Error {
    // existing…
    reasoning_chars: u32,
    finish_reason: Option<String>,
    stream_chars: u32,
    attempt: u32,
}
```

- Every new optional field is skipped when absent; new numeric fields carry
  `#[serde(default)]`, so an older client parses a newer feed.
- No chat frame changes; no status-stream changes; no new routes.
- Regenerate the TS file from the Rust source of truth; never hand-edit it.

---

## 7. Data policy

Unchanged from `llm_activity_log_plan.md` §9: local-only, in-memory, redacted server-side, no
system prompt by default, no tool arguments, no provider bodies, no keys. Two additions:

1. `MM_LLM_ACTIVITY_STREAM_CHARS` (default 20,000) is the new roll-up cap; the existing
   `MM_LLM_ACTIVITY_PREVIEW_CHARS` (4,000) still bounds the one-line summary. Both are enforced
   before publish.
2. The text tab copies text that already passed the server-side policy. The copy action must not
   fetch anything; it serializes the rows the browser already holds.

---

## 8. Acceptance criteria

1. A thinking model that exhausts its budget produces an `ERR` row with `reasoning_only` and the
   reasoning character count, not a bare transport string.
2. The same turn recovers automatically through exactly one retry with a larger budget, and both
   rounds are visible and nested in the dock and the ledger.
3. No model call is cut at 30 seconds. A 90-second first byte succeeds; a silent socket is cut by
   the idle bound, and the `ERR` row says `timeout` with the budget that applied.
4. The RSP line carries the full streamed body (up to the configured cap, with a truncation
   marker), and expanding it shows the whole text.
5. Every request row names its `source`, `pass`, `round`, `trigger`, `origin`, and — where
   relevant — `step`, `tool`, `parent`, and `attempt`.
6. Responses and errors are indented under the request they answer; a retry is indented under the
   request that failed; an evicted parent yields a marked orphan, never a missing row.
7. The dock has two tabs; the text tab renders exactly three lines per block, contains no
   interactive controls inside the text, and copies as plain text.
8. `wireVersion` stays `1`; chat frames, the status stream, and the existing SSE codec are
   untouched.
9. With `MM_LLM_REASONING_RETRY=off`, the failure is terminal and honest; with
   `MM_LLM_ACTIVITY=false`, the dock explains that the feed is unavailable.

---

## 9. Risks and decisions

### 9.1 Decisions

| # | Decision | Why |
|---|---|---|
| D1 | Budget per purpose, not one global `max_tokens` | The failure is a budget failure; one number cannot serve `classify` and `agent` |
| D2 | `ReasoningOnly` is a typed error, not `Transport` | Retry policy, ledger `error_kind`, and the dock all need to tell it apart from an outage |
| D3 | Retry is a new call with `parent_call_id` | Keeps accounting and the journal honest, and gives the dock a real nesting to render |
| D4 | One retry only, escalation capped | An unbounded retry loop on a broken model is worse than a clear error |
| D5 | Progress bound (idle) replaces the wall clock | A thinking call is long by design; a stalled one is not |
| D6 | Deltas stay capped; the terminal event rolls up | Bandwidth stays flat while the RSP line becomes complete |
| D7 | Grouping is a client-side projection of the existing ring | No new endpoint, no new storage, no wire version bump |
| D8 | The text tab is three fixed lines, not "about three" | It is a copy/scan surface; a variable shape defeats the purpose |

### 9.2 Risks

| Risk | Mitigation |
|---|---|
| Doubling `max_tokens` doubles worst-case cost | One retry only; `worst_case_cost_micros` routing already budgets on `max_tokens`; ledger shows the retry |
| Unbounded calls hang a turn | Idle bound (300 s) plus the existing `mm-cme` turn wall budgets |
| A proxy rejects `reasoning_effort` | `thinking_style` is profile config; default is "send nothing" |
| Grouping costs a second pass over 500 rows | O(n) with a map; recomputed in a `useMemo` keyed by `rows` |
| The 20,000-char roll-up bloats the SSE frame | Configurable cap; one frame per call, not per delta |
| Process lease expires during a long call | Phase 1 verifies renewal and raises `lease_ms` if it does not renew |
| Text tab drifts from the structured tab | Both render from `groupRows`; tests assert the same group count |

### 9.3 Open questions

1. Should the retry also raise the budget when the first round produced *partial* visible text
   (`finish_reason: length` with text)? Start with no — partial text is a usable answer.
2. Should `MM_LLM_THINKING=disabled` be the default for structured calls only? Start with a
   global default of `auto`; revisit after dogfooding.
3. Is 20,000 the right roll-up cap? Keep it configurable; tune against a real 4k-token reply.
4. Should the text tab also be exposed on the inspector's log surface? The store is
   workspace-level, so a second mount is cheap later; not in this plan.

---

## 10. Traceability

| Requirement | Workstream | Test |
|---|---|---|
| Fix `… reasoning characters but no visible text …` | §4.1, §4.2 | `a_purpose_budget_replaces_the_default`, `reasoning_only_is_a_typed_error` |
| Recover instead of failing the turn | §4.3 | `reasoning_only_is_retried_once_with_a_larger_budget` |
| Stop paying twice for a reasoning-only stream | §4.4 | `a_reasoning_only_stream_makes_one_provider_round` |
| Remove the 30-second limit | §4.5 | `an_unbounded_profile_has_no_total_timeout_but_keeps_the_idle_bound` |
| Know where a request came from | §4.6 | `request_event_carries_trigger_parent_step_tool_origin` |
| All streaming data on the RSP line | §4.7, §5.2 | `response_event_carries_the_full_stream_text`, `the rsp line rolls up the full stream text` |
| Responses/errors nested under requests | §5.1, §5.3 | `groupRows nests an outcome under its request` |
| Text-only tab, 3 lines per block | §5.5, §5.6 | `the text tab renders exactly three lines per block` |
| Nothing new leaks to the browser | §7 | existing redaction tests plus the new cap |
| Old clients keep working | §6 | `wireVersion` assertion, wire round-trip test |

---

## 11. Out of scope

- Changing chat frames, the three-pass UI, or `wireVersion`.
- Persisting activity, backfilling from `llm_calls`, or adding a history endpoint.
- Reworking `mm-cme`'s turn budget model beyond the wall-clock values in §4.5.
- Provider-specific streaming of reasoning *into the chat transcript*; reasoning stays a
  diagnostic channel.
- Generic tool timeouts in `controller/exec.rs` and `cm_tools.rs` read/write paths.
