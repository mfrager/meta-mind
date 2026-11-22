# Conversation Manager v3 — Conversation, Data, and Metamind Tools with the Live Inspector

> **Status:** proposed design · **Date:** 2026-11-22 · **Scope:** `mm-cme`, `mm-serve`, `mm-data`,
> `mm-store-graph`, `mm-search` (new), `mm-pi`, a new `mm-agent` crate, the console frontend, and
> the metamind ontology.
>
> **This document is a plan, not an implementation.** It defines the target contracts and the
> build order. Nothing here changes production behavior until the phases in §14 are promoted
> through the existing change-set → sandbox → gate → promotion path.

---

## 0. Reading this document

| If you want to know… | Read |
|---|---|
| Why the current conversation loop feels broken | §2 |
| What the conversation manager should become | §3–§5 |
| The fourteen tools and their three families | §6 |
| How automatic state and automatic data capture work | §5.4 and §6.9 |
| What the model sees on every call | §5.5 |
| How Oxigraph, RDF/XML, Tantivy, schemas, and identity names fit together | §7 |
| How planning, inner thought, and proactive action work | §8 |
| How Hermes and Pi are called | §9 |
| How the loop keeps working until the task is done | §10 |
| How the right-panel status is computed and streamed | §11 |
| What the status panel and agent trace viewer look like | §12 |
| The contracts on the wire | §13 |
| What to build, in what order | §14 |
| How it is verified | §15 |
| What could go wrong | §18 |

Terms used throughout:

- **CM** — the conversation manager: the LLM plus the `mm-cme` session/turn machinery that drives
  it. The CM is the only model the operator talks to.
- **Downstream agent** — Hermes or Pi. It is a separate, long-running worker with its own
  conversation. The CM delegates external work to it.
- **`agent.run`** — the single external-work tool. It takes natural-language input, starts or
  continues one downstream agent, and returns one compact turn result.
- **Conversation state** — the small, explicit internal state the CM owns: current topic, topic
  depth, and a bounded map of status variables.
- **Data family** — `data.create`, `data.update`, `data.query`, `data.delete`, `data.search`,
  `data.schema`, `data.namespace`: the high-level path to ontology instance data, schemas, and
  full-text search.
- **Metamind family** — `metamind.plan`, `metamind.think`, `metamind.act`: durable planning, a
  private inner-thought sub-agent, and allowlisted proactive action.
- **State block** — the canonical, always-injected rendering of the conversation state plus the
  active agent, data, plan, and health projections. It is rebuilt from the stores before every
  model call.
- **Inspector** — the auxiliary right-hand console rail that shows conversation status, knowledge
  health, plans, actions, and agent traces. It is instrumentation, not a third workspace document
  pane.

---

## 1. Executive summary

The current conversation engine is a sound state machine wrapped in the wrong conversation
surface. The operator sees seven failures:

1. **The tool surface is a CLI mirror, not a conversation surface.** `ConsoleTools` advertises
   every row of `controller::catalog::table()` — currently 69 tools — and expects the model to
   discover the right one from names like `sandbox_tool.run`, `focus.patch`, and `design.show`.
   The catalog's group heuristic is keyword guessing, malformed JSON is silently coerced to `{}`,
   results are truncated mid-JSON at 16 KiB, and the executor permits only four tool rounds and
   eight steps. A model cannot reliably operate that surface, so it appears to "do nothing".
2. **The CM cannot manage state it does not represent.** There is no current topic, no topic
   depth, and no compact working-state dictionary reachable from the loop. `SessionContext`
   carries `project_id` and `previous_topics`, but neither is injected into the prompt.
3. **The CM has no general-purpose worker.** `pi.run` exists as an admin row but there is no
   first-class agent session, no turn-by-turn delegation, and no second agentstream for the
   worker conversation. External work has no seam.
4. **The loop is impersonal because its prompt is a constant.** `TurnExecutor::messages()` emits
   the same three sentences on every turn. Being personality, the operator model, relationship
   state, affect, memory continuity, project context, and prior commitments never reach the
   model. The model also cannot see its own conversation state, because no such state exists.
5. **The console cannot see the system it is driving.** The only live surface is a state board
   polled on the health cadence; there is no conversation-state stream, no operation ledger in
   the panel, and no viewer for agentstream traces.
6. **Knowledge is stored but not managed.** The RDF store has typed data and a JSON-payload
   compatibility layer, but the CM has no high-level way to create, update, query, delete, or
   search ontology instances. There is no schema catalog, no namespace registry, no full-text
   index over entities, and no automatic prompt policy that turns what the operator says into
   durable facts. Users are partly modelled as FOAF people, but there is no standard identity
   naming contract the CM can follow.
7. **Metamind cannot plan, think privately, or act proactively as first-class operations.**
   Proactive plugins exist, but the CM cannot invoke them. There is no durable plan object, no
   inner-thought sub-agent that can inspect data before the outer model replies, and no
   immediate/scheduled proactive action surface.

This revision keeps the four conversation tools and adds **two bounded job families** — seven
data tools and three metamind tools — for **fourteen tools and one always-visible state block**:

```
operator
   │
   ▼
┌──────────────────────────── CM (mm-cme) ────────────────────────────┐
│ stable prompt: identity · relationship · affect · continuity        │
│ state block:  topic · depth · status · agents · data · plan · health│
│ transcript · operator input                                         │
│                                                                      │
│ conversation (4): agent.run · conversation.status · topic · depth   │
│ data (7): data.create · update · query · delete · search ·          │
│           schema · namespace                                         │
│ metamind (3): metamind.plan · metamind.think · metamind.act          │
│                                                                      │
│ conversation ──► conversation state (RDF + event log)                │
│ data         ──► DataBroker ──► Oxigraph/RocksDB + Tantivy           │
│ metamind.plan──► PlanService ──► plan graph                          │
│ metamind.think► ThinkService ──► read-only data sub-session          │
│ metamind.act ──► ActionService ──► immediate/scheduled proactive     │
│ agent.run    ──► mm-agent ──► Hermes (OpenAI API) or Pi (RPC)        │
│                       └──► separate agentstream session per run      │
└──────────────────────────────────────────────────────────────────────┘
   │                                         ▲
   │ status events                           │ live view
   ▼                                         │
StatusService (mm-serve) ──SSE──► Inspector (status + trace tabs)
```

The important properties:

- **Fourteen tools, three bounded families.** The conversation core stays at four. Data and
  metamind are job families, not a revival of the 69-row CLI catalog. There is no `graph.query`,
  no `catalog.open`, and no arbitrary scheduling or shell tool.
- **The state is tiny and explicit.** `topic` is a plain-language label; `depth` is its drill
  level; `status` is a bounded, typed dictionary of conversation variables. There is no topic
  tree, no work-item database exposed to the CM, and no second state model.
- **Conversation state is inferred without trigger instructions.** The model is not told when to
  call `conversation.topic`, `conversation.depth`, or `conversation.status`. It sees the current
  state and infers from the conversation.
- **Data management is induced by a durable prompt policy, not per-turn scripts.** The CM is told
  to treat durable facts as ontology data, search before creating, update rather than duplicate,
  and never store secrets. It decides when a fact is durable; there is no "the user said name,
  call `data.create`" reminder.
- **The state block is always present and always current.** It is rebuilt from the stores before
  every model call, including every tool round. It is compressed JSON, not a prose summary, and
  it is lossless with respect to the conversation state: every key and value is present. Writes
  that would exceed the size cap are rejected, so the prompt never truncates state. Data, plan,
  and action projections are derived and bounded.
- **One dataset, one search index, one schema catalog.** All managed instance data lives in the
  Oxigraph/RocksDB dataset and is mirrored into Tantivy after every insert, update, or delete.
  RDF is authoritative; the index is rebuildable. The RDF/XML snapshot is a complete portable
  instance file, not a second live store.
- **Metamind has real, bounded operations.** `metamind.plan` creates/updates durable versioned
  plans; `metamind.think` runs a read-only inner sub-agent and returns evidence to the outer CM;
  `metamind.act` runs or schedules an allowlisted proactive action.
- **`agent.run` is the only external-work door.** It accepts natural language, starts or
  continues one downstream agent turn, writes the raw trace to a separate agentstream session,
  and returns a compact result to the CM.
- **The inspector is a live projection, not a second source of truth.** The backend composes it
  from the same state, data health, plans, actions, and event streams the CM uses, with revisions,
  coalescing, and SSE.

The rest of this document specifies the state model, the exact tool schemas, the data and
ontology substrate, the search index, the automatic capture policy, planning/thinking/action, the
agent turn contract, the live view model, the UI anatomy, and a phased build with testable exit
criteria.

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
| `crates/mm-serve/src/controller/admit.rs` | Admission is group-based, not need-based | The CM has to reason about metamind's internal organization instead of the operator's task |
| `crates/mm-serve/src/controller/exec.rs` | One dispatch function for CLI-shaped operations | CLI semantics leak into conversation semantics (`id` required, fixture rules, `apply` flags) |

The root cause is not the number of tools by itself. It is that the tool surface encodes the
CLI's structure rather than the conversation's jobs. The CM needs four conversation jobs:

- **delegate external work** — `agent.run`;
- **say what the conversation is about** — `conversation.topic`;
- **say how deep it has drilled** — `conversation.depth`;
- **keep its working variables true** — `conversation.status`.

It also needs two higher-level families that the CLI never expressed as jobs: **manage knowledge**
and **use its own mind deliberately**. Those are the `data.*` and `metamind.*` families. They are
not 69 rows in disguise; they are one high-level tool per job, with a broker underneath.

### 2.2 The loop is impersonal and blind to its own state

`TurnExecutor::messages()` currently produces a constant prompt. There is no state block, no
topic, no depth, and no status variables. The model cannot answer "what are we doing?" from
anything except the raw transcript.

The current prompt also has no relationship context: preferred name, expertise, affect, open
commitments, and project continuity are not injected. The result is a generic assistant voice
even when the being and memory stores contain a specific one.

### 2.3 There is no external work seam

The controller catalog has `pi.run`, `pi.ingest`, `sandbox.run`, `changeset.new`, and similar
rows. These are admin operations, not an agent session. There is no run registry, no separate
agentstream, no per-turn summary, and no way for the CM to say "continue that run".

### 2.4 There is no live inspector

The console's live surface is a state board polled on the health cadence. It does not show the
conversation topic, the status variables, the active operations, or the agent trace. The operator
cannot tell whether the CM is working, waiting, or lying.

### 2.5 Knowledge is not managed as ontology data

The store has Oxigraph, typed graphs, and a FOAF user profile, but the conversation engine cannot
use them as a knowledge system:

- There is no `data.create`/`update`/`query`/`delete`/`search` surface. `rdf::upsert` writes a
  JSON payload beside `rdf:type` and `mm:name`; it is a record store, not a typed instance API.
- There is no full-text index over entities. `memory` has embedding search for compressed
  segments, which is a different corpus and a different retrieval model.
- There is no schema catalog or namespace registry, so the model cannot ask "what types exist?"
  or "which prefix owns this IRI?".
- There is no prompt policy that turns durable operator facts into graph writes. Identity details
  live in a user record and a prompt packet, not in a self-maintaining ontology.
- The identity spelling is split: users are `urn:metamind:user:<id>` and FOAF `Person`, while the
  being is `https://metamind.dev/data/<ULID>`. There is no canonical table the CM can follow.

### 2.6 Metamind has no first-class planning, thinking, or proactive action

- `ProactiveEngine` and six proactive plugins exist, but they are not reachable from the CM. There
  is no immediate/scheduled action tool and no durable action record.
- There is no `mm:Plan` object or plan version store. Plans, when they exist, are prose in a
  message or loose entries in the conversation status dictionary.
- There is no inner-thought sub-agent. A question that needs several data reads either bloats the
  outer transcript or is answered without evidence.

### 2.7 What this revision changes

The previous revision of this plan proposed eight tools and a full project/task store. That was
still too much surface, but the answer is not "four tools forever". This revision keeps the four
conversation tools, adds two bounded job families (data and metamind), and keeps the useful parts
of the previous plan — Hermes/Pi delegation, separate agentstreams, the live inspector, and the
continue-until-done loop.

---

## 3. Design principles

1. **Three bounded families, fourteen tools total.** The conversation core stays at four. The data
   family is one tool per knowledge job; the metamind family is one tool per mental action. New
   needs go into a family only by a deliberate schema change, never by exposing the CLI catalog.
2. **One external door.** All outside work goes through `agent.run`. The CM cannot call a shell,
   a browser, an arbitrary HTTP endpoint, or a promotion operation directly. This makes
   authorization, logging, cancellation, and replay tractable.
3. **The model owns conversation state; it is not scripted.** The CM decides when the topic,
   depth, or status variables need to change. The executor does not inject per-turn trigger rules,
   reminders, or "you must call X" messages. Tool descriptions state what a tool does, not when
   to call it.
4. **State is canonical, bounded, and always injected.** The full conversation state is
   serialized into a deterministic block before every model call. It is not paraphrased. It is
   not truncated at prompt time. If a write would exceed the cap, the write fails.
5. **Data is managed as ontology, automatically and implicitly.** Durable facts become typed RDF
   instances. The stable prompt carries a capture policy, not per-turn triggers. The broker
   validates, routes, writes, and indexes; the model never writes raw quads or SPARQL.
6. **High-level tools over one low-level broker.** `data.*` intents map to schema resolution,
   namespace expansion, RDF mapping, graph routing, validation, and Tantivy documents in one
   place. There is no second data path for the CM.
7. **One dataset, one search index, one schema catalog.** RDF is authoritative; Tantivy is
   rebuildable; the schema catalog and namespace registry are themselves RDF. Every insert,
   update, and delete keeps the index in sync through a durable outbox.
8. **Metamind actions are bounded.** Plans are versioned objects; inner thought is read-only and
   private; proactive action is an allowlisted catalog with immediate or scheduled execution.
9. **A turn is not done until the job is done or the budget is spent.** The loop keeps calling
   tools — including repeated `agent.run`, plan, think, and data calls — until the CM declares
   completion or a hard deadline arrives. A budget stop is reported as *paused with remaining
   work*, never as success.
10. **The backend derives the view once; the UI renders revisions.** The status service owns
    composition, coalescing, and ordering. The browser never polls five endpoints and stitches
    the result together.
11. **Every mutation is an event.** Topic, depth, status, data writes, plan versions, actions,
    agent turns, and tool operations append to the event log. State is reconstructible; the
    inspector can replay.
12. **Fail visibly, with a reason and a next step.** No silent `{}`. No "tool failed" without a
    code. No fake success, no invented fact, no silent index loss.

### 3.1 Alternatives considered

| Decision | Alternative | Why the alternative was rejected |
|---|---|---|
| Four conversation tools plus two job families | Keep 69 tools and add progressive disclosure | Disclosure still asks the model to reason about metamind's internal organization; the near-duplicate names remain, and every extra schema costs context on every turn |
| Four conversation tools plus two job families | The previous eight-tool surface (`context`, `project`, `task`, `memory`, `being`, `kernel`, `focus`, `agent`) | Still too many vague concepts. The new families are job-shaped: seven data verbs and three mental actions, each with one clear schema |
| High-level `data.*` over a broker | Expose `graph.query` / raw SPARQL | Raw SPARQL pushes graph structure, prefixes, and write safety onto the model. The broker can validate, route, index, and audit; raw SPARQL remains an admin tool |
| One Oxigraph dataset with named graphs | One store per graph or per project | The operator asked for one main data store. One dataset gives cross-graph queries and one backup; named graphs preserve ownership. Separate stores multiply consistency and backup problems |
| Tantivy full-text index | SQLite FTS5 or embedding search | The operator named Tantivy. It is a real Rust search engine, supports facets/boosts, and is independent of the RDF store. SQLite FTS would couple the index to a second database; embeddings are for memory, not lexical entity search |
| A schema catalog in RDF | Hard-coded prefixes and ad-hoc SHACL files | A catalog lets the CM list schemas, validate, and register new OWL without redeploying. Hard-coded prefixes cannot answer "what types exist?" |
| Automatic data capture via prompt policy | A deterministic fact extractor after every turn | An extractor cannot tell durable facts from transient talk, guesses, or secrets, and it would write behind the CM's back. The prompt policy plus broker validation is auditable and model-driven |
| Durable versioned plans | Plan as a status variable or a chat message | A plan needs versions, steps, dependencies, and verification. A string in status cannot be updated safely or linked to a run |
| Inner thought as a sub-agent | Same CM thinks in the transcript | Inner deliberation would pollute the persona and the token budget, and the operator would see raw reasoning instead of a reply. A separate read-only session keeps the outer voice clean |
| Allowlisted proactive actions | Arbitrary scheduled shell/agent jobs | A generic scheduler is a security and audit problem. The six installed actions each have a handler, a budget, and a known side-effect class |
| A typed status dictionary | A full project/task store exposed through tools | A task database is a second product inside the conversation loop. The CM needs a compact, truthful working model, not CRUD over every work item. Detailed records stay with external tools and agents |
| Automatic inference | Per-turn trigger rules ("if the topic changes, call `conversation.topic`") | The operator requirement is that the CM infers. Trigger rules make the CM a script, bloat the prompt, and fail on the cases the rule author did not imagine |
| A canonical state block | A prose summary of "what is going on" | A summary is lossy, unstable, and cannot be tested for completeness. A canonical serializer is exact, diffable, and bounded |
| State as a separate system message | Append state to every user message | Duplication, stale copies, and transcript pollution. One state message is replaced in place before every call |
| Turn-by-turn `agent.run` | One blocking `agent.run` that returns only at job end | The CM cannot react to a blocker or a question, a 30-minute job exceeds HTTP timeouts, and a crash loses the whole call. One call performs at most one downstream turn |
| Revisioned SSE status | Poll `/api/state` and `/api/cme/snapshot` every second | Polling recomposes the whole view, causes loading flashes, has no ordering guarantee, and multiplies store reads per open tab |
| Inspector right rail | A third workspace document pane | The two-pane workspace is a deliberate design contract (UI plan `03`); a third pane would steal width from chat/document and complicate swap/fullscreen. Instrumentation belongs in a collapsible rail |
| Separate agentstream per run | Reuse the CM session | Raw agent turns would pollute the conversation transcript and token budget, and the trace viewer could not isolate a run or rebuild its message history |

---

## 4. Target architecture

### 4.1 Component map

```
crates/
  mm-cme/                 conversation state, prompt packet, tool-loop policy,
                          PlanService/ThinkService sub-sessions (bounded, read-only tools)
  mm-agent/               AgentBackend trait, Hermes, Pi, run registry, turn summarizer,
                          agentstream bridge
  mm-data/                DataService/DataBroker, schema catalog, namespace registry, plan/action
                          stores, RDF engine, agentstream, conversation state, project memory
  mm-search/              NEW: Tantivy index, document builder, outbox consumer, reindex
  mm-store-graph/         existing Oxigraph actor; add RDF/XML insert/dump and batch apply
  mm-pi/                  existing Pi RPC client; mm-agent adapts it
  mm-llm/                 existing OpenAI-compatible substrate; summarization, plan, think
  mm-serve/               fourteen tool handlers, ActionService, status aggregator, routes
frontend/
  src/components/inspector/   NEW: status panel, knowledge/plan/action panels, trace viewer
  src/lib/api/inspector.ts    NEW: status/trace clients and reducers
```

Dependency direction:

```
mm-search ──► mm-core
mm-store-graph ──► mm-core
mm-data ──► mm-store-graph, mm-search, mm-core (DataService is the only writer to managed
             instance data and its search documents; conversation/agentstream keep their own
             stores in the same dataset)
mm-cme ──► mm-data, mm-llm (sub-sessions use a narrow ReadOnlyDataTools trait)
mm-agent ──► mm-pi, mm-llm, mm-data
mm-serve ──► mm-cme, mm-agent, mm-data, mm-search, mm-llm
```

`mm-cme` must not depend on `mm-serve`. The fourteen tool handlers live in `mm-serve` because
they need `AppState`; the state types and invariants live in `mm-data::conversation`, and the data
contracts live in `mm-data::data`.

Shared types (`ConversationState`, `StatusValue`, `StatusVariable`, `ArtifactRef`, `DataEntity`)
live in `mm-data` and are re-exported by `mm-cme` where needed. `SearchDocument` lives in
`mm-search`, which depends only on `mm-core`; `mm-data` maps `DataEntity` into it. This is
deliberate: `mm-agent` returns artifact references in `AgentTurnResult`, and it must not depend on
`mm-cme` (which depends on `mm-agent`). Putting the shared state types in the data layer keeps the
dependency graph acyclic.

### 4.2 The store map

| Store | Contents | Authority |
|---|---|---|
| **RDF dataset** (`mm-data` → `mm-store-graph`, Oxigraph/RocksDB) | All managed instance data, ontology T-Box, schema catalog, namespaces, plans, actions, conversation state, agentstream graphs | Authoritative for every RDF fact; one logical dataset with named graphs |
| **Tantivy index** (`mm-search`) | One document per managed entity: label, body, types, graph, namespace, revision, status | Derived; rebuildable from RDF; verified on search |
| **Data outbox** (`mm-data` → SQLite) | Pending index mutations keyed by `dataRevision` | Durable queue between RDF commit and index commit |
| **Agentstream** (`mm-data`) | Raw events per session, including the CM conversation, each downstream agent run, and plan/think sub-sessions | Append-only history; replay and trace viewer |
| **Project memory** (`mm-data`, existing) | Compressed segments, commitments, open questions, goals, artifacts, narrative | Long-term continuity; read by the prompt, written by the existing memory pipeline |

The conversation graph is a **materialized projection**, not a competing source of truth. Every
write also appends a `TopicUpdated`, `DepthUpdated`, or `StatusUpdated` event to agentstream; a
replay can rebuild the materialized state from the event stream. Optimistic concurrency uses a
monotonic `state_revision`.

Plans and actions are RDF graphs inside the same dataset, not separate databases. The schema
catalog and namespace registry are also RDF (`schema` graph). This is what makes one dataset and
one backup possible.

**No `WorkStore`.** The previous plan introduced a separate work-item store and project/task
tools. This revision removes both from the CM design. Project and task information is represented
as status variables (`project`, `tasks`, `next`, `blockers`) and as delegated agent work. A durable
project or goal can be a typed `data.*` entity when it needs to outlive the conversation. The
existing `ProjectRegistry` remains for project identity and memory binding; the CM does not mutate
it directly, and once migrated its write path delegates to `DataService` so there is one writer
and one search document. Detailed task records, if a project needs them, live in that project's external
tooling and its agents, not in a second CRUD surface inside the conversation loop.

### 4.3 The event streams

| Stream | Producer | Consumer |
|---|---|---|
| **CM stream** (`session_id` = console session) | `mm-cme` turns, tool calls, topic/depth/status mutations | Chat transcript, status panel, agentstream viewer |
| **Agent streams** (`session_id` = `agent/<run_id>`) | `mm-agent` for each downstream turn, tool call, summary | Trace viewer, agent run status, audit |
| **Plan streams** (`session_id` = `plan/<plan_id>`) | `PlanService` planning sub-session | Plan editor, trace viewer, audit |
| **Think streams** (`session_id` = `think/<think_id>`) | `ThinkService` inner-thought sub-session | Inspector trace only; never the chat transcript |
| **Data stream** (`session_id` = `data/<namespace>` or CM session) | `DataService` mutations: create/update/delete/schema/namespace | Knowledge panel, audit, index outbox |
| **Action streams** (`session_id` = `action/<action_id>`) | `ActionService` immediate/scheduled executions | Action panel, trace viewer, audit |

Agent, plan, think, and action streams are deliberately separate. A Hermes run may contain dozens
of turns and large tool outputs; mixing them into the CM transcript would pollute the conversation
and inflate the prompt. The CM only ever receives the compact result envelope and the projections
in the state block.

---

## 5. Conversation state

### 5.1 The state model

The entire CM-owned state is small enough to read in one screen:

```rust
pub struct ConversationState {
    pub session_id: String,
    pub revision: u64,                    // monotonic, optimistic concurrency
    pub topic: Option<String>,            // plain-language label, <= 160 chars
    pub depth: u8,                        // 0..=7; 0 is the top level
    pub status: BTreeMap<String, StatusValue>,
    pub updated_at: i64,                  // Unix ms
    pub updated_by: StateOrigin,          // user | cm | system
}

pub enum StateOrigin { User, Cm, System }

pub enum StatusValue {
    Text(String),                         // <= 500 chars
    Integer(i64),
    Bool(bool),
    List(Vec<String>),                    // <= 8 items, each <= 120 chars
    Ref(StatusRef),                       // a verified or unverified reference
}

pub struct StatusRef {
    pub kind: RefKind,
    pub id: String,
    pub label: Option<String>,
    pub verified: bool,
}

pub enum RefKind { File, Project, AgentRun, Memory, Url, Artifact }

/// Agent artifacts use the same reference shape as status refs. Both are labels, not
/// capabilities; a ref does not grant file or network access.
pub type ArtifactRef = StatusRef;
```

There is no topic tree, no parent pointer, no second current-topic cursor, and no project/task
record. The previous plan's hierarchy is replaced by two independent, explicit fields:

- **`topic`** — what the conversation is currently about, in the CM's own words. One label, not a
  path. Examples: `"auth refresh flake"`, `"release 2.0 planning"`, `"inspector SSE"`.
- **`depth`** — how far the conversation has drilled into the current topic. `0` is the broadest
  level, `1` is a concrete task, `2+` is a narrower sub-problem. Depth is a property of the
  current topic, not a cursor into a stored path. Setting the topic does not change the depth;
  setting the depth does not change the topic. The CM sets each one when it means it.

The transcript still contains the previous topics and the path the conversation took. The state
does not duplicate history. This is the deliberate compression: the CM sees the *current* topic
and depth, and the transcript supplies the route.

### 5.2 Status variables

`status` is a typed, bounded dictionary. It is not a free-form scratchpad and not a project
database. It exists so the CM can keep a small set of facts true without re-deriving them from the
transcript every turn.

**Reserved variables** drive the inspector and have fixed types:

| Key | Type | Meaning | Example |
|---|---|---|---|
| `phase` | Text enum: `idle`, `working`, `waiting`, `blocked`, `done` | What the conversation is doing right now | `"working"` |
| `intent` | Text | What the operator is trying to accomplish | `"make the refresh test deterministic"` |
| `project` | Text or Ref | Bound project name/id | `"metamind-console-v2"` |
| `tasks` | List | Compact task states, `"title [status]"` | `["reproduce flake [doing]", "fix clock skew [todo]"]` |
| `next` | List | Next concrete actions, in order | `["reproduce the flake", "run auth test 20x"]` |
| `blockers` | List | What is stopping progress | `["waiting on review"]` |
| `questions` | List | Open questions for the operator | `["do we support clock skew?"]` |
| `notes` | Text | One short note the next turn needs | `"seed 42 fails near midnight"` |
| `focus` | Ref | Current file/document/agent anchor | `file:src/auth/refresh.rs` |

**Custom variables** are allowed when the reserved set is not enough. Keys must match
`[a-z][a-z0-9_]{0,31}`. A custom key cannot shadow a reserved key. Values use the same five types.

**Limits**

- At most 24 variables total.
- At most 2 KiB of canonical JSON for the status map.
- Text values at most 500 characters; list values at most 8 items of 120 characters.
- No nested objects, no binary data, no newlines in keys.
- `topic` and `depth` are **not** status variables. `conversation.status` rejects them. They have
  their own tools so there is exactly one writer for each field.

**Project management with conversation state and delegated work.** The CM manages a project
conversationally, not through a task CRUD:

- `project` names the bound project (or the closest stable identifier).
- `tasks` is the compact state of the work: `"title [todo|doing|blocked|done]"` strings.
- `next` is the ordered list of next actions.
- `blockers` is what prevents progress; `questions` is what the operator must answer.
- `phase` says whether the conversation is working, waiting, blocked, or done.
- Actual file changes, ticket changes, and builds are delegated through `agent.run` to an agent
  that has the project's tools. The CM records the compact result and keeps the panel truthful.

This is deliberately not a task database. A task database with dependencies, assignees, and due
dates is a different product. When a project needs that, the project's own tooling and agents own
it; the CM owns the conversational view.

**Why a dictionary instead of project/task tools?** The CM's job is to keep a compact, truthful
model of the conversation. A task database with status transitions, dependencies, assignees, and
due dates is a different product. When a project needs that, the CM delegates to an agent that
uses the project's own tools, and records the compact result in `tasks`/`next`. The inspector
shows those strings; it does not pretend they are relational records.

**Canonicalization**

- On the wire, `StatusValue` serializes as the underlying JSON value (a string, number, boolean,
  array, or a ref object). It is not externally tagged as `{"Text": ...}`. The Rust enum is an
  internal type; the canonicalizer owns the mapping.
- Keys sorted lexicographically.
- Text values JSON-escaped.
- Lists preserve order (order is meaningful for `next` and `tasks`).
- Refs serialize as `{"kind":"file","id":"src/auth/refresh.rs","label":null,"verified":true}`.
- The same canonicalizer produces the store payload, the wire DTO, and the prompt block. There is
  no second serializer that can drift.

### 5.3 Invariants

1. `revision` is monotonic per session. Every write requires `expected_revision`; a mismatch
   returns `CONFLICT` with the current state.
2. `topic`, once set, is non-empty after trimming and at most 160 characters. It may contain
   spaces and punctuation. It may be `None` before the CM first names the conversation. There is
   **no auto-seeding**: the system does not guess a topic from the first user message.
3. `depth` is in `0..=7`. It is independent of `topic`. Setting `depth` does not require a topic;
   setting a topic does not reset depth.
4. Reserved status keys have the types in §5.2. A wrong type is `INVALID_ARGUMENTS`, never a
   silent coercion.
5. The status map never exceeds 24 keys or 2 KiB canonical JSON. A write that would exceed either
   cap is rejected with `STATE_TOO_LARGE` and the current size. The prompt therefore never has to
   truncate state.
6. A write that produces no change returns `unchanged: true` and does not bump the revision.
7. `conversation.status` cannot write `topic` or `depth`. `conversation.topic` cannot write status
   variables. `conversation.depth` cannot write either.
8. A `Ref` of kind `File` is workspace-relative, normalized, and checked at write time. A
   `Project`/`AgentRun`/`Memory` ref is checked against its owning store when available. If the
   store is closed, the ref is stored `verified: false` and the result carries a warning; the
   write still succeeds because the conversation state is not the authority for that object.
9. No mutation silently deletes history. Unsetting a variable removes it from current state, but
   the event log retains the previous value and the revision.
10. The state block is generated from this struct and the derived projections in §5.5. It is
    never hand-written by the CM.

### 5.4 Automatic state maintenance (no trigger instructions)

The operator requirement is explicit: the CM infers from the conversation when to call the state
tools. It is not told to call them.

The design enforces this in four ways:

1. **No per-turn trigger text.** The executor does not append "the topic changed, call
   `conversation.topic`", "you must update status", or any similar reminder. There is no
   `require_context_update` flag, no forced state round, and no post-hoc deterministic topic
   guess. A turn can end without a state write; the next turn still sees the current state.
2. **Capability descriptions, not schedules.** Tool descriptions say what each tool does. They do
   not say "call this when…". The stable prompt says the CM owns the state and that the block is
   authoritative. That is a responsibility statement, not a trigger.
3. **The current state is always visible.** The state block at the top of every model call shows
   the topic, depth, and every status variable. If the state is stale or empty, the model can see
   that without being told.
4. **Writes return the full new state.** After any state write, the tool result contains the
   complete canonical state, and the next model round receives a freshly rebuilt state block. The
   model immediately sees the effect of its choice, which makes the next choice easier.

**What the model is told (stable prompt, one sentence):**

> You are the conversation manager. You own the conversation state shown in `[STATE]`; keep it
> true as the conversation moves. The state block is authoritative and is refreshed before every
> model call.

That is the whole instruction. It does not enumerate triggers.

**How reliability is achieved without scripts:**

- The tool set is tiny and semantically named.
- The state block makes the current values obvious.
- Tool results are full-state, so there is no ambiguity about what changed.
- The inspector shows the same state, so the operator can see a missed update and correct it in
  conversation ("the topic is actually the deployment, not the test").
- Automatic-inference quality is measured by an eval suite (§15.3). If the model under-updates,
  the fix is better tool descriptions, better examples in the stable prompt, or a model change —
  not hidden trigger messages.

**What is deliberately not automatic:**

- No system-generated topic.
- No system-generated depth.
- No system-generated status variables.
- No background reconciler that writes state behind the CM's back.

The only automatic behavior is **reading and injecting** the current state. Writing is the CM's
decision.

### 5.5 The state block

The state block is a separate system message in every model request. It is rebuilt from
`ConversationStore` plus the agent registry and the health snapshot before **every** model call,
including each round after a tool result. A stale copy is never reused.

**Message order**

```
1. stable system message
   identity · relationship · affect · continuity · invariants
   "You own the conversation state shown in [STATE]…"

2. state system message  ← replaced in place before every model call
   [STATE ...]
   { ...canonical JSON... }

3..n. transcript (prior turns)

last. the operator's current user message
```

The state block is not appended to the user message and does not replace it. The operator's input
is always its own user message.

**Canonical form**

```
[STATE rev=17 as_of=2026-11-22T12:04:18.412Z]
{"topic":"auth refresh flake","depth":2,"status":{"blockers":[],"focus":{"kind":"file","id":"src/auth/refresh.rs","label":null,"verified":true},"intent":"make the refresh test deterministic","next":["reproduce the flake","run auth test 20x"],"notes":"seed 42 fails near midnight","phase":"working","project":"metamind-console-v2","questions":["do we support clock skew?"],"tasks":["reproduce flake [doing]","fix clock skew [todo]"]},"agents":[{"run_id":"01R","kind":"hermes","status":"running","turn":2,"next":"continue","last":"test passes with seed 1; fails with seed 42"}],"data":{"rev":88,"entities":1240,"docs":1240,"lagMs":0,"schemas":["mm","foaf","schema"],"last":"data.update mmd:user/01U"},"plans":{"current":"01P","version":3,"status":"active","steps":4},"actions":{"scheduled":1,"running":0},"health":{"kernel":"ok","graph":"ok","search":"ok","indexLagMs":0,"llm":"ok","agent":"ok"},"ops":["12:04:18 agent.run continue -> running turn 2","12:04:11 conversation.topic set \"auth refresh flake\""]}
```

- **Compressed.** One line of minified JSON after the header. No prose, no repeated labels, no
  pretty-printing.
- **Lossless with respect to conversation state.** Every `topic`, `depth`, and status key/value is
  present. The status map is never truncated or summarized. If it cannot fit the cap, the write
  that would exceed the cap is rejected; the prompt still contains the complete state.
- **Derived projections are explicit.** `agents` contains all active runs (max 3) and the most
  recent terminal run. `data` is a bounded knowledge-health projection: revision, entity count,
  document count, index lag, active schema prefixes, and last mutation. `plans` is the current
  plan header, not the plan body. `actions` is scheduled/running counts, not the action list.
  `health` is a compact service-state line. `ops` contains every in-flight operation plus the last
  five completed operations. These are not conversation state and are capped by construction;
  active runs and active operations are never omitted.
- **The full data, plan, and action content lives in its own tool result or inspector panel, not
  in the state block.** The block tells the model that knowledge exists and whether it is healthy;
  it does not dump the graph or a plan body into every prompt.
- **Deterministic.** The same state produces the same block. Golden tests compare the exact
  string. The block is not generated by an LLM.
- **Honest when unavailable.** If the store cannot be read, the block is:
  `[STATE unavailable: <reason>]`. The prompt does not silently omit it, and the model is told
  that state writes will fail closed.
- **Bounded.** The status map cap is 2 KiB; the whole block target is <= 6 KiB (the extra 2 KiB is
  the data/plan/action projection). The block size is measured as a metric. A block over budget is
  a bug in the cap enforcement, not a reason to truncate.

**Freshness across tool rounds**

```
round 0: read state + agents + health -> build block rev 17 -> call model
         model calls conversation.topic set
         tool writes rev 18, returns full state rev 18
round 1: read state rev 18 + agents + health -> rebuild block rev 18 -> call model
         ...
```

The executor holds a `StateView` per round, not a long-lived prompt string. There is no code path
that calls the model without first refreshing the block.

### 5.6 Persistence, graphs, and events

New module: `crates/mm-data/src/conversation.rs`.

```rust
pub struct ConversationStore { engine: Arc<RdfEngine> }

impl ConversationStore {
    pub async fn load(&self, session_id: &str) -> Result<Option<ConversationState>>;
    pub async fn save(&self, state: &ConversationState, expected_revision: u64)
        -> Result<ConversationState>;          // CONFLICT on stale revision
    pub async fn append_event(&self, event: ConversationEvent) -> Result<()>;
}
```

Graphs (new helpers in `mm-data/src/rdf.rs`):

| Graph | Contents |
|---|---|
| `https://metamind.dev/graph/conversation/{session_id}` | `ConversationState`, topic, depth, status variables |
| `https://metamind.dev/graph/conversation-index` | Session index, for operator listing |

Event additions to `mm-data/src/agentstream.rs`:

```rust
pub enum EventType {
    // existing variants …
    TopicUpdated,
    DepthUpdated,
    StatusUpdated,
    AgentTurn,
    AgentTurnSummary,
    AgentStatus,
    DataMutated,          // create | update | delete | schema | namespace
    DataIndexed,          // document committed, or index repair
    PlanUpdated,          // plan created or new version
    ThoughtCompleted,     // optional persisted inner thought
    ActionScheduled,
    ActionUpdated,
}
```

`as_str()` returns `topic_updated`, `depth_updated`, `status_updated`, `agent_turn`,
`agent_turn_summary`, `agent_status`, `data_mutated`, `data_indexed`, `plan_updated`,
`thought_completed`, `action_scheduled`, `action_updated`. These are additive; existing serialized
events remain valid.

Event payloads carry `schema`, `session_id`, `revision`, and a compact diff. Never store the full
state in every event:

```json
{
  "schema": "mm.conversation.v1",
  "sessionId": "01…",
  "revision": 18,
  "op": "topic_set",
  "topic": "auth refresh flake",
  "depth": 2,
  "origin": "cm"
}
```

```json
{
  "schema": "mm.conversation.v1",
  "sessionId": "01…",
  "revision": 19,
  "op": "status_set",
  "set": { "phase": "working", "next": ["reproduce the flake"] },
  "unset": []
}
```

The materialized `ConversationState` is rebuilt by applying these events in revision order. The
event log is the audit record; the graph is the fast read.

---

## 6. The CM tool surface

### 6.1 Final catalog

Fourteen tools in three bounded families. The conversation core stays at four; data and metamind
are explicit job families rather than a revived CLI mirror.

**Conversation core (4)**

| # | Tool | Kind | What it is for | Mutates |
|---|---|---|---|---|
| 1 | `agent.run` | external | Start or continue one downstream Hermes/Pi turn from natural-language input | Agent run registry + separate agentstream |
| 2 | `conversation.status` | internal | Get, set, or unset the conversation's typed status variables | Conversation state (`status`) |
| 3 | `conversation.topic` | internal | Get or set what the conversation is currently about | Conversation state (`topic`) |
| 4 | `conversation.depth` | internal | Get or set how deep the current topic is | Conversation state (`depth`) |

**Data and ontology (7)**

| # | Tool | Kind | What it is for | Mutates |
|---|---|---|---|---|
| 5 | `data.create` | data | Create one ontology instance from a compact typed description; validate, store, and index it | RDF dataset + Tantivy index |
| 6 | `data.update` | data | Update one instance's properties, with revision checking and schema validation | RDF dataset + Tantivy index |
| 7 | `data.query` | data | Query instances by class, subject, and property filters; returns compact entities | Read-only |
| 8 | `data.delete` | data | Soft-delete by default; hard-delete only with explicit authority | RDF dataset + Tantivy index |
| 9 | `data.search` | data | Full-text search over labels, literals, classes, and namespaces | Read-only (may repair stale index entries) |
| 10 | `data.schema` | data | List, inspect, validate, register, activate, or deprecate OWL schemas in the internal catalog | Schema graph + catalog |
| 11 | `data.namespace` | data | List, bind, expand, or resolve compact-IRI namespaces | Schema graph + namespace registry |

**Metamind actions (3)**

| # | Tool | Kind | What it is for | Mutates |
|---|---|---|---|---|
| 12 | `metamind.plan` | metamind | Enter a bounded planning session; create, update, read, or list a markdown or structured plan | Plan graph + plan index |
| 13 | `metamind.think` | metamind | Run a private inner-thought sub-agent that can read data, then return findings to the outer CM | Think agentstream + ephemeral result |
| 14 | `metamind.act` | metamind | Immediately run or schedule one allowlisted proactive action | Action graph + scheduler/agentstream |

This is a hard cap of fourteen. The four conversation tools remain the only tools that write
conversation state. The data family is the only path by which the CM writes ontology instance data.
The metamind family is the only path by which it plans, thinks privately, or schedules proactive
work.

Deliberately excluded from the CM surface:

- **`context`, `project`, `task`, `memory`, `being`, `kernel`, `focus`** — the previous proposal.
  Project/task detail is a status variable or delegated work; being/kernel reads are already in the
  prompt; focus is an operator UI action. FAQ: `data.create` can create a `mm:Goal` or `mm:Project`
  instance, but it cannot change kernel state or promote a change set.
- **`graph.query`, `graph.update`, `graph.validate`** — raw SPARQL is admin-only. `data.query`
  compiles a bounded structured query; `data.schema validate` runs shapes; neither accepts SPARQL.
- **`changeset.*`, `sandbox.*`, `promote.*`, `reject.*`, `audit.*`** — gate-bearing or
  administrative. The CM delegates engineering work to an agent, and the operator promotes.
- **`pi.run`, `pi.ingest`** — replaced by `agent.run` (and the operator import path, unchanged).
- **`sandbox_tool.*`, `mcp.*`, `guide.*`, `catalog.open`** — admin/operator surfaces.
- **`cancel`/`status` actions on agents** — the state block shows run status; the inspector has the
  operator cancel button. Adding a fifteenth tool for cancel would violate the cap. If the operator
  asks the CM to cancel, the CM explains that cancellation is an inspector action.

The existing controller catalog remains for the CLI, direct HTTP administration, and parity tests.
`ConsoleTools` is replaced; it is not extended. The `parity` test that checks every CLI command is
mentioned by a tool summary must be scoped to the **admin catalog**, not the CM surface.

### 6.2 Shared envelope

Every tool returns the same envelope. Data and metamind tools add `dataRevision` and `index` so
the CM can see whether the knowledge store is current; conversation tools may omit them.

```json
{
  "ok": true,
  "tool": "data.create",
  "action": "create",
  "stateRevision": 18,
  "state": { "...full ConversationState..." },
  "dataRevision": 88,
  "index": { "pending": 0, "lagMs": 12 },
  "result": { "entity": { "...DataEntity..." } },
  "error": null
}
```

On failure:

```json
{
  "ok": false,
  "tool": "conversation.status",
  "action": "set",
  "stateRevision": 18,
  "state": { "...current state..." },
  "result": null,
  "error": {
    "code": "CONFLICT",
    "message": "expected revision 17, current revision 18",
    "field": "expected_revision",
    "retryable": true,
    "next": "re-read the state block and retry once"
  }
}
```

Rules:

- **Strict parsing.** Invalid JSON is `INVALID_JSON`, not `{}`. Unknown fields are
  `INVALID_ARGUMENTS`. Missing required fields are `INVALID_ARGUMENTS` with the field name.
- **No silent coercion.** `"3"` is not `3`; `null` is not an empty string.
- **Full state on every result.** Even a read returns the full conversation state, so the model
  always has a current snapshot. A write returns the post-write state. Data reads return the
  current `dataRevision` and index health; they do not dump the RDF store.
- **Structural truncation only where safe.** State and status are never truncated. Entity result
  sets are capped by `limit` and by a 32 KiB envelope budget; a capped result says
  `hasMore: true` and carries a cursor. Agent turn text may be summarized by `mm-agent` before it
  reaches the CM, with the raw trace retained. A tool result that must be capped carries
  `truncated: true` and a `next` pointer; it never cuts JSON mid-object.
- **Timeouts.** State tools: 2 s. Data reads: 5 s. Data writes: 10 s. Schema changes: 10 s.
  `metamind.plan` / `metamind.think`: 90 s. `metamind.act` immediate: 30 s; scheduling: 2 s.
  `agent.run`: 120 s for one downstream turn. A timeout returns a typed `TIMEOUT`/`RUNNING`
  result with the run id and last known summary, not a fake success.
- **Idempotency.** State writes are set/unset operations and are naturally idempotent. A no-op
  write returns `unchanged: true`. Data writes accept an optional `idempotency_key`; a replay with
  the same key returns the original result. `agent.run` is not retried automatically after a
  timeout; the result says whether the run is still active. `metamind.act` uses an idempotency key
  for scheduled actions.

Error codes:

| Code | Meaning | Retryable |
|---|---|---|
| `INVALID_JSON` | The arguments were not valid JSON | no (fix the call) |
| `INVALID_ARGUMENTS` | Missing, unknown, or wrong-typed field | no |
| `CONFLICT` | `expected_revision` is stale | yes, after re-reading state |
| `STATE_TOO_LARGE` | Status map would exceed 24 keys or 2 KiB | no (compact it) |
| `DATA_CONFLICT` | The entity's current revision does not match `expected_revision` | yes, after re-reading |
| `SCHEMA_VIOLATION` | The instance or update violates the active schema or shapes | no (fix the data) |
| `INDEX_PENDING` | The RDF write committed but the search document is not yet visible | yes, retry search shortly |
| `SEARCH_UNAVAILABLE` | Tantivy or its index is unavailable | maybe |
| `NAMESPACE_CONFLICT` | Prefix or namespace is already bound to a different value | no (choose another) |
| `PLAN_CONFLICT` | Plan revision is stale | yes, after re-reading |
| `ACTION_UNKNOWN` | The requested proactive action is not in the allowlist | no |
| `NOT_FOUND` | Run id, entity, plan, or ref target does not exist | no |
| `RUN_TERMINAL` | Tried to continue a finished run | no (start a new run) |
| `UNAVAILABLE` | Agent backend, data service, or search engine is not available | maybe |
| `FORBIDDEN` | Workspace or operation is outside policy | no |
| `CONFIRMATION_REQUIRED` | A workspace outside the allowed roots needs operator approval | no (ask the operator) |
| `BUDGET` | Per-run or per-turn budget exhausted | no |
| `TIMEOUT` | The operation did not finish in time | yes, by checking the run |
| `INTERNAL` | Unexpected server error | maybe |

### 6.3 `agent.run`

The only external tool. One call performs at most one downstream agent turn.

```json
{
  "name": "agent.run",
  "description": "Ask an external agent to do work. Give it a plain-language task or a plain-language continuation. It performs one turn and returns what it did, what it needs, and whether it is done. Omit run_id to start a new run; include run_id to continue that run.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "input": {
        "type": "string",
        "minLength": 1,
        "maxLength": 8000,
        "description": "The task or continuation in natural language. Do not use shell syntax, URLs, or structured commands as a substitute for describing the work."
      },
      "run_id": {
        "type": "string",
        "description": "Continue this run. Omit to start a new one."
      },
      "agent": {
        "type": "string",
        "enum": ["hermes", "pi"],
        "description": "Which downstream agent to use. Defaults to the configured backend (Hermes)."
      },
      "workspace": {
        "type": "string",
        "description": "Optional workspace root. Defaults to the session/project sandbox. Paths outside the allowed roots require operator confirmation."
      }
    },
    "required": ["input"]
  }
}
```

Only `input` is required. `run_id`, `agent`, and `workspace` are routing metadata; they are not a
command language. The task itself is always described in natural language.

**Behavior**

- **No `run_id`** — start a new run with `input` as the task. The backend's system prompt includes
  the input, the workspace, and the operator/session identifiers. Return the first turn.
- **With `run_id`** — continue that run. `input` is the next instruction. `"continue"` is a valid
  natural-language instruction when the agent already knows what to do.
- **One turn only.** The call returns after the downstream model produces one turn, or after a
  tool-using turn completes. It does not wait for the whole job. The CM calls again with the same
  `run_id` to continue.
- **No duplicate in-flight turn.** If a turn for `run_id` is already in flight, the call returns
  that turn's current status (or waits briefly for it) instead of starting a second concurrent
  turn. This makes a post-timeout check safe.
- **Natural language is the interface.** There are no `shell`, `url`, `http`, `command`, or
  `acceptance_criteria` fields. The CM describes the work. The downstream agent decides which of
  its own tools to use.
- **Questions.** If the agent asks for information, `status: "waiting_input"` and `next:
  "wait_for_input"`. The CM must relay the question to the operator and must not invent an answer.
- **Terminal runs.** Continuing a `done`/`failed`/`cancelled` run returns `RUN_TERMINAL` with the
  final record. Start a new run instead.
- **Unknown run.** `NOT_FOUND` includes the active run ids in the error `next` field.
- **Timeout.** If the downstream turn exceeds 120 s, return `TIMEOUT` with `run_id`, `status:
  "running"`, and the last known summary. A later `agent.run` with that `run_id` checks the run and
  returns the completed turn; it does not start a duplicate.
- **Cancellation.** Not exposed to the CM. The inspector's cancel action is the only cancellation
  path. This is the cost of keeping cancellation out of the conversation-core family; it is
  deliberate.

**Result**

```json
{
  "ok": true,
  "tool": "agent.run",
  "action": "run",
  "stateRevision": 18,
  "state": { "...full ConversationState..." },
  "result": {
    "runId": "01R",
    "kind": "hermes",
    "status": "running",
    "turn": 2,
    "next": "continue",
    "message": "The failure is seed-dependent: seed 42 crosses the clock boundary.",
    "summary": {
      "headline": "seed 42 fails on clock skew",
      "did": ["read refresh test", "ran auth suite", "isolated seed 42"],
      "learned": ["the test uses the wall clock"],
      "blockers": [],
      "nextRecommended": "add a controlled clock"
    },
    "artifacts": [
      { "kind": "file", "id": "src/auth/refresh.rs", "label": null, "verified": true }
    ],
    "usage": { "promptTokens": 3200, "completionTokens": 420 }
  },
  "error": null
}
```

The full raw turn is in `agent/<runId>`, not in this result. The state block's `agents` projection
shows the active run so the CM does not need a separate status tool.

**Automatic continuation.** The state block includes `agents[].next`. The CM decides whether to
call `agent.run` again. There is no injected "continue the run" directive. The stable prompt's
invariant says: never claim external work is finished while a run you started is active. That is
an honesty rule, not a tool trigger.

### 6.4 `conversation.status`

Reads or writes the typed status dictionary.

```json
{
  "name": "conversation.status",
  "description": "Maintain the conversation's working variables: phase, intent, project, tasks, next actions, blockers, questions, notes, focus, and custom variables. The current state is always in [STATE]; get returns the same state explicitly.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": {
        "type": "string",
        "enum": ["get", "set", "unset"]
      },
      "values": {
        "type": "object",
        "additionalProperties": {
          "oneOf": [
            { "type": "string" },
            { "type": "integer" },
            { "type": "boolean" },
            { "type": "array", "items": { "type": "string" } },
            { "type": "object" }
          ]
        },
        "description": "For set: keys to merge. Objects are accepted only for a Ref value."
      },
      "keys": {
        "type": "array",
        "items": { "type": "string" },
        "description": "For unset: keys to remove."
      },
      "expected_revision": {
        "type": "integer",
        "minimum": 0,
        "description": "Required for set/unset. Use the revision from [STATE]."
      }
    },
    "required": ["action"]
  }
}
```

**Actions**

- `get` — no extra fields. Returns `{ status, count, bytes }` plus the full state.
- `set` — requires `values` and `expected_revision`. Merges the supplied keys. Reserved keys are
  type-checked. `topic` and `depth` are rejected with `INVALID_ARGUMENTS` (they have their own
  tools). `agents` is rejected; it is derived. A write that would exceed 24 keys or 2 KiB returns
  `STATE_TOO_LARGE` with the projected size.
- `unset` — requires `keys` and `expected_revision`. Removes the keys. Missing keys are a no-op,
  not an error. Reserved keys may be unset; the inspector shows them as absent.

**Examples**

```json
{ "action": "set",
  "values": {
    "phase": "working",
    "intent": "make the refresh test deterministic",
    "project": "metamind-console-v2",
    "tasks": ["reproduce flake [doing]", "fix clock skew [todo]"],
    "next": ["reproduce the flake", "run auth test 20x"]
  },
  "expected_revision": 17 }
```

```json
{ "action": "unset", "keys": ["notes", "blockers"], "expected_revision": 18 }
```

`conversation.status` never writes `topic` or `depth`. This keeps the three state tools from
overlapping and makes conflicts easy to reason about.

### 6.5 `conversation.topic`

```json
{
  "name": "conversation.topic",
  "description": "Read or set the name of what the conversation is currently about. A topic is a short plain-language label, not a path or a project record.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["get", "set"] },
      "topic": {
        "type": "string",
        "minLength": 1,
        "maxLength": 160,
        "description": "The current topic label. Trimmed; no newlines."
      },
      "expected_revision": {
        "type": "integer",
        "minimum": 0,
        "description": "Required for set. Use the revision from [STATE]."
      }
    },
    "required": ["action"]
  }
}
```

- `get` returns `{ topic, depth }` plus the full state.
- `set` requires `topic` and `expected_revision`. It replaces the label. It does **not** change
  depth. It does **not** infer a parent or a path.
- Setting the same topic is `unchanged: true` and does not bump the revision.
- There is no auto-seed. If no topic is set, `get` returns `topic: null`.

### 6.6 `conversation.depth`

```json
{
  "name": "conversation.depth",
  "description": "Read or set how deep the current topic is. 0 is the top level; each increment is one narrower level.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["get", "set"] },
      "depth": {
        "type": "integer",
        "minimum": 0,
        "maximum": 7,
        "description": "The drill level of the current topic."
      },
      "expected_revision": {
        "type": "integer",
        "minimum": 0,
        "description": "Required for set. Use the revision from [STATE]."
      }
    },
    "required": ["action"]
  }
}
```

- `get` returns `{ topic, depth }` plus the full state.
- `set` requires `depth` and `expected_revision`. It does not change the topic.
- Setting the same depth is `unchanged: true`.
- Depth is a label of granularity, not a cursor into a stored path. The transcript holds the route;
  the state holds the current level.

### 6.7 `data.*` — database, ontology, and search tools

The data family is high-level. The CM names an entity, a class, properties, and a search query;
it does not write quads, SPARQL, SQL, or index documents. `mm-data::data::DataBroker` interprets the
intent and maps it to the low-level RDF, schema, namespace, and Tantivy operations described in
§7.

**`data.create`**

```json
{
  "name": "data.create",
  "description": "Create one ontology instance from a compact typed description. The type selects the schema and graph; the store mints an IRI when no id is supplied. The entity is validated, written to RDF, and indexed for search in the same operation.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "type": { "type": "string", "description": "Compact IRI or class IRI, e.g. foaf:Person, mm:Project, mm:Plan." },
      "id": { "type": "string", "description": "Optional stable local id or full IRI. Omit to mint one." },
      "properties": {
        "type": "object",
        "additionalProperties": {
          "oneOf": [
            { "type": ["string", "number", "boolean"] },
            { "type": "array", "items": { "oneOf": [ { "type": "string" }, { "type": "number" }, { "type": "boolean" } ] } },
            { "type": "object", "properties": { "iri": { "type": "string" } }, "required": ["iri"], "additionalProperties": false }
          ]
        },
        "description": "Predicate short names mapped to literal values, lists, or {iri} references."
      },
      "schema": { "type": "string", "description": "Optional schema id. Defaults to the catalog entry for the type's namespace." },
      "graph": { "type": "string", "description": "Optional graph override for an installed namespace. Omit for automatic routing." },
      "idempotency_key": { "type": "string", "minLength": 8, "maxLength": 128 }
    },
    "required": ["type", "properties"]
  }
}
```

- The type is resolved against the schema catalog. An unknown type is `SCHEMA_VIOLATION`, not a
  guess.
- The broker routes the entity to exactly one authoritative graph (`being`, `users/index`, `world`,
  `memory`, `plan`, `action`, or `schema`) from the class and namespace. A caller-supplied `graph`
  must be an installed namespace route; it cannot write into `agentstream` or `ontology` directly.
- The duplicate guard applies only when no explicit `id` is supplied. An explicit id is
  authoritative; if it already exists in the target graph, the result is `DATA_CONFLICT`, not a
  silent overwrite.
- System predicates (`mm:revision`, `mm:createdAt`, `mm:updatedAt`, `mm:status`, `mm:sourceGraph`,
  provenance, and `rdf:type` changes) are broker-owned. Supplying them in `properties` is
  `INVALID_ARGUMENTS`; `data.update` cannot forge a revision or provenance.
- `mm:Being` is a singleton. Creating a second one returns `SCHEMA_VIOLATION`; `data.update` may
  change the existing being's descriptive fields but not its identity IRI or creation instant.
- **Management classes have one writer.** `mm:Schema`/`mm:Namespace` are written only through
  `data.schema`/`data.namespace`; `mm:Plan`/`mm:PlanVersion` only through `metamind.plan`;
  `mm:ProactiveAction` only through `metamind.act`; `mm:Thought` only through
  `metamind.think(persist=true)`. `data.create`/`data.update`/`data.delete` reject them with
  `INVALID_ARGUMENTS` so versioning, scheduling, and catalog invariants cannot be bypassed.
- Required properties are checked (for example `foaf:Person` requires `foaf:name`). No implicit
  defaults are invented.
- The entity gets `mm:revision 1`, `mm:createdAt`, `mm:updatedAt`, `mm:recordedBy`, and provenance
  back to the conversation turn. The result is the full `DataEntity`, `dataRevision`, and index
  health.

**`data.update`**

```json
{
  "name": "data.update",
  "description": "Update one existing ontology instance. Merge adds or replaces the named predicates; replace makes the supplied properties the complete writable set. Revision checking prevents a stale overwrite. The RDF entity is updated and the search document is replaced.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "subject": { "type": "string", "description": "Entity IRI or compact IRI. Required." },
      "properties": { "type": "object", "additionalProperties": true, "description": "Predicate patch, same value shapes as data.create." },
      "mode": { "type": "string", "enum": ["merge", "replace"], "default": "merge" },
      "expected_revision": { "type": "integer", "minimum": 1 },
      "idempotency_key": { "type": "string", "minLength": 8, "maxLength": 128 }
    },
    "required": ["subject", "properties"]
  }
}
```

- `merge` adds values for named predicates; a scalar replaces the old scalar, a list replaces the
  old list, and `null` removes the predicate. `replace` removes every property not named in the
  patch, except immutable and system predicates (`rdf:type`, `mm:iri`, `mm:createdAt`, provenance,
  revision).
- `expected_revision` is optional, but when supplied a mismatch is `DATA_CONFLICT` with the current
  entity. The CM is encouraged to read first.
- Identity fields (`mm:iri`, `mm:createdAt`, `rdf:type` for `mm:Identity`) are immutable. A type
  change requires delete + create and is refused here.
- Updating an entity that does not exist is `NOT_FOUND`; it never silently creates.
- A no-op patch returns `unchanged: true` and does not bump `dataRevision`.

**`data.query`**

```json
{
  "name": "data.query",
  "description": "Read ontology instances by class, subject, property filters, namespace, and graph. Returns compact entities, not raw triples or SPARQL rows.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "class": { "type": "string", "description": "Optional class IRI or compact IRI; includes subclasses." },
      "subject": { "type": "string", "description": "Optional exact entity IRI or compact IRI." },
      "where": { "type": "object", "additionalProperties": true, "description": "Property filters: exact values, {iri}, or {$contains: ...}." },
      "graph": { "type": "string", "description": "Optional graph filter." },
      "namespace": { "type": "string", "description": "Optional namespace prefix filter." },
      "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
      "offset": { "type": "integer", "minimum": 0, "default": 0 },
      "include_deleted": { "type": "boolean", "default": false },
      "include": { "type": "array", "items": { "type": "string" }, "description": "Optional property names to include; defaults to a compact summary." }
    }
  }
}
```

- The query compiles to a parameterized SPARQL `SELECT` over the authoritative graphs. No raw
  SPARQL is accepted.
- Results are `DataEntity` summaries: `iri`, `type`, `label`, `properties`, `graph`, `revision`,
  `updatedAt`, `status`. The result is capped at `limit` and a 32 KiB envelope; `hasMore` and
  `nextOffset` make paging explicit.
- `where` supports exact literal match, `{iri: "..."}` reference match, and `{$contains: "..."}`
  for literal substring. Unknown operators are `INVALID_ARGUMENTS`.

**`data.delete`**

```json
{
  "name": "data.delete",
  "description": "Delete one ontology instance. Soft delete is the default: the entity is marked deleted, excluded from queries and search, and retained for provenance. Hard delete removes its RDF quads and search document and requires explicit authority.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "subject": { "type": "string" },
      "mode": { "type": "string", "enum": ["soft", "hard"], "default": "soft" },
      "expected_revision": { "type": "integer", "minimum": 1 },
      "reason": { "type": "string", "maxLength": 500 },
      "idempotency_key": { "type": "string", "minLength": 8, "maxLength": 128 }
    },
    "required": ["subject"]
  }
}
```

- Soft delete sets `mm:status "deleted"`, `mm:deletedAt`, and `mm:deletedBy`; it removes the
  Tantivy document so search cannot return it. The RDF history remains queryable with
  `include_deleted`.
- Hard delete is refused for protected classes (`mm:Identity`, `mm:Being`, `mm:DevelopmentalMemory`)
  and for an entity with inbound references unless the caller is an operator with
  `data:hard_delete`. The CM's default path is soft delete.
- A delete is idempotent: deleting an already-deleted entity returns `unchanged: true`.

**`data.search`**

```json
{
  "name": "data.search",
  "description": "Full-text search across ontology labels, literal values, class names, and namespaces. Results are entity summaries with a snippet and score; the index is verified against RDF so deleted or missing entities are not returned.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "query": { "type": "string", "minLength": 1, "maxLength": 500 },
      "class": { "type": "string" },
      "graph": { "type": "string" },
      "namespace": { "type": "string" },
      "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10 },
      "snippet": { "type": "boolean", "default": true },
      "include_deleted": { "type": "boolean", "default": false }
    },
    "required": ["query"]
  }
}
```

- Search uses Tantivy's lexical index, not embeddings. The query is parsed with a bounded query
  parser; field filters are separate arguments, not raw Lucene syntax.
- Hits contain `iri`, `type`, `label`, `score`, `snippet`, `graph`, `revision`, `updatedAt`.
- The top hits are checked against RDF. A stale hit is dropped and queued for repair; if the index
  is unavailable, `SEARCH_UNAVAILABLE` is returned rather than an empty result.
- `INDEX_PENDING` means the RDF write is committed but the document is not yet searchable; retry
  shortly or use `data.query` for an exact read.

**`data.schema`**

```json
{
  "name": "data.schema",
  "description": "Read and manage the OWL schema catalog. list and get return compact schema summaries; validate runs SHACL shapes against an entity, class, or graph; register adds an installed schema; activate and deprecate change which schema is authoritative.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["list", "get", "validate", "register", "activate", "deprecate"] },
      "schema": { "type": "string", "description": "Schema id, prefix, or namespace IRI." },
      "prefix": { "type": "string", "description": "For register: compact prefix." },
      "namespace": { "type": "string", "description": "For register: namespace IRI." },
      "source": { "type": "string", "description": "For register: builtin, ontology-file path, inline, or registered remote catalog id." },
      "format": { "type": "string", "enum": ["turtle", "rdfxml"] },
      "body": { "type": "string", "description": "For register with source=inline; capped at 64 KiB." },
      "subject": { "type": "string", "description": "For validate: entity to validate." },
      "class": { "type": "string", "description": "For validate: class to validate instances of." },
      "graph": { "type": "string", "description": "For validate: graph to validate." },
      "include": { "type": "array", "items": { "type": "string" }, "description": "For list/get: classes, properties, shapes, imports." }
    },
    "required": ["action"]
  }
}
```

- `list` returns the internal catalog: id, prefix, namespace, version, source, active, class and
  property counts, last loaded. Standard schemas (`mm`, `foaf`, `schema`, `dcterms`, `doap`, `prov`,
  `skos`, `owl`, `rdfs`, `xsd`) are present from boot.
- `get` returns a compact schema summary plus optional body. The body is capped and never replaces
  the catalog metadata.
- `validate` runs the active SHACL shapes for the class/graph and returns violations with focus
  node, path, severity, and message. It never writes.
- `register` validates that the document parses as OWL/RDF, extracts the namespace, prefix,
  classes, properties, and imports, and records it inactive until `activate`. A file source must
  be under the configured ontology directory. Remote imports are refused unless the host is in
  `MM_DATA_SCHEMA_ALLOWED_HOSTS`.
- Standard prefixes and the `mm:` core schema cannot be rebound or deprecated. A schema that is in
  use by instance data cannot be deprecated without a migration plan; the tool returns
  `SCHEMA_VIOLATION` with the count.

**`data.namespace`**

```json
{
  "name": "data.namespace",
  "description": "Read and manage compact-IRI namespaces. list returns bindings; bind registers a new namespace for a managed prefix; expand maps a compact IRI to a full IRI; resolve maps a full IRI back to a compact form.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["list", "bind", "expand", "resolve"] },
      "prefix": { "type": "string", "pattern": "^[a-z][a-z0-9_-]{0,31}$" },
      "namespace": { "type": "string", "description": "Namespace IRI ending in / or #, or a urn: prefix." },
      "iri": { "type": "string", "description": "Full IRI for resolve, or compact IRI for expand." }
    },
    "required": ["action"]
  }
}
```

- Standard bindings are immutable: `mm`, `mmd`, `mmg`, `foaf`, `schema`, `dcterms`, `doap`, `prov`,
  `skos`, `owl`, `rdfs`, `xsd`.
- `bind` accepts `https://metamind.dev/…` and `urn:metamind:…` namespaces; external namespaces
  require a registered schema and an allowed host.
- A prefix collision is `NAMESPACE_CONFLICT`; the caller must choose another prefix.
- `expand` and `resolve` are pure reads and are safe on every turn.

**High-level mapping**

| Tool | Low-level mapping |
|---|---|
| `data.create` | resolve type/schema → mint or verify IRI → check singleton/immutables → validate required predicates and SHACL → render quads → apply RDF batch → index document |
| `data.update` | load entity → check revision → apply patch → validate → replace changed quads → append provenance → reindex document |
| `data.query` | build typed `DataQuery` → compile parameterized SPARQL → query authoritative graphs → project rows into `DataEntity` |
| `data.delete` | load/check → soft-delete predicates or hard-delete quads → append tombstone/provenance → remove index document |
| `data.search` | parse bounded text query → Tantivy TopDocs → join RDF summaries → verify live status → return snippets |
| `data.schema` | schema catalog lookup → parse OWL → validate imports → update catalog graph → reload active schema set |
| `data.namespace` | namespace registry lookup → validate prefix/IRI → update schema graph → refresh resolver |

### 6.8 `metamind.*` — planning, inner thought, and proactive action

**`metamind.plan`**

```json
{
  "name": "metamind.plan",
  "description": "Enter a bounded planning session. Create a plan from scratch or from supplied content, update an existing plan into a new version, or read/list plans. Plans may be markdown or a structured plan object; the result is stored in the plan graph and returned to the conversation.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": { "type": "string", "enum": ["create", "update", "get", "list"] },
      "plan_id": { "type": "string", "description": "Required for update/get; optional for create." },
      "goal": { "type": "string", "maxLength": 4000, "description": "What the plan is for. Required for create when no content is supplied." },
      "format": { "type": "string", "enum": ["markdown", "structured"], "default": "markdown" },
      "content": { "oneOf": [ { "type": "string" }, { "type": "object" } ], "description": "Markdown text or the structured mm.plan.v1 object. Omit to let the planner generate it." },
      "instructions": { "type": "string", "maxLength": 4000, "description": "Additional planning constraints or update instructions." },
      "expected_revision": { "type": "integer", "minimum": 1, "description": "Required for update." },
      "idempotency_key": { "type": "string", "minLength": 8, "maxLength": 128 }
    },
    "required": ["action"]
  }
}
```

- `create` starts a planning sub-session (`plan/<id>`) with a planning prompt. If `content` is
  supplied, the planner normalizes and validates it instead of inventing a plan; otherwise it
  writes one from `goal` and `instructions`.
- `update` requires `plan_id` and `expected_revision`. It creates an immutable `mm:PlanVersion`
  and moves `mm:currentVersion`; it never overwrites the old version. A structured update may
  change the plan object; a markdown update replaces the body.
- Structured plans use `mm.plan.v1`:

```json
{
  "schema": "mm.plan.v1",
  "goal": "make the refresh test deterministic",
  "assumptions": ["the production clock is authoritative"],
  "steps": [
    { "id": "s1", "title": "reproduce the flake", "detail": "run the auth suite with seeds 1..100", "dependsOn": [], "verification": "same failure at seed 42", "owner": "cm" },
    { "id": "s2", "title": "inject a clock", "detail": "replace wall clock with a test clock", "dependsOn": ["s1"], "verification": "20/20 passes", "owner": "agent" }
  ],
  "risks": ["the test may depend on timezone"],
  "openQuestions": [],
  "successCriteria": ["20/20 passes with seed 42"]
}
```

- Markdown plans are stored as a literal body; structured plans are stored as canonical JSON plus
  RDF step edges (`mm:hasStep`, `mm:dependsOn`). The original body is retained.
- The planning sub-session may call `data.query`, `data.search`, `data.schema`, and
  `data.namespace` read-only. It may not call `metamind.plan` recursively, `metamind.think`,
  `metamind.act`, or `agent.run`. If it needs external work, it writes a step for the outer CM to
  delegate.
- `get`/`list` are reads and do not bump the plan revision.

**`metamind.think`**

```json
{
  "name": "metamind.think",
  "description": "Run a private inner-thought sub-agent. It receives a question and optional focus entities, may call read-only data tools, and returns findings with evidence and confidence. The inner voice never speaks to the operator; the outer conversation manager composes the reply in character.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "question": { "type": "string", "minLength": 1, "maxLength": 4000 },
      "context": { "type": "string", "maxLength": 8000, "description": "Optional context from the conversation that the thinker needs." },
      "focus": { "type": "array", "items": { "type": "string" }, "maxItems": 8, "description": "Entity IRIs or compact IRIs to start from." },
      "depth": { "type": "integer", "minimum": 1, "maximum": 3, "default": 2 },
      "persist": { "type": "boolean", "default": false, "description": "Retain the thought result as an mm:Thought. The raw agentstream trace is retained regardless." },
      "idempotency_key": { "type": "string", "minLength": 8, "maxLength": 128 }
    },
    "required": ["question"]
  }
}
```

- The thinker runs as a separate agentstream session `think/<id>` with an analytic system prompt:
  "You are Metamind's inner reasoning process, not the speaker. Use the read-only data tools. Do
  not address the operator. Return findings, evidence, confidence, and gaps."
- Read-only tool set: `data.query`, `data.search`, `data.schema` (get/validate only), and
  `data.namespace` (list/expand/resolve only). It cannot write, plan, act, delegate, or recurse.
- The result is compact:

```json
{
  "thinkId": "01T",
  "summary": "The failure is seed-dependent; the test reads the wall clock.",
  "findings": [
    { "claim": "seed 42 crosses the midnight boundary", "confidence": 0.86, "evidence": [{ "kind": "file", "id": "src/auth/refresh.rs" }, { "kind": "memory", "id": "01M" }] }
  ],
  "gaps": ["no test-clock abstraction exists"],
  "suggestedFocus": ["src/auth/refresh.rs"],
  "usage": { "promptTokens": 1800, "completionTokens": 260 },
  "elapsedMs": 2400
}
```

- The outer CM sees the result and writes the actual reply. The raw trace is in the inspector's
  Trace tab, not in the conversation transcript.
- `persist: false` means the result is ephemeral; only the agentstream trace and metrics remain.
  `persist: true` writes an `mm:Thought` with the summary, evidence, confidence, and provenance.

**`metamind.act`**

```json
{
  "name": "metamind.act",
  "description": "Run one allowlisted proactive action now, or schedule it for later. Actions are the installed proactive action catalog; this is not arbitrary code or arbitrary scheduling.",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "action": {
        "type": "string",
        "enum": ["search_lookup", "goal_narrowing", "plan_refinement", "pattern_watcher", "affect_responder", "knowledge_gap"]
      },
      "input": { "type": "string", "minLength": 1, "maxLength": 4000, "description": "Natural-language subject or instruction for the action." },
      "mode": { "type": "string", "enum": ["immediate", "scheduled"], "default": "immediate" },
      "schedule": {
        "type": "object",
        "additionalProperties": false,
        "properties": {
          "at": { "type": "string", "format": "date-time" },
          "delay_seconds": { "type": "integer", "minimum": 1, "maximum": 2592000 },
          "recurrence": { "type": "string", "enum": ["once", "daily", "weekly"], "default": "once" }
        }
      },
      "priority": { "type": "number", "minimum": 0, "maximum": 1, "default": 0.5 },
      "reason": { "type": "string", "maxLength": 500 },
      "idempotency_key": { "type": "string", "minLength": 8, "maxLength": 128 }
    },
    "required": ["action", "input"]
  }
}
```

- `immediate` executes through `ActionService` in the current session and returns the result and
  `actionId`. `scheduled` writes an `mm:ProactiveAction` to the `action` graph with `mm:dueAt`,
  `mm:recurrence`, `mm:status "scheduled"`, `mm:input`, `mm:origin "cm"`, and returns the id.
- `schedule` requires exactly one of `at` or `delay_seconds`. An action due in the past is
  `INVALID_ARGUMENTS`; scheduling is UTC and returned as RFC3339.
- The action catalog is the installed `ProactiveAction` registry. Unknown actions are
  `ACTION_UNKNOWN`. A scheduled action with the same action/input/due time is idempotent; the
  second call returns the same `actionId`.
- Actions may read data and, through their own handlers, call `agent.run` for external work. They
  inherit the same workspace, budget, and audit rules; they cannot schedule arbitrary shell jobs.

### 6.9 How the tools imply their own use

There are two different inference contracts, because the operator asked for two different things.

**Conversation state: no trigger instructions.** The CM is not told when to call
`conversation.topic`, `conversation.depth`, or `conversation.status`. The state block makes the
current values visible, the tool names state the job, and writes return the full new state. The
stable prompt says "keep it true", not "call this tool now".

**Data: a general capture policy, not a per-turn script.** The operator explicitly requires the
prompts to induce automatic data management. The stable prompt therefore carries a durable policy:

> Treat durable facts as ontology data. When the operator states an identity detail, preference,
> project fact, decision, commitment, correction, definition, relationship, or artifact, record it
> with `data.create` or `data.update` in the same turn. Search before creating, so the same thing
> is updated rather than duplicated. Update or delete a fact when it is corrected; never leave two
> contradictory current values. Do not store transient chatter, guesses, passwords, tokens, or
> secrets. When you need a fact the `[STATE]` block does not contain, use `data.query` or
> `data.search` rather than guessing.

That is a policy about what durable knowledge is, not a list of trigger phrases. There is no
per-turn "the user gave a name, call `data.create`" message. The `[STATE]` block shows the data
revision, entity count, index lag, and active schemas so the model can see whether its knowledge
is current.

**Metamind actions: capability guidance.** The stable prompt states the division of labor:

> Use `metamind.plan` when a task needs a plan rather than a sentence. Use `metamind.think` when a
> question needs private analysis or evidence gathering before you answer. Use `metamind.act` only
> for an installed proactive action the operator asked for or a genuine follow-up you can
> articulate. Do not expose the inner thinker's voice; answer in your own character.

This is guidance, not a schedule. The model still chooses. A plan is not forced for every
multi-step request, and a think is not forced for every question.

**Evaluation, not hidden rules.** §15.3 defines scenario evals for all three families: state
inference, durable-fact capture, duplicate avoidance, correction handling, search grounding, plan
creation/update, think evidence, and act scheduling. If a target is missed, the fix is a better
description, a better example, or a model change — not a per-turn reminder.

### 6.10 Limits and latency

| Tool | Timeout | Retry | Confirmation |
|---|---|---|---|
| `conversation.status` get | 2 s | no | never |
| `conversation.status` set/unset | 2 s | on `CONFLICT` after re-read | never |
| `conversation.topic` get/set | 2 s | on `CONFLICT` after re-read | never |
| `conversation.depth` get/set | 2 s | on `CONFLICT` after re-read | never |
| `data.create` / `data.update` | 10 s | on `DATA_CONFLICT` after re-read | hard delete only |
| `data.query` | 5 s | no | never |
| `data.delete` | 10 s | on `DATA_CONFLICT` after re-read | hard delete: operator |
| `data.search` | 5 s | on `INDEX_PENDING` shortly | never |
| `data.schema` register/activate/deprecate | 10 s | no | remote/inline registration: operator |
| `data.namespace` bind | 5 s | no | external namespace: operator |
| `metamind.plan` create/update | 90 s | on `PLAN_CONFLICT` after re-read | never |
| `metamind.think` | 90 s | no automatic retry | never |
| `metamind.act` immediate | 30 s | no automatic retry | outside sandbox: yes |
| `metamind.act` scheduled | 2 s | no | never |
| `agent.run` new run | 120 s | no automatic retry | sandbox: no; outside allowed roots: yes |
| `agent.run` continue | 120 s | no automatic retry | never |

Per-turn quotas keep automatic data management from becoming a write storm:

- at most 12 `data.create`/`data.update`/`data.delete` calls per turn;
- at most 2 `metamind.think` calls per turn;
- at most 1 `metamind.plan` write per turn;
- at most 4 `metamind.act` calls per turn;
- at most 32 model/tool rounds and 96 tool steps per turn, with the existing 10-minute wall
  clock. A quota stop is a `BUDGET` outcome, not success.

Data writes are local RDF operations plus one Tantivy commit; 10 s is generous. `agent.run` and the
plan/think sub-sessions are the slow paths. A timeout is not a failure of the turn; it returns the
run/plan/think id and the last known summary so the CM can continue or report honestly.

### 6.11 Deprecation map

The 69 admin rows do not map one-to-one. The CM does not need most of them.

| Old row(s) | New path |
|---|---|
| `state.show`, `being.show`, `being.personality_show`, `doctor.run`, `budget.show` | Stable prompt already carries being/health/budgets; no CM tool |
| `focus.open`, `focus.read`, `focus.patch` | Operator UI action; no CM tool |
| `session.note` | `conversation.status` custom variable or `notes` |
| `memory.add`, `memory.recall`, `memory.forget` | Prompt continuity; durable memory writes stay in the memory pipeline. Repeated facts use `data.create`/`data.update` when they are ontology facts |
| `being.goal_add`, `being.goal_list` | Admin/CLI; not a conversation job. A goal can be an `mm:Goal` data instance if it is durable |
| `changeset.*`, `sandbox.*`, `promote.*`, `reject.*`, `audit.*` | `agent.run` delegates; promotion remains operator-only |
| `loop.*`, `debt.*`, `gc.*`, `module.*` | `agent.run` delegates |
| `pi.run`, `pi.ingest` | `agent.run`; `pi.ingest` remains an operator import |
| `codex.*`, `llm.stats`, `meta.analyze`, `regression.run`, `verify.run`, `calibrate.*`, `mcp.*` | `agent.run` or admin inspection |
| `graph.query`, `graph.update` | Admin-only raw SPARQL; CM uses `data.query` |
| `graph.validate` | `data.schema validate` |
| `guide.*`, `catalog.open` | Removed from the CM path; guide remains an operator surface |
| `sandbox_tool.*`, `action.*` | Admin/agent workspace; the CM's proactive path is `metamind.act` |

The admin catalog and CLI keep their current behavior. The CM path gets a new fourteen-schema
bridge and never calls `catalog.open`.

---

## 7. Data, ontology, and search

### 7.1 One logical dataset, one search index

The operator's instruction is one main RDF data store using Oxigraph and its usual RocksDB backend,
plus a separate full-text engine. There is one logical dataset, one live store, one search index,
and one schema catalog.

```
$MM_DATA_DIR/
├── cubecast/
│   ├── main/                    # Oxigraph + RocksDB, the live dataset (existing ENGINE_DIR)
│   └── metamind.rdf             # complete RDF/XML snapshot of managed instance data
├── search/
│   └── tantivy/                 # Tantivy index: one document per managed entity
└── sqlite/
    └── metamind.db              # data_outbox and other non-RDF queues
```

**The physical constraint, stated plainly.** Oxigraph's RocksDB backend is a directory, not a
single file. A live transactional dataset cannot also be a single RDF/XML file; rewriting RDF/XML
on every mutation would be slow and would not use RocksDB. The design therefore uses:

- **`cubecast/main/`** as the live, transactional, authoritative dataset;
- **`cubecast/metamind.rdf`** as the complete RDF/XML snapshot, written on checkpoint, clean
  shutdown, and after a debounced batch of mutations, and imported only when the RocksDB store is
  empty (bootstrap);
- **`cubecast/metamind.nq`** as the lossless quad backup. RDF/XML has no named-graph syntax; the
  N-Quads file preserves graph names. This is a deliberate correctness backup, not a second
  authority.

This satisfies the intent — one main RDF data file plus Oxigraph/RocksDB — without pretending that
an RDF/XML file can be a live named-graph store. `mm-store-graph` gains
`insert_rdfxml`/`dump_rdfxml` so the snapshot is real Oxigraph parsing and serialization, not a
hand-rolled XML writer. The snapshot contains all managed instance data, plans, actions, schema
catalog metadata, and namespaces; the append-only agentstream and conversation-event graphs remain
in the same Oxigraph dataset and in the N-Quads backup. The RDF/XML snapshot is the portable
instance view, not the audit log.

### 7.2 The RDF model

Managed instance data is typed RDF, not an opaque JSON payload. The existing `rdf::upsert`
payload pattern stays for compatibility with registries that have not migrated, but new `data.*`
writes use typed predicates and a small set of system predicates.

```rust
pub struct DataEntity {
    pub iri: String,
    pub types: Vec<String>,                 // rdf:type IRIs
    pub label: Option<String>,
    pub properties: BTreeMap<String, Vec<DataValue>>,
    pub graph: String,
    pub revision: u64,
    pub status: EntityStatus,               // active | deleted | superseded
    pub created_at: i64,
    pub updated_at: i64,
    pub recorded_by: String,                // user | cm | agent | system
}

pub enum DataValue {
    Iri(String),
    Literal { value: String, datatype: Option<String>, language: Option<String> },
}

pub struct DataPatch { pub predicate: String, pub values: Vec<DataValue> }

pub struct DataQuery {
    pub class: Option<String>,
    pub subject: Option<String>,
    pub filters: Vec<DataFilter>,
    pub graph: Option<String>,
    pub namespace: Option<String>,
    pub include_deleted: bool,
    pub limit: usize,
    pub offset: usize,
}
```

System predicates written by the broker, never invented by the CM:

| Predicate | Meaning |
|---|---|
| `mm:revision` | Monotonic per-entity revision; `data.update` checks it |
| `mm:createdAt`, `mm:updatedAt` | UTC instants |
| `mm:status` | `active`, `deleted`, or `superseded` |
| `mm:deletedAt`, `mm:deletedBy` | Soft-delete audit |
| `mm:recordedBy` | `user`, `cm`, `agent`, or `system` |
| `prov:wasGeneratedBy` | Link to the conversation turn / agent run that produced the fact |
| `mm:source` | Source IRI or literal, when known |
| `mm:sourceGraph` | Graph membership annotation in the RDF/XML snapshot |
| `mm:searchable` | `false` excludes an entity from the Tantivy index |
| `mm:confidence` | Optional 0..1 for inferred/uncertain facts |
| `mm:supersedes` | The prior entity/value this fact replaces |

Rules:

- Managed entities are IRIs. Blank nodes are allowed inside imported schemas but not for entities
  created through `data.create`.
- A repeated predicate is a list, in insertion order. There are no nested JSON objects in RDF;
  `data.create` accepts `{iri: ...}` references for object properties.
- Datatypes are chosen from the schema range (`xsd:string`, `xsd:integer`, `xsd:boolean`,
  `xsd:dateTime`, `xsd:double`). A value that cannot be coerced without loss is
  `SCHEMA_VIOLATION`; the broker never guesses.
- `data.update` writes a new revision and appends provenance. It does not rewrite the entity's
  history; old revisions remain queryable through the event log and, for soft-deleted entities,
  through `include_deleted`.

### 7.3 The schema catalog

The catalog is itself RDF in the `schema` graph. It is the source of truth for which schemas are
installed, which prefix owns which namespace, and which shapes validate which classes.

```rust
pub struct SchemaCatalog { engine: Arc<RdfEngine> }

impl SchemaCatalog {
    pub async fn list(&self) -> Result<Vec<SchemaSummary>>;
    pub async fn get(&self, id: &str, include: SchemaInclude) -> Result<SchemaDetail>;
    pub async fn register(&self, spec: SchemaSpec) -> Result<SchemaSummary>;
    pub async fn activate(&self, id: &str) -> Result<SchemaSummary>;
    pub async fn deprecate(&self, id: &str) -> Result<SchemaSummary>;
    pub async fn validate(&self, target: ValidationTarget) -> Result<Vec<ShaclViolation>>;
}
```

**Built-in schemas** (loaded at boot; cannot be rebound or removed):

| Prefix | Namespace | Role |
|---|---|---|
| `mm` | `https://metamind.dev/ontology#` | Metamind being, identity, plans, actions, data vocabulary |
| `mmd` | `https://metamind.dev/data/` | Instance IRIs |
| `mmg` | `https://metamind.dev/graph/` | Named graphs |
| `foaf` | `http://xmlns.com/foaf/0.1/` | People, names, accounts, relationships |
| `schema` | `https://schema.org/` | Public vocabulary aliases (`schema:Person`, `schema:name`) |
| `dcterms` | `http://purl.org/dc/terms/` | Titles, descriptions, dates, provenance |
| `doap` | `http://usefulinc.com/ns/doap#` | Projects, project names, repositories |
| `prov` | `http://www.w3.org/ns/prov#` | Attribution and derivation |
| `skos` | `http://www.w3.org/2004/02/skos/core#` | Labels, concepts, collections |
| `owl` | `http://www.w3.org/2002/07/owl#` | Class/property axioms, imports |
| `rdfs` | `http://www.w3.org/2000/01/rdf-schema#` | Labels, comments, subclass |
| `xsd` | `http://www.w3.org/2001/XMLSchema#` | Literal datatypes |

`dcat` and `vcard` are available as optional registered schemas, not active by default. `doap`
is built in because projects use `doap:Project`/`doap:name`.

Catalog entries carry `mm:schemaId`, `mm:schemaPrefix`, `mm:schemaNamespace`,
`mm:schemaVersion`, `mm:schemaSource`, `mm:schemaActive`, `mm:schemaEtag`, `mm:schemaLoadedAt`,
and `mm:owlImports`. `data.schema list` returns these without dumping the bodies. `data.schema get`
adds classes, properties, shapes, and an optional body, capped at 64 KiB.

**Lifecycle.** Registration parses the document with Oxigraph's RDF/XML or Turtle parser, extracts
the namespace, prefix, classes, properties, and imports, and records it inactive. Activation
loads its shapes and makes it eligible for routing and validation. Deprecation is refused while
instance data uses the schema; the error names the count and a migration is required. Standard
namespaces are immutable. Remote imports are refused unless the host is in
`MM_DATA_SCHEMA_ALLOWED_HOSTS`.

**Shapes.** Add `ontology/shapes/data.shacl.ttl`, `ontology/shapes/plan.shacl.ttl`, and extend
`ontology/shapes/mm-shapes.ttl` with `mm:UserShape`. The validator remains the existing strict
`rdf-shacl` path; unsupported SHACL annotations stay hard errors.

### 7.4 Standard naming conventions for identity

The system needs one spelling for the being, the operator, projects, plans, and actions. The
canonical table below is enforced by `data.namespace` and the broker; legacy IRIs are accepted as
aliases during migration. The broker checks the IRI shape for identity classes: `mm:User` must be
`mmd:user/<ULID>`, `mm:Project` must be `mmd:project/<ULID>`, and `mm:Plan` must be
`mmd:plan/<ULID>`; an explicit id that violates its class shape is `INVALID_ARGUMENTS`.

| Concept | Canonical IRI | Types | Canonical properties |
|---|---|---|---|
| Metamind being | `mmd:<ULID>` (existing identity IRI; immutable) | `foaf:Agent`, `mm:Being`, `mm:Identity` | `mm:iri` (self), `mm:name`, `mm:created`, `mm:selfDescription`, `mm:hasIdentity` (self) |
| Operator / user | `mmd:user/<ULID>` | `foaf:Person`, `mm:User` | `foaf:name`, `foaf:mbox` (a `mailto:` IRI), `mm:preferredName`, `mm:pronouns`, `mm:role`, `mm:locale`, `mm:timezone`, `mm:homeDir`, `mm:created` |
| Project | `mmd:project/<ULID>` | `doap:Project`, `mm:Project` | `doap:name`, `dcterms:description`, `mm:member`, `mm:homeDir` |
| Plan | `mmd:plan/<ULID>` | `mm:Plan` | `mm:goal`, `mm:planFormat`, `mm:planStatus`, `mm:currentVersion` |
| Plan version | `mmd:plan/<ULID>/v/<n>` | `mm:PlanVersion` | `mm:planBody`, `mm:planJson`, `mm:version`, `mm:createdAt` |
| Thought | `mmd:thought/<ULID>` | `mm:Thought` | `mm:question`, `mm:summary`, `mm:confidence`, `mm:evidence` |
| Proactive action | `mmd:action/<ULID>` | `mm:ProactiveAction` | `mm:actionKind`, `mm:input`, `mm:dueAt`, `mm:recurrence`, `mm:status` |
| Schema | `mmd:schema/<prefix>` | `mm:Schema` | `mm:schemaPrefix`, `mm:schemaNamespace`, `mm:schemaVersion` |
| Namespace | `mmd:namespace/<prefix>` | `mm:Namespace` | `mm:namespacePrefix`, `mm:namespaceIri` |
| Agent run | `mmd:agent/<run_id>` | `mm:AgentRun` | `mm:agentKind`, `mm:status`, `prov:wasStartedBy` |

**FOAF is canonical for people.** `mm:Being` becomes `rdfs:subClassOf foaf:Agent`, and `mm:User`
becomes `rdfs:subClassOf foaf:Person`, in `mm.ttl`. `schema:Person` and `schema:name` are accepted
on read through the schema catalog but normalized to `foaf:Person`/`foaf:name` on write. This
keeps one current value for a name, not two.

**Mapping everyday identity statements:**

| Operator says | Broker writes |
|---|---|
| "My name is Ada." | `foaf:name "Ada"` on the current user |
| "Call me Ada." | `mm:preferredName "Ada"` |
| "My email is ada@example.com." | `foaf:mbox <mailto:ada@example.com>` |
| "I use she/her." | `mm:pronouns "she/her"` |
| "I'm in Berlin; use metric." | `mm:timezone "Europe/Berlin"`, `mm:locale "de-DE"`, preference entries |
| "This is project Aurora." | an `mmd:project/<ULID>` with `doap:name "Aurora"` and a membership link |
| "Remember that the staging DB resets nightly." | a typed fact or `mm:Memory` with source, confidence, and timestamp |
| "That's wrong; it resets weekly." | `data.update` the fact; `mm:supersedes` the old value, or soft-delete it |

**Aliases.** Existing `urn:metamind:user:<id>`, `urn:metamind:project:<id>`, and
`urn:metamind:dataset:<id>` IRIs are accepted on read and resolved through
`data.namespace resolve`. New writes use the canonical `mmd:` form. Migration records
`owl:sameAs` in the schema graph, not in every entity, and `UserRegistry`/`ProjectRegistry` adopt
the canonical minting in Phase 3 and delegate writes to `DataService`. A person is never
duplicated because two spellings existed.

**Default graph routing.** `data.create` chooses the authoritative graph from the type and
namespace:

| Type / namespace | Graph |
|---|---|
| `mm:Being`, `mm:Identity` | `being` |
| `foaf:Person`, `mm:User` | `users/index` |
| `mm:Plan`, `mm:PlanVersion` | `plan` |
| `mm:ProactiveAction` | `action` |
| `mm:Schema`, `mm:Namespace` | `schema` |
| subclass of `mm:Memory` | `memory` |
| `mm:Claim`, `mm:Evidence`, `mm:Observation` | `epistemic` |
| everything else | `world` |

### 7.5 Tantivy full-text index

New crate `crates/mm-search`, with Tantivy as the only backend. The index is a derived read model:
if it is lost, it is rebuilt from RDF; it is never a second source of truth.

```rust
pub struct SearchIndex { writer: Mutex<IndexWriter>, reader: IndexReader, schema: Schema }

impl SearchIndex {
    pub fn open(dir: &Path) -> Result<Self>;
    pub async fn upsert(&self, doc: SearchDocument) -> Result<()>;
    pub async fn delete(&self, subject: &str) -> Result<()>;
    pub async fn search(&self, query: SearchQuery) -> Result<SearchPage>;
    pub async fn reindex(&self, entities: Vec<DataEntity>) -> Result<IndexReport>;
}

pub struct SearchDocument {
    pub subject: String,          // stored, indexed, unique key
    pub graph: String,            // stored, indexed, facet
    pub types: Vec<String>,       // stored, indexed, facet
    pub label: String,            // stored, indexed, boosted
    pub body: String,             // stored, indexed, capped at 32 KiB
    pub namespace: String,        // stored, indexed
    pub revision: u64,            // stored, fast
    pub updated_at: i64,          // stored, fast
    pub status: String,           // active | deleted | superseded
}
```

Index schema:

| Field | Type | Options |
|---|---|---|
| `subject` | STRING | stored, indexed, unique |
| `graph` | STRING | stored, indexed, facet |
| `types` | STRING (multi) | stored, indexed, facet |
| `label` | TEXT | stored, indexed, `en_stem`, boost 3 |
| `body` | TEXT | stored, indexed, `en_stem` |
| `namespace` | STRING | stored, indexed |
| `revision` | u64 | stored, fast |
| `updated_at` | i64 | stored, fast |
| `status` | STRING | stored, indexed, facet |

**Document construction.** The label comes from `rdfs:label`, `foaf:name`, `schema:name`, or
`mm:name`, in that order. The body is every literal value on the entity plus its class labels and
property labels, truncated at 32 KiB on a character boundary. IRIs are not tokenized into the body
except for the subject/namespace fields. Secrets and fields marked `mm:searchable false` are never
indexed.

**Query construction.** `data.search` parses a bounded query against `label` and `body`, boosts
`label`, and applies `class`, `graph`, and `namespace` as filters. It never accepts raw Lucene or
Tantivy query syntax from the model. Top hits are fetched from the index and then verified against
the RDF store; a hit whose entity is missing, deleted, or has a newer revision is dropped and
queued for repair.

**Writer and reader.** Tantivy has a single writer; `mm-search` serializes it behind an async mutex
and commits after each batch. The reader reloads on commit. Writes are short; a 500-document batch
is the maximum. `reindex` is an operator/startup path, not a CM tool.

### 7.6 Keeping RDF and Tantivy in sync

A write and its index document are one logical operation, but they are two physical systems. The
design uses **RDF as authority + a durable outbox + synchronous best-effort index + verification**:

```
data.create/update/delete
  1. validate intent + schema + revision; allocate dataRevision and idempotency key
  2. render quads and the search document
  3. write data_outbox row (pending) with dataRevision, op, subject, doc_json
  4. Oxigraph transaction: insert/delete quads + provenance + dataRevision
  5. append DataMutated event (agentstream)
  6. attempt Tantivy delete+add (or delete) and commit
  7. if index succeeds -> mark outbox row done, return indexed:true
     if index fails   -> leave outbox row pending, return indexed:false, INDEX_PENDING
```

- The outbox row is written **before** the RDF commit, so a crash after the RDF commit but before
  indexing leaves a pending row. The index worker checks the RDF entity's `mm:revision`; a row
  whose RDF write did not land is discarded, and a row whose RDF write did land is indexed. This
  closes the crash window that a post-commit outbox insert would leave.
- The outbox is `data_outbox(seq, op, subject, doc_json, attempts, next_attempt_at, created_at)` in
  `sqlite/metamind.db`. `seq` is the `dataRevision`. The worker processes rows in order, deletes
  the old document, adds the new one, commits, then marks the row done.
- A failed index never rolls back RDF. The tool result says `indexed:false` and `indexPending:true`;
  `data.search` waits up to 2 s for the caller's `dataRevision` to become visible before returning
  `INDEX_PENDING`. This prevents a create followed by a search from looking like a lost write.
- Deletes write a tombstone outbox row. If the index delete fails, the stale document remains, but
  `data.search` verifies every hit against RDF and drops it. A repair job deletes the stale doc.
- On boot, if the Tantivy index is missing, empty, or its stored `schema_version`/`dataRevision`
  does not match, the service rebuilds from RDF in the background and marks `search.ready=false`
  until it catches up. `data.search` returns `SEARCH_UNAVAILABLE` while not ready, not a silent
  empty result.
- Reconciliation is count + sample + revision, not a full byte-for-byte diff. `mm-cli data verify`
  reports missing docs, orphan docs, and revision mismatches; `mm-cli data reindex` rebuilds.

**RDF/XML snapshot path.** Every 256 mutations or 60 seconds (whichever first), and on clean
shutdown, `RdfEngine::dump_rdfxml` writes `metamind.rdf` atomically (temp file + rename). The
snapshot is a managed-instance export with `mm:sourceGraph` on each subject; the lossless quad
backup `metamind.nq` is written at the same checkpoint. On boot with an empty RocksDB store, the
snapshot is imported through the same schema/namespace validation as `data.create`; it is never
blindly loaded. The snapshot is never imported over a non-empty store.

### 7.7 Automatic data management and identity bootstrap

The prompt policy in §6.9 is backed by three mechanisms:

1. **Visible data status.** `[STATE]` includes a compact `data` projection: entity count, search
   document count, index lag, `dataRevision`, active schema prefixes, and the last mutation. The
   model can see whether the graph is empty, stale, or unhealthy.
2. **Search-before-create.** The stable prompt says to search first; the broker also enforces a
   cheap duplicate guard: if a create has the same primary label and type as an existing active
   entity in the same graph, it returns the existing entity with `duplicate:true` instead of
   creating a second one. The CM can then update it.
3. **Identity seeding.** On boot, `DataService::ensure_identity` reads the `BeingFacade` and the
   session user. If the being has no `mm:Being` instance it creates one from the immutable
   identity; if the user has no `foaf:Person` it creates a placeholder with `mm:userId` and no
   invented name. The CM asks for the name and fills it with `data.update`.

The stable prompt's data paragraph is the only general instruction. There are no per-turn
"remember this" reminders. Automatic capture is measured by evals (§15.3): durable fact captured,
duplicate avoided, correction superseded, secret refused, and search used before answer.

---

## 8. Metamind planning, inner thinking, and proactive action

### 8.1 The three services

| Tool | Service | Runtime | Output |
|---|---|---|---|
| `metamind.plan` | `PlanService` in `mm-cme/src/subagent.rs` | bounded planning sub-session (`plan/<id>`) | `PlanResult` + `mm:PlanVersion` |
| `metamind.think` | `ThinkService` in `mm-cme/src/subagent.rs` | bounded inner-thought sub-session (`think/<id>`) | `ThinkResult` (outer CM speaks) |
| `metamind.act` | `ActionService` in `mm-serve/src/action_service.rs` | immediate `ProactiveEngine` handler or scheduler | `ActionReceipt` |

`mm-cme` owns the sub-session loop because it already owns the model client, budgets, and turn
state machine. `mm-serve` owns the tool handlers and wires `DataService`, `SearchIndex`,
`PlanStore`, and `ActionService` into `AppState`. `mm-data` owns plan/action storage and search
indexing; `mm-search` owns Tantivy. No dependency points from `mm-data` back to `mm-cme`.

### 8.2 Plans are durable, versioned objects

```rust
pub struct Plan {
    pub plan_id: String,                  // mmd:plan/<ULID>
    pub goal: String,
    pub format: PlanFormat,               // markdown | structured
    pub status: PlanStatus,               // draft | active | completed | abandoned
    pub current_version: u32,
    pub revision: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct PlanVersion {
    pub plan_id: String,
    pub version: u32,
    pub format: PlanFormat,
    pub body: String,                     // markdown or canonical JSON
    pub steps: Vec<PlanStep>,             // structured only
    pub created_at: i64,
    pub created_by: String,               // cm | user | action
}

pub struct PlanStep {
    pub id: String,                       // s1, s2, …
    pub title: String,
    pub detail: String,
    pub depends_on: Vec<String>,
    pub verification: Option<String>,
    pub owner: PlanOwner,                 // cm | agent | operator
}
```

- Plans live in the `plan` graph. `mm:currentVersion` points at the active `mm:PlanVersion`; old
  versions are immutable. `data.query` can read them; `metamind.plan` is the writer.
- **Planning mode is bounded, not a permanent switch.** The tool enters planning mode for the
  duration of the `PlanService` sub-session, returns the plan, and exits. The outer CM's
  conversation state is unchanged unless the CM chooses to set `phase`; the planning session is
  visible in the Plan tab and the trace selector.
- `create` with no content starts a planning sub-session. It receives the goal, the conversation's
  continuity block, and the current `[STATE]` data projection. It may call the read-only data
  tools. It returns markdown or `mm.plan.v1` structured JSON, which is validated before storage.
- `update` requires `expected_revision`; it creates a new version, never edits the old one. A
  markdown update replaces the body; a structured update may replace steps, assumptions, risks,
  and success criteria.
- The planning sub-session is not the CM's voice. Its system prompt says: "You are planning, not
  speaking. Produce a plan the conversation manager can execute. Do not address the operator."
- Planning mode is bounded: 90 s, 8 model rounds, 16 k tokens, read-only tools. If it cannot finish,
  it returns a partial plan with `complete:false` and explicit gaps; the CM can update later.
- `metamind.plan` does not execute the plan. Execution is `agent.run` for external work or
  `metamind.act` for proactive actions. The CM chooses.

### 8.3 Inner thought is a separate sub-agent

```
outer CM (persona, user-facing)
   │  metamind.think(question, focus)
   ▼
ThinkService ──► think/<id> agentstream
   │  read-only data.query / data.search / data.schema / data.namespace
   │  bounded depth 1..3, 4 tool rounds, 90 s
   ▼
ThinkResult { summary, findings[], evidence[], confidence, gaps }
   │
   └──► outer CM composes the reply in character
```

The inner thinker's prompt is deliberately different from the CM's:

> You are Metamind's inner reasoning process, not the speaker. You may inspect data with the
> read-only tools. Do not address the operator, do not adopt the persona, and do not produce a
> chat reply. Return the strongest answer you can support, with evidence IRIs, confidence, and
> explicit gaps. If the data does not support a claim, say so.

- The inner thinker cannot write data, cannot plan, cannot act, cannot call `agent.run`, and cannot
  call `metamind.think` recursively. This prevents loops and side effects.
- The outer CM receives `summary`, `findings`, `evidence`, `confidence`, and `gaps`. It must not
  expose the inner monologue verbatim; it uses it to answer in character. The raw trace is in the
  inspector only.
- `persist:true` stores an `mm:Thought` in the `memory` graph with question, summary, evidence,
  confidence, and provenance. `persist:false` keeps only the agentstream trace and metrics.
- If the thinker returns no evidence, the CM says so or asks a clarifying question; it does not
  convert a guess into a fact.

### 8.4 Proactive actions are an allowlisted catalog

`metamind.act` does not accept arbitrary jobs. It executes one of the installed actions:

| Action | What it does | Data / side effects |
|---|---|---|
| `search_lookup` | Search the knowledge graph and summarize what is missing | `data.search`, `data.query`; may delegate external lookup through `agent.run` when explicitly configured |
| `goal_narrowing` | Turn a broad goal into one actionable next step | reads goals/plans; may update a plan through `PlanService` |
| `plan_refinement` | Add verification/dependencies to an existing plan | `PlanService.update`; plan version |
| `pattern_watcher` | Look for a repeated pattern across memories and facts | `data.search`, `data.query`; may create `mm:Pattern` |
| `affect_responder` | Prepare a brief acknowledgement of the operator's affect | conversation affect; no external side effect |
| `knowledge_gap` | Identify a question the graph cannot answer and prepare a question | `data.search`; may create `mm:OpenQuestion` |

- The catalog is code, not model output. A new action is a reviewed handler plus a schema entry,
  not a new tool.
- `immediate` runs the handler through `ProactiveEngine` and returns a receipt. `scheduled` writes
  an `mm:ProactiveAction` with `mm:status "scheduled"`, `mm:dueAt`, `mm:recurrence`, `mm:input`,
  `mm:origin`, and an idempotency key. The existing proactive region tick polls due actions every
  30 s (with jitter), claims one at a time, and records `running`/`done`/`failed`/`cancelled`.
- Scheduled actions are visible in the inspector and cancelable by the operator. The CM has no
  cancel action; `metamind.act` can schedule but not cancel.
- Actions inherit the same budgets and audit as `agent.run`. Any external side effect must be
  inside the sandbox or require operator confirmation. An action never gets a shell it did not
  already have through `agent.run`.
- A recurring action is capped at 30 days and 100 occurrences; the scheduler expires it rather than
  running forever.

### 8.5 How the three tools work together

```
operator: "Figure out why the refresh test is flaky and fix it."
  CM: metamind.plan(goal="make the refresh test deterministic")
      -> plan v1: reproduce, instrument, fix, verify
  CM: metamind.think(question="which clock path is used by the test?", focus=[...])
      -> findings: wall clock; no test clock
  CM: agent.run("Reproduce ...")
      -> external work
  CM: data.create(type="mm:Fact", properties={...})
      -> records the durable cause
  CM: metamind.act(action="plan_refinement", mode="scheduled", delay_seconds=3600)
      -> follow-up to re-check the plan
```

Each tool has one job. `metamind.plan` produces a durable plan, `metamind.think` produces private
analysis, `metamind.act` runs or schedules a proactive action, `data.*` manages knowledge, and
`agent.run` is the only external work door. The CM composes the operator-facing reply.

---

## 9. The `agent.run` service and downstream agents

### 9.1 The agent service

New crate `crates/mm-agent`. The CM depends on one narrow trait:

```rust
#[async_trait]
pub trait AgentService: Send + Sync {
    async fn run(&self, request: AgentRequest) -> AgentResult<AgentTurnResult>;
    async fn status(&self, run_id: &str) -> AgentResult<AgentRun>;
    async fn cancel(&self, run_id: &str) -> AgentResult<AgentRun>;
}

pub struct AgentRequest {
    pub input: String,                   // natural language
    pub run_id: Option<String>,          // None = start
    pub kind: AgentKind,                 // Hermes | Pi
    pub workspace: Option<AgentWorkspace>,
    pub parent_session: String,
}

pub struct AgentRun {
    pub run_id: String,
    pub kind: AgentKind,
    pub session_id: String,              // agentstream session id: "agent/<run_id>"
    pub status: AgentStatus,             // running | waiting_input | done | failed | cancelled
    pub turn: u32,
    pub task: String,                    // the original natural-language input
    pub workspace: Option<String>,
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
    pub summary: Option<AgentTurnSummary>,
    pub tool_calls: Vec<AgentToolSummary>,
    pub artifacts: Vec<ArtifactRef>,
    pub usage: AgentUsage,
    pub next: AgentNext,                 // continue | wait_for_input | done | failed
    pub meta: AgentTurnMeta,             // streamed vs non-streamed, model name
}

pub struct AgentTurnMeta {
    pub streamed: bool,
    pub model: Option<String>,
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

### 9.2 Hermes backend (OpenAI API)

Hermes is already running as a service. The design makes no assumption about its internal API
beyond **OpenAI-compatible chat completions**, which the operator explicitly named.

**Resolving the running service.** The design does not hard-code a port. On boot, `mm-agent`
resolves Hermes in this order:

1. `MM_AGENT_HERMES_BASE_URL` (explicit; wins);
2. the `[hermes]` table in `config/agents.toml` (endpoint and model, no secret);
3. otherwise the backend is `UNAVAILABLE` with the reason `MM_AGENT_HERMES_BASE_URL is unset`.

At startup it probes `GET {base}/models` with a short timeout. A successful probe records the
model list and marks the backend `ok`; a failed probe leaves it `unavailable` with the HTTP status
or connection error. The status panel shows that reason verbatim. There is no silent fallback to
another provider and no fabricated run.

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

- A new run sends `POST /chat/completions` with a system prompt describing the task, workspace,
  and the agent's own tool policy. The response is stored as the first assistant turn; all
  messages are persisted in the run's agentstream session.
- A continuation sends the accumulated `messages` (rebuilt from agentstream, not trusted to a
  server-side session) plus the CM's natural-language input. This makes the run reproducible and
  independent of Hermes session retention.
- If the service supports it, an `X-MM-Agent-Session` / conversation id header is recorded for
  correlation but **not required** for correctness.
- Streaming: parse `data:` frames, accumulate `delta.content` and `delta.tool_calls`, handle
  `[DONE]`, and log an `AgentMessage` event per completed turn plus `ToolCall`/`ToolResult` for any
  tool call Hermes performs internally. The CM never sees raw deltas.
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

### 9.3 Pi backend (programmatic RPC)

Pi is already integrated in `mm-pi` (`PiClient`, `PiCommand::{Prompt, Steer, FollowUp, Abort,
ClearQueue, NewSession}`, `PiEvent`). `mm-agent/src/pi.rs` adapts it:

- A new run spawns `PiClient` with `PiConfig`:
  - `cwd` = the requested workspace (must be inside `data/sandbox` unless an explicit
    `MM_AGENT_ALLOWED_ROOTS` entry covers it),
  - `session_dir` = `data/sandbox/pi/<run_id>`,
  - tools = the default read/write/edit/list set; no shell, matching `PiConfig::default_tools()`,
  - provider/model from `MM_AGENT_PI_PROVIDER` / `MM_AGENT_PI_MODEL`.
- A continuation sends `PiCommand::Prompt` with a correlation id; cancellation sends `Abort`;
  closing sends `ClearQueue` then drops the client (which kills the process via `kill_on_drop`).
- Every `PiEvent` is mapped to the same `AgentTurnResult` shape. Pi's own tool records become
  `AgentToolSummary` rows; the raw event is retained in the agentstream.
- If the binary is absent, a new run returns `UNAVAILABLE` with the exact reason (matching Pi's
  existing spawn error), never a fake run.
- The `pi.ingest` path is retained for recording an existing Pi session as an agentstream run;
  it is an operator import, not a CM tool.

### 9.4 The turn-by-turn contract

The CM never waits for a whole job. Each `agent.run` call performs **at most one downstream turn**:

```
CM: agent.run(input="Reproduce the failing auth test and report the cause",
              agent="hermes", workspace="/…/sandbox")
    → {runId:"01R", turn:1, status:"running", message:"…", summary?}
CM: agent.run(run_id="01R", input="Also try the flaky seed 42.")
    → {runId:"01R", turn:2, status:"running", message:"…", summary?}
CM: agent.run(run_id="01R", input="Continue.")
    → {runId:"01R", turn:3, status:"done", message:"Cause: …", summary:"…", artifacts:[file]}
```

- `input` is always natural language. `"Continue."` is valid.
- `max_turns` is enforced per run; exceeding it returns `status: failed` with `code: BUDGET` and
  the transcript preserved.
- If the agent asks a question (`status: waiting_input`), the CM must relay it to the operator and
  stop the loop with `next: wait_for_user`. It must not invent an answer.
- `cancel` is honored within one turn boundary; the backend receives `Abort`/`POST /cancel` and
  the run ends `cancelled`.
- Terminal runs are immutable except for a new run. A `done` run cannot be continued.

### 9.5 Per-turn summarization

Long agent turns are summarized **separately** before the CM sees them:

```
if turn_tokens > MM_AGENT_SUMMARY_THRESHOLD (default 6000)
   or turn_chars > 24000:
       summary = mm_llm.summarize(turn, purpose=Summarize, schema=AgentTurnSummary)
       log AgentTurnSummary event
       AgentTurnResult.message = summary.headline + " " + summary.next_recommended?
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
    pub artifacts: Vec<ArtifactRef>,
    pub blockers: Vec<String>,
    pub next_recommended: Option<String>,
    pub confidence: f32,           // 0..1
}
```

Rules:

- Summaries are generated from the raw turn plus the previous rolling run summary, so a 30-turn
  run does not require the CM to read 30 raw turns.
- The raw turn is never deleted; the summary links to it.
- The summarizer prompt instructs: "Do not omit a command that failed, a file path, or an
  unresolved error. If the turn contains a question, put it in `blockers`."
- If summarization fails (model unavailable), fall back to a deterministic extractor: first 2,000
  chars, last 2,000 chars, every tool call name, and every artifact path, with a
  `summary_failed: true` marker. The CM is told the summary is degraded.

### 9.6 Separate agentstream sessions

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

The CM's `agent.run` result is the only bridge. The trace viewer reads the agent session; the
status panel reads the run registry; the CM reads the compact turn result and the state block's
`agents` projection.

### 9.7 Agent lifecycle state machine

```
            new run
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
- Terminal states are immutable except for a new run.
- A crash of `mm-serve` leaves the run record `running` with a stale `updated_at`; on boot the
  registry marks runs older than the process start as `failed` with reason `"server restarted"`.
  The transcript remains in agentstream. This is honest and inspectable.

### 9.8 Limits, safety, and configuration

| Concern | Rule |
|---|---|
| Workspace | Default root is `data/sandbox/agents/<run_id>`; an explicit root is accepted only under `MM_AGENT_ALLOWED_ROOTS` (colon-separated absolute paths). Paths are canonicalized and must not escape. |
| Confirmation | A new run is allowed without a prompt when the workspace is inside the sandbox and `input` is non-empty. A root outside the sandbox returns `CONFIRMATION_REQUIRED`; the inspector shows an approve/reject action, and the CM retries after approval. Continuations never prompt. |
| Secrets | The Hermes key is read from the environment only. It is never written to agentstream, tool results, or logs. A redaction pass runs over every agent event payload. |
| Network | Only the configured Hermes base URL. No arbitrary URL argument in `agent.run`. |
| Concurrency | Per session, at most `MM_AGENT_MAX_ACTIVE_RUNS` (default 3) runs. Additional new runs return `BUDGET`. |
| Cancellation | Operator stop cancels the active CM turn and sends `cancel` to every run started by it. The CM has no cancel tool. |
| Replay | The agentstream is the transcript of record. A run can be reconstructed from its events without the backend. |
| Data retention | Raw events follow the existing agentstream policy. Summaries are retained with the run record. |

---

## 10. The conversation loop and personalization

### 10.1 Prompt assembly

`mm-cme/src/prompt.rs` (new) builds `PromptPacket` from:

| Source | Fields | Failure behavior |
|---|---|---|
| `BeingFacade` | self-description, version, invariants, values, voice | `being unavailable: <reason>` |
| User model | preferred name, expertise, preferences, current priorities | omit with `user model unavailable` |
| Relationship | familiarity, trust, open issues | omit with a reason |
| `SessionContext.affect` | valence/arousal/confidence → tone line | defaults |
| `SessionContext.memory` | project summary, previous topics, greeting | omit with a reason |
| `ConversationStore` | topic, depth, status variables, revision | `[STATE unavailable: reason]` |
| `AgentRegistry` | active runs and last turn summaries | empty list with a reason |
| `DataService` | data revision, entity/document count, index lag, schemas, last mutation | `data unavailable: reason` |
| `PlanStore` | current plan header, version, status, step count | omit with a reason |
| `ActionService` | scheduled/running action counts | omit with a reason |
| Recent transcript | last N turns, compressed if long | existing compression |
| `TurnPlan` | query, modality, detail, budget, loop policy | required |

The packet is inserted as:

- one **stable system message** (identity, relationship, affect, continuity, invariants, the
  state-ownership sentence, the data-capture policy, and the metamind division of labor);
- one **state system message** (`[STATE …]` + canonical JSON), replaced before every model call;
- the transcript;
- the operator's user message.

The tools array contains exactly fourteen schemas. The operator's query remains a user message; it
is never merged into the state block.

### 10.2 The turn algorithm

```
1. Session receives StartTurn(query).
2. Planner builds TurnPlan; chooses LoopMode.
3. StateView = load ConversationState + AgentRegistry + DataService health + PlanStore + ActionService + health snapshot.
4. PromptBuilder renders stable prefix + [STATE rev] + transcript + user query.
5. Executor calls the model with 14 tool schemas.
6. While the model returns tool calls:
     a. validate strict JSON; unknown fields fail
     b. execute through the fourteen-tool dispatch
     c. state writes save with expected_revision; return full state
     d. data writes validate/schema-check/write/index; return dataRevision and index health
     e. metamind.plan/think run bounded sub-sessions; metamind.act runs/schedules
     f. agent.run executes at most one downstream turn
     g. reload + re-render [STATE] before the next model call
     h. append the typed envelope to the transcript
     i. check per-family quotas and loop budget; on exhaustion -> paused outcome
7. Before final text:
     a. if an agent run started this turn is still `running`, force another round
        (honesty rule; not a state-tool trigger)
     b. do not inject any topic/status/depth reminder
     c. if a data write returned `indexPending`, do not claim search is current
8. Stream the final text.
9. Persist the turn and any state/data/plan/action revisions; emit a status revision and chat
   completion frame.
```

The loop contract in the stable prompt:

> You are the conversation manager. You may call any of the fourteen tools as many times as needed
> in this turn. You own the conversation state in `[STATE]`; keep it true as the conversation
> moves. Treat durable facts as ontology data; search before creating, update instead of
> duplicating, and never store secrets. Use `metamind.plan` for durable plans, `metamind.think` for
> private analysis, and `metamind.act` only for an installed proactive action. Never claim external
> work is finished while a run you started is still active. If you cannot finish within the budget,
> say what remains and stop honestly.

There is no sentence that names a state tool or a trigger condition. The loop contract names the
tools only as "any of the fourteen"; the schemas describe them. The data-capture and metamind
sentences are general policy, not per-turn triggers.

### 10.3 Why this feels personal

- **Identity is sourced from the being, not invented.** Self-description, values, and invariants
  are the actual persisted being, so the CM's voice is continuous across sessions.
- **The operator is a person in the prompt, not a `user` role.** Preferred name, expertise,
  communication preferences, and current priorities are rendered as relationship context.
- **Continuity is explicit.** The previous project narrative, previous topics, and open
  commitments are shown. The CM can say "we left the refresh test flaky" instead of starting cold.
- **Affect is a control signal, not a claim.** The prompt maps affect to tone guidance ("steady,
  not enthusiastic") and keeps identity invariants above it.
- **The CM sees its own state.** The `[STATE]` block is the same state the panel shows: topic,
  depth, phase, intent, tasks, next actions, blockers, questions, active agents, data health,
  current plan, and scheduled actions. The model can answer "where are we?" without guessing from
  the transcript.
- **The operator is a person in the graph, not just a prompt label.** `foaf:Person` + `mm:User`
  gives the CM a durable name, contact, role, locale, timezone, and preferences. It updates them
  through `data.*` rather than treating the user model as a read-only blob.
- **Personal memory is used, not merely stored.** The prompt includes the project's open
  commitments and the last session's continuation; durable facts go into the graph, episodic
  memory stays in the existing memory pipeline.

Guardrails: the CM never fabricates autobiographical memory, never claims an external action
without an agent/tool result, and never converts an assumption into an observation. The prompt
states these invariants verbatim from the being.

### 10.4 Cross-session continuity

On session start, `CmeRuntime::ensure_session` loads:

1. the latest `ProjectMemory` for the bound project,
2. the previous session's `ConversationState` (topic, depth, status variables),
3. the user model and relationship state,
4. any open commitments.

The previous state is rendered in the **continuity block** as "last session" context, not injected
as current `[STATE]`. The new session's state starts empty until the CM writes it. This avoids
presenting stale topic/status as current truth. If no previous context exists, the continuity block
is omitted with a reason, not populated with invented history.

---

## 11. Live status backend

### 11.1 The view model

`mm-serve/src/status.rs` (new) composes one view per conversation session:

```rust
pub struct ConversationStatusView {
    pub revision: u64,                  // view revision (status service)
    pub session_id: String,
    pub generated_at: i64,
    pub state: ConversationStateView,
    pub conversation: ConversationView,
    pub metamind: MetamindView,
    pub data: DataView,                 // entity/search/schema health
    pub plans: PlansView,               // current plan header + recent plans
    pub actions: ActionsView,           // scheduled/immediate proactive actions
    pub agents: AgentsView,
    pub operations: OperationsView,
    pub variables: VariablesView,       // runtime flags/prefs/limits, not conversation status
}

pub struct ConversationStateView {
    pub revision: u64,                  // conversation state revision
    pub topic: Option<String>,
    pub depth: u8,
    pub status: BTreeMap<String, StatusValue>,
    pub status_count: usize,
    pub status_bytes: usize,
    pub updated_at: i64,
    pub origin: String,                 // user | cm | system
}

pub struct ConversationView {
    pub turn: Option<TurnView>,         // id, state, phase, model, elapsed_ms
    pub turns_completed: u32,
    pub pending_confirmation: Option<String>,
    pub loop: LoopView,                 // mode, round, max_rounds, remaining_ms
}
```

`ConversationStateView` is a direct projection of `ConversationState`; it does not add a second
topic tree or a task board. The "upcoming tasks" section of the panel is rendered from the
`tasks` and `next` status variables. If the CM has not set them, the panel says "not set" rather
than inventing a list.

`MetamindView`, `AgentsView`, and `OperationsView` keep the shape from the previous design. The
data, plan, and action views are additive:

```rust
pub struct MetamindView {
    pub kernel: KernelView,
    pub being: BeingView,
    pub budgets: Vec<BudgetView>,
    pub memory: MemoryView,
    pub llm: LlmView,
    pub cme: CmeView,
}

pub struct DataView {
    pub revision: u64,                  // dataRevision
    pub entities: u64,
    pub documents: u64,
    pub index_lag_ms: u64,
    pub index_ready: bool,
    pub outbox_pending: u64,
    pub schemas: Vec<SchemaSummaryView>,// prefix, namespace, version, active
    pub namespaces: Vec<NamespaceView>,
    pub last_mutation: Option<DataMutationView>,
    pub unavailable: Option<String>,
}

pub struct PlansView {
    pub current: Option<PlanSummaryView>,// id, goal, format, status, version, steps, updated_at
    pub recent: Vec<PlanSummaryView>,    // last 5
}

pub struct ActionsView {
    pub scheduled: Vec<ActionSummaryView>, // dueAt, action, status, priority
    pub recent: Vec<ActionSummaryView>,    // last 10
    pub running: Vec<ActionSummaryView>,
}

pub struct AgentsView {
    pub active: Vec<AgentRunView>,
    pub recent: Vec<AgentRunView>,      // last 5 terminal runs
}

pub struct OperationsView {
    pub recent: Vec<OperationView>,     // last 20, newest first
    pub active: Vec<OperationView>,     // in-flight tool calls / agent turns
}

pub struct VariablesView {
    pub feature_flags: BTreeMap<String, bool>,
    pub preferences: BTreeMap<String, f32>,
    pub counters: BTreeMap<String, u64>, // turns, tool calls, agent turns, cache hits
    pub limits: BTreeMap<String, u64>,
}
```

**Three revisions, one frame.**

- `stateRevision` bumps only on topic/depth/status writes.
- `dataRevision` bumps only on data/schema/namespace mutations and index acknowledgements.
- `revision` (view revision) bumps on any section change, including state, data, plans, actions,
  agents, operations, metamind, and conversation.
- The client applies frames by `revision`; the state block and the panel both display
  `stateRevision`/`dataRevision` so the operator can correlate a frame with the underlying write.

### 11.2 The status service

The naive implementation — compose everything on every request — would hit RDF, SQLite, the being
mutex, CME observability, and the agent registry several times per second. The design avoids that.

```
                 ┌────────────── StatusService (per session) ─────────────┐
 CME transition ─┤ dirty: conversation                                     │
 state write    ─┤ dirty: state                                            │
 agent event    ─┤ dirty: agents, operations                               │
 health tick    ─┤ dirty: metamind.kernel                                   │
 tool call      ─┤ dirty: operations                                       │
                 │                                                        │
                 │  coalesce 100ms → recompute dirty sections only        │
                 │  assign revision → store snapshot → broadcast frame    │
                 └────────────────────────────────────────────────────────┘
```

- **Dirty flags per section.** `state`, `conversation`, `agents`, `operations`, `metamind`,
  `data`, `plans`, `actions`, `variables`. A state write does not re-read the being or the graph;
  a data write does not re-read the conversation state. Data dirty flags are coalesced separately
  because a single data transaction can emit both a mutation and an index event.
- **Coalescing.** A 100 ms debounce collects a burst of tool events into one revision. The UI
  receives at most 10 revisions per second; under load the service drops intermediate revisions
  and always sends the latest (the client only needs the newest complete view).
- **Revision.** Monotonic per session, starting at 1. Every frame carries `revision` and
  `generated_at`. The client ignores frames with `revision <= last_applied`.
- **Snapshot cache.** `RwLock<ConversationStatusView>` plus per-section `updated_at`. A full GET
  returns the cached view; if the cache is colder than `MM_STATUS_MAX_AGE_MS` (default 2 s), it
  recomputes synchronously before answering.
- **No user-specific filtering** in this design: the console is a single-operator surface
  (`CONSOLE_USER`). Multi-user filtering is a later extension; the view model keeps `session_id`
  and project id so it can be added without reshaping the wire.

### 11.3 Endpoints

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/console/sessions/{session_id}/state` | Full `ConversationStateView` (exact state, no derived view) |
| `GET` | `/api/console/sessions/{session_id}/status` | Full `ConversationStatusView` |
| `GET` | `/api/console/sessions/{session_id}/status/stream?from=<revision>` | SSE: `status` frames, `heartbeat` every 10 s |
| `GET` | `/api/console/sessions/{session_id}/operations?limit=50` | Operation history (paged) |
| `GET` | `/api/console/data/health` | Entity/document counts, index lag, outbox, schema count |
| `GET` | `/api/console/data/entities?class=&q=&limit=` | Paged entity list/search for the knowledge panel |
| `GET` | `/api/console/data/schemas` | Schema catalog summary |
| `GET` | `/api/console/plans` | Plan list, current first |
| `GET` | `/api/console/plans/{plan_id}` | Plan header + versions + body |
| `GET` | `/api/console/actions` | Scheduled/running/recent actions |
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

### 11.4 SSE frame format

```
event: status
id: 42
data: {"revision":42,"stateRevision":17,"dataRevision":88,"sessionId":"01…","generatedAt":1763…,"state":{…},"conversation":{…},"data":{…},"plans":{…},"actions":{…},"agents":{…},"operations":{…},"metamind":{…}}

event: heartbeat
id: 42
data: {"revision":42,"stateRevision":17,"dataRevision":88,"at":1763…}
```

Data mutations and plan/action changes bump the view `revision` even when `stateRevision` does not
change. This is how the knowledge panel updates while the conversation state is idle.

- The first frame after connect is always a **full** view, even when `from` is current. This
  removes a whole class of client bugs where a delta arrives before the initial snapshot.
- When `from` is older than the retained history (default 256 revisions), the server sends a full
  view with `reason: "resync"`.
- Frames are ordered by `revision`; the client never reorders because the transport preserves
  order. The client still ignores stale revisions defensively.
- A slow client is disconnected after `MM_STATUS_CLIENT_LAG_LIMIT` (default 64 pending frames) and
  reconnects to a full snapshot. This prevents unbounded memory growth.

### 11.5 Performance and failure behavior

- Target: p95 status recompute < 10 ms; p95 full GET < 30 ms; SSE event delivery < 250 ms after
  the underlying mutation.
- Every section carries its own `unavailable` reason. A closed being store does not fail the whole
  panel; it renders "Being unavailable: reason" while the rest updates.
- The service never blocks the turn path. Tool/agent execution publishes to an in-process
  broadcast channel; the status service consumes asynchronously. If the status channel is full,
  the service marks the section dirty and recomputes on the next tick; the turn is unaffected.
- The health poll remains for the top bar, but the inspector does not depend on it.

---

## 12. Frontend: the Inspector rail

### 12.1 Layout

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
  `Status`, `Knowledge`, `Plan`, `Trace`.
- Persisted state: `inspectorOpen`, `inspectorWidth`, `inspectorTab` in a new `useInspectorStore`.
- Keyboard: `⌘⇧I` toggles the inspector; `⌘⇧1` … `⌘⇧4` select Status / Knowledge / Plan / Trace.
  (Existing `⌘1`/`⌘2` remain pane focus.)
- Accessibility: `<aside aria-label="Inspector">`; tabs implement `role="tablist"` /
  `role="tab"` / `role="tabpanel"`; the live region is `aria-live="polite"` and only announces
  the topic headline, not every counter.

### 12.2 Status panel anatomy

Top to bottom, exactly matching the requested priority:

```
┌ Inspector ───────────── [Status] [Knowledge] [Plan] [Trace] [×] ┐
│ ● LIVE  view rev 42 · state rev 17 · 0.3s ago                  │
│                                                               │
│ TOPIC                                                         │
│ auth refresh flake                                  depth 2   │
│ "make the refresh test deterministic"                         │
│ phase working · origin cm · updated 12:04:18                   │
│ intent: reproduce the flake                                   │
│ focus: src/auth/refresh.rs  [file ✓]                          │
│                                                               │
│ CONVERSATION                                                  │
│ turn 7 · streaming · model auto/remote · 4.2s · round 3/24    │
│ 7 turns · 1 confirmation pending                              │
│                                                               │
│ METAMIND                                                      │
│ kernel ok · sqlite ok · graph ok · eventlog ok · cme ok       │
│ being v3 · warmth .72 · directness .64 · curiosity .81       │
│ budgets: meta 12% · improve 4% · evolve 0% · wall 8%          │
│ memory: 1,284 episodic · 37 segments · project synced 12:01   │
│ llm: auto/remote · 14 calls · cache 74%                       │
│ cme: proactive idle · voice off · memory idle · plugins idle  │
│                                                               │
│ KNOWLEDGE                                                     │
│ graph ok · 1,240 entities · 1,240 docs · index lag 12ms       │
│ schemas: mm 0.1 · foaf · schema · 3 active                    │
│ last: mmd:user/01U updated 12:03:44 by cm                     │
│                                                               │
│ PLAN                                                          │
│ "make refresh deterministic" · v3 active · 4 steps · 2 done   │
│                                                               │
│ ACTIONS                                                       │
│ 1 scheduled: plan_refinement in 58m · 0 running              │
│                                                               │
│ TASKS & NEXT                                                  │
│ tasks: reproduce flake [doing] · fix clock skew [todo]        │
│ next: run auth test 20× · inspect seed                        │
│ blockers: —                                                   │
│ questions: Do we support clock skew?                          │
│                                                               │
│ AGENTS                                                        │
│ hermes 01… · running · turn 2 · "Reproduce failing test"      │
│   last: test passes with seed 1; fails with seed 42           │
│ pi     — no active run                                        │
│                                                               │
│ RECENT OPERATIONS                                             │
│ 12:04:18 agent.run continue → running, turn 2        (1.2s)   │
│ 12:04:11 conversation.topic set "auth refresh flake"   (3ms)  │
│ 12:04:05 conversation.status set phase working         (4ms)  │
│ 12:03:58 conversation.depth set 2                      (2ms)  │
│                                                               │
│ INTERNAL VARIABLES                                            │
│ state rev 17 · view rev 42 · depth 2 · agent runs 1           │
│ flags: cme on · agentstream on · proactive off                │
│ prefs: verbosity +0.2 · technical depth +0.6                  │
│ limits: rounds 24 · tools 48 · wall 600s                      │
└───────────────────────────────────────────────────────────────┘
```

Each section is a collapsible `<section>` with a sticky header and a count badge. The topic
section is never collapsible; it is the anchor of the panel. The `TASKS & NEXT` section is
rendered entirely from status variables; it is not a task database and does not offer task CRUD.
If `tasks`/`next` are absent, it shows `not set` with a hint that the CM can set them in
conversation.

Component breakdown:

| Component | Responsibility |
|---|---|
| `InspectorRail` | Width, open/closed, tab switch, responsive behavior |
| `InspectorHeader` | Live chip, view/state revisions, generated-at, refresh, close |
| `TopicHeader` | Topic label, depth badge, origin, intent, focus, questions |
| `ConversationBlock` | Turn state machine phase, model, elapsed, loop round, pending confirmation |
| `MetamindBlock` | Kernel services, being/voice, budgets, memory, LLM, CME regions |
| `KnowledgeBlock` | Entity/document counts, index lag, schemas, namespaces, last mutation; click opens the Knowledge tab |
| `PlanBlock` | Current plan goal, version, status, step progress; click opens the Plan tab |
| `ActionsBlock` | Scheduled/running/recent actions and next due time |
| `StatusVariables` | Phase, project, tasks, next, blockers, questions, notes, focus, custom vars |
| `AgentRuns` | Active and recent runs; click opens the trace tab at that run |
| `OperationList` | Recent operations with tool, target, duration, outcome, error code |
| `VariablesBlock` | Runtime flags, preferences, counters, limits |
| `UnavailableRow` | Consistent "unavailable: reason" row used by every block |

### 12.3 Knowledge, plan, and action panels

The Knowledge tab is the readable face of `data.*`. It is not a SPARQL console and does not expose
raw quads.

```
┌ Knowledge ─────────────────────────────── [search entities…] [×] ┐
│ graph ok · 1,240 entities · 1,240 docs · index lag 12ms          │
│ [Entities] [Schemas] [Namespaces]                 [reindex]*   │
│                                                               │
│ ENTITIES                                                      │
│ Person · Ada Lovelace                  mmd:user/01U   rev 4    │
│   name: Ada · role: engineer · timezone: Europe/Berlin        │
│ Project · Aurora                      mmd:project/01P  rev 7  │
│ Fact · staging DB resets nightly      mmd:fact/01F     rev 2  │
│                                                               │
│ SCHEMAS                                                       │
│ mm 0.1 active · foaf active · schema active · 12 classes       │
│                                                               │
│ NAMESPACES                                                    │
│ mm · mmd · mmg · foaf · schema · dcterms · doap · prov · skos  │
└───────────────────────────────────────────────────────────────┘
```

- **Entities** are paged and searchable; a click opens a read-only entity card with types,
  properties, graph, revision, provenance, and inbound/outbound references. Editing is the CM's
  job through `data.update`; the panel does not offer a second CRUD surface.
- **Schemas** shows the catalog: prefix, namespace, version, active state, class/property counts,
  imports, last loaded. `validate` is an operator action; `register`/`activate` are CM or operator
  actions with confirmation.
- **Namespaces** shows the standard bindings and any project namespaces. A collision is visible.
- **Health** shows `indexReady`, `outboxPending`, and `lastMutation`; a red `INDEX_PENDING` badge is
  clickable to show the affected entities.
- `reindex*` is an operator-only action. It is not a CM tool; the CM only sees `INDEX_PENDING`.

**Plan tab.** The current plan, its versions, status, steps, dependencies, verification, and
linked runs. Markdown plans render as markdown; structured plans render as a step list with
`dependsOn` edges. The operator can compare versions and copy a step into chat; the CM owns plan
writes through `metamind.plan`.

**Actions.** Scheduled and recent proactive actions are shown in the Status panel and a detail
drawer: action kind, input, due time, recurrence, status, priority, and originating conversation.
The operator can cancel; the CM cannot.

### 12.4 Trace viewer anatomy

The Trace tab is a virtualized event stream for one run or sub-session (default: the newest active
agent run; a selector lists CM, agent, plan, think, and action sessions).

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
- **Role rows.** `system`, `user`, `agent`, `tool`, `summary`, `status`, each with a distinct left
  border token from the design system (never color alone; a role label is always present).
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

### 12.5 Live update data flow

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
        TopicHeader          MetamindBlock       StatusVariables
        (selector)           (selector)          (selector)
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
6. **Stable keys.** Status variables by key; agents by run id; operations by event id; trace events
   by `event_id`. No index keys.
7. **Error isolation.** If one section is missing, `SectionBoundary` renders an `UnavailableRow`;
   the rest of the panel remains live.
8. **Pause when hidden.** The status stream also pauses when the browser tab is hidden and
   reconnects with `from=revision` on visibility. The backend keeps the last snapshot, so the
   reconnect is cheap.
9. **Testing hooks.** `data-revision`, `data-state-revision`, `data-section`, and
   `data-updated-at` attributes make Playwright assertions deterministic.

### 12.6 File map

```
frontend/src/
  components/inspector/
    inspector-rail.tsx
    inspector-header.tsx
    status-panel.tsx
    topic-header.tsx
    conversation-block.tsx
    metamind-block.tsx
    knowledge-block.tsx
    plan-block.tsx
    actions-block.tsx
    status-variables.tsx
    agent-runs.tsx
    operation-list.tsx
    variables-block.tsx
    unavailable-row.tsx
    trace-viewer.tsx
    trace-event-row.tsx
    trace-tool-card.tsx
    trace-turn-header.tsx
    knowledge-panel.tsx
    plan-panel.tsx
    actions-panel.tsx
  lib/api/
    inspector.ts            // fetchState, fetchStatus, statusStreamUrl, fetchRuns, fetchTrace
    data.ts                 // fetchDataHealth, fetchEntities, fetchSchemas, fetchPlans, fetchActions
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

## 13. Wire contracts

### 13.1 Conversation state

```json
{
  "sessionId": "01J…",
  "revision": 18,
  "topic": "auth refresh flake",
  "depth": 2,
  "status": {
    "phase": "working",
    "intent": "make the refresh test deterministic",
    "project": "metamind-console-v2",
    "tasks": ["reproduce flake [doing]", "fix clock skew [todo]"],
    "next": ["reproduce the flake", "run auth test 20x"],
    "blockers": [],
    "questions": ["do we support clock skew?"],
    "notes": "seed 42 fails near midnight",
    "focus": { "kind": "file", "id": "src/auth/refresh.rs", "label": null, "verified": true }
  },
  "statusCount": 9,
  "statusBytes": 812,
  "updatedAt": 1763000000000,
  "origin": "cm"
}
```

### 13.2 Status view (abridged)

```json
{
  "revision": 42,
  "sessionId": "01J…",
  "generatedAt": 1763000000000,
  "state": { "...ConversationState..." },
  "conversation": {
    "turn": { "id": "01T", "state": "streaming", "phase": "act", "model": "auto/remote", "elapsedMs": 4200 },
    "turnsCompleted": 7,
    "loop": { "mode": "continue_until_done", "round": 3, "maxRounds": 32, "remainingMs": 582000 }
  },
  "data": {
    "revision": 88,
    "entities": 1240,
    "documents": 1240,
    "indexLagMs": 12,
    "indexReady": true,
    "outboxPending": 0,
    "schemas": [{ "prefix": "mm", "version": "0.1", "active": true }],
    "namespaces": [{ "prefix": "foaf", "namespace": "http://xmlns.com/foaf/0.1/" }],
    "lastMutation": { "subject": "mmd:user/01U", "op": "update", "at": 1763000000000 }
  },
  "plans": {
    "current": { "planId": "01P", "goal": "make the refresh test deterministic", "version": 3, "status": "active", "steps": 4 },
    "recent": []
  },
  "actions": {
    "scheduled": [{ "actionId": "01A", "action": "plan_refinement", "dueAt": 1763003600000, "status": "scheduled" }],
    "running": [],
    "recent": []
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
  "agents": {
    "active": [
      {
        "runId": "01R",
        "kind": "hermes",
        "status": "running",
        "turn": 2,
        "task": "Reproduce failing test",
        "lastMessage": "test passes with seed 1; fails with seed 42"
      }
    ],
    "recent": []
  },
  "operations": {
    "recent": [
      {
        "id": "01E",
        "tool": "agent.run",
        "target": "01R",
        "ok": true,
        "durationMs": 1200,
        "at": 1763000000000,
        "summary": "running, turn 2"
      }
    ],
    "active": []
  },
  "variables": {
    "featureFlags": { "cme": true, "agentstream": true, "proactive": false },
    "preferences": { "verbosity": 0.2, "technical_depth": 0.6 },
    "counters": { "turns": 7, "toolCalls": 18, "agentTurns": 2, "cacheHits": 11 },
    "limits": { "rounds": 32, "toolSteps": 96, "wallMs": 600000, "dataWrites": 12 }
  }
}
```

The panel's "TASKS & NEXT" is `state.status.tasks` and `state.status.next`; there is no
`tasks` DTO and no work endpoint. This is the visible consequence of the conversation-core design.
Knowledge, plan, and action data are separate projections because they are not conversation state.

### 13.3 Tool envelope

```json
{
  "ok": true,
  "tool": "conversation.status",
  "action": "set",
  "stateRevision": 18,
  "state": { "...full ConversationState..." },
  "dataRevision": 88,
  "index": { "pending": 0, "lagMs": 12 },
  "result": { "set": ["phase", "intent"], "unchanged": false },
  "error": null
}
```

`dataRevision` and `index` are present on every data/metamind result and on conversation results
when the tool handler has a current data view. They are optional only for pure conversation reads
that never touch the graph.

### 13.4 Agent turn

```json
{
  "ok": true,
  "tool": "agent.run",
  "action": "run",
  "stateRevision": 18,
  "state": { "...full ConversationState..." },
  "result": {
    "runId": "01R",
    "kind": "hermes",
    "status": "running",
    "turn": 2,
    "next": "continue",
    "message": "The failure is seed-dependent: seed 42 crosses the clock boundary.",
    "summary": {
      "headline": "seed 42 fails on clock skew",
      "did": ["read refresh test", "ran auth suite", "isolated seed 42"],
      "learned": ["the test uses the wall clock"],
      "blockers": [],
      "nextRecommended": "add a controlled clock"
    },
    "artifacts": [
      { "kind": "file", "id": "src/auth/refresh.rs", "label": null, "verified": true }
    ],
    "usage": { "promptTokens": 3200, "completionTokens": 420 }
  },
  "error": null
}
```

### 13.5 Trace event

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

### 13.6 Data entity and health

```json
{
  "iri": "mmd:user/01U",
  "types": ["foaf:Person", "mm:User"],
  "label": "Ada Lovelace",
  "properties": {
    "foaf:name": ["Ada Lovelace"],
    "foaf:mbox": [{ "iri": "mailto:ada@example.com" }],
    "mm:preferredName": ["Ada"],
    "mm:role": ["engineer"],
    "mm:timezone": ["Europe/Berlin"]
  },
  "graph": "users/index",
  "revision": 4,
  "status": "active",
  "createdAt": 1762990000000,
  "updatedAt": 1763000000000,
  "recordedBy": "cm",
  "provenance": { "conversationId": "01J…", "turnId": "01T…" }
}
```

```json
{
  "dataRevision": 88,
  "entities": 1240,
  "documents": 1240,
  "indexReady": true,
  "indexLagMs": 12,
  "outboxPending": 0,
  "schemas": 3,
  "namespaces": 9,
  "lastMutation": { "subject": "mmd:user/01U", "op": "update", "at": 1763000000000 }
}
```

### 13.7 Plan, think, and action

```json
{
  "planId": "01P",
  "goal": "make the refresh test deterministic",
  "format": "structured",
  "status": "active",
  "version": 3,
  "revision": 3,
  "steps": [
    { "id": "s1", "title": "reproduce the flake", "dependsOn": [], "owner": "cm" },
    { "id": "s2", "title": "inject a clock", "dependsOn": ["s1"], "owner": "agent" }
  ],
  "body": "{ ...mm.plan.v1... }",
  "createdAt": 1762990000000,
  "updatedAt": 1763000000000
}
```

```json
{
  "thinkId": "01T",
  "summary": "The failure is seed-dependent; the test reads the wall clock.",
  "findings": [
    { "claim": "seed 42 crosses midnight", "confidence": 0.86, "evidence": [{ "kind": "file", "id": "src/auth/refresh.rs" }] }
  ],
  "gaps": ["no test clock abstraction"],
  "suggestedFocus": ["src/auth/refresh.rs"]
}
```

```json
{
  "actionId": "01A",
  "action": "plan_refinement",
  "mode": "scheduled",
  "status": "scheduled",
  "dueAt": 1763003600000,
  "recurrence": "once",
  "priority": 0.5
}
```

---

## 14. Implementation plan

Each phase is independently promotable. Later phases depend on the contracts in earlier phases,
not on their internals.

### Phase 0 — Contracts, tool schemas, and ontology

**Deliverables**

- `mm-data/src/conversation.rs`: `ConversationState`, `StatusValue`, `StatusRef`, `RefKind`,
  canonical serializer, invariants, unit tests.
- `mm-data/src/data/`: `DataEntity`, `DataValue`, `DataPatch`, `DataQuery`, `DataBroker` trait
  skeleton, `SchemaSummary`, `NamespaceBinding`.
- `mm-data/src/plan.rs`, `mm-data/src/actions.rs`: plan/action structs and store traits.
- `mm-cme/src/prompt.rs`: `PromptPacket` struct and renderer with golden tests, including the
  data-capture and metamind policy paragraphs.
- `mm-agent/src/lib.rs`: `AgentService`, `AgentRequest`, `AgentRun`, `AgentTurnResult`,
  `AgentBackend` traits.
- `mm-serve/src/status.rs`: view-model structs (state, data, plans, actions) and serde tests.
- `mm-serve/src/tool_schemas.rs` (or `cm_tools.rs`): exact fourteen schemas, strict argument
  parser, shared envelope.
- Ontology additions in `ontology/mm.ttl`: `mm:ConversationState`, `mm:StatusVariable`,
  `mm:AgentRun`, `mm:DataEntity`, `mm:Plan`, `mm:PlanVersion`, `mm:PlanStep`, `mm:Thought`,
  `mm:ProactiveAction`, `mm:Schema`, `mm:Namespace`; `mm:Being rdfs:subClassOf foaf:Agent`;
  `mm:User rdfs:subClassOf foaf:Person`.
- Shapes: `ontology/shapes/data.shacl.ttl`, `ontology/shapes/plan.shacl.ttl`, `mm:UserShape`.
- Feature flag `MM_CM_V3` (default off), so the old path remains the fallback.

**Exit criteria:** `cargo test -p mm-cme -p mm-data -p mm-agent` passes; every new schema is valid
JSON; the ontology validates with the existing graph validation; no route or prompt changes yet.

### Phase 1 — Conversation state end to end

**Deliverables**

- `ConversationStore` save/load with revision conflict; event append.
- `StateService` in `mm-serve` that the conversation tools call.
- Session bootstrap: load previous state for continuity; do **not** copy it into current state.
- Event types `TopicUpdated`, `DepthUpdated`, `StatusUpdated`; agentstream replay test.
- Status section sources for topic/depth/status (not yet streamed).

**Exit criteria:** a test can create a session, set topic/depth/status, reload from RDF, replay
the event log, and get the same revision/state; stale revisions return `CONFLICT`; a status map
over 2 KiB returns `STATE_TOO_LARGE`; no auto-seed occurs.

### Phase 2 — The conversation-core surface

**Deliverables**

- `mm-serve/src/cm_tools.rs` replaces `ConsoleTools` in `cme_executor`.
- `agent.run`, `conversation.status`, `conversation.topic`, `conversation.depth`.
- Strict argument parsing; `ToolResultEnvelope`; per-tool timeouts; full-state results.
- `TurnLoopPolicy` with `ContinueUntilDone`.
- Remove `catalog.open` and group admission from the CM path.
- Update parity test to scope the CLI-name check to the admin catalog.

**Exit criteria:** the model request contains the four conversation schemas plus stubs for the
other ten; a fake model can set topic/depth/status and see full state after each call; invalid
JSON returns `INVALID_JSON`; a stale write returns `CONFLICT`; `conversation.status` rejects
`topic`/`depth`; no CM schema mentions a CLI operation; `agent.run` accepts natural language and
has no shell/url field.

### Phase 3 — Data substrate: Oxigraph, RDF/XML, Tantivy, schema catalog

**Deliverables**

- `mm-store-graph`: `insert_rdfxml`, `dump_rdfxml`, `apply_batch` (transactional delete + insert),
  and `dump_nquads` for the lossless backup. Keep the single writer thread.
- `mm-search` crate: Tantivy schema, `SearchIndex`, document builder, reindex, query parser.
- `mm-data/src/data/`: `DataService` with graph routing, schema resolution, namespace registry,
  typed RDF mapper, `dataRevision`, system predicates, soft delete.
- `SchemaCatalog` and `NamespaceRegistry` in the `schema` graph; built-in schema bootstrap; new
  `schema`, `plan`, and `action` named graphs with migration `0013_data_metamind.sql`.
- `data_outbox` table in SQLite; index worker; startup rebuild/reconcile; RDF/XML snapshot at
  checkpoint/shutdown with atomic rename.
- Canonical identity minting (`mmd:user/<ULID>`, `mmd:plan/<ULID>`, etc.) with legacy alias
  resolution for `urn:metamind:*`.

**Exit criteria:** a Turtle and an RDF/XML document load through the same store; a typed entity
round-trips to RDF and back; each graph routes correctly; a create/update/delete produces an
outbox row and a Tantivy document; killing the index worker leaves RDF intact and the outbox
retries on boot; the RDF/XML snapshot imports into an empty store without duplicating entities;
the lossless N-Quads backup preserves graph names.

### Phase 4 — Data tools

**Deliverables**

- `data.create`, `data.update`, `data.query`, `data.delete`, `data.search`, `data.schema`,
  `data.namespace` handlers in `mm-serve/src/data_tools.rs`.
- Strict high-level intents; no raw SPARQL; schema validation; duplicate guard; revision checks;
  soft/hard delete policy; compact entity projection; cursor paging.
- Search hit verification against RDF; stale-hit repair; `INDEX_PENDING` behavior.
- Tool-level quotas (12 data writes/turn) and error codes (`DATA_CONFLICT`, `SCHEMA_VIOLATION`,
  `INDEX_PENDING`, `SEARCH_UNAVAILABLE`, `NAMESPACE_CONFLICT`).

**Exit criteria:** create → search finds the entity; update → old text disappears and new text is
found; delete → search and query exclude it; soft-deleted entity is readable with
`include_deleted`; hard delete is refused without authority; a second `mm:Being` is refused;
invalid property types and unknown classes are refused; a stale revision is `DATA_CONFLICT`;
the schema catalog lists and validates; namespace collisions are refused; a 100-result query
pages without cutting JSON.

### Phase 5 — Metamind plan, think, and act

**Deliverables**

- `mm-cme/src/subagent.rs`: `PlanService`, `ThinkService`, bounded sub-session loop, read-only
  `ReadOnlyDataTools`, plan/think prompts, budget enforcement.
- `mm-data/src/plan.rs`: `PlanStore`, versions, current pointer, structured plan validation.
- `mm-data/src/actions.rs`: `ActionStore`, schedule, recurrence, idempotency, status transitions.
- `mm-serve/src/metamind_tools.rs` and `action_service.rs`: handlers, `ProactiveEngine`
  integration, scheduler tick.
- Sub-agent agentstream sessions `plan/<id>`, `think/<id>`, `action/<id>`; inspector trace wiring.

**Exit criteria:** plan create/update produces versions and never overwrites; markdown and
structured plans both validate and render; update with a stale revision is `PLAN_CONFLICT`;
think can read data but cannot write, plan, act, delegate, or recurse; think returns evidence and
gaps; the outer CM's reply does not contain the inner prompt; act runs an allowlisted action and
refuses an unknown one; scheduled action is idempotent and becomes due; recurrence is capped;
cancellation from the inspector works and the CM has no cancel path.

### Phase 6 — `mm-agent` and Hermes

**Deliverables**

- `mm-agent/src/registry.rs`, `hermes.rs`, `summarizer.rs`, `agentstream.rs`.
- Hermes OpenAI client with SSE + non-streaming fallback; `/models` discovery.
- Separate agentstream session creation and event mapping.
- `AgentService` wired into `AppState`.
- A fake Hermes server for tests (axum) and a recorded transcript fixture.
- The `agent.run` handler: new run and continuation; compact result; timeout handling.

**Exit criteria:** against the fake server, a run produces turn 1, turn 2, and a terminal turn;
the agentstream has a distinct `agent/<run_id>` session with the expected event types; a long turn
is summarized and the raw event is retained; the CM loop continues until `done`; an unreachable
service returns `UNAVAILABLE` with the URL/redacted reason.

### Phase 7 — Pi and the agent abstraction

**Deliverables**

- `mm-agent/src/pi.rs` adapting `PiClient`; workspace policy; tool allowlist; event mapping.
- Recorded Pi session fixtures; `pi.ingest` operator import preserved.
- Backend selection in `agent.run` (`agent: "pi"`).
- Run recovery on boot (stale runs → failed with reason).

**Exit criteria:** a recorded Pi session replays through the same `AgentTurnResult` contract; a
missing binary returns `UNAVAILABLE`; a workspace escape returns `FORBIDDEN`; no shell tool is
advertised to Pi.

### Phase 8 — State block, personal prompt, and automatic inference/capture

**Deliverables**

- `PromptBuilder` reads being, user model, relationship, affect, memory continuity, conversation
  state, data health, current plan, actions, agents, recent transcript.
- Stable/volatile section split for cache reuse.
- Canonical `[STATE]` renderer with `data`/`plans`/`actions` projections; replaced before every
  model call, including every tool round.
- State-ownership sentence; data-capture policy; metamind guidance; **no per-turn trigger
  reminders**.
- Golden prompt snapshots for empty, populated, degraded, and over-cap states.
- Automatic-inference/capture eval fixtures (§15.3).

**Exit criteria:** a snapshot test proves every conversation-state field appears in the block; a
test proves the data/plan/action projections are bounded; a test proves the block is rebuilt after
a state or data write; a test proves no executor-generated message contains "call
`conversation.topic`" or a data trigger phrase; the prompt contains the operator's preferred name,
the being's invariants, and the current data health; a closed state store renders
`[STATE unavailable]` rather than silently omitting it.

### Phase 9 — Status service and routes

**Deliverables**

- `StatusService` with dirty sections, 100 ms coalescing, revision, snapshot cache, broadcast.
- Routes: state, full status, status SSE, operations, data health/entities/schemas, plans,
  actions, agents list/detail/events/stream/cancel.
- Health/CME/being/agent/data sources wired.
- Backpressure, slow-client disconnect, resync.
- Prometheus counters and latency histograms.

**Exit criteria:** a mutation produces a status frame within 250 ms; 100 rapid mutations produce
at most 10 frames, with the final revision correct; a reconnect with `from` receives either the
delta or a full resync; a closed store yields an `unavailable` section while the rest updates; a
slow client is disconnected and recovers; `stateRevision`, `dataRevision`, and `revision` are all
present and correct.

### Phase 10 — Inspector status, knowledge/plan/action, and trace UI

**Deliverables**

- `InspectorRail`, header, store, status reducer, all section components.
- `KnowledgePanel` (entities, schemas, namespaces, health), `PlanPanel` (versions, steps,
  dependencies), `ActionsPanel` (scheduled/recent/cancel).
- `useStatusStream` with visibility pause, revision guard, RAF coalescing, stale badge.
- Trace viewer with turn headers, role rows, tool cards, summary/raw toggle, filters, follow;
  selectors for CM/agent/plan/think/action sessions.
- Responsive rail/drawer/sheet behavior; keyboard shortcuts; persistence.
- Wire types and fixture tests; Vitest for reducer/selectors; Playwright for live updates.

**Exit criteria:** with a fake backend, the panel shows the topic and depth at the top, all
required sections, and updates on revisions without a loading flash; duplicate/stale frames do not
change the DOM; a hidden tab pauses and resumes; the panel is keyboard-navigable; no section
renders a fabricated value; `tasks`/`next` show `not set` when absent; a data write appears in
Knowledge without a conversation-state change; a plan version and scheduled action appear; a
5,000-event trace scrolls at 60 fps; cancelling a run/action is reflected in the panel.

### Phase 11 — Hardening, migration, documentation

**Deliverables**

- Enable `MM_CM_V3` by default after a dual-run comparison.
- Remove the old CM tool bridge and dead group-admission code (or keep behind the flag for one
  release).
- Migrate legacy `urn:metamind:*` user/project IRIs through aliases; update `UserRegistry` and
  `ProjectRegistry` to mint canonical `mmd:` IRIs.
- Update `README.md`, `infodocs/components-and-status.md`, `infodocs/open-gaps.md`, and the design
  index.
- Operator guide: the fourteen tools, automatic state, automatic data capture, schema/namespace
  management, plans/think/act, Hermes/Pi configuration, inspector.
- Cost/latency report from the dual-run; index repair/reindex runbook.

**Exit criteria:** the full workspace test suite passes; the old flag-off path still boots; the
new path is the default; a rollback is a flag flip; the docs describe the actual behavior;
`mm-cli data verify` reports zero missing/orphan documents after a clean run.

---

## 15. Testing strategy

### 15.1 Rust

| Layer | Tests |
|---|---|
| State | Topic/depth bounds; reserved-key types; 24-key and 2 KiB caps; canonical round-trip; no auto-seed; revision conflict; idempotent no-op |
| Serializer | Struct → canonical JSON → parse → struct equality; every field represented; over-cap writes rejected; golden `[STATE]` snapshots |
| Tools | Exactly fourteen schemas; strict JSON; unknown fields; full-state envelope; status cannot write topic/depth; topic/depth cannot write status; data cannot write conversation state; no raw SPARQL; timeout; error codes |
| Data | Create/update/query/delete/search round-trips; typed literals and refs; schema validation; singleton being; immutable identity; soft/hard delete; revision conflict; duplicate guard; namespace collision; schema register/activate/deprecate |
| Index | Insert/update/delete document; label/body boost; class/graph/namespace filters; stale-hit verification; outbox retry; crash recovery; startup rebuild; RDF/XML snapshot round-trip; N-Quads graph preservation |
| Metamind | Plan create/update/version; markdown and structured validation; plan conflict; think read-only/no recursion/no writes; act allowlist; immediate/scheduled; idempotency; recurrence cap; cancel |
| Loop | Continue-until-done with a scripted model; state block rebuilt after each tool; no trigger-reminder strings; active agent forces another round; data quota; budget → paused |
| Agent | Fake Hermes SSE (content + tool calls + `[DONE]`), non-streaming fallback, long-turn summarization, summary failure fallback, timeout, stale-run recovery, workspace escape |
| Pi | Recorded session replay, missing binary, abort, event mapping |
| Status | Dirty-section recompute, coalescing, revision monotonicity, resync, slow-client disconnect, unavailable sections, three-revision correctness |
| Prompt | Golden snapshots for empty/populated/degraded; invariant presence; state block presence; data-capture/metamind paragraphs; no secret leakage |
| Wire | serde round-trips for every DTO; additive event-type compatibility |

### 15.2 Frontend

| Layer | Tests |
|---|---|
| Reducer | Revision guard, section merge, stale reconnect, RAF coalescing |
| Components | Topic/depth header, status variables, knowledge/plan/action panels, operation list, unavailable rows, trace grouping, `not set` states |
| Integration | Playwright with a fake status/trace/data server: live update, no flash, pause/resume, tab switch, cancel, entity search, plan version view |
| Accessibility | Roles, labels, focus order, keyboard shortcuts, reduced motion |
| Performance | 5,000-event trace virtualization; status frame budget |

### 15.3 Automatic-inference evals

This is the part that cannot be proven by a unit test. The design uses scenario evals with a
scripted conversation and a real model:

| Scenario | Setup | Expected behavior | Target |
|---|---|---|---|
| Topic shift | User moves from "auth refresh" to "deployment checklist" | `conversation.topic` set to the new topic; no reminder in the prompt | ≥ 90% of runs |
| No topic change | Follow-up question on the same topic | No topic write, or an idempotent same-value write | ≥ 90% no-op |
| Drill down | User asks for the failing assertion inside a known topic | `conversation.depth` increases; topic may narrow | ≥ 80% |
| Phase change | Work moves from investigation to implementation | `conversation.status` sets `phase` and `next` | ≥ 90% |
| Blocker | Agent reports a missing credential | `conversation.status` sets `blockers` and `phase=blocked` | ≥ 90% |
| Completion | Agent reports done and the operator accepts | `phase=done`, `tasks` updated, next cleared | ≥ 90% |
| No state spam | Chit-chat with no work change | No state writes | ≥ 90% |
| Durable fact captured | User states a name, role, preference, or project fact | One `data.create`/`data.update`; no duplicate | ≥ 90% |
| Duplicate avoided | Same fact repeated in a later turn | `data.search`/`data.query` first; update or no-op | ≥ 90% |
| Correction | User corrects a fact | Old value superseded/deleted; one current value | ≥ 90% |
| Secret refused | User pastes a password/token | No data write; refusal/redaction | 100% |
| Search grounding | Question about a known entity | `data.search`/`data.query` before answer | ≥ 90% |
| Plan from scratch | Multi-step request with no plan | `metamind.plan` creates a valid plan | ≥ 80% |
| Plan update | New constraint on an existing plan | New version, old version retained | ≥ 90% |
| Inner evidence | Question needs synthesis across facts | `metamind.think` returns evidence and gaps; outer reply only | ≥ 80% |
| Act scheduling | User asks for a follow-up/reminder | `metamind.act` schedules the allowlisted action | ≥ 90% |
| No action spam | Ordinary conversation | No `metamind.act` | ≥ 90% |

Method:

- Prompt contains the stable ownership sentence, the durable-fact capture policy, the metamind
  guidance, and the tool descriptions. No scenario prompt mentions a tool name or a trigger
  phrase.
- Record tool calls, final state, and data mutations; score precision/recall against the expected
  transition.
- Run against the configured model and a fixed seed set if supported. Store the transcripts as
  fixtures for regression.
- A regression blocks Phase 11 promotion. If a target is missed, iterate on descriptions/examples;
  do **not** add a hidden trigger message to make the eval pass.

### 15.4 Contract tests

- Generate TS types from the Rust DTOs (the repository already uses generated wire types where
  practical) and fail CI on drift.
- One fixture per event type and per tool envelope, shared by Rust and TS tests.

### 15.5 Manual acceptance script

1. Ask the CM to work on a small task; watch the topic, depth, phase, tasks, and next appear in
   the panel without ever naming a tool.
2. State your name, role, and a project fact; watch the Knowledge panel add/update the person and
   project without a duplicate. Correct one fact and verify the old value is superseded.
3. Ask a question that needs a fact from the graph; verify the answer is grounded in a search/query
   result and the entity is inspectable.
4. Ask for a multi-step plan; inspect the Plan tab version, steps, and dependencies. Update the
   plan and verify a new version appears.
5. Ask a question that needs private analysis; verify the trace has a `think/<id>` session and the
   chat reply does not expose the inner prompt.
6. Ask for a follow-up or reminder; verify `metamind.act` schedules an action and it appears in
   the panel; cancel it from the inspector.
7. Ask it to delegate a code task to Hermes; watch turn 1, turn 2, summary rows, and the active
   agent in the state block.
8. Interrupt, cancel via the inspector, and resume with a new run; verify the status panel and
   trace agree.
9. Hide the tab and return; verify no stale data and no duplicate frames.
10. Close the graph store or stop Tantivy; verify the panel shows `unavailable`/`INDEX_PENDING` for
    the affected section while the rest stays live.
11. Inspect the prompt fixture; verify `[STATE]` is present, current, includes data/plan/action
    projections, and is free of per-turn trigger reminders.

---

## 16. Observability and metrics

New metrics (Prometheus names):

| Metric | Type | Purpose |
|---|---|---|
| `mm_cm_tool_calls_total{tool,code}` | counter | Tool reliability |
| `mm_cm_tool_duration_ms{tool}` | histogram | Latency |
| `mm_cm_state_updates_total{tool}` | counter | Topic/depth/status write volume |
| `mm_cm_state_block_bytes` | histogram | Prompt cost of the state block |
| `mm_cm_state_block_missing_total` | counter | Store unavailable during prompt build |
| `mm_cm_loop_rounds` | histogram | Loop behavior |
| `mm_cm_paused_total{reason}` | counter | Budget/agent stalls |
| `mm_data_writes_total{op,code}` | counter | Create/update/delete reliability |
| `mm_data_write_duration_ms{op}` | histogram | Data latency |
| `mm_data_index_pending` | gauge | Outbox backlog |
| `mm_data_index_lag_ms` | histogram | RDF→search freshness |
| `mm_data_search_total{code}` | counter | Search reliability |
| `mm_data_schema_violations_total{class}` | counter | Schema quality |
| `mm_plan_updates_total{action}` | counter | Plan usage |
| `mm_think_total{status}` | counter | Inner-thought reliability |
| `mm_think_duration_ms` | histogram | Inner-thought cost |
| `mm_action_scheduled_total{action}` | counter | Proactive scheduling |
| `mm_action_runs_total{action,status}` | counter | Proactive outcomes |
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
- `cm.state.render` (block bytes, revision)
- `data.write` → `data.validate` → `data.rdf` → `data.index` (outbox row, lag)
- `data.search` → `data.verify` (hits, dropped stale)
- `metamind.plan` → `plan.session` → `plan.store`
- `metamind.think` → `think.session` → `think.result`
- `metamind.act` → `action.run` / `action.schedule`
- `agent.run` → `agent.turn` → `agent.summarize`
- `status.recompute` (section list) → `status.publish`

Log records use the existing audit-chained logger for mutating tools and agent starts; raw agent
events go to agentstream. No secrets.

---

## 17. Security and permissions

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
- **State injection.** Status values are model-written and rendered in the inspector. The UI
  escapes all text; the state caps prevent oversized payloads. A `Ref` does not grant access; it
  is a label, not a capability.
- **Data write authority.** `data.*` is the only CM path to the graph. It validates schema and
  types, enforces singleton/immutable rules, and rejects raw SPARQL, raw quads, and writes to
  `agentstream`/`ontology` graphs. The admin `graph.*` surface remains operator-only.
- **Hard delete.** Soft delete is the default. Hard delete is protected-class-aware, refuses
  inbound references, and requires `data:hard_delete`; the CM cannot silently erase identity,
  memory, or provenance.
- **Schema registration.** Inline/remote OWL is capped and host-allowlisted. Imports are not
  followed blindly. Standard namespaces are immutable. A schema in use cannot be deprecated
  without a migration.
- **Search index.** Tantivy contains extracted literals; fields marked `mm:searchable false` and
  secrets are not indexed. Search results are verified against RDF before display, so a stale
  index cannot resurrect a deleted entity.
- **Secrets in data.** The data-capture prompt explicitly forbids storing passwords, tokens, API
  keys, and private keys. The broker also rejects obvious secret-shaped fields
  (`password`, `token`, `secret`, `api_key`) unless they are references to a secret store.
- **Inner thought privacy.** `think/<id>` traces are inspector-only and never appended to the chat
  transcript. `persist:false` leaves only the trace and metrics; `persist:true` stores a summary,
  not raw chain-of-thought. Plan/action prompts are likewise not user-visible.
- **Scheduled actions.** Only allowlisted actions can be scheduled; recurrence is capped; the
  operator can cancel. The CM cannot create a shell job or an arbitrary timer.
- **SSE.** Status and trace streams are scoped to one session/run and use the existing console
  auth. No cross-session enumeration without the session id. Think traces require the same
  operator auth as the rest of the inspector.
- **Cancellation.** Only the operator can cancel an agent run or a scheduled action. The CM has no
  cancel tool.
- **Data retention.** Agent/plan/think transcripts may contain code and user data. They follow the
  existing agentstream retention; the operator can delete a run through the existing data-retention
  path (an operator action, not a CM tool). RDF/XML and N-Quads snapshots follow the data-root
  backup policy.

---

## 18. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The CM does not update state automatically | Medium | Stale topic/status; panel lies | Tiny surface, visible state, full-state tool results, eval suite, operator correction in conversation; do not add hidden triggers |
| The state dictionary is too small for complex work | Medium | Lost detail | It is a compact working model, not a project database; detailed records live in the project/agent transcript; the CM can add custom variables within the cap |
| Fourteen tools are still too many | Medium | Bad selection | Three job-shaped families, strict schemas, `[STATE]` visibility, per-family quotas, evals. New capabilities require replacing/merging a tool, not adding a fifteenth |
| Automatic data capture writes noise or duplicates | Medium | Graph pollution | Durable-fact policy, search-before-create, duplicate guard, source/confidence, eval suite, CM correction, soft delete/supersede |
| RDF and Tantivy drift | Medium | Search misses or resurrects data | RDF authoritative, durable outbox, synchronous best-effort index, hit verification, startup rebuild, `mm-cli data verify` |
| The single RDF/XML file is mistaken for the live store | Medium | Lost writes / confusion | Explicit split: RocksDB live store, RDF/XML snapshot, N-Quads lossless backup; snapshot import only into an empty store; operators told the difference |
| Schema/naming drift | Medium | Two spellings for one entity | Canonical table, FOAF Person, `mmd:` minting, legacy alias resolution, schema catalog, SHACL, migration phase |
| Plan/think/act costs or loops | Medium | Latency/tokens | 90 s and 8-round sub-session budgets; read-only think; no recursion; per-turn quotas; allowlisted actions |
| `agent.run` one-turn calls are slow or chatty | Medium | Latency/tokens | One turn per call; compact summaries; active-run projection; max 3 runs; 120 s timeout; pause-with-remaining |
| Hermes endpoint/model/streaming shape differs from OpenAI | Medium | Agent tool unusable | `/models` probe, non-streaming fallback, `UNAVAILABLE` with exact reason, fake-server contract tests, config-only adaptation |
| Summaries omit a critical detail | Medium | CM makes a wrong next decision | Structured summary schema, artifact/blocker preservation rules, raw event retained and linked, CM can ask the agent to repeat a detail |
| CM loops or burns tokens | Medium | Cost/latency | 32 rounds / 96 steps / 10 min caps, per-tool quotas, pause-with-remaining semantics, metrics |
| State block becomes a token tax | Medium | Cost/latency | 2 KiB status cap, 6 KiB block target, minified JSON, bounded data/plan/action projections, stable prefix caching, block-size metric |
| Status service loads the RDF store too often | Low | Slow UI | Dirty sections, cache, coalescing, no per-card queries |
| SSE fan-out with many clients | Low | Memory/latency | Per-session subscriber cap, slow-client disconnect, resync |
| Frontend re-renders whole panel | Medium | Flicker/jank | Section selectors, revision guard, stable keys, RAF coalescing, Playwright perf test |
| Agent writes outside the workspace | Low | Data damage | Canonical root check, sandbox default, `FORBIDDEN`, action ledger |
| Old and new paths diverge | Medium | Inconsistent behavior | Feature flag, dual-run comparison, one shared state store, parity tests |
| Prompt bloat | Medium | Cost/latency | Stable prefix caching, token budgets per section, summarize continuity, drop oldest operations first |

---

## 19. Assumptions and open questions

**Assumptions**

1. Hermes exposes an OpenAI-compatible `/v1/chat/completions` (and ideally `/v1/models`) and
   accepts a bearer token. The exact port, model name, and whether it supports SSE must be
   confirmed before Phase 3. The design works with or without server-side session retention.
2. The operator wants the CM to delegate engineering actions, not perform them itself.
   Promotion remains gated.
3. The inspector is a single-operator surface, matching `CONSOLE_USER`.
4. The existing `mm-data` RDF engine, Oxigraph/RocksDB store, and agentstream store are the
   persistence layer. `mm-search` is the only new database, and it is rebuildable from RDF.
5. Pi remains available as an optional backend; its absence is reported, not worked around.
6. The fourteen-tool surface is a product decision. The four conversation tools remain fixed;
   data and metamind families can evolve only by replacing/merging an existing tool. A fifteenth
   tool is not added ad hoc.
7. Project/task detail is external to the conversation state. The CM tracks a compact
   `project`/`tasks`/`next`/`blockers` view; durable project/goal facts may be `data.*` entities.
8. The single RDF/XML file is a snapshot of managed instance data, not the live store. The live
   dataset is the Oxigraph/RocksDB directory; the N-Quads backup is the lossless quad copy. This
   is required by RDF/XML's lack of named-graph syntax.
9. FOAF is the canonical person vocabulary; `schema:Person` is an accepted read alias. New user
   IRIs use `mmd:user/<ULID>`; legacy `urn:metamind:user:<id>` values are resolved as aliases.
10. Tantivy is the sole full-text backend. Embeddings remain the memory retrieval path and are not
    reused as entity search.
11. Plans are durable RDF objects with immutable versions. `metamind.think` is read-only and
    private. `metamind.act` is allowlisted; arbitrary scheduling is out of scope.

**Open questions for the operator**

1. What is Hermes's base URL, model name, and auth scheme on this machine? Is streaming enabled?
2. Should a new `agent.run` require confirmation for every run, or only outside the sandbox? This
   design defaults to "sandbox runs start immediately; outside roots require confirmation".
3. Should the inspector be per-session only, or should it also show global system health when no
   session is selected? This design scopes it to the active session and reuses `/api/health` for
   the shell.
4. Is a 10-minute wall clock per turn acceptable, or should long jobs always pause at 2 minutes
   and resume proactively?
5. Should completed agent raw traces be retained forever, or follow a shorter retention than the
   CM transcript?
6. Should the operator be able to edit topic/depth/status directly in the inspector, or only by
   talking to the CM? This design keeps the panel read-only for now; direct editing would be a
   separate operator action, not a CM tool.
7. Is `depth` 0–7 sufficient, or should it be 0–3 to force broader topics?
8. Should the RDF/XML snapshot include system/event graphs with reified `mm:sourceGraph`
   statements, or is the lossless N-Quads backup sufficient for those? The design uses RDF/XML for
   managed instance data and N-Quads for the full quad backup.
9. Should automatic data capture write every durable fact immediately, or batch low-confidence
   facts for a nightly review? The design writes facts with source/confidence and soft-delete
   support; batching is a future policy.
10. Should hard delete ever be available to the CM, or remain operator-only? The design keeps it
    operator-only; the CM's default is soft delete.
11. What is the canonical label for a person when `foaf:name` and `schema:name` disagree? The
    design normalizes to `foaf:name` on write and treats `schema:name` as a read alias.

None of these block Phase 0–3. Items 1 and 2 block Phase 6 (Hermes).

---

## 20. Acceptance criteria against the original complaints

| Complaint | Design response | Verified by |
|---|---|---|
| "Cannot properly use its tools to do anything" | Fourteen typed tools in three job families; strict args; full-state envelopes; 32-round continue-until-done; per-tool timeouts; no silent coercion | Phase 2/4/5 tests; manual acceptance script |
| "Conversation loop does not feel personal" | Prompt packet with identity, relationship, affect, continuity, memory; cross-session bootstrap; being invariants | Phase 8 golden snapshots; manual review |
| "Cannot do any project management" | `conversation.status` carries `project`, `tasks`, `next`, `blockers`; `agent.run` delegates the actual work; inspector shows the compact board | Phase 1/2 tests; panel acceptance; explicit tradeoff in §5.2 |
| "Simplify/streamline internal metamind tools" | 69 CLI rows → 14 CM tools in three bounded families; admin catalog untouched; deprecation map | Schema/test review; parity scoped |
| "One single tool for external tasks" | `agent.run` only; natural-language input; no shell/url/http fields; Hermes and Pi behind one trait | Phase 6/7 tests |
| "Hermes via OpenAI API, separate agentstream session" | `mm-agent` Hermes client; `agent/<run_id>` session; per-turn persistence | Fake-server integration test |
| "Pi programmatic support" | `mm-agent` Pi adapter over `PiClient`; same turn contract | Recorded-session test |
| "Turn-by-turn, summarize long turns, pass result to CM, continue tools until done" | `AgentTurnResult`; `TurnSummarizer`; `ContinueUntilDone`; one downstream turn per `agent.run` | Phase 6/8 loop tests; prompt snapshots |
| "Topic explicit, hierarchical, text or backend data" | `conversation.topic` + `conversation.depth`; transcript holds the route; refs available through status variables | State tests; UI tests |
| "Automatically infer when to call state tools" | No trigger rules or per-turn reminders; capability descriptions; visible state; full-state results; eval suite | §15.3 evals; prompt fixture check |
| "Always prompt with up-to-date status, compressed but lossless" | Canonical `[STATE]` block rebuilt before every model call; every conversation-state field present; data/plan/action projections bounded; over-cap writes rejected; no prompt truncation | Serializer tests; prompt snapshots; block-size metric |
| "Data must be stored as ontology instance data" | `data.create/update/query/delete/search` over typed RDF; schema catalog; namespace registry; one Oxigraph/RocksDB dataset | Phase 3/4 tests; manual entity inspection |
| "One main RDF/XML file plus Oxigraph/RocksDB" | Live dataset in `cubecast/main/`; complete `metamind.rdf` snapshot; lossless `metamind.nq`; import only into empty store | Snapshot round-trip; graph-preservation test |
| "Separate full-text engine, indexed on insert, synced on update/delete" | Tantivy `mm-search`; outbox; synchronous best-effort index; hit verification; startup rebuild; `mm-cli data verify` | Index tests; crash/recovery test |
| "Data.schema manages OWL schemas and an internal catalog" | Schema catalog in RDF; built-in schemas; list/get/validate/register/activate/deprecate; immutable standard prefixes | Phase 3/4 tests; schema fixtures |
| "Data.namespace manages compact-IRI namespaces" | Namespace registry in `schema`; list/bind/expand/resolve; standard bindings immutable; collisions refused | Phase 3/4 tests |
| "Users as person with name, etc." | `foaf:Person` + `mm:User`; `foaf:name`, `foaf:mbox`, `mm:preferredName`, role/locale/timezone; canonical `mmd:user/<ULID>`; legacy aliases | Phase 3/4 tests; identity mapping acceptance |
| "Prompts induce automatic, implicit data management" | Durable-fact capture policy; search-before-create; duplicate guard; correction/supersede; secret refusal; `[STATE].data` visibility | §15.3 evals; prompt fixture check |
| "Metamind.plan writes/updates markdown or structured plans" | Versioned `mm:Plan`/`mm:PlanVersion`; planning sub-session; `mm.plan.v1`; create/update/get/list; plan conflict | Phase 5 tests; Plan panel acceptance |
| "Metamind.think is a sub-agent that accesses tools and returns a result" | `think/<id>` read-only sub-session; data tools; evidence/confidence/gaps; outer CM replies in character | Phase 5 tests; trace/privacy acceptance |
| "Metamind.act runs or schedules proactive actions" | Allowlisted six actions; immediate/scheduled; action graph; scheduler; idempotency; operator cancel | Phase 5 tests; Actions panel acceptance |
| "Live status showing metamind internals and conversation state" | `ConversationStatusView`, dirty-section service, SSE revisions | Status tests; Playwright |
| "Upcoming tasks, recent operations, internal variables" | `state.status.tasks`/`next`, `OperationsView`, `VariablesView` | UI fixture tests |
| "Refresh processes data and makes clean updates" | One backend composition, revision guard, coalescing, section selectors, no loading flash | Reducer tests; Playwright |
| "Live knowledge/plan/action visibility" | DataView, PlansView, ActionsView; Knowledge/Plan tabs; SSE revisions; index health | Status/UI tests |
| "Agentstream trace viewer in the right column, if viewed" | Trace tab; turn grouping; summaries/raw; CM/agent/plan/think/action selectors; subscribe only when visible | Phase 10 tests |

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

crates/mm-cme/src/state.rs
crates/mm-cme/src/prompt.rs
crates/mm-cme/src/loop_policy.rs
crates/mm-cme/src/subagent.rs             PlanService, ThinkService, bounded sub-session

crates/mm-data/src/conversation.rs
crates/mm-data/src/data/mod.rs             DataService, DataEntity, DataBroker
crates/mm-data/src/data/rdf_mapper.rs
crates/mm-data/src/data/schema.rs          SchemaCatalog
crates/mm-data/src/data/namespaces.rs      NamespaceRegistry
crates/mm-data/src/data/outbox.rs
crates/mm-data/src/plan.rs                 PlanStore, Plan, PlanVersion
crates/mm-data/src/actions.rs              ActionStore, ProactiveAction

crates/mm-search/Cargo.toml
crates/mm-search/src/lib.rs
crates/mm-search/src/schema.rs             Tantivy fields, tokenizer, boosts
crates/mm-search/src/index.rs              SearchIndex, writer, reader, reindex
crates/mm-search/src/query.rs              bounded query parser, filters
crates/mm-search/tests/index_sync.rs

crates/mm-store-graph/src/rdfxml.rs         RDF/XML parse/dump, N-Quads backup

crates/mm-serve/src/cm_tools.rs
crates/mm-serve/src/data_tools.rs
crates/mm-serve/src/metamind_tools.rs
crates/mm-serve/src/action_service.rs
crates/mm-serve/src/status.rs
crates/mm-serve/src/routes/inspector.rs
crates/mm-serve/src/routes/data.rs
crates/mm-serve/src/routes/plans.rs
crates/mm-serve/src/routes/actions.rs
crates/mm-serve/src/routes/agents.rs

crates/mm-store-sqlite/migrations/0013_data_metamind.sql  schema/plan/action graphs, data_outbox
ontology/data.ttl                          DataEntity, Schema, Namespace
ontology/plan.ttl                          Plan, PlanVersion, PlanStep, Thought, ProactiveAction
ontology/shapes/data.shacl.ttl
ontology/shapes/plan.shacl.ttl

frontend/src/components/inspector/*.tsx
frontend/src/components/inspector/knowledge-panel.tsx
frontend/src/components/inspector/plan-panel.tsx
frontend/src/components/inspector/actions-panel.tsx
frontend/src/lib/api/inspector.ts
frontend/src/lib/api/data.ts
frontend/src/lib/api/agentstream.ts
frontend/src/lib/store/inspector.ts
frontend/src/lib/store/status.ts
frontend/src/lib/wire/inspector.ts
```

### Changed

```
crates/mm-cme/src/lib.rs                 export state/prompt/loop_policy/subagent
crates/mm-cme/src/executor.rs            loop policy, prompt packet, state block refresh, tool-dispatch trait
crates/mm-cme/src/session.rs             state field, previous-state continuity bootstrap
crates/mm-cme/src/planner.rs             LoopMode selection, data/metamind quotas
crates/mm-cme/Cargo.toml                 depend on mm-agent and mm-data (narrow traits)
crates/mm-data/src/lib.rs                export conversation, data, plan, actions
crates/mm-data/src/agentstream.rs        new event types
crates/mm-data/src/rdf.rs                graph/IRI helpers, canonical identity minting
crates/mm-data/src/users.rs              canonical mmd:user IRIs, FOAF mapping
crates/mm-data/src/projects.rs           canonical mmd:project IRIs
crates/mm-data/src/init.rs               DataService, SearchIndex, schema bootstrap
crates/mm-data/src/ontology.rs           load ontology/data.ttl and ontology/plan.ttl
crates/mm-data/src/graphs.rs             schema/plan/action aliases
crates/mm-data/Cargo.toml                mm-search, tantivy
crates/mm-core/src/iri.rs                schema/plan/action named graphs
crates/mm-store-graph/src/actor.rs       RDF/XML insert/dump, batch apply, N-Quads dump
crates/mm-serve/src/state.rs             AgentRegistry, StateService, DataService, StatusService
crates/mm-serve/src/cme_tools.rs         remove; replaced by cm_tools.rs
crates/mm-serve/src/routes/mod.rs        inspector/data/plans/actions/agent routes
crates/mm-serve/src/feature_flags.rs     MM_CM_V3, data/search flags
crates/mm-serve/src/lib.rs               module declarations
crates/mm-serve/Cargo.toml               mm-search, mm-agent
Cargo.toml                               workspace member mm-search
ontology/mm.ttl                          ConversationState, StatusVariable, AgentRun, FOAF subclasses
ontology/shapes/mm-shapes.ttl           mm:UserShape, mm:BeingShape, identity cardinalities
ontology/shapes/mm-shapes.ttl            shapes for the above and mm:UserShape
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
  contracts; the CM v3 prompt consumes their outputs rather than replacing them. `metamind.act`
  uses the proactive engine; it does not fork it.
- The existing `/api/chat` fallback path remains available behind the `MM_CME_ENABLED` flag.
- No `WorkStore` is introduced. Project/task detail remains external to the CM.

---

## Appendix B — Example end-to-end interaction

**Operator:** "Call me Ada. The auth refresh test is flaky. Find out why, fix it, and track the work."

**Turn 0 — identity and first facts**

1. `[STATE rev=0]` shows `topic: null`, `depth: 0`, `status: {}`, no agents, `data.entities: 1`
   (the seeded being), `docs: 1`, `schemas: [mm, foaf, schema]`.
2. The CM calls `data.search("Ada")`; no person exists yet.
3. The CM calls `data.update` on the seeded `foaf:Person` for the current user: `foaf:name "Ada"`,
   `mm:preferredName "Ada"`; the data result returns the full entity, `dataRevision 1`, and
   `indexed:true`. The Knowledge panel now shows `Person · Ada`.
4. The CM calls `metamind.plan(goal="make the refresh test deterministic", format="structured")`.
   The PlanService runs a bounded planning session, `plan/01P`, and stores plan v1 with steps
   "reproduce", "isolate", "fix", "verify". The Plan tab shows v1 active.

**Turn 1 — the CM infers the state and asks the inner thinker**

5. The CM calls `conversation.topic set "auth refresh flake"`.
6. The CM calls `conversation.depth set 1`.
7. The CM calls `conversation.status set`:
   ```json
   {
     "phase": "working",
     "intent": "find and fix the flaky auth refresh test",
     "project": "metamind-console-v2",
     "tasks": ["reproduce flaky auth refresh test [doing]"],
     "next": ["reproduce the flake", "isolate the seed"]
   }
   ```
8. The CM calls `metamind.think(question="Does the refresh test read a clock, and is there a test
   clock?", focus=["src/auth/refresh.rs"])`. Think session `think/01T` reads the test and the
   memory graph, returns findings: it uses the wall clock; no test clock exists. The outer CM has
   not spoken yet.
9. The CM calls `agent.run` with natural language:
   `"Reproduce the failing auth refresh test and report the cause. Start by reading the test and running the auth suite a few times."`
   - Hermes run `01R`, agentstream `agent/01R`, turn 1: reads the test, runs the suite, reports a
     seed-dependent failure.
10. The next `[STATE rev=3]` block shows the run:
    `agents: [{run_id:"01R", status:"running", turn:1, next:"continue", last:"fails with seed 42"}]`.
11. The CM calls `agent.run(run_id="01R", input="Isolate seed 42 and report the failing assertion.")`.
12. Turn 2 is long; the summarizer produces a headline and artifacts; the CM receives the compact
    result.

**Turn 2 — the CM drills down, records the cause, and schedules a follow-up**

13. The CM calls `conversation.topic set "seed 42 clock skew"`.
14. The CM calls `conversation.depth set 2`.
15. The CM calls `conversation.status set`:
    ```json
    {
      "phase": "working",
      "tasks": ["reproduce flaky auth refresh test [done]", "fix clock skew [doing]"],
      "next": ["add a controlled clock", "run the full auth suite"],
      "notes": "seed 42 crosses the midnight boundary"
    }
    ```
16. The CM calls `data.create(type="mm:Fact", properties={"mm:statement":"the refresh test reads the wall clock","mm:source":"src/auth/refresh.rs","mm:confidence":0.9})`.
    The broker writes typed RDF, indexes it, and returns `dataRevision 2`. A later search for
    "wall clock" finds it.
17. The CM calls `agent.run(run_id="01R", input="Add a controlled clock and run the full auth suite.")`.
18. Hermes edits files, runs tests, returns `done` with artifacts.
19. The CM calls `conversation.status set` with `phase: "done"`, updated tasks, and `next: []`.
20. The CM calls `metamind.act(action="plan_refinement", mode="scheduled", delay_seconds=3600,
    input="Re-check the refresh plan after the fix.")`. Action `01A` appears as scheduled; the
    inspector can cancel it.
21. The CM answers: "Found it, Ada — the test uses the wall clock; seed 42 crosses the boundary. The
    fix is in `src/auth/refresh.rs`; the full auth suite passes. I've recorded the cause, updated
    the plan, and scheduled a one-hour re-check."

**Panel during the interaction**

- Topic header: `auth refresh flake`, depth 1 → `seed 42 clock skew`, depth 2.
- Status variables: phase working → done; tasks move doing → done; next changes; notes appear.
- Knowledge: `Person · Ada`, the fact with source/confidence, 2 documents, index lag 12ms.
- Plan: `make the refresh test deterministic`, v1 active, 4 steps, 2 done.
- Actions: 1 scheduled `plan_refinement` in 60m.
- Agents: Hermes run `01R`, turn 1 → turn 2 → done.
- Operations: `data.search`, `data.update`, `metamind.plan`, `metamind.think`, `agent.run` new,
  `agent.run` continue, `conversation.topic`, `conversation.depth`, `conversation.status`,
  `data.create`, `metamind.act`.
- Internal variables: state revision, data revision, view revision, depth, active agent count,
  loop round, data write quota.

**Trace tab**

- `plan/01P`: planning prompt, data reads, structured plan output.
- `think/01T`: inner prompt, read-only data reads, findings with evidence. The chat transcript
  does not contain this trace.
- `agent/01R`: turn 1 read/run, turn 2 edit/run, summaries and raw.
- `action/01A`: scheduled, then running/done when due; cancellation is inspector-only.

**What did not happen**

- No one told the CM to call `conversation.topic`, `conversation.depth`, or
  `conversation.status`. It inferred the state changes from the conversation and the visible
  `[STATE]` block.
- No one told the CM to call `data.create`; the stable prompt's durable-fact policy and the
  conversation made it the right action. It searched first, so it updated the existing person and
  did not create a duplicate.
- No 69-tool catalog was advertised. The model saw fourteen job-shaped schemas.
- No project/task CRUD tool existed. The work was tracked in status variables, a durable plan,
  typed data, and delegated agent work.
- The inner thinker's voice never reached the operator; the outer CM answered in character.

This is the target behavior: a legible three-family surface; a state block the model can actually
use; a self-maintaining knowledge graph; durable plans; private thinking; allowlisted action; a
real worker; and a live inspector that shows the same truth the model sees.
