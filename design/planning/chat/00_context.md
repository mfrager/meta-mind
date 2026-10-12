# 00 — Context and objectives

> Parent: [`INDEX.md`](./INDEX.md) · Next: [`01_being_and_voice.md`](./01_being_and_voice.md)

---

## 1. What exists today

The kernel is a Rust workspace driven by `mm-cli` (42 subcommands in `crates/mm-cli/src/main.rs`).
State lives in one data directory: SQLite, the Oxigraph/RocksDB graph, the JSONL log, the ULID
watermark. The invariant that already governs every command stays in force:

> The LLM proposes; the control plane structures; the decision core makes bounded judgments; symbolic
> layers prove; deterministic code enforces; the environment decides truth; memory preserves;
> meta-analysis improves. **Production is written only by promotion.**

The web console (`crates/mm-serve`, `frontend/`) is a local window on that kernel.

- `mm-serve` binds `127.0.0.1:8899`. On boot it opens SQLite, the event log, and the RDF store
  (`AppState::boot` in `crates/mm-serve/src/state.rs`). It does **not** open `BeingFacade`. The chat
  route's being reply is still the sentence in `BEING_REASON`.
- The Vite host binds `0.0.0.0:8900` and proxies `/api`.
- `GET /api/health` reports sqlite, eventlog, graph, and llm. The llm line is `ok` when both
  `OPENAI_BASE_URL` and `OPENAI_API_KEY` are set, and the reason names the models in `config/llm.toml`
  plus a redacted host. That line does not mean a completion has been made.
- `POST /api/chat` classifies the query in `crates/mm-serve/src/parity.rs`: first token against the 42
  names, plus two phrases ("stage" + "slow|why", and "production-tree"). A handful of names read the
  kernel (`budget show`, `doctor`, `logs`, `graph`, `changeset show`). The rest return a fixed reason.
  There is no `LlmClient` on this route. The file header says so.
- The transcript already has the frames a controller needs: `llm_request_started`, `llm_response`,
  `llm_request_finished`, `tool_call`, `tool_result`, `focus`, `input_request`. They are defined in
  `crates/mm-serve/src/dto/chat.rs` and rendered by `frontend/src/components/chat/`.
- The shell is a menu plus a two-pane split (`WorkspaceSplit`): a document pane and a chat pane, a
  command palette of the 42 names, a status bar, and routes for `/`, `/chat/:sessionId`, `/map/:source`,
  `/settings`, `/dev/catalog`.

The being already has a personality constitution, not a script. `Personality::baseline()` in
`crates/mm-being/src/personality.rs` stores values, five dispositions, and constraints. `project`
turns a context into `BehaviorParams` (warmth, directness, curiosity, playfulness, formality) by
arithmetic. The model is supposed to elaborate tone from those numbers. Nothing in the console reads
them.

The sandbox tools already have an MCP bridge: `mm-cli mcp serve` exposes `tools/list` and `tools/call`
over stdio (`crates/mm-cli/src/tools_cmd.rs`). That bridge is not connected to the chat pane. Its tools
are the registered sandbox tools (filesystem, process, and the rest under `mm-tools`), not the 42 CLI
subcommands.

## 2. The gap

An operator who opens the console cannot have a conversation, cannot be told how the system works by
the system, and cannot ask the being to do the work. They must already know a command name. The left
pane is a catalog of documents, many of them fixtures, rather than the thing the conversation is about.

The classifier must be retired as the executor. It can stay for one release as a fallback only when no
provider is configured, and phase 5 removes even that.

## 3. Objectives

1. **One speaker.** Every assistant turn is the being, using the personality stored for this data
   directory. A fresh being uses `Personality::baseline()` plus the chat projection in
   [`01_being_and_voice.md`](./01_being_and_voice.md). The starting voice is friendly and short.
2. **One introduction.** The first frame of a new session is the controller speaking, before the
   operator types. It is filled from live facts, not from a model guess. See `01` §4.
3. **One teacher.** The controller can explain how to use the system because it has a guide tool whose
   text is the operator manual, not whatever the model remembers. See `01` §5.
4. **One actor.** Every capability the CLI and the MCP bridge can reach is a tool the controller may
   call. The operator does not run a second UI to do the same work. See `02`.
5. **One process.** A turn is a fixed middleware pipeline inside `mm-serve`, on the stores that process
   already holds. The model never receives a shell, a connection string, or the API key. See `03`.
6. **One screen.** The default view is the conversation plus either the being's state or the single
   document in focus. The controller opens and edits that document by tool. See `04`.
7. **The old gates stay.** Unavailable stays unavailable. Invariants stay in `IdentityGuard`. Production
   writes stay behind a change set, a sandbox, and `promote`. Budgets still deny a stage whole.

## 4. Non-goals

- Replacing `mm-cli`. Headless use, CI, and `scripts/ui_flows.sh` keep calling the binary.
- A second personality system. No prompt file that overrides `Personality` when they disagree. The
  constitution wins; the prompt only renders it.
- Letting the model pick an arbitrary shell command, HTTP URL, or file path outside the tool catalog.
- Opening the graph from both `mm-serve` and `mm-cli` at once. RocksDB is the writer. The design says
  which process holds it (`03` §8).
- Inventing live numbers. A tool that has no row returns the kernel reason. The state pane renders that
  reason. It does not draw a placeholder gauge.
- Multimodal tool use. Image attachments stay on the transcript as captions, with the existing sentence
  "this route records images as attachments; the model will see captions only", until a later plan
  gives the provider an image part.
- A public network service. `mm-serve` stays on loopback. The Vite host may stay on `0.0.0.0`.

## 5. Locked decisions

| Decision | Choice |
|---|---|
| Where the controller lives | `crates/mm-serve/src/controller/`, called by the existing chat routes. Not a new crate in phase 1. A crate split is allowed in phase 5 only if `mm-serve` becomes a thin HTTP shell. |
| Who speaks | The being for the data directory `mm-serve` opened. One being, one conversation partner. |
| Starting voice | `Personality::baseline()` projected with the `chat` context in `01` §3. Replies default to one to three sentences. |
| How the model is called | `LlmService` with `Purpose::Converse` (new) and `OpenAiClient`. Tool calls are a provider-native `tools` array validated against the catalog. Mock client in tests. No direct `reqwest` from the controller. |
| How a tool runs | In-process, on `AppState`'s stores. Never `std::process::Command` of `mm-cli`. |
| How a document changes | A focus tool returns a `FocusDoc`. The HTTP route already serves it. The model does not push DOM. |
| How a destructive call is confirmed | The existing `input_request` frame (Approve / Reject / Defer). The tool does not run until the answer is Approve. |
| What happens with no provider | The session still opens and the deterministic introduction still appears. Turns say the provider is not configured, in the being's words, and do not fall through to a fake answer. Phase 1 may keep the old classifier behind an explicit "command mode" chip; phase 5 deletes it. |
| Wire version | Stay on wire version 1. New frames are additive, with `skip_serializing_if`. Regenerate `frontend/src/lib/wire` only through `cargo test -p mm-serve export_types -- --ignored`. |
| UI law | Tokens, radius lint, and honesty strings from the UI plan still apply. |

## 6. Personas

**The operator** sits at the console and types what they want. They may know nothing about subcommand
names. They can ask "what is a change set", "remember this", "should I promote that", "show me the
budget". They confirm the few calls that write production or spend a budget.

**The being** is the only assistant. It is not a help desk bolted on. Its self-description, goals, and
personality are the ones in the being store. When those are the baseline, it is a friendly, direct
partner that would rather say "that store is closed" than guess.

**The kernel** does not speak. It admits, runs, and returns reasons. Middleware is what turns those
reasons into a turn the being can finish.
