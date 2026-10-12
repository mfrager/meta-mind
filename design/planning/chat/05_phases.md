# 05 — Five implementation phases

> Parent: [`INDEX.md`](./INDEX.md) · Previous: [`04_ui.md`](./04_ui.md)

Each phase is shippable on its own. Later phases do not start inside an earlier phase's pull request.
`mm-cli` keeps working at every phase. The console's honesty rules keep working at every phase: a
route that does not yet have a tool still returns its reason, and the controller quotes it.

Paths below are the files to add or change. Tests are part of the phase, not a follow-up.

Shared gates, run from the repo root at the end of every phase:

```bash
TMPDIR=$HOME/tmp CARGO_TARGET_DIR=$PWD/target cargo test -p mm-serve -- --test-threads=1
cd frontend && npm run typecheck && npm run test
```

Phase 3 and after also run `npm run test:e2e` and `./scripts/ui_flows.sh --offline`. `ui_flows.sh`
talks to the HTTP API, not to the model. It must keep passing against the classifier until phase 5
switches it to the controller's tool endpoint with `MockClient`. Do not point `ui_flows.sh` at a live
provider.

The provider in tests is always `MockClient`. A live call to `OPENAI_BASE_URL` is a manual check
recorded in the phase notes, not a CI dependency.

---

## Phase 1 — The being speaks

**Outcome.** A new session introduces itself from the live being. A user message gets a short reply
in the projected voice. No tools yet. If the provider is missing, the intro and the fixed sentence
still appear.

### 1.1 Server

- `AppState` gains `being: Option<Arc<BeingFacade>>` and `being_error: Option<String>`. `boot` calls
  `BeingFacade::open` on the graph and sqlite it already holds. Failure is stored, not fatal.
- Health grows a `being` service: `ok` when the facade is open, `unavailable` with `being_error`
  otherwise. Kernel rollup treats a failed being open like a failed graph open: the kernel is
  `degraded`. A missing *provider* does not degrade the kernel.
- First open of an empty being store runs the facade's identity init with the self-description in
  `01` §2.1 and `Personality::baseline()`. A second boot does not reset dispositions. Test: set warmth
  to `0.2`, reboot the state in the same data dir, warmth is still `0.2`.
- New module `crates/mm-serve/src/controller/`:
  - `persona.rs` — load personality, apply `chat` and optional `high_stakes`, render the four prompt
    blocks.
  - `intro.rs` — `IntroFacts` and the deterministic text. Optional rephrase through `LlmService`
    with a schema and no tools. Fallback to the deterministic text.
  - `turn.rs` — one model round, no tool loop. Caps unused.
  - `prompt.rs` — the length rule and the conduct lines, as constants.
- `Purpose::Converse` added in `mm-llm`. `OpenAiClient` unchanged except that a converse call with no
  tools is a normal chat completion. Ledger row purpose `converse`.
- `POST /api/chat/sessions` writes the intro run before returning the session id.
- `POST /api/chat` : when `command_mode` is absent or false, call `Controller::turn`. When
  `command_mode: true`, keep today's `classify` path so `ui_flows.sh` stays green. The flag is a JSON
  field, default false. The web client does not send it.
- Personality modifier `chat` is data in `persona.rs` passed into `ContextVector`, not a fork of
  `Personality::project`. If the baseline constitution gains the modifier permanently, that is a
  one-line addition to `Personality::baseline`'s `context_modifiers`. Do it there so `being show` and
  the controller agree. The deltas are exactly the table in `01` §3.

### 1.2 Voice tests

- Project baseline with `chat` only: warmth `0.70`, directness `0.80`, playfulness `0.30`, formality
  `0.40`, curiosity `0.70`, within `1e-5`.
- Project with `chat` and `high_stakes`: playfulness `0.00` (0.40 − 0.10 − 0.30, clamped), formality
  `0.80` (0.50 − 0.10 + 0.30). Directness `0.90`.
- A stored warmth of `0.9` projects to `1.0` under `chat` (clamped), not to `0.70`.
- Prompt snapshot contains the self-description, the four invariant codes, and the five numbers.
- Intro with the mock model returning invalid JSON stores the deterministic text.
- Intro with the mock model returning a 600-character string stores that string only if every integer
  in it appears in `IntroFacts`. A stray `99` fails the test helper and the server falls back. This
  helper is the same function production uses.
- No provider: intro is deterministic, user turn returns the fixed sentence, `llm_calls` has no new
  row.

### 1.3 UI in this phase

Do not rearrange the shell yet. The existing chat pane shows the intro as the first bubble and the
reply as the second. Suggestion chips for a new session come from the server: the three strings in
`01` §4.3. They send themselves as user text.

### 1.4 Pass gate

- `cargo test -p mm-serve controller::` green.
- `cargo test -p mm-being` still green (baseline deltas live in `Personality::baseline` only if that
  edit landed; the being crate's existing projection tests must be updated in the same commit to expect
  the `chat` modifier when the context is empty they must still expect raw baselines — `chat` is applied
  by the controller, not by `project` of an empty context).
- Manual: start `./start-mm-serve.sh`, open the console, confirm the first bubble is the introduction
  and a question receives a short reply. Record the model name from the llm health line in the phase
  note. Then stop the server.

### 1.5 Explicitly not in this phase

Tools, confirmations, state board, nav removal, `focus.patch`, hosted `decide`.

---

## Phase 2 — Read tools and the guide

**Outcome.** The being can explain the system from the guide and can read the kernel: health, logs,
graph reads, budgets, being, memory recall, change-set list. Writes are not offered (`available: false`).

### 2.1 The tool runtime

- `controller/catalog.rs` — the full `ToolSpec` list from `02`, with `available` flags. Phase 2 sets
  `available: true` for group `core` (except `focus.patch`), group `kernel`, `being.show`,
  `being.goal_list`, `being.verify`, `being.personality_show`, `memory.recall`, `memory.verify`,
  `budget.show`, `changeset.show`.
- `controller/admit.rs` — the checks in `03` §6 that do not need the firewall yet. Schema validation
  via the `jsonschema` crate already in the workspace if present, or the validator `mm-llm` uses for
  structured output. One validator, not two.
- `controller/exec.rs` — the function table. Implementations call existing store methods. `graph.query`
  refuses non-reads.
- `controller/guide/` — the eight Markdown chapters in `01` §5. `guide.search` is token overlap.
- `controller/turn.rs` grows the loop in `03` §5.3 with caps 4 and 6. `MockClient` scripts tool calls.
- Extend `LlmRequest` / `LlmResponse` / `OpenAiClient` with `tools` and `tool_calls` as in `03` §5.2.
  A unit test in `mm-llm` locks the JSON body: `tools[0].type == "function"`, no `response_format`
  on that request.
- SSE: existing `tool_call` and `tool_result` frames. Add `voice` on `llm_request_started` if the
  TypeScript export can take an optional field without a wire version bump. If a golden fixture test
  rejects the extra field, put `voice` only in the log record and skip the frame. Do not break
  `fixtures/chat_frames.json` silently; update it through the generator.
- `chat::execute` arms for budget, doctor, logs, graph, changeset call the new tools when
  `command_mode` is true, so there is one body of code. The HTTP shapes those tests assert stay
  stable: `ui_flows.sh` still finds `meta_analysis`, `RDF`, and "No change sets are recorded."

### 2.2 Middleware behaviour to test

- "how do I use this?" scripted as a model that calls `guide.read` with `start`, then answers using a
  sentence from the chapter. The tool card is on the transcript. The final text is under 600
  characters in the script.
- A scripted model that calls `loop.run` gets `available: false` and the spec sentence. No execution.
- A scripted model that calls `graph.query` with `INSERT` gets the read-only reason.
- `catalog.open` of `kernel` then `doctor.run` succeeds. A fourth `catalog.open` in one turn is refused.
- `memory.recall` on an empty store returns the kernel's empty result, not a fabricated hit.
- Step cap: a mock that always returns a tool call stops at 6 tool results and one final summarize
  call.
- `controller.tool` and `controller.denied` appear in the log, and `logs verify` accepts them.

### 2.3 Guide content

Write the chapters from `infodocs/user-guide.md`, shortened. Each chapter starts with the result the
operator cares about, then the tool the controller will call, then the CLI equivalent for scripts.
The `start` chapter includes the RocksDB single-writer rule (`03` §12). The `chat` chapter describes
phase 2's UI (chat pane still inside the old shell) and is rewritten in phase 3.

### 2.4 Pass gate

- `cargo test -p mm-llm openai::tools_body` and `cargo test -p mm-serve controller::` green.
- `./scripts/ui_flows.sh --offline` green with `command_mode` still the script's path (the script does
  not send the flag, so phase 2 must keep the default **for that script** working). Practical choice:
  the script sends `"commandMode": true` in this phase, and the web client sends false. Add the field
  to the script in this phase. Document it in the script header.
- A manual conversation: ask how promotion works, see a `guide.read` card for `change`, see a short
  answer. Ask for budgets, see the four names in a tool card.

### 2.5 Explicitly not in this phase

Confirms, firewall, writes, UI shell rewrite, hosted decide.

---

## Phase 3 — The screen

**Outcome.** The default route matches `04`. The operator cannot act except by talking. The side pane
is the state board or one focused document.

### 3.1 Frontend

- `frontend/src/components/state/state-board.tsx` and a small `useStateBoard` query that calls
  `GET /api/state` (new, returns the five sections). The route is a read. It is also what `state.show`
  returns, so the board and the tool share `controller/tools/state.rs`.
- `frontend/src/routes/workspace.tsx` becomes `SidePane` plus `ControllerPane`. Delete the document
  index from this route.
- `frontend/src/components/blocks/app-shell.tsx` drops `NAV_SECTIONS` from the default layout. "More"
  links to `/settings` and `/dev/catalog`.
- `frontend/src/components/shell/command-palette.tsx` stops enqueueing chat commands. Ctrl/⌘K focuses
  `#composer`. Guide search inside a palette is optional and off unless it is cheap; the controller
  is the search. Do not leave a second executor "just in case".
- Composer: remove the slash menu. Placeholder "Ask Metamind…".
- Focus frames with id `state` select the board. Other ids fetch `/api/focus/:id` as today.
- Status bar: remove the unwired budget ticks. LLM chip uses the health reason.
- Keep `/dev/catalog` tests and the parity count. They are not the product surface.

### 3.2 Focus from tools

- `budget.show`, `being.goal_list`, `changeset.show`, `memory.recall`, and `doctor.run` set `focus`
  as in `02` §7. `doctor.run` focuses `state`, not a document.
- Client test: a fixture transcript with a `focus` frame for budgets renders the budgets viewer and
  not the state board. A following `focus` of `state` renders the board again.
- "Back to state" is local UI state until the next turn, when ContextMw reports whichever pane is
  actually showing.

### 3.3 Editors

Viewers only. Hide or disable inline edit, drag-and-drop reorder, and save on the focused document.
The disabled control's accessible name says "Ask Metamind to change this." It does not call an API.

### 3.4 Tests

- Playwright, viewport 1440×900: a fresh session shows the intro, no left document index, composer
  placeholder "Ask Metamind…", and the state board's kernel line. Mock the model by running `mm-serve`
  with the provider unset: the intro is deterministic, the board still renders health. This test must
  not require a live LLM.
- A second Playwright test drives `commandMode` off and a test-only hook `MM_CONTROLLER_SCRIPT` (a
  path to a MockClient script) so the server returns a scripted `budget.show` focus. The pane shows
  `meta_analysis`. The hook is read only when `cfg` is a temp data dir or `MM_CONTROLLER_SCRIPT` is
  set, and it refuses to start if `OPENAI_API_KEY` is also set, so a script cannot silently replace
  a live provider. If that coupling is too sharp, gate the hook on `--controller-script` clap flag
  instead, used only by Playwright.
- Keyboard: Ctrl/⌘K focuses the composer. Esc does not send. Confirm buttons are absent.
- Radius lint and the contrast tests stay green. No new `rounded-xl`.

### 3.5 Pass gate

- `npm run test:e2e` green.
- Manual: the five stories in `04` §9 that do not need writes (1, 2, 3, 4, 5). Story 6 waits for
  phase 4. Story 2 needs the script hook or a live provider; do the live one once and keep the script
  hook as the regression.

### 3.6 Explicitly not in this phase

Mutating tools, Approve buttons, promotion.

---

## Phase 4 — Writes, gates, and the focused document

**Outcome.** The controller can change the being, memory, decisions, change sets, and sandbox tools.
Every mutation the catalog marks `IfMutating` or `Always` waits for Approve. Production still moves
only through `promote.run`. The focused document can be patched where a live editor exists.

### 4.1 Admission and confirm

- Implement ConfirmMw and `POST /api/chat/{run_id}/answer`.
- Persist `PendingCall` in the run body inside `chat-sessions.json`. Restart the process, reload the
  session, press Approve, and see the tool run. Test this with a temp data dir.
- Firewall call before writes, as in `03` §6. A scripted denial blocks `memory.add`. The button must
  not appear if admission already failed.
- `confirm_body` snapshots for `memory.add`, `memory.forget` with `apply: true`, `promote.run`,
  `sandbox_tool.run`, `loop.run`.

### 4.2 Tools to enable

Set `available: true` and implement exec for:

- `being.goal_add`
- `memory.add`, `memory.forget`
- `decide.ask` for `rules` first, then `hosted` and `local`
- `firewall.eval`, `compare.check`, `risk.analyze`
- `changeset.new`, `sandbox.run`, `promote.run`, `reject.run`, `audit.production_tree`
- `sandbox_tool.list`, `sandbox_tool.describe`, `sandbox_tool.run`, `action.ledger`, `action.rollback`
- `focus.patch` for goal description and for memory content only, both delegating to the owning tool
  and therefore confirming

`changeset.new` accepts either a gap file under `bench/gaps/` or a structured gap
`{ title, hypothesis }` that the existing change-set constructor already allows. If the constructor
requires a file, the tool writes a temp gap under the data dir and passes that path. It does not
accept a path outside the repo and the data dir.

`decide.ask` with `hosted` is `Confirm::Always` because it spends a provider call and writes a
decision row. The tool card shows `core` and `admitted`. An `admitted: false` result is a normal
success of the tool: the core abstained. The being says so. It does not shop for another core unless
the operator asks.

`promote.run` calls the same function `mm-cli promote` calls. If that function still lives only in
`mm-cli`, move the gate invocation into `mm-selfeng` (or call the existing library function if it is
already there) and leave `mm-cli` as a printer. The CLI output tested by e2e stays. The reason string
"the gate found nothing to reject" remains the gate's sentence when that is the outcome.

### 4.3 Document edits

- For a focused goal list, an edit control sends `focus.patch`, which proposes `being.goal_add` or a
  goal update if one exists. The transcript shows the confirm. The list refetches only after `ok`.
- Fixture docs: the control stays disabled, with the fixture reason as its title.
- There is still no direct `PUT` from the browser to the being store. Grep the frontend for a write
  to `/api/being` or `/api/memory` and keep it empty. Writes go to `/api/chat` and the answer route.

### 4.4 Honesty

- Script: model says "I stored that" with no tool. Bubble is marked unbacked.
- Script: `memory.add` returns `ok: false` with a reason. The final text that omits the reason gets
  the system line appended. Test the append, not the model's manners.
- Script: operator presses Reject. Result reason is `operator rejected`. No row in the memory table.

### 4.5 Pass gate

- A temp-dir integration test, no network: scripted controller adds a goal after Approve, `being show`
  through the tool lists it, Reject on a second add leaves the count unchanged.
- Same for `changeset.new` from `bench/gaps/gap_01.json`, then `promote.run` after Approve, then
  `audit.production_tree` reports one promotion. This is the CLI tour in `infodocs/user-guide.md`
  §5.5, driven by tools.
- `cargo test -p mm-selfeng` and the promotion e2e still pass.
- Manual: one hosted `decide.ask` against the configured endpoint, recorded as a note, not as CI.

### 4.6 Explicitly not in this phase

`loop.run` execution (the tool may exist as `available: false` until phase 5 if the loop runner cannot
be called in-process cleanly). Personality mutation. Module hot-load.

If `loop.run` is easy to call in-process because `mm-runtime` already exposes it, implement it here
under `Confirm::Always` and the path allow-list. Do not shell out to `mm-cli loop run`.

---

## Phase 5 — The wrap

**Outcome.** Every CLI capability in `02` §2 is a tool, including the ones left unavailable. The old
classifier is gone. The console's only interactive executor is the controller. `mm-cli` is unchanged
for operators and CI.

### 5.1 Remaining tools

Enable and implement, each with admit checks from `02`:

- `logs.verify`, `replay.run`, `graph.validate`
- `epistemic.ingest`, `epistemic.show`
- `library.applicable`, `policy.history`, `skill.retrieve`, `case.retrieve`, `frame.missing`
- `calibration.show`, `conformance.run`, `calibrate.score`
- `loop.run`, `loop.status`, `self_model.report`, `debt.scan`, `gc.propose`, `gc.apply`
- `module.load`, `module.rollback`, `design.show`
- `pi.run`, `pi.ingest` (`Confirm::Always`, `Effect::External`)
- `codex.scan`, `codex.verify`, `experience.compile`, `llm.stats`, `meta.analyze`, `regression.run`,
  `verify.run`, `mcp.list`

`gc.apply` and `module.load` and `module.rollback` are `Confirm::Always`. `gc.propose` is a read.
`module.load` without a change set returns the existing sentence "use the change-set flow" from the
subsystem, not a new sentence. `mcp.list` is the in-process `tools/list`. It does not start a stdio
server. `sandbox_tool.run` remains the way a listed tool is called.

`pi.run` uses the Pi integration the CLI uses. If Pi is absent on `PATH`, the tool result is that
absence. The controller does not download Pi.

### 5.2 Retire the classifier

- Delete `command_mode` and `parity::classify` as an executor. `COMMANDS` may remain as the list the
  catalog is checked against: a test asserts every `CommandSpec::name` has at least one tool whose
  summary mentions it, so a new CLI subcommand fails the test until a tool exists.
- `scripts/ui_flows.sh` drives the controller with `--controller-script` pointing at
  `crates/mm-serve/tests/fixtures/ui_flows_script.json`. The script's assertions (budget names, empty
  change sets, module load 409, guide RDF or graph open) stay. The script no longer posts raw command
  strings expecting the classifier.
- The HTTP error for `POST /api/modules/{id}/load` stays 409 with "use the change-set flow" for any
  leftover caller. The web client has no such caller.

### 5.3 One writer, documented in the product

- The intro's guide chapter `start` is the operator-facing copy.
- `doctor.run` includes a line `graph_holder: this process` when the store is open.
- A test boots `mm-serve` on a temp dir and asserts a second `GraphStore::open` on that dir returns an
  error. The controller does not try to be clever about it.

### 5.4 End-to-end conversation

One Playwright test with `--controller-script` covering, in one session:

1. Intro is present.
2. "How do I use this?" → guide tool → short final text.
3. "Show me your state" → state board.
4. "Remember that the migration order is fixed" → Approve → memory tool ok.
5. "What did I just ask you to remember?" → `memory.recall` → the text.
6. "Promote something" with no change set → the being asks to make one, or the tool returns the empty
   list. No promotion row.

A second test, API-level, runs the gap-file promotion from phase 4 through the scripted controller
and checks `audit.production_tree`.

### 5.5 Cleanup

- Remove dead suggestion-chip command names, dead palette execution, and dead `chat::execute` arms.
- `frontend/README.md` and `infodocs/ui.md` get a short "You are talking to Metamind" section and the
  single-writer note. Do not rewrite the user guide; the guide chapters in the binary are the
  operator manual the being reads.
- Clippy on `mm-serve` for the new module only. Pre-existing lints outside `controller/` are not part
  of this phase unless the phase touches the line.

### 5.6 Pass gate

```bash
TMPDIR=$HOME/tmp CARGO_TARGET_DIR=$PWD/target cargo test -p mm-serve
TMPDIR=$HOME/tmp CARGO_TARGET_DIR=$PWD/target cargo test -p mm-llm --lib
cd frontend && npm run typecheck && npm run lint && npm run test && npm run test:e2e
./scripts/ui_flows.sh --offline
```

Manual, once: a live session against the configured provider that asks for a rules decision and a
hosted decision, confirms the hosted one, and shows two different `core` values on two tool cards.
The reply stays within a few sentences. The state board shows the new goal if one was added.

### 5.7 Done means

The operator can sit at the console, read the introduction, ask how the system works, ask the being
to do the work, watch the state or the document change, and confirm the dangerous parts. They do not
need a command name. The kernel's gates are the same ones the CLI uses, called in-process, on the
stores this server already opened.
