# Agentstream as the Sole LLM Interface — Migration Plan

> **Status:** proposed design · **Date:** 2026-11-23 · **Baseline:** `integration/cme-rdf-backend`
> **Scope:** `mm-llm-agentstream` (new crate), `mm-llm`, `mm-agent`, `mm-cli`, `mm-serve`,
> `mm-data`, `mm-cme`, `mm-process`, `mm-decision`, `mm-metaanalysis`, `mm-metacog`, `config/`
> **Flags:** `MM_LLM_AGENTSTREAM` (default `true` after the migration gate)
>
> This document is a plan, not an implementation. It defines the contract, the exact seams,
> the migration order, and the acceptance tests. No code below ships until its phase is
> implemented and verified.
>
> **Implementation status (2026-11-24):** P0, P1, P2, P3, P4, P5, and P6 are implemented. `mm-llm-agentstream`
> owns the gateway contract and core (`AgentstreamLlm`, `GatewayParts`, `GatewayBuilder`,
> `ProviderProfile`, `ProviderProfiles`, `ProviderPool`, `LiveLlmEvent`, `JournalMode`). The
> gateway appends a required `llm_request` before dispatch, one `llm_attempt` per provider round,
> and one terminal `llm_response`/`llm_error`; it owns `LlmService`, schema forwarding, the
> `default`/`hermes` profiles, per-profile timeout and `omit_max_tokens`, injected scripted/replay
> clients, and a bounded live broadcast. `LlmService` now honours a gateway-assigned call id and
> reports the selected profile through `provider_for`.
>
> P2 builds one `Arc<AgentstreamLlm>` at server boot and routes the controller, CME, process,
> automation, summarizer, and hosted-decision paths through it. `LlmActivityFeed` subscribes to
> the gateway live events and projects them onto the unchanged activity DTO. Scripted controller
> mode injects its `MockClient` into the gateway. `AppState::observed`, the old server-side
> provider wrappers, and `controller::turn::service` are gone; no source under `mm-serve/src`
> constructs an `OpenAiClient`.
>
> P3 removes Hermes's private HTTP/SSE/JSON transport. `HermesBackend::with_gateway` maps
> `AgentRequest` onto the gateway's neutral request and maps `LlmResponse` back into the same
> `AgentTurnResult`, scoped to `agent/<run_id>`. `HermesBackend::refresh` probes through
> `gateway.models("hermes")`, which journals an `llm_models` event and preserves the redacted
> unavailable reason. The Phase 6 fake server covers streaming, non-streaming, JSON fallback,
> request-body policy, status/usage mapping, and probe journaling. Pi remains unchanged.
>
> P4 moves the CLI's remaining callers onto the gateway. `decision --core hosted` opens the
> persistent data layer and journals its call there; `llm replay` and `llm schema-reject` build
> gateways over an in-memory journal with injected replay and mock clients. No direct
> `OpenAiClient` or `LlmService` construction remains outside the gateway and its own tests, and
> the workspace source guard passes.
>
> P5 retires the observation decorator. `mm-llm` now keeps only the neutral `CallContext` and
> `scope` in `mm_llm::call`; `ObservedClient`, `ActivitySink`, the old `observe` wrapper, and the
> decorator's tests are gone. The gateway's live bus owns delta coalescing and the 256-char /
> 50 ms / 2,000-char display policy, including `MM_LLM_ACTIVITY_DELTA_MS`. `LlmActivityFeed`
> projects gateway live events only, and its tests publish through `LiveLlmEvent` rather than a
> second client wrapper.
>
> P6 makes the migration enforceable. `tests/no_direct_clients.rs` is a required test: it scans
> workspace `src/` trees, allows the adapter's own definition, and asserts one gateway site for
> `OpenAiClient::from_env` and `LlmService::new`. `MM_LLM_AGENTSTREAM=off` (also `false`, `0`, `no`)
> makes the gateway's live calls and model probes return one configuration error without journaling
> or dialing; the health line reports the disabled reason. The README, operator guide, LLM
> integration doc, design index, and the historical activity plan now describe the gateway as the
> live door and mark the decorator as superseded.
>
> **Pi is explicitly out of scope.** `mm-pi` is an external process protocol, not an
> OpenAI-compatible LLM client. `PiBackend` and the Pi RPC path stay exactly as they are.

---

## 0. Reading this document

| If you want to know… | Read |
|---|---|
| Why this migration exists | §1 |
| Every current direct provider path | §2 |
| The target shape in one page | §3 |
| The public API and request/response changes | §4 |
| The agentstream event contract | §5 |
| How sessions and calls are correlated | §6 |
| How the live activity dock survives | §7 |
| How Hermes is converted | §8 |
| What happens to Pi | §9 |
| What gets deleted and how bypass is prevented | §10 |
| What to build, in what order | §11 |
| How it is tested and accepted | §12–§13 |
| Risks and open decisions | §14–§15 |

**Vocabulary.**

- **Gateway** — `mm_llm_agentstream::AgentstreamLlm`, the only LLM interface application code
  may construct or call.
- **Journal** — the agentstream append path that records one LLM call's request, outcome, and
  error. Backed by `mm_data::AgentstreamStore`.
- **Provider pool** — the gateway's private dispatch table of provider profiles. It is an
  implementation detail, never exposed to callers.
- **Backend/adapter** — the existing thin OpenAI-compatible transport in `mm-llm`. It remains,
  but only the gateway constructs it.
- **Caller** — `mm-cme`, `mm-agent`, `mm-serve`, `mm-cli`, `mm-process`, `mm-decision`,
  `mm-metaanalysis`, `mm-metacog`, or a test.
- **Agent call** — a downstream Hermes turn. It is still an LLM call and is journaled, but it
  uses the agent profile rather than the chat routing/cache policy.

---

## 1. Executive summary

Today there are three independent ways to reach a model:

1. `mm-serve` constructs `OpenAiClient` through `llm_feed::openai_client`;
2. `mm-cli decision` constructs `OpenAiClient` directly;
3. `mm-agent::HermesBackend` has its own blocking HTTP/1.1 client, SSE parser, JSON parser,
   `/models` probe, and URL splitter.

That means agentstream is not the interface to the model. It is a memory store that sometimes
receives a copy of what happened after a turn, and only for the paths that remember to write it.
A failing intro call can leave no trace at all, which is exactly the blind spot that made the
last empty-completion incident hard to diagnose.

This plan makes one object the door:

```
every caller ──► AgentstreamLlm ──► LlmService ──► provider pool ──► HTTP adapter
                       │
                       ├─ writes llm_request before the provider is touched
                       ├─ writes llm_response / llm_error after it returns
                       └─ publishes live request/delta/response/error for the activity dock
```

The gateway lives in a new crate, `mm-llm-agentstream`, because `mm-llm` must not depend on
`mm-data` and `mm-data` must not own model transport. The gateway depends on both and is the
only crate allowed to construct the service or its adapters.

After the migration:

- `OpenAiClient`, `LlmService`, and the provider transport are internal implementation details.
- Hermes has no HTTP code of its own; it builds a provider-neutral request and maps the neutral
  response back to `AgentTurnResult`.
- Every call has an agentstream `llm_request` event before it starts and exactly one terminal
  `llm_response` or `llm_error` event when it ends.
- The existing `llm_calls` ledger, audit records, cache, routing, schema validation, and repair
  behavior are unchanged. Agentstream is the interface and the call trace; it is not a second
  accounting system.
- `wireVersion: 1` and the existing chat/status/activity wire contracts are unchanged.
- Pi is untouched.

---

## 2. Baseline: every current provider path

### 2.1 Live provider construction

| Path | Where | Today | Problem |
|---|---|---|---|
| Server controller, chat, CME, process, automation, summarizer | `crates/mm-serve/src/llm_feed.rs` (`openai_client`, `observed_openai`) | Builds `OpenAiClient`, optionally wraps with `ObservedClient` | Construction is centralized but still an adapter, not agentstream |
| Hosted decision core | `crates/mm-cli/src/decision.rs` | Calls `mm_llm::OpenAiClient::new` directly | Bypasses the server, the feed, and agentstream |
| Hermes agent turns | `crates/mm-agent/src/hermes.rs` | Private `TcpStream` HTTP client, own SSE/JSON parsing, own `/models` probe | Third implementation of the same provider protocol and no journal |
| LLM CLI commands | `crates/mm-cli/src/llm_cmd.rs` | Builds `LlmService` directly and injects replay/mock clients | Direct service construction; replay/test calls bypass the gateway |
| Controller intro | `crates/mm-serve/src/controller/turn.rs:382` | Builds `LlmService` around the resolved client and registers `controller.intro` | A second service-construction seam |
| Process engine | `crates/mm-serve/src/state.rs:508` | Builds client, then wraps in `LlmService` | Another construction seam |

`crates/mm-serve/src/state.rs:1181` already contains a source test named
`only_the_activity_feed_constructs_a_provider_client`. It proves only one construction exists
inside `mm-serve`; it does not cover `mm-cli` or Hermes. It will be replaced by a workspace-wide
guard after the migration.

### 2.2 Existing client layers

| Layer | Responsibility | Stays? |
|---|---|---|
| `LlmClient` trait | Provider-neutral request/response contract | Yes, re-exported by the gateway |
| `OpenAiClient` | OpenAI-compatible JSON/SSE transport | Yes, private to the gateway |
| `TcpTransport` | HTTP/1.1 over TCP | Yes, private to `mm-llm` |
| `LlmService` | Routing, cache, schema validation, repair, accounting | Yes, owned by the gateway |
| `ObservedClient` | Activity decorator | Removed after the live feed consumes gateway events |
| `LlmActivityFeed` | Bounded ring + SSE snapshot | Kept, but becomes a subscriber |
| `MockClient` / `ReplayLlmClient` | Test and recorded replay | Kept, injected into the gateway in test/replay mode |

### 2.3 Agentstream today

`AgentstreamStore` already provides:

- `start_session(session_id, metadata)` — idempotent named graph + index entry;
- `log_event(AgentstreamEvent)` — RDF upsert + daily N-Triples append;
- `events(session_id)` — ordered replay;
- `snapshot_session(session_id)` — Turtle snapshot;
- `EventProvenance { source, model }` and `turn_id`.

It has no `EventType` for a model call. `EventType::Custom` is the only option for one today,
which is why calls are not a first-class inspectable trace. This plan adds dedicated event
types and a payload schema; it does not change the store's append-only model.

### 2.4 What must not change

- `LlmClient` call semantics: cache, routing, schema validation, repair, and ledger rows.
- The `llm_calls` table and its `call_id` join key.
- `wireVersion: 1`, chat frames, status SSE, and the activity dock's wire types.
- The audit log's content and ordering.
- Pi RPC semantics and `AgentRunStatus` mapping.
- Provider endpoint/key resolution from environment; no URL or secret is committed.

---

## 3. Target architecture

### 3.1 One public gate, private internals

```
                 application crates
  ┌──────────────────────────────────────────────────────────────┐
  │ mm-cme  mm-agent  mm-serve  mm-cli  mm-process  mm-decision   │
  └───────────────────────────┬──────────────────────────────────┘
                              │ only this dependency for live calls
                              ▼
                 mm-llm-agentstream::AgentstreamLlm
                 (implements LlmClient; re-exports types)
                              │
              ┌───────────────┼──────────────────┐
              ▼               ▼                  ▼
       CallJournal       LlmService          LiveBus
   AgentstreamStore   routing/cache/      bounded broadcast
   llm_request        validation/repair   request/delta/response
   llm_response       accounting                │
   llm_error                 │                   └─► mm-serve LlmActivityFeed
                             ▼
                      ProviderPool (private)
                      ├─ profile "default"  → OpenAiClient
                      ├─ profile "hermes"   → OpenAiClient
                      └─ scripted/replay    → MockClient/ReplayLlmClient
```

### 3.2 Why a new crate

- `mm-llm` is the provider substrate. Giving it a dependency on `mm-data` would invert the
  layer and force every adapter test to boot RDF.
- `mm-data` is the memory/data layer. Giving it model transport and API-key resolution would
  make storage responsible for network policy.
- `mm-llm-agentstream` depends on `mm-llm` and `mm-data`, owns the composition, and is small
  enough to test without the server.

Proposed dependency graph:

```
mm-llm-agentstream
  ├── mm-llm        (types, service, adapters)
  ├── mm-data       (AgentstreamStore)
  ├── mm-core       (ids, config paths)
  ├── mm-log        (audit on journal failure)
  ├── tokio
  ├── async-trait
  ├── serde / serde_json
  └── ulid
```

`mm-agent`, `mm-serve`, `mm-cli`, `mm-cme`, and `mm-process` add the gateway dependency and
drop direct construction of `OpenAiClient`/`LlmService`.

### 3.3 One logical call

```
caller constructs LlmRequest
        │
        ▼
AgentstreamLlm::complete / complete_stream
        │
        ├─ resolve context (CallContext + request provider)
        ├─ resolve session id (conversation / agent / llm-source)
        ├─ append llm_request  ── failure ──► return LlmError::Journal, no provider call
        ├─ publish live Request
        │
        ├─ LlmService::call / call_stream
        │      ├─ route, cache, repair, validate, account
        │      └─ ProviderPool → OpenAiClient → HTTP
        │
        ├─ append llm_response or llm_error
        ├─ publish live Response/Error
        └─ return the exact LlmResponse/LlmError from the service
```

### 3.4 Invariants

1. **Only the gateway constructs a live client.** No application crate calls
   `OpenAiClient::new`, `LlmService::new`, or a provider transport.
2. **No journal, no call.** The pre-call append is synchronous and required. A journal failure
   returns `LlmError::Journal` before any socket is opened.
3. **One logical call, one request event, one terminal event, plus one attempt event per
   provider round.** Repair attempts remain inside `LlmService`; the gateway's private provider
   journaler records each actual HTTP round as an `llm_attempt` child of the logical
   `call_id`. The ledger's `call_id` is the same id used in the logical events.
4. **The provider pool is not public.** Callers select a profile by name; they cannot hold an
   adapter or endpoint.
5. **Observation cannot change the answer.** Live publication is best-effort and cannot fail a
   call. Durable journaling is the mandatory part.
6. **No token stream in RDF.** Deltas are live-only. The terminal event carries bounded text,
   a hash, and the final tool-call count.
7. **The existing dock remains a projection.** It does not become a second reply source.

### 3.5 Degraded modes

The gateway must make unavailability explicit rather than falling back to a direct client:

| Missing prerequisite | Behavior |
|---|---|
| `AgentstreamStore` unavailable | Gateway construction fails; live model paths report unavailable |
| Kernel graph/accounting handle unavailable | Gateway construction fails; no unaccounted provider call is made |
| Provider profile endpoint/key unset | The call is journaled as a request and then fails with `provider_not_configured`; no socket is opened |
| `MM_LLM_AGENTSTREAM=off` | Live calls return a configuration error; there is no direct-client fallback |
| Scripted/replay client supplied | The gateway still journals and publishes; only the inner client changes |

This is a deliberate availability trade: a working provider endpoint is not enough if the call
cannot be traced and accounted. The health line must report which prerequisite is missing.

---

## 4. Public API contract

### 4.1 New crate surface

```rust
// crates/mm-llm-agentstream/src/lib.rs

pub use mm_llm::{
    CacheMode, DecoderKind, LlmClient, LlmError, LlmRequest, LlmResponse, Message,
    ProviderToolCall, Purpose, Result, Role, SchemaId, StructuredOut, ToolSchema, Usage,
};

pub struct AgentstreamLlm { /* service, journal, pool, live bus, profiles */ }

impl AgentstreamLlm {
    pub fn builder(parts: GatewayParts) -> GatewayBuilder;
    pub fn provider_profiles(&self) -> &[ProviderProfile];

    /// Register a typed or ad-hoc schema with the service.
    pub fn register_schema(&self, id: SchemaId, schema: serde_json::Value) -> Result<()>;
    pub fn register<T: StructuredOut>(&self) -> Result<()>;

    /// Probe a profile through the same transport and journal the result.
    pub async fn models(&self, profile: &str) -> Result<Vec<String>>;

    /// Live call events for the console dock; no-op when no subscribers.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<LiveLlmEvent>;
}

#[async_trait]
impl LlmClient for AgentstreamLlm { /* complete, complete_stream, provider, decoder */ }
```

`GatewayBuilder` inputs:

- `LlmConfig` loaded from `config/llm.toml`;
- `SqliteStore`, `GraphHandle`, `Arc<Logger>` for `LlmService` accounting;
- `Arc<AgentstreamStore>` for the journal;
- `ProviderProfiles` resolved from config/environment;
- optional injected `Arc<dyn LlmClient>` for scripted/replay tests;
- `JournalMode::{Required, BestEffort}` (production uses `Required`; tests use `Required`
  unless a test is specifically proving journal-failure behavior);
- default `user_id` for system-created sessions.

The builder rejects a profile whose endpoint or key is missing only when that profile is
selected, not at construction time. This preserves the current “unconfigured is reported,
never defaulted” behavior and lets tests construct the gateway with a scripted client.

### 4.2 Additive `LlmRequest` fields

These are additive and serde-defaulted so recorded replay sessions remain readable:

```rust
pub struct LlmRequest {
    // existing fields...
    /// Provider profile name. None means "default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// Per-call wall-clock ceiling. None uses the profile's timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,

    /// Stable id assigned by the gateway before the call starts.
    /// The service uses it for the ledger and the terminal event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<ulid::Ulid>,
}
```

Add matching builders: `with_provider(name)`, `with_timeout(ms)`, and `with_call_id(id)`. The
gateway assigns `call_id` when the caller does not.

`LlmClient` gains an additive `provider_for(&self, req: &LlmRequest) -> &str` defaulting to
`provider()`. The private provider pool returns the selected profile name, and `LlmService`
records that name in `CallAccount.provider` and the audit row. Without this, every call would
be accounted as `"gateway"` instead of `"default"`/`"hermes"`.

The provider profile carries transport-only policy rather than adding more request fields:

```rust
pub struct ProviderProfile {
    pub name: String,
    pub kind: ProviderKind,             // OpenAiCompatible for now
    pub endpoint_env: String,
    pub api_key_env: String,
    pub model_env: Option<String>,
    pub model: Option<String>,
    pub timeout_ms: u64,
    pub stream: bool,
    pub temperature: Option<f32>,
    pub omit_max_tokens: bool,
    pub structured_output: StructuredOutput,
}
```

`OpenAiClient::request_body` already builds the body; it gains a per-profile
`omit_max_tokens` and a per-call timeout override. No caller sees the profile struct.

### 4.3 Additive `LlmResponse` fields

Hermes needs finish-reason/status information the current neutral response drops. Add:

```rust
pub struct LlmResponse {
    // existing fields...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,

    /// Non-content provider metadata needed to preserve downstream status.
    /// Never contains prompts, tool arguments, keys, or headers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_metadata: BTreeMap<String, serde_json::Value>,

    /// True when the provider used SSE.
    #[serde(default)]
    pub streamed: bool,
}
```

`provider_metadata` carries `mm_status` for Hermes. The OpenAI adapter already parses the
response `Value`; it can capture `mm_status` and `finish_reason` without a second parser.

### 4.4 Provider profile configuration

Add profiles to `config/llm.toml`. Endpoint URLs and keys remain environment-only:

```toml
[profiles.default]
kind = "openai_compatible"
endpoint_env = "OPENAI_BASE_URL"
api_key_env = "OPENAI_API_KEY"
timeout_ms = 30000
structured_output = "json_object"

[profiles.hermes]
kind = "openai_compatible"
endpoint_env = "MM_AGENT_HERMES_BASE_URL"
api_key_env = "MM_AGENT_HERMES_API_KEY"
model_env = "MM_AGENT_HERMES_MODEL"
timeout_ms = 120000
stream = true
temperature = 0.2
omit_max_tokens = true
```

Compatibility rules:

- `default` falls back to the existing `OPENAI_BASE_URL` / `OPENAI_API_KEY` behavior.
- `hermes` reads the environment first, then the existing `[hermes] endpoint` in
  `config/agents.toml`, so an existing deployment does not need a config rewrite on day one.
- A profile with no configured model may use the first `/models` result; the resolved model is
  recorded in the call event.
- No profile may embed an endpoint URL or key in committed configuration.

### 4.5 `Purpose::Agent`

Add `Purpose::Agent` with stable string `"agent"`. It is:

- never served by the semantic cache;
- routed only when the request pins a model (Hermes always does);
- accounted like any other purpose;
- excluded from the default “cheapest sufficient” choice.

This keeps agent calls distinguishable in `llm_calls` without pretending they are conversation.
The agent profile contributes its pinned/discovered model as a `ModelSpec` so the router does
not reject a model the endpoint advertises but `config/llm.toml` does not list. If discovery
changes the model after boot, `LlmService` gains `register_model` to update the router before
the next call.

### 4.6 Schema registration

`register_schema` and `register::<T>` are gateway methods. The service remains the authoritative
registry and still forwards schema definitions to native structured-output adapters. The
provider pool forwards `provide_schema` to every profile adapter that supports it.

`mm-serve` no longer builds a `LlmService` to register `controller.intro`. At boot it obtains
the gateway and calls:

```rust
gateway.register_schema(SchemaId::new("controller.intro"), intro::intro_schema())?;
```

CLI schema-reject tests use the same method on a gateway backed by a mock client.

---

## 5. Agentstream event contract

### 5.1 New event types

Add to `mm_data::agentstream::EventType`:

| Variant | Wire name | Meaning |
|---|---|---|
| `LlmRequest` | `llm_request` | A logical LLM call is about to be sent |
| `LlmAttempt` | `llm_attempt` | One actual provider round inside the logical call |
| `LlmResponse` | `llm_response` | The call returned a response |
| `LlmError` | `llm_error` | The call failed before producing a response |
| `LlmModels` | `llm_models` | A provider `/models` probe completed |

These are additive. `EventType::as_str` gains the names; old traces deserialize unchanged.
`EventType` is not exhaustive in projections; `CleanLog` ignores them.

### 5.2 Payload schema

`mm.llm.call.v1`:

```json
{
  "schema": "mm.llm.call.v1",
  "phase": "request",
  "callId": "01...",
  "traceId": "01...",
  "provider": "default",
  "purpose": "converse",
  "model": "nvidia/...",
  "source": "chat",
  "sessionId": "01...",
  "runId": "01...",
  "pass": "process",
  "round": 2,
  "streamed": true,
  "cached": false,
  "messageCount": 8,
  "toolCount": 17,
  "schemaId": "controller.intro",
  "promptHash": "sha256...",
  "timeoutMs": 30000
}
```

Provider attempt (one per cache miss / repair round):

```json
{
  "schema": "mm.llm.call.v1",
  "phase": "attempt",
  "callId": "01...",
  "attemptNo": 1,
  "provider": "default",
  "model": "nvidia/...",
  "status": "ok | schema_rejected | transport | timeout",
  "latencyMs": 1200,
  "tokensIn": 1200,
  "tokensOut": 300
}
```

The attempt event has no prompt, text, or tool arguments; it links the actual provider round to
the logical call. Cache hits produce no attempt event.

Mechanically, the gateway wraps the private provider pool in an internal `AttemptJournal`. For
the duration of `LlmService::call`, the gateway establishes a task-local scope carrying the
logical `call_id` and an atomic attempt counter. Every inner `complete`/`complete_stream` on the
provider pool writes `llm_attempt` before forwarding and records its outcome after. Repair
retries therefore become attempts 1 and 2 of the same logical call, not new logical calls.

Terminal response:

```json
{
  "schema": "mm.llm.call.v1",
  "phase": "response",
  "callId": "01...",
  "status": "ok | cached | schema_rejected",
  "model": "nvidia/...",
  "tokensIn": 1200,
  "tokensOut": 300,
  "costMicros": 300,
  "latencyMs": 1200,
  "schemaValid": true,
  "cached": false,
  "streamed": true,
  "attemptCount": 1,
  "toolCallCount": 2,
  "finishReason": "stop",
  "text": "bounded reply text",
  "truncated": false,
  "textHash": "sha256..."
}
```

Terminal error:

```json
{
  "schema": "mm.llm.call.v1",
  "phase": "error",
  "callId": "01...",
  "kind": "transport | timeout | schema_rejected | provider_not_configured | journal",
  "message": "redacted, bounded error",
  "latencyMs": 20000,
  "partialChars": 0
}
```

Model probe:

```json
{
  "schema": "mm.llm.call.v1",
  "phase": "models",
  "provider": "hermes",
  "available": true,
  "models": ["..."],
  "reason": null
}
```

### 5.3 What is deliberately not stored

- The full system prompt and full message list. The request event stores counts, a prompt
  hash, the schema id, and the bounded redacted user excerpt only.
- Tool arguments and tool result bodies. Only names/counts.
- API keys, authorization headers, endpoint URLs.
- Token-level deltas. Those are live-only.

This matches the existing audit/activity privacy boundary and avoids turning every call into a
second copy of the conversation in RDF.

### 5.4 Text bound

`MM_LLM_JOURNAL_TEXT_CHARS` caps the terminal reply text, default `16000` characters (the same
order as the clean-log budget). Over the cap, store a prefix, `truncated: true`, and the full
text's hash. The response returned to the caller is never truncated.

### 5.5 Retention

Agentstream is append-only. LLM events are written to the same daily `events-YYYY-MM-DD.nt`
files and session graphs as every other event. No deletion, compaction, or retention change is
introduced by this plan. Operators already archive those files; the plan adds a runbook note,
not a new retention subsystem.

The `llm_calls` ledger remains the accounting source. Agentstream events are the call trace and
join to the ledger by `callId`.

---

## 6. Session and turn correlation

The gateway resolves one agentstream session and one turn id per call.

### 6.1 Session resolution

| Context | Session id | Rationale |
|---|---|---|
| `CallContext.session_id` is set | use it | chat, CME, automation already have a conversation session |
| `source == "agent"` and `run_id` is set | `agent/<run_id>` | Hermes already owns this isolated session |
| no session id | `llm/<sanitized source>` | summarizer, process, startup, hosted decision, CLI |

`sanitize` lowercases and keeps `[a-z0-9/_-]`; other characters become `-`. Empty source
becomes `llm/other`. The gateway ensures the session exists with
`SessionMetadata { user_id, project_id: None, title: Some("LLM calls: <source>") }`.

`start_session` is idempotent, so concurrent first calls cannot create two index entries. The
gateway keeps a small started-session cache only to avoid duplicate writes, not as a source of
truth.

### 6.2 Turn id

- If `run_id` is present, use it.
- Else if `pass` is present, use `pass` or `pass:round`.
- Else use the `call_id` string.

This keeps events orderable within a turn without pretending every call has a conversation turn.

### 6.3 Call id

The gateway allocates one ULID before the request event and puts it in `LlmRequest.call_id`.
`LlmService` uses that id instead of allocating a new one. Therefore:

- `llm_request.callId` = `llm_calls.call_id` = `llm_response.callId`;
- the activity dock's existing `call_id` remains stable;
- a request with no terminal event is immediately visible as an interrupted call.

---

## 7. Live activity feed

The existing `ObservedClient` decorator is removed. It was correct for observation, but after
the gateway owns every call it is a second wrapper around the same seam and it would duplicate
request/response publication.

### 7.1 Gateway live events

`AgentstreamLlm` owns a bounded `tokio::sync::broadcast` of:

```rust
pub enum LiveLlmEvent {
    Request { call_id, context, provider, streamed, request },
    Delta { call_id, text, total_chars, truncated },
    Response { call_id, latency_ms, response },
    Error { call_id, latency_ms, partial_chars, error },
}
```

Deltas are coalesced with the existing `DeltaPolicy` constants (256 chars / 50 ms / 2,000-char
call cap), moved from `mm-llm::activity` into the gateway crate. A slow subscriber is dropped;
it never delays the model call.

### 7.2 `mm-serve` feed

`LlmActivityFeed` changes from “wrapped by `AppState::observed`” to “subscribed to the gateway”:

- `AppState` subscribes once at boot;
- the feed maps `LiveLlmEvent` into the existing `LlmActivityEvent` DTO;
- `GET /api/llm-activity` and `GET /api/llm-activity/stream` are unchanged;
- `seq`, ring size, pause/resume, gap recovery, and frontend types are unchanged;
- `MM_LLM_ACTIVITY` remains the kill switch for the live feed only; the durable journal is
  controlled separately by `MM_LLM_AGENTSTREAM`.

The dock remains a display projection. The chat reply text still comes from the chat final
frame, never from the feed.

### 7.3 Tests and scripted clients

Scripted clients are injected into the gateway, so Playwright and unit tests still produce
activity. They also produce journal events unless the test explicitly uses an in-memory
agentstream. This is important: “test mode” must not be a hole in the interface.

---

## 8. Hermes migration

### 8.1 What Hermes loses

Delete from `crates/mm-agent/src/hermes.rs`:

- `http_request`, `read_chunked`, `split_url`, `HttpResponse`;
- `post_completion`, `parse_completion_json`, `parse_sse`, `parse_usage`, `parse_tool_calls`
  (the neutral response now supplies the same data);
- the private blocking transport and its unit tests.

The Hermes endpoint remains `http://`-only because the gateway's built-in transport still
refuses TLS. That limitation is unchanged and now reported by the gateway.

### 8.2 What Hermes keeps

- `HermesConfig` (endpoint/file/env resolution, model hint, stream choice, timeout,
  max-context tokens);
- `HermesProbe` and health reporting;
- the `AgentBackend` implementation;
- status inference (`mm_status`, finish reason, tool calls, question detection);
- the isolated `agent/<run_id>` agentstream session and existing `agent_turn` events.

### 8.3 Constructor

```rust
pub fn with_gateway(config: HermesConfig, gateway: Arc<AgentstreamLlm>) -> Self
```

`AgentRuntime::from_env` resolves Hermes config, builds the gateway (already built by
`mm-serve`/tests), probes through `gateway.models("hermes").await`, and stores both.

If no gateway is supplied, Hermes is `Unavailable` with the exact reason. There is no fallback
direct client.

### 8.4 Turn translation

`run_turn` becomes:

1. `build_messages(request)` produces the same system/context/messages ordering.
2. Build `LlmRequest`:
   ```rust
   LlmRequest::new(Purpose::Agent, messages)
       .with_model(model)
       .with_provider("hermes")
       .with_cache(CacheMode::Bypass)
       .with_timeout(request.timeout_ms.max(1).min(profile.timeout_ms))
   ```
   Set temperature `0.2` via the profile policy. The profile sets `omit_max_tokens = true`,
   preserving Hermes's current body shape.
3. If streaming is configured, call `complete_stream` with a no-op sink and record
   `streamed = true`; otherwise call `complete`.
4. Map `LlmResponse` back:
   - `text` → message/summary;
   - `tool_calls` → `AgentToolSummary { name, arguments truncated to 500 chars }`;
   - `finish_reason` + `provider_metadata["mm_status"]` → `status_from`;
   - `usage` → `AgentUsage`;
   - `streamed` → `AgentTurnResult.streamed`.
5. The agentstream session is the request's existing `agent/<run_id>` session; the gateway
   journals into that same trace. The existing `agent_turn` event still carries the raw turn.

### 8.5 `/models`

`HermesBackend::refresh` calls `gateway.models("hermes")`. The gateway:

- resolves the profile;
- performs `GET /models` through the same transport;
- appends an `llm_models` event with available/reason/models;
- returns the list.

If the profile is unconfigured, it returns a redacted unavailable reason without a socket.
This preserves current health behavior while removing the separate HTTP code path.

### 8.6 Timeouts

The current Hermes turn uses `tokio::time::timeout` around a blocking request. The gateway
keeps a per-call `timeout_ms`; the adapter transport applies it to connect/write/read. The
agent layer may keep an outer timeout as a defense, but it must not be the only timeout.

### 8.7 Hermes tests

`crates/mm-serve/tests/phase6.rs` already runs a fake OpenAI-compatible server. It stays, but
its assertions change:

- the request body has `model`, `messages`, `stream`, `temperature`, no `max_tokens`;
- the gateway journals `llm_request` + one `llm_attempt` per provider round + `llm_response`
  into `agent/<run_id>`;
- tool calls, usage, `mm_status`, and `finish_reason` map to the same `AgentTurnResult`;
- `/models` probe produces `llm_models` and still drives availability;
- a non-SSE JSON response falls back exactly as before.

---

## 9. Pi is not migrated

`mm-pi` is an external process and RPC protocol. It is not an OpenAI-compatible client, has no
provider endpoint/key in Metamind, and is governed by the self-engineering sandbox rules.

Explicitly unchanged:

- `crates/mm-pi/*`;
- `crates/mm-agent/src/pi.rs`;
- `AgentRuntime`'s Pi backend, timeout, model, and `AgentTurnResult` mapping;
- `mm-cli pi`, session ingestion, and recorded replay.

The gateway may be present in the same process, but Pi never calls it. No Pi source may be
added to the provider pool or journal as an LLM call unless a later design explicitly changes
the Pi contract.

---

## 10. Deletions and bypass prevention

### 10.1 Deleted or retired

| Item | Action |
|---|---|
| `mm-serve/src/llm_feed.rs::openai_client` / `observed_openai` | Delete; construction moves to the gateway |
| `mm-serve/src/state.rs::AppState::observed` | Delete; the gateway publishes directly |
| `mm-llm/src/activity.rs::ObservedClient` | Delete after the feed subscribes to gateway events |
| `ControllerModel::FromEnv` construction | Replaced by gateway handle |
| `controller/turn.rs::service` | Delete; gateway owns service/schema registration |
| `mm-cli/decision.rs` direct `OpenAiClient::new` | Replace with gateway |
| `mm-cli/llm_cmd.rs` direct `LlmService::new` | Replace with gateway builder (replay/mock injection) |
| `mm-agent/hermes.rs` HTTP helpers | Delete |
| `mm-serve` source test `only_the_activity_feed_constructs_a_provider_client` | Replace with workspace-wide guard |
| `mm-llm` activity tests for `ObservedClient` | Move equivalent coverage to gateway live bus |

`LlmActivityFeed`, `LlmActivityEvent`, the DTO, routes, frontend store, and dock remain.

### 10.2 Workspace-wide source guard

Add an integration test (in `mm-llm-agentstream/tests/no_direct_clients.rs`) that walks
`crates/` and asserts:

- `OpenAiClient::new` and `OpenAiClient::from_env` appear only under
  `crates/mm-llm-agentstream/` and `crates/mm-llm/src/openai.rs` tests;
- `LlmService::new` appears only under `crates/mm-llm-agentstream/` and `mm-llm` tests;
- `TcpStream` and `/chat/completions` do not appear under `crates/mm-agent/`;
- no source outside the gateway constructs `ProviderEnv` for a live call.

Comment/test code is exempt by construction (the guard reads source and checks paths, not
semantics); the goal is to make a new direct client an intentional, reviewable change rather
than an easy accident.

### 10.3 Documentation

Update:

- `infodocs/llm-integration.md` — the architecture diagram and “only door” statement;
- `infodocs/operator-guide.md` — Hermes now uses the gateway; journal failure behavior;
- `README.md` — environment/provider profiles and the new `MM_LLM_AGENTSTREAM` flag;
- `design/DESIGN_INDEX.md` — this plan.

Do not rewrite historical plans; add a short “superseded for provider access” note where
`llm_activity_log_plan.md` describes `ObservedClient` as the tap.

---

## 11. Build order

### P0 — Contract and guard skeleton

Deliverables:

- add workspace member `crates/mm-llm-agentstream`;
- define `GatewayParts`, `GatewayBuilder`, `AgentstreamLlm`, `ProviderProfile`,
  `LiveLlmEvent`, `JournalMode`;
- add additive `LlmRequest`/`LlmResponse` fields, `Purpose::Agent`,
  `LlmError::Journal`;
- add `EventType::{LlmRequest, LlmResponse, LlmError, LlmModels}` and payload constructors;
- add the workspace source guard (it can fail until P1–P4 complete, so gate it behind an
  explicit ignored test until the cutover).

Exit criteria:

- `cargo check --workspace`;
- serialization round-trips for old and new `LlmRequest`/`LlmResponse`;
- `EventType` wire names and old trace deserialization pass;
- no behavior change in existing tests.

### P1 — Gateway core

Deliverables:

- `ProviderPool` with `"default"` and `"hermes"` profiles;
- journal resolution/session bootstrap;
- pre-call append, provider-attempt append, terminal append, error append, live publish;
- `LlmService` ownership and schema registration;
- per-call timeout and `omit_max_tokens`;
- scripted/replay client injection;
- fake journal + counting provider tests.

Exit criteria:

- one successful call writes request + attempt + response in the correct session;
- one failing call writes request + attempt + error;
- a repair round writes one request, two attempt events, and one terminal response;
- journal failure performs zero provider calls;
- cache hit writes request + response with `cached: true`;
- streamed call writes one request and one response, not token events;
- `llm_calls.call_id` equals both event call ids.

### P2 — Server cutover

Deliverables:

- `AppState` builds one `Arc<AgentstreamLlm>` at boot;
- `controller/intro`, controller turns, CME, process, automation, summarizer use it;
- remove `AppState::observed`, `llm_feed::openai_client`, `observed_openai`,
  `controller/turn::service`;
- `LlmActivityFeed` subscribes to gateway live events;
- scripted controller mode injects `MockClient` into the gateway.

Exit criteria:

- existing `cargo test -p mm-serve --lib` and controller/CME tests pass;
- `cargo test -p mm-serve --test phase6` and `phase7` pass;
- activity dock e2e still passes with unchanged wire types;
- a server boot with no provider still starts, and hosted paths report unavailable;
- no `OpenAiClient` construction remains under `crates/mm-serve/src`.

### P3 — Hermes cutover

Deliverables:

- `HermesBackend::with_gateway`, probe via `gateway.models("hermes")`;
- delete Hermes HTTP/SSE/JSON transport;
- translate `AgentRequest` ↔ `LlmRequest`/`LlmResponse`;
- journal into `agent/<run_id>`;
- update phase6 fake-server tests.

Exit criteria:

- Hermes streaming and non-streaming both work against the fake server;
- tool calls, `mm_status`, finish reason, usage, and question detection map identically;
- `/models` unavailable reasons are unchanged in shape;
- no `TcpStream`/`/chat/completions` under `crates/mm-agent`;
- Pi tests are untouched and pass.

### P4 — CLI and remaining callers

Deliverables:

- `mm-cli decision` builds a gateway over the data layer instead of `OpenAiClient`;
- `mm-cli llm replay` and `llm schema-reject` build gateways with replay/mock clients;
- any direct `LlmService` construction outside the gateway is removed;
- CLI commands that make live calls open the agentstream data layer (or use an explicit
  in-memory journal for tests).

Exit criteria:

- hosted decision tests pass with journal events;
- replay still proves zero provider calls and byte-identical replays;
- schema-reject still rejects every malformed fixture for the right reason;
- `mm-cli llm stats` is unchanged.

### P5 — Retire the old observer

Deliverables:

- delete `ObservedClient`, `ActivitySink`, and the old `observe` wrapper;
- keep `CallContext`/`scope` (move to a neutral `mm-llm::call` module if that reads better);
- move delta coalescing constants/policy into the gateway;
- update `llm_activity` tests to publish through the gateway test harness.

Exit criteria:

- dock parity: request/delta/response/error shapes, filters, clear, pause, resize, reconnect;
- no double events (one request from gateway, none from a surviving decorator);
- `cargo test -p mm-llm --lib` passes with the observer tests removed or migrated.

### P6 — Enforcement, docs, and cleanup

Deliverables:

- turn the source guard into a required test;
- add `MM_LLM_AGENTSTREAM` kill-switch behavior (default on; off means a single explicit error,
  never a direct client fallback);
- update docs and `DESIGN_INDEX.md`;
- remove dead imports/dependencies from application crates;
- run the full verification matrix.

Exit criteria:

- every phase's tests pass;
- the guard finds exactly one provider construction site;
- no direct Hermes HTTP remains;
- Pi has no new dependency on the gateway;
- `cargo fmt --all -- --check`, `cargo check --workspace`, and the full relevant test suite pass.

---

## 12. Test matrix

| Area | Test | Asserts |
|---|---|---|
| Journal order | gateway with recording journal + counting client | request appended before client call; attempt before each provider round; terminal after |
| Repair attempts | schema-repair fake client | one request, two `llm_attempt` events, one response |
| Fail closed | journal append fails | client call count is zero; `LlmError::Journal` |
| Session mapping | context with session / agent run / no session | correct session id and metadata |
| Call identity | successful call | `request.callId == response.callId == llm_calls.call_id` |
| Cache | repeated exact request | one request + one response, `cached: true`, no provider call on second |
| Schema rejection | invalid payload | request + error, `schema_rejected`, repair count preserved |
| Streaming | streaming fake client | one request, live deltas, one response; no delta events in RDF |
| Provider dispatch | default vs hermes profile | correct endpoint/key/model; unknown profile is a config error |
| Timeout | slow fake transport | `LlmError::Timeout`, error event, per-call budget respected |
| Redaction | error with key/prompt text | no key or raw prompt in the event |
| Text bound | oversized reply | stored text truncated, hash of full text, caller sees full text |
| Concurrent calls | two sessions in parallel | no session/index corruption; both terminal events |
| Hermes JSON | fake non-SSE completion | text/tool/usage/status mapping preserved |
| Hermes SSE | fake SSE completion | same mapping; `streamed: true` |
| Hermes models | fake `/models` | probe event; availability/reason preserved |
| CLI decision | hosted core | call journaled; no direct adapter construction |
| CLI replay | recorded session | zero provider calls, zero network attempts, journaled |
| CLI schema-reject | malformed fixtures | rejection kinds unchanged |
| Source guard | workspace scan | no direct client outside gateway |
| Frontend activity | existing Playwright `activity` project | dock snapshot/SSE/merge/resize unchanged |
| Pi | existing phase7 Pi tests | unchanged; no gateway call required |

Minimum commands after each phase:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p mm-llm --lib
cargo test -p mm-llm-agentstream
cargo test -p mm-agent
cargo test -p mm-serve --lib
cargo test -p mm-serve --test phase6
cargo test -p mm-serve --test phase7
cargo test -p mm-cli
```

After P2/P5, also run the existing frontend activity e2e:

```bash
cd frontend && npm run test:e2e -- --project=activity
```

---

## 13. Acceptance

The migration is complete when all of these are true:

1. `crates/mm-llm-agentstream` is the only crate that constructs live provider clients or
   `LlmService`.
2. Every live LLM invocation in `mm-serve`, `mm-cme`, `mm-agent`, `mm-cli`, `mm-process`,
   `mm-decision`, `mm-metaanalysis`, and `mm-metacog` goes through `AgentstreamLlm`.
3. Hermes has no HTTP, SSE, JSON, or `/models` code of its own; it maps neutral requests and
   responses and journals into `agent/<run_id>`.
4. Pi remains on `mm-pi`/`PiBackend` and has no new gateway dependency.
5. A failed agentstream append prevents the provider call and returns `LlmError::Journal`.
6. A request event exists before the provider call; an attempt event exists for every actual
   provider round; a terminal response or error exists for every completed/failed call.
7. `llm_calls` accounting, cache, routing, schema validation, repair, audit ordering, chat
   frames, `wireVersion: 1`, and the activity dock are unchanged.
8. The source guard and the full verification matrix pass.
9. No key, endpoint URL, full prompt, or tool argument is stored in the LLM events.

---

## 14. Risks and trade-offs

| Risk | Impact | Mitigation |
|---|---|---|
| Per-call RDF append adds latency | Slower first token | Measure the append; write request + attempt + terminal only, never deltas |
| Agentstream grows without bound | Disk growth | Existing archival policy + bounded text; add a runbook note, no new retention engine |
| Journal failure blocks all model calls | Availability | Intentional fail-closed requirement; health line names the journal failure; kill switch is explicit, not a silent direct path |
| Terminal append fails after a successful call | Missing terminal trace | Return the valid response, emit an audit `llm.journal.commit_failed`, and surface it in health/metrics; never pretend the call did not happen |
| Concurrency on session creation | Duplicate index entries | `start_session` is idempotent; start cache avoids the common duplicate; test parallel first calls |
| Provider pool routing/accounting for Hermes | Wrong model/cost | Pin the agent model, add `Purpose::Agent`, use a synthetic zero-cost spec when the endpoint does not advertise one |
| Hermes status semantics regress | Wrong run status | Explicit mapping tests for `mm_status`, `finish_reason`, tool calls, and question detection |
| CLI hosted calls need RDF boot | Startup cost | Initialize the data layer only for commands that can make live calls; replay/schema-reject can use an in-memory journal |
| Removing `ObservedClient` loses detail | Dock regression | Feed parity e2e; gateway live bus carries the same request/delta/response/error fields |
| Source guard is lexical | False positives in comments/tests | Scope the scan to non-test source paths, allow the adapter/gateway, and document the rule |

### 14.1 Why not put the gateway in `mm-data`

It would make the data layer own API keys, network policy, and provider parsing. The store
should remain a journal and memory layer. The new crate is the composition boundary.

### 14.2 Why not put it in `mm-llm`

It would force `mm-llm` to depend on `mm-data` or introduce a journal trait with only one
implementation. The adapter layer stays provider-only; the gateway composes adapter + store.

### 14.3 Why not keep Hermes separate

Hermes is the second-largest source of model traffic and the only path with its own protocol
implementation. Leaving it separate would preserve the exact duplication this migration is
meant to remove, and its calls would remain invisible to agentstream.

---

## 15. Open decisions

1. **Full request retention.** Recommendation: no. Store counts/hash/preview. If replay of
   arbitrary calls is later required, add an explicit opt-in, not a default.
2. **Delta durability.** Recommendation: no. Deltas are UI-only; the terminal text is the
   durable record.
3. **Terminal append failure.** Recommendation: return the provider response and record an
   audit/health failure, because the call already happened. Revisit only if the operator
   prefers strict post-call failure.
4. **CLI journal scope.** Recommendation: live CLI calls require a real journal; replay and
   schema-reject tests may use an in-memory store but must still go through the gateway.
5. **Retention/compaction.** Recommendation: out of scope. Agentstream remains append-only;
   operators archive daily event files as they do today.
6. **`MM_LLM_AGENTSTREAM=off`.** Recommendation: return a configuration error for live calls;
   never fall back to a direct client. Tests that do not need a journal should use an in-memory
   store, not the kill switch.
7. **Provider profile names.** Recommendation: `default` for `OPENAI_*`, `hermes` for the
   Hermes endpoint. Additional profiles are additive and require no caller change.

---

## 16. References

- `crates/mm-llm/src/client.rs` — `LlmRequest`, `LlmResponse`, `LlmClient`
- `crates/mm-llm/src/service.rs` — routing, cache, validation, repair, accounting
- `crates/mm-llm/src/openai.rs` — the only provider adapter and transport
- `crates/mm-llm/src/call.rs` — neutral `CallContext` and `scope` (the old activity decorator is gone)
- `crates/mm-data/src/agentstream.rs` — store, event types, sessions, projections
- `crates/mm-agent/src/hermes.rs` — Hermes `AgentRequest`/`LlmResponse` mapping through the gateway
- `crates/mm-agent/src/agentstream.rs` — `agent/<run_id>` session and replay
- `crates/mm-serve/src/state.rs` — gateway construction and live-feed subscription
- `crates/mm-serve/src/llm_feed.rs` — activity projection from gateway live events
- `crates/mm-serve/src/controller/turn.rs` — controller scope and gateway call path
- `crates/mm-cli/src/llm_gateway.rs` — CLI gateway construction over a data-layer or in-memory journal
- `crates/mm-cli/src/decision.rs` — hosted decision through the gateway
- `crates/mm-cli/src/llm_cmd.rs` — replay/mock service construction
- `design/llm_activity_log_plan.md` — current activity dock and observer design
- `design/three_pass_turn_sequence.md` — pass/session semantics
- `design/DESIGN_INDEX.md` — plan index
