# 02 — The tool surface

> Parent: [`INDEX.md`](./INDEX.md) · Previous: [`01_being_and_voice.md`](./01_being_and_voice.md) · Next: [`03_middleware.md`](./03_middleware.md)

Every capability the operator can reach from `mm-cli` or from `mm-cli mcp` becomes a tool the
controller may call. The chat pane stops being a second, poorer CLI. Tools are grouped so a turn sees
a small set, not all 42 schemas at once.

---

## 1. Shape of a tool

```rust
struct ToolSpec {
    name: &'static str,          // "memory.recall"
    summary: &'static str,       // one line, shown to the model
    input_schema: JsonSchema,    // draft 2020-12, additionalProperties: false
    effect: Effect,              // Read | Write | Gate | External
    confirm: Confirm,            // Never | IfMutating | Always
    focus: FocusKind,            // None | State | Document
}
```

`Effect::Read` does not append a domain event except the controller's own `controller.tool` audit.
`Effect::Write` mutates a store through the subsystem's existing API. `Effect::Gate` is a write that
can change production or spend a budget (`promote`, `loop.run`, `module.load`, `gc.apply`).
`Effect::External` leaves the process (`tool.run` of `process.exec`, `pi.run`). External tools are
`Confirm::Always`.

The model sees `name`, `summary`, and `input_schema`. It does not see Rust types. Admission (`03` §5)
sees `effect` and `confirm`. Unknown names are a tool error, not a shell fallback.

Results are always a JSON object:

```json
{ "ok": true, "summary": "one line the being may quote", "data": { }, "focus": null }
{ "ok": false, "summary": "the kernel reason, verbatim", "reason": "…", "data": { } }
```

`summary` is short on purpose so the reply stays short. `data` is the full payload and is what the
focus pane and the tool card render. The model is given `summary` plus a truncated `data` (16 KiB).
The operator's tool card gets the full `data`.

## 2. Groups and disclosure

The provider receives tools in groups. A turn starts with the **core** group only. Other groups are
attached when the operator's message or a prior tool result names that area, or when the model calls
`catalog.open` with a group id. This keeps a Nemotron-class context from carrying every schema on
every "hello".

| Group | When it is attached | Tools |
|---|---|---|
| `core` | every turn | `guide.list`, `guide.read`, `guide.search`, `catalog.open`, `state.show`, `focus.open`, `focus.read`, `session.note` |
| `kernel` | "doctor", "health", "log", "replay", "graph" | `doctor.run`, `logs.head`, `logs.verify`, `replay.run`, `graph.query`, `graph.validate` |
| `being` | "being", "goal", "who are you", identity | `being.show`, `being.goal_add`, `being.goal_list`, `being.verify`, `being.personality_show` |
| `memory` | "remember", "recall", "forget" | `memory.add`, `memory.recall`, `memory.forget`, `memory.verify` |
| `epistemic` | "claim", "evidence", "assumption" | `epistemic.ingest`, `epistemic.show` |
| `library` | "library", "policy", "skill", "case", "frame" | `library.applicable`, `policy.history`, `skill.retrieve`, `case.retrieve`, `frame.missing` |
| `decide` | "decide", "should", "firewall", "risk" | `decide.ask`, `firewall.eval`, `compare.check`, `risk.analyze`, `calibration.show`, `conformance.run` |
| `change` | "change set", "sandbox", "promote", "reject" | `changeset.new`, `changeset.show`, `sandbox.run`, `promote.run`, `reject.run`, `audit.production_tree`, `budget.show` |
| `loop` | "loop", "self-model", "debt", "gc", "module" | `loop.run`, `loop.status`, `self_model.report`, `debt.scan`, `gc.propose`, `gc.apply`, `module.load`, `module.rollback`, `design.show` |
| `sandbox` | "tool", "file", "run", "rollback" | `sandbox_tool.list`, `sandbox_tool.describe`, `sandbox_tool.run`, `action.ledger`, `action.rollback` |
| `author` | "pi", "write the module" | `pi.run`, `pi.ingest` |

`catalog.open` is the escape hatch: the model asks for a group by id, the next model call in the same
turn includes it. A turn may open at most three extra groups. A fourth is refused with a reason so a
confused model cannot pull the entire catalog in one step.

CLI subcommands that are not in the table above (`codex`, `experience`, `llm stats`, `meta`, `regression`,
`verify`, `calibrate`, `mcp`) land in phase 5 as `codex.scan`, `codex.verify`, `experience.compile`,
`llm.stats`, `meta.analyze`, `regression.run`, `verify.run`, `calibrate.score`, `mcp.list`. Phase 2
ships `core`, `kernel`, `being`, `memory`, and `guide`. Phase 4 ships `decide`, `change`, `sandbox`.
Phase 5 ships the rest. The catalog file lists every tool from the start, with `available: false`
until its phase, so the model is told "not wired yet" instead of hallucinating a call that admission
then rejects as unknown. An unavailable tool's `summary` is the honest sentence, for example
"loop.run is not wired on this build".

## 3. Core tools, specified

### 3.1 `state.show`

Read. Input: `{ "sections": ["health", "being", "budgets", "goals"] }`, default all four. Data is the
same structures the state pane renders (`04` §3). Focus kind `State`, which clears a document focus
and shows the state board. This is how "show me your state" works without a document.

### 3.2 `focus.open`

Read. Input: `{ "id": "<focus id or tool-result id>" }`. Ids are the ones `mm-serve` already serves
(`/api/focus/:id`) plus ids minted by a tool result (`focus_id` on the result). The pane navigates to
that document. The model does not send Markdown to be rendered as a document; it sends an id.

### 3.3 `focus.read`

Read. Input: `{ "id": "<id>", "range": optional }`. Returns the document body the viewer has, so the
model can talk about the focused doc without inventing its contents. If the operator has a document
focused, the turn's system block includes that id and a 2 KiB preview automatically (`03` §4), and the
model calls `focus.read` only when it needs the rest.

### 3.4 `focus.patch`

Write, `Confirm::IfMutating`. Input: `{ "id", "patch": { "op": "set_field"|"insert_row"|"replace_range", ... } }`.
Only document kinds that already have an editor accept a patch (trees that are live, not fixtures).
A fixture document — any focus doc whose snapshot reason says it is a fixture — rejects the patch with
that reason. The controller says so and does not write a parallel copy. Live patches go through the
subsystem that owns the data (a goal edit is `being.goal_add` or a later goal update, not a raw RDF
string from the model). `focus.patch` is a thin adapter: it refuses kinds it cannot map, and for kinds
it can map it calls the owning tool. Phase 3 wires the pane; phase 4 wires the mutating adapters.

### 3.5 `session.note`

Write to the transcript only. Input: `{ "text": "…" }`. Used when the operator says "remember for this
chat" and a `memory.add` would be too strong. It is not autobiographical memory. It does not enter
`/being`.

## 4. Kernel tools that already have a route

These replace the bespoke arms in `chat::execute`. The old arms call the new tools so there is one
implementation.

| Tool | Wraps | Confirm | Notes |
|---|---|---|---|
| `doctor.run` | the health envelope plus, once the being is open, ontology triple count from the graph | Never | Does not spawn `mm-cli doctor`. Reads stores already open. |
| `logs.head` | `EventLog::head_seq` | Never | |
| `logs.verify` | the audit-chain verifier the CLI uses | Never | Read-only, may be slow. Step budget still counts it as one tool. |
| `replay.run` | `mm-eventlog` replay | Never | Input `from`, `until`, `as_of`. Returns the state hash. Does not write. |
| `graph.query` | SPARQL on the open `GraphStore` | Never | Input is a query string, `SELECT`/`ASK` only. `INSERT`/`DELETE` is rejected here; graph writes go through subsystem APIs. |
| `graph.validate` | SHACL for one named graph | Never | |
| `budget.show` | the four seeded rows | Never | Same rows the budget route returns. No invented meters. |
| `changeset.show` | the change-set list | Never | Empty list keeps the sentence "No change sets are recorded." |

`graph.query` is the one new read that can be abused as a write if the query language is not closed.
The executor parses with the SPARQL parser already in `mm-store-graph` and refuses any query that is
not a read. That check is code, not a prompt.

## 5. Being, memory, decision

| Tool | Effect | Confirm | Gate |
|---|---|---|---|
| `being.show` | Read | Never | The facade's show snapshot: identity, invariants, affect, goal counts, accounts. |
| `being.goal_add` | Write | IfMutating | `BeingOp` through `IdentityGuard`. Description and priority required. Origin `operator`. |
| `being.goal_list` | Read | Never | |
| `being.verify` | Read | Never | The invariant corpus. Failures are data, not a cheerful summary. |
| `being.personality_show` | Read | Never | Values, dispositions, constraints, and the projected `chat` params. The model uses this to explain its own voice. It cannot set dispositions in this plan. A `being.personality_set` tool is out of scope until a later plan, because a chat should not retune its constitution mid-conversation. |
| `memory.add` | Write | IfMutating | Kind required (`semantic` or `episodic`). Content is the operator's words, not a paraphrase, unless the operator asked to paraphrase. |
| `memory.recall` | Read | Never | Returns hits with the five score terms. The reply may quote the top hit. It may not round the scores into a vibe. |
| `memory.forget` | Write | Always | Dry-run unless `apply: true`, matching the CLI. `apply: true` still confirms. |
| `memory.verify` | Read | Never | SQLite against `/memory`. |
| `decide.ask` | Write | Never for `rules`; Always for `hosted` and `local` | Input is the bounded question JSON the CLI already accepts (`kind`, `prompt`). `core` defaults to `rules`. `hosted` uses `LlmService` inside `mm-decision`, which is a second model call and must be visible as its own tool card, not merged into the controller's reply. `no_record` is available and is the default when the operator said "don't save this". |
| `firewall.eval` | Read | Never | One episode or an inline action. Returns the verdict and the reason. The controller does not override a denial. |
| `promote.run` | Gate | Always | Requires a change-set id that exists. Runs the same gate as `mm-cli promote`. The reason string is the gate's, stored on the decision. |
| `reject.run` | Gate | Always | `reason` is required in the input schema (`minLength: 8`). |
| `loop.run` | Gate | Always | Goal file must be a path under `bench/` or a goal id already in the being store. No arbitrary path. Budget file defaults to the shipped qualification budget. |
| `sandbox_tool.run` | External | Always | The MCP executor's `tools/call`, in-process, with principal `operator`. Default-deny permissions stay in `mm-tools`. A denial is the tool result. |
| `action.rollback` | Write | Always | Refuses when no snapshot exists, with the CLI's reason. |

`decide.ask` with `core: hosted` is the path that talks to `OPENAI_BASE_URL` for a judgment. The
controller's own turns also talk to that endpoint. They are different purposes (`Converse` vs the
decision purpose `mm-decision` already uses) and different ledger rows. The state pane can show both.

## 6. Confirmations

`Confirm::Always` and a mutating call under `Confirm::IfMutating` emit an `input_request` frame before
execution:

```json
{
  "kind": "approve",
  "title": "memory.add",
  "body": "Store a semantic memory: \"the migration order is 0001 through 0012\"",
  "options": ["Approve", "Reject", "Defer"]
}
```

The body is built from the tool input by code, not by the model, so the operator confirms what will
actually run. Reject records the tool result `ok: false`, `reason: "operator rejected"`. Defer leaves
the request on the transcript and ends the turn. Approve runs the tool. A turn never auto-approves a
`Gate` or `External` effect, including when the operator's message said "yes, do it". The message is
why the model *proposed* the call. The button is why it *ran*.

`Confirm::Never` reads run immediately and appear as tool cards.

## 7. Focus results

A tool with `focus: Document` returns `focus: { "id", "kind", "title" }`. Middleware emits the existing
`focus` SSE frame. The client sets the pane to that document and nothing else. Examples:

| Tool | Document |
|---|---|
| `budget.show` | the budgets tree, live rows |
| `being.goal_list` | the goals tree |
| `memory.recall` | a table of hits |
| `changeset.show` | the change-set list, or the empty-state sentence |
| `decide.ask` | the decision card (verdict, core, confidence, admitted) |
| `sandbox.run` | the sandbox report |
| `logs.head` | the log document, tail from the event head |

A tool with `focus: State` emits `focus` with `id: "state"`, which is not a `FocusDoc` in the catalog.
The client switches the pane to the state board (`04` §3). `focus: None` leaves the pane where it is.

## 8. What is deliberately not a tool

- A raw shell (`bash`, `cargo`, `sh`).
- `process.exec` except through `sandbox_tool.run`, which already default-denies it.
- Writing `config/llm.toml` or `.env`. The operator edits those on disk. `state.show` may display the
  redacted host and the model names.
- Setting personality dispositions from chat (`01` §3, `being.personality_show` only).
- Loading an arbitrary module URI. `module.load` accepts only URIs that `module list` returned or that
  are already in `codex.lock`.
- Calling `mm-cli` as a subprocess "to be sure". One implementation, in process.
