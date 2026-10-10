# LLM integration — the one door to a model, and everything behind it

**Purpose.** How `metamind` talks to a language model: what a call is, what the substrate does
around every one of them (routing, caching, schema validation, decoding, accounting, provenance,
replay), how a prompt/context is assembled, how structured results are requested and what happens
when they are wrong, and how tools relate to all of it. Read this when you are changing
`crates/mm-llm`, adding a caller, or wiring a new provider.

**Provenance.** Written by reading the code, not the plans. State when written: `HEAD = 0204d73`
("Initial 12 phase build complete"). Line numbers are from that revision and are anchors, not
contracts — grep the symbol name if one has drifted. The subsystem's own tests and fixtures are
listed at the end so a claim here can be falsified directly.

**Scope.** The substrate (`crates/mm-llm`), its in-tree callers, the Pi authoring path where it is a
*different* door, and the operator surface. Provider-side behaviour of a real OpenAI-compatible
endpoint is out of scope: no live provider call has been made in this environment (see
[`open-gaps.md#g6`](./open-gaps.md#g6)).

## 1. The shape of the integration

```text
  caller (a struct that owns Arc<dyn LlmClient>)
        │
        │  LlmRequest { purpose, messages, schema?, cache, trace_id, … }
        ▼
  LlmService ── route ──► ThresholdRouter        (deterministic, cheapest sufficient model)
        │           │
        │           ├─ exact cache  (sha256 over the whole request)  ── hit ──┐
        │           ├─ semantic cache (opt-in per purpose, thresholded) ─ hit ─┤
        │           │                                                         │
        │           ▼                                                         │
        │      LlmClient::complete ──► provider adapter (mock | openai)       │
        │           │                                                         │
        │      validate ── fail ──► bounded repair ── still bad ──► REJECT     │
        │           │  (never coerce)                                         │
        │           ▼                                                         │
        └── one llm_calls row + llm_repair_attempts rows + audited             │
            llm.accounting.commit record + one mm:LlmCall node ◄──────────────┘
```

Two rules shape everything below, and they are stated at the top of `crates/mm-llm/src/lib.rs`:

1. **Constrain at decode when the backend exposes logits; otherwise ask for provider-native JSON
   Schema; otherwise validate and reject — never coerce.** A plausible value that violates its
   schema is worse than no value, because nothing downstream can tell the two apart.
2. **No component may call a provider directly.** Callers depend on `LlmClient` (the trait) or
   `LlmService` (the facade), never on an adapter's JSON. That is what makes a second provider a
   small change rather than a second implementation of the contract.

Rule 2 has a documented, deliberate reading: a caller may hold an `Arc<dyn LlmClient>` (the three
in-tree drivers do — §3.4), because the trait is provider-neutral. What it may not do is reach a
provider's wire format, or construct an adapter's internals.

## 2. What a call is

`crates/mm-llm/src/client.rs` is the whole vocabulary. Everything a provider must be reduced to:

| Type | Meaning |
|---|---|
| `Purpose` | Why the call is made: `interpret`, `plan`, `extract`, `critique`, `summarize`, `code_review`, `classify`, `diagnose`. Not a log label — it selects the model, decides whether the semantic cache may serve, and is the axis `llm stats` aggregates on. |
| `Message` / `Role` | `system` / `user` / `assistant` turns. Content is a string; there is no multimodal path, no tool-call message type, and no conversation state the substrate maintains for you. |
| `SchemaId` | The name of a registered strict schema. `None` means an unstructured call. |
| `DecoderKind` | `llguidance`, `xgrammar`, `provider_native`, `none` — which decoder produced (or was asked to produce) the output. `is_constrained()` is true for the first two only. |
| `CacheMode` | `Use` (default: consult and populate), `Bypass` (ignore the cache, still account), `Refresh` (ignore a hit, overwrite), `Replay` (serve a recorded session only; a miss is an error, never a request). |
| `LlmRequest` | `purpose`, `model: Option<String>` (the router fills it), `messages`, `schema`, `max_tokens` (default 1024), `temperature` (default **0.0**), `cache`, `trace_id`. Builder: `new`, `with_schema`, `with_model`, `with_cache`, `with_trace`. |
| `LlmResponse` | `call_id` (shared with the ledger row and the log record), `text`, the model that actually answered, `Usage`, `cached`, `schema_valid`, `decoder`. |
| `LlmClient` | `complete`, `provider`, `decoder`. An adapter knows one provider and does not cache, route, retry, account or validate. |

`Usage { tokens_in, tokens_out, cost_micros, latency_ms }` is what one call cost. Cost is in
**millionths of a currency unit** so money is never a float.

## 3. The pipeline: `LlmService`

`crates/mm-llm/src/service.rs` is "the one place a caller touches": every guarantee is enforced in
one order, for every call. `LlmService::new` takes the config, the SQLite store, the graph handle,
the logger, and **one** adapter, and picks the decoder backend from the client (what knows whether
its endpoint exposes logits) falling back to `config/llm.toml`'s `defaults.decoder`.

The order, and why each step sits where it does:

| # | Step | Where | Notes |
|---|---|---|---|
| 1 | Route | `ThresholdRouter::choose(&RoutingRequest)` | The request's `model`, if set, is passed through as `pinned_model`. Result is recorded as a `RoutingRecord`. |
| 2 | Refuse an unregistered schema | `service.rs` (`self.schemas.get`) | A named-but-unregistered schema can never be enforced, so the call is refused *before* a provider is asked. |
| 3 | Hash the request | `cached::prompt_hash(provider, model, &req)` | sha256 over provider, model, purpose, schema, `max_tokens`, `temperature` (through its bit pattern), and every message role+content. That hash is the log field, the cache key, and the provenance field. |
| 4 | Exact cache | `FileExactCache::get` | On a hit the index row is re-mirrored (a body whose write landed without its index row heals here), then `finish_cached` accounts the call: **a hit bills nothing** — tokens are copied for information, `cost_micros` and `latency_ms` are 0, `cached = true`, `cache_layer = "exact"`. |
| 5 | Semantic cache | `semantic_lookup` | Only when enabled *and* the purpose is allowlisted (§6). |
| 6 | Provider | `LlmClient::complete` | A provider error (transport, timeout, not configured, budget) is accounted first — status, error kind, latency — then returned. A failed call still leaves a complete row. |
| 7 | Validate → repair → reject | `ensure_valid` | Only for structured calls. See §7. |
| 8 | Store | `store_cached` | For `Use`/`Refresh`: body on disk, `llm_cache_index` row, and a semantic entry when the purpose allows it. |
| 9 | Account | `finish` | One `llm_calls` row, every buffered `llm_repair_attempts` row, the audited `llm.accounting.commit` record, and the `mm:LlmCall` provenance node — all written from the *same* `CallAccount`, so ledger, audit record and graph cannot disagree. |

`call_structured<T: StructuredOut>(req)` is the typed entry point. It looks up `T::schema_id()`,
compiles (and persists) the grammar first, sets `req.schema`, calls `call`, then validates and
`decode`s. A failure to decode after validation is `LlmError::SchemaRejected`, never a default
value.

### 3.1 Routing is deterministic first

`crates/mm-llm/src/routing.rs`. A labelled request — `purpose`, `complexity`, `stakes`,
`required_precision`, `latency_budget_ms`, `cost_budget_micros`, `pinned_model` — is met by the
**cheapest configured model that can take it**. High stakes or a high precision bar raise the
effective ceiling to `High` (`effective_ceiling`), so a consequential call cannot be answered by the
cheapest small model just because the caller called the task "medium".

The decision is recorded with one of five reasons: `pinned`, `cheapest_sufficient`,
`only_candidate`, `budget_fallback`, `default`. An embedding model may *propose* a complexity, but a
threshold decides, so the same request routes the same way every run and a golden table can pin it.
`Router` is the seam a learned router (e.g. Phase 9's `DecisionCore`) can replace without a caller
changing; `LlmService::with_router` is how you inject one.

`mm-cli llm routes --purpose … --complexity … --stakes … --precision …` reports the choice without
making a call.

### 3.2 The cache is a hash of the whole question

`crates/mm-llm/src/cached.rs`. Bodies live on disk (`data/llm_cache/<prompt_hash>.json`, root
resolvable via `resolve_cache_dir`); `llm_cache_index` (`migrations/0003_llm.sql:43`) mirrors them
as a projection that can be rebuilt. `CachedResponse` carries `response_sha`, and `is_intact()` is
checked before anything is served, so a corrupted body is detected rather than answered from.

Anything that changes the question changes the hash: a different provider, model, purpose, schema,
token ceiling, temperature, or message. `mm-cli llm verify-cache --strict` re-hashes every row and
fails on a body no index row covers (equally: an index row whose body is gone).

### 3.3 The semantic cache is opt-in, per purpose

`crates/mm-llm/src/semantic_cache.rs`. A semantic hit is an *inference* — "this prompt is close
enough to that one" — which is fine for a summary and catastrophic for a decision, so:

- a purpose must be named in `defaults.semantic_cache_purposes` before its prompts are even
  considered, and `allowed_for` is the only authority on that; the shipped config allowlists
  `summarize` only. Consequential structured work (`extract`, `plan`, `critique`) stays exact-hash
  only.
- the similarity must clear `defaults.semantic_cache_threshold` (shipped: `0.97`); nothing is served
  on a "closest match" basis.

The embedding is a deterministic hashed bag of tokens (**64 dims**, sha256 of each token into a
dimension with a sign bit, normalized, cosine similarity). It is deliberately *not* a learned
embedding: a cache whose behaviour changes when a model ships is not a cache.
`mm-memory` reuses exactly this function for its vector index (`crates/mm-memory/src/index.rs`), so
there is one embedding in the system, not two.

### 3.4 The service is the facade, but three drivers hold the client

Accurate statement of the current wiring: the substrate's *guarantees* live in `LlmService`, and the
three in-tree model callers each hold `Arc<dyn LlmClient>` instead:

| Caller | Holds | Consequence |
|---|---|---|
| `HostedCore` (`crates/mm-decision/src/hosted.rs`) | `client` + its own `SchemaRegistry` | Validates and decodes itself; **no** ledger row, cache, grammar persistence or provenance node for this path. |
| `LlmScanDriver` (`crates/mm-metacog/src/scan.rs`) | `client` + `Arc<SchemaRegistry>` | Same: `client.complete` → `registry.validate` → `decode` → `ScanResult::validate`. |
| `LlmDiagnoser` (`crates/mm-metaanalysis/src/diagnosis.rs`) | `Arc<dyn LlmClient>` | Parses the answer with `serde_json::from_str` by hand (§7.4). |

So "every call is accounted" is true of `LlmService` calls and **not** true of these three. The CLI
wires `HostedCore` for `decide --core hosted` and `conformance --core hosted`; two of the three
drivers have no production wiring at all (§11). This is the single most important thing to know
before adding a caller: either go through `LlmService`, or accept that you are bypassing the ledger,
cache and provenance.

## 4. Providers, and the two that ship

| Adapter | File | Behaviour |
|---|---|---|
| `MockClient` / `ScriptedResponse` | `crates/mm-llm/src/mock.rs` | Deterministic answers from a script: `constant`, and variants that count calls. The default in every test and in `mm-cli llm schema-reject` (which hands it a malformed payload verbatim, so the substrate is the only thing between a violating payload and a typed value). |
| `OpenAiClient` | `crates/mm-llm/src/openai.rs` | One adapter for everything that speaks `/chat/completions`. Endpoint and key come from `OPENAI_BASE_URL` / `OPENAI_API_KEY` **at construction**; when either is unset the adapter is `LlmError::ProviderNotConfigured` rather than pointed at a default host. The built-in transport speaks plain HTTP only — TLS is expected to be terminated in front of the kernel — and `crates/mm-llm/tests/no_hardcoded_endpoint.rs` scans the sources to prove there is no hard-coded fallback. The adapter does not cache, retry, route, account or validate. |

`decoder()` lets an adapter declare that it makes schema-valid output by construction
(`llguidance`/`xgrammar` token-level, or provider-native JSON Schema). `LlmService::new` prefers the
client's claim and falls back to `defaults.decoder`; the choice changes what happens on a
validation failure (§7.3).

## 5. The config surface

`config/llm.toml` — model specs and policy only. **No secrets, no URLs**, so a committed config can
never pin a provider host.

```toml
[defaults]
decoder = "llguidance"                    # llguidance | xgrammar | provider_native | none
semantic_cache_purposes = ["summarize"]   # opt-in per purpose
semantic_cache_threshold = 0.97
max_repair_attempts = 1                   # bounded repair passes per structured call
timeout_ms = 30000
max_tokens = 1024

[[model]]  # small | frontier | mock-small: name, input/output cost per 1k micros,
           # typical_latency_ms, complexity_ceiling
```

The `mock-small` spec is declared like any other model on purpose: routing and cost accounting
exercise a real spec rather than a special case. `LlmConfig::load_or_default` is what the CLI uses
(`crates/mm-cli/src/llm_cmd.rs:llm_config`), so a missing file is a default, not a crash.

Environment: `OPENAI_BASE_URL`, `OPENAI_API_KEY` (`ProviderEnv`). Cache directory:
`resolve_cache_dir(root, cfg.cache_dir)`.

## 6. Structured results: when the system uses them, and how it guarantees them

### 6.1 A type owns its schema

```rust
pub trait StructuredOut: serde::de::DeserializeOwned + Send + 'static {
    fn schema_id() -> SchemaId;
    fn schema() -> Value;      // the strict JSON Schema
}
```

(`crates/mm-llm/src/schema.rs:644`.) Registering it means the type *is* the schema — the two cannot
drift without a round-trip test failing. `register::<T>()` / `register_structured::<T>()` put it in a
per-run `SchemaRegistry`; `LlmService::register_schema(id, value)` adds an ad-hoc one (that is what
`bench/llm/schemas.json` feeds, via `llm_cmd::register_bench_schemas`).

The shipped implementors are exactly two, plus test types:

| Type | Schema id | Registered by |
|---|---|---|
| `HostedDecisionReply` (`mm-decision/src/hosted.rs:165`) | `HOSTED_DECISION_SCHEMA_ID` | `mm-cli/src/decision.rs:302` when building the hosted core |
| `ScanResult` (`mm-metacog/src/scan.rs:294`) | `metacog.scan.v1` | `LlmScanDriver`'s caller |
| `Rating` | `rating.v1` | tests only |

### 6.2 "Strict" is enforced at registration, not hoped for

`schema.rs` implements a deliberate subset of JSON Schema and **rejects** anything else, because a
schema that lets an extra field through or leaves `required` implicit is a schema whose output the
substrate cannot trust:

- supported keywords are listed in `KEYWORDS` (`type`, `properties`, `required`,
  `additionalProperties`, `items`, `enum`, `const`, numeric/length/count bounds, `description`,
  `title`, `$schema`, `$id`);
- `FORBIDDEN_KEYWORDS` — `oneOf`, `anyOf`, `allOf`, `not`, `$ref`, `$defs`, `patternProperties`,
  `pattern`, `format`, `default`, `uniqueItems`, … — are a hard `UnsupportedSchema` error, never a
  silently weakened check.

Validation returns `SchemaError { kind, path, detail }` with
`SchemaErrorKind ∈ {not_json, empty, type_mismatch, missing_field, extra_field, out_of_range,
enum_mismatch, unsupported_schema}`. A schema is hashed through `canonical_json` (sorted keys), so
two equal schemas hash the same and a schema change invalidates derived artifacts by hash rather
than by a version number someone must remember to bump.

### 6.3 Constrained decoding, when it is available

`crates/mm-llm/src/grammar.rs` compiles a schema into a grammar in one of two dialects
(`llguidance`, `xgrammar`) via `DecoderBackend`. `GrammarSpec { schema_id, schema_sha, decoder,
grammar, fingerprint }` is keyed by `(schema_sha, decoder)` and persisted in `llm_grammars`
(`migrations/0003_llm.sql`), so a repeat structured call neither recompiles nor re-persists
(`compile_grammar` keeps an in-memory map keyed by fingerprint). Compilation is **total for the
supported subset and a hard error outside it**: a grammar that silently dropped a constraint would
make the decoder produce values the validator then rejects, and the validator would look like the
bug. `GrammarSpec::accepts` is the conformance predicate — a valid sample must be accepted, and a
value a permissive JSON grammar would allow (an extra field, an out-of-range score) must not.

### 6.4 The repair loop, and why it is bounded

`ensure_valid` is only entered for a call that named a schema:

1. Validate. If it passes, return.
2. Emit `llm.schema.reject` at `Warn`, with `error_kind` and **`raw_len` — never the content**, since
   a rejected payload is caller data and may carry a secret.
3. Attempts = `max_repair_attempts` (shipped: 1) normally, but **0 when the response came from a
   constrained decoder**: a decode that violated its own grammar is a decoder inconsistency, not
   something a second identical decode fixes.
4. Each attempt calls the client again; accepted → `repair_attempts` row with `accepted = 1` and the
   repair's usage is **added** to the first attempt's, so the ledger row is the cost of *the call*,
   not of its last try; rejected → a row with `accepted = 0` and the loop continues.
5. Exhausted → `CallStatus::SchemaRejected` + `LlmError::SchemaRejected`. The answer is never
   coerced, and the error names the schema and the last reason.

Repair rows are buffered and written by `finish` *after* the call row, because
`llm_repair_attempts.call_id` is a real foreign key — the FK is enforced, not merely declared.
`mm-cli llm schema-reject --fixtures bench/llm/malformed --assert-all-rejected` additionally
asserts the count identity: **every rejected call leaves exactly one repair row**.

### 6.5 The one deviation: `LlmDiagnoser` parses by hand

`crates/mm-metaanalysis/src/diagnosis.rs:238` is the plan's "one bounded call" path, but it does
**not** register a `StructuredOut`: it sends a system prompt + rendered episode with no schema, then
`serde_json::from_str::<DiagnosisReply>(&response.text)` and validates the sixteen error-class names
itself. Consequences worth knowing: no grammar, no strict-schema rejection, no `llm_calls.schema_id`
— and a malformed answer surfaces as `MetaError::Diagnosis("the answer is not a diagnosis: …")`.
This is a real gap against §6.2/§6.3, not a design choice stated anywhere. (`HeuristicDiagnoser` is
the wired default; see §11.)

## 7. Building contexts (how a prompt is assembled)

There is no prompt templating language, no conversation memory, and no retrieval that silently
pastes documents into a call. Every shipped prompt is **a constant system turn plus a deterministic
rendering of typed state**, and that determinism is what makes the exact cache and the recorded
replay meaningful: the same request renders the same bytes.

| Caller | System turn | User turn |
|---|---|---|
| Metacog scan (`scan.rs`) | `SCAN_SYSTEM_PROMPT` | `ScanRequest::prompt()` — `Goal: …`, `Novelty: {:.3}`, `Time pressure: {:.3}` |
| Hosted decision core (`hosted.rs:240`) | `HOSTED_DECISION_SYSTEM_PROMPT` | `HostedCore::prompt(question, state)` — question kind, the question, options/scale anchors, then `state.canonical()` |
| Diagnoser (`diagnosis.rs`) | `DIAGNOSIS_SYSTEM_PROMPT` | `LlmDiagnoser::prompt(ctx)` — trigger, goal, failure, attempts, conditions |
| Pi authoring (not `mm-llm`) | `pi/system_prompt.md` | a task contract rendered from `pi/prompts/module_scaffold.md` |

Two of these are worth spelling out.

**The scan's context is a bounded, typed episode, not a transcript.** `Context` is
`{ summary, novelty, time_pressure }` with `summary` non-empty because it is what the compiled
program asks `Recall` for (`crates/mm-metacog/src/episode.rs:169`, `validate` at `:201`). The
episode as a whole holds *references* to records owned by other organs — claims and assumptions from
`/epistemic`, frames and techniques from `/library`, a goal from the being — never copies, so there
is one authority for each. The scan's answer (`ScanResult`) is then the *only* model input to
compilation: `DefaultCompiler::compile` selects operations from the episode's frames/techniques and
the tier's allowed `OpClass`es, scoring each with the deterministic `OperationValue`
(`program.rs:312`, rules R1–R7, `value_for` at `:838`). Selection is deterministic **by invariant**:
a cost that came from a model would make two runs of the same episode choose differently.

**The Pi path builds its context from files.** `pi/system_prompt.md` states where Pi may write (a
prepared git worktree under `data/sandbox/<change-set-ulid>/`, nothing else, ever — and specifically
never `config/`, the being's `invariants.rs`, `permissions.rs`, `ontology/shapes/**`, migrations, or
the regression tests), and that its output is a *candidate* until the deterministic promotion gate
admits it. `pi/prompts/module_scaffold.md` is filled with the goal, module IRI, the skill to follow,
the allowed paths, and the definition of done; `pi/skills/` holds `mm-module-scaffold` and
`mm-design-revise`. The runtime drives this through `mm-pi` (`PiClient::spawn`), gated on the `pi`
binary being present (`pi_binary_available()`), and the loop's goal fixture carries the contract
(`mm-runtime/src/lib.rs` `LoopGoal::pi_task`).

## 8. "Tool selection": what the system actually does

This is the part most likely to be misread, because the repository contains three different things
that sound like tool selection. Only one of them is live.

### 8.1 The model never chooses or calls a tool

There is no tool-calling message type in `client.rs`, no function-calling schema, and no path from an
LLM answer to a tool invocation. `rg 'mm_tools' crates modules` resolves to exactly two CLI files
(`tools_cmd.rs`, `policy_cmd.rs`): the tool layer is reached by an operator command or the MCP
bridge, never by a model turn. What the model *does* provide that mentions tools is a **plan naming
required tools as data**:

- An operation class that means "do something in the world" is `OpClass::Act` (`OpClass::Decide |
  OpClass::Act` are the two classes whose nodes `is_terminal`, `mm-metacog/src/loop_.rs:320`).
- Compilation collects the tools a program needs into `CognitiveProgram::required_tools` (sorted,
  deduplicated) and hashes them into the program's content hash, so two programs that need different
  tools cannot share a hash.
- Lowering builds a `ToolCatalog` from the program and **refuses** a program that requires a tool
  the runner does not hold — "a tool the catalog does not hold is a refusal, not a runtime
  surprise", and the refusal names the tool (`mm-metacog/src/lower.rs:21`, `ToolCatalog` at `:205`).

So the metacog controller *constrains* which tools a plan may need and makes that requirement part
of the plan's identity; it does not execute them. Execution is §8.3.

### 8.2 The selection split fixture has no reader

`bench/tools/selection/{selection.jsonl,eval.jsonl}` are a ToolBench-style split
(`{"query", "tools", "expected"}`), documented in `bench/tools/README.md §3` as existing "so a
selector's accuracy is measured on queries it was not tuned on". The Phase 10 plan states the intent
outright — "tool selection consumes `ToolSpec` schemas; `bench/tools/` mirrors ToolBench's
selection/eval split" (`design/planning/full/phase_10_tools_execution_build_plan.md:25`) — but a
repository-wide search for a reader finds none: neither the CLI, the tests, nor the gate load either
file. They are a fixture for a selector nobody built. If you want model-side tool selection, this is the fixture to build it
against — and the first thing to write is the test that reads them. (Contrast
`bench/tools/adversarial/`, which *is* loaded by `tests/e2e/phase10_tools.rs:335`, and
`bench/library/evolution/eval.jsonl`, which `mm-cli policy` grades against.)

### 8.3 What is live: the deterministic tool-use pipeline

Tools are used through `mm-cli` (or the MCP bridge), and the pipeline is fixed by the executor, not
by the caller:

```text
resolve → authorize → tier → snapshot → claim → invoke → observe → ledger
```

(`crates/mm-tools/src/executor.rs`, module doc.) The properties hold because of *where* each step
sits:

- **A denial short-circuits before any side effect** — authorization runs before the idempotency
  claim, the snapshot and the invocation — and the refusal is still *recorded* (a `tool_calls` row,
  a ledger entry, an `ActionResult` with status `denied`), because "nothing happened" is a fact the
  ledger has to attest to.
- **Authorization is a pure function of `(grants, policy sets, as_of, request)`**
  (`crates/mm-tools/src/permissions.rs`), decided by code: a matching `forbid` denies whatever else
  matches; no covering grant denies (absence is denial); an `Irreversible` tool the caller has not
  explicitly confirmed **escalates** rather than allowing. Nothing here reads a plan, a confidence
  or a model output. A resource is `kind:verb:pattern` — the same string in a tool's
  `ToolSpec::permissions`, in a grant's `scope`, and on the CLI — so a grant on `fs.*` cannot
  authorize `process.exec`.
- **The observation is built from the run, never supplied.** The only constructor call site is the
  executor's success arm, handed the tool's own `ToolOutcome`, a fresh evidence id, and a fresh claim
  id. A plan, a model or a caller has no path to an observation.
- **The claim the observation produces is admitted through Phase 6's own barrier** before it is
  written; without an observer the executor still builds the claim, asks the barrier, and records the
  observation and its evidence — it just does not mirror `/world`.
- **Effects are exactly-once** (idempotency key claimed before, finished after), **a reversible
  action is snapshotted before it runs** so `action rollback` has something to restore, and every
  step emits one of the ledger's closed set of events.

`mm-cli tool list` shows what is registered and what it declares; `action ledger verify` walks the
chain; the adversarial corpus (`bench/tools/adversarial/`) is every one of these refusals as data —
"every case here is a denial that deterministic code produces, never a judgment a model makes".

## 9. Accounting, provenance, redaction

`crates/mm-llm/src/accounting.rs` and `migrations/0003_llm.sql`:

| Table | One row per |
|---|---|
| `llm_calls` (`:12`) | call — purpose, provider, model, `prompt_hash`, schema, decoder, `schema_ok`, `routed_from`, `route_reason`, `cached`, `cache_layer`, tokens, cost, latency, status, `error_kind`, trace |
| `llm_cache_index` (`:43`) | cached body (projection, rebuildable, re-hashable) |
| `llm_semantic_cache` (`:52`) | semantic entry with its embedding, purpose and similarity |
| `llm_repair_attempts` (`:65`) | repair pass, FK to `llm_calls(id)` |
| `llm_routing` (`:74`) | routing decision, including calls that never happened |
| `llm_grammars` | compiled grammar per `(schema_sha, decoder)` |

`CallStatus ∈ {ok, schema_rejected, provider_error, timeout, replay}`. The invariant is **exactly one
complete row per call**; `assert_complete` makes it checkable rather than hoped for, and
`mm-cli llm stats --assert-complete` is how the gate checks it. `StatsWindow` (`All`, `Since`,
`Purpose`) is what `llm stats` aggregates over, per purpose.

Provenance is written from the same `CallAccount`, as one `mm:LlmCall` node in the `/provenance`
graph, keyed by the call ULID (`provenance.rs`): `GRAPH = "provenance"`, node IRI
`mm_core::iri::DATA + call_id`. It carries the hash and the numbers and **no prompt text and no
key**. Re-recording is idempotent in RDF terms, so replaying the log converges to the same graph.

Redaction (`redact.rs`): the substrate never logs a prompt — it logs the sha256 and, where a human
needs context, a length-bounded single-line excerpt that has already been through the kernel's shared
`RedactionPolicy` (the same policy `mm-log`'s sinks use, so one added secret pattern protects both).
`Redactor::is_secret_key` exists so a caller can drop a secret-named field from a structured record
rather than hash it. One consequence recorded in the code: the completion-token count is logged
under `usage.out`, not `tokens_out`, because the kernel's redaction is deliberately blunt and
triggers on field names containing `token`.

## 10. Replay and offline determinism

`crates/mm-llm/src/replay.rs`. A recorded session is a list of `(request, response)` pairs
(`Session { session_id, records }`, committed under `bench/llm/sessions/`), keyed by the same request
hash the cache uses. Two independent halves prove the no-network claim:

- `ReplayLlmClient` has no transport at all and reports `provider_calls` (the counter must be 0);
- `NetworkGuard::attempt(host)` turns any outbound attempt made *anywhere in the process* into a hard
  error while a replay is armed.

`mm-cli llm replay --session bench/llm/sessions/recorded-session-01 --assert-zero-calls` is the gate
criterion: `provider_calls == 0`. `Session::fingerprint()` hashes the recorded content so a replay can
prove which session it replayed. Replay is the substrate's determinism story for everything above it:
a being that re-runs yesterday's cognition must not depend on today's model. `CacheMode::Replay`
composes with this — serve only recorded responses, and treat a miss as an error rather than a
request.

## 11. What is wired, and what only exists

Honest status at `HEAD = 0204d73`. The substrate is heavily tested; the *consumers* are thin.

| Path | State |
|---|---|
| `mm-cli decide --core hosted`, `conformance --core hosted` | **Wired.** Builds `OpenAiClient` from config when a provider is configured, registers `HostedDecisionReply`, else `BuiltCore::unavailable(id, reason)` — the firewall then degrades to rules + escalate rather than fabricating (`crates/mm-cli/src/decision.rs:257–306`). |
| `mm-cli llm …` (`stats`, `replay`, `verify-cache`, `schema-reject`, `routes`, `grammars`) | **Wired**, over the store, a recorded session, or a mock client. This is the substrate's operator surface. |
| `LlmScanDriver` (metacog) | **Tests only.** `mm-cli episode` runs `MockScanDriver` (`crates/mm-cli/src/episode.rs:468`), i.e. the scan is a fixture, not a call, in the shipped CLI path. |
| `LlmDiagnoser` (metaanalysis) | **No production wiring.** The default is `HeuristicDiagnoser` (`diagnosis.rs:360`, `lessons.rs:359`). |
| `mm-llm` dependency of `mm-library` | Declared in `Cargo.toml`, **no use in `src/`** (the planned `StructuredOut<SeedEntry>` extraction path was not built). |
| `mm-llm` dependency of `mm-memory` | Used, but only for `semantic_cache::embed` / `cosine` (one embedding in the system). |
| `mm-runtime` | **No `mm-llm` dependency at all.** The closed loop reaches models only through the Pi authoring path — a *second* door, with its own accounting story (Pi sessions are ingested and traced, not written to `llm_calls`). |
| Live provider traffic | **None in this environment.** See [`open-gaps.md#g6`](./open-gaps.md#g6) and the factuality seam at [`#g2`](./open-gaps.md#g2) (the Phase 10 tool seam, deferred integration point D3). |

Two consequences to keep in mind when reading the phase plans: the plans describe a system in which
the metacog scan and the diagnoser are live model calls and the tool seam is closed, and the tree
delivers the substrate plus one live hosted core. And because the Pi path is not `mm-llm`, a
complete cost/provenance story for "what did the being spend on models today" requires both paths.

## 12. Where the invariants live, and how to falsify them

```bash
# The substrate as a unit.
cargo test -p mm-llm                      # accounting, cache_replay, grammar_conformance,
                                          # no_hardcoded_endpoint, routing, schema_reject
# Every malformed fixture is rejected, for the right reason, one repair row each.
mm-cli llm schema-reject --fixtures bench/llm/malformed --assert-all-rejected
# Every ledger row is complete.
mm-cli llm stats --assert-complete --json
# Cache bodies and index rows agree, with no orphans.
mm-cli llm verify-cache --strict
# A recorded session replays with zero provider calls, network guard armed.
mm-cli llm replay --session bench/llm/sessions/recorded-session-01 --assert-zero-calls
# Routing is a table, not a mood.
mm-cli llm routes --purpose plan --complexity high --stakes high --precision normal
# The three gates a tool layer must not break.
mm-cli tool list && mm-cli action ledger verify
cargo test --workspace && mm-cli codex verify && mm-cli logs verify
```

Fixtures: `bench/llm/schemas.json` (registration), `bench/llm/malformed/` (8 violating payloads with
expected rejection kinds), `bench/llm/grammar_fixtures/` (including `unsupported_combinator.json`,
which must fail compilation), `bench/llm/sessions/recorded-session-01/`. The adversarial tool corpus
lives at `bench/tools/adversarial/`; the unused selection split at `bench/tools/selection/`.

## 13. Reading order for the code

1. `crates/mm-llm/src/lib.rs` — the two rules, and the module map.
2. `client.rs` — the vocabulary; everything else is behaviour around these types.
3. `service.rs` — the pipeline in §3, and `call_structured` / `ensure_valid`.
4. `cached.rs` + `semantic_cache.rs` — what a hit means and when one is allowed.
5. `schema.rs` + `grammar.rs` — strictness and constrained decoding.
6. `routing.rs`, `accounting.rs`, `provenance.rs`, `redact.rs`, `replay.rs` — the guarantees around
   a call.
7. `crates/mm-decision/src/hosted.rs` and `crates/mm-metacog/src/scan.rs` — the two structured
   callers, i.e. how a caller is supposed to look.
