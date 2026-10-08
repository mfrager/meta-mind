# Phase 5 — Persistent Memory Architecture (Extended Build Plan)

> Extended from: `design/planning/phase_05_memory_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 5 · Codename Metamind (`mm`)

This revision keeps the same scope and gate as the Phase 5 plan but locks in the concrete memory
mechanisms — a **tiered** store, **hybrid retrieval**, a **consolidation tree**, and **time-decayed
forgetting** — borrowing only from projects whose code is public.

---

## 0. Research foundation (code-available)

Every idea below is borrowed from a project with source available. Only code-backed ideas are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| Agentic memory organization (notes + dynamic links) | A-MEM — https://github.com/agiresearch/a-mem | Note construction, dynamic link generation, memory evolution | `mm-memory` writes a `Memory` note plus `memory_links` edges; links are generated on write and reinforced on access |
| Bi-temporal knowledge-graph memory | Graphiti (Zep) — https://github.com/getzep/graphiti | Separate *valid time* from *ingestion time*; incremental entity/edge extraction | Every memory carries `valid_from/until` **and** `recorded_at/recorded_ulid`; the graph is append-only |
| Extraction + consolidation (ADD/UPDATE/DELETE) | Mem0 — https://github.com/mem0ai/mem0 | Pipeline that extracts candidate memories and reconciles duplicates/updates | `Consolidator` classifies each new candidate as add/merge/supersede; merges keep provenance |
| Multi-hop graph retrieval (Personalized PageRank) | HippoRAG — https://github.com/OSU-NLP-Group/HippoRAG | PPR over an entity graph to retrieve connected memories across hops | `entity_graph` + `ppr_scores()` seed from query entities and expand hits beyond lexical matches |
| Hierarchical community summaries | GraphRAG — https://github.com/microsoft/graphrag | Community detection + summaries for global/"themes" queries | `memory_communities` holds cluster summaries; global recall uses them before drilling into members |
| Recursive abstractive summary tree | RAPTOR — https://github.com/parthsarthi03/raptor | Recursive clustering + summarization into a tree | `memory_summaries` is a tree built by `Consolidator`; summaries are themselves retrievable memories |
| Time-decayed retention (forgetting curve) | MemoryBank — https://github.com/zhongwanjun/MemoryBank-SiliconFriend | Ebbinghaus-style decay modulated by importance and rehearsal | `retention_score()` decays with age, rises with access/importance; `forget_cycle` archives below threshold |
| Evaluation methodology + task categories | LongMemEval — https://github.com/xiaowu0162/LongMemEval · LoCoMo — https://github.com/snap-research/locomo · memory-benchmarks — https://github.com/mem0ai/memory-benchmarks · survey index — https://github.com/DEEP-PolyU/Awesome-GraphMemory | Task categories (multi-session, temporal, knowledge-update, abstention) and scoring | `bench/memory/` uses these categories with a local gold set and precision@k/recall@k thresholds |

---

## 1. Objective and scope

Deliver the being's **long-term memory organ**: the ten memory classes of design §32 as typed,
provenance-bearing traces, with deterministic hybrid retrieval, episodic→semantic→summary
consolidation, deliberate time-decayed forgetting, and mistake/near-miss retention.

**In scope:** `mm-memory` crate (model, store, utility, retrieval, index, consolidation, forgetting,
mistake/near-miss, procedural records); SQLite tables + packed vector index + entity graph; mirrored
`/memory` RDF graph + SHACL; `modules/cognition/memory-recall`; `bench/memory/`; `mm-cli memory …`.

**Out of scope (later phases):** world-model facts (Phase 6), library cases (Phase 7), prediction
calibration (Phase 11), Pi/self-modification (Phase 11). Phase 5 stores prediction- and mistake-memory
**records**; it does not yet evaluate them.

**Invariant:** the **immutable developmental ledger is never deleted or rewritten** — forgetting may
only archive non-protected records, and every archival is logged with its reason.

---

## 2. Architecture

```
                 ┌───────────────────────── CORE (always in context, bounded) ─────────────────────────┐
 write ─▶ extract ─▶ consolidate? ─▶ Memory ─▶ content_hash ─▶ SQLite rows ─┐                          │
                                        │                                     │                          │
                                        ├─▶ cues (FTS5 + embeddings) ─▶ data/index/memory.pack          │
                                        ├─▶ entities ─▶ entity_graph (edges + weights)                   │
                                        └─▶ /memory RDF graph (rdf-codec, canonical)                      │
                 └──────────────────────────────────────────────────────────────────────────────────────┘

 recall(query) ─▶ [1] lexical  FTS5/BM25 candidates
                ─▶ [2] vector    BGE-small kNN over memory.pack        (union of [1]∪[2])
                ─▶ [3] graph     entity PPR expansion (HippoRAG-style)  (1–2 hops from seed entities)
                ─▶ [4] rerank    hybrid_score = lexical + vector + graph + recency + importance
                ─▶ [5] tiers     core ▸ recall ▸ archival; global queries hit community summaries
                ─▶ RecallHit{ memory, score, parts }  ─▶ record_access (reinforces retention)
```

**Tiered model.** *Core* = a bounded, always-in-context set (identity/current-task-relevant) selected
by salience. *Recall* = everything retrievable by hybrid search. *Archival* = low-retention records kept
for provenance but not normally surfaced. Tiers are *views* over one store, not separate stores.

**Consolidation pipeline** (RAPTOR + GraphRAG + Mem0):
`raw episodes → repeated pattern → semantic memory → community summary → summary tree (root)`.
Each step writes a new record with `mm:consolidatedFrom` provenance; sources are archived only after
the target commits, and source lineage stays queryable forever.

**Forgetting** (MemoryBank): `retention_score(m, now)` decays with age, is boosted by access and
importance, and is compared to `thresholds.toml`. Below threshold → `archived`; protected/developmental
records and records referenced by an open commitment are hard-refused and logged.

**Bi-temporality** (Graphiti): `valid_from/valid_until` (world time) is independent of
`recorded_at/recorded_ulid` (system time), so we can answer "what did I know, and when did I know it".

---

## 3. Deliverables and workspace layout

```
~/Build/metamind/
├── crates/mm-memory/
│   ├── Cargo.toml
│   ├── src/lib.rs                 # re-exports
│   ├── src/error.rs               # MemoryError
│   ├── src/model.rs               # Memory, MemoryKind, Tier, RetrievalCue, Mistake, NearMiss, SummaryNode
│   ├── src/utility.rs             # memory_utility, retention_score, salience
│   ├── src/store.rs               # MemoryStore trait + SqliteMemoryStore
│   ├── src/graph.rs               # entity_graph + ppr_scores()
│   ├── src/index.rs               # packed vector index (kg-embed convention)
│   ├── src/retrieval.rs           # hybrid recall (lexical ▸ vector ▸ graph ▸ rerank)
│   ├── src/consolidate.rs         # Consolidator: add/merge/supersede + summary tree + communities
│   ├── src/forgetting.rs          # forget_cycle + ledger/reference protection
│   ├── src/mistake.rs             # Mistake / NearMiss
│   ├── src/procedural.rs          # procedural memory / skill records
│   ├── src/rdf.rs                 # ToRdf/FromRdf for the /memory graph
│   └── tests/{utility.rs,retrieval.rs,graph.rs,consolidation.rs,forgetting.rs,rdf_roundtrip.rs}
├── crates/mm-store-sqlite/migrations/0005_memory.sql
├── ontology/shapes/memory.ttl
├── modules/cognition/memory-recall/{plugin.toml,src/lib.rs,manual/module.md,tests/}
├── bench/memory/{gold.jsonl,episodes.jsonl,thresholds.toml,adversarial/}
└── data/index/memory.pack          # generated, gitignored
```

`mm-cli`: `memory add|get|recall|consolidate|forget|stats|eval|verify`.

---

## 4. Detailed specifications

### 4.1 SQLite (`0005_memory.sql`)

All ids `TEXT(26)` ULIDs; times are integer ns (Phase 1 `Timestamp`).

```sql
PRAGMA journal_mode=WAL;

CREATE TABLE memories (
  id            TEXT(26) PRIMARY KEY,
  kind          TEXT NOT NULL CHECK (kind IN
                  ('episodic','semantic','procedural','working','autobiographical',
                   'relational','prediction','mistake','near_miss','developmental')),
  tier          TEXT NOT NULL DEFAULT 'recall' CHECK (tier IN ('core','recall','archival')),
  content       TEXT NOT NULL,
  source_ulid   TEXT,
  confidence    REAL NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  importance    REAL NOT NULL CHECK (importance BETWEEN 0 AND 1),
  utility       REAL,                                  -- last computed
  retention     REAL,                                  -- last computed retention_score
  valid_from    INTEGER NOT NULL,
  valid_until   INTEGER,                               -- NULL = open
  recorded_at   INTEGER NOT NULL,                      -- system time
  recorded_ulid TEXT NOT NULL REFERENCES events(ulid),
  provenance    TEXT NOT NULL,                         -- PROV node ULID
  protected     INTEGER NOT NULL DEFAULT 0,            -- 1 = never forgotten
  status        TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','archived')),
  content_hash  TEXT NOT NULL                          -- sha256(canonical content)
);

CREATE TABLE memory_cues     (id TEXT(26) PRIMARY KEY, memory_id TEXT NOT NULL REFERENCES memories(id),
                              cue TEXT NOT NULL, cue_kind TEXT NOT NULL);   -- Keyword|Embedding|Entity
CREATE TABLE memory_entities (id TEXT(26) PRIMARY KEY, memory_id TEXT NOT NULL, entity_ulid TEXT NOT NULL, role TEXT);
CREATE TABLE memory_links    (id TEXT(26) PRIMARY KEY, from_id TEXT NOT NULL, to_id TEXT NOT NULL,
                              relation TEXT NOT NULL, weight REAL NOT NULL, created_ulid TEXT NOT NULL);
CREATE TABLE memory_access   (id TEXT(26) PRIMARY KEY, memory_id TEXT NOT NULL, accessed_at INTEGER NOT NULL,
                              query_hash TEXT NOT NULL, score REAL NOT NULL, trace_id TEXT NOT NULL);
CREATE TABLE memory_consolidations(id TEXT(26) PRIMARY KEY, source_ids TEXT NOT NULL /* JSON */,
                              target_id TEXT NOT NULL, method TEXT NOT NULL, created_ulid TEXT NOT NULL);
CREATE TABLE memory_summaries(id TEXT(26) PRIMARY KEY, memory_id TEXT NOT NULL, parent_id TEXT,   -- RAPTOR tree
                              level INTEGER NOT NULL, member_ids TEXT NOT NULL /* JSON */, created_ulid TEXT NOT NULL);
CREATE TABLE memory_communities(id TEXT(26) PRIMARY KEY, memory_id TEXT NOT NULL, label TEXT NOT NULL,
                              member_ids TEXT NOT NULL /* JSON */, modularity REAL, created_ulid TEXT NOT NULL);
CREATE TABLE entity_edges    (id TEXT(26) PRIMARY KEY, from_entity TEXT NOT NULL, to_entity TEXT NOT NULL,
                              relation TEXT NOT NULL, weight REAL NOT NULL, valid_from INTEGER NOT NULL,
                              valid_until INTEGER);                                  -- bi-temporal

CREATE TABLE mistakes (
  id TEXT(26) PRIMARY KEY, situation_ulid TEXT, decision_ulid TEXT, outcome_ulid TEXT,
  failure_mode TEXT NOT NULL, root_cause_ulid TEXT, missed_signal TEXT /* JSON */,
  corrective_rule_ulid TEXT, recurrence_risk REAL NOT NULL CHECK (recurrence_risk BETWEEN 0 AND 1),
  created_ulid TEXT NOT NULL);

CREATE VIRTUAL TABLE memory_fts USING fts5(content, content='memories', content_rowid='rowid');
CREATE INDEX idx_memories_kind   ON memories(kind, status);
CREATE INDEX idx_memories_tier   ON memories(tier, status);
CREATE INDEX idx_memories_valid  ON memories(valid_from, valid_until);
CREATE INDEX idx_entity_edges    ON entity_edges(from_entity, to_entity);
```

### 4.2 RDF (`/memory` = `https://metamind.dev/graph/memory`)

Classes: `mm:Memory` + the ten subclasses, `mm:RetrievalCue`, `mm:Consolidation`, `mm:SummaryNode`,
`mm:Community`, `mm:Mistake`, `mm:NearMiss`, `mm:CorrectiveRule`.
Predicates: `mm:memoryKind, mm:tier, mm:content, mm:source, mm:confidence, mm:importance, mm:utility,
mm:retention, mm:validFrom, mm:validUntil, mm:recordedAt, mm:protected, mm:retrievalCue,
mm:relatedEntity, mm:derivedFrom, mm:consolidatedFrom, mm:consolidatedInto, mm:summarizes, mm:accessedBy,
mm:failureMode, mm:rootCause, mm:missedSignal, mm:correctiveRule, mm:recurrenceRisk, mm:supports,
mm:contradicts, mm:similarTo`, plus `prov:wasDerivedFrom` / `prov:wasGeneratedBy`.
Instance IRIs are `https://metamind.dev/data/{ulid}` via `rdf-codec` (canonical, deterministic).

`ontology/shapes/memory.ttl` (SHACL): every `mm:Memory` has exactly one `mm:memoryKind`, `mm:tier`,
`mm:content`, `mm:confidence∈[0,1]`, `mm:importance∈[0,1]`, `mm:recordedAt`, and a source or explicit
`mm:unknownSource`; every `mm:Consolidation` has ≥1 `mm:consolidatedFrom` that resolves; every
`mm:SummaryNode` has ≥1 `mm:summarizes`; `mm:DevelopmentalMemory` requires `mm:protected true`. Any
unsupported construct is a hard error, never a silent skip.

### 4.3 Rust interfaces

```rust
pub enum MemoryKind { Episodic, Semantic, Procedural, Working, Autobiographical,
                      Relational, Prediction, Mistake, NearMiss, Developmental }
pub enum Tier { Core, Recall, Archival }

pub struct Memory {
    pub id: Ulid, pub kind: MemoryKind, pub tier: Tier, pub content: String,
    pub source: Option<Ulid>, pub confidence: f32, pub importance: f32,
    pub validity: TimeInterval, pub recorded_at: Timestamp, pub provenance: Ulid,
    pub entities: Vec<Ulid>, pub cues: Vec<RetrievalCue>, pub protected: bool,
}

pub struct UtilityInputs { pub future_behavior_impact: f32, pub retrieval_probability: f32,
                           pub reliability: f32, pub storage_cost: f64 }
pub fn memory_utility(i: &UtilityInputs) -> Result<f64, MemoryError>;   // impact*prob*reliability/cost

/// Ebbinghaus-style, modulated by importance and rehearsal. Deterministic; `age` and `half_life` in ns.
pub fn retention_score(age_ns: u64, half_life_ns: u64, importance: f32, access_count: u32) -> f64;

/// Partly-normalized hybrid score; `parts` lets callers audit the contribution of each channel.
pub struct ScorePart { pub channel: Channel, pub raw: f64, pub weight: f64, pub contribution: f64 }
pub enum Channel { Lexical, Vector, Graph, Recency, Importance }
pub struct RecallQuery { pub text: String, pub k: usize, pub kinds: Vec<MemoryKind>, pub now: Timestamp }
pub struct RecallHit  { pub memory: Memory, pub score: f64, pub parts: Vec<ScorePart> }

#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    async fn put(&self, m: &Memory) -> Result<Ulid, MemoryError>;
    async fn get(&self, id: &Ulid) -> Result<Option<Memory>, MemoryError>;
    async fn link(&self, from: Ulid, to: Ulid, relation: LinkKind, weight: f32) -> Result<(), MemoryError>;
    async fn record_access(&self, id: Ulid, score: f64, trace: Ulid) -> Result<(), MemoryError>;
    async fn archive(&self, id: Ulid) -> Result<(), MemoryError>;
}

pub struct MemoryEngine { /* store, embedder, index, graph, llm, sink */ }
impl MemoryEngine {
    pub async fn remember(&self, m: Memory) -> Result<Ulid, MemoryError>;
    pub async fn recall(&self, q: RecallQuery) -> Result<Vec<RecallHit>, MemoryError>;
    pub async fn reinforce(&self, id: Ulid, evidence: f32) -> Result<(), MemoryError>;
    pub async fn consolidate(&self, window: TimeInterval) -> Result<Vec<Consolidation>, MemoryError>;
    pub async fn forget_cycle(&self, now: Timestamp) -> Result<ForgetReport, MemoryError>;
    pub async fn record_mistake(&self, m: Mistake) -> Result<Ulid, MemoryError>;
}
```

**Retrieval scoring (deterministic):**
`hybrid = 0.35·norm_bm25 + 0.30·cosine + 0.20·ppr + 0.10·recency + 0.05·importance`, then multiplied by
`(0.5 + 0.5·confidence)`. Ties break on ULID. Weights are config-driven (`thresholds.toml`) and pinned in
the retrieval snapshot so replay is exact.

**CLI:** `memory add --kind episodic --content @file` · `memory get <ulid>` ·
`memory recall "query" --k 8 [--kinds semantic,episodic] [--tier recall]` · `memory consolidate --window 7d` ·
`memory forget --now <ts> --dry-run` · `memory stats --reconcile` · `memory eval --gold …` · `memory verify`.

---

## 5. Build sequence

1. Add `crates/mm-memory` (`#![forbid(unsafe_code)]`), `error.rs`; register with `mm-cli codex scan`.
2. `model.rs`: `Memory`/`MemoryKind`/`Tier`/`RetrievalCue`/`Mistake`/`NearMiss` + validation.
3. `utility.rs`: `memory_utility` (golden + monotonicity), `retention_score` (decay/rehearsal), `salience`.
4. `0005_memory.sql`; `SqliteMemoryStore` over the Phase 1 `Tabular` trait; FTS5 sync triggers.
5. `rdf.rs`: `ToRdf`/`FromRdf`; mirror every write to `/memory`; emit `prov:` links.
6. `index.rs`: embed cues with `kg-embed` (BGE-small, L2-normalized), build `data/index/memory.pack`.
7. `graph.rs`: maintain `entity_edges`; implement `ppr_scores(seed_entities, hops=2)`.
8. `retrieval.rs`: lexical ▸ vector ▸ graph ▸ rerank with `parts`; record `memory_access` + `trace_id`.
9. `consolidate.rs`: candidate reconciliation (add/merge/supersede), episode→semantic, community
   summaries, RAPTOR-style summary tree; commit targets before archiving sources.
10. `forgetting.rs`: `forget_cycle` by `retention_score`; refuse protected/developmental/committed.
11. `procedural.rs`: skill records (preconditions, procedure, outcomes, failure modes, proficiency) +
    habit promotion on repeated success.
12. `mistake.rs`: mistake/near-miss records linked to episode/decision/outcome.
13. `mm-cli memory …` + `memory verify` (SQLite↔RDF reconciliation).
14. `ontology/shapes/memory.ttl`; validate via `rdf-shacl`.
15. `bench/memory/`: gold set across LongMemEval/LoCoMo categories, episodes for consolidation,
    adversarial duplicates/stale/contradictory, `thresholds.toml`.
16. Wire `cognition.memory_recall` handler + module manual.

---

## 6. Logging and observability

All records via `mm-log` with global fields (`ts, level, target, event, msg, trace_id, span_id`) plus:

| Event code | Required fields |
|---|---|
| `memory.add` | `memory_id, kind, tier, content_hash, confidence, importance, protected, provenance` |
| `memory.recall` | `query_hash, k, kinds, lexical_n, vector_n, graph_n, fallback(bool), result_ids, scores, latency_ms` |
| `memory.rerank` | `result_ids, weights, parts_summary` |
| `memory.access` | `memory_id, score, query_hash, trace_id` |
| `memory.consolidate` | `source_ids, target_id, method, provenance_closure_ok(bool), archived_ids` |
| `memory.summary.build` | `level, parent_id, member_ids, tree_depth` |
| `memory.forget` | `memory_id, utility, retention, action(archive\|skip), reason` |
| `memory.protected_refusal` | `memory_id, kind, attempted_action, blocking_reference` |
| `memory.mistake.create` | `mistake_id, failure_mode, root_cause, corrective_rule, recurrence_risk` |
| `memory.near_miss.create` | `near_miss_id, failure_mode, missed_signal` |
| `memory.index.rebuild` | `count, embedder_version, pack_hash, latency_ms` |
| `memory.verify` | `sqlite_rows, rdf_triples, reconciled(bool), drift_ids` |

Free text (`content`) is length-bounded and content-hashed in logs; no raw secrets/PII. State changes are
audit records (immutable, gapless) and `mm-cli logs verify` checks schema, audit completeness, ULID
correlation, and the redaction suite.

---

## 7. Testing

- **Unit** (`tests/utility.rs`): `memory_utility` golden values; monotone in impact/retrieval/reliability;
  strictly decreasing in storage_cost; `storage_cost = 0` is an error. `retention_score` decays
  monotonically and rises with importance/access.
- **Property** (`proptest`): put→get round-trip preserves fields; consolidation provenance is transitively
  closed; `forget_cycle` never removes `protected = 1` or `developmental`; RDF round-trips canonically.
- **Graph** (`tests/graph.rs`): PPR is deterministic for a fixed graph/seed; 1–2 hop expansion finds
  connected memories not present in the lexical candidate set.
- **Golden / replay**: retrieval `parts` match `insta` snapshots; a recorded consolidation replays with
  **zero** provider calls (`CachedLlmClient`).
- **Retrieval eval** (`memory eval`): precision@k / recall@k over `bench/memory/gold.jsonl`, thresholds
  from `bench/memory/thresholds.toml` (floor: precision@5 ≥ 0.80, recall@5 ≥ 0.80); non-zero exit below.
- **Ontology**: `/memory` SHACL-validates; unsupported constructs are hard errors.
- **Adversarial**: duplicate/near-duplicate, stale, contradictory, orphan-entity, missing-provenance
  fixtures are flagged, never silently accepted.
- **E2E**: `mm-cli memory add|recall|consolidate|forget|stats` against a temp store.

---

## 8. Pass gate

Run as plain commands; all must exit 0 and earlier phases must still pass.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p mm-cli -- doctor
cargo run -p mm-cli -- graph validate --graph memory                 # 0 SHACL violations
cargo run -p mm-cli -- memory eval --gold bench/memory/gold.jsonl --k 5 \
        --thresholds bench/memory/thresholds.toml                     # precision@5/recall@5 >= thresholds
cargo run -p mm-cli -- memory consolidate --window 30d --input bench/memory/episodes.jsonl
cargo run -p mm-cli -- memory stats --reconcile                       # SQLite rows == /memory triples
cargo run -p mm-cli -- memory forget --now 2026-10-08T00:00:00Z --dry-run   # protected untouched
cargo run -p mm-cli -- memory verify
cargo run -p mm-cli -- logs verify
```

Objective criteria:
- Retrieval precision@5 and recall@5 meet or exceed `bench/memory/thresholds.toml`.
- Consolidating the episode stream **reduces memory count** while SPARQL can reconstruct every source
  episode through `mm:consolidatedFrom`/`prov:wasDerivedFrom`; the summary tree is well-formed.
- `forget_cycle` archives only records below the retention threshold; all `protected` and `developmental`
  records remain and each refusal is logged.
- A seeded mistake yields a retrievable `Mistake` naming a `corrective_rule`.
- `memory stats --reconcile` matches SQLite rows to `/memory` triples exactly.
- `graph validate --graph memory` reports 0 SHACL violations.
- `mm-cli logs verify` passes (schema, audit completeness, gapless sequence, ULID correlation, redaction).

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Embedding model unavailable / drifted | Keyword (FTS5) is primary; embeddings only rank/expand; model version pinned in `memory.index.rebuild` and the index header |
| Unbounded growth | Retention decay + consolidation compaction; archival (never destructive); protected ledger |
| Forgetting destroys continuity | `protected` flag + open-commitment/reference checks + archive-before-delete; refusals logged |
| Consolidation loses provenance | `mm:consolidatedFrom`/PROV required by SHACL; provenance-closure property test |
| Graph retrieval blows up | PPR capped at 2 hops with a node budget; deterministic damping/tie-break |
| Retrieval poisoning (adversarial writes) | Provenance + confidence weighting; adversarial fixtures; no memory enters `/memory` without a source |
| LLM compression hallucination | Generalizations validated against shapes + entities; commit only with resolvable provenance |
| Nondeterministic retrieval | Fixed weights, ULID tie-break, golden snapshots, replay with zero provider calls |

---

## 10. References (code-available)

- A-MEM — https://github.com/agiresearch/a-mem
- Graphiti (Zep) — https://github.com/getzep/graphiti
- Mem0 — https://github.com/mem0ai/mem0
- HippoRAG — https://github.com/OSU-NLP-Group/HippoRAG
- GraphRAG — https://github.com/microsoft/graphrag
- RAPTOR — https://github.com/parthsarthi03/raptor
- MemoryBank — https://github.com/zhongwanjun/MemoryBank-SiliconFriend
- LongMemEval — https://github.com/xiaowu0162/LongMemEval
- LoCoMo — https://github.com/snap-research/locomo
- memory-benchmarks — https://github.com/mem0ai/memory-benchmarks
- Awesome-GraphMemory (survey index) — https://github.com/DEEP-PolyU/Awesome-GraphMemory
