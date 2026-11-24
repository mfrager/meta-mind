# Metamind operator guide — Conversation Manager v3 and Inspector

**Audience.** An operator running the console locally, or an engineer who needs to
know exactly what the conversation manager can do without reading the phase plan.

**Scope.** The Phase 1–12 conversation-manager surface implemented in `mm-cme`,
`mm-prompt`, and `mm-serve`, plus the inspector UI in `frontend/`. The older
kernel CLI (`doctor`, `being`, `memory`, `loop`, …) is documented in
[`user-guide.md`](./user-guide.md); this guide only covers the console.

---

## 1. Start the console

```bash
cargo build --workspace
DATA=$(mktemp -d)
cargo run -q -p mm-serve -- --data-dir "$DATA"
# listening on http://127.0.0.1:8899 (loopback only)
```

In another terminal:

```bash
cd frontend
npm ci
npm run dev          # Vite dev server, proxies /api to :8899
```

Open the printed Vite URL. The inspector is a full-screen route:
`http://127.0.0.1:5173/inspector` (or `/inspector/<sessionId>`). `⌘⇧I` opens it;
`⌘⇧1`–`⌘⇧4` switch the rail tabs.

The server refuses non-loopback binds and takes `data/console.lock` while it runs.
Stop it before running `mm-cli data` against the same data root.

### Model configuration

The CM uses the same `config/llm.toml` model specs as the rest of the system.
Provider credentials come only from the environment:

```bash
export OPENAI_BASE_URL=...
export OPENAI_API_KEY=...
```

With either missing, the deterministic executor is used and every hosted path
reports `Unavailable` with the missing variable names. A scripted controller is
available for tests and offline demos:

```bash
MM_CONTROLLER_SCRIPT=path/to/script.json cargo run -q -p mm-serve -- --data-dir "$DATA"
```

The server refuses a controller script when `OPENAI_API_KEY` is set, so a recorded
script can never silently replace a live provider.

### Feature flags

| Variable | Default | Effect |
|---|---|---|
| `MM_CM_V3` | `true` | Attach the seventeen-tool conversation-manager contract to the CME executor. `false` leaves the CME model round without CM tools. |
| `MM_CME_ENABLED` | `true` | Route `POST /api/chat` through the conversation engine instead of the legacy console controller. Set `false` to roll back. |
| `MM_AUTOMATION_V1` | `true` | Start session automation, frame timers, and the plan monitor. |
| `MM_CME_PROACTIVE` | `true` | Attach the proactive plugin engine. |
| `MM_CME_MEMORY_COMPRESSION` | `true` | Compress completed turns into memory segments. |
| `MM_CME_PREFERENCE_LEARNING` | `true` | Learn preference signals from completed turns. |
| `MM_CME_AGENTSTREAM_LOGGING` | `true` | Write CM turns and frames to agentstream. |
| `MM_PROMPT_V1` | `false` | Declared prompt-tree flag. The reviewed tree under `config/prompts/` is loaded by the CME prompt path regardless of this flag. |
| `MM_FRAMES_V1` | `false` | Declared frame flag. Frame tools and timers are governed by `MM_AUTOMATION_V1` and `config/automation.toml`. |
| `MM_THREE_PASS_V2` | `true` | Run turns through the three-pass runner (Interpret → Process → Verify). Checks, staged writes, retries, and recursive think are implemented. Set `false` for the single-loop path. |
| `MM_PROCESS_V1` | `false` | Enable durable process templates/instances (Phase 5). Plan creation uses the four-pass builder, `metamind.plan run=true` starts a process, and `metamind.act` exposes process controls. Keep off until a live soak is recorded. See [`process-pipeline.md`](./process-pipeline.md). |
| `MM_LLM_AGENTSTREAM` | `true` | Live-call kill switch. `off` makes every live gateway call return one configuration error; no direct-client fallback exists. |

Rollback to the legacy console route is `MM_CME_ENABLED=false`. Rollback to the
single-loop CME turn (keeping the conversation engine and CM tools) is
`MM_THREE_PASS_V2=false`. Rollback only the CM tool surface is `MM_CM_V3=false`
(the CME still runs, with no CM tools attached).

### Three-pass turns

With `MM_THREE_PASS_V2=true` (the default), a turn is Interpret → Process → Verify. The checks
are turn-scoped; Verify stages durable writes and commits only on approval; a
rejected reply retries Process up to `verify.max_retries`; exhaustion fails with
`VERIFY_EXHAUSTED` and names the durable changes already made. Retries and
budgets come from `config/three_pass.toml`; `MM_VERIFY_MAX_RETRIES` may only
lower the retry count. Recursive think (`[think] three_pass = true`) runs
`metamind.think` through the same verified path under the read-only Think scope
and never commits a write. The full contract is in
[`three-pass-verify.md`](./three-pass-verify.md); the live dual-run gate is in
[`phase12-dual-run.md`](./phase12-dual-run.md).

---

## 2. The seventeen tools

Every tool has a strict JSON Schema in `crates/mm-serve/schemas/tools.json`.
The reviewed work catalog is seventeen tools. Phase 1 adds two pass-control
schemas (`metamind.verify`, `metamind.approve`) and Phase 2 consumes them in the
Interpret pass; they are only advertised inside a pass scope, so the legacy path
still sees exactly seventeen. Unknown fields, wrong types, and missing required fields are
rejected before any side effect. Every result is a full envelope carrying the post-call conversation
state; a refused tool returns a typed error (`INVALID_ARGUMENTS`,
`NOT_AVAILABLE`, `FORBIDDEN`, `CONFLICT`, `BUDGET`, …) rather than a fake success.

### Conversation (5)

| Tool | Use |
|---|---|
| `conversation.topic` | Read or set the short plain-language topic. Revision-checked. |
| `conversation.depth` | Read or set depth (`0` = top level). Revision-checked. |
| `conversation.status` | Read/set working variables: phase, intent, project, tasks, next, blockers, questions, notes, focus, frame progress, `awaiting_user`, `next_utterance_at`, custom keys. Cannot write topology keys owned by the frame stack. |
| `conversation.frame` | Read, push, replace, or pop the frame stack. Also schedules or cancels the next in-character utterance with a natural delay. |
| `agent.run` | One downstream agent turn: start a run (`run_id` omitted) or continue one. Returns what it did, what it needs, and whether it is done. |

### Managed data (7)

| Tool | Use |
|---|---|
| `data.create` | Create one typed ontology instance. The class chooses the schema and graph; the store mints an IRI when none is supplied. Validated, written to RDF, and indexed. |
| `data.update` | Update one instance (`merge` by default, `replace` on request) with revision checking. Replaces the search document. |
| `data.query` | Read entities by class, subject, property filters, namespace, and graph. Returns compact entities, never raw SPARQL. |
| `data.delete` | Soft-delete by default (retained for provenance, excluded from queries/search); hard delete requires explicit authority. |
| `data.search` | Full-text search over labels, literals, classes, and namespaces. Hits are verified against RDF before being returned. |
| `data.schema` | List/get schemas, run SHACL validation, register a schema, activate or deprecate one. |
| `data.namespace` | List bindings; bind, expand, or resolve compact IRIs. |

### Metamind (5)

| Tool | Use |
|---|---|
| `metamind.plan` | Create, version, read, or list plans in the plan graph. Markdown or structured plans are both accepted. |
| `metamind.think` | Private inner thought with read-only data access. Returns findings, evidence, and confidence; never speaks to the operator directly. |
| `metamind.memory` | Recall, remember, forget, or inspect memory health. Memory is not ontology data; use `data.*` for typed facts. |
| `metamind.log` | Overview, latest, paginated segment, or append across agentstream traces and RDF `mm:LogEntry` records. |
| `metamind.act` | Run or schedule a registered internal action; control a plan (start/stop/pause/resume/retry/advance/complete/abandon); cancel an action or cron job; invoke introspection. |

Admin-only operations (raw graph writes, shell, catalog management) are not on
this surface. They remain CLI-only.

### Inspect the advertised schemas

The console exposes the catalog the model sees through the CM tool contract; the
authoritative copy is the embedded schema file:

```bash
python3 - <<'PY'
import json
for t in json.load(open("crates/mm-serve/schemas/tools.json")):
    print(t["name"])
PY
```

---

## 3. Prompts and frame templates

Prompt prose lives only under `config/prompts/`; the executor contains no
system-message literals. The tree is validated at boot.

```
config/prompts/
├── manifest.toml                 # strategies and their fragment order
├── applications/                 # cm.turn.v1, cm.turn.pending/resume, repair,
│                                 # tutorial, frame.utterance, plan/think/memory/log
├── fragments/                    # identity, character, state, tools, policies,
│                                 # data/memory/plan/log/frame guidance, budget
└── frames/
    ├── manifest.toml             # intro, resume, work, tutorial, tutorial_offer, goodbye
    └── frame.*.toml
```

To change behavior:

1. Edit a fragment or strategy TOML.
2. Keep the declared variables consistent with `PromptVariables`; a missing or
   mistyped variable fails closed with `PROMPT_UNAVAILABLE` and makes no model call.
3. Restart `mm-serve` (the store validates and caches templates at boot).
4. Render one instance in tests with `cargo test -p mm-prompt`; the golden prompt
   snapshots are the fastest way to see the resulting `[STATE]` block.

Frame templates provide intro/connect/resume/work/tutorial/goodbye prose. The
frame stack is runtime data; templates only supply text and allowed steps. Push a
frame through `conversation.frame`, then keep frame progress in
`conversation.status` (`frame_step`, `frame_attempt`, dynamic values). Frame
utterances use the natural delay/jitter policy in `config/automation.toml`; user
input cancels the pending utterance, and repeated attempts back off.

---

## 4. Sessions, presence, expiry, and resume

The console session API is under `/api/console/sessions`:

| Method and path | Purpose |
|---|---|
| `GET /api/console/sessions` | Cursor-paginated session list. |
| `POST /api/console/sessions` | Create a session. |
| `GET /api/console/sessions/{id}` | One session bundle. |
| `DELETE /api/console/sessions/{id}` | Soft-delete; purge is explicit. |
| `POST /api/console/sessions/{id}/presence` | Heartbeat: `present`, `away`, `idle`. |
| `POST /api/console/sessions/{id}/resume` | Restore stack/work/traces for a resumable session. |
| `POST /api/console/sessions/{id}/end` | End with a reason and stop the supervisor. |
| `GET /api/console/sessions/{id}/state` | Conversation state and frame stack. |
| `GET /api/console/sessions/{id}/status` | Full status projection (rail/tabs). |
| `GET /api/console/sessions/{id}/status/stream` | Session-scoped SSE status stream. |
| `GET /api/console/sessions/{id}/operations` | Operations/trace overview. |
| `GET /api/console/sessions/{id}/traces` | Cursor-paginated traces; exactly one `main`. |
| `GET /api/console/sessions/{id}/logs` | Agentstream/RDF log page. |
| `GET /api/console/sessions/{id}/frames` | Frame stack plus scheduled utterance. |
| `GET /api/console/sessions/{id}/interjections` | Per-session interjection decisions. |

Presence is a heartbeat, not a keep-alive for idle expiry: a heartbeat updates
`present`/`away` but does not reset the idle clock. The defaults in
`config/automation.toml` are:

| Setting | Default |
|---|---|
| heartbeat | 30 s |
| present window | 2 min |
| away window | 15 min |
| idle expiry | 30 min |
| hard TTL | enforced by the session registry |
| work-stall | derived from `working` plus the last work touch |

Opening or resuming a session starts exactly one `SessionAutomation` supervisor.
Ending, idle expiry, and hard TTL stop it. A new session supersedes the previous
active session for that user. A session-scoped status stream only receives that
session's broadcasts; it never forwards another session's events.

---

## 5. Interjections

`config/interjections.toml` resolves policy with this precedence:

```
instance → user → kind → service → global
```

Each policy decides `when_working` (`queue`, `suppress`, `badge`, `deliver`),
`when_idle`, allowed presence, minimum gap, hourly cap, priority, and expiry. A
user preference may narrow delivery but cannot force a safety-suppressed item.
Defaults: internal actions are suppressed while `working`; agent results queue;
frame utterances suppress while working; plan-monitor chatter is disabled;
introspection is present-only. Quiet hours are 22:00–08:00.

Set `MM_INTERNAL_ACTIONS_DISABLED=kind1,kind2` to disable specific internal
action kinds without editing the registry.

---

## 6. Automatic state and data capture

The prompt contract (not a per-turn reminder) tells the model to:

- keep topic/depth/status/frame progress current when the conversation changes;
- search before creating managed data, update instead of duplicating, and
  supersede or correct rather than silently overwrite;
- keep secrets out of data and memory;
- use `metamind.plan` for durable multi-step work and `metamind.log` for explicit
  session records;
- treat `[STATE]` as the source of truth and rebuild it after any state/data/frame
  write.

`[STATE]` is rebuilt before every model call, including every tool round. It
contains the current user, session/work/traces, frame stack, pending work, budget,
memory continuity, data health, plans, actions, and recent transcript. A closed
state store renders `[STATE unavailable]`; the model is told to recover rather
than guess.

---

## 7. Memory

Use `metamind.memory` from the conversation, or the CLI for direct inspection:

```bash
mm-cli --data-dir "$DATA" memory stats
mm-cli --data-dir "$DATA" memory recall --query "deployment decision"
```

The CM memory path has four actions: `recall`, `remember`, `forget`, `status`.
Memory is separate from ontology data: a durable preference or episode belongs in
memory; a typed project/note/decision belongs in `data.*`. Compression of completed
turns is enabled by `MM_CME_MEMORY_COMPRESSION`; forgetting is a dry run unless
explicitly applied and never hard-deletes protected records.

---

## 8. Plans, think, log, act

- **Plan.** `metamind.plan` writes a versioned plan into the plan graph. A plan is
  a living hypothesis: version it when the approach changes. The plan monitor
  reads `mmd:plan-registry`, not an in-memory queue.
- **Think.** `metamind.think` is a private sub-agent turn with read-only data
  tools. The outer CM composes the operator-visible reply; the inner voice never
  speaks directly.
- **Log.** `metamind.log` reads agentstream traces and RDF `mm:LogEntry` rows
  through one cursor API. `append` is for explicit RDF log entries, not a way to
  rewrite history.
- **Act.** `metamind.act` runs an internal action kind through the generic
  registry, or schedules it (`immediate`, `schedule`, `cron`) through the durable
  scheduler. It also owns plan control and cancellation. There is no
  kind-specific scheduler branch: a new kind is data in the registry.

Plan control is revision-checked. Stopping a plan cancels the runs it owns. A
failed step retries within policy; external steps go through `agent.run` only.

---

## 9. Background automation, cron, and the plan monitor

`MM_AUTOMATION_V1=true` (the default) enables:

- one supervisor per live session, started idempotently on open/resume;
- a proactive tick that claims due scheduled actions;
- a five-minute built-in internal action (suppressed while `working`, using recent
  RDF log outputs to avoid repetition);
- frame utterance timers with jitter, cancellation on user input, and incremental
  backoff;
- cron jobs persisted in RDF, restored on boot, capped, and cancelable;
- the global plan monitor, which ticks every 15 seconds over the RDF plan registry.

The decisive settings are in `config/automation.toml`:

```toml
[supervisor] tick_secs = 30
[presence]   heartbeat_ms = 30000; idle_expiry_ms = 1800000
[frames]     max_depth = 8; default_delay_ms = 4500
[actions]    max_scheduled_per_session = 64
```

The plan monitor is suppressed for operator-visible interjections by default
(`[services.plan_monitor] enabled = false`), but it still advances plans.

---

## 10. User workspace, Hermes, and Pi

Downstream agent work is confined to `data/workspace/<user-id>/`. `agent.run`
accepts a `workspace` relative to that root; absolute paths, `..`, and symlink
escapes are refused with `FORBIDDEN`. The default is the user's root.

### Hermes

```toml
# config/agents.toml
[hermes]
endpoint = ""       # or set MM_AGENT_HERMES_BASE_URL
model = ""          # empty = first model from /models discovery
```

Relevant environment variables:

| Variable | Purpose |
|---|---|
| `MM_AGENT_HERMES_BASE_URL` | OpenAI-compatible endpoint. Unset → Hermes is unavailable with that reason. |
| `MM_AGENT_HERMES_API_KEY` | Bearer token for Hermes. |
| `MM_AGENT_HERMES_MODEL` | Model hint; empty uses discovery. |
| `MM_AGENT_HERMES_STREAM` | Enable streaming. |
| `MM_AGENT_HERMES_TIMEOUT_MS` | Per-turn timeout. |
| `MM_AGENT_HERMES_MAX_CONTEXT_TOKENS` | Context cap. |
| `MM_AGENT_ALLOWED_ROOTS` | Additional roots allowed by the workspace resolver. |

Hermes now maps its turns through the shared `AgentstreamLlm` gateway instead of owning an HTTP
client. Its profile is `hermes`, its calls are journaled under `agent/<run_id>`, and its streamed
answer still returns as one agent turn. A required journal append that fails before dispatch
prevents the provider call and reports the journal error; the gateway never falls back to a direct
client. `MM_LLM_AGENTSTREAM=off` disables live calls explicitly, including the Hermes model probe.

### Pi

| Variable | Purpose |
|---|---|
| `MM_AGENT_PI_BINARY` | Path to the Pi binary (or `pi` on `PATH`). |
| `MM_AGENT_PI_PROVIDER` | Provider name passed to Pi. Unset → offline replay/refusal. |
| `MM_AGENT_PI_MODEL` | Model name. |
| `MM_AGENT_PI_TIMEOUT_MS` | Per-turn timeout. |

Pi sessions are created under `<workspace>/.pi/<run-id>/`; the agent runtime
records a run in agentstream and returns one turn at a time. `agent.run` is the
only external-work door: background automation never spawns shell or HTTP itself.

---

## 11. Inspector

The inspector is a dedicated route, not a modal over chat:

```
/inspector                 # chooses the first session
/inspector/:sessionId
```

It reads only real endpoints — no fixture rows. The rail shows presence/working,
the current frame and step, topic and depth. Tabs cover:

- **Status** — conversation state, work catalog, pending/budget, automation,
  interjections, plan-monitor state.
- **Knowledge** — managed-data health, entity counts, schemas, search health.
- **Plan** — plans, versions, steps, attempts, and plan control.
- **Trace** — turn headers, role rows, tool cards, summary/raw, filters, follow;
  selectors for CM/agent/plan/think/memory/action/introspection/automation
  sessions.

Status updates arrive over `/status/stream` with revision guards; duplicate and
stale frames do not change the DOM, and a hidden tab pauses the stream then
resumes with a stale badge. The Files tree pages each directory lazily. Sessions
page rather than downloading the whole list. Every absent value renders `not set`
or an explicit `unavailable` reason; the UI never fabricates a number.

Keyboard: `⌘⇧I` inspector, `⌘⇧1`–`⌘⇧4` tabs, arrow keys move the session/file
selection, Enter activates, Escape closes the drawer/sheet.

---

## 12. Index verification and repair

RDF is authority; Tantivy is a derived read model. Every managed write goes
through a SQLite outbox before the index is touched, so a failed index never rolls
back a write.

### Verify

Stop `mm-serve` first (it holds `data/console.lock`, and the graph store is
single-writer), then:

```bash
mm-cli --data-dir "$DATA" data verify
```

The command applies pending outbox rows, compares live RDF entities (excluding
soft-deleted ones) with indexed subjects, and prints both disagreement classes:

```
data index: 42 RDF entities, 42 indexed documents, 0 pending row(s) applied
  missing   0
  orphaned  0
index clean
```

`--json` is the machine-readable form:

```json
{"clean":true,"documents":42,"entities":42,"missing":[],"orphaned":[],
 "pendingApplied":0}
```

Exit code `0` means clean; non-zero means at least one missing or orphan document.

### Repair

```bash
mm-cli --data-dir "$DATA" data reindex
mm-cli --data-dir "$DATA" data verify
```

`reindex` rebuilds the whole index from the RDF entity set. It skips soft-deleted
entities and deduplicates entities that appear in more than one managed graph, so
it always produces at most one document per live entity.

### When to run it

- after an unclean shutdown or a schema change in `mm-search`;
- after restoring a snapshot/backup;
- if `data.search` returns a stale hit (the service verifies hits against RDF and
  should hide them, but `verify` finds the document that needs repair);
- as the last step of a migration or an upgrade.

If `verify` still reports missing documents after `reindex`, the index directory
could not be opened or written. Check the `mm-serve`/CLI warning for the search
directory (`<data>/search/`), permissions, and free space; `verify` will report
`SearchUnavailable` rather than pretending the index is clean.

### Startup reconciliation

`DataService::open` performs a startup reconciliation and rebuilds the index when
the outbox and index disagree. `data verify` is the explicit operator check; the
startup path is the automatic one. A clean run of `data verify` is the Phase 12
acceptance check.
