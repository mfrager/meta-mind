# Phase 7 Build Plan — Cognitive library, conceptual frames, and the policy genome

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 7 · Codename Metamind (`mm`)
> Goal: turn reusable ways of thinking — doctrines, principles, heuristics, techniques, patterns, cases,
> anti-patterns, skills, policies — plus temporary conceptual frames and a versioned policy genome into
> persistent, queryable, schema-validated objects, so the being stops rediscovering reasoning and starts
> reusing it (design §13–17, §44–45, §69).

---

## 1. Objective and scope

Deliver the **Cognitive Library** organ: a persistent, versioned repertoire of library entries stored in
the Oxigraph `/library` named graph and indexed in SQLite, a **conceptual-frame** subsystem that composes
and instantiates temporary frames, and a **policy genome** with immutable versioning, parentage, and
fitness records.

In scope:
- `mm-library` crate with the eleven entry types and their RDF codecs.
- SHACL shapes for every class, enforced as a hard commit gate (no coercion).
- An LLM extraction pipeline (the copied-in `kg-extract` + `kg-llm` + `kg-validate` pattern) that
  populates a seed corpus and validates before commit.
- Multi-frame composition, frame inheritance, frame instances bound to episodes.
- The policy genome: `id, version, scope, activation, behavior, evidence, fitness, confidence, parent`;
  no policy is ever mutated in place.
- Case records (successful / failed / unusual / edge) with `analogy-ir`-backed retrieval and the
  case-utility model.
- Applicability queries and the `mm-cli library`/`frame`/`policy` operator surface.

Out of scope (later phases): metacognitive program compilation (Phase 8), the sanity firewall (Phase 9),
self-engineering of the library (Phase 11). Phase 7 only *stores and selects*; it does not decide episodes.

---

## 2. Prerequisites and dependencies

- **Phase 1**: `mm-core` (ULID/IRI funnels), `mm-store-graph` (Oxigraph actor, named graphs,
  `graph validate`), `mm-store-sqlite` (sqlx + migrations), `mm-eventlog`, `mm-log`, `rdf-codec`,
  `ontology/mm.ttl` loader.
- **Phase 2**: `mm-codex` (module registry, `mmc:copiedFrom` provenance, `codex verify`).
- **Phase 3**: `mm-llm` (`LlmClient`, `StructuredOut<T>`, request cache, replay) for seed extraction.
- **Phase 6** (soft): `EpistemicStatus` enum, needed because library `evidence` links carry status. If
  Phase 6 is not yet complete, use the `mm-core` status enum forward-declared in `ontology/mm.ttl`.
- **Foreign references to copy in** (see §6): `analogy-ir`, `evolution-ir`, `learning-ir`, `mechanism-ir`
  (rust_symbolic); `kg-extract`, `kg-llm`, `kg-validate` (rust_extract); `rdf-shacl`.

---

## 3. Deliverables (exact paths)

```
crates/mm-library/
  Cargo.toml
  src/lib.rs                 # re-exports, LibraryManager
  src/entry.rs               # EntryKind + the eleven entry structs
  src/doctrine.rs            # Doctrine, Principle, Heuristic, AntiPattern
  src/technique.rs           # Technique, Pattern, Skill, Evaluation
  src/frame.rs               # Frame, FrameInstance, SlotValue, Script
  src/policy.rs              # Policy, PolicyDelta, FitnessRecord, Scope, ActivationCondition
  src/case_db.rs             # Case, CaseOutcome, adaptation over analogy-ir
  src/applicability.rs       # ApplicabilityState, applicability(), case_utility()
  src/seed.rs                # LLM extraction pipeline + JSONL schemas
  src/rdf.rs                 # ToRdf/FromRdf for every entry type (rdf-codec)
  src/validate.rs            # SHACL wrapper (rdf-shacl) used as the commit gate
  src/store.rs               # SQLite index + /library graph writes (actor handle)
  src/prompts/library_extract.md   # extraction prompt template
  tests/entry_roundtrip.rs
  tests/shacl_gate.rs
  tests/applicability_gold.rs
  tests/policy_versioning.rs
  tests/case_retrieval.rs
  tests/extraction_rejection.rs
crates/mm-store-sqlite/migrations/0007_library.sql
modules/cognition/library-query/        # plugin.toml + src/lib.rs + tests + manual
modules/cognition/frame-activate/       # plugin.toml + src/lib.rs + tests + manual
ontology/library.ttl                    # T-Box additions (mm: classes/relations)
ontology/shapes/library.shacl.ttl       # SHACL shapes (generated + hand-tuned)
ontology/seed/library_seed.ttl          # committed seed corpus (validated)
bench/library/state_uncertain_strategy.json
bench/library/gold_applicability.jsonl
bench/library/extraction/raw.jsonl
bench/library/extraction/malformed.jsonl
bench/library/gold_cases.jsonl
vendor/rust_symbolic/COPYING.md         # origin repo/revision/license for copied crates
vendor/rust_extract/COPYING.md
```

---

## 4. Data model and ontology deltas

**Named graph:** `https://metamind.dev/graph/library` (entries, frames, policy versions, case records).

**Ontology (`ontology/library.ttl`), prefix `mm: = https://metamind.dev/ontology#`:**

- Classes: `mm:LibraryEntry` (superclass), `mm:Doctrine`, `mm:Principle`, `mm:Heuristic`,
  `mm:Technique`, `mm:Pattern`, `mm:Case`, `mm:AntiPattern`, `mm:Skill`, `mm:Policy`, `mm:Frame`,
  `mm:FrameInstance`, `mm:Evaluation`.
- Relations: `mm:applicableWhen`, `mm:inapplicableWhen`, `mm:contraindication`, `mm:example`,
  `mm:counterexample`, `mm:evidence`, `mm:improves`, `mm:replaces`, `mm:derivedFrom`, `mm:testedBy`,
  `mm:activationCondition`, `mm:behavior`, `mm:fitness`, `mm:confidence`, `mm:parentPolicy`,
  `mm:parentFrame`, `mm:frameSlot`, `mm:role`, `mm:affordance`, `mm:typicalAction`, `mm:failureMode`,
  `mm:scriptStep`, `mm:outcomeQuality`, `mm:transferability`, `mm:evidenceQuality`.
- Datatype/label properties: `mm:title`, `mm:purpose`, `mm:version`, `mm:scope`, `mm:strength`,
  `mm:domain`.

**Stable identifiers** (from parent plan §4): entries are named, versioned things, so they use
path-derived URIs, not ULIDs:
- Head entry: `https://metamind.dev/library/{slug}` (e.g. `.../library/technique/inversion`).
- Versioned entry: `https://metamind.dev/library/{slug}@{version}`.
- Frame: `https://metamind.dev/library/frame/{slug}`; policy: `https://metamind.dev/policy/{name}@{version}`.
- Runtime instances (frame instances, retrieval runs, extraction jobs) use ULID IRIs
  `https://metamind.dev/data/{ulid}`.

**SQLite index (`0007_library.sql`):**

```sql
CREATE TABLE library_entries (
  id            TEXT(26) PRIMARY KEY,            -- ULID of the current revision
  iri           TEXT NOT NULL,                   -- head URI
  version_iri   TEXT NOT NULL UNIQUE,            -- URI@version
  kind          TEXT NOT NULL CHECK (kind IN ('Doctrine','Principle','Heuristic','Technique',
                                              'Pattern','Case','AntiPattern','Skill','Policy',
                                              'Frame','Evaluation')),
  slug          TEXT NOT NULL,
  version       INTEGER NOT NULL DEFAULT 1,
  title         TEXT NOT NULL,
  body_ttl      TEXT NOT NULL,
  content_hash  TEXT NOT NULL,                   -- sha256 of canonical Turtle (rdf-codec)
  created_ulid  TEXT NOT NULL,
  valid_from    TEXT NOT NULL,
  valid_until   TEXT,
  UNIQUE (iri, version)
);

CREATE TABLE policies (
  id            TEXT(26) PRIMARY KEY,
  iri           TEXT UNIQUE NOT NULL,
  name          TEXT NOT NULL,
  head_version  INTEGER NOT NULL,
  created_ulid  TEXT NOT NULL
);

CREATE TABLE policy_versions (
  id              TEXT(26) PRIMARY KEY,
  policy_iri      TEXT NOT NULL REFERENCES policies(iri),
  version         INTEGER NOT NULL,
  parent_version  INTEGER,                       -- NULL only for version 1
  scope_ttl       TEXT NOT NULL,
  activation_ttl  TEXT NOT NULL,
  behavior_ttl    TEXT NOT NULL,
  evidence_ttl    TEXT NOT NULL,
  confidence      REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
  created_ulid    TEXT NOT NULL,
  UNIQUE (policy_iri, version)
);

CREATE TABLE policy_fitness (
  id            TEXT(26) PRIMARY KEY,
  policy_iri    TEXT NOT NULL,
  version       INTEGER NOT NULL,
  trials        INTEGER NOT NULL,
  successes     INTEGER NOT NULL,
  mean_utility  REAL,
  updated_ulid  TEXT NOT NULL,
  UNIQUE (policy_iri, version)
);

CREATE TABLE frame_instances (
  id            TEXT(26) PRIMARY KEY,            -- ULID of the instance
  frame_iri     TEXT NOT NULL,
  episode_ulid  TEXT NOT NULL,
  slots_json    TEXT NOT NULL,
  missing_json  TEXT NOT NULL,
  created_ulid  TEXT NOT NULL
);

CREATE TABLE cases (
  id                TEXT(26) PRIMARY KEY,
  iri               TEXT UNIQUE NOT NULL,
  kind              TEXT NOT NULL CHECK (kind IN ('success','failure','unusual','edge')),
  outcome_quality   REAL NOT NULL CHECK (outcome_quality BETWEEN 0.0 AND 1.0),
  transferability   REAL NOT NULL CHECK (transferability BETWEEN 0.0 AND 1.0),
  evidence_quality  REAL NOT NULL CHECK (evidence_quality BETWEEN 0.0 AND 1.0),
  created_ulid      TEXT NOT NULL
);

CREATE TABLE applicability_runs (
  id            TEXT(26) PRIMARY KEY,
  episode_ulid  TEXT NOT NULL,
  state_hash    TEXT NOT NULL,                   -- sha256 of the canonical state
  ranked_json   TEXT NOT NULL,
  created_ulid  TEXT NOT NULL
);
```

**Invariants.** Every entry has purpose, ≥1 activation condition, ≥1 contraindication, ≥1 example, and
evidence; every referenced IRI resolves within `/library` or the being/world graphs; `content_hash` is
stable across re-parses (canonical `rdf-codec` serialization).

---

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

```rust
// crates/mm-library/src/entry.rs
pub enum EntryKind { Doctrine, Principle, Heuristic, Technique, Pattern, Case,
                     AntiPattern, Skill, Policy, Frame, Evaluation }

pub trait LibraryEntry: Send + Sync {
    fn kind(&self) -> EntryKind;
    fn head_iri(&self) -> NamedNode;                       // stable URI
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<(), CodecError>;
    fn from_rdf(g: &Graph, iri: &NamedNode) -> Result<Self, CodecError> where Self: Sized;
    fn referenced_iris(&self) -> Vec<NamedNode>;           // for orphan/existence checks
}

// crates/mm-library/src/technique.rs
pub struct ActivationCondition(pub String);                 // e.g. "high_uncertainty"
pub struct Technique {
    pub iri: NamedNode, pub title: String, pub purpose: String,
    pub domain: Vec<String>, pub applicable_when: Vec<ActivationCondition>,
    pub inapplicable_when: Vec<ActivationCondition>,
    pub contraindications: Vec<String>,
    pub examples: Vec<NamedNode>, pub counterexamples: Vec<NamedNode>,
    pub evidence: Vec<NamedNode>, pub tested_by: Vec<NamedNode>,
    pub confidence: f32, pub domain_fitness: BTreeMap<String, f32>,
}

// crates/mm-library/src/policy.rs
pub enum Scope { Global, Domain(String), Frame(NamedNode), Relationship(NamedNode) }
pub struct ActivationCondition(pub String);
pub struct PolicyBehavior(pub String);                      // typed DSL fragment, not free prose
pub struct FitnessRecord { pub trials: u32, pub successes: u32, pub mean_utility: f32 }
pub struct Policy {
    pub iri: NamedNode, pub version: u32, pub scope: Scope,
    pub activation: ActivationCondition, pub behavior: PolicyBehavior,
    pub evidence: Vec<NamedNode>, pub fitness: FitnessRecord,
    pub confidence: f32, pub parent: Option<NamedNode>,    // parent policy version
}

// crates/mm-library/src/frame.rs
pub enum SlotValue { Known(String), Unknown, Inferred(String) }
pub struct Script(pub Vec<String>);                        // ordered expected steps
pub struct Frame {
    pub iri: NamedNode, pub slug: String, pub parent: Option<NamedNode>,
    pub roles: BTreeMap<String, String>, pub slots: Vec<String>,
    pub relationships: Vec<String>, pub typical_actions: Vec<String>,
    pub affordances: Vec<String>, pub failure_modes: Vec<String>,
    pub scripts: Vec<Script>, pub domain: Vec<String>,
}
pub struct FrameInstance {
    pub ulid: Ulid, pub frame_iri: NamedNode, pub episode: Ulid,
    pub slots: BTreeMap<String, SlotValue>, pub parents: Vec<NamedNode>,
}

// crates/mm-library/src/applicability.rs
pub struct ApplicabilityState {
    pub frames: Vec<NamedNode>, pub goal: NamedNode, pub domain: Vec<String>,
    pub uncertainty: f32, pub stakes: f32, pub history: Vec<NamedNode>,
}
pub struct ApplicabilityScore { pub total: f32, pub activation: f32,
    pub fitness: f32, pub contraindication_penalty: f32 }
pub fn applicability(t: &Technique, s: &ApplicabilityState) -> ApplicabilityScore; // deterministic
pub fn case_utility(c: &Case, similarity: f32) -> f32;     // Sim × Outcome × Transfer × Evidence

// crates/mm-library/src/lib.rs
pub struct LibraryManager { /* graph actor handle, SqlitePool, Arc<dyn LlmClient>, AuditLog */ }
impl LibraryManager {
    pub async fn upsert_entry<E: LibraryEntry>(&self, e: E) -> Result<NamedNode>;
    pub async fn get_entry(&self, iri: &NamedNode) -> Result<Box<dyn LibraryEntry>>;
    pub async fn applicable(&self, s: &ApplicabilityState, top: usize)
        -> Result<Vec<(NamedNode, ApplicabilityScore)>>;
    pub async fn compose_frames(&self, bases: &[NamedNode], episode: Ulid)
        -> Result<FrameInstance>;
    pub async fn retrieve_cases(&self, q: &CaseQuery) -> Result<Vec<(NamedNode, f32)>>;
    pub async fn new_policy_version(&self, name: &str, delta: PolicyDelta,
        evidence: &[NamedNode]) -> Result<Policy>;
    pub async fn record_fitness(&self, iri: &NamedNode, version: u32, o: &Outcome)
        -> Result<FitnessRecord>;
    pub async fn extract_seed(&self, job: ExtractionJob) -> Result<ExtractionReport>;
}
```

**CLI (`mm-cli`):**

```bash
mm-cli library import ontology/seed/library_seed.ttl
mm-cli library validate --graph library          # SHACL; exits non-zero on any violation
mm-cli library orphans --fail-if-any             # referenced IRIs all resolve
mm-cli library list --kind Technique
mm-cli library show https://metamind.dev/library/technique/inversion
mm-cli library applicable --state bench/library/state_uncertain_strategy.json --top 5 --gold bench/library/gold_applicability.jsonl
mm-cli policy history https://metamind.dev/policy/prefer_simpler_solution@2
mm-cli policy fitness https://metamind.dev/policy/prefer_simpler_solution@2
mm-cli frame compose --frames problem_solving,software,debugging,high_stakes --episode <ULID>
mm-cli frame missing <frame-instance-ULID>
```

**Plugin modules** (nexus contract, parent plan §8):

```toml
# modules/cognition/library-query/plugin.toml
[plugin]
name = "mm-library-query"
uri = "https://metamind.dev/code/module/cognition/library-query"
version = "0.1.0"
[metadata]
category = "cognition"
owned_by_phase = 7
capability = "mm:TechniqueSelection"
[tbox.functions]
"cognition.library_applicable" = { source = "handlers::library_applicable" }
"cognition.case_retrieve" = { source = "handlers::case_retrieve" }
[monad.operations]
name = "cognition"
arity = 1
[build]
rust_edition = "2021"
```

---

## 6. External references copied in and integration

External code is **reference only**; the needed source is copied into the workspace and integrated as
Metamind-owned, with provenance recorded (parent plan §7).

| Reference | Copy target | What is integrated | Provenance |
|---|---|---|---|
| `analogy-ir` (`Case`, `Correspondence`, `TransferCandidate`) | `vendor/rust_symbolic/crates/analogy-ir` | `mm-library::case_db` structural similarity + transfer candidates | `vendor/rust_symbolic/COPYING.md`; `mmc:copiedFrom` in `/code` |
| `evolution-ir` (populations, fitness) | `vendor/rust_symbolic/crates/evolution-ir` | policy `FitnessRecord` aggregation and population comparison | same |
| `learning-ir` | `vendor/rust_symbolic/crates/learning-ir` | domain-fitness bookkeeping for techniques | same |
| `mechanism-ir` | `vendor/rust_symbolic/crates/mechanism-ir` | policy applicability as incentive-compatibility-free selection | same |
| `rdf-shacl` (`generate_shacl`, `validate`) | `vendor/rust_symbolic/crates/rdf-shacl` | the commit gate in `mm-library::validate` | same |
| `kg-extract` + `kg-llm` + `kg-validate` pattern | `vendor/rust_extract/crates/{kg-extract,kg-llm,kg-validate}` | seed-corpus extraction: structured output, schema validation, explicit rejections | `vendor/rust_extract/COPYING.md` |
| `rdf-codec` | already integrated Phase 1 | canonical RDF encode/decode for every entry | — |

Rule: no path dependencies, submodules, or `[patch]` on unowned code. Copied crates become workspace
members and are adapted behind `mm-library` types; `mm-cli codex verify` (Phase 2) confirms the
`mmc:copiedFrom` triples exist.

---

## 7. Step-by-step implementation tasks

1. **Add the ontology.** Extend `ontology/library.ttl` with the classes and relations in §4; run
   `mm-cli graph validate --ontology` and register the namespace in `mm-core::ns`.
2. **Generate SHACL shapes.** Use `rdf-shacl::generate_shacl` for the structural fragment, then hand-tune
   `ontology/shapes/library.shacl.ttl`: require `title`, `purpose`, ≥1 `applicableWhen`, ≥1
   `contraindication`, ≥1 `example`, ≥1 `evidence`; require `Policy` to have `version` and `behavior`;
   forbid `Case` without `outcomeQuality`.
3. **Implement the entry structs** (`entry.rs`, `doctrine.rs`, `technique.rs`, `frame.rs`, `policy.rs`,
   `case_db.rs`) with `rdf-codec` `ToRdf`/`FromRdf`; decode is total on the supported fragment
   (unknown properties are hard errors).
4. **Implement the store** (`store.rs`): write canonical Turtle to `/library` through the Phase 1 graph
   actor; upsert the SQLite index in the same event-log transaction (append → apply → commit).
5. **Implement the SHACL commit gate** (`validate.rs`): every `upsert_entry` validates first; a violation
   returns `LibraryError::ShapeViolation` and is rejected, never coerced or partially written.
6. **Implement duplicate detection:** reject an `upsert` whose `content_hash` equals an existing
   different entry, or whose `slug` exists with different body at the same version.
7. **Implement the policy genome** (`policy.rs`): `new_policy_version` always inserts a new immutable
   version with `parent_version = head`; mutating an existing `policy_versions` row is impossible (no
   UPDATE path); `record_fitness` appends to `policy_fitness`.
8. **Implement frames** (`frame.rs`): `compose_frames` merges bases with child-delta semantics
   (`Frame_child = Frame_parent ⊕ Δ`), records a `FrameInstance` with `slots`, `missing`, and `parents`;
   `missing_slots` feeds Phase 8's question-value computation.
9. **Implement applicability** (`applicability.rs`): deterministic scoring of a `Technique` against an
   `ApplicabilityState` (activation match, domain fitness, fitness, contraindication penalty); store the
   ranked result in `applicability_runs`; expose `--gold` comparison for reproducibility.
10. **Implement case retrieval** (`case_db.rs`): retrieve successful / failed / unusual / edge precedents,
    rank by `case_utility = Similarity × OutcomeQuality × Transferability × EvidenceQuality`.
11. **Implement the seed extraction pipeline** (`seed.rs` + `prompts/library_extract.md`): send the
    extraction prompt through `mm-llm` `StructuredOut<SeedEntry>`, validate with the SHACL gate, write
    accepted entries and a rejection ledger (`bench/library/extraction/validated.jsonl` +
    `rejections.jsonl`); never commit an unvalidated entry.
12. **Add the plugin modules** `modules/cognition/library-query` and `modules/cognition/frame-activate`
    and register them via Phase 2 (`codex scan`), then run `codex verify`.

---

## 8. Detailed logging requirements

All records go through `mm-log` (parent plan §10) with the global fields (`trace_id` = episode or job ULID,
`span_id`, `target`, `event`, `result`, `latency_ms`, `error`). Phase 7 event codes and fields:

| Event code | Required fields |
|---|---|
| `library.entry.upsert` | `entry_iri`, `kind`, `version`, `content_hash`, `result` |
| `library.entry.validated` | `entry_iri`, `shapes`, `violation_count` |
| `library.entry.rejected` | `entry_iri`, `reason`, `shape_violations[]`, `job_ulid` |
| `library.dup.detected` | `entry_iri`, `existing_iri`, `content_hash` |
| `library.extract.job.start` / `.end` | `job_ulid`, `source`, `accepted`, `rejected`, `model`, `tokens`, `cost` |
| `library.applicable.rank` | `episode_ulid`, `state_hash`, `ranked[]`, `score_components{activation,fitness,contraindication_penalty}` |
| `library.case.retrieve` | `query_hash`, `case_iri`, `similarity`, `utility_components{outcome,transferability,evidence}` |
| `library.frame.compose` | `instance_ulid`, `frame_iri`, `parents[]`, `slots_known`, `slots_missing` |
| `library.frame.switch` | `episode_ulid`, `from_frame`, `to_frame`, `reason` |
| `policy.version.create` | `policy_iri`, `version`, `parent_version`, `scope`, `confidence` |
| `policy.fitness.update` | `policy_iri`, `version`, `trials`, `successes`, `mean_utility` |
| `policy.promote` | `policy_iri`, `from_version`, `to_version`, `evidence[]` |

Audit records (append-only, immutable): every `library.entry.upsert`, `policy.version.create`, and
`policy.promote` is transactional with its event-log entry. Operational records (JSONL
`data/logs/*.jsonl`): the ranking/retrieval detail above.

---

## 9. Testing plan

- **Round-trip** (`entry_roundtrip.rs`): every entry type encodes and decodes to an identical
  `content_hash`; unknown properties are hard errors.
- **SHACL gate** (`shacl_gate.rs`): a well-formed entry commits; an entry missing `applicableWhen`,
  `evidence`, or `example` is rejected with the specific shape id.
- **Extraction rejection** (`extraction_rejection.rs`): `bench/library/extraction/malformed.jsonl`
  yields rejections and a non-zero exit; valid `raw.jsonl` entries all commit.
- **Applicability gold** (`applicability_gold.rs`): `applicable(...)` on the committed state matches
  `bench/library/gold_applicability.jsonl` exactly (deterministic ranking).
- **Policy versioning** (`policy_versioning.rs`): versions are monotonic; `parent_version` chains;
  attempting an in-place update has no API path and fails; fitness appends.
- **Case retrieval** (`case_retrieval.rs`): against `bench/library/gold_cases.jsonl`, expected precedents
  are returned; a failed case with high similarity is not outranked by a lower-similarity success when
  outcome quality dominates.
- **Orphans**: `library orphans --fail-if-any` returns 0 on the seed corpus and ≥1 on a fixture with a
  dangling `mm:testedBy`.
- **Property**: `content_hash` is stable across two encodes and across entry insertion order.
- **Logs**: `mm-cli logs verify` passes for the corpus run.

---

## 10. Pass gate

Run as plain commands; earlier phases' gates must still pass.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 1. Seed library valid, one entry per class, no orphans, 0 SHACL violations.
mm-cli library import ontology/seed/library_seed.ttl
mm-cli library validate --graph library          # exit 0, 0 violations
mm-cli library list --kind Doctrine | wc -l      # >= 1 for each of the 11 kinds
mm-cli library orphans --fail-if-any             # exit 0

# 2. A malformed LLM-extracted entry is rejected (not coerced).
mm-cli library extract --source bench/library/extraction/malformed.jsonl \
    --out bench/library/extraction/validated.jsonl ; test $? -ne 0

# 3. Applicability ranking is deterministic and matches gold.
mm-cli library applicable --state bench/library/state_uncertain_strategy.json --top 5 \
    --gold bench/library/gold_applicability.jsonl    # exit 0 on exact match

# 4. Policy genome: versioned, parented, fitnessed, never mutated in place.
mm-cli policy history https://metamind.dev/policy/prefer_simpler_solution@2   # monotonic versions
mm-cli policy fitness https://metamind.dev/policy/prefer_simpler_solution@2   # trials/successes present

# 5. Logging verification.
mm-cli logs verify
```

Pass criteria: all commands exit 0; `library validate` reports **0** violations; each of the eleven entry
kinds has ≥1 committed seed entry; the malformed extraction **exits non-zero**; applicability matches the
gold file exactly; every policy resolves by stable URI with a `version`/`parent`/`fitness`; `logs verify`
reports gapless audit and clean redaction.

---

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| One-philosophy hammer (over-applying a single doctrine) | Applicability profiles (`applicableWhen`/`inapplicableWhen`) plus explicit disagreement support; ranked selection, not a default |
| Frame explosion | Compositional frames + nested inheritance (`parent ⊕ Δ`); only surprising/durable frames become stored entries |
| Over-generalization of a provisional technique | New entries commit with low `confidence`; success in one domain does not raise `domain_fitness` elsewhere |
| Duplicate/contradictory entries | `content_hash` dedup on ingest; contradictions surface as explicit records (Phase 6) rather than silent overwrite |
| LLM extraction hallucination | SHACL + orphan checks are a hard commit gate; unvalidated entries are never written |
| Copied reference drifts from upstream | `mmc:copiedFrom` provenance + per-origin `COPYING.md`; re-copy is a reviewed change-set (Phase 11) |
| Non-deterministic applicability | Scoring is pure arithmetic; results are stored and gold-compared |

---

## 12. Design traceability

| Design section | Where addressed |
|---|---|
| §13 Cognitive Library (entry classes, distinctions) | §1, §4, §5, §7 tasks 1–6 |
| §14 Conceptual frames (composition, slots, scripts) | §5 `Frame`/`FrameInstance`, §7 task 8 |
| §15 Philosophies as lenses / applicability profiles | §4 `mm:applicableWhen`, §7 task 3, §11 |
| §16 Cognitive techniques library | §5 `Technique`, §7 task 9 |
| §17 Similarity, analogy, precedent, case utility | §5 `case_utility`, §7 task 10, §9 |
| §44 Policy genome | §4/§5 `Policy`, §7 task 7, §9 |
| §45 Cognitive portfolios / multiple lenses | §5 multi-frame, §11 disagreement mitigation |
| §69 Automatic discovery of new techniques | §7 task 11 (extraction pipeline), §11 |
| §87–88 RDF/OWL layer, runtime model | §4 ontology deltas, §5 types |
| §108–110 anti-overbuild / invariants | §1 scope limits, §9 orphan/determinism checks |
