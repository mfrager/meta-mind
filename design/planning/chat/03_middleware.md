# 03 — Middleware and the turn

> Parent: [`INDEX.md`](./INDEX.md) · Previous: [`02_tool_surface.md`](./02_tool_surface.md) · Next: [`04_ui.md`](./04_ui.md)

This is how a sentence becomes a kernel action without a second control plane. The controller is a
pipeline of middleware inside the `mm-serve` process that already holds the stores. Each stage has one
job, a type it accepts, a type it returns, and a failure that is a reason rather than a panic.

---

## 1. Where it sits

```text
browser composer
    │  POST /api/chat          (existing route, new body handler)
    ▼
ChatCatalog                  sessions, runs, frame sequences, resume
    │
    ▼
Controller::turn             the pipeline below
    │
    ├─ SessionMw
    ├─ PersonaMw              BeingFacade + Personality::project
    ├─ ContextMw              focused doc preview, open tool groups
    ├─ ModelMw                LlmService, Purpose::Converse, tools array
    ├─ AdmitMw                catalog, effect, confirm, SPARQL read-only, path allow-list
    ├─ ConfirmMw              input_request, park the turn
    ├─ ExecMw                 in-process subsystem call
    ├─ FocusMw                focus frame, state vs document
    ├─ HonestyMw              unbacked claims, reason passthrough
    └─ TranscriptMw           SSE frames, chat-sessions.json
    │
    ▼
existing kernel subsystems   mm-being, mm-memory, mm-decision, mm-eventlog, …
```

`crates/mm-serve/src/chat.rs` keeps sessions, cancellation, and frame storage. `execute`'s match arms
move into `controller/tools/*.rs` as the tools in `02`. `parity::classify` is not on this path. Phase 1
leaves it behind a `command_mode` flag defaulting to false. Phase 5 deletes the flag and the match.

No middleware opens a second `SqliteStore`, `GraphStore`, or `UlidFactory`. They take `&AppState`.

## 2. The turn type

```rust
struct Turn {
    session_id: Ulid,
    run_id: Ulid,
    user_text: String,
    facts: IntroFacts,                 // refreshed every turn, not only at intro
    voice: BehaviorParams,
    high_stakes: bool,
    groups: Vec<GroupId>,              // always starts with Core
    pending_confirm: Option<PendingCall>,
    calls: Vec<ToolRecord>,            // what actually ran
    steps: u32,                        // model rounds, cap 4
    tool_steps: u32,                   // tool executions, cap 6
}

struct ToolRecord {
    name: String,
    input: Value,
    result: ToolResult,               // ok, summary, reason, data, focus
    admitted: bool,
}
```

Caps are part of admission, not a hope. Step 5 does not call the model; the being gets a final
instruction "summarize what the tools returned and stop". Tool step 7 is a result
`reason: "tool step cap"`. Caps are constants in `controller/limits.rs` so tests can lower them.

Cancellation (`ChatCatalog`'s existing set) is checked before each model call and each tool. A
cancelled turn finishes with the frames it already sent and a `failed` frame reason "cancelled".

## 3. SessionMw

Runs on `POST /api/chat/sessions` and on every message.

1. Resolve the data directory from `AppState.cfg`. Refuse to start if `console.lock` is not held by
   this process (it is, because boot acquired it).
2. Create or load the session in `chat-sessions.json`.
3. If the session has no frames, run the introduction (`01` §4) and store it as frame 1. This happens
   inside session creation, not on a timer.
4. On a user message, append the user frame, open a run, and hand the `Turn` to PersonaMw.

Resume (`GET` with `?from=seq`) is unchanged. Middleware does not replay tools on reconnect. Frames
already stored are the truth. A confirm that was pending is still pending because `PendingCall` is
stored on the run before the process waits.

## 4. PersonaMw and ContextMw

PersonaMw:

1. Load `Personality` from `BeingFacade`. On failure, use `Personality::baseline()` and set
   `facts.being_store = false`.
2. Build a `ContextVector`. Always insert `chat` at weight `1.0`. Insert `high_stakes` at weight `1.0`
   when the user text matches the gate verbs in `01` §3 (a small stem list: promote, reject, loop,
   hot-load, roll back, forget, apply) **or** when a tool about to be confirmed is `Effect::Gate` or
   `Effect::External`. The stem list only raises the voice. It does not select the tool.
3. `voice = personality.project(context)`.
4. Render the system prompt as four blocks, in order, each with a stable header so tests can snapshot
   them:
   - **Identity** — `self_description` and the four invariant codes, from facts.
   - **Voice** — the rendered `BehaviorParams`, constraints, and values (`01` §3).
   - **Length** — the one-to-three sentence rule, plus the high-stakes addendum when that tag is on.
   - **Conduct** — the five "must not" lines from `01` §3.2, plus "use a tool to act; do not claim the
     act without a tool result in this turn".

ContextMw appends, when present:

- **Focus** — id, kind, title, and a 2 KiB preview from `focus.read` of the current document. Omitted
  when the pane is the state board; then it appends the state snapshot from `state.show` with sections
  `health` and `being` only (budgets and goals are a tool away, so the prompt stays small).
- **Catalog** — names and summaries of the groups already open, not their full schemas. Schemas go in
  the provider `tools` array.
- **Guide pointer** — "To explain the system, call guide.search or guide.read before answering."

The prompt is rebuilt every model round in the turn so a tool that changed a goal is visible to the
next round. It is not a growing chat-memory dump. History sent to the model is the transcript of this
session, capped at the last 30 frames or 24 KiB of text, whichever is smaller, always keeping the intro
and the current user message.

## 5. ModelMw

### 5.1 Purpose

Add `Purpose::Converse` to `crates/mm-llm/src/client.rs`. It is stored as the string `converse` on
`llm_calls`. It is **not** in `semantic_cache_purposes`. A conversational answer must not be served
from a near-match of an older operator. Exact cache stays available and keys on the full request,
including tool schemas, so a repeated identical turn can hit. Default cache mode for the controller is
`Bypass` in phase 1 and `Use` after phase 2 tests show the key includes the tool list. A cache hit is
still a ledger row.

Routing uses `config/llm.toml` as it does for every other purpose. No controller-specific model name.
If `defaults.default_model` is unset, `ThresholdRouter` picks the cheapest model whose ceiling can take
a `converse` task. The task complexity for ordinary chat is `low`. A turn with `high_stakes` is
`medium`. That is a routing input, not a change to the voice math.

The OpenAI adapter stays `DecoderKind::ProviderNative`. The controller does not send an llguidance
grammar. Remote endpoints do not expose logits (`crates/mm-llm/src/openai.rs`).

### 5.2 Tool calls on the wire

`LlmRequest` gains an optional `tools: Vec<ToolSchema>` and the response gains
`tool_calls: Vec<ProviderToolCall>`. `OpenAiClient::request_body` sends `tools` in the shape
OpenAI-compatible servers already accept (`type: function`, `function.name`, `function.parameters`).
`tool_choice` is `auto`. The adapter parses `tool_calls` off the assistant message. A response with
neither text nor tool calls is `schema_valid: false` and becomes a kernel reason, not an empty bubble.

This is the one extension to `mm-llm`. The controller does not speak HTTP itself. `MockClient` gains a
script of either a text reply or a tool call so every phase test runs without a network.

Provider-native JSON schema remains the path for the introduction rephrase (`01` §4.2), which is a
separate `LlmRequest` with a schema and **no** tools. Mixing a schema and a tools array in one call is
forbidden: one structured channel per request.

### 5.3 The loop

```text
round = 0
loop
  round += 1
  if round > 4 → final summarize instruction, one call with tools cleared, break
  emit llm_request_started (purpose converse, no model field until the response)
  response = LlmService::complete(request)
  emit llm_response / llm_request_finished
  if response.tool_calls is empty → emit final text, break
  for call in response.tool_calls
     AdmitMw → ConfirmMw (may park) → ExecMw → FocusMw
     append a tool role message { name, call_id, content: result.summary + truncated data }
  the next round sees those messages
```

Text alongside tool calls in the same provider message is narration and is stored as a non-final turn
block. The operator sees it above the tool card. The final bubble is only the round that returned no
tool calls.

If the provider is not configured, ModelMw does not run. TranscriptMw emits one assistant frame whose
text is the deterministic sentence: "I can't call a model yet. {llm health reason}." No tool runs,
because there is no proposal. The intro has already told the operator the same fact.

## 6. AdmitMw

Pure over `(ToolSpec, input, Turn) -> Result<Admitted, Reason>`. No I/O except reading the catalog.

Reject, with the reason stored on the tool card:

| Check | Reason |
|---|---|
| Name not in the catalog | `unknown tool {name}` |
| Spec `available: false` | the spec's own sentence |
| Group not open and the tool is not `catalog.open` | `group {id} is not open; call catalog.open` |
| Input fails the JSON schema | the schema error text |
| `graph.query` is not a read | `graph queries on this tool are read-only` |
| `loop.run` path is not under `bench/` and not a known goal id | `goal path is not in the allow-list` |
| `module.load` URI is not in the listed set | `module uri was not listed` |
| Tool step cap | `tool step cap` |
| A `Gate` or `External` tool while another confirm is pending | `a confirmation is already waiting` |

Admission does not ask the model to try a different tool. The reason goes back as the tool result so
the next round can apologize in one sentence and stop or pick an admitted tool.

Firewall: before `ExecMw` of any `Effect::Write`, `Effect::Gate`, or `Effect::External`, AdmitMw calls
`firewall.eval` in-process on a small action record `{ tool, principal: operator, effect }`. A denial
is the tool result. The controller does not get a retry that weakens the action. This uses the
deterministic prohibitions first, the same order the CLI documents. A firewall that is itself
unavailable returns that unavailability and the write does not proceed.

Budgets: `loop.run` and `meta.analyze` ask the budget service whether the stage's bucket can accept
the estimate. A denial is whole, with the budget's reason. The controller does not split the work to
sneak under the cap.

## 7. ConfirmMw

If the spec's `confirm` says the call must wait, ConfirmMw stores `PendingCall` on the run, emits
`input_request`, and returns `Parked`. The HTTP handler ends the stream with the run in state
`waiting`. The model loop does not continue in the background.

`POST /api/chat/{run_id}/answer` (new; the frame type already exists) resumes the same `Turn` with the
button. Approve continues at ExecMw. Reject and Defer are tool results and then one ModelMw round with
tools cleared so the being can close the turn in a sentence. The answer endpoint rejects an answer for
a run that is not `waiting`.

The confirmation body is rendered by `confirm_body(spec, input)` in code. Tests snapshot it. The model
is not asked to describe what it is about to do for the button.

## 8. ExecMw

```rust
async fn exec(app: &AppState, call: Admitted) -> ToolResult
```

A table of `name → fn`. Each function calls the subsystem the CLI command calls, passing `app.sqlite`,
`app.graph`, `app.events`, `app.logger`, `app.ids`, and the being facade. The CLI's printing stays in
`mm-cli`. Shared work moves, when it is currently trapped in a `main.rs` arm, into the subsystem crate
or into `controller/tools` as a thin wrapper. Phase 2's rule: if a wrapper starts to reimplement a
guard, stop and call the crate.

ExecMw sets a per-tool timeout of 30s, the same order as `llm.toml`'s `timeout_ms`. A timeout is
`ok: false`, `reason: "{name} timed out"`. The stores are left as the subsystem left them; there is no
process-wide rollback in this plan except `action.rollback`, which is its own tool.

Writes append their normal domain events. In addition, ExecMw asks the logger for one
`controller.tool` record: `tool`, `ok`, `session_id`, `run_id`, `trace_id`. The trace id is the run id,
so `llm_calls` and the tool share it. Secrets never enter the record. Tool input is redacted by the
same redactor `mm-log` already applies.

`decide.ask` with `core: hosted` constructs `OpenAiClient` the way `mm-cli decide --core hosted` does,
inside `mm-decision`, and the controller waits for that call. It is not a recursive controller turn.
The tool card's `data` includes `core`, `admitted`, `answer`, `confidence`, and `decision_id`.

## 9. FocusMw

After a successful tool, if `result.focus` is set, emit a `focus` frame:

- `Document`: the client calls `GET /api/focus/{id}` (existing) and renders the registered viewer.
- `State`: the client renders `StateBoard` (`04` §3) and does not request a focus doc.

A failed tool does not move the pane. The operator keeps what they were looking at, and the tool card
shows the failure. That is deliberate: a refused promote must not wipe the change-set document.

`focus.patch` success emits `focus` again with the same id so the viewer refetches.

## 10. HonestyMw

After the final text is chosen, a small checker runs. It does not call a model.

- If the text matches `(?i)(I (have )?(stored|remembered|promoted|loaded|decided|written))` and no
  `ToolRecord` in this turn has `ok: true` for a write of that family, emit `controller.unbacked` and
  set `turn.unbacked = true`. The client shows a chip "not backed by a tool" on that bubble. Phase 1
  ships the chip for the scripted cases in the test list. The regex is a backstop, not a parser.
- If a tool result has `ok: false`, the final prompt already includes that `reason`. HonestyMw checks
  the final text contains the reason's first 40 characters or the tool name. If it contains neither,
  it appends one system line under the bubble: the reason. The being's sentence stays. The reason
  cannot be dropped on the floor.
- Numbers in the final text are not checked against the whole kernel. They are checked against tool
  `summary` strings when the summary contains a digit: a test-only mode (`MM_CONTROLLER_STRICT_FACTS`,
  off in production) rejects the turn in unit tests when a digit sequence in the reply is absent from
  facts and tool summaries. Production does not block on it.

Unavailable copy the tools must pass through unchanged when that is the whole result:

- "no language-model provider is configured"
- "No events are recorded."
- "No change sets are recorded."
- "no run is recorded"
- "No stage timings are recorded for this run."
- "use the change-set flow" when a tool tries to load a module without a change set
- the image caption sentence, until images are real parts

The being may add a short clause before the reason. It may not replace the reason with a softer one.
HonestyMw's "first 40 characters" rule is what makes that testable.

## 11. TranscriptMw

Frames use the existing `ChatFrame` enum. Additions, all optional:

| Frame field | When |
|---|---|
| `voice` on `llm_request_started` | the five projected numbers, so the transcript explains the tone |
| `unbacked: true` on the final assistant frame | HonestyMw |
| `focus: "state" \| { doc id }` | already representable as a focus frame; `state` is a new id the client recognizes |

`export_types` regenerates the TypeScript. Hand-edits to `frontend/src/lib/wire` are still forbidden.

Sequence numbers stay per-run, starting at 1, including the intro which belongs to a synthetic run
`intro` so a later user run starts at 1 again. The reducer already resets `seen` on a new `started`.
The intro run completes before the composer is enabled.

## 12. Process, locks, and the CLI

`mm-serve` is the writer for its `--data-dir` for as long as it holds `console.lock` and the graph.
The controller does not take an extra lock. Subsystem calls use the open handles.

`mm-cli` against the same directory while the server is up will fail at `GraphStore::open` with the
RocksDB lock error. The guide chapter `start` says this in one sentence. `doctor.run` inside the
server does not hit that error because it does not reopen the graph. Operators who want a scratch CLI
session use another `--data-dir`. Operators who want the CLI on the console's data stop `mm-serve`
first. There is no shared-writer mode in this plan.

Environment: `mm-serve`'s `main` already calls `load_project_env()` before clap, so `OPENAI_*` from
`<repo>/.env` are present. The controller reads them only through `ProviderEnv::from_env()` inside
`OpenAiClient`. Tool code never prints the key. Logs use `ProviderEnv::redacted()`.

A controller turn that itself calls `decide.ask --core hosted` can therefore have two completions in
flight on the same provider, one after the other, never overlapping on the same `LlmService` call.
`LlmService` is not documented as internally parallel for one client; ExecMw awaits the decision
before the next model round. That keeps the ledger ordered.

## 13. Failure map

| Failure | Operator sees |
|---|---|
| Being store failed at boot | Intro says so. Voice is the in-memory baseline. `being.*` tools return the boot error. |
| Provider missing | Intro plus the fixed sentence. No tools. State pane still works: it is not model-backed. |
| Provider HTTP error | Tool-less final frame with the `LlmError` text. Run state `failed` only when zero frames were produced; otherwise `completed` with an error block, so the intro is not lost. |
| Schema-invalid intro rephrase | Deterministic intro. |
| Tool timeout | Tool card, reason `{name} timed out`. Being gets one more round. |
| Firewall denial | Tool card, firewall reason. Pane unchanged. |
| Operator rejects confirm | Tool card, `operator rejected`. One closing sentence. |
| Process restart mid-confirm | Run reloads from `chat-sessions.json` in `waiting`. The button still works. The model is not re-invoked until the button. |

## 14. Logging codes

Add to `mm-log` (next to the existing controller-free codes; names are the contract):

| Code | When | Fields |
|---|---|---|
| `controller.session` | session created | `session_id`, `being_store` |
| `controller.intro` | intro stored | `session_id`, `phrased` (bool) |
| `controller.turn` | user turn started | `session_id`, `run_id`, `high_stakes` |
| `controller.tool` | after ExecMw | `tool`, `ok`, `run_id`, `trace_id` |
| `controller.denied` | AdmitMw or firewall | `tool`, `reason`, `run_id` |
| `controller.unbacked` | HonestyMw | `run_id` |
| `controller.confirm` | request or answer | `tool`, `answer` |

`logs verify` must accept the new codes. No code is reused for a different shape.
