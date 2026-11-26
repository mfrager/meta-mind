# Simple Core: Five-Phase Single-Call Plan

> **Status:** proposed · **Date:** 2026-11-26 · **Scope:** whole workspace,
> agentstream schema, Cubecast context, prompt, server routes, and console
>
> **Goal:** make one user turn one model call, one prompt, one tiny RDF context,
> and one event stream that carries both MetaMind app events and the raw LLM
> records around them. The console renders that stream in two interchangeable
> forms: plain text and a collapsible GUI log.

This plan supersedes the turn-processing half of
`design/streamlined_core_four_phase_plan.md`. The archival, JSONL, prompt-tree,
and single-workspace decisions from that plan stay. The three passes, the
verification loop, the staged conversation state, and the six tools do not.

---

## 1. Executive summary

The current core is already small, but it still treats a turn as a program:
Interpret → Process → Verify, with retries, staged state, verification checks,
and a tool loop. That machinery exists to make a reply *safe*, and it costs
three or more model calls, a large prompt, and a second state system beside the
event log.

The simple core replaces it with a conversation:

1. Load a small RDF context (identity, user, session, topic).
2. Render one prompt that contains the context and the current situation.
3. Make **one** model call through the only LLM wrapper, `AgentStream`.
4. Stream the reply; execute any `data.read` / `data.write` calls from the same
   response; persist the results.
5. Run a deterministic topic/name update if the model did not do it.
6. Emit the MM events that bracket the LLM records.

The frontend no longer reconstructs prompt data from a view model. It renders
the same `llm.request` / `llm.delta` / `llm.response` payloads that were written
to `data/core/agentstream/<session>.jsonl`, with app-level `mm.*` rows before
and after them.

| Concern | Today | After this plan |
|---|---|---|
| Turn shape | Interpret → Process → Verify, retries | One request, one response |
| Model calls per turn | 3–5+ | Exactly 1 |
| Prompt | Three strategies plus pass/state fragments | One strategy, `mm.turn.v1` |
| State | `SessionState` projections plus staged patches | ~8 RDF fields in Cubecast |
| Tools | 6 conversation/metamind tools | 2 data tools |
| LLM wrapper | `LlmGateway` inside a pass pipeline | `AgentStream::complete`, the only door |
| Log | LLM events nested under pass events | MM and LLM events in one ordered stream |
| Console | Transcript + dock + prompt/tools viewer | Transcript + text log + GUI log |
| Crates | 6 (`mm-core`, `mm-prompt`, `mm-llm`, `mm-agentstream`, `mm-cme`, `mm-serve`) | 6 (`mm-cme` leaves; `mm-context` enters) |

---

## 2. Why this is simpler

The painful parts of the current system are not the event log or the JSONL
store. They are the layers that try to decide *whether* a turn is good before it
is shown:

- **Three prompts per turn.** The same conversation is described three times,
  with different tool scopes and different state blocks. A change to one prompt
  usually needs a change to the other two.
- **A retry loop with staged state.** Process proposes a patch; Verify can
  reject it; the patch is discarded or committed. The log has to explain a
  branch that the user never sees.
- **A second state system.** `SessionState`, frames, status, depth, and staged
  patches duplicate facts that the context and the transcript already hold.
- **Tool catalogs that exist for the pipeline, not the user.** Four
  `conversation.*` tools exist to feed the next prompt. `metamind.verify` and
  `metamind.approve` exist to run the retry loop.
- **A frontend that knows the pipeline.** The dock groups by pass and call and
  has tabs for verification and internals. When the pipeline changes, the UI
  changes.

Removing those layers removes the reason for the UI to know anything except
"here is an event, here is its payload".

---

## 3. Target architecture

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ mm-serve                                                                     │
│  POST /api/turns                       start one turn                        │
│  POST /api/mm/events                   append one console/app event          │
│  GET  /api/agentstream/sessions        session index                         │
│  GET  /api/agentstream/sessions/{id}/events?from=<seq>                       │
│  GET  /api/agentstream/sessions/{id}/stream?from=<seq>       (SSE)           │
│  GET  /api/health                                                            │
└───────────────┬──────────────────────────────────────────────────────────────┘
                │ run_turn(session, text, app_context)
┌───────────────▼──────────────────────────────────────────────────────────────┐
│ mm-agentstream                                                               │
│  TurnRunner        load context → render prompt → call → tools → finalize    │
│  AgentStream       THE only LLM wrapper: log, stream, call, log result       │
│  EventStore        append-only JSONL, seq, snapshot, live broadcast          │
│  CleanLog          user/assistant history for the next prompt                │
└───────────────┬──────────────────────────────────────────────────────────────┘
                │ context snapshot / write
┌───────────────▼──────────────────────────────────────────────────────────────┐
│ mm-context (Cubecast)                                                        │
│  core graph        urn:mm:self, urn:mm:user, urn:mm:unknown                   │
│  session graph     urn:mm:session, urn:mm:conversation                        │
│  OWL               compact class/property definitions, embedded in prompt    │
│  tools             data.read(uri, key), data.write(uri, key, value)          │
└───────────────┬──────────────────────────────────────────────────────────────┘
                │ one prompt, one call
┌───────────────▼──────────────────────────────────────────────────────────────┐
│ mm-prompt / mm-llm                                                           │
│  one strategy: mm.turn.v1                                                    │
│  one provider call, streamed                                                 │
└───────────────┬──────────────────────────────────────────────────────────────┘
                │ same records, unchanged
┌───────────────▼──────────────────────────────────────────────────────────────┐
│ Console                                                                      │
│  chat transcript          user.message / agent.message                       │
│  Text log                 one line per event                                 │
│  GUI log                  one row per message, collapsible, prompt sub-rows  │
│  both views               stream deltas, coalesced at ≤ 500 ms               │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Crates

| Crate | Responsibility after this plan |
|---|---|
| `mm-core` | Config, ids, time, errors. No pipeline concepts. |
| `mm-prompt` | Prompt tree, strict renderer, persona, **one** `mm.turn.v1` strategy. |
| `mm-llm` | Provider client, messages, tool schemas, streaming, usage. |
| `mm-context` | **New.** Cubecast-backed RDF context, OWL, defaults, data tools. |
| `mm-agentstream` | Event store, v3 schema, `AgentStream` wrapper, `TurnRunner`, CleanLog. |
| `mm-serve` | Boot, lock, routes, SSE, DTOs. Owns no turn logic. |

`mm-cme` is archived in Phase 2. Its prompt loader moves to `mm-prompt`, its
turn orchestration is replaced by `mm-agentstream::TurnRunner`, and its passes,
tools, budgets, verification, frames, and state block are retired.

### 3.2 Data layout

```
data/core/
  server.lock
  agentstream/
    sessions.jsonl                    # cross-session index
    <sessionId>.jsonl                 # v3 events, seq 1..N
  context/
    cubecast/                         # Cubecast/Oxigraph RocksDB store
    context.lock                      # single-writer guard for the context
```

The context is the only durable state besides the event log. There is no
SQLite database, no RDF event graph, and no projection database. `CleanLog`
folds the JSONL on demand; it is not a store.

---

## 4. The one turn

### 4.1 Request and result

```rust
pub struct TurnRequest {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub user_input: String,
    pub app: AppContext,          // app, view, function that started the turn
}

pub struct TurnResult {
    pub turn_id: TurnId,
    pub reply: String,
    pub model: String,
    pub usage: Usage,
    pub tool_calls: Vec<ToolCallRecord>,
    pub duration_ms: u64,
}
```

There is no `attempts` field and no `verified` field. A turn either produced a
reply or failed; the event log is the record of which.

### 4.2 Sequence

```
mm.turn.submitted          user text, app context
mm.context.loaded          entity IRIs, graph names, field count
mm.prompt.rendered         prompt instance id + hash, not the content
llm.request                complete messages + complete tool schemas
llm.delta                  live only, not persisted
llm.response               complete text + tool calls + usage
llm.tool.call              one per data.read / data.write call
mm.tool.result             value or validation error
mm.context.written         one per successful write
agent.message              the reply the user sees
mm.turn.finished           duration, tool count, stop reason
```

Any of these may be preceded or followed by an app event:

```
mm.app.started             process boot
mm.view.changed            console route or pane changed
mm.error                   a UI or server error worth showing in the log
```

The rule is positional, not structural: an MM event before `llm.request` says
what the app was doing when the call started; an MM event after `llm.response`
says what the app did with it. The frontend does not infer this from nesting.

### 4.3 One call is a hard invariant

`TurnRunner` calls `AgentStream::complete` exactly once. There is no retry, no
fallback provider, no verifier, and no continuation call.

Tool calls are **write-behind/read-behind** under this invariant:

- The prompt contains the complete context block, so the model already has every
  value the context holds.
- If the model emits `data.write`, the runner validates and applies it after the
  response is final. The change is visible in the next turn's context block and
  in `mm.context.written`.
- If the model emits `data.read`, the runner reads the value, logs
  `mm.tool.result`, and includes the result in the next turn's situation
  block. It does not re-prompt the model.
- Tool calls never change the already-streamed reply. The model is instructed to
  write only facts the user stated in this turn, because it cannot consume a
  tool result until the next turn.

This is the deliberate trade: one call, no hidden loop. The context is small
enough that a read is rarely necessary; the tool exists for explicit checks and
for the log to show what the model asked for.

### 4.4 Streaming

`AgentStream::complete` streams from the provider and:

1. appends `llm.request` before the provider is touched (fail-closed);
2. publishes `llm.delta` records on the live bus, never to the file;
3. appends one `llm.response` with the complete text, tool calls, usage, and
   finish reason after the stream ends;
4. appends one `llm.tool.call` per tool call in the response, for the log;
5. returns the exact `LlmResponse` that was logged.

The console merges deltas into the open row. At the terminal event it replaces
the merged text with `llm.response.data.text`, so a dropped delta cannot leave a
permanently truncated reply.

### 4.5 Failure

- Provider error → `llm.error` + `mm.turn.failed`; the turn ends.
- JSON-schema-invalid tool call → `mm.tool.result` with `ok: false`; the reply
  still stands.
- Context store unavailable → `mm.error` + `mm.turn.failed` before any provider
  call.
- Event append failure → no provider call; `mm.turn.failed` is attempted on a
  best-effort basis.

No failure triggers a second model call.

---

## 5. The MM context

### 5.1 Entities and fields

Each entity is its own URI instance. Values are text literals, except the
explicit unknown value.

| Path | Entity URI | Key | Class | Writable | Initial value |
|---|---|---|---|---|---|
| `self.name` | `urn:mm:self` | `name` | `mm:Self` | no | `Metamind` |
| `self.env` | `urn:mm:self` | `env` | `mm:Self` | no | OS, platform, host, cwd (once) |
| `self.state` | `urn:mm:self` | `state` | `mm:Self` | yes | `urn:mm:unknown` |
| `user.name` | `urn:mm:user` | `name` | `mm:User` | yes | `urn:mm:unknown` |
| `session.id` | `urn:mm:session` | `id` | `mm:Session` | no | new ULID per session |
| `session.name` | `urn:mm:session` | `name` | `mm:Session` | yes | `urn:mm:unknown` |
| `conversation.topic` | `urn:mm:conversation` | `topic` | `mm:Conversation` | yes | `urn:mm:unknown` |

Graphs:

- `urn:mm:graph:core` holds `self`, `user`, the OWL schema, and `unknown`.
  Shared by every session.
- `urn:mm:graph:session:<sessionId>` holds `session` and `conversation`, so two
  console tabs cannot overwrite each other's topic or session name.
- `urn:mm:conversation` is the current conversation within that session's graph.
  The fixed URI is intentional: tools and the prompt use the short paths above,
  and graph selection supplies the session scope.

### 5.2 The unknown value

`urn:mm:unknown` is a real instance of `mm:Unknown`:

```turtle
urn:mm:unknown a mm:Unknown .
```

It is both the value of an unknown field and the definition of "no value". The
store normalises all of the following to it on read:

- a missing triple;
- an empty literal `""`;
- a literal whose trimmed value is `Unknown` or `unknown` (case-insensitive);
- a resource `urn:mm:unknown`.

`data.write` with an empty value removes the field's literal and writes
`urn:mm:unknown`. `data.read` always returns a string; unknown fields return
`"Unknown"` with `known: false`.

### 5.3 Compact OWL

The schema is one small Turtle file,
`config/context/mm-context.ttl`, embedded into the binary with `include_str!`.
It is also loaded into the core graph at startup.

```turtle
@prefix mm:   <urn:mm:> .
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .

mm:Self         a owl:Class .
mm:User         a owl:Class .
mm:Session      a owl:Class .
mm:Conversation a owl:Class .
mm:Unknown      a owl:Class .

mm:name  a owl:AnnotationProperty ; rdfs:label "name" .
mm:env   a owl:AnnotationProperty ; rdfs:label "environment" .
mm:state a owl:AnnotationProperty ; rdfs:label "state" .
mm:id    a owl:AnnotationProperty ; rdfs:label "session id" .
mm:topic a owl:AnnotationProperty ; rdfs:label "conversation topic" .
```

`owl:AnnotationProperty` is chosen because a value may be a text literal **or**
the `urn:mm:unknown` resource. That keeps one property per key instead of a
parallel "unknown" property, and keeps the schema short. Domains are omitted on
purpose: `mm:name` is used by both `mm:Self` and `mm:User`, and the instance's
class already supplies the meaning.

### 5.4 Startup

`ContextStore::open` runs once, after the server lock and before the first turn:

1. Open Cubecast at `data/core/context/cubecast` (persistent RocksDB).
2. Create the core graph if absent and load the OWL schema.
3. Create `urn:mm:self` if absent:
   - `mm:name` = `Metamind`;
   - `mm:env` = `format!("{} {}; host {}; cwd {}", os, arch, hostname, cwd)`
     using `std::env::consts::OS`, `std::env::consts::ARCH`, `std::env::var("HOSTNAME")`,
     and `std::env::current_dir()`. Read once; later runs keep the stored value.
   - `mm:state` = `urn:mm:unknown`.
4. Create `urn:mm:user` with `mm:name` = `urn:mm:unknown` if absent.
5. Leave session/conversation creation to `start_session`.

`self.env` is a snapshot, not a live view. If the process starts in a different
directory, the new value is written only when the user explicitly asks the agent
to refresh it (the field is read-only to the model). A `--refresh-env` server
flag may overwrite it in a later phase; it is not required here.

### 5.5 Session, topic, and name rules

`start_session(session_id)`:

- writes `mm:id` = the session ULID;
- writes `mm:name` = `urn:mm:unknown`;
- writes `mm:topic` = `urn:mm:unknown`;
- reuses an existing session graph when the console resumes a session, so a
  reload does not erase the topic.

Topic is set automatically, before the prompt is rendered, on the first turn:

1. If `conversation.topic` is unknown, derive a topic from the user text: the
   first six words, stripped of leading/trailing punctuation, collapsed to one
   line, at most 64 characters. If that is empty, use `"Conversation"`.
2. Write `mm:topic` and emit `mm.context.written`.
3. If `session.name` is unknown, copy the topic into `mm:name`.

The model may refine `conversation.topic` with `data.write` on any turn. A
model write wins for that turn; the deterministic rule only fills an unknown
value. It never overwrites a known topic.

`user.name` follows the same first-write-wins rule:

- If `user.name` is unknown, the situation block asks the model to learn it
  naturally and to call `data.write("urn:mm:user", "name", <name>)` only when
  the user states it in that turn.
- After the response, the runner also checks the user text for
  `my name is X`, `i'm X`, `i am X`, and `call me X`. A match is written only
  when the field is still unknown and the value is 1–4 words of letters,
  apostrophes, or hyphens.
- Once known, the field is not overwritten. The prompt tells the model to use
  the known name and never ask again.

### 5.6 The situation block

`mm.turn.v1` has one general-purpose slot, `{{situation}}`, for real-world and
turn state. The runner composes it from conditions, not from a template per
case:

| Condition | Line added |
|---|---|
| `user.name` unknown | `The user's name is Unknown. Ask for it naturally in this reply. If they state it, call data.write("urn:mm:user", "name", <name>). Do not ask again after it is known.` |
| `user.name` known | `The user's name is {name}. Address them by it when natural.` |
| topic was just set | `This conversation's topic is "{topic}".` |
| session name unknown | `The session will be named from the first topic.` |
| pending read result | `Last data.read result: {uri} {key} = {value}.` |
| tool hint | `Use data.read only for confirmation. The full context is already below.` |

The block is plain text, so it can also carry time, a maintenance notice, or a
future "app is doing X" line without changing the prompt strategy.

### 5.7 The prompt context block

The runner asks `ContextStore::render_rdf_xml(session_id)` for one
`rdf:RDF` document containing:

1. the OWL class and property definitions used by the included entities;
2. the `mm:Unknown` class and the `urn:mm:unknown` instance;
3. the core entities (`self`, `user`);
4. the session's `session` and `conversation` entities.

Only used classes and properties are emitted, so the block stays small and the
model can see the schema for exactly the data in front of it. Escaping is
XML-correct (`&`, `<`, `>`, quotes).

```xml
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns:owl="http://www.w3.org/2002/07/owl#"
         xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#"
         xmlns:mm="urn:mm:">

  <owl:Class rdf:about="urn:mm:Self"/>
  <owl:Class rdf:about="urn:mm:User"/>
  <owl:Class rdf:about="urn:mm:Session"/>
  <owl:Class rdf:about="urn:mm:Conversation"/>
  <owl:Class rdf:about="urn:mm:Unknown"/>

  <owl:AnnotationProperty rdf:about="urn:mm:name"/>
  <owl:AnnotationProperty rdf:about="urn:mm:env"/>
  <owl:AnnotationProperty rdf:about="urn:mm:state"/>
  <owl:AnnotationProperty rdf:about="urn:mm:id"/>
  <owl:AnnotationProperty rdf:about="urn:mm:topic"/>

  <mm:Unknown rdf:about="urn:mm:unknown"/>

  <mm:Self rdf:about="urn:mm:self">
    <mm:name>Metamind</mm:name>
    <mm:env>linux x86_64; host studio; cwd /srv/metamind</mm:env>
    <mm:state rdf:resource="urn:mm:unknown"/>
  </mm:Self>

  <mm:User rdf:about="urn:mm:user">
    <mm:name rdf:resource="urn:mm:unknown"/>
  </mm:User>

  <mm:Session rdf:about="urn:mm:session">
    <mm:id>01JC9W4T7Q9K5S2V8M3N6P1R0X</mm:id>
    <mm:name rdf:resource="urn:mm:unknown"/>
  </mm:Session>

  <mm:Conversation rdf:about="urn:mm:conversation">
    <mm:topic rdf:resource="urn:mm:unknown"/>
  </mm:Conversation>

</rdf:RDF>
```

### 5.8 The two tools

Only these are sent to the model:

```json
{
  "name": "data.read",
  "description": "Read one text attribute of a MetaMind context entity.",
  "parameters": {
    "type": "object",
    "properties": {
      "uri": { "type": "string", "description": "Entity IRI, e.g. urn:mm:user" },
      "key": { "type": "string", "pattern": "^[a-z_]+$" }
    },
    "required": ["uri", "key"]
  }
}
```

```json
{
  "name": "data.write",
  "description": "Write one text attribute of a MetaMind context entity.",
  "parameters": {
    "type": "object",
    "properties": {
      "uri": { "type": "string", "description": "Entity IRI, e.g. urn:mm:user" },
      "key": { "type": "string", "pattern": "^[a-z_]+$" },
      "value": { "type": "string", "description": "Text value; empty means Unknown" }
    },
    "required": ["uri", "key", "value"]
  }
}
```

Validation is a fixed allow-list, not an open graph API:

| URI | Allowed keys | Write |
|---|---|---|
| `urn:mm:self` | `name`, `env`, `state` | only `state` |
| `urn:mm:user` | `name` | yes, only when unknown |
| `urn:mm:session` | `id`, `name` | only `name` |
| `urn:mm:conversation` | `topic` | yes |

An unknown URI, key, or read-only write returns
`{ "ok": false, "error": "..." }` in `mm.tool.result`. No arbitrary IRIs,
no blank nodes, no SPARQL, and no graph creation are exposed to the model.

Execution order after the response:

1. `data.write` calls, in the order the model emitted them.
2. The deterministic user-name check.
3. `data.read` calls (their results are only for the log and the next
   situation block).
4. Emit `mm.context.written` for each changed field.

Writes are applied even if the reply is empty, because the model may have
chosen to answer entirely through the tool. The turn still ends with
`agent.message` carrying the (possibly empty) text.

---

## 6. The one prompt

### 6.1 Strategy

`config/prompts/applications/mm.turn.v1.toml` is the only application strategy.
It references four fragments, in order:

| Fragment | Contents |
|---|---|
| `mm/identity` | Who MetaMind is, tone, honesty rules, and the single-call contract. |
| `mm/context-guide` | How to read the RDF/XML block; what `self.*`, `user.*`, `session.*`, and `conversation.*` mean; when to use `data.read` / `data.write`. |
| `mm/situation` | `{{situation}}` only. |
| `mm/reply` | Reply shape, tool-call rules, and the unknown/ask-for-name behaviour. |

The manifest lists exactly this strategy. `cm.interpret.v1`, `cm.process.v1`,
`cm.verify.v1`, and every frame strategy are archived. The prompt manager,
strict single-pass renderer, hashing, and `PromptInstance` remain unchanged.

### 6.2 Assembly

```
system:
  <mm/identity>
  <mm/context-guide>
  ## Context (RDF/XML)
  <rendered rdf:RDF block>
  ## Situation
  <situation lines>
  <mm/reply>

history:
  CleanLog turns, oldest first, as normal user/assistant messages,
  bounded to the last N turns and a character budget

user:
  the current user input
```

The RDF block and situation live in the system message, not in the user
message, so the transcript stays clean and the model sees the schema as
instructions. `mm.prompt.rendered` records the instance id and hash; the full
text is in `llm.request.data.messages`, as it is today.

### 6.3 Prompt instructions that matter

The assembled prompt must say, in plain language:

- `self.name` is the entity's name; `self.env` describes the machine and
  working directory and is read-only; `self.state` is your current focus,
  confidence, and open questions, and you may update it.
- `user.name` is the person you are talking to. If it is Unknown, ask for it
  once, naturally. When they tell you, call `data.write` with the exact name.
  Never guess it and never ask again once it is known.
- `session.id` identifies this conversation; `session.name` is its label;
  `conversation.topic` is what you are talking about. The backend sets the
  first topic and session name automatically; you may refine the topic with
  `data.write`.
- The context block is complete. Use `data.read` only to confirm a value; its
  result arrives next turn. Use `data.write` only for facts the user stated.
- There is exactly one model call per turn. Do not ask for a second pass, do
  not emit a verification request, and do not plan out loud unless the user
  asked for a plan.
- If a tool call is invalid, the reply still stands; do not retry it.

---

## 7. The event log, v3

### 7.1 Envelope

```json
{
  "v": 3,
  "seq": 17,
  "id": "01JC9W4T7Q9K5S2V8M3N6P1R0X",
  "sessionId": "01JC9W4T7Q9K5S2V8M3N6P1R0X",
  "turnId": "01JC9W4T7Q9K5S2V8M3N6P1R0Y",
  "at": 1764115256123,
  "type": "llm.request",
  "source": "agentstream",
  "actor": "llm",
  "model": "nvidia/nemotron-3.5-lightning-30b-a3b",
  "context": {
    "app": "mm.console",
    "view": "workspace",
    "fn": "turn.submit"
  },
  "data": {}
}
```

New in v3:

| Field | Meaning |
|---|---|
| `actor` | `mm`, `llm`, or `user`. The console uses it for row colour and filters. |
| `context` | Optional app context: `app`, `view`, `fn`. Every LLM record carries the context of the turn that caused it. |

`pass` and `round` are removed. `attempt` is removed. `seq` still orders the
stream and is the SSE id.

### 7.2 Event types

| Type | Actor | Data |
|---|---|---|
| `session.started` | mm | `{ userId, title? }` |
| `session.ended` | mm | `{ reason }` |
| `user.message` | user | `{ text }` |
| `agent.message` | mm | `{ text, callId }` |
| `mm.turn.submitted` | mm | `{ userInput }` |
| `mm.turn.finished` | mm | `{ reply, model, usage, durationMs, toolCalls }` |
| `mm.turn.failed` | mm | `{ error, kind }` |
| `mm.prompt.rendered` | mm | `{ instanceId, hash, strategy, variables }` |
| `llm.request` | llm | `{ callId, messages, tools, params, model, prompt }` |
| `llm.delta` | llm | `{ callId, index, text }` — live only |
| `llm.response` | llm | `{ callId, text, toolCalls, usage, finishReason, streamed }` |
| `llm.error` | llm | `{ callId, message, kind }` |
| `llm.tool.call` | llm | `{ callId, toolCallId, name, arguments }` |
| `mm.tool.result` | mm | `{ callId, toolCallId, name, ok, value?, error? }` |
| `mm.context.loaded` | mm | `{ graphs, entities, fields }` |
| `mm.context.written` | mm | `{ uri, key, value, by }` |
| `mm.app.started` | mm | `{ version, pid }` |
| `mm.view.changed` | mm | `{ from, to }` |
| `mm.error` | mm | `{ message, kind, fn }` |

`llm.request.data.messages` and `.tools` are the exact values sent to the
provider. `llm.response.data.text` is the exact text returned. The console does
not rebuild either from a projection. `llm.delta` is broadcast live and never
appended, so the file remains replayable and compact.

### 7.3 MM events from the console

`POST /api/mm/events` appends one app-level event to a session:

```json
{
  "sessionId": "01JC...",
  "type": "mm.view.changed",
  "context": { "app": "mm.console", "view": "workspace", "fn": "route.change" },
  "data": { "from": "chat", "to": "log" }
}
```

Allowed types from the console are `mm.view.changed`, `mm.error`, and
`mm.app.started`. The server stamps `seq`, `id`, `at`, and `actor: "mm"`; the
client cannot set those fields. The event flows through the same broadcast as
LLM events, so an open SSE stream shows app events inline.

The server emits the same event types for its own functions, with
`context.fn` naming the function (`turn.submit`, `context.load`, `tool.write`).
This is what makes the log readable as "what the MM app was doing around this
model call".

### 7.4 Routes and SSE

Unchanged from the four-phase plan, except for the added `POST /api/mm/events`:

- `POST /api/turns` → `202 { sessionId, turnId, seq }`, starts the turn.
- `GET /api/agentstream/sessions`
- `GET /api/agentstream/sessions/{id}/events?from=<seq>` → `EventPage`
- `GET /api/agentstream/sessions/{id}/stream?from=<seq>` → SSE, `id: <seq>`,
  `event: <type>`, `data: <record>`.
- `resync` payload carries an `EventPage`.

Reconnect uses `lastSeq + 1`; duplicate seqs are ignored. No new transport is
introduced for the console's app events.

---

## 8. The console

### 8.1 Two views of one stream

A single toggle, persisted in `prefs`, switches the log between:

- **Text** — monospaced, one line per event:
  `12:04:07.312  MM   mm.turn.submitted   text="…"`
  `12:04:07.318  LLM  llm.request         3 messages · 2 tools · 412 chars`
  `12:04:07.402  LLM  llm.response        88 chars · 142/31 tok`
  `12:04:07.403  MM   mm.context.written  user.name = "Alex"`
  Long values wrap or truncate with a copy action. Streaming updates the last
  line in place.
- **GUI** — one row per message, including outer MM messages, with a
  collapsible body.

Both read the same store. Switching views never re-fetches.

### 8.2 Rows and sub-rows

GUI row kinds:

| Kind | One row per | Body when expanded |
|---|---|---|
| `user` | `user.message` | full text |
| `mm` | each `mm.*` event | event fields, formatted; `context.fn`, graph, or error |
| `llm` | each `llm.request` (its response is part of the row) | itemized prompt messages, tools, streamed/received response |
| `tool` | each `llm.tool.call` | arguments and result on one line, expandable |

Rules:

- Every row header shows actor, event type, time, turn id, and status
  (`streaming`, `ok`, `error`).
- The LLM prompt's `messages` array is itemized into sub-rows, one per message,
  in order. Each sub-row has a role badge and its own collapse control.
- A collapsed row shows **only its newest sub-row**. A prompt row that has just
  received a new user message therefore previews that message; an expanded row
  shows the whole prompt.
- Tool calls get their own line under the LLM row, not inside the prompt body:
  `data.write urn:mm:user.name = "Alex"  ✓`.
- Outer MM rows sit between LLM rows in seq order: context load and prompt
  render before the request, tool execution and context write after the
  response, turn completion last.

Collapsed GUI example (the newest sub-row is the preview):

```
[MM]  mm.turn.submitted    12:04:07.312  >  user: "My name is Alex"
[MM]  mm.context.loaded    12:04:07.315  >  7 fields, 2 graphs
[LLM] llm.request          12:04:07.318  >  user: "My name is Alex"
[LLM] llm.response         12:04:07.402  v  "Nice to meet you, Alex."
      [tool] data.write urn:mm:user.name = "Alex"  ok
[MM]  mm.context.written   12:04:07.403  >  user.name = "Alex"
```

### 8.3 Streaming and coalescing

- `llm.delta` updates the open LLM row's response buffer immediately in the
  store, but the visible text is published on a 250 ms tick (never more than
  500 ms). This is the "coalesce within ½ second" rule for both views.
- A terminal `llm.response` replaces the buffer with the authoritative text and
  cancels the pending tick.
- Expanding or collapsing a row is local UI state and does not reset the
  stream or lose the buffer.
- Auto-scroll only while the viewport is pinned to the bottom; a manual scroll
  pauses it until the "jump to latest" control is used.
- The text view applies the same tick to its last line.

### 8.4 Chat transcript

The transcript stays a small projection of `user.message` and `agent.message`.
It does not read prompt or tool records. Streaming may show the live assistant
row, but the final `agent.message` is authoritative.

### 8.5 Components and state

New:

```
frontend/src/components/log/
  LogPanel.tsx          dock shell, mode toggle, filters, resize, pause/clear
  TextLog.tsx           one line per event, streamed last line
  EventRow.tsx          header + collapsible body, newest-sub-row preview
  PromptSubrows.tsx     itemized messages, per-message collapse
  ToolLine.tsx          one tool call per line
frontend/src/lib/store/log.ts       v3 rows, deltas, mode, coalescing
frontend/src/lib/log/model.ts       row grouping and summaries
```

Retired in Phase 5:

```
frontend/src/components/agentstream/AgentstreamDock.tsx
frontend/src/components/agentstream/CallViewer.tsx
frontend/src/lib/store/agentstream.ts
frontend/src/lib/transcript/model.ts     (replaced by log/model + wire v3)
```

The transcript reducer stays, narrowed to `user.message` / `agent.message`.
The wire types are regenerated from the Rust DTOs.

---

## 9. Five phases

Each phase ends with a green check and a working demo. No phase leaves the
workspace half-migrated.

### Phase 1 — Contract, fixtures, and the first archive

**Goal:** freeze the v3 shape and remove code that is already unused. No
runtime behaviour changes.

**Work**

- Add this plan and index it.
- Archive the currently unreferenced frontend modules:
  `components/ui/controls.tsx`, `components/ui/display.tsx`, and
  `lib/store/session.ts`.
- Record the v3 envelope and event list as a fixture-local JSON schema,
  `crates/mm-agentstream/tests/fixtures/v3/schema.json`. Do not change the Rust
  constants or the generated TS wire file yet; the console stays untouched.
- Add golden fixtures under `crates/mm-agentstream/tests/fixtures/v3/`:
  - `one_turn.jsonl` (MM before, request, deltas, response, tool write, MM
    after);
  - `unknown_context.rdf.xml`;
  - `one_turn.messages.json` (the exact prompt messages).
- Add a fixture test that parses every line, checks monotonic `seq`, and checks
  that `llm.request.data.messages` is non-empty and complete.
- Extend `scripts/check-core-only.sh` with a fixture check that rejects a v3
  line with a missing envelope field. Enable the `mm-context` member check in
  Phase 3.

**Archive**

- The three unused frontend files above, with rows in
  `archive/legacy/MANIFEST.md`.
- No backend module; every current backend file is still live in Phase 1.

**Tests / exit**

- `cargo test --workspace` green.
- `npm run typecheck`, `npm test`, `npm run build` green.
- Fixture test green.
- `git status` shows only the archive moves, this plan, fixtures, and the event
  constants.

### Phase 2 — One call, one prompt, one wrapper

**Goal:** replace the three-pass pipeline with the single-call runner, without
adding the RDF context yet (a placeholder string is fine).

**Work**

- `mm-agentstream`
  - Replace `LlmGateway` with `AgentStream`:
    `complete(CompleteRequest) -> Result<LlmResponse>`.
    It appends `llm.request`, streams `llm.delta`, appends `llm.response` or
    `llm.error`, and returns the logged response.
  - Add `turn.rs`: `TurnRunner::run(TurnRequest) -> TurnResult`.
  - Add `prompt_input.rs` (or reuse `projection::CleanLog`) to turn history into
    provider messages.
  - Trim `event.rs` to the v3 set; remove pass, verification, state, and
    internal-action records.
  - Trim `state.rs` to what `CleanLog` needs; remove frames and `StatePatch`.
- `mm-prompt`
  - Move `Persona` and `PromptVariables` from `mm-cme/src/prompt.rs`.
  - Add the single `mm.turn.v1` strategy and four fragments.
  - Remove the three-pass strategies and frame support from `config/`.
- `mm-serve`
  - `AppState` holds `EventStore`, `AgentStream`, `TurnRunner`, `PromptStore`.
  - `POST /api/turns` calls the runner directly.
  - Remove pass/verification filters and DTO fields.
- Console (compile-only adaptation)
  - Regenerate `frontend/src/lib/wire/agentstream.ts` from the v3 DTOs.
  - Remove references to pass, attempt, and verification fields so `npm run
    typecheck` stays green. Keep the current dock working in reduced form; its
    replacement is Phase 5.
- `mm-cme`
  - `git mv` the crate to `archive/legacy/crates/mm-cme/`.
  - Remove it from `Cargo.toml` and from the guard's expected set.
- `mm-serve` no longer depends on `mm-cme`.

**Archive**

- `mm-cme` in full: pipeline, interpret, process, verify, tools, budgets,
  pass_scope, state_block, verification, prompt adapter, tests, fixtures that
  are not reused.
- `config/three_pass.toml`, `config/prompts/...` strategies and fragments that
  are not part of `mm.turn.v1`.
- Old `AgentstreamDock` / `CallViewer` stay until Phase 5, but their pass and
  verification tabs are already empty; do not add new features to them.

**Tests / exit**

- New `crates/mm-agentstream/tests/single_call.rs`:
  - the mock provider counts exactly one request per turn;
  - `llm.request` contains the full prompt and exactly two tool schemas;
  - streamed deltas assemble to `llm.response.text`;
  - a provider error produces one `llm.error` and `mm.turn.failed`, with no retry;
  - a resumed session rebuilds history from `CleanLog`.
- Existing route tests are rewritten for v3.
- `cargo test --workspace` green with five crates.
- `npm run typecheck` and `npm test` green after the compile-only wire
  adaptation; the old dock still renders one-call rows.
- A mock turn logs the full prompt and receives a reply in one call.

### Phase 3 — The Cubecast MM context

**Goal:** add the tiny RDF context and the two data tools.

**Work**

- New crate `crates/mm-context`:
  - `store.rs` — Cubecast open/init, graph helpers, read/write, unknown
    normalisation, restart persistence.
  - `schema.rs` — embedded `config/context/mm-context.ttl`, used-class
    rendering, property filtering.
  - `model.rs` — entity/key allow-list, typed errors, `ContextSnapshot`.
  - `env.rs` — one-shot OS/platform/host/cwd probe.
  - `tools.rs` — `data.read` / `data.write` execution and schemas.
  - `tests/` — defaults, unknown, round-trip, restart, render, validation.
- `ContextStore::start_session`, `set_topic_if_unknown`,
  `set_user_name_if_unknown`, `render_rdf_xml`.
- Wire `mm-context` into `AppState` and `TurnRunner`:
  - `mm.context.loaded` before the prompt;
  - RDF/XML block in the system prompt;
  - tool execution after the response;
  - `mm.context.written` events.
- Cubecast dependency follows `design/rdf_backend_integration.md`
  (`Cubecast::from_config`, `RdfStore`, persistent RocksDB). Keep Cubecast
  behind `ContextStore`; no route or prompt code touches its API.
- Config: `[context] dir = "data/core/context"` in `config/server.toml`.

**Archive**

- Any remaining `mm-cme` fixture that encodes conversation state.
- Old `crates/mm-cme/tests/fixtures/golden/` if not already moved.

**Tests / exit**

- The prompt contains one `rdf:RDF` block with the seven fields and the OWL for
  the used classes only.
- Initial values are exactly: `self.name = Metamind`, `self.env` populated,
  `self.state = Unknown`, `user.name = Unknown`, `session.id` new,
  `session.name = Unknown`, `conversation.topic = Unknown`.
- `urn:mm:unknown` round-trips as `Unknown` with `known: false`; an empty write
  restores it.
- A write survives a process restart.
- `data.write` rejects an unknown URI, an unknown key, and a read-only field.
- `cargo test --workspace` green with six crates.

### Phase 4 — MM events, dynamic situation, and the identity of the log

**Goal:** make the log readable as an MM app trace, and finish the topic/name
behaviour.

**Work**

- Emit the v3 MM events around every turn:
  `mm.turn.submitted`, `mm.context.loaded`, `mm.prompt.rendered`,
  `llm.tool.call`, `mm.tool.result`, `mm.context.written`, `mm.turn.finished`,
  `mm.error`. The model-emitted tool call is logged by the wrapper; the runner
  logs the result, so there is no duplicate "requested" event.
- Add `POST /api/mm/events` and the console client call for
  `mm.view.changed` / `mm.error`.
- Add the `context` / `actor` fields to the envelope and to every DTO.
- Compose the `situation` block from conditions:
  unknown user name (ask + write instructions), known user name, first topic,
  session naming, pending read result, tool hint.
- Implement the deterministic topic extractor and the first-write-wins name
  rules.
- Ensure identity: `llm.request` and `llm.response` payloads are byte-for-byte
  the data the wrapper sent/received; no previews, no truncation, no
  re-serialisation that changes field order.
- Update `file/log` tests to compare the JSONL line bytes with the SSE payload
  bytes.

**Archive**

- `mm.prompt.rendered.data.variables` after it is confirmed that the RDF block is
  in `llm.request` (the viewer does not need a duplicate).
- Any old route or DTO that names a pass or a verification.

**Tests / exit**

- One turn's JSONL contains MM rows before and after the LLM rows, in order.
- `llm.request` shows the complete prompt with the RDF/XML block.
- `llm.response` shows the complete reply.
- Unknown name adds the ask-for-name line; providing a name writes it once and
  the next prompt shows the known name with no ask line.
- First turn sets topic and session name; a model `data.write` can refine the
  topic; a known topic is never overwritten by the heuristic.
- SSE reconnect after seq N duplicates nothing.
- `curl` of the events route shows the full prompt and the RDF block.

### Phase 5 — The two console views

**Goal:** replace the dock/viewer with the text log and GUI log, and finish the
frontend prune.

**Work**

- Regenerate `frontend/src/lib/wire/agentstream.ts` for v3.
- Add `lib/store/log.ts`:
  - bounded records;
  - delta buffer by `callId`;
  - 250 ms coalesced publish;
  - mode (`text` | `gui`), filters, paused, pinned;
  - row grouping by turn and seq.
- Add `components/log/`:
  - `LogPanel.tsx` — toggle, filters (All / MM / LLM / Tools / Errors),
    pause, clear, resize, collapse, connection state;
  - `TextLog.tsx` — one line per event, streaming last line, copy;
  - `EventRow.tsx` — header, collapsed newest-sub-row preview, expansion;
  - `PromptSubrows.tsx` — itemized prompt messages, each collapsible;
  - `ToolLine.tsx` — one tool call per line with arguments and result.
- Replace `AgentstreamDock` in `workspace-page.tsx` / `chat-pane.tsx`.
- Keep the chat transcript and composer; narrow the reducer to
  `user.message` / `agent.message`.
- Persist the chosen view and dock height in `prefs`.
- Update the e2e mock server and spec for a full one-call turn with MM rows.
- Update `README.md`, `frontend/README.md`, and the archive manifest.

**Archive**

- `components/agentstream/AgentstreamDock.tsx`,
  `components/agentstream/CallViewer.tsx`, and their tests.
- `lib/store/agentstream.ts` and its tests; `lib/transcript/model.ts` and its
  tests if the new `log/model.ts` replaces them.
- `hooks/use-agentstream.ts` if the new store owns the stream; otherwise keep
  and adapt it.
- Any filter or CSS that exists only for pass/verification rows.

**Tests / exit**

- Text view: one line per event, in seq order, streamed text updates in place.
- GUI view: one row per user/MM/LLM message; collapsed rows show only the
  newest sub-row; expansion shows all prompt sub-rows; tool calls have their
  own line.
- Both views share the same live buffer and survive a view toggle.
- Coalescing test with fake timers: 20 deltas in 100 ms produce at most one
  visible update per 250 ms tick and the final text is exact.
- `npm run typecheck`, `npm test`, `npm run build`, `npm run lint`,
  `npx playwright test` green.
- Only the kept frontend files exist; archive guard passes.
- A mock turn shows MM before/after rows, the full prompt as sub-rows, the
  streamed reply, and the `data.write` line.

---

## 10. Archive and cleanup

### 10.1 Done now (this plan)

| Path | Destination | Reason |
|---|---|---|
| `frontend/src/components/ui/controls.tsx` | `archive/legacy/frontend/src/components/ui/` | No kept source imports it. |
| `frontend/src/components/ui/display.tsx` | `archive/legacy/frontend/src/components/ui/` | No kept source imports it. |
| `frontend/src/lib/store/session.ts` | `archive/legacy/frontend/src/lib/store/` | Unused session store; the workspace owns session selection. |

No backend file qualifies today: every module in the six crates is reachable
from `mm-serve` or a test. `mm-cme` is archived in Phase 2 only after the
single-call runner replaces it, so the workspace never breaks.

`archive/legacy/MANIFEST.md` records the moves and the current kept list.

### 10.2 Later archives

| Phase | Moved |
|---|---|
| 2 | `crates/mm-cme/` (whole crate), `config/three_pass.toml`, pass strategies/fragments |
| 3 | Remaining CME golden fixtures |
| 4 | Old pass/verification routes and DTOs, `mm.prompt.rendered.variables` if duplicated |
| 5 | `components/agentstream/`, `lib/store/agentstream.ts`, old transcript model/tests |

### 10.3 Archive rules

- Use `git mv`, never `rm`, so history follows the code.
- Every move gets a manifest row with the reason.
- Archived code is excluded from the workspace, Vitest, Playwright, and CI.
- The guard fails if a kept source imports an archived crate or surface.
- No archived file is edited to keep it compiling; it is frozen.

---

## 11. Acceptance criteria

| Requirement | Evidence |
|---|---|
| One LLM call per turn | Mock provider count is exactly 1 in `single_call.rs`; one `llm.request` and one terminal response per turn. |
| One internal wrapper | `rg "LlmClient" crates/mm-agentstream crates/mm-serve` shows the client only inside `AgentStream`; the runner calls `complete` once. |
| One main prompt | `config/prompts/applications/` contains only `mm.turn.v1.toml`; startup validation rejects a second application. |
| MM events before/after LLM | `one_turn.jsonl` has `mm.*` rows on both sides of `llm.request` / `llm.response`, in seq order. |
| LLM data is the agentstream data | `llm.request` / `llm.response` DTOs are the logged records; a round-trip test compares bytes. |
| Seven context fields | Context test asserts all seven paths and initial values. |
| Unknown URI | `urn:mm:unknown a mm:Unknown`; read/write/empty/missing all normalise to `Unknown`. |
| OWL per class | Prompt test asserts only used classes/properties and one RDF/XML block. |
| Topic auto-set | First turn sets `conversation.topic` and copies it to a blank `session.name`. |
| Name learned once | Unknown prompt line; a name in the user text is written once; a known name adds no ask line. |
| Two log views | Text and GUI toggle renders the same event set; tests cover both. |
| Rows collapse | Collapsed row shows only its newest sub-row; expansion shows all. |
| Prompt itemized | Sub-rows equal `llm.request.data.messages.length`. |
| Tools own line | Each `llm.tool.call` has its own row/line. |
| Streaming | Deltas update both views and settle to the terminal text. |
| Coalescing ≤ 500 ms | Fake-timer test asserts visible update cadence and final value. |
| Archive integrity | `scripts/check-core-only.sh` passes; manifest matches `git status`. |

---

## 12. Decisions, risks, and open questions

### Decisions

1. **Strictly one call.** Tools are executed after the response, not before a
   continuation. This is the only way to keep the "one call" guarantee and the
   UI's one-request-per-turn shape. Reads are one turn behind by construction;
   the full context block makes that acceptable.
2. **`mm-cme` is retired, not shrunk.** A pipeline crate with one function is
   still a pipeline crate. `TurnRunner` lives in `mm-agentstream` beside the
   only wrapper and the event log.
3. **The context is tiny and fixed.** Seven fields, four entities, five
   properties. There is no generic RDF tool and no SPARQL surface. Extending it
   is a code change, not a model action.
4. **Fixed URIs plus per-session graphs.** Short paths in the prompt, no URI
   collisions across sessions. `self` and `user` are shared; `session` and
   `conversation` are per graph.
5. **Unknown is a resource.** `urn:mm:unknown` is an instance and a value, and
   empty/missing text normalises to it. This gives the model one thing to check.
6. **Annotation properties.** They allow a literal for known values and the
   unknown resource for unknown ones without a parallel property family.
7. **Topic is deterministic first, model-refined later.** No extra model call
   is spent naming a conversation.
8. **Name is prompt-first with a deterministic fallback.** The prompt teaches
   the tool call; the fallback guarantees the requirement when the model
   forgets.
9. **Deltas are live-only.** The durable log stores the complete response. The
   UI merges deltas and replaces them at the terminal event.
10. **Cubecast stays behind `mm-context`.** The integration doc's API can
    change without touching the runner, routes, or prompt.

### Risks

| Risk | Mitigation |
|---|---|
| Cubecast build weight / RocksDB | One persistent store; no per-event RDF writes; measure build time in Phase 3; vendor only if the path dependency is unstable. |
| Single-call reply quality | The context and situation do the work the verifier used to do; keep the prompt short and explicit; add an eval fixture before Phase 3. |
| Tool calls cannot inform the current reply | Document it in the prompt; keep the context block complete; include the previous read result in the situation block. |
| Prompt injection through context text | All context values are app-controlled except the user name and topic; escape XML; keep tool allow-list fixed. |
| Streaming tool-call JSON is partial | Buffer tool-call deltas by index; only execute after `llm.response`; never render partial arguments as final. |
| Session concurrency | Named graphs per session; one writer lock; the console can open several sessions without mixing fields. |
| XML block growth | Only used classes/properties; the graph is fixed at seven fields; add a 4 KiB render budget test. |
| Losing verification | This is intentional. The trade is transparency and one call, not guaranteed correctness. The log makes mistakes visible; a future verifier can be a separate, explicit turn. |

### Open questions

1. Should `self.state` be free text or a tiny `key=value` string? The plan
   assumes free text; the tool writes one string either way.
2. Should `data.read` be sent to the model at all, given the complete context
   block? The plan keeps it because the requirement names it and it makes the
   model's information requests visible in the log.
3. Should a model write to `conversation.topic` replace the deterministic first
   topic? The plan says yes for later turns; only unknown values are filled
   automatically.
4. Should the console's `mm.view.changed` events be on by default? They add
   noise; the plan makes them opt-in in the log filter and persisted in prefs.

---

## 13. Target file map

```
crates/
  mm-core/            config, error, id, time (hash only if still used)
  mm-prompt/          prompt store, renderer, persona, mm.turn.v1
  mm-llm/             client, openai, mock, schema, usage
  mm-context/         NEW: cubecast store, OWL, entities, data tools
  mm-agentstream/     event v3, store, bus, projection, AgentStream, TurnRunner
  mm-serve/           main, state, routes/{turns,agentstream,sessions,health,mm},
                      dto, sse, metrics

config/
  server.toml         data dirs, context dir, model
  llm.toml            one provider profile
  persona.toml        identity text used by mm/identity
  context/mm-context.ttl
  prompts/
    manifest.toml
    applications/mm.turn.v1.toml
    fragments/mm/{identity,context-guide,situation,reply}.toml

frontend/src/
  components/log/{LogPanel,TextLog,EventRow,PromptSubrows,ToolLine}.tsx
  components/chat/{chat-pane,composer,transcript-view}.tsx
  components/workspace/workspace-page.tsx
  lib/store/{log,prefs}.ts
  lib/log/model.ts
  lib/api/{agentstream,turns,mm}.ts
  lib/wire/agentstream.ts        (generated v3)

archive/legacy/
  crates/mm-cme/                 (Phase 2)
  frontend/src/components/agentstream/   (Phase 5)
  frontend/src/lib/store/agentstream.ts  (Phase 5)
  frontend/src/components/ui/{controls,display}.tsx   (now)
  frontend/src/lib/store/session.ts                   (now)
```
