# Phase 5 Build Plan — Persistent memory architecture

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 5 · Codename Metamind (`mm`)

## 1. Objective and scope

Deliver the being's **long-term memory organ**: the ten memory classes of design §32 as typed,
provenance-bearing traces, with deterministic retrieval, episodic→semantic consolidation, deliberate
forgetting, and mistake/near-miss retention.

In scope:
- `mm-memory` crate: memory model, typed store, deterministic utility, hybrid retrieval, consolidation,
  forgetting, mistake/near-miss, procedural-memory records.
- SQLite tables + packed vector index; mirrored `/memory` RDF named graph; SHACL shapes.
- `modules/cognition/memory-recall` plugin module exposing `cognition.memory_recall` to the DSL.
- `bench/memory/` gold corpus and thresholds; `mm-cli memory …` subcommands.

Out of scope (later phases): world-model facts (Phase 6), library cases (Phase 7), prediction
calibration (Phase 11), Pi/self-modification (Phase 11). Phase 5 stores prediction-memory and
mistake-memory **records**; it does not yet evaluate them.

## 2. Prerequisites and dependencies

- **Phase 1** artifacts: `mm-core` (`Id`, `Timestamp`, `MmError`, async `Tabular`/`Graph`/`EventSink`
  traits), `mm-log`, `mm-store-sqlite` (migration harness), `mm-store-graph` (Oxigraph actor +
  `rdf-codec` wiring), `mm-eventlog` (append→apply→commit + replay), `ontology/mm.ttl` v0.
- **Phase 2** code metadata (`mm-codex`) so the new crate/modules register with stable URIs.
- **Phase 3** `mm-llm` for consolidation and semantic compression (schema-validated, replayable); the
  consolidation step may run fully offline via `MockLlmClient`/`CachedLlmClient`.
- **Phase 4** `mm-being` for `Belief<T>`/`EpistemicStatus`, goal/relationship IDs referenced by memory
  records, and resource budgets (retrieval and embedding cost debits).
- Copied-in references (see §6): `memory-ir`, `kg-embed`, `temporal-store`.

## 3. Deliverables (exact paths)

```
~/Build/metamind/
├── crates/mm-memory/
│   ├── Cargo.toml
│   ├── src/lib.rs                 # public re-exports
│   ├── src/error.rs               # MemoryError
│   ├── src/model.rs               # Memory, MemoryKind, RetrievalCue, Mistake, NearMiss
│   ├── src/utility.rs             # MemoryUtility + Salience
│   ├── src/store.rs               # MemoryStore trait + SqliteMemoryStore
│   ├── src/retrieval.rs           # hybrid keyword+embedding retrieval
│   ├── src/index.rs               # packed vector index (kg-embed convention)
│   ├── src/consolidate.rs         # episode→pattern→semantic→rule
│   ├── src/forgetting.rs          # forget_cycle + ledger protection
│   ├── src/mistake.rs             # mistake/near-miss records
│   ├── src/procedural.rs          # procedural memory / skill records
│   └── src/rdf.rs                 # ToRdf/FromRdf for the /memory graph
│   └── tests/{utility.rs,retrieval.rs,consolidation.rs,forgetting.rs,rdf_roundtrip.rs}
├── crates/mm-store-sqlite/migrations/0005_memory.sql
├── ontology/shapes/memory.ttl
├── modules/cognition/memory-recall/{plugin.toml,src/lib.rs,manual/module.md,tests/}
├── bench/memory/{gold.jsonl,episodes.jsonl,thresholds.toml,adversarial/}
└── data/index/memory.pack        # generated, gitignored
```

`mm-cli` subcommands: `memory add`, `memory get`, `memory recall`, `memory consolidate`,
`memory forget`, `memory stats`, `memory eval`, `memory verify`.

## 4. Data model and ontology deltas

**SQLite (`0005_memory.sql`).** All ULIDs are `TEXT(26)`; times are integer ns (Phase 1 `Timestamp`).

```sql
PRAGMA journal_mode=WAL;

CREATE TABLE memories (
  id            TEXT(26) PRIMARY KEY,
  kind          TEXT NOT NULL CHECK (kind IN
                  ('episodic','semantic','procedural','working','autobiographical',
                   'relational','prediction','mistake','near_miss','developmental')),
  content       TEXT NOT NULL,
  source_ulid   TEXT,
  confidence    REAL NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  importance    REAL NOT NULL CHECK (importance BETWEEN 0 AND 1),
  utility       REAL,                      -- last computed, cached
  valid_from    INTEGER NOT NULL,
  valid_until   INTEGER,                   -- NULL = open
  recorded_at   INTEGER NOT NULL,
  recorded_ulid TEXT NOT NULL REFERENCES events(ulid),
  provenance    TEXT NOT NULL,             -- PROV node ULID
  protected     INTEGER NOT NULL DEFAULT 0, -- 1 = never forgotten
  status        TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','archived')),
  content_hash  TEXT NOT NULL              -- sha256(canonical content)
);

CREATE TABLE memory_cues   (id TEXT(26) PRIMARY KEY, memory_id TEXT NOT NULL REFERENCES memories(id),
                            cue TEXT NOT NULL, cue_kind TEXT NOT NULL);
CREATE TABLE memory_entities(id TEXT(26) PRIMARY KEY, memory_id TEXT NOT NULL, entity_ulid TEXT NOT NULL, role TEXT);
CREATE TABLE memory_links  (id TEXT(26) PRIMARY KEY, from_id TEXT NOT NULL, to_id TEXT NOT NULL,
                            relation TEXT NOT NULL, weight REAL NOT NULL, created_ulid TEXT NOT NULL);
CREATE TABLE memory_access (id TEXT(26) PRIMARY KEY, memory_id TEXT NOT NULL, accessed_at INTEGER NOT NULL,
                            query_hash TEXT NOT NULL, score REAL NOT NULL, trace_id TEXT NOT NULL);
CREATE TABLE memory_consolidations(id TEXT(26) PRIMARY KEY, source_ids TEXT NOT NULL /* JSON */,
                            target_id TEXT NOT NULL, method TEXT NOT NULL, created_ulid TEXT NOT NULL);
CREATE TABLE mistakes (
  id TEXT(26) PRIMARY KEY, situation_ulid TEXT, decision_ulid TEXT, outcome_ulid TEXT,
  failure_mode TEXT NOT NULL, root_cause_ulid TEXT, missed_signal TEXT /* JSON */,
  corrective_rule_ulid TEXT, recurrence_risk REAL NOT NULL CHECK (recurrence_risk BETWEEN 0 AND 1),
  created_ulid TEXT NOT NULL);

CREATE VIRTUAL TABLE memory_fts USING fts5(content, content='memories', content_rowid='rowid');
CREATE INDEX idx_memories_kind ON memories(kind, status);
CREATE INDEX idx_memories_valid ON memories(valid_from, valid_until);
```

**RDF (`/memory` named graph = `https://metamind.dev/graph/memory`).** Add to `ontology/mm.ttl`:
classes `mm:Memory`, `mm:EpisodicMemory`, `mm:SemanticMemory`, `mm:ProceduralMemory`, `mm:WorkingMemory`,
`mm:AutobiographicalMemory`, `mm:RelationalMemory`, `mm:PredictionMemory`, `mm:MistakeMemory`,
`mm:NearMissMemory`, `mm:DevelopmentalMemory`, `mm:RetrievalCue`, `mm:Consolidation`, `mm:Mistake`,
`mm:NearMiss`, `mm:CorrectiveRule`; predicates `mm:memoryKind, mm:content, mm:source, mm:confidence,
mm:importance, mm:utility, mm:validFrom, mm:validUntil, mm:recordedAt, mm:protected, mm:retrievalCue,
mm:relatedEntity, mm:derivedFrom, mm:consolidatedFrom, mm:consolidatedInto, mm:accessedBy, mm:failureMode,
mm:rootCause, mm:missedSignal, mm:correctiveRule, mm:recurrenceRisk, mm:supports, mm:contradicts,
mm:similarTo`; plus `prov:wasDerivedFrom` / `prov:wasGeneratedBy`. Instance IRIs are
`https://metamind.dev/data/{ulid}` via `rdf-codec`.

`ontology/shapes/memory.ttl` SHACL: every `mm:Memory` has exactly one `mm:memoryKind`, one `mm:content`,
one `mm:confidence` in `[0,1]`, one `mm:importance` in `[0,1]`, one `mm:recordedAt`, and a `mm:source` or
explicit `mm:unknownSource`. Every `mm:Mistake` has one `mm:failureMode` and one `mm:recurrenceRisk`.
`mm:DevelopmentalMemory` nodes are `sh:minCount 1` for `mm:protected true` (structural marker).

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

```rust
// model.rs
pub enum MemoryKind { Episodic, Semantic, Procedural, Working, Autobiographical,
                      Relational, Prediction, Mistake, NearMiss, Developmental }

pub struct RetrievalCue { pub text: String, pub kind: CueKind }      // Keyword | Embedding | Entity

pub struct Memory {
    pub id: Ulid, pub kind: MemoryKind, pub content: String,
    pub source: Option<Ulid>, pub confidence: f32, pub importance: f32,
    pub validity: TimeInterval, pub recorded_at: Timestamp, pub provenance: Ulid,
    pub entities: Vec<Ulid>, pub cues: Vec<RetrievalCue>, pub outcomes: Vec<Ulid>,
    pub protected: bool,
}

pub struct Mistake {
    pub id: Ulid, pub situation: Option<Ulid>, pub decision: Option<Ulid>, pub outcome: Option<Ulid>,
    pub failure_mode: String, pub root_cause: Option<Ulid>, pub missed_signals: Vec<String>,
    pub corrective_rule: Option<Ulid>, pub recurrence_risk: f32,
}

// utility.rs — deterministic, no floats beyond f32 inputs, fixed evaluation order
pub struct UtilityInputs { pub future_behavior_impact: f32, pub retrieval_probability: f32,
                           pub reliability: f32, pub storage_cost: f64 }
pub fn memory_utility(i: &UtilityInputs) -> f64;                 // impact*prob*reliability / storage_cost
pub fn salience(m: &Memory, goal_relevance: f32, novelty: f32, prediction_error: f32) -> f32;

// store.rs
#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    async fn put(&self, m: &Memory) -> Result<Ulid, MemoryError>;
    async fn get(&self, id: &Ulid) -> Result<Option<Memory>, MemoryError>;
    async fn link(&self, from: Ulid, to: Ulid, relation: LinkKind, weight: f32) -> Result<(), MemoryError>;
    async fn record_access(&self, id: Ulid, score: f64, trace: Ulid) -> Result<(), MemoryError>;
    async fn mark_archived(&self, id: Ulid) -> Result<(), MemoryError>;
}

// retrieval.rs
pub struct RecallQuery { pub text: String, pub k: usize, pub kinds: Vec<MemoryKind>, pub now: Timestamp }
pub struct RecallHit { pub memory: Memory, pub score: f64, pub parts: Vec<ScorePart> }
pub struct MemoryEngine { /* store, embedder, index, config, sink */ }
impl MemoryEngine {
    pub async fn remember(&self, m: Memory) -> Result<Ulid, MemoryError>;
    pub async fn recall(&self, q: RecallQuery) -> Result<Vec<RecallHit>, MemoryError>;
    pub async fn reinforce(&self, id: Ulid, evidence: f32) -> Result<(), MemoryError>;
    pub async fn consolidate(&self, window: TimeInterval) -> Result<Vec<Consolidation>, MemoryError>;
    pub async fn forget_cycle(&self, now: Timestamp) -> Result<ForgetReport, MemoryError>;
    pub async fn record_mistake(&self, m: Mistake) -> Result<Ulid, MemoryError>;
}
```

`plugin.toml` for `modules/cognition/memory-recall/`:

```toml
[plugin]
name = "mm-memory-recall"
uri = "https://metamind.dev/code/module/cognition/memory-recall"
version = "0.1.0"
[metadata]
category = "cognition"
owned_by_phase = 5
capability = "mm:MemoryRecall"
[tbox.functions]
"cognition.memory_recall" = { source = "handlers::memory_recall" }
[monad.operations]
name = "cognition"
arity = 1
```

CLI: `mm-cli memory add --kind episodic --content @file` · `memory get <ulid>` ·
`memory recall "query" --k 8 [--kinds semantic,episodic]` · `memory consolidate --window 7d` ·
`memory forget --now <ts> --dry-run` · `memory stats --reconcile` · `memory eval --gold …` ·
`memory verify`.

## 6. External references copied in and integration

Copy the needed source into `vendor/…` as workspace members (never submodules/path deps); record
`mmc:copiedFrom` (repo, revision, license) and a `vendor/<origin>/COPYING.md` in the Phase 2 scan.

| Reference | Copied from | Integration |
|---|---|---|
| `memory-ir` (`MemoryEngine`, `MemoryKind`, `Consolidation`, `Retrieval`, `ReasoningTrajectory`) | rust_symbolic | Reference for `mm-memory`'s model and consolidation semantics; adapted behind `MemoryStore`/`MemoryEngine`; RDF shapes reused. |
| `kg-embed` (`Embedder`, `BgeSmallEmbedder`, BGE-small ONNX, `PackedSynsetIndex`, `SynsetSearch`, L2-normalized dot-product math) | rust_extract | Copied into `vendor/rust_extract/kg-embed`; `mm-memory::index` reuses the packed-index artifact format; embeddings are L2-normalized so cosine is a dot product. |
| `temporal-store` / `temporal-ir` (`TimeInterval`, bitemporal validity, immutable record versions) | rust_symbolic | Validity intervals and append-only versioning semantics; `Memory::validity` uses `TimeInterval`. |
| `rdf-codec` (`RdfContext`, `Namespace`, `ToRdf`/`FromRdf`, `new_ulid`, canonical serialization) | rust_symbolic | RDF encoding of memory records; canonical hashing for `content_hash`. |
| `kg-llm` `LlmClient` via `mm-llm` | rust_extract | Consolidation/compression prompts (schema-validated, cache/replay). |

## 7. Step-by-step implementation tasks

1. Add `crates/mm-memory` to the workspace; `#![forbid(unsafe_code)]`, errors in `error.rs`; register
   the crate and module with `mm-cli codex scan`.
2. Implement `model.rs` (`Memory`, `MemoryKind`, `RetrievalCue`, `Mistake`, `NearMiss`) with validation
   (confidence/importance in `[0,1]`, non-empty content, `valid_until > valid_from`).
3. Implement `utility.rs`: `memory_utility` (impact × probability × reliability / storage_cost) and
   `salience` (novelty + goal relevance + emotional importance + prediction error + social importance).
4. Add migration `0005_memory.sql`; implement `SqliteMemoryStore` over the Phase 1 `Tabular` trait; FTS5
   backfill trigger on insert.
5. Implement `rdf.rs` (`ToRdf`/`FromRdf`) and mirror every write to `/memory`; emit `prov:` links.
6. Implement `index.rs`: load/BGE-embed cues with `kg-embed`, build `data/index/memory.pack`; expose
   `search(query_vec, k)`.
7. Implement `retrieval.rs`: FTS5 keyword candidates → embedding ranking of the union → embedding-only
   fallback when keyword returns nothing; deterministic tie-break by ULID; return `parts` explaining the
   score; record access rows + `trace_id`.
8. Implement `consolidate.rs`: cluster episodes by entity/pattern; LLM proposes a generalization;
   validate against shapes; commit one `mm:SemanticMemory` with `mm:consolidatedFrom` provenance;
   optionally compress to a rule; archive low-value sources only after the target commits.
9. Implement `forgetting.rs`: `forget_cycle` computes utility per active, non-protected memory and
   archives below threshold; hard-refuses (and logs) any attempt to forget protected/developmental
   records or records referenced by an open commitment.
10. Implement `procedural.rs`: skill records (preconditions, procedure, expected outcomes, failure
    modes, required capabilities, proficiency) and habit promotion when success is repeated.
11. Implement `mistake.rs`: `Mistake`/`NearMiss` with `failure_mode`, `root_cause`, `missed_signal`,
    `corrective_rule`, `recurrence_risk`; link to the relevant episode/decision/outcome.
12. Implement reinforcement (`reinforce`): raise confidence/importance with new evidence under bounds.
13. Add `mm-cli memory …` subcommands and `memory verify` (SQLite↔RDF reconciliation).
14. Author `ontology/shapes/memory.ttl`; generate/validate SHACL via `rdf-shacl`.
15. Build `bench/memory/gold.jsonl` (query → expected memory ULIDs) + `thresholds.toml`, and
    `bench/memory/episodes.jsonl` for consolidation; add adversarial duplicates/stale/contradictory items.
16. Wire the module's `cognition.memory_recall` handler and manual.

## 8. Detailed logging requirements

All records go through `mm-log` with the global fields (`ts`, `level`, `target`, `event`, `msg`,
`trace_id`, `span_id`) plus:

| Event code | Required fields |
|---|---|
| `memory.add` | `memory_id`, `kind`, `content_hash`, `confidence`, `importance`, `protected`, `provenance`, `latency_ms` |
| `memory.retrieve` | `query_hash`, `k`, `kinds`, `keyword_hits`, `embedding_fallback` (bool), `result_ids`, `scores`, `latency_ms` |
| `memory.access` | `memory_id`, `score`, `query_hash`, `trace_id` |
| `memory.consolidate` | `source_ids`, `target_id`, `method`, `provenance_closure_ok` (bool), `archived_ids` |
| `memory.forget` | `memory_id`, `utility`, `impact`, `retrieval_probability`, `reliability`, `storage_cost`, `action` (`archive`/`skip`), `reason` |
| `memory.protected_refusal` | `memory_id`, `kind`, `attempted_action`, `blocking_reference` |
| `memory.mistake.create` | `mistake_id`, `failure_mode`, `root_cause`, `corrective_rule`, `recurrence_risk` |
| `memory.near_miss.create` | `near_miss_id`, `failure_mode`, `missed_signal` |
| `memory.index.rebuild` | `count`, `embedder_version`, `pack_hash`, `latency_ms` |
| `memory.verify` | `sqlite_rows`, `rdf_triples`, `reconciled` (bool), `drift_ids` |

Errors carry the full cause chain. Free text (`content`) is length-bounded and content-hashed in logs;
never emit raw secrets/PII.

## 9. Testing plan

- **Unit** (`tests/utility.rs`): utility formula golden values; monotonic in `impact`,
  `retrieval_probability`, `reliability`; strictly decreasing in `storage_cost`; `storage_cost = 0` is an
  error (no division by zero).
- **Property** (`proptest`): `put`→`get` round-trip preserves all fields; consolidation provenance is
  closed (every target’s `consolidatedFrom` resolves, transitively, to committed sources); `forget_cycle`
  never removes `protected = 1` or `kind = developmental` rows; RDF encode/decode round-trips canonically.
- **Golden / replay**: retrieval results and their `parts` match `insta` snapshots; a recorded session
  replays with **zero** provider calls (consolidation via `CachedLlmClient`).
- **Retrieval eval** (`memory eval`): precision@k / recall@k on `bench/memory/gold.jsonl`; thresholds from
  `bench/memory/thresholds.toml` (initial floor: precision@5 ≥ 0.80, recall@5 ≥ 0.80). The command exits
  non-zero below threshold.
- **Ontology**: `/memory` graph SHACL-validates; unsupported constructs are hard errors.
- **Adversarial**: duplicate/near-duplicate, stale (validity elapsed), contradictory, orphan-entity, and
  missing-provenance fixtures are flagged, never silently accepted.
- **End-to-end**: `mm-cli memory add|recall|consolidate|forget|stats` against a temp store.

## 10. Pass gate

Run as plain commands; all must exit 0 and earlier phases must still pass.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p mm-cli -- doctor
cargo run -p mm-cli -- graph validate --graph memory            # 0 SHACL violations
cargo run -p mm-cli -- memory eval --gold bench/memory/gold.jsonl --k 5 \
        --thresholds bench/memory/thresholds.toml                # precision@5/recall@5 >= thresholds
cargo run -p mm-cli -- memory consolidate --window 30d --input bench/memory/episodes.jsonl
cargo run -p mm-cli -- memory stats --reconcile                  # SQLite rows == /memory triples, exit 0
cargo run -p mm-cli -- memory forget --now 2026-10-08T00:00:00Z --dry-run   # protected untouched
cargo run -p mm-cli -- memory verify
cargo run -p mm-cli -- logs verify
```

Objective criteria:
- Retrieval precision@5 and recall@5 meet or exceed the thresholds in `bench/memory/thresholds.toml`.
- Consolidating the synthetic episode stream **reduces memory count** while a SPARQL query can
  reconstruct every source episode through `mm:consolidatedFrom`/`prov:wasDerivedFrom`.
- `forget_cycle` removes only memories with utility below threshold; all `protected` and `developmental`
  records remain, and the refusal is logged.
- A seeded mistake yields a retrievable `Mistake` record naming a `corrective_rule`.
- `memory stats --reconcile` matches SQLite rows to `/memory` triples exactly (no drift).
- `mm-cli graph validate --graph memory` reports 0 SHACL violations.
- `mm-cli logs verify` passes (schema, audit completeness, gapless sequence, ULID correlation, redaction).

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Embedding model unavailable / drift | Embeddings only **rank/fallback**; keyword (FTS5) is the primary path, matching `kg-resolve`; model version pinned in `memory.index.rebuild` log and index header. |
| Memory growth without bound | `forget_cycle` + consolidation compaction; utility formula + retention; immutable developmental ledger protected. |
| Forgetting destroys needed continuity | `protected` flag + reference checks (open commitments) + archive-then-delete; refusals logged. |
| Consolidation loses provenance | `mm:consolidatedFrom` + PROV links required by SHACL; provenance-closure property test. |
| Nondeterministic retrieval | Deterministic scoring and ULID tie-break; golden snapshots; replay makes zero provider calls. |
| LLM compression hallucination | Generalizations validated against shapes and entities; only commit with resolvable provenance. |
| Storage cost dominates utility | `storage_cost` measured as bytes + index entries; formula test prevents division-by-zero. |

## 12. Design traceability

- `design/digital_mind_design1.md` §32 (memory architecture, ten classes), §33 (deliberate forgetting,
  memory utility), §34 (mistake and near-miss memory), §35 (habits/procedural memory), §22 (dependency
  and uncertainty propagation), §23 (contradiction detection), §30 (attention/salience), §51 (world
  model separation), §58 (code/data co-evolution); §87–90 (ontology/runtime/ops).
- `design/congitive_elements2.md` §11 (memory maintenance: store/retrieve/link/merge/compress/archive/
  forget/reinforce/reconstruct; episodes→patterns→generalizations→knowledge with provenance).
- `design/congitive_elements3.md` §15–§17 (skill/procedural memory, habit formation, attention+salience).
- `design/state_structure1.md` (generated vs persistent vs verified state; episodes→LLM consolidation→
  semantic memories; store only what changes future behavior).
- Parent plan `design/planning/implementation_plan1.md` §5 (storage topology), §6 (ontology/SHACL),
  §7 (copy-in integration), §9 (testing), §10 (logging), §13 Phase 5.
