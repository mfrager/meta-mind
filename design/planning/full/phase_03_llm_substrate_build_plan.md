# Phase 3 — LLM Cognitive Substrate (Extended Build Plan)

> Extended from: `design/planning/phase_03_llm_substrate_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 3 · Codename Metamind (`mm`)

The LLM is the being's semantic substrate. This phase gives it exactly one door: a typed, cached,
accountable interface (`mm-llm`) that guarantees schema-valid output, records everything, and replays
offline with zero provider calls. This revision streamlines the plan and locks in the mechanisms below.

---

## 0. Research foundation (code-available)

Only ideas with available source are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| Constrained decoding (token-level schema enforcement) | llguidance — https://github.com/guidance-ai/llguidance | Grammar/token-mask constrained sampling, fast Rust engine (~40–50 µs/token) | Preferred decoder when the backend exposes logits: output is schema-valid **by construction**, no retry loop |
| Schema→grammar + guaranteed structure | Outlines — https://github.com/dottxt-ai/outlines | Compile JSON Schema to a grammar; "guarantees structured outputs during generation" | `GrammarSpec` is compiled from the registered strict schema and cached by schema hash |
| Alternative grammar runtime | XGrammar — https://github.com/mlc-ai/xgrammar | Second constrained-decoding backend | `DecoderBackend::XGrammar` selectable; both engines behind one trait |
| Decoder evaluation methodology | https://github.com/Saibo-creator/Awesome-LLM-Constrained-Decoding | Bench engines on JSON Schema rather than trusting claims | `mm-cli llm grammars` scores backends on the committed fixture set before selecting a default |
| Semantic/intent routing | vLLM Semantic Router — https://github.com/vllm-project/semantic-router · semantic-router — https://github.com/aurelio-labs/semantic-router | Embedding-based intent/complexity routing layer | Router features are computed deterministically; embeddings only *propose*, a threshold decides |
| Learned model routing | RouteLLM — https://github.com/lm-sys/RouteLLM | A cheap router classifier that picks "small vs frontier" | `Router` trait with `ThresholdRouter` now and a learned `Router` impl in Phase 11 (deterministic-first) |
| Semantic cache | GPTCache — https://github.com/zilliztech/GPTCache | Embedding-similarity cache with a similarity threshold | Exact-hash cache is always on; semantic cache is **opt-in per purpose**, gated by a threshold + policy |
| Normalized provider surface | LiteLLM — https://github.com/BerriAI/litellm | One request/response/accounting model across providers | `LlmRequest/LlmResponse/Usage` normalize providers; adapters are thin |
| Optimizable prompt modules | DSPy — https://github.com/stanfordnlp/dspy | Prompts/pipelines are modules that can be optimized from traces | Every call records a trace now; Phase 11 optimizes prompts/pipelines from those traces |

---

## 1. Objective and scope

Deliver one typed, cached, accountable LLM interface (`mm-llm`). **No component may call a provider
directly**; all inference flows through `mm-llm`, which guarantees each call is schema-valid, budgeted,
provenance-recorded, logged, and reproducible offline.

**In scope:** provider-neutral async client; schema→grammar structured output with a decoder-backend
choice; exact + optional semantic caching; replay with a hard network guard; deterministic-first routing;
per-call accounting and provenance; `mm-cli llm`; adversarial fixtures.

**Out of scope:** the metacognitive controller/program compiler (Phase 8); `DecisionCore` + calibration
(Phase 9); being/memory semantics (Phases 4–5). Phase 3 supplies the interface, not their logic.

**Invariant:** *constrain at decode when the backend exposes logits; otherwise request provider-native
JSON Schema; otherwise validate-and-reject — never coerce.*

---

## 2. Architecture

```
LlmRequest
   │
   ▼
Router.select ──► RoutingDecision{model, reason}         (deterministic thresholds; Phase 9 seam)
   │
   ▼
Cache: exact hash (always) ── hit ─► LlmResponse(cached)
   │            └ semantic (opt-in, threshold + policy)
   ▼ miss
DecoderBackend: llguidance | xgrammar | provider_native | none
   │   (schema → GrammarSpec, cached by schema hash)
   ▼
Provider adapter (kg-llm, env-configured)  ── NetworkGuard in replay/test
   │
   ▼
Validation: constrained ⇒ valid by construction; else strict validate → bounded repair → reject
   │
   ▼
Account (llm_calls) + provenance (mm:LlmCall) + mm-log  ── all in one event-log transaction
   │
   ▼
LlmResponse / StructuredOut<T>
```

Rules:
- **Structured calls name a `SchemaId`.** The schema is compiled to a `GrammarSpec` once and cached.
- **Validation never coerces.** On failure it records one `llm_repair_attempts` row and either runs a
  bounded repair pass (re-decode with the same grammar) or returns `SchemaRejected`.
- **Replay is authoritative.** `ReplayLlmClient` serves recorded responses keyed by `prompt_hash`; the
  `NetworkGuard` makes any outbound attempt a hard error.
- **Semantic cache is never used for consequential structured decisions** unless the purpose is declared
  idempotent in `config/llm.toml`; its risk is documented in §9.

---

## 3. Deliverables and workspace layout

```
crates/mm-llm/
├── src/{lib,client,config,error,accounting,schema,grammar,backend_llguidance,
│         backend_xgrammar,mock,cached,semantic_cache,openai,routing,replay,service,provenance,redact}.rs
├── tests/{accounting,cache_replay,schema_reject,routing,grammar_conformance}.rs
├── migrations/0003_llm.sql          # llm_calls, llm_cache_index, llm_semantic_cache,
│                                     # llm_repair_attempts, llm_routing, llm_grammars
└── metadata.ttl                     # GENERATED by mm-codex (never hand-edited)
vendor/rust_extract/kg-llm/{…, COPYING.md}
vendor/nexus/agentstream/{…, COPYING.md}
vendor/rust_symbolic/llm-interface/{…, COPYING.md}     # orchestration/repair pattern only
vendor/nexus/monad-llm/{…, COPYING.md}                 # DSL-level access for later modules
ontology/llm.ttl                                       # mm:LlmCall vocabulary (merged into mm.ttl)
config/llm.toml                                        # non-secret model specs, thresholds, purpose policy
bench/llm/{sessions/recorded-session-01/, malformed/, grammar_fixtures/}
mm-cli subcommands: llm {stats, replay, verify-cache, schema-reject, routes, grammars}
```

---

## 4. Detailed specifications

### 4.1 Core traits and types (`mm-core`-aligned, `#![forbid(unsafe_code)]`)

```rust
#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError>;
}

pub enum Purpose { Interpret, Plan, Extract, Critique, Summarize, CodeReview, Classify, Diagnose }
pub enum CacheMode { Use, Bypass, Refresh, Replay }

pub struct LlmRequest {
    pub purpose: Purpose,
    pub model: Option<String>,        // None => router chooses
    pub messages: Vec<Message>,
    pub schema: Option<SchemaId>,     // Some => structured output enforced
    pub max_tokens: u32,
    pub temperature: f32,
    pub cache: CacheMode,
    pub trace_id: Option<ulid::Ulid>,
}
pub struct LlmResponse {
    pub call_id: ulid::Ulid, pub text: String, pub model: String,
    pub usage: Usage, pub cached: bool, pub schema_valid: bool, pub decoder: DecoderKind,
}
pub struct Usage { pub tokens_in: u32, pub tokens_out: u32, pub cost_micros: i64, pub latency_ms: u32 }

// schema + grammar
pub trait StructuredOut: serde::de::DeserializeOwned + Send {
    fn schema_id() -> SchemaId;
    fn schema() -> serde_json::Value;          // strict JSON Schema (no implicit coercion)
}
pub trait SchemaRegistry {
    fn register(&mut self, id: SchemaId, schema: serde_json::Value);
    fn get(&self, id: &SchemaId) -> Option<&serde_json::Value>;
    fn validate(&self, id: &SchemaId, raw: &str) -> Result<(), SchemaError>;
}
pub struct GrammarSpec { pub schema_id: SchemaId, pub grammar: String /* cached by schema hash */ }
pub enum DecoderKind { Llguidance, XGrammar, ProviderNative, None }

pub trait DecoderBackend: Send + Sync {
    fn kind(&self) -> DecoderKind;
    fn compile(&self, schema: &serde_json::Value) -> Result<GrammarSpec, LlmError>;
    fn available(&self) -> bool;
}

// cache
pub struct CacheKey { pub prompt_hash: String, pub model: String, pub schema_id: Option<SchemaId> }
pub trait ExactCache { fn get(&self, k: &CacheKey) -> Option<CachedResponse>;
                       fn put(&self, k: CacheKey, r: CachedResponse) -> Result<(), LlmError>; }
pub struct SemanticCache { /* embedding index; similarity_threshold; purpose allow-list */ }

// routing (deterministic-first; Phase 9 DecisionCoreRouter implements the same trait)
pub trait Router: Send + Sync { fn choose(&self, req: &RoutingRequest) -> RoutingDecision; }
pub struct RoutingRequest { pub purpose: Purpose, pub complexity: Complexity, pub stakes: Stakes,
    pub required_precision: Precision, pub latency_budget_ms: u32, pub cost_budget_micros: i64,
    pub pinned_model: Option<String> }
pub struct RoutingDecision { pub model: String, pub reason: RoutingReason }
pub struct ThresholdRouter { /* cheapest model meeting the requirement, from config/llm.toml */ }

// accounting + facade
pub struct CallAccount { pub call_id: ulid::Ulid, pub trace_id: Option<ulid::Ulid>, pub purpose: Purpose,
    pub provider: String, pub model: String, pub prompt_hash: String, pub schema_id: Option<SchemaId>,
    pub schema_ok: bool, pub decoder: DecoderKind, pub cached: bool, pub tokens_in: u32,
    pub tokens_out: u32, pub cost_micros: i64, pub latency_ms: u32, pub status: CallStatus,
    pub error_kind: Option<String> }

pub struct LlmService { /* pool, graph handle, client, router, exact cache, semantic cache, schemas,
                           decoder backends, config */ }
impl LlmService {
    pub fn new(cfg: LlmConfig, pool: sqlx::SqlitePool, graph: GraphHandle) -> Result<Self, LlmError>;
    pub async fn call(&self, req: LlmRequest) -> Result<LlmResponse, LlmError>;
    pub async fn call_structured<T: StructuredOut>(&self, req: LlmRequest)
        -> Result<(T, LlmResponse), LlmError>;
    pub async fn stats(&self, window: StatsWindow) -> Result<LlmStats, LlmError>;
}
```

### 4.2 SQLite schema — `crates/mm-store-sqlite/migrations/0003_llm.sql`

```sql
CREATE TABLE llm_calls (
  id            TEXT(26) PRIMARY KEY,     -- ULID
  trace_id      TEXT(26),
  purpose       TEXT NOT NULL,            -- interpret|plan|extract|critique|summarize|code_review|classify|diagnose
  provider      TEXT NOT NULL,
  model         TEXT NOT NULL,
  prompt_hash   TEXT NOT NULL,            -- sha256 over canonical request
  schema_id     TEXT,
  decoder       TEXT NOT NULL,            -- llguidance|xgrammar|provider_native|none
  schema_ok     INTEGER NOT NULL DEFAULT 1,
  routed_from   TEXT,
  route_reason  TEXT,
  cached        INTEGER NOT NULL DEFAULT 0,
  cache_layer   TEXT,                     -- exact|semantic|none
  tokens_in     INTEGER NOT NULL DEFAULT 0,
  tokens_out    INTEGER NOT NULL DEFAULT 0,
  cost_micros   INTEGER NOT NULL DEFAULT 0,
  latency_ms    INTEGER NOT NULL DEFAULT 0,
  status        TEXT NOT NULL,            -- ok|schema_rejected|provider_error|timeout|replay
  error_kind    TEXT,
  created_ulid  TEXT(26) NOT NULL,
  created_at    TEXT NOT NULL
);
CREATE INDEX idx_llm_calls_purpose ON llm_calls(purpose);
CREATE INDEX idx_llm_calls_hash    ON llm_calls(prompt_hash);
CREATE INDEX idx_llm_calls_trace   ON llm_calls(trace_id);

CREATE TABLE llm_cache_index (
  prompt_hash TEXT PRIMARY KEY, model TEXT NOT NULL, schema_id TEXT,
  response_path TEXT NOT NULL, response_sha TEXT NOT NULL, created_ulid TEXT(26) NOT NULL
);

CREATE TABLE llm_semantic_cache (
  id TEXT(26) PRIMARY KEY, prompt_hash TEXT NOT NULL, embedding BLOB NOT NULL,
  response_path TEXT NOT NULL, purpose TEXT NOT NULL, similarity REAL NOT NULL,
  created_ulid TEXT(26) NOT NULL
);

CREATE TABLE llm_repair_attempts (
  id TEXT(26) PRIMARY KEY, call_id TEXT(26) NOT NULL REFERENCES llm_calls(id),
  attempt_no INTEGER NOT NULL, error_kind TEXT NOT NULL, accepted INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE llm_routing (
  id TEXT(26) PRIMARY KEY, call_id TEXT(26), requested TEXT, selected TEXT NOT NULL,
  reason TEXT NOT NULL, cost_micros INTEGER NOT NULL DEFAULT 0, created_ulid TEXT(26) NOT NULL
);

CREATE TABLE llm_grammars (
  schema_id TEXT PRIMARY KEY, schema_sha TEXT NOT NULL, grammar TEXT NOT NULL,
  decoder TEXT NOT NULL, created_ulid TEXT(26) NOT NULL
);
```

### 4.3 Schema → grammar rules

- A `SchemaId` maps 1:1 to a strict JSON Schema (draft 2020-12); `additionalProperties: false` and
  explicit `required` are mandatory.
- `GrammarSpec` is compiled once per `(schema_sha, decoder)` and persisted in `llm_grammars`; a schema
  change invalidates the cache by hash.
- Supported fragments only; an unsupported construct is a hard error (never a silent loosened grammar).
- `provider_native` is used when the backend cannot expose logits but supports JSON Schema
  (`response_format`); if neither holds, `DecoderKind::None` + strict validate is the fallback.

### 4.4 Config — env-only endpoints

```toml
# config/llm.toml  (NO secrets, NO URLs)
[defaults]
decoder = "llguidance"          # llguidance | xgrammar | provider_native | none
semantic_cache_purposes = ["summarize"]   # opt-in; consequential structured calls are exact-only
semantic_cache_threshold = 0.97

[[model]]
name = "small"
input_cost_micros_per_1k = 150
output_cost_micros_per_1k = 600
typical_latency_ms = 400
complexity_ceiling = "medium"
[[model]]
name = "frontier"
input_cost_micros_per_1k = 2500
output_cost_micros_per_1k = 10000
typical_latency_ms = 2500
complexity_ceiling = "very_high"
```

`openai.rs` reads `OPENAI_BASE_URL`/`OPENAI_API_KEY` at construction and returns
`LlmError::ProviderNotConfigured` when absent — **no hard-coded fallback URL**.

### 4.5 CLI

```bash
mm-cli llm stats [--since <rfc3339>] [--purpose <p>] [--json]
mm-cli llm replay --session bench/llm/sessions/recorded-session-01 --assert-zero-calls
mm-cli llm verify-cache --strict              # every cache row resolves and re-hashes identically
mm-cli llm schema-reject --fixtures bench/llm/malformed/ --assert-all-rejected
mm-cli llm routes --purpose plan --complexity high --stakes high --cost-budget 50000
mm-cli llm grammars --fixtures bench/llm/grammar_fixtures/ --score   # pick a decoder backend
```

---

## 5. Build sequence

1. **Vendor copy.** Copy `kg-llm` + the agentstream/llm-interface patterns into `vendor/…`, add
   `COPYING.md`, register workspace members. *Check:* vendored crate builds in-workspace.
2. **Crate skeleton + config.** `crates/mm-llm` with `#![forbid(unsafe_code)]`; `config.rs` loads
   `config/llm.toml` + env. *Check:* loads with no env and with env.
3. **Client/types/errors.** `client.rs`, `error.rs` (`ProviderNotConfigured, Transport, Timeout,
   SchemaRejected, CacheMiss, Io, Db`).
4. **Accounting + migration.** `accounting.rs` + `0003_llm.sql`; one `llm_calls` row per call inside the
   event-log transaction. *Check:* one row + one audit event per call.
5. **Mock client.** Deterministic fixture/scripted responses, able to emit malformed payloads.
6. **Exact cache + request hash.** Canonicalize `(provider, model, purpose, messages, schema_id,
   max_tokens, temperature)` → sha256. *Check:* property test — hash stable and field-order independent.
7. **Decoder backends.** `grammar.rs`, `backend_llguidance.rs`, `backend_xgrammar.rs`, `provider_native`;
   compile + persist `GrammarSpec`. *Check:* `llm grammars --score` on fixtures; unsupported schema
   fragment errors.
8. **OpenAI-compatible adapter.** `openai.rs` via vendored client; env-only; bounded concurrency/retries/
   timeout. *Check:* missing env ⇒ `ProviderNotConfigured`.
9. **Structured output.** Wire `schema.rs` into `call_structured`: constrained decode when available;
   else strict validate → bounded repair → `SchemaRejected`. Never coerce. *Check:* `schema_reject.rs`
   rejects every fixture.
10. **Semantic cache (opt-in).** `semantic_cache.rs`: embed, compare, threshold; purpose allow-list.
11. **Routing.** `routing.rs` `ThresholdRouter`; record `llm_routing`; support `pinned_model` +
    `budget_exhausted`. Leave the `Router` seam for Phase 9.
12. **Replay + network guard.** `replay.rs` serves recorded responses by `prompt_hash`; guard hard-errors
    on egress. *Check:* zero calls, byte-identical output.
13. **Service + provenance.** `service.rs`, `provenance.rs`: one `mm:LlmCall` in `/provenance` per call.
14. **CLI.** `mm-cli llm {stats,replay,verify-cache,schema-reject,routes,grammars}` with `--json`.
15. **Fixtures + redaction.** Author `bench/llm/*`; `redact.rs` (hash prompts, length-bound excerpts).
16. **Registration.** `mm-codex scan` so `mm-llm` + vendored crates carry stable URIs, phase 3, tests.

---

## 6. Logging and observability

Global `mm-log` fields (`ts, level, target, event, msg, trace_id, span_id`) plus `call_id`/`purpose`.

| Event code | Level | Required fields |
|---|---|---|
| `llm.request` | INFO | call_id, purpose, provider, model, prompt_hash, schema_id, decoder, cache_mode, trace_id |
| `llm.route.select` | INFO | requested_model, selected_model, reason, cost_budget_micros, latency_budget_ms |
| `llm.cache.hit` / `llm.cache.miss` | DEBUG | call_id, prompt_hash, model, schema_id, layer |
| `llm.grammar.compile` | DEBUG | schema_id, schema_sha, decoder, ok |
| `llm.response` | INFO | call_id, tokens_in, tokens_out, cost_micros, latency_ms, cached, status |
| `llm.schema.reject` | WARN | call_id, schema_id, error_kind, raw_len (never raw content) |
| `llm.repair.attempt` | INFO | call_id, attempt_no, error_kind, accepted |
| `llm.replay.start` / `llm.replay.end` | INFO | session_id, records, provider_calls (must be 0) |
| `llm.accounting.commit` | DEBUG | call_id, trace_id, row_written |
| `llm.error` | ERROR | call_id, error_kind, full cause chain; never key or raw prompt |

`mm-cli logs verify` must confirm every committed `llm_calls` row has exactly one audit record and that
`llm.replay.end.provider_calls == 0` for replay runs.

---

## 7. Testing

- **Unit:** config (none/partial/full env); error causes; `ModelSpec` cost math; `Purpose` serde.
- **Property (`proptest`):** request-hash stability; replay equivalence; accounting conservation
  (tokens/cost non-negative; per-call sums reconcile).
- **Golden (`insta`):** routing decision table; grammar compile output for each fixture schema;
  recorded session replays byte-identically.
- **Adversarial:** `bench/llm/malformed/*` (`truncated`, `wrong_types`, `not_json`,
  `schema_extra_field`, `empty`) never crash, never yield a typed value, and produce exactly one
  `llm.schema.reject` each.
- **Zero-network:** replay/mock run under `NetworkGuard`; any socket attempt fails the test.
- **Grammar conformance:** every compiled grammar accepts a valid sample and rejects a mutated one.
- **Concurrency:** N bounded parallel calls preserve accounting (no lost/duplicated rows).
- **Redaction:** the shared secret-pattern suite finds no key/token in any sink.
- **E2E:** `llm stats` reconciles with `llm_calls`; `verify-cache --strict` re-hashes every entry.

---

## 8. Pass gate

```bash
cargo build --workspace
cargo clippy -p mm-llm --all-targets -- -D warnings
cargo test -p mm-llm
cargo test -p mm-llm --test schema_reject --test cache_replay --test routing --test grammar_conformance
cargo run -p mm-cli -- llm schema-reject --fixtures bench/llm/malformed/ --assert-all-rejected
cargo run -p mm-cli -- llm replay --session bench/llm/sessions/recorded-session-01 --assert-zero-calls
cargo run -p mm-cli -- llm verify-cache --strict
cargo run -p mm-cli -- llm stats --assert-complete --json
cargo run -p mm-cli -- logs verify
```

Objective criteria:

- **Zero provider calls on replay** — `provider_calls == 0`, byte-identical outputs, NetworkGuard armed.
- **100% schema-violation rejection** — every malformed fixture rejected, one repair row each, no typed value.
- **Complete accounting** — every call has one `llm_calls` row with non-null purpose/model/prompt_hash/
  tokens/cost/latency/status; totals reconcile with `llm stats`.
- **Deterministic routing** — `ThresholdRouter` picks the cheapest model meeting each labeled requirement;
  repeated runs identical (golden).
- **No hard-coded endpoint** — a test asserts no `http(s)://` literal exists in `mm-llm` source.
- **Logging** — `mm-cli logs verify` passes (schema, audit completeness, ULID correlation, redaction).
- Workspace builds with zero warnings; `mm-codex verify` stays green after `scan`.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Provider output drifts from schema | Constrained decode when possible; else strict validate + bounded repair + explicit `SchemaRejected`; never coerce |
| Semantic cache returns a wrong-but-similar answer | Off by default; opt-in per purpose; high similarity threshold; never for consequential structured decisions |
| Constrained decoding unavailable on a hosted endpoint | Fall back provider-native JSON Schema, then strict validate; `llm grammars --score` picks the default |
| Cost/latency runaway | `CallAccount` + budgets; cheapest-sufficient routing; `budget_exhausted` fallback |
| Secrets in logs | `Redactor` + prompt hashing; redaction suite across all sinks |
| Nondeterministic tests | Mock/cached/replay clients are the test default; live calls opt-in via env |
| Hard-coded endpoint sneaks in | Env-only config; source-scan test forbids `http(s)://` literals |
| Vendored `kg-llm` drift | `mmc:copiedFrom` pins revision; re-copy is a reviewed change-set (Phase 11) |

---

## 10. References (code-available)

- llguidance — https://github.com/guidance-ai/llguidance
- Outlines — https://github.com/dottxt-ai/outlines
- XGrammar — https://github.com/mlc-ai/xgrammar
- Constrained-decoding benchmark — https://github.com/Saibo-creator/Awesome-LLM-Constrained-Decoding
- vLLM Semantic Router — https://github.com/vllm-project/semantic-router
- aurelio-labs/semantic-router — https://github.com/aurelio-labs/semantic-router
- RouteLLM — https://github.com/lm-sys/RouteLLM
- GPTCache — https://github.com/zilliztech/GPTCache
- LiteLLM — https://github.com/BerriAI/litellm
- DSPy — https://github.com/stanfordnlp/dspy
