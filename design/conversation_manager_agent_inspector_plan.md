# Conversation Manager v2 — Reliable Tools, Agent Delegation, and the Live Inspector

> **Status:** proposed design · **Date:** 2026-11-22 · **Scope:** `mm-cme`, `mm-serve`, `mm-data`,
> `mm-pi`, a new `mm-agent` crate, the console frontend, and the metamind ontology.
>
> **This document is a plan, not an implementation.** It defines the target contracts and the
> build order. Nothing here changes production behavior until the phases in §12 are promoted
> through the existing change-set → sandbox → gate → promotion path.

---

## 0. Reading this document

| If you want to know… | Read |
|---|---|
| Why the conversation loop feels broken today | §2 |
| What the conversation manager should become | §3–§5 |
| Exactly which tools the conversation LLM gets | §6 |
| How Hermes and Pi are called | §7 |
| How the loop keeps working until the task is done | §8 |
| How the right-panel status is computed and streamed | §9 |
| What the status panel and agent trace viewer look like | §10 |
| The contracts on the wire | §11 |
| What to build, in what order | §12 |
| How it is verified | §13 |
| What could go wrong | §16 |

Terms used throughout:

- **CM** — the conversation manager, i.e. the LLM plus the `mm-cme` session/turn machinery that
  drives it. The CM is the *only* model the operator talks to.
- **Downstream agent** — Hermes or Pi. It is a separate, long-running worker with its own
  conversation. The CM delegates external work to it.
- **Internal tool** — a deterministic metamind operation that reads or writes metamind state.
  It never performs external side effects.
- **Agent tool** — the single `agent` tool. It is the only tool in the CM's surface that reaches
  outside metamind.
- **Inspector** — the auxiliary right-hand console rail that shows conversation status and agent
  traces. It is instrumentation, not a third workspace document pane.

---

## 1. Executive summary

The current conversation engine is a sound state machine wrapped in a weak conversation surface.
The failure modes the operator sees are not one bug; they are five separate contracts that were
never finished:

1. **The tool surface is a CLI mirror, not a conversation surface.** `ConsoleTools` advertises
   every row of `controller::catalog::table()` — currently 69 tools — and expects the model to
   discover the right one from names like `sandbox_tool.run`, `focus.patch`, and `design.show`.
   The catalog's group heuristic is keyword guessing, malformed JSON is silently coerced to `{}`,
   results are truncated mid-JSON at 16 KiB, and the executor permits only four tool rounds and
   eight steps. A model cannot reliably operate that surface, so it appears to "do nothing".
2. **The CM cannot manage state it does not represent.** There is no conversation topic, no topic
   hierarchy, no project binding, and no task model reachable from the loop. `SessionContext`
   carries `project_id` and `previous_topics`, but neither is injected into the prompt.
3. **The CM has no general-purpose worker.** `pi.run` exists as an admin row but there is no
   first-class agent session, no turn-by-turn delegation, and no second agentstream for the
   worker conversation. External work has no seam.
4. **The loop is impersonal because its prompt is a constant.** `TurnExecutor::messages()` emits
   the same three sentences on every turn. Being personality, the operator model, relationship
   state, affect, memory continuity, project context, and prior commitments never reach the
   model.
5. **The console cannot see the system it is driving.** The only live surface is a state board
   polled on the health cadence; there is no conversation-state stream, no operation ledger in
   the panel, and no viewer for agentstream traces.

The redesign turns the CM into an **explicit state manager with eight tools and one worker door**:

```
operator
   │
   ▼
┌──────────────────────────── CM (mm-cme) ────────────────────────────┐
│ prompt packet: identity · relationship · affect · topic tree         │
│               · project/tasks · recent turns · available tools       │
│                                                                      │
│ 8 internal tools ──► metamind state (RDF + event log + SQLite)       │
│ 1 agent tool    ──► mm-agent ──► Hermes (OpenAI API) or Pi (RPC)     │
│                        │                                             │
│                        └──► separate agentstream session per run     │
└──────────────────────────────────────────────────────────────────────┘
   │                                         ▲
   │ status events                           │ live view
   ▼                                         │
StatusService (mm-serve) ──SSE──► Inspector (status tab + trace tab)
```

The rest of this document specifies the state model, the exact tool schemas, the agent turn
contract, the live view model, the UI anatomy, and a phased build with testable exit criteria.

---

## 2. Diagnosis: what is actually wrong

### 2.1 Tool use fails

Evidence from the current code:

| Location | Behavior | Consequence |
|---|---|---|
| `crates/mm-serve/src/cme_tools.rs` | `schemas()` returns **every** `catalog::table()` row | 69 schemas on every turn; the model spends context on irrelevant tools and picks near-duplicates |
| `crates/mm-serve/src/controller/catalog.rs` | Groups are admitted by keyword hits (`groups_for`) | `catalog.open` becomes a meta-tool the model must call to reveal more tools; one wrong guess loses the turn |
| `crates/mm-serve/src/cme_tools.rs` | `parse_arguments` does `unwrap_or_else(|_| json!({}))` | Invalid JSON becomes an empty object; the tool runs with missing arguments or fails with a misleading validation error |
| `crates/mm-serve/src/cme_tools.rs` | Results are truncated to 16 KiB by byte boundary, then `...[truncated]` | The model receives invalid JSON and cannot tell a real result from a cut one |
| `crates/mm-cme/src/executor.rs` | `MAX_TOOL_ROUNDS = 4`, `MAX_TOOL_STEPS = 8` | Any multi-step job stops with "tool round cap", which the model reports as a failure |
| `crates/mm-cme/src/executor.rs` | Tool errors are plain strings: ``tool `x` failed: …`` | No error code, no retryability, no structured remediation |
| `crates/mm-serve/src/controller/admit.rs` | Admission is group-based, not need-based | The CM has to reason about *metamind's internal organization* instead of the operator's task |
| `crates/mm-serve/src/controller/exec.rs` | One dispatch function for CLI-shaped operations | CLI semantics leak into conversation semantics (`id` required, fixture rules, `apply` flags) |

The root cause is not the number of tools by itself. It is that **the tool surface encodes the
CLI's structure rather than the conversation's jobs**. A CM needs tools for "what are we working
on", "what should happen next", "remember this", "delegate this", and "show me that" — not for
every kernel subcommand.

### 2.2 The loop is impersonal

`TurnExecutor::messages()` currently produces:

```rust
let system = format!(
    "You are the Metamind assistant. Answer in the {} shape at detail {}/5. \
     Be direct and never invent sources. Use the provided tools when the \
     operator asks for kernel state, memory, goals or any operation rather \
     than answering from memory.",
    plan.modality.as_str(),
    plan.detail_level
);
```

Nothing else is supplied. The following data already exists but never reaches the model:

- `BeingFacade`: self-description, values, personality-derived voice, goals, invariants.
- The user model: preferred name, expertise, communication preferences, recurring priorities.
- Relationship state: familiarity, trust, open issues, shared history.
- `SessionContext::memory`: `project_summary`, `previous_topics`, `greeting`.
- `SessionContext::affect`.
- Learned preference profile.
- Any notion of "what we were talking about" inside the current session.

The result is a competent but anonymous assistant. It cannot open with continuity, cannot use the
operator's name, cannot connect a request to yesterday's commitment, and cannot adjust tone to the
relationship.

### 2.3 Project management is absent

`mm-data` has a project registry (`ProjectRegistry`, `ProjectRecord`) and a tier-3
`ProjectMemory` with commitments, open questions, goals, artifacts, and a suggested
continuation. None of that is exposed as a tool, none of it is rendered in the prompt, and the
console has no task surface. The CM therefore cannot:

- create or bind a project,
- add a task or milestone,
- mark work blocked or done,
- show what is upcoming,
- connect an agent run to the task it serves.

### 2.4 External work has no seam

`pi.run` is an admin catalog row that shells out to Pi with `Confirm::Always`. There is no:

- stable `agent` interface for the CM,
- separate worker conversation,
- turn-by-turn progress,
- per-turn summarization,
- separate agentstream for the worker,
- way to continue a long job across multiple CM tool calls.

### 2.5 The console cannot observe the conversation

- `StateBoard` reads `/api/state` and refetches when the 5-second health poll moves.
- `/api/cme/sessions/{id}/snapshot` exists but no UI consumes it.
- There is no topic, task, operation, or agent data in the board.
- `AgentstreamStore` records raw events, but no route and no viewer expose them.
- There is no stable live-update protocol: the panel flashes through loading states, and there is
  no way to know whether a displayed value is current.

---

## 3. Design principles

1. **The CM is a manager, not an operator console.** It sees jobs and state, not kernel
   subcommands. Administrative breadth stays available through `mm-cli` and the controller HTTP
   API; it is not advertised to the conversation model.
2. **One external door.** All outside work goes through the `agent` tool. The CM cannot call a
   shell, a browser, or an arbitrary HTTP endpoint directly. This makes authorization, logging,
   cancellation, and replay tractable.
3. **Explicit state, never inferred state.** The topic is a first-class object with a label, an
   optional backend reference, and a parent. The CM writes it, the UI reads it, and the event log
   records it.
4. **Small, typed, idempotent tools.** Each internal tool has one job, strict argument
   validation, a typed result envelope, and an idempotency key for writes.
5. **A turn is not done until the job is done or the budget is spent.** The loop keeps calling
   tools — including repeated `agent` turns — until the CM declares completion or a hard deadline
   arrives. A budget stop is reported as *paused with remaining work*, never as a success.
6. **The backend derives the view once; the UI renders revisions.** The status service owns
   composition, coalescing, and ordering. The browser never polls five endpoints and stitches the
   result together.
7. **Every mutation is an event.** Context changes, task changes, agent turns, summaries, and tool
   operations all append to the event log. State is reconstructible; the inspector can replay.
8. **Fail visibly, with a reason and a next step.** No silent `{}`. No "tool failed" without a
   code. No fake success.

### 3.1 Alternatives considered

| Decision | Alternative | Why the alternative was rejected |
|---|---|---|
| Eight job-shaped tools | Keep 69 tools and add progressive disclosure | Disclosure still asks the model to reason about metamind's internal organization; the near-duplicate names remain, and every extra schema costs context on every turn |
| New `mm-agent` crate | Put Hermes/Pi clients in `mm-cme` or `mm-serve` | `mm-cme` must stay UI- and service-agnostic; `mm-serve` would make the backends untestable without the console. A crate gives a stable trait and a fake-server test surface |
| Turn-by-turn agent calls | One long `agent.run` call that returns at the end | The operator gets no progress, the CM cannot react to a blocker, a 30-minute job exceeds every HTTP timeout, and a crash loses the whole call |
| Revisioned SSE status | Poll `/api/state` and `/api/cme/snapshot` every second | Polling recomposes the whole view, causes loading flashes, has no ordering guarantee, and multiplies store reads per open tab |
| Inspector right rail | A third workspace document pane | The two-pane workspace is a deliberate design contract (UI plan `03`); a third pane would steal width from chat/document and complicate swap/fullscreen. Instrumentation belongs in a collapsible rail |
| Topic tree | One mutable `topic: String` | A flat string cannot represent drill-down, cannot close a sub-thread without losing the parent, and cannot answer "where are we?" after a context switch |
| Separate agentstream per run | Reuse the CM session | Raw agent turns would pollute the conversation transcript and token budget, and the trace viewer could not isolate a run or rebuild its message history |
| Explicit `context` writes | Infer the topic from the query on every turn | Inference is invisible and un-correctable; explicit writes give the operator a visible, editable state and the event log a real record |

---

## 4. Target architecture

### 4.1 Component map

```
crates/
  mm-cme/                 session/turn/chunk state machines, context state, prompt packet,
                          the tool-loop policy (ContinueUntilDone)
  mm-agent/               NEW: AgentBackend trait, Hermes, Pi, run registry, turn summarizer,
                          agentstream bridge
  mm-data/                RDF stores: conversation context, work items, agentstream
  mm-pi/                  existing Pi RPC client; mm-agent adapts it
  mm-llm/                 existing OpenAI-compatible substrate; used for summarization
  mm-serve/               CM tool implementations, status aggregator, routes, app state
frontend/
  src/components/inspector/   NEW: status panel, trace viewer, shared rail
  src/lib/api/inspector.ts    NEW: status/trace clients and reducers
```

Dependency direction:

```
mm-agent ──► mm-pi
   │  └────► mm-llm
   │  └────► mm-data
   ▼
mm-cme ──► mm-agent (only through a narrow AgentService trait)
   ▼
mm-serve ──► mm-cme, mm-agent, mm-data, mm-llm
```

`mm-cme` must not depend on `mm-serve`. The CM tool implementations live in `mm-serve`
(`cm_tools.rs`) because they need `AppState`; the *state types* and *invariants* live in `mm-cme`
and `mm-data`.

Shared reference types (`TopicRef`, `RefKind`) live in `mm-data::conversation` and are re-exported
by `mm-cme`. This is deliberate: `mm-agent` needs to return artifact references in
`AgentTurnResult`, and it must not depend on `mm-cme` (which depends on `mm-agent`). Putting the
ref type in the data layer keeps the dependency graph acyclic.

### 4.2 The three stores

| Store | Contents | Authority |
|---|---|---|
| **Agentstream** (`mm-data`) | Raw events per session, including the CM conversation and each downstream agent run | Append-only history; replay and trace viewer |
| **Conversation graph** (`mm-data`, new) | Latest materialized `ConversationContext` and topic tree per session; latest work items per project | Current state; event-derived, revision-guarded |
| **Project memory** (`mm-data`, existing) | Compressed segments, commitments, open questions, goals, artifacts, narrative | Long-term continuity; written by the memory bridge and aggregation pipeline |

The conversation graph is a **materialized projection**, not a competing source of truth. Every
write also appends a `ContextUpdated` event to agentstream; a replay can rebuild the materialized
graph from the event stream. Optimistic concurrency uses a monotonic `revision`.

### 4.3 The two event streams

| Stream | Producer | Consumer |
|---|---|---|
| **CM stream** (`session_id` = console session) | `mm-cme` turns, tool calls, context/task mutations | Chat transcript, status panel, agentstream viewer |
| **Agent streams** (`session_id` = `agent/<run_id>`) | `mm-agent` for each downstream turn, tool call, summary | Trace viewer, agent run status, audit |

The agent stream is deliberately separate. A Hermes run may contain dozens of turns and large tool
outputs; mixing them into the CM transcript would pollute the conversation and inflate the prompt.
The CM only ever receives the compact `AgentTurnResult` envelope.

---

## 5. Conversation state

### 5.1 The topic tree

Topics are explicit, hierarchical, and optionally anchored to backend data.

```rust
/// One node in the conversation's topic tree.
pub struct Topic {
    pub id: TopicId,                    // ULID
    pub label: String,                  // "Fix failing auth tests"
    pub parent: Option<TopicId>,        // None only for the main topic
    pub status: TopicStatus,            // active | parked | done
    pub summary: Option<String>,        // one or two sentences, model-written
    pub refs: Vec<TopicRef>,            // zero or more backend anchors (type in mm-data)
    pub created_at: i64,
    pub updated_at: i64,
    pub created_turn: Option<TurnId>,
}

pub enum TopicStatus { Active, Parked, Done }

/// A topic can point at real backend data. The ref is validated before it is stored;
/// an unvalidated ref is kept but marked `unverified` and shown in the panel with a warning.
pub struct TopicRef {
    pub kind: RefKind,
    pub id: String,
    pub label: Option<String>,
    pub uri: Option<String>,
    pub verified: bool,
}

pub enum RefKind {
    File,          // workspace-relative path, checked against the codex/git tree
    FocusDoc,      // a console focus document id
    GraphNode,     // an IRI in the RDF store
    Memory,        // memory id
    Project,       // project id
    Task,          // work item id
    AgentRun,      // agent run id
    Url,           // external URL (never fetched by the CM)
    Artifact,      // change set, module, report, dashboard
}
```

The per-session context:

```rust
pub struct ConversationContext {
    pub session_id: String,
    pub revision: u64,                  // monotonic, optimistic concurrency
    pub main_topic: Option<TopicId>,
    pub current_topic: Option<TopicId>, // the drill-down cursor; equal to main at depth 0
    pub topics: BTreeMap<TopicId, Topic>,
    pub intent: Option<String>,         // what the operator is trying to accomplish
    pub open_questions: Vec<OpenQuestion>,
    pub next_actions: Vec<NextAction>,  // small, concrete; shown in the panel
    pub pinned_refs: Vec<TopicRef>,     // context-level anchors beyond the topic
    pub notes: Vec<ContextNote>,        // short turn-scoped notes, capped at 20
    pub project_id: Option<String>,     // bound project, if any
    pub updated_at: i64,
    pub updated_by: ContextOrigin,      // user | cm | system
}
```

**Invariants**

1. There is at most one root topic (`main_topic`). Every other topic has exactly one parent and
   the parent chain reaches the root with no cycles. `current_topic` is always on that chain.
2. `current_topic` starts at the root. Drilling creates a child and moves the cursor; returning
   moves to the parent; closing marks the subtree `Done` and moves the cursor to the nearest
   non-done ancestor.
3. A topic label is non-empty and at most 120 characters. A summary is at most 500 characters.
4. A `TopicRef` of kind `File` must be workspace-relative, normalized, and exist at write time.
   A `FocusDoc`/`Memory`/`Project`/`Task`/`AgentRun` ref must resolve through its owning store. If
   resolution is unavailable (store closed), the ref is stored `verified: false` and the tool
   result reports a warning rather than failing the turn.
5. Every mutation carries `expected_revision`. A mismatch returns `CONFLICT` with the current
   revision and the current topic path, so the CM can re-read and retry once.
6. No mutation silently deletes history: closing a topic changes status; it does not remove the
   node. The event log retains every revision.

**Why a tree and not a single string?** The operator's work is naturally nested: "release 2.0" →
"auth migration" → "fix the token refresh test". A flat label loses the parent context that makes
the status panel useful. The tree also gives the CM a cheap way to decide whether a new request is
a continuation (same ancestors) or a context switch (new root).

**Auto-seeding.** If a session has no topic after the first user message, the server seeds a root
topic from the first 80 characters of the query, marked `updated_by: system` and shown with an
`auto` badge. The CM is instructed to rename or replace it on its first tool call. The system
never guesses a hierarchy it was not told; it only ensures the panel is never empty.

### 5.2 Projects, tasks, and how they differ from goals and commitments

Four related concepts must not be conflated:

| Concept | Meaning | Owner | Example |
|---|---|---|---|
| **Project** | A bounded body of work with a home, members, and memory | `ProjectRegistry` (`mm-data`) | "Metamind console v2" |
| **Task / milestone** | A concrete unit of work with status, priority, due date, blockers | New `WorkStore` (`mm-data`) | "Wire the inspector status SSE" |
| **Being goal** | An enduring direction the being pursues across projects | `mm-being` | "Make the console trustworthy under load" |
| **Commitment** | A promise made to a person, with a deadline | Project memory (`mm-data`) | "I will have the trace viewer ready by Friday" |

The CM tool surface exposes all four, but with different tools: `task` for work items,
`project` for project lifecycle/binding, `being` for goals, and `memory` for commitments learned
from conversation. This separation prevents the common failure where every TODO becomes an
identity-level goal.

```rust
pub struct WorkItem {
    pub id: TaskId,                    // ULID
    pub revision: u64,                 // optimistic concurrency
    pub project_id: Option<String>,
    pub title: String,                 // required, <= 200 chars
    pub detail: Option<String>,        // <= 4 KiB
    pub kind: WorkKind,                // task | milestone
    pub status: WorkStatus,            // todo | doing | blocked | done | cancelled
    pub priority: u8,                  // 0..100, higher is sooner
    pub due_at: Option<i64>,           // Unix ms
    pub parent: Option<TaskId>,        // subtask
    pub depends_on: Vec<TaskId>,
    pub blockers: Vec<String>,         // human-readable reasons
    pub topic_id: Option<TopicId>,     // the conversation topic this serves
    pub refs: Vec<TopicRef>,
    pub agent_run_id: Option<String>,  // set when delegated
    pub origin: WorkOrigin,            // user | cm | agent | memory
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

pub enum WorkKind { Task, Milestone }
pub enum WorkStatus { Todo, Doing, Blocked, Done, Cancelled }
```

Rules:

- A task may exist without a project (quick personal work). The panel groups those under
  "Unprojected".
- `done` and `cancelled` are terminal; reopening is an explicit `update` with a reason.
- A task that depends on an incomplete task cannot move to `doing`; the tool returns
  `CONFLICT` with the blocking ids.
- An agent run linked to a task sets it to `doing` on start and `done` only after the agent
  reports completion **and** the CM confirms the acceptance criterion. The agent cannot mark
  metamind tasks itself; it has no access to metamind tools.
- Deleting a task is not exposed. Cancel it, preserving the record.

### 5.3 Persistence and events

New module: `crates/mm-data/src/conversation.rs`.

```rust
pub struct ConversationStore { engine: Arc<RdfEngine> }

impl ConversationStore {
    pub async fn load(&self, session_id: &str) -> Result<Option<ConversationContext>>;
    pub async fn save(&self, ctx: &ConversationContext, expected_revision: u64)
        -> Result<ConversationContext>;          // CONFLICT on stale revision
    pub async fn append_event(&self, event: ContextEvent) -> Result<()>;
}
```

New module: `crates/mm-data/src/work.rs`.

```rust
pub struct WorkStore { engine: Arc<RdfEngine> }

/// A partial update; `None` means "leave unchanged". Status is a separate enum so a
/// typo cannot silently erase a task.
pub struct WorkPatch {
    pub title: Option<String>,
    pub detail: Option<String>,
    pub status: Option<WorkStatus>,
    pub priority: Option<u8>,
    pub due_at: Option<Option<i64>>,
    pub blockers: Option<Vec<String>>,
    pub depends_on: Option<Vec<TaskId>>,
    pub agent_run_id: Option<String>,
    pub reason: Option<String>,
}

impl WorkStore {
    pub async fn create(&self, item: WorkItem) -> Result<WorkItem>;
    pub async fn update(&self, id: &TaskId, patch: WorkPatch, expected_revision: u64)
        -> Result<WorkItem>;
    pub async fn list(&self, filter: WorkFilter) -> Result<Vec<WorkItem>>;
    pub async fn upcoming(&self, project_id: Option<&str>, limit: usize) -> Result<Vec<WorkItem>>;
}
```

Graphs (new helpers in `mm-data/src/rdf.rs`):

| Graph | Contents |
|---|---|
| `https://metamind.dev/graph/conversation/{session_id}` | `ConversationContext`, topics |
| `https://metamind.dev/graph/work/{project_id}` | work items for one project |
| `https://metamind.dev/graph/work-index` | all work items, for cross-project queries |

Event additions to `mm-data/src/agentstream.rs`:

```rust
pub enum EventType {
    // existing variants …
    ContextUpdated,
    WorkUpdated,
    AgentTurn,
    AgentTurnSummary,
    AgentStatus,
}
```

`as_str()` returns `context_updated`, `work_updated`, `agent_turn`, `agent_turn_summary`,
`agent_status`. These are additive; existing serialized events remain valid.

Event payloads carry `schema`, `session_id`, `revision`, and a compact diff. Never store the full
context in every event:

```json
{
  "schema": "mm.context.v1",
  "sessionId": "01…",
  "revision": 12,
  "op": "topic_drill",
  "topic": { "id": "01…", "label": "token refresh", "parent": "01…" },
  "path": ["release 2.0", "auth migration", "token refresh"],
  "origin": "cm"
}
```

### 5.4 The prompt packet

The prompt is assembled from stable and volatile sections, in this order. The packet is a typed
struct, not string concatenation scattered through the executor.

```rust
pub struct PromptPacket {
    pub identity: IdentityBlock,        // being self-description, values, invariants
    pub relationship: RelationshipBlock,// familiarity, trust, open issues, preferred name
    pub affect: AffectBlock,            // valence, arousal, confidence → tone guidance
    pub continuity: ContinuityBlock,    // project summary, previous topics, greeting
    pub context: ContextBlock,          // topic tree, current path, intent, open questions,
                                        // next actions, pinned refs
    pub work: WorkBlock,                // project, in-progress/upcoming/blocked tasks
    pub operations: OperationsBlock,    // last N operations and active agent runs
    pub turn: TurnBlock,                // query, modality, detail level, budget, tool policy
    pub tools: ToolBlock,               // the 8 schemas
}
```

Rendered form (abridged; section order is fixed so prompt caching can reuse the stable prefix):

```
[IDENTITY]
You are Metamind. <self-description>
Values: <...>. Invariants: never fabricate autobiographical memory; never silently convert an
assumption to an observation; never claim external action without execution evidence.
Voice: warmth 0.72, directness 0.64, curiosity 0.81, playfulness 0.35, formality 0.22.

[RELATIONSHIP]
You are speaking with <preferred name> (<user id>). Familiarity 0.68, trust 0.74.
They are expert in Rust and systems; they prefer direct answers and concrete file references.
Open issue: <...>. Do not re-ask what is already known.

[AFFECT]
Current tone: engaged, moderately confident. Keep it steady; do not perform enthusiasm.

[CONTINUITY]
Project: <name> — <narrative one-liner>.
Previous topics: <...>. Suggested continuation: <...>.
Last commitment: "I will ... by Friday" (open, due ...).

[CONVERSATION CONTEXT — revision 12]
Main topic: release 2.0
Path: release 2.0 › auth migration › token refresh
Current topic ref: src/auth/refresh.rs (file, verified)
Intent: make the refresh test deterministic
Open questions: Do we support clock skew? (asked by operator)
Next actions: reproduce the flake; run the auth test 20×
Pinned: change set 01… (artifact)

[PROJECT WORK]
Project: Metamind console v2 (active, 62% of 8 tasks done)
Doing: Wire inspector SSE (priority 80, due 2026-11-24)
Upcoming: Trace viewer event grouping (70), ...
Blocked: Promote agent crate (waiting on review) — reason: ...

[RECENT OPERATIONS]
12:04:11 context.topic drill → token refresh (ok, 3ms)
12:04:18 agent.start hermes "Reproduce failing test" → run 01… (running, turn 2)
...

[TURN]
Operator: "..."
Answer shape: explain, detail 3/5. Tool policy: continue-until-done (24 rounds, 10 min).
You must call `context` whenever the topic changes. You may call `agent` repeatedly until the
run is done. Do not answer as if finished while an agent run is `running`.
```

The stable prefix (`IDENTITY`, `RELATIONSHIP`, values, invariants) is cacheable. Volatile
sections follow. A missing source is rendered as an explicit `unavailable: <reason>` line, never
omitted silently — the same honesty rule the kernel already follows.

---

## 6. The CM tool surface

### 6.1 Final catalog

Eight tools. Six internal, one presentation, one external.

| # | Tool | Kind | What it is for | Mutates |
|---|---|---|---|---|
| 1 | `context` | internal | Set/read the topic tree, intent, open questions, next actions, notes | conversation graph |
| 2 | `project` | internal | Create a project, bind the session, update project metadata | project registry |
| 3 | `task` | internal | Create/update/close work items and milestones; link an agent run | work graph |
| 4 | `memory` | internal | Remember a durable fact/commitment; recall relevant memories | memory / project memory |
| 5 | `being` | internal | Read being state; add/update goals; read voice projection | being graph |
| 6 | `kernel` | internal | Read health, query the graph, validate a graph, inspect recent events | none |
| 7 | `focus` | presentation | Open a document or state board in the left pane | UI focus only |
| 8 | `agent` | external | Start/continue/status/cancel/close a Hermes or Pi run | external + agent runs |

Deliberately excluded from the CM surface:

- `promote`, `sandbox.run`, `changeset.new`, `loop.run`, `module.load` — these are gate-bearing
  engineering operations. The operator invokes them through the console/CLI, or explicitly asks
  the CM to *delegate* them to an agent in a sandbox. The CM cannot self-promote.
- `graph.query` as a free-form escape hatch — folded into `kernel` with a read-only query action
  and a result cap.
- `guide.*`, `catalog.open`, `state.show`, `session.note`, `focus.patch`, `design.show` — replaced
  by the eight above.
- `pi.run`, `pi.ingest` — replaced by `agent`.
- `sandbox_tool.*`, `action.*`, `audit.*` — not conversational jobs; available to the operator and
  to a delegated agent's own workspace tools, not to the CM.

The existing controller catalog remains for the CLI, direct HTTP administration, and parity tests.
`ConsoleTools` is replaced; it is not extended. The `parity` test that checks every CLI command is
mentioned by a tool summary must be scoped to the **admin catalog**, not the CM surface.

### 6.2 Tool definitions

Each tool is one schema with a small, explicit action set. The handler validates action-specific
fields; JSON Schema alone cannot express every conditional, so the result of a bad combination is
a typed `INVALID_ARGUMENTS` error naming the offending field.

#### 6.2.1 `context`

```json
{
  "name": "context",
  "description": "Read or update the conversation's explicit state: topic hierarchy, intent, open questions, next actions, pinned references, notes.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": {
        "type": "string",
        "enum": ["read", "set_main", "drill", "back", "close_topic", "rename", "attach_ref",
                 "detach_ref", "set_intent", "add_question", "resolve_question",
                 "set_next_actions", "note"],
        "description": "The state operation."
      },
      "expected_revision": { "type": "integer", "minimum": 0 },
      "label": { "type": "string", "minLength": 1, "maxLength": 120 },
      "summary": { "type": "string", "maxLength": 500 },
      "topic_id": { "type": "string" },
      "ref": {
        "type": "object",
        "additionalProperties": false,
        "properties": {
          "kind": { "type": "string", "enum": ["file","focus_doc","graph_node","memory","project","task","agent_run","url","artifact"] },
          "id": { "type": "string", "minLength": 1 },
          "label": { "type": "string" },
          "uri": { "type": "string" }
        },
        "required": ["kind", "id"]
      },
      "intent": { "type": "string", "maxLength": 500 },
      "question": { "type": "string", "maxLength": 500 },
      "question_id": { "type": "string" },
      "answer": { "type": "string", "maxLength": 1000 },
      "actions": {
        "type": "array",
        "maxItems": 8,
        "items": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "text": { "type": "string", "minLength": 1, "maxLength": 200 },
            "topic_id": { "type": "string" },
            "task_id": { "type": "string" }
          },
          "required": ["text"]
        }
      },
      "text": { "type": "string", "maxLength": 1000 }
    },
    "required": ["action"]
  }
}
```

Behavior:

| Action | Required | Effect |
|---|---|---|
| `read` | — | Return the whole context, path, and revision |
| `set_main` | `label` | Create or replace the root; the old root becomes a sibling? No — reject unless empty or `close_topic` was used; this is an explicit context switch |
| `drill` | `label` (+ optional `summary`, `ref`) | Create a child of `current_topic`, move the cursor |
| `back` | — | Move the cursor to the parent |
| `close_topic` | optional `topic_id` | Mark the current topic (or named one) and its descendants `Done`; move the cursor up |
| `rename` | `label` (+ `topic_id`) | Rename one node |
| `attach_ref` / `detach_ref` | `ref` + optional `topic_id` | Attach a validated reference to the topic |
| `set_intent` | `intent` | Replace the session's intent statement |
| `add_question` / `resolve_question` | `question` / `question_id` (+ `answer`) | Manage open questions |
| `set_next_actions` | `actions` | Replace the next-action list (cap 8) |
| `note` | `text` | Append a turn-scoped note (cap 20, oldest dropped) |

A successful write returns the new revision, the full path, and a compact topic tree so the model
does not need a second `read`.

#### 6.2.2 `project`

```json
{
  "name": "project",
  "description": "Create a project, bind this conversation to a project, or update project metadata.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["create", "bind", "update", "read"] },
      "project_id": { "type": "string" },
      "name": { "type": "string", "maxLength": 200 },
      "description": { "type": "string", "maxLength": 2000 },
      "status": { "type": "string", "enum": ["active", "paused", "done"] }
    },
    "required": ["action"]
  }
}
```

- `bind` writes `ConversationContext.project_id` and returns the project summary plus open tasks.
- `create` also binds the new project in the same operation.
- Project status is a small addition to `ProjectRecord` (`status`, `updated_at`) stored in the
  registry graph. It must not fork the registry into a second project model.

#### 6.2.3 `task`

```json
{
  "name": "task",
  "description": "Create, update, block, complete, or cancel a project task or milestone.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["create", "update", "close", "link_agent"] },
      "task_id": { "type": "string" },
      "expected_revision": { "type": "integer", "minimum": 0 },
      "project_id": { "type": "string" },
      "title": { "type": "string", "minLength": 1, "maxLength": 200 },
      "detail": { "type": "string", "maxLength": 4000 },
      "kind": { "type": "string", "enum": ["task", "milestone"] },
      "status": { "type": "string", "enum": ["todo", "doing", "blocked", "done", "cancelled"] },
      "priority": { "type": "integer", "minimum": 0, "maximum": 100 },
      "due_at": { "type": "integer", "description": "Unix ms" },
      "parent": { "type": "string" },
      "depends_on": { "type": "array", "maxItems": 20, "items": { "type": "string" } },
      "blockers": { "type": "array", "maxItems": 20, "items": { "type": "string", "maxLength": 300 } },
      "topic_id": { "type": "string" },
      "agent_run_id": { "type": "string" },
      "reason": { "type": "string", "maxLength": 500 }
    },
    "required": ["action"]
  }
}
```

`update` accepts any subset of mutable fields and requires at least one; `expected_revision` is
required for `update` and `close` and returns `CONFLICT` with the current row when stale. `close`
is sugar for `status: done | cancelled` plus a reason. `link_agent` is idempotent: the same
`(task_id, agent_run_id)` pair returns the current link rather than creating another.

#### 6.2.4 `memory`

```json
{
  "name": "memory",
  "description": "Store or recall durable information: user preferences, decisions, commitments, artifacts, and facts.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["remember", "recall"] },
      "kind": { "type": "string", "enum": ["semantic", "episodic", "commitment", "decision", "artifact", "preference"] },
      "content": { "type": "string", "minLength": 1, "maxLength": 4000 },
      "query": { "type": "string", "minLength": 1, "maxLength": 500 },
      "limit": { "type": "integer", "minimum": 1, "maximum": 20 },
      "project_id": { "type": "string" },
      "due_at": { "type": "integer" }
    },
    "required": ["action"]
  }
}
```

`remember` for `commitment` writes a `Commitment` into project memory when a project is bound,
and a semantic memory otherwise. `recall` uses the existing hybrid retrieval and returns at most
`limit` rows with score reasons.

#### 6.2.5 `being`

```json
{
  "name": "being",
  "description": "Read being state, or add/update a being-level goal.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["read", "goal_add", "goal_update", "goal_list"] },
      "description": { "type": "string", "minLength": 1, "maxLength": 1000 },
      "priority": { "type": "number", "minimum": 0, "maximum": 100 },
      "goal_id": { "type": "string" },
      "status": { "type": "string", "enum": ["active", "paused", "done", "abandoned"] }
    },
    "required": ["action"]
  }
}
```

Read returns identity, invariants, affect, voice projection, and goal counts. It never returns
the full memory or internal config.

#### 6.2.6 `kernel`

```json
{
  "name": "kernel",
  "description": "Read metamind health, inspect recent events, run a read-only SPARQL query, or validate a graph.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["health", "events", "query", "validate"] },
      "graph": { "type": "string" },
      "query": { "type": "string", "maxLength": 8000 },
      "limit": { "type": "integer", "minimum": 1, "maximum": 200 }
    },
    "required": ["action"]
  }
}
```

`query` is SELECT/ASK/CONSTRUCT only; `INSERT`, `DELETE`, `LOAD`, `SERVICE`, and `CLEAR` are
refused by keyword and by the store's read-only enforcement. Results are capped at 200 rows and
8 KiB of serialized data.

#### 6.2.7 `focus`

```json
{
  "name": "focus",
  "description": "Show a document in the console's left pane, or return to the state board.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["open", "state"] },
      "doc_id": { "type": "string" }
    },
    "required": ["action"]
  }
}
```

This replaces `focus.open`, `focus.read`, and the implicit focus behavior in the old tool results.
The tool result includes a `focus` object that the chat frame layer turns into a `ChatFrame::Focus`.

#### 6.2.8 `agent`

The full contract is in §7. Shape:

```json
{
  "name": "agent",
  "description": "Delegate external work to a downstream coding/research agent. Start a run, continue it turn-by-turn, check status, cancel it, or close it. This is the only tool that performs external work.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["start", "continue", "status", "cancel", "close"] },
      "agent_id": { "type": "string" },
      "kind": { "type": "string", "enum": ["hermes", "pi"] },
      "task": { "type": "string", "minLength": 1, "maxLength": 8000 },
      "message": { "type": "string", "maxLength": 8000 },
      "workspace": {
        "type": "object",
        "additionalProperties": false,
        "properties": {
          "root": { "type": "string" },
          "files": { "type": "array", "maxItems": 100, "items": { "type": "string" } }
        }
      },
      "acceptance_criteria": { "type": "array", "maxItems": 20, "items": { "type": "string", "maxLength": 500 } },
      "task_id": { "type": "string" },
      "max_turns": { "type": "integer", "minimum": 1, "maximum": 50 },
      "summarize": { "type": "boolean", "default": true }
    },
    "required": ["action"]
  }
}
```

There is no `shell`, no `url`, no `http`, and no free-form command argument. The downstream agent
receives a task, a workspace, and acceptance criteria; its own tool execution is confined to the
workspace policy.

### 6.3 Result envelope

Every CM tool returns the same JSON shape, serialized to the model:

```json
{
  "ok": true,
  "code": "OK",
  "message": "topic drilled",
  "data": { "...": "..." },
  "next": {
    "kind": "continue",
    "hint": "The agent run is still running. Continue with action=continue."
  },
  "meta": {
    "tool": "context",
    "duration_ms": 4,
    "revision": 13,
    "truncated": false,
    "idempotency_key": "sess/turn-4/ctx-drill"
  }
}
```

Error:

```json
{
  "ok": false,
  "code": "CONFLICT",
  "message": "expected revision 11, current revision 13",
  "data": {
    "current_revision": 13,
    "current_path": ["release 2.0", "auth migration"],
    "allowed_actions": ["read", "set_main", "drill", "back"]
  },
  "next": { "kind": "retry", "hint": "Call context action=read, then retry once." },
  "meta": { "tool": "context", "duration_ms": 2, "truncated": false }
}
```

Codes:

| Code | Meaning | Model action |
|---|---|---|
| `OK` | Success | Continue |
| `INVALID_ARGUMENTS` | Schema or action-specific validation failed; message names the field | Fix the call |
| `NOT_FOUND` | Referenced id/graph/doc does not exist | Read or create first |
| `CONFLICT` | Revision/staleness/dependency violation | Re-read, then retry once |
| `FORBIDDEN` | Policy or authorization refused | Do not retry; report |
| `UNAVAILABLE` | A backing service is closed or unreachable, with a reason | Report; do not fabricate |
| `TIMEOUT` | The operation exceeded its budget | Retry only if idempotent |
| `BUDGET` | Turn or tool budget exhausted | Pause and report remaining work |
| `INTERNAL` | Unexpected handler failure | Report with the correlation id |

`message` is human-readable and safe to quote. `data` is the machine payload. `next` is a small
enum (`continue`, `retry`, `stop`, `wait_for_user`) plus one sentence. The model sees `next` on
every result; the loop uses it to decide whether to keep going.

**Truncation.** Results larger than 24 KiB are truncated only at a **structural** boundary:

```json
{
  "ok": true,
  "code": "OK",
  "data": {
    "truncated": true,
    "returned": 50,
    "total": 412,
    "preview": [ "... first 50 rows ..." ],
    "handle": "tool-result/01J..."
  },
  "meta": { "truncated": true, "artifact": "tool-result/01J..." }
}
```

The full payload is stored in the action/agentstream log and can be fetched with a follow-up
`kernel` or `memory` read. The model never receives malformed JSON.

### 6.4 Invocation pipeline

```
model tool call
   │
   ▼
1. parse        strict JSON; empty string is `{}` only for no-argument tools
   ▼
2. schema       JSON Schema validation + action-specific validation
   ▼
3. authorize    CM tool policy (session, project, workspace, confirmation)
   ▼
4. idempotency  for writes: key = (session, turn, tool, action, canonical args)
   ▼
5. execute      tokio::time::timeout(per-tool budget)
   ▼
6. envelope     typed result, structural truncation, duration, revision
   ▼
7. record       agentstream ToolCall/ToolResult + action ledger for mutating tools
   ▼
8. return       envelope to the model; emit status invalidation
```

Per-tool budgets:

| Tool | Timeout | Retry | Confirmation |
|---|---|---|---|
| `context` | 2 s | once on conflict (after read) | never |
| `project` | 5 s | no | on `create` if it writes outside the data dir |
| `task` | 5 s | no | never |
| `memory` | 10 s | once on transient | on `remember` when it contains personal data? No — the operator asked |
| `being` | 5 s | no | on goal mutation |
| `kernel` | 15 s | once on timeout for `health`/`events` only | never |
| `focus` | 1 s | no | never |
| `agent` start/continue | 90 s | no automatic retry | policy in §7.8 |

Writes are never automatically retried without an idempotency key. The key is derived from the
provider's tool-call id when present, otherwise from `(session, turn, call index)`, so a replay of
the same turn does not duplicate a task or a topic.

### 6.5 The loop policy: continue until done

The executor's current `MAX_TOOL_ROUNDS = 4` / `MAX_TOOL_STEPS = 8` is replaced by a policy
object attached to the turn plan:

```rust
pub struct TurnLoopPolicy {
    pub mode: LoopMode,              // SingleAnswer | ContinueUntilDone
    pub max_rounds: u32,             // default 24
    pub max_tool_steps: u32,         // default 48
    pub wall_clock_ms: u64,          // default 600_000
    pub max_agent_turns: u32,        // default 30 across the job
    pub require_context_update: bool // default true
}

pub enum LoopMode { SingleAnswer, ContinueUntilDone }
```

Rules:

1. A turn with `ContinueUntilDone` keeps alternating model calls and tool calls while:
   - the model returns tool calls, **or**
   - any agent run started in this turn has status `running`/`waiting_input`, **or**
   - the context has `next_actions` that the model has not marked done and the model did not
     explicitly state it is pausing.
2. Before the model may produce a user-visible final answer, the loop checks:
   - if any linked agent run is not terminal → do not finalize; inject a system message:
     "Run 01… is still running. Call `agent` action=continue or action=status. Do not tell the
     operator the work is finished."
   - if `require_context_update` and the topic changed → inject a reminder to call `context`.
3. At `max_rounds`/`max_tool_steps`/`wall_clock_ms`, the turn does **not** fail. It ends in
   `paused` with a structured `TurnProgress` block: completed actions, active agent runs,
   remaining next actions, and a one-click "continue" suggestion. The session stores the
   continuation so the next operator message or a proactive tick can resume.
4. `SingleAnswer` is retained for direct questions and proactive asides. The planner chooses
   `ContinueUntilDone` when the query is operational (imperative verb, project/task nouns, or an
   active agent run is referenced).

The loop is implemented once, in `mm-cme/src/executor.rs`, not in each tool. Tools only return
`next` hints; the policy owns termination.

### 6.6 Deprecation map

| Old catalog row(s) | New tool | Notes |
|---|---|---|
| `state.show`, `being.show`, `being.personality_show`, `doctor.run`, `budget.show` | `kernel` + `being` | Prompt already carries most of it; tools are for explicit reads |
| `catalog.open`, `guide.*` | — | The CM no longer discovers tools at runtime |
| `session.note` | `context` `note` | Notes are conversation state, not memory |
| `focus.open`, `focus.read`, `focus.patch` | `focus` | Patching a document is an operator action, not a CM tool |
| `memory.add`, `memory.recall`, `memory.forget` | `memory` | `forget` is not exposed; operator-only |
| `being.goal_add`, `being.goal_list` | `being` | |
| `changeset.*`, `sandbox.*`, `promote.*`, `reject.*`, `audit.*` | `agent` (delegated) | The CM delegates; it cannot promote |
| `loop.*`, `debt.*`, `gc.*`, `module.*` | `agent` (delegated) | Engineering work goes through an agent in a sandbox |
| `pi.run`, `pi.ingest` | `agent` | |
| `codex.*`, `llm.stats`, `meta.analyze`, `regression.run`, `verify.run`, `calibrate.*`, `mcp.*` | `kernel` / `agent` | Read-only inspection or delegated work |
| `decide.ask`, `firewall.eval`, `compare.check`, `risk.analyze`, `conformance.run` | — | Not conversational jobs; the operator or a delegated agent uses them explicitly |
| `library.*`, `policy.history`, `skill.retrieve`, `case.retrieve`, `frame.missing` | `memory` | Retrieval is memory-shaped; library administration stays in the CLI |
| `action.ledger`, `action.rollback` | status panel / operator | The panel shows operations; rollback is an operator action |

---

## 7. The agent tool and downstream agents

### 7.1 The agent service

New crate `crates/mm-agent`. The CM depends on one narrow trait:

```rust
#[async_trait]
pub trait AgentService: Send + Sync {
    async fn start(&self, spec: AgentSpec) -> AgentResult<AgentRun>;
    async fn next_turn(&self, run_id: &str, input: AgentInput) -> AgentResult<AgentTurnResult>;
    async fn status(&self, run_id: &str) -> AgentResult<AgentRun>;
    async fn cancel(&self, run_id: &str) -> AgentResult<AgentRun>;
    async fn close(&self, run_id: &str) -> AgentResult<AgentRun>;
}

pub struct AgentSpec {
    pub kind: AgentKind,                 // Hermes | Pi
    pub task: String,
    pub workspace: Option<AgentWorkspace>,
    pub acceptance_criteria: Vec<String>,
    pub linked_task: Option<String>,
    pub max_turns: u32,
    pub model: Option<String>,
}

pub struct AgentRun {
    pub run_id: String,
    pub kind: AgentKind,
    pub session_id: String,              // agentstream session id: "agent/<run_id>"
    pub status: AgentStatus,             // running | waiting_input | done | failed | cancelled
    pub turn: u32,
    pub task: String,
    pub linked_task: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub summary: Option<String>,         // rolling run summary
    pub last_error: Option<String>,
    pub usage: AgentUsage,
}

pub struct AgentTurnResult {
    pub run_id: String,
    pub turn: u32,
    pub status: AgentStatus,
    pub message: String,                 // the agent's reply for this turn (possibly summarized)
    pub summary: Option<String>,         // present when the turn was long
    pub tool_calls: Vec<AgentToolSummary>,
    pub artifacts: Vec<TopicRef>,
    pub usage: AgentUsage,
    pub next: AgentNext,                 // continue | wait_for_input | done | failed
}

pub enum AgentNext { Continue, WaitForInput, Done, Failed }
```

The registry in `mm-agent/src/registry.rs` owns live runs:

```rust
pub struct AgentRegistry {
    runs: DashMap<String, AgentHandle>,   // run_id -> backend handle + metadata
    backends: BackendRegistry,            // kind -> Arc<dyn AgentBackend>
    store: Arc<AgentstreamStore>,
    summarizer: Arc<TurnSummarizer>,
}
```

`AgentHandle` holds the backend-specific cursor (Hermes conversation messages, Pi process) and
the run record. The registry is shared through `AppState`.

### 7.2 Hermes backend (OpenAI API)

Hermes is already running as a service. The design makes no assumption about its internal API
beyond **OpenAI-compatible chat completions**, which the operator explicitly named.

**Resolving the running service.** The design does not hard-code a port. On boot, `mm-agent`
resolves Hermes in this order:

1. `MM_AGENT_HERMES_BASE_URL` (explicit; wins);
2. the `[hermes]` table in `config/agents.toml` (endpoint and model, no secret);
3. otherwise the backend is `UNAVAILABLE` with the reason `MM_AGENT_HERMES_BASE_URL is unset`.

At startup it probes `GET {base}/models` with a short timeout. A successful probe records the
model list and marks the backend `ok`; a failed probe leaves it `unavailable` with the HTTP
status or connection error. The status panel shows that reason verbatim. There is no silent
fallback to another provider and no fabricated run.

Configuration (environment, never committed):

| Variable | Meaning | Default |
|---|---|---|
| `MM_AGENT_HERMES_BASE_URL` | Hermes OpenAI-compatible base, e.g. `http://127.0.0.1:PORT/v1` | none — `UNAVAILABLE` with a reason if unset |
| `MM_AGENT_HERMES_API_KEY` | Bearer token | none; may be empty for a local service |
| `MM_AGENT_HERMES_MODEL` | Model name advertised by Hermes | none; discovered from `GET /models` if possible |
| `MM_AGENT_HERMES_STREAM` | Use SSE streaming | `true` |
| `MM_AGENT_HERMES_TIMEOUT_MS` | Per-turn timeout | `120000` |
| `MM_AGENT_HERMES_MAX_CONTEXT_TOKENS` | Context budget before summarization | `96000` |

Protocol:

- `start` sends `POST /chat/completions` with a system prompt describing the task, workspace,
  acceptance criteria, and the agent's own tool policy. The response is stored as the first
  assistant turn; all messages are persisted in the run's agentstream session.
- `continue` sends the accumulated `messages` (rebuilt from agentstream, not trusted to a
  server-side session) plus the CM's `message`. This makes the run reproducible and independent
  of Hermes session retention.
- If the service supports it, a `X-MM-Agent-Session` / conversation id header is recorded for
  correlation but **not required** for correctness.
- Streaming: parse `data:` frames, accumulate `delta.content` and `delta.tool_calls`, handle
  `[DONE]`, and log a `AgentMessage` event per completed turn plus `ToolCall`/`ToolResult` for
  any tool call Hermes performs internally. The CM never sees raw deltas.
- Non-streaming fallback: if the service returns a normal JSON completion, use it. The backend
  reports which mode it used in `AgentTurnResult.meta`.

A dedicated client lives in `mm-agent/src/hermes.rs` rather than reusing `mm-llm::OpenAiClient`
because:

- `mm-llm` is optimized for typed, schema-validated metamind calls, not agent transcripts with
  arbitrary tool payloads;
- the agent client needs SSE and long timeouts;
- the agent endpoint is a separate service with its own model catalog and failure semantics;
- keeping it separate means a Hermes version change cannot destabilize the metamind LLM substrate.

`mm-llm` is still used for **summarization** of long turns, because that is a typed,
purpose-scoped call (`Purpose::Summarize`).

### 7.3 Pi backend (programmatic RPC)

Pi is already integrated in `mm-pi` (`PiClient`, `PiCommand::{Prompt, Steer, FollowUp, Abort,
ClearQueue, NewSession}`, `PiEvent`). `mm-agent/src/pi.rs` adapts it:

- `start` spawns `PiClient` with `PiConfig`:
  - `cwd` = the requested workspace (must be inside `data/sandbox` unless an explicit
    `MM_AGENT_ALLOWED_ROOTS` entry covers it),
  - `session_dir` = `data/sandbox/pi/<run_id>`,
  - tools = the default read/write/edit/list set; no shell, matching `PiConfig::default_tools()`,
  - provider/model from `MM_AGENT_PI_PROVIDER` / `MM_AGENT_PI_MODEL`.
- `continue` sends `PiCommand::Prompt` with a correlation id; `cancel` sends `Abort`; `close`
  sends `ClearQueue` then drops the client (which kills the process via `kill_on_drop`).
- Every `PiEvent` is mapped to the same `AgentTurnResult` shape. Pi's own tool records become
  `AgentToolSummary` rows; the raw event is retained in the agentstream.
- If the binary is absent, `start` returns `UNAVAILABLE` with the exact reason (matching Pi's
  existing spawn error), never a fake run.
- The `pi.ingest` path is retained for recording an existing Pi session as an agentstream run;
  it is an operator import, not a CM tool.

### 7.4 The turn-by-turn contract

The CM never waits for a whole job. Each `agent` call performs **at most one downstream turn**:

```
CM: agent.start(kind=hermes, task="Reproduce the failing auth test and report the cause",
                workspace={root:"..."}, acceptance_criteria=["cause identified","command recorded"])
    → {run_id, turn:1, status:"running", message:"...", summary?}
CM: agent.continue(run_id, message="Also try the flaky seed 42.")
    → {run_id, turn:2, status:"running", message:"...", summary?}
CM: agent.continue(run_id, message="")
    → {run_id, turn:3, status:"done", message:"Cause: ...", summary:"...", artifacts:[file ref]}
```

- An empty `message` means "continue with the existing task" and is allowed.
- `max_turns` is enforced per run; exceeding it returns `status: failed` with `code: BUDGET` and
  the transcript preserved.
- If the agent asks a question (`status: waiting_input`), the CM must relay it to the operator and
  stop the loop with `next: wait_for_user`. It must not invent an answer.
- `cancel` is honored within one turn boundary; the backend receives `Abort`/`POST /cancel` and
  the run ends `cancelled`.
- `close` is idempotent. A run that is already terminal returns its record unchanged.

### 7.5 Per-turn summarization

Long agent turns are summarized **separately** before the CM sees them:

```
if turn_tokens > MM_AGENT_SUMMARY_THRESHOLD (default 6000)
   or turn_chars > 24000:
       summary = mm_llm.summarize(turn, purpose=Summarize, schema=AgentTurnSummary)
       log AgentTurnSummary event
       AgentTurnResult.message = summary.short
       AgentTurnResult.summary = summary
       AgentTurnResult.raw_ref = "agentstream/<session>/<event-id>"
else:
       message stays verbatim
```

`AgentTurnSummary`:

```rust
pub struct AgentTurnSummary {
    pub headline: String,          // <= 200 chars
    pub did: Vec<String>,          // <= 8 concrete actions
    pub learned: Vec<String>,      // facts the CM needs to decide next
    pub artifacts: Vec<TopicRef>,  // files, diffs, commands, URLs
    pub blockers: Vec<String>,
    pub next_recommended: Option<String>,
    pub confidence: f32,           // 0..1
}
```

Rules:

- Summaries are generated from the **raw turn** plus the previous rolling run summary, so a
  30-turn run does not require the CM to read 30 raw turns.
- The raw turn is never deleted; the summary links to it.
- The summarizer prompt instructs: "Do not omit a command that failed, a file path, or an
  unresolved error. If the turn contains a question, put it in `blockers`."
- If summarization fails (model unavailable), fall back to a deterministic extractor: first 2,000
  chars, last 2,000 chars, every tool call name, and every artifact path, with a
  `summary_failed: true` marker. The CM is told the summary is degraded.

### 7.6 Separate agentstream sessions

Each run gets its own session id:

```
agent/<run_id>
```

Metadata:

```json
{
  "userId": "console-operator",
  "projectId": "<bound project>",
  "title": "Hermes: Reproduce failing auth test",
  "kind": "agent",
  "parentSessionId": "<conversation session>",
  "agentRunId": "<run_id>",
  "agentKind": "hermes"
}
```

This gives:

- an isolated trace for the viewer,
- an isolated message history for rebuilding the backend conversation,
- a clean join key from `AgentRun.session_id`,
- no pollution of the CM transcript or the CM's token budget.

The CM's `agent` tool result is the only bridge. The trace viewer reads the agent session; the
status panel reads the run registry; the CM reads the compact turn result.

### 7.7 Agent lifecycle state machine

```
            start
              │
              ▼
          ┌────────┐   continue (turn ok)   ┌────────┐
          │running │◄──────────────────────►│running │
          └────┬───┘                        └────────┘
               │ asks a question
               ▼
        ┌──────────────┐  operator/CM input
        │waiting_input │──────────────────────┐
        └──────────────┘                      │
               │ done/failed/cancelled        │
               ▼                              ▼
          ┌─────────┐                    (continue)
          │ done    │
          │ failed  │
          │cancelled│
          └─────────┘
```

- `running` → `running` is the normal turn loop.
- `running` → `waiting_input` is the only state that forces `wait_for_user`.
- Terminal states are immutable except for `close` (idempotent) and a new run.
- A crash of `mm-serve` leaves the run record `running` with a stale `updated_at`; on boot the
  registry marks runs older than the process start as `failed` with reason `"server restarted"`.
  The transcript remains in agentstream. This is honest and inspectable.

### 7.8 Limits, safety, and configuration

| Concern | Rule |
|---|---|
| Workspace | Default root is `data/sandbox/agents/<run_id>`; an explicit root is accepted only under `MM_AGENT_ALLOWED_ROOTS` (colon-separated absolute paths). Paths are canonicalized and must not escape. |
| Confirmation | `agent.start` is allowed without a prompt when the workspace is inside the sandbox and `task` is non-empty. A root outside the sandbox, or an action marked `destructive`, requires operator confirmation. `continue`, `status`, `cancel`, `close` never prompt. |
| Secrets | The Hermes key is read from the environment only. It is never written to agentstream, tool results, or logs. A redaction pass runs over every agent event payload. |
| Network | Only the configured Hermes base URL. No arbitrary URL argument. |
| Concurrency | Per session, at most `MM_AGENT_MAX_ACTIVE_RUNS` (default 3) runs. Additional starts return `BUDGET`. |
| Cancellation | Operator stop cancels the active CM turn and sends `cancel` to every run started by it. |
| Replay | The agentstream is the transcript of record. A run can be reconstructed from its events without the backend. |
| Data retention | Raw events follow the existing agentstream policy. Summaries are retained with the run record. |

---

## 8. The conversation loop and personalization

### 8.1 Prompt assembly

`mm-cme/src/prompt.rs` (new) builds `PromptPacket` from:

| Source | Fields | Failure behavior |
|---|---|---|
| `BeingFacade` | self-description, version, invariants, values, voice | `being unavailable: <reason>` |
| User model | preferred name, expertise, preferences, current priorities | omit with `user model unavailable` |
| Relationship | familiarity, trust, open issues | omit with a reason |
| `SessionContext.affect` | valence/arousal/confidence → tone line | defaults |
| `SessionContext.memory` | project summary, previous topics, greeting | omit with a reason |
| `ConversationContext` | topic tree, path, intent, questions, next actions, refs | seed a system topic if empty |
| `WorkStore` | project, doing/upcoming/blocked tasks | empty list |
| `AgentRegistry` | active runs and last turn summaries | empty list |
| Recent transcript | last N turns, compressed if long | existing compression |
| `TurnPlan` | query, modality, detail, budget, loop policy | required |

The packet is inserted as one system message before the transcript. A second system message
carries the loop contract (continue until done, context update requirement, agent honesty rule).
The operator's query remains a user message.

### 8.2 The turn algorithm

```
1. Session receives StartTurn(query).
2. Planner builds TurnPlan; chooses LoopMode.
3. PromptBuilder assembles PromptPacket (context read, work read, agent registry read).
4. Executor calls the model with 8 tool schemas.
5. While the model returns tool calls:
     a. validate and execute each call through the invocation pipeline
     b. append the typed envelope
     c. if a tool mutated context/work, emit status invalidation
     d. if the agent tool returned running/waiting_input, append the loop directive
     e. check loop budget; on exhaustion → paused outcome
6. Before final text:
     a. if an agent run is non-terminal → force another round with a directive
     b. if the topic changed and require_context_update → force a context reminder
7. Stream the final text.
8. Persist the turn, context revision, task changes, and agent links.
9. Emit a status revision and a chat completion frame.
```

The loop contract text is explicit and repeated at the point of decision:

> You are the conversation manager. You may call any of the eight tools as many times as needed
> in this turn. If an agent run is `running`, you must continue it or check it; do not tell the
> operator the work is finished. If the topic has changed, call `context` before answering. If
> you cannot finish within the budget, say what remains and stop honestly.

### 8.3 Why this feels personal

- **Identity is sourced from the being, not invented.** Self-description, values, and invariants
  are the actual persisted being, so the CM's voice is continuous across sessions.
- **The operator is a person in the prompt, not a `user` role.** Preferred name, expertise,
  communication preferences, and current priorities are rendered as relationship context.
- **Continuity is explicit.** The previous project narrative, previous topics, and open
  commitments are shown. The CM can say "we left the refresh test flaky" instead of starting cold.
- **Affect is a control signal, not a claim.** The prompt maps affect to tone guidance ("steady,
  not enthusiastic") and keeps identity invariants above it.
- **Topic continuity is visible and editable.** The CM updates the topic tree; the operator sees
  the same tree in the panel. When the CM gets it wrong, the operator can correct it and the next
  turn starts from the correction.
- **Personal memory is used, not merely stored.** `memory.recall` is available, and the prompt
  includes the project's open commitments and the last session's continuation.

Guardrails: the CM never fabricates autobiographical memory, never claims an external action
without an agent/tool result, and never converts an assumption into an observation. The prompt
states these invariants verbatim from the being.

### 8.4 Cross-session continuity

On session start, `CmeRuntime::ensure_session` loads:

1. the latest `ProjectMemory` for the bound project,
2. the previous session's `ConversationContext` (topic tree and open next actions),
3. the user model and relationship state,
4. any open commitments and upcoming tasks.

The greeting and "suggested continuation" are passed into the prompt packet; the first turn does
not need to ask "what were we doing?". If no previous context exists, the continuity block is
omitted with a reason, not populated with invented history.

---

## 9. Live status backend

### 9.1 The view model

`mm-serve/src/status.rs` (new) composes one view per conversation session:

```rust
pub struct ConversationStatusView {
    pub revision: u64,
    pub session_id: String,
    pub generated_at: i64,
    pub topic: TopicView,
    pub project: Option<ProjectView>,
    pub conversation: ConversationView,
    pub metamind: MetamindView,
    pub tasks: TaskBoardView,
    pub operations: OperationsView,
    pub agents: AgentsView,
    pub variables: VariablesView,
}
```

```rust
pub struct TopicView {
    pub main: Option<TopicSummary>,
    pub path: Vec<TopicSummary>,        // root → current
    pub breadcrumb: String,             // "release 2.0 › auth migration › token refresh"
    pub current_refs: Vec<RefView>,
    pub intent: Option<String>,
    pub open_questions: Vec<String>,
    pub next_actions: Vec<String>,
    pub updated_at: i64,
    pub origin: String,                 // user | cm | system
}

pub struct ConversationView {
    pub turn: Option<TurnView>,         // id, state, phase, model, elapsed_ms
    pub turns_completed: u32,
    pub context_revision: u64,
    pub pending_confirmation: Option<String>,
    pub loop: LoopView,                 // mode, round, max_rounds, remaining_ms
}

pub struct MetamindView {
    pub kernel: KernelView,             // state, reason, services, uptime, event head, audit head
    pub being: BeingView,               // version, self_description, goals, affect, voice
    pub budgets: Vec<BudgetView>,
    pub memory: MemoryView,             // episodic count, segments, project memory updated_at
    pub llm: LlmView,                   // model, calls, cache hit rate, reason
    pub cme: CmeView,                   // region states, transition counts
}

pub struct TaskBoardView {
    pub counts: TaskCounts,             // todo/doing/blocked/done
    pub doing: Vec<TaskSummary>,
    pub upcoming: Vec<TaskSummary>,     // sorted by due, then priority desc, limit 8
    pub blocked: Vec<TaskSummary>,      // with reasons
    pub unprojected: Vec<TaskSummary>,
}

pub struct OperationsView {
    pub recent: Vec<OperationView>,     // last 20, newest first
    pub active: Vec<OperationView>,     // in-flight tool calls / agent turns
}

pub struct AgentsView {
    pub active: Vec<AgentRunView>,
    pub recent: Vec<AgentRunView>,      // last 5 terminal runs
}

pub struct VariablesView {
    pub feature_flags: BTreeMap<String, bool>,
    pub preferences: BTreeMap<String, f32>,
    pub counters: BTreeMap<String, u64>, // turns, tool calls, agent turns, cache hits
    pub routing: Option<RoutingView>,
    pub limits: BTreeMap<String, u64>,
}
```

`TaskSummary` is deliberately small (id, title, status, priority, due, blocked reason, agent run)
so the panel stays readable.

### 9.2 The status service

The naive implementation — compose everything on every request — would hit RDF, SQLite, the being
mutex, CME observability, and the agent registry several times per second. The design avoids that.

```
                 ┌────────────── StatusService (per session) ─────────────┐
 CME transition ─┤ dirty: conversation                                     │
 context write  ─┤ dirty: topic, tasks, variables                          │
 work write     ─┤ dirty: tasks                                            │
 agent event    ─┤ dirty: agents, operations                               │
 health tick    ─┤ dirty: metamind.kernel                                   │
 tool call      ─┤ dirty: operations                                       │
                 │                                                        │
                 │  coalesce 100ms → recompute dirty sections only        │
                 │  assign revision → store snapshot → broadcast frame    │
                 └────────────────────────────────────────────────────────┘
```

- **Dirty flags per section.** `topic`, `project`, `tasks`, `operations`, `agents`, `metamind`,
  `variables`, `conversation`. A context write does not re-read the graph or the being.
- **Coalescing.** A 100 ms debounce collects a burst of tool events into one revision. The UI
  receives at most 10 revisions per second; under load the service drops intermediate revisions
  and always sends the latest (the client only needs the newest complete view).
- **Revision.** Monotonic per session, starting at 1. Every frame carries `revision` and
  `generated_at`. The client ignores frames with `revision <= last_applied`.
- **Snapshot cache.** `RwLock<ConversationStatusView>` plus per-section `updated_at`. A full GET
  returns the cached view; if the cache is colder than `MM_STATUS_MAX_AGE_MS` (default 2 s), it
  recomputes synchronously before answering.
- **No user-specific filtering** in this design: the console is a single-operator surface
  (`CONSOLE_USER`). Multi-user filtering is a later extension; the view model keeps
  `session_id` and `project_id` so it can be added without reshaping the wire.

### 9.3 Endpoints

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/console/sessions/{session_id}/status` | Full `ConversationStatusView` |
| `GET` | `/api/console/sessions/{session_id}/status/stream?from=<revision>` | SSE: `status` frames, `heartbeat` every 10 s |
| `GET` | `/api/console/sessions/{session_id}/operations?limit=50` | Operation history (paged) |
| `GET` | `/api/agents` | Agent runs (active + recent), optional `session_id` |
| `GET` | `/api/agents/{run_id}` | One `AgentRunView` |
| `GET` | `/api/agents/{run_id}/events?from=<seq>&types=...` | Raw trace events |
| `GET` | `/api/agents/{run_id}/stream?from=<seq>` | SSE trace events |
| `POST` | `/api/agents/{run_id}/cancel` | Operator cancellation |
| `GET` | `/api/agentstream/sessions` | Sessions available in the viewer |
| `GET` | `/api/agentstream/sessions/{id}/events` | Generic agentstream read (CM traces too) |
| `GET` | `/api/agentstream/sessions/{id}/stream` | Generic agentstream SSE |

All routes require the existing console auth/lock posture. The agent cancel route is a mutating
operator action; it is not exposed to the CM.

### 9.4 SSE frame format

```
event: status
id: 42
data: {"revision":42,"sessionId":"01…","generatedAt":1763…,"topic":{…},"tasks":{…},…}

event: heartbeat
id: 42
data: {"revision":42,"at":1763…}
```

- The first frame after connect is always a **full** view, even when `from` is current. This
  removes a whole class of client bugs where a delta arrives before the initial snapshot.
- When `from` is older than the retained history (default 256 revisions), the server sends a full
  view with `reason: "resync"`.
- Frames are ordered by `revision`; the client never reorders because the transport preserves
  order. The client still ignores stale revisions defensively.
- A slow client is disconnected after `MM_STATUS_CLIENT_LAG_LIMIT` (default 64 pending frames)
  and reconnects to a full snapshot. This prevents unbounded memory growth.

### 9.5 Performance and failure behavior

- Target: p95 status recompute < 10 ms; p95 full GET < 30 ms; SSE event delivery < 250 ms after
  the underlying mutation.
- Every section carries its own `unavailable` reason. A closed being store does not fail the whole
  panel; it renders "Being unavailable: reason" while the rest updates.
- The service never blocks the turn path. Tool/agent execution publishes to an in-process
  broadcast channel; the status service consumes asynchronously. If the status channel is full,
  the service marks the section dirty and recomputes on the next tick; the turn is unaffected.
- The health poll remains for the top bar, but the inspector does not depend on it.

---

## 10. Frontend: the Inspector rail

### 10.1 Layout

The existing workspace is a two-pane split (`document` left, `chat` right). The inspector is a
**right-hand auxiliary rail**, not a third workspace pane:

```
┌──────────┬─────────────────────────────┬──────────────────────────────┬──────────────┐
│ sidebar  │ Document pane               │ Chat pane                    │ Inspector    │
│ (menu)   │ (focus doc / state board)   │ (transcript + composer)      │ 320–380 px   │
│          │                             │                              │ Status|Trace │
└──────────┴─────────────────────────────┴──────────────────────────────┴──────────────┘
```

Rules:

- **Wide (≥ 1440 px):** inspector is a resizable third rail, 320 px default, 280–480 px range.
  It does not count as a document pane and does not participate in the swap. The two-pane
  workspace remains intact inside the middle region.
- **Medium (1024–1439 px):** inspector collapses to a 40 px rail with two icon buttons; opening
  it overlays the chat pane as a 360 px drawer with a scrim on the document side.
- **Narrow (< 1024 px):** inspector is a full-height sheet opened from the chat header; tabs are
  `Status` and `Trace`.
- Persisted state: `inspectorOpen`, `inspectorWidth`, `inspectorTab` in a new `useInspectorStore`.
- Keyboard: `⌘⇧I` toggles the inspector; `⌘⇧1` / `⌘⇧2` select Status / Trace. (Existing
  `⌘1`/`⌘2` remain pane focus.)
- Accessibility: `<aside aria-label="Inspector">`; tabs implement `role="tablist"` /
  `role="tab"` / `role="tabpanel"`; the live region is `aria-live="polite"` and only announces
  the topic headline, not every counter.

### 10.2 Status panel anatomy

Top to bottom, exactly matching the requested priority:

```
┌ Inspector ────────────────────────────── [Status] [Trace] [×] ┐
│ ● LIVE  rev 42 · 0.3s ago                                     │
│                                                               │
│ TOPIC                                                         │
│ release 2.0 › auth migration › token refresh       [auto|cm]  │
│ "make the refresh test deterministic"                         │
│ ref: src/auth/refresh.rs  [file ✓]                            │
│ intent: reproduce the flake                                   │
│ open: Do we support clock skew?                               │
│ next: run auth test 20× · inspect seed                        │
│                                                               │
│ CONVERSATION                                                  │
│ turn 7 · streaming · model auto/remote · 4.2s · round 3/24    │
│ context rev 42 · 7 turns · 1 confirmation pending             │
│                                                               │
│ METAMIND                                                      │
│ kernel ok · sqlite ok · graph ok · eventlog ok · cme ok       │
│ being v3 · warmth .72 · directness .64 · curiosity .81       │
│ budgets: meta 12% · improve 4% · evolve 0% · wall 8%          │
│ memory: 1,284 episodic · 37 segments · project synced 12:01   │
│ llm: auto/remote · 14 calls · cache 74%                       │
│ cme: proactive idle · voice off · memory idle · plugins idle  │
│                                                               │
│ TASKS                                                         │
│ doing: Wire inspector SSE (80, due 11-24)                     │
│ upcoming: Trace viewer grouping (70) · Agent crate tests (60) │
│ blocked: Promote agent crate — waiting on review              │
│ 3 todo · 1 doing · 1 blocked · 4 done                         │
│                                                               │
│ AGENTS                                                        │
│ hermes 01… · running · turn 2 · "Reproduce failing test"      │
│   last: test passes with seed 1; fails with seed 42           │
│ pi     — no active run                                        │
│                                                               │
│ RECENT OPERATIONS                                             │
│ 12:04:18 agent.continue → running, turn 2            (1.2s)   │
│ 12:04:11 context.drill "token refresh"                 (3ms)  │
│ 12:04:05 task.update doing                             (4ms)  │
│ 12:03:58 kernel.health ok                             (11ms)  │
│                                                               │
│ INTERNAL VARIABLES                                            │
│ revision 42 · agent runs 1 · topic depth 2 · pins 1          │
│ flags: cme on · agentstream on · proactive off                │
│ prefs: verbosity +0.2 · technical depth +0.6                  │
│ limits: rounds 24 · tools 48 · wall 600s                      │
└───────────────────────────────────────────────────────────────┘
```

Each section is a collapsible `<section>` with a sticky header and a count badge. The topic
section is never collapsible; it is the anchor of the panel.

Component breakdown:

| Component | Responsibility |
|---|---|
| `InspectorRail` | Width, open/closed, tab switch, responsive behavior |
| `InspectorHeader` | Live chip, revision, generated-at, refresh button, close |
| `TopicHeader` | Breadcrumb, current label, origin badge, refs, intent, questions, next actions |
| `ConversationBlock` | Turn state machine phase, model, elapsed, loop round, pending confirmation |
| `MetamindBlock` | Kernel services, being/voice, budgets, memory, LLM, CME regions |
| `TaskBoard` | Doing, upcoming, blocked, counts; click opens the task document |
| `AgentRuns` | Active and recent runs; click opens the trace tab at that run |
| `OperationList` | Recent operations with tool, target, duration, outcome, error code |
| `VariablesBlock` | Flags, preferences, counters, routing, limits |
| `UnavailableRow` | Consistent "unavailable: reason" row used by every block |

### 10.3 Trace viewer anatomy

The Trace tab is a virtualized event stream for one agent run (default: the newest active run;
a selector lists all runs).

```
┌ Trace ───────────────────────────────────────────── [run ▾] [×] ┐
│ hermes · 01J… · running · turn 2/30 · 00:01:42 · model hermes-1 │
│ [Turns] [Tools] [Raw]   follow ☑   filter: [all ▾]   [cancel]  │
│                                                                │
│ ── TURN 1 ───────────────────────────────────────── 00:00:12 ── │
│ system   Task: reproduce failing auth test …                    │
│ agent    I'll inspect the test file first.                      │
│ tool     read_file src/auth/refresh.rs            ok · 4ms      │
│ tool     run_tests auth::refresh                  fail · 1.8s   │
│ agent    The failure is seed-dependent …                        │
│ summary  headline: seed 42 fails on clock skew                  │
│          did: read file · ran test · isolated seed              │
│          artifacts: src/auth/refresh.rs                         │
│                                                                │
│ ── TURN 2 ───────────────────────────────────────── 00:01:30 ── │
│ agent    I'll add a controlled clock …                          │
│ tool     edit_file src/auth/refresh.rs            ok · 6ms      │
│ …                                                              │
└────────────────────────────────────────────────────────────────┘
```

Features:

- **Turn grouping.** Events are grouped by `turn_id`; a sticky turn header shows index, status,
  duration, and the per-turn summary badge.
- **Role rows.** `system`, `user`, `agent`, `tool`, `summary`, `status`, each with a distinct
  left border token from the design system (never color alone; a role label is always present).
- **Tool cards.** Tool name, arguments (collapsible JSON, pretty-printed), result preview,
  duration, `ok`/`fail`, and an "open artifact" action for files/diffs.
- **Summary rows.** The LLM summary is shown by default; "Show raw" expands the exact assistant
  content. This is the key affordance for long agent runs.
- **Filters.** `all`, `messages`, `tools`, `summaries`, `errors`. A turn filter selects one turn.
- **Follow.** Auto-scroll while the user is at the bottom; pause when they scroll up and show a
  "jump to live" chip. Resume on click.
- **Only when viewed.** The trace stream is subscribed only when the Trace tab is active and the
  document is visible. On tab switch or `visibilitychange`, close the EventSource; reconnect with
  `from=<lastSeq>` on return. This is the "if being viewed" requirement.
- **Virtualization.** `@tanstack/react-virtual` over a flattened event list; turn headers and
  summary rows are items in the same list. Target 60 fps with 5,000 events.
- **Empty/error states.** "No runs yet", "This run has no events", "Trace unavailable: reason",
  each with the next action (start an agent, retry).

### 10.4 Live update data flow

```
StatusService ──SSE status──► useStatusStream(sessionId)
                                  │
                                  ▼
                         statusReducer (revision-guarded)
                                  │
                                  ▼
                    useInspectorStore (sections)
                                  │
              ┌───────────────────┼───────────────────┐
              ▼                   ▼                   ▼
        TopicHeader          MetamindBlock         TaskBoard
        (selector)           (selector)            (selector)
```

Rules for clean updates:

1. **One subscription per session**, owned by `InspectorRail`, not by each card. Cards select
   slices from the store.
2. **Revision guard.** `applyStatusFrame(state, frame)` returns state unchanged if
   `frame.revision <= state.revision`. This makes duplicate frames and reconnect replays harmless.
3. **Section-level identity.** Each section has a `revision`/`updated_at`; the reducer replaces
   only changed sections. React memoized selectors (`useShallow` or explicit equality) prevent a
   kernel update from re-rendering the topic header.
4. **No loading flash.** The store keeps the last good view while reconnecting; a small "stale"
   badge appears after `2 × heartbeat` (20 s), not a skeleton. On first mount, one full GET
   provides data before the SSE opens, so there is no empty frame.
5. **Batched frame application.** Frames that arrive in the same animation frame are coalesced:
   `queueMicrotask`/`requestAnimationFrame` applies only the highest revision. The UI never
   renders an intermediate state that is immediately replaced.
6. **Stable keys.** Topic nodes keyed by `TopicId`; tasks by `TaskId`; operations by event id;
   trace events by `event_id`. No index keys.
7. **Error isolation.** If one section is missing, `SectionBoundary` renders an `UnavailableRow`;
   the rest of the panel remains live.
8. **Pause when hidden.** The status stream also pauses when the browser tab is hidden and
   reconnects with `from=revision` on visibility. The backend keeps the last snapshot, so the
   reconnect is cheap.
9. **Testing hooks.** `data-revision`, `data-section`, and `data-updated-at` attributes make
   Playwright assertions deterministic.

### 10.5 File map

```
frontend/src/
  components/inspector/
    inspector-rail.tsx
    inspector-header.tsx
    status-panel.tsx
    topic-header.tsx
    conversation-block.tsx
    metamind-block.tsx
    task-board.tsx
    agent-runs.tsx
    operation-list.tsx
    variables-block.tsx
    unavailable-row.tsx
    trace-viewer.tsx
    trace-event-row.tsx
    trace-tool-card.tsx
    trace-turn-header.tsx
  lib/api/
    inspector.ts            // fetchStatus, statusStreamUrl, fetchRuns, fetchTrace
    agentstream.ts          // event parsing and normalization
  lib/store/
    inspector.ts            // open, width, tab, selected run
    status.ts               // statusReducer, applyStatusFrame, selectors
  lib/wire/
    inspector.ts            // TS types mirroring the Rust DTOs
  routes/workspace.tsx      // compose InspectorRail beside WorkspaceSplit
```

`WorkspaceSplit` itself is unchanged except for a wrapper that reserves the rail width. The
inspector is outside the two-pane group, so resizing the document/chat split does not affect it.

---

## 11. Wire contracts

### 11.1 Status view (abridged)

```json
{
  "revision": 42,
  "sessionId": "01J…",
  "generatedAt": 1763000000000,
  "topic": {
    "main": { "id": "01A", "label": "release 2.0", "status": "active" },
    "path": [
      { "id": "01A", "label": "release 2.0", "status": "active" },
      { "id": "01B", "label": "auth migration", "status": "active" },
      { "id": "01C", "label": "token refresh", "status": "active" }
    ],
    "breadcrumb": "release 2.0 › auth migration › token refresh",
    "currentRefs": [{ "kind": "file", "id": "src/auth/refresh.rs", "verified": true }],
    "intent": "make the refresh test deterministic",
    "openQuestions": ["Do we support clock skew?"],
    "nextActions": ["run auth test 20×", "inspect seed"],
    "updatedAt": 1763000000000,
    "origin": "cm"
  },
  "project": { "id": "01P", "name": "Metamind console v2", "status": "active" },
  "conversation": {
    "turn": { "id": "01T", "state": "streaming", "phase": "act", "model": "auto/remote", "elapsedMs": 4200 },
    "turnsCompleted": 7,
    "contextRevision": 42,
    "loop": { "mode": "continue_until_done", "round": 3, "maxRounds": 24, "remainingMs": 582000 }
  },
  "metamind": {
    "kernel": {
      "state": "ok",
      "services": [
        { "name": "sqlite", "state": "ok" },
        { "name": "graph", "state": "ok" },
        { "name": "eventlog", "state": "ok" },
        { "name": "cme", "state": "ok", "reason": "conversation engine enabled" },
        { "name": "llm", "state": "ok" }
      ],
      "uptimeMs": 3600000,
      "eventHead": "01J…",
      "auditHead": "01J…"
    },
    "being": { "version": "3", "goals": 6, "voice": { "warmth": 0.72, "directness": 0.64 } },
    "budgets": [{ "kind": "meta_analysis", "limit": 100, "spent": 12 }],
    "memory": { "episodic": 1284, "segments": 37, "projectUpdatedAt": 1762999000000 },
    "llm": { "model": "auto/remote", "calls": 14, "cacheHitRate": 0.74 },
    "cme": { "regions": [{ "name": "proactive", "state": "idle", "transitions": 12 }] }
  },
  "tasks": {
    "counts": { "todo": 3, "doing": 1, "blocked": 1, "done": 4 },
    "doing": [{ "id": "01K", "title": "Wire inspector SSE", "priority": 80, "dueAt": 1763… }],
    "upcoming": [{ "id": "01L", "title": "Trace viewer event grouping", "priority": 70 }],
    "blocked": [{ "id": "01M", "title": "Promote agent crate", "reason": "waiting on review" }],
    "unprojected": []
  },
  "operations": {
    "recent": [
      { "id": "01E", "tool": "agent.continue", "target": "01R", "ok": true, "durationMs": 1200, "at": 1763…, "summary": "running, turn 2" }
    ],
    "active": []
  },
  "agents": {
    "active": [
      { "runId": "01R", "kind": "hermes", "status": "running", "turn": 2, "task": "Reproduce failing test", "lastMessage": "test passes with seed 1; fails with seed 42" }
    ],
    "recent": []
  },
  "variables": {
    "featureFlags": { "cme": true, "agentstream": true, "proactive": false },
    "preferences": { "verbosity": 0.2, "technical_depth": 0.6 },
    "counters": { "turns": 7, "toolCalls": 18, "agentTurns": 2, "cacheHits": 11 },
    "limits": { "rounds": 24, "toolSteps": 48, "wallMs": 600000 }
  }
}
```

### 11.2 Trace event

```json
{
  "eventId": "01X",
  "sessionId": "agent/01R",
  "turnId": "1",
  "timestamp": 1763000000000,
  "eventType": "agent_turn",
  "payload": {
    "schema": "mm.agent.turn.v1",
    "runId": "01R",
    "turn": 1,
    "role": "agent",
    "text": "I'll inspect the test file first.",
    "toolCalls": [
      { "name": "read_file", "arguments": "{\"path\":\"src/auth/refresh.rs\"}", "ok": true, "durationMs": 4 }
    ],
    "usage": { "promptTokens": 1200, "completionTokens": 180 }
  },
  "provenance": { "source": "hermes", "model": "hermes-1" }
}
```

The trace viewer never invents fields. A missing field renders as an em dash or an explicit
"unknown", not a plausible default.

---

## 12. Implementation plan

Each phase is independently promotable. Later phases depend on the contracts in earlier phases,
not on their internals.

### Phase 0 — Contracts and scaffolding

**Deliverables**

- `mm-data/src/conversation.rs`: shared `TopicRef`, `RefKind`, `ConversationContext`, `Topic`,
  invariants, mutation methods, unit tests.
- `mm-cme/src/context.rs`: re-export the state types and add the session-facing mutation API.
- `mm-cme/src/prompt.rs`: `PromptPacket` struct and renderer with golden tests.
- `mm-data/src/work.rs`: `WorkStore` API and graph constants (no routes yet).
- `mm-agent/src/lib.rs`: `AgentService`, `AgentRun`, `AgentTurnResult`, `AgentBackend` traits.
- `mm-serve/src/status.rs`: view-model structs and serde tests.
- Ontology additions: `mm:Topic`, `mm:TopicRef`, `mm:WorkItem`, `mm:AgentRun`,
  `mm:ConversationContext` in `ontology/mm.ttl` + shapes in `ontology/shapes/mm-shapes.ttl`.
- Feature flag `MM_CM_V2` (default off), so the old path remains the fallback.

**Exit criteria:** `cargo test -p mm-cme -p mm-data -p mm-agent` passes; the ontology validates
with the existing graph validation; no route or prompt changes yet.

### Phase 1 — Conversation state end to end

**Deliverables**

- `ConversationStore` save/load with revision conflict; event append.
- `WorkStore` create/update/list/upcoming; task transition rules.
- Session bootstrap: seed a root topic when absent; load project/work/continuity.
- `ContextService` in `mm-serve` that the future tools call.
- Event types `ContextUpdated`, `WorkUpdated`; agentstream replay test.
- Status section sources for topic/tasks (not yet streamed).

**Exit criteria:** a test can create a session, set a topic path, create a task, reload both from
RDF, replay the event log, and get the same revision/state; stale revisions return `CONFLICT`.

### Phase 2 — The eight-tool surface

**Deliverables**

- `mm-serve/src/cm_tools.rs` replaces `ConsoleTools` in `cme_executor`.
- `context`, `project`, `task`, `memory`, `being`, `kernel`, `focus` implementations.
- Strict argument parsing; `ToolResultEnvelope`; structural truncation; per-tool timeouts;
  idempotency keys; action/agentstream logging.
- `TurnLoopPolicy` with `ContinueUntilDone`; loop directive messages.
- Remove `catalog.open` and group admission from the CM path.
- Update parity test to scope the CLI-name check to the admin catalog.

**Exit criteria:** a fake model that calls `context.set_main`, `task.create`, and then answers
completes a turn with exactly those tool results; invalid JSON returns `INVALID_ARGUMENTS`; a
stale context update returns `CONFLICT`; no CM tool schema mentions CLI-only operations.

### Phase 3 — `mm-agent` and Hermes

**Deliverables**

- `mm-agent/src/registry.rs`, `hermes.rs`, `summarizer.rs`, `agentstream.rs`.
- Hermes OpenAI client with SSE + non-streaming fallback; `/models` discovery.
- Separate agentstream session creation and event mapping.
- `AgentService` wired into `AppState`.
- A fake Hermes server for tests (axum) and a recorded transcript fixture.
- The `agent` tool: `start`, `continue`, `status`, `cancel`, `close`; `link_agent` on tasks.

**Exit criteria:** against the fake server, a run produces turn 1, turn 2, and a terminal turn;
the agentstream has a distinct `agent/<run_id>` session with the expected event types; a long
turn is summarized and the raw event is retained; the CM loop continues until `done`; cancellation
is honored; an unreachable service returns `UNAVAILABLE` with the URL/redacted reason.

### Phase 4 — Pi and the agent abstraction

**Deliverables**

- `mm-agent/src/pi.rs` adapting `PiClient`; workspace policy; tool allowlist; event mapping.
- Recorded Pi session fixtures; `pi.ingest` operator import preserved.
- Backend selection in the `agent` tool (`kind: pi`).
- Run recovery on boot (stale runs → failed with reason).

**Exit criteria:** a recorded Pi session replays through the same `AgentTurnResult` contract; a
missing binary returns `UNAVAILABLE`; a workspace escape returns `FORBIDDEN`; no shell tool is
advertised to Pi.

### Phase 5 — Personal prompt and continuity

**Deliverables**

- `PromptBuilder` reads being, user model, relationship, affect, memory continuity, context,
  work, agents, recent transcript.
- Stable/volatile section split for cache reuse.
- Identity invariants rendered verbatim; missing sources rendered as `unavailable`.
- Session bootstrap loads previous `ConversationContext` and project memory.
- Golden prompt snapshots for empty, populated, and degraded states.

**Exit criteria:** a snapshot test proves the prompt contains the operator's preferred name, the
current topic path, the project's open commitments, and the being's invariants, and that a closed
being store renders a reason instead of silently omitting the section. A manual review confirms
the voice is not the generic "Metamind assistant".

### Phase 6 — Status service and routes

**Deliverables**

- `StatusService` with dirty sections, 100 ms coalescing, revision, snapshot cache, broadcast.
- Routes: full status, status SSE, operations, agents list/detail/events/stream/cancel.
- Health/CME/being/task/agent sources wired.
- Backpressure, slow-client disconnect, resync.
- Prometheus counters and latency histograms.

**Exit criteria:** a mutation produces a status frame within 250 ms; 100 rapid mutations produce
at most 10 frames, with the final revision correct; a reconnect with `from` receives either the
delta or a full resync; a closed store yields an `unavailable` section while the rest updates; a
slow client is disconnected and recovers.

### Phase 7 — Inspector status UI

**Deliverables**

- `InspectorRail`, header, store, status reducer, all section components.
- `useStatusStream` with visibility pause, revision guard, RAF coalescing, stale badge.
- Responsive rail/drawer/sheet behavior; keyboard shortcuts; persistence.
- Wire types and fixture tests; Vitest for reducer/selectors; Playwright for live updates.

**Exit criteria:** with a fake backend, the panel shows the topic at the top, all required
sections, and updates on revisions without a loading flash; duplicate/stale frames do not change
the DOM; a hidden tab pauses and resumes; the panel is keyboard-navigable; no section renders a
fabricated value.

### Phase 8 — Trace viewer

**Deliverables**

- `TraceViewer`, turn headers, role rows, tool cards, summary/raw toggle, filters, follow.
- `useAgentTrace` stream, only while the tab is active and visible.
- Virtualization; stable keys; empty/error states; cancel action.
- Agentstream session list and run selector.

**Exit criteria:** a 5,000-event recorded trace scrolls at 60 fps; turn grouping and summaries
match the fixture; "Show raw" reveals the exact text; scrolling up pauses follow and "jump to
live" resumes; closing the tab closes the EventSource; cancelling a run is reflected in the
status panel.

### Phase 9 — Hardening, migration, documentation

**Deliverables**

- Enable `MM_CM_V2` by default after a dual-run comparison.
- Remove the old CM tool bridge and dead group-admission code (or keep behind the flag for one
  release).
- Update `README.md`, `infodocs/components-and-status.md`, `infodocs/open-gaps.md`, and the
  design index.
- Operator guide: what the eight tools do, how to configure Hermes/Pi, how to read the inspector.
- Cost/latency report from the dual-run.

**Exit criteria:** the full workspace test suite passes; the old flag-off path still boots; the
new path is the default; a rollback is a flag flip; the docs describe the actual behavior.

---

## 13. Testing strategy

### Rust

| Layer | Tests |
|---|---|
| Context | Tree invariants, cycle rejection, drill/back/close, revision conflict, ref validation, auto-seed |
| Work | Status transitions, blocked dependencies, terminal immutability, upcoming sort, agent link idempotency |
| Tools | Schema validation per action, strict JSON, envelope shape, structural truncation, timeout, idempotency, forbidden SPARQL keywords |
| Loop | Continue-until-done with a scripted model, non-terminal agent forces another round, budget → paused, context-reminder injection |
| Agent | Fake Hermes SSE (content + tool calls + `[DONE]`), non-streaming fallback, long-turn summarization, summary failure fallback, cancel, stale-run recovery, workspace escape |
| Pi | Recorded session replay, missing binary, abort, event mapping |
| Status | Dirty-section recompute, coalescing, revision monotonicity, resync, slow-client disconnect, unavailable sections |
| Prompt | Golden snapshots for empty/populated/degraded; invariant presence; no secret leakage |
| Wire | serde round-trips for every DTO; additive event-type compatibility |

### Frontend

| Layer | Tests |
|---|---|
| Reducer | Revision guard, section merge, stale reconnect, RAF coalescing |
| Components | Topic breadcrumb, task sorting, operation list, unavailable rows, trace grouping |
| Integration | Playwright with a fake status/trace server: live update, no flash, pause/resume, tab switch, cancel |
| Accessibility | Roles, labels, focus order, keyboard shortcuts, reduced motion |
| Performance | 5,000-event trace virtualization; status frame budget |

### Contract tests

- Generate TS types from the Rust DTOs (the repository already uses generated wire types where
  practical) and fail CI on drift.
- One fixture per event type and per tool envelope, shared by Rust and TS tests.

### Manual acceptance script

1. Ask the CM to plan a small project; see the topic and tasks appear in the panel.
2. Ask it to delegate a code task to Hermes; watch turn 1, turn 2, summary rows, and task status.
3. Interrupt, cancel, and resume; verify the status panel and trace agree.
4. Hide the tab and return; verify no stale data and no duplicate frames.
5. Close the graph store; verify the panel shows `unavailable` for the graph while the rest stays
   live.

---

## 14. Observability and metrics

New metrics (Prometheus names):

| Metric | Type | Purpose |
|---|---|---|
| `mm_cm_tool_calls_total{tool,code}` | counter | Tool reliability |
| `mm_cm_tool_duration_ms{tool}` | histogram | Latency |
| `mm_cm_loop_rounds` | histogram | Loop behavior |
| `mm_cm_paused_total{reason}` | counter | Budget/agent stalls |
| `mm_agent_runs_total{kind,status}` | counter | Agent outcomes |
| `mm_agent_turns_total{kind}` | counter | Turn volume |
| `mm_agent_turn_duration_ms{kind}` | histogram | Downstream latency |
| `mm_agent_summary_failures_total` | counter | Summarizer degradation |
| `mm_status_revisions_total` | counter | Update volume |
| `mm_status_recompute_ms{section}` | histogram | Status cost |
| `mm_status_subscribers` | gauge | SSE load |
| `mm_status_resync_total` | counter | Client lag |

Tracing spans:

- `cm.turn` → `cm.round` → `cm.tool` (with tool name, code, duration)
- `agent.start` → `agent.turn` → `agent.summarize`
- `status.recompute` (section list) → `status.publish`

Log records use the existing audit-chained logger for mutating tools and agent starts; raw agent
events go to agentstream. No secrets.

---

## 15. Security and permissions

- **Workspace confinement.** Agent roots are canonicalized; `..` and symlink escapes are refused;
  default roots are under `data/sandbox`.
- **No ambient shell.** The CM has no shell tool. Hermes/Pi only receive their configured tools;
  Pi's default set excludes shell.
- **Promotion is not delegated silently.** An agent may propose a change set, but the CM cannot
  call `promote`. Promotion remains an operator/gate action.
- **Secrets.** Hermes keys and Pi provider credentials come from the environment; they are
  redacted from agentstream, logs, tool results, and error messages. The redactor runs on every
  agent event payload before persistence.
- **Prompt injection.** Tool results and agent messages are untrusted content. The prompt wraps
  them in labelled blocks and instructs the CM not to follow instructions found inside tool
  output or agent output. The CM's tool policy is enforced server-side, not by the prompt.
- **SSE.** Status and trace streams are scoped to one session/run and use the existing console
  auth. No cross-session enumeration without the session id.
- **Data retention.** Agent transcripts may contain code and user data. They follow the existing
  agentstream retention; the operator can delete a run through the existing data-retention path
  (to be added as an operator action, not a CM tool).

---

## 16. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Hermes endpoint/model/streaming shape differs from OpenAI | Medium | Agent tool unusable | `/models` probe, non-streaming fallback, `UNAVAILABLE` with exact reason, fake-server contract tests, config-only adaptation |
| Summaries omit a critical detail | Medium | CM makes a wrong next decision | Structured summary schema, artifact/blocker preservation rules, raw event retained and linked, CM can request `agent.status` with raw tail |
| CM loops or burns tokens | Medium | Cost/latency | 24 rounds / 48 steps / 10 min caps, 100 ms coalescing unrelated, per-tool budgets, pause-with-remaining semantics, metrics |
| Context tree becomes noisy | Medium | Panel clutter | Cap depth at 8, cap nodes at 50, close/park semantics, auto-seed only, no auto-hierarchy |
| Status service loads the RDF store too often | Low | Slow UI | Dirty sections, cache, coalescing, no per-card queries |
| SSE fan-out with many clients | Low | Memory/latency | Per-session subscriber cap, slow-client disconnect, resync |
| Frontend re-renders whole panel | Medium | Flicker/jank | Section selectors, revision guard, stable keys, RAF coalescing, Playwright perf test |
| Tool surface still too small for an edge case | Medium | CM cannot act | `kernel` read-only query covers inspection; an operator can add a tool only through the change-set path; do not grow the surface ad hoc |
| Agent writes outside the workspace | Low | Data damage | Canonical root check, sandbox default, `FORBIDDEN`, action ledger |
| Old and new paths diverge | Medium | Inconsistent behavior | Feature flag, dual-run comparison, one shared context/work store, parity tests |
| Prompt bloat | Medium | Cost/latency | Stable prefix caching, token budgets per section, summarize continuity, drop oldest operations first |

---

## 17. Assumptions and open questions

**Assumptions**

1. Hermes exposes an OpenAI-compatible `/v1/chat/completions` (and ideally `/v1/models`) and
   accepts a bearer token. The exact port, model name, and whether it supports SSE must be
   confirmed before Phase 3. The design works with or without server-side session retention.
2. The operator wants the CM to *delegate* engineering actions, not perform them itself.
   Promotion remains gated.
3. The inspector is a single-operator surface, matching `CONSOLE_USER`.
4. The existing `mm-data` RDF engine and agentstream store are the persistence layer; no new
   database is introduced.
5. Pi remains available as an optional backend; its absence is reported, not worked around.

**Open questions for the operator**

1. What is Hermes's base URL, model name, and auth scheme on this machine? Is streaming enabled?
2. Should `agent.start` require confirmation for every run, or only outside the sandbox? This
   design defaults to "sandbox runs start immediately; outside roots require confirmation".
3. Should the inspector be per-session only, or should it also show global system health when no
   session is selected? This design scopes it to the active session and reuses `/api/health` for
   the shell.
4. Is a 10-minute wall clock per turn acceptable, or should long jobs always pause at 2 minutes
   and resume proactively?
5. Should completed agent raw traces be retained forever, or follow a shorter retention than the
   CM transcript?

None of these block Phase 0–2. Items 1 and 2 block Phase 3.

---

## 18. Acceptance criteria against the original complaints

| Complaint | Design response | Verified by |
|---|---|---|
| "Cannot properly use its tools to do anything" | Eight typed tools; strict args; typed envelopes; structural truncation; 24-round continue-until-done; per-tool timeouts; idempotency | Phase 2 tests; manual acceptance script |
| "Conversation loop does not feel personal" | Prompt packet with identity, relationship, affect, continuity, memory, project context; cross-session bootstrap; being invariants | Phase 5 golden snapshots; manual review |
| "Cannot do any project management" | `project` + `task` tools; `WorkStore`; upcoming/blocked/doing board; agent-task links | Phase 1/2 tests; panel acceptance |
| "Simplify/streamline internal metamind tools" | 69 CLI rows → 8 CM tools; admin catalog untouched; deprecation map | Schema/test review; parity scoped |
| "One single tool for external tasks" | `agent` only; no shell/url/http; Hermes and Pi behind one trait | Phase 3/4 tests |
| "Hermes via OpenAI API, separate agentstream session" | `mm-agent` Hermes client; `agent/<run_id>` session; per-turn persistence | Fake-server integration test |
| "Pi programmatic support" | `mm-agent` Pi adapter over `PiClient`; same turn contract | Recorded-session test |
| "Turn-by-turn, summarize long turns, pass result to CM, continue tools until done" | `AgentTurnResult`; `TurnSummarizer`; `ContinueUntilDone` policy and directives | Loop tests; prompt snapshots |
| "Topic explicit, hierarchical, text or backend data" | `Topic` tree, `TopicRef`, revisioned store, panel header | Context tests; UI tests |
| "Live status showing metamind internals and conversation state" | `ConversationStatusView`, dirty-section service, SSE revisions | Status tests; Playwright |
| "Upcoming tasks, recent operations, internal variables" | `TaskBoardView`, `OperationsView`, `VariablesView` | UI fixture tests |
| "Refresh processes data and makes clean updates" | One backend composition, revision guard, coalescing, section selectors, no loading flash | Reducer tests; Playwright |
| "Agentstream trace viewer in the right column, if viewed" | Trace tab; turn grouping; summaries/raw; subscribe only when visible | Phase 8 tests |

---

## Appendix A — New and changed files

### New

```
crates/mm-agent/Cargo.toml
crates/mm-agent/src/lib.rs
crates/mm-agent/src/error.rs
crates/mm-agent/src/registry.rs
crates/mm-agent/src/service.rs
crates/mm-agent/src/hermes.rs
crates/mm-agent/src/pi.rs
crates/mm-agent/src/summarizer.rs
crates/mm-agent/src/agentstream.rs
crates/mm-agent/src/workspace.rs
crates/mm-agent/tests/hermes_fake.rs
crates/mm-agent/tests/pi_recorded.rs

config/agents.toml                         Hermes/Pi endpoint + model (no secrets)

crates/mm-cme/src/context.rs
crates/mm-cme/src/prompt.rs
crates/mm-cme/src/loop_policy.rs

crates/mm-data/src/conversation.rs
crates/mm-data/src/work.rs

crates/mm-serve/src/cm_tools.rs
crates/mm-serve/src/status.rs
crates/mm-serve/src/routes/inspector.rs
crates/mm-serve/src/routes/agents.rs

frontend/src/components/inspector/*.tsx
frontend/src/lib/api/inspector.ts
frontend/src/lib/api/agentstream.ts
frontend/src/lib/store/inspector.ts
frontend/src/lib/store/status.ts
frontend/src/lib/wire/inspector.ts
```

### Changed

```
crates/mm-cme/src/lib.rs                 export context/prompt/loop_policy
crates/mm-cme/src/executor.rs            loop policy, prompt packet, envelope handling
crates/mm-cme/src/session.rs             context field, previous-context bootstrap
crates/mm-cme/src/planner.rs             LoopMode selection
crates/mm-cme/Cargo.toml                 depend on mm-agent (narrow trait)
crates/mm-data/src/lib.rs                export conversation/work
crates/mm-data/src/agentstream.rs        new event types
crates/mm-data/src/rdf.rs                graph/IRI helpers
crates/mm-serve/src/state.rs             AgentRegistry, StatusService, new executor
crates/mm-serve/src/cme_tools.rs         remove; replaced by cm_tools.rs
crates/mm-serve/src/routes/mod.rs        inspector and agent routes
crates/mm-serve/src/feature_flags.rs     MM_CM_V2, agent flags
crates/mm-serve/src/lib.rs               module declarations
crates/mm-serve/Cargo.toml               mm-agent
Cargo.toml                               workspace member mm-agent
ontology/mm.ttl                          Topic, WorkItem, AgentRun, ConversationContext
ontology/shapes/mm-shapes.ttl            shapes for the above
frontend/src/routes/workspace.tsx        mount InspectorRail
frontend/src/lib/store/pane.ts           inspector open/width/tab
frontend/src/index.css                   inspector tokens (if any new)
README.md                                operator guide
infodocs/components-and-status.md        new components and status
infodocs/open-gaps.md                    what remains
design/DESIGN_INDEX.md                   index entry for this document
```

### Not changed

- `crates/mm-cli` and the admin controller catalog continue to expose the full CLI surface.
- The promotion gate, sandbox, change-set, and audit paths are untouched.
- The voice region, proactive region, and memory compression pipeline keep their current
  contracts; the CM v2 prompt consumes their outputs rather than replacing them.
- The existing `/api/chat` fallback path remains available behind the `MM_CME_ENABLED` flag.

---

## Appendix B — Example end-to-end interaction

**Operator:** "The auth refresh test is flaky. Find out why, fix it, and track the work."

**Turn 1**

1. CM `context.set_main` → "Fix flaky auth refresh test" (root).
2. CM `project.bind` → "Metamind console v2" (or creates a project if none).
3. CM `task.create` → "Reproduce flaky auth refresh test", priority 80, status doing.
4. CM `agent.start(kind=hermes, task="Reproduce …", acceptance_criteria=[...],
   task_id=<task>)`.
   - Run `01R`, agentstream `agent/01R`, turn 1: Hermes reads the test, runs it, reports a
     seed-dependent failure. The turn is short, so it is passed verbatim.
5. CM `agent.continue(run_id, "Isolate the seed and report the failing assertion.")`
   - Turn 2 is long; the summarizer produces a headline and artifacts; the CM receives the
     compact result.
6. The loop sees `status: running`; it continues. The turn's budget is 10 minutes, so it can
   perform several more turns without ending the conversation.

**Turn 2**

7. CM `context.drill` → "seed 42 / clock skew".
8. CM `task.create` → "Fix clock skew in refresh test", depends_on the reproduction task.
9. CM `agent.continue(run_id, "Implement the controlled clock and run the auth suite.")`
10. Hermes edits files, runs tests, returns `done` with artifacts.
11. CM `task.update` → reproduction done, fix doing.
12. CM `context.set_next_actions` → "review diff", "run full suite".
13. CM answers: "Found it — the test uses the wall clock; seed 42 crosses the boundary. The fix is
    in `src/auth/refresh.rs`; the full auth suite passes. I've tracked both tasks."

**Panel during the interaction**

- Topic breadcrumb updates at steps 1, 7, and 12.
- Tasks move todo → doing → done.
- Agents shows the Hermes run and its latest summary.
- Operations lists each tool call with duration.
- Internal variables show context revision, loop round, and agent turn count.

**Trace tab**

- Turn 1: read test, run test, failure output.
- Turn 2: edit file, run suite, pass.
- Each turn has a summary row and a "Show raw" expansion.
- Cancelling mid-run closes the trace cleanly and marks the run `cancelled`.

This is the target behavior: the operator sees a coherent conversation, a visible plan, real
progress from a real worker, and a live system view that does not lie.
