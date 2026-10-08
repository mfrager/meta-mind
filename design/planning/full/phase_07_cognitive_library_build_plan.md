# Phase 7 — Cognitive Library, Conceptual Frames, and the Policy Genome (Extended Build Plan)

> Extended from: `design/planning/phase_07_cognitive_library_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 7 · Codename Metamind (`mm`)

This is the research-grounded, streamlined revision. It keeps the Phase 7 scope and gate but locks in the
specific mechanisms — a verification-gated **skill library**, **experience→insight→technique→policy**
compilation, **structure-mapped** case retrieval, and an **evolving policy genome** — that make the
library a durable, self-improving repertoire rather than a static ontology. Phase 7 stores, validates and
selects; it does not run episodes (Phase 8) or decide actions (Phase 9).

---

## 0. Research foundation (code-available)

Only ideas with available source code are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| Verified, composable **skill library** | Voyager — https://github.com/MineDojo/Voyager | Executable skills with a natural-language description, a verification test, and embedding retrieval; new skills are verified before they enter the library | `skills` table + `mm:Skill` entries; a skill is promotable only after its `tests_ref` passes (`skill verify`) |
| **Experience → insight → rule** | ExpeL — https://github.com/LeapLabTHU/ExpeL | Extract cross-task insights from success and failure trajectories; track insight up/down-votes | `insights` store; `compile_experience()` turns repeated insights into candidate techniques/policies |
| Failure→lesson reuse | Reflexion — https://github.com/noahshinn/reflexion | Persist verbal lessons from failures as retrievable entries | `mm:AntiPattern` + `mm:Case(failure)` with a `correctiveRule` link |
| Induced **procedural memory** | Agent Workflow Memory — https://github.com/zorazrw/agent-workflow-memory | Induce reusable workflows from trajectories | `workflows` table; a validated workflow compiles to a `mm:Pattern`/`mm:Skill` |
| Case-based reasoning loop | CBR ref — https://github.com/sipemu/case-based-reasoning · CaBRNet — https://github.com/aiser-team/cabrnet | Retrieve→adapt→revise→retain; case representation and distance | `cases` table; `case_utility` = Similarity × OutcomeQuality × Transferability × EvidenceQuality |
| Structure-mapped analogy | ANASIME — https://github.com/Tijl/ANASIME | Relational, structural correspondence (not surface similarity) | `case_map` correspondence edges; `structural_similarity()` drives analogy and transfer |
| **Policy genome** + fitness | DEAP — https://github.com/DEAP/deap | Population, fitness, mutation/crossover of behaviors | `policy_population`; `policy evolve` runs a generation; versions stay immutable |
| Open-ended program evolution | OpenEvolve — https://github.com/codelion/openevolve | Island populations, LLM-guided mutation, fitness-ranked retention | Candidate policies/techniques are evolved as programs and retained only on fitness gain |
| Declarative/procedural separation + activation-based selection | Soar — https://github.com/SoarGroup/Soar · ACT-R — https://github.com/asmaloney/ACT-R · LIDA — https://github.com/CognitiveComputingResearchGroup/lida-framework | Separate declarative entries (knowledge) from procedural entries (skills); select by an activation score | Every entry carries an `activation_score` combining recency, frequency and utility |

---

## 1. Objective and scope

Deliver the **Cognitive Library** organ: a persistent, versioned repertoire of reusable ways of thinking
(doctrine / principle / heuristic / technique / pattern / case / anti-pattern / skill / policy / frame /
evaluation) stored in the Oxigraph `/library` graph and indexed in SQLite, plus conceptual-frame
composition and an evolving policy genome.

**In scope**
- `mm-library` crate: the eleven entry types and their RDF codecs.
- SHACL shapes for every class, enforced as a hard commit gate (reject, never coerce).
- A **verified skill library**: skills are executable, described, embedding-retrievable and promotable
  only after their tests pass.
- An **experience compiler**: `compile_experience()` distils trajectories into insights → techniques →
  policies (ExpeL/Reflexion/AWM).
- **Case + analogy** retrieval via structure mapping (`case_map` correspondences).
- Multi-frame composition with slot-gap reporting.
- The **policy genome**: immutable versions with parentage, fitness, and a generation-based evolution loop.

**Out of scope (later phases).** Metacognitive program compilation (8), the sanity firewall and bounded
decisions (9), tool execution (10), and self-engineering of the library itself (11). Phase 7 only stores,
validates and selects.

---

## 2. Architecture

```
                    episode / trajectory (Phase 8 evidence, Phase 5 memories)
                                       │
                          ┌────────────┴────────────┐
                          ▼                         ▼
                  compile_experience()          case retrieval
             insights → technique/policy      (structure mapping)
                          │                         │
                          ▼                         ▼
        ┌───────────────────────── /library (Oxigraph) ─────────────────────────┐
        │  declarative entries        procedural entries        frames          │
        │  doctrine/principle/…       skill / workflow          frame(+Δ)       │
        │  case / anti-pattern        policy(+genome)           frame_instance  │
        └───────────────────────────────────┬───────────────────────────────────┘
                                            │  applicability + activation score
                                            ▼
                                     Phase 8 program compiler
```

**Invariants**
1. **SHACL-or-nothing.** No entry reaches `/library` without passing its shapes and the orphan check.
2. **Procedural entries are executable and verified.** A `mm:Skill`/`workflow` is promotable only after
   its referenced test passes (`skill verify`); unverified skills are retrievable only as drafts.
3. **Immutable versions.** Entries and policies are versioned by insertion; there is no UPDATE path.
4. **Provenance always.** Every entry links to `evidence` and `derivedFrom`; copied reference code is
   recorded via `mmc:copiedFrom` (Phase 2).
5. **Deterministic selection.** `applicability()` and `activation_score()` are pure arithmetic; ranking is
   stored and gold-compared.

---

## 3. Deliverables and workspace layout

```
crates/mm-library/
  src/lib.rs          # LibraryManager
  src/entry.rs        # EntryKind + LibraryEntry trait
  src/doctrine.rs     # Doctrine, Principle, Heuristic, AntiPattern
  src/technique.rs    # Technique, Pattern, Evaluation
  src/skill.rs        # Skill, Workflow, verification + composition
  src/frame.rs        # Frame, FrameInstance, SlotValue, Script
  src/policy.rs       # Policy, PolicyDelta, FitnessRecord, Scope, ActivationCondition, PolicyBehavior
  src/genome.rs       # policy_population, evolve(), mutation/crossover
  src/case_db.rs      # Case, CaseElement, Correspondence, structural_similarity
  src/experience.rs   # compile_experience(): insights → technique/policy
  src/applicability.rs# ApplicabilityState/Score, activation_score(), case_utility()
  src/seed.rs         # LLM extraction pipeline (kg-llm StructuredOut + kg-validate)
  src/rdf.rs          # ToRdf/FromRdf for every entry type (rdf-codec)
  src/validate.rs     # SHACL commit gate (rdf-shacl)
  src/store.rs        # SQLite index + /library writes via the graph actor
  src/prompts/{library_extract.md, insight_extract.md, workflow_induce.md}
  tests/{entry_roundtrip,shacl_gate,skill_verify,case_retrieval,applicability_gold,policy_versioning,genome_evolve,experience_compile,extraction_rejection}.rs
crates/mm-store-sqlite/migrations/0007_library.sql
modules/cognition/library-query/     # plugin.toml + src/lib.rs + tests + manual
modules/cognition/frame-activate/    # plugin.toml + src/lib.rs + tests + manual
ontology/library.ttl                 # T-Box additions (mm: classes/relations)
ontology/shapes/library.shacl.ttl    # SHACL shapes (generated + hand-tuned)
ontology/seed/library_seed.ttl       # committed, validated seed corpus
bench/library/{state_uncertain_strategy.json, gold_applicability.jsonl, gold_cases.jsonl, gold_activation.json}
bench/library/trajectories/{success.jsonl, failure.jsonl}     # experience-compiler fixtures
bench/library/evolution/{baseline_population.json, generations.json}
bench/library/extraction/{raw.jsonl, malformed.jsonl}
data/library/skills/                 # skill implementation artefacts (WASM/DSL), content-addressed
vendor/rust_symbolic/COPYING.md      # origin repo/revision/license for copied crates
vendor/rust_extract/COPYING.md
```

---

## 4. Detailed specifications

### 4.1 Named graph and identifiers

Named graph `https://metamind.dev/graph/library`. Declarative entries use path-derived, human-diffable
URIs (they are named, versioned things); runtime instances use ULID IRIs:

```
head entry     https://metamind.dev/library/{kind}/{slug}
versioned      https://metamind.dev/library/{kind}/{slug}@{version}
policy         https://metamind.dev/policy/{name}@{version}
frame          https://metamind.dev/library/frame/{slug}
skill          https://metamind.dev/library/skill/{slug}@{version}
instance       https://metamind.dev/data/{ulid}          # frame instances, runs, jobs, insights
```

### 4.2 Ontology (`ontology/library.ttl`, prefix `mm: = https://metamind.dev/ontology#`)

- Classes: `mm:LibraryEntry` ⊃ `mm:Doctrine, mm:Principle, mm:Heuristic, mm:Technique, mm:Pattern,
  mm:Case, mm:AntiPattern, mm:Skill, mm:Policy, mm:Frame, mm:FrameInstance, mm:Evaluation, mm:Insight,
  mm:Workflow`.
- Relations: `mm:applicableWhen, mm:inapplicableWhen, mm:contraindication, mm:example, mm:counterexample,
  mm:evidence, mm:derivedFrom, mm:correctiveRule, mm:improves, mm:replaces, mm:testedBy,
  mm:activationCondition, mm:behavior, mm:fitness, mm:confidence, mm:parentPolicy, mm:parentFrame,
  mm:frameSlot, mm:role, mm:affordance, mm:typicalAction, mm:failureMode, mm:scriptStep, mm:outcomeQuality,
  mm:transferability, mm:evidenceQuality, mm:correspondsTo, mm:implements, mm:verifies, mm:upvotes,
  mm:downvotes, mm:generation, mm:derivedFromTrajectory`.
- Datatypes: `mm:title, mm:purpose, mm:version, mm:scope, mm:strength, mm:domain, mm:activationScore`.

### 4.3 SQLite index (`0007_library.sql`)

```sql
CREATE TABLE library_entries (
  id               TEXT(26) PRIMARY KEY,
  iri              TEXT NOT NULL,
  version_iri      TEXT NOT NULL UNIQUE,
  kind             TEXT NOT NULL CHECK (kind IN ('Doctrine','Principle','Heuristic','Technique','Pattern',
                                                'Case','AntiPattern','Skill','Policy','Frame','Evaluation',
                                                'Insight','Workflow')),
  slug             TEXT NOT NULL,
  version          INTEGER NOT NULL DEFAULT 1,
  title            TEXT NOT NULL,
  body_ttl         TEXT NOT NULL,
  content_hash     TEXT NOT NULL,
  activation_score REAL NOT NULL DEFAULT 0.0,   -- ACT-R-style: recency+frequency+utility
  embedding_ref    TEXT,                        -- kg-embed vector row id
  created_ulid     TEXT NOT NULL,
  valid_from       TEXT NOT NULL,
  valid_until      TEXT,
  UNIQUE (iri, version)
);

CREATE TABLE skills (
  id            TEXT(26) PRIMARY KEY,
  iri           TEXT NOT NULL,
  name          TEXT NOT NULL,
  description   TEXT NOT NULL,
  impl_ref      TEXT NOT NULL,          -- content-addressed artefact under data/library/skills/
  entrypoint    TEXT NOT NULL,
  signature     TEXT NOT NULL,
  tests_ref     TEXT NOT NULL,
  verification  TEXT NOT NULL CHECK (verification IN ('draft','verified','failing')),
  verified_at   TEXT,
  embedding_ref TEXT,
  version       INTEGER NOT NULL DEFAULT 1,
  created_ulid  TEXT NOT NULL,
  UNIQUE (iri, version)
);

CREATE TABLE workflows (
  id            TEXT(26) PRIMARY KEY,
  iri           TEXT NOT NULL,
  name          TEXT NOT NULL,
  steps_json    TEXT NOT NULL,          -- induced from trajectory
  induced_from  TEXT NOT NULL,          -- trajectory ULID
  version       INTEGER NOT NULL DEFAULT 1,
  created_ulid  TEXT NOT NULL
);

CREATE TABLE cases (
  id                TEXT(26) PRIMARY KEY,
  iri               TEXT UNIQUE NOT NULL,
  kind              TEXT NOT NULL CHECK (kind IN ('success','failure','unusual','edge')),
  problem_ttl       TEXT NOT NULL, solution_ttl TEXT NOT NULL, outcome_ttl TEXT NOT NULL,
  outcome_quality   REAL NOT NULL CHECK (outcome_quality   BETWEEN 0.0 AND 1.0),
  transferability   REAL NOT NULL CHECK (transferability   BETWEEN 0.0 AND 1.0),
  evidence_quality  REAL NOT NULL CHECK (evidence_quality  BETWEEN 0.0 AND 1.0),
  embedding_ref     TEXT,
  created_ulid      TEXT NOT NULL
);

CREATE TABLE case_map (                  -- structure-mapping correspondences
  id          TEXT(26) PRIMARY KEY,
  case_iri    TEXT NOT NULL,
  source_elem TEXT NOT NULL,
  target_elem TEXT NOT NULL,
  relation    TEXT NOT NULL,
  score       REAL NOT NULL,
  created_ulid TEXT NOT NULL
);

CREATE TABLE insights (                  -- ExpeL/Reflexion
  id           TEXT(26) PRIMARY KEY,
  iri          TEXT UNIQUE NOT NULL,
  text         TEXT NOT NULL,
  kind         TEXT NOT NULL CHECK (kind IN ('success_rule','failure_lesson','workflow','preference')),
  evidence     TEXT NOT NULL,            -- JSON array of trajectory ULIDs
  upvotes      INTEGER NOT NULL DEFAULT 0,
  downvotes    INTEGER NOT NULL DEFAULT 0,
  created_ulid TEXT NOT NULL
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
  parent_version  INTEGER,               -- NULL only for version 1
  scope_ttl       TEXT NOT NULL,
  activation_ttl  TEXT NOT NULL,
  behavior_ttl    TEXT NOT NULL,         -- typed DSL fragment, not free prose
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

CREATE TABLE policy_population (         -- DEAP/OpenEvolve genome evolution
  id            TEXT(26) PRIMARY KEY,
  generation    INTEGER NOT NULL,
  policy_iri    TEXT NOT NULL,
  version       INTEGER NOT NULL,
  parent_a      TEXT, parent_b TEXT,
  mutation      TEXT,                    -- LLM mutation description / prompt hash
  fitness       REAL,
  retained      INTEGER NOT NULL DEFAULT 0,
  created_ulid  TEXT NOT NULL
);

CREATE TABLE frame_instances (
  id           TEXT(26) PRIMARY KEY,
  frame_iri    TEXT NOT NULL,
  episode_ulid TEXT NOT NULL,
  parents_json TEXT NOT NULL,
  slots_json   TEXT NOT NULL,
  missing_json TEXT NOT NULL,
  created_ulid TEXT NOT NULL
);

CREATE TABLE applicability_runs (
  id           TEXT(26) PRIMARY KEY,
  episode_ulid TEXT NOT NULL,
  state_hash   TEXT NOT NULL,
  ranked_json  TEXT NOT NULL,
  created_ulid TEXT NOT NULL
);
```

### 4.4 Public interfaces (Rust)

```rust
pub enum EntryKind { Doctrine, Principle, Heuristic, Technique, Pattern, Case,
                     AntiPattern, Skill, Policy, Frame, Evaluation, Insight, Workflow }

pub trait LibraryEntry: Send + Sync {
    fn kind(&self) -> EntryKind;
    fn head_iri(&self) -> NamedNode;
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<(), CodecError>;
    fn from_rdf(g: &Graph, iri: &NamedNode) -> Result<Self, CodecError> where Self: Sized;
    fn referenced_iris(&self) -> Vec<NamedNode>;
}

pub struct Technique {
    pub iri: NamedNode, pub title: String, pub purpose: String, pub domain: Vec<String>,
    pub applicable_when: Vec<ActivationCondition>, pub inapplicable_when: Vec<ActivationCondition>,
    pub contraindications: Vec<String>, pub examples: Vec<NamedNode>, pub counterexamples: Vec<NamedNode>,
    pub evidence: Vec<NamedNode>, pub tested_by: Vec<NamedNode>,
    pub confidence: f32, pub domain_fitness: BTreeMap<String, f32>, pub activation_score: f32,
}

/// Voyager-style: an executable, described, verified procedure.
pub struct Skill {
    pub iri: NamedNode, pub name: String, pub description: String,
    pub impl_ref: String, pub entrypoint: String, pub signature: String,
    pub tests_ref: String, pub verification: Verification, pub embedding_ref: Option<String>,
}
pub enum Verification { Draft, Verified, Failing }

/// AWM-style: a procedure induced from a trajectory.
pub struct Workflow { pub iri: NamedNode, pub name: String, pub steps: Vec<String>, pub induced_from: Ulid }

/// CBR + structure mapping (ANASIME).
pub struct Case {
    pub iri: NamedNode, pub kind: CaseKind, pub problem_ttl: String, pub solution_ttl: String,
    pub outcome_ttl: String, pub outcome_quality: f32, pub transferability: f32, pub evidence_quality: f32,
}
pub struct Correspondence { pub source: CaseElement, pub target: CaseElement, pub relation: String, pub score: f32 }
pub fn structural_similarity(a: &Case, b: &Case) -> Vec<Correspondence>;   // relational, not surface

/// ExpeL/Reflexion.
pub struct Insight { pub iri: NamedNode, pub text: String, pub kind: InsightKind,
                     pub evidence: Vec<Ulid>, pub upvotes: i32, pub downvotes: i32 }
pub struct CompiledExperience { pub techniques: Vec<Technique>, pub policies: Vec<PolicyDelta> }
pub fn compile_experience(traj: &[Trajectory], insights: &[Insight]) -> CompiledExperience;

/// Policy genome.
pub enum Scope { Global, Domain(String), Frame(NamedNode), Relationship(NamedNode) }
pub struct ActivationCondition(pub String);
pub struct PolicyBehavior(pub String);                 // typed DSL fragment
pub struct FitnessRecord { pub trials: u32, pub successes: u32, pub mean_utility: f32 }
pub struct Policy {
    pub iri: NamedNode, pub version: u32, pub scope: Scope, pub activation: ActivationCondition,
    pub behavior: PolicyBehavior, pub evidence: Vec<NamedNode>, pub fitness: FitnessRecord,
    pub confidence: f32, pub parent: Option<NamedNode>,
}

/// Frames.
pub enum SlotValue { Known(String), Unknown, Inferred(String) }
pub struct Frame { pub iri: NamedNode, pub slug: String, pub parent: Option<NamedNode>,
    pub roles: BTreeMap<String,String>, pub slots: Vec<String>, pub typical_actions: Vec<String>,
    pub failure_modes: Vec<String>, pub scripts: Vec<Vec<String>>, pub domain: Vec<String> }
pub struct FrameInstance { pub ulid: Ulid, pub frame_iri: NamedNode, pub episode: Ulid,
    pub slots: BTreeMap<String, SlotValue>, pub parents: Vec<NamedNode> }

/// Selection (deterministic).
pub struct ApplicabilityState { pub frames: Vec<NamedNode>, pub goal: NamedNode, pub domain: Vec<String>,
    pub uncertainty: f32, pub stakes: f32, pub history: Vec<NamedNode>, pub now: Timestamp }
pub struct ApplicabilityScore { pub total: f32, pub activation: f32, pub fitness: f32,
    pub contraindication_penalty: f32 }
pub fn applicability(t: &Technique, s: &ApplicabilityState) -> ApplicabilityScore;
pub fn activation_score(recency: f32, frequency: f32, utility: f32) -> f32;  // ACT-R-style blend
pub fn case_utility(c: &Case, sim: f32) -> f32;            // Sim × Outcome × Transfer × Evidence

pub struct LibraryManager { /* graph actor, SqlitePool, Arc<dyn LlmClient>, AuditLog */ }
impl LibraryManager {
    pub async fn upsert_entry<E: LibraryEntry>(&self, e: E) -> Result<NamedNode>;
    pub async fn register_skill(&self, s: Skill) -> Result<NamedNode>;      // verification = Draft
    pub async fn verify_skill(&self, iri: &NamedNode) -> Result<Verification>;
    pub async fn compile_experience(&self, job: ExperienceJob) -> Result<CompiledExperience>;
    pub async fn applicable(&self, s: &ApplicabilityState, top: usize)
        -> Result<Vec<(NamedNode, ApplicabilityScore)>>;
    pub async fn retrieve_cases(&self, q: &CaseQuery) -> Result<Vec<(NamedNode, Vec<Correspondence>)>>;
    pub async fn compose_frames(&self, bases: &[NamedNode], episode: Ulid) -> Result<FrameInstance>;
    pub async fn new_policy_version(&self, name: &str, d: PolicyDelta, ev: &[NamedNode]) -> Result<Policy>;
    pub async fn record_fitness(&self, iri: &NamedNode, v: u32, o: &Outcome) -> Result<FitnessRecord>;
    pub async fn evolve(&self, name: &str, generations: u32, budget: EvoBudget) -> Result<EvolutionReport>;
}
```

### 4.5 Seed extraction, verification and promotion

- `library extract` sends `prompts/library_extract.md` through `mm-llm` `StructuredOut<SeedEntry>`,
  validates with the SHACL gate, then writes accepted entries + a rejection ledger. **Never commit an
  unvalidated entry.**
- `skill register` stores a skill as `Draft`; `skill verify` runs its `tests_ref` in the Phase 10 sandbox
  and flips it to `Verified` or `Failing`. Only `Verified` skills are returned by `skill retrieve`.
- `compile_experience` reads trajectories + insights and emits candidate techniques/policies as `Draft`
  entries with `confidence` below the promotion threshold; promotion happens through Phase 11.
- `policy evolve` runs the DEAP/OpenEvolve-style loop: mutate/crossover behavior fragments, evaluate
  against a benchmark, retain only fitness gains, and insert each candidate as an immutable version.

### 4.6 CLI (`mm-cli`)

```bash
mm-cli library import ontology/seed/library_seed.ttl
mm-cli library validate --graph library            # SHACL; non-zero on any violation
mm-cli library orphans --fail-if-any
mm-cli library list --kind Technique
mm-cli library applicable --state bench/library/state_uncertain_strategy.json --top 5 \
    --gold bench/library/gold_applicability.jsonl
mm-cli skill register --manifest data/library/skills/skill.json
mm-cli skill verify https://metamind.dev/library/skill/api-capability-check@1
mm-cli skill retrieve --query "verify external api before use" --top 3
mm-cli case retrieve --problem bench/library/gold_cases.jsonl
mm-cli experience compile --trajectories bench/library/trajectories/success.jsonl \
    --out bench/library/compiled.jsonl
mm-cli policy history  https://metamind.dev/policy/prefer_simpler_solution@2
mm-cli policy fitness  https://metamind.dev/policy/prefer_simpler_solution@2
mm-cli policy evolve   prefer_simpler_solution --generations 3 --budget bench/library/evolution/budget.toml
mm-cli frame compose   --frames problem_solving,software,debugging,high_stakes --episode <ULID>
mm-cli frame missing   <frame-instance-ULID>
```

### 4.7 Plugin modules (nexus contract)

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
"cognition.skill_retrieve"     = { source = "handlers::skill_retrieve" }
"cognition.case_retrieve"      = { source = "handlers::case_retrieve" }
[monad.operations]
name = "cognition"
arity = 1
[build]
rust_edition = "2021"
```

---

## 5. Build sequence

1. **Ontology + shapes.** Extend `ontology/library.ttl`; generate `library.shacl.ttl` with
   `rdf-shacl::generate_shacl` and hand-tune. *Check:* `graph validate --graph library` clean vs broken.
2. **Entry structs + RDF codecs** (`entry/doctrine/technique/skill/frame/policy/case_db`).
   *Check:* round-trip `content_hash` stable; unknown properties are hard errors.
3. **Store + SHACL gate** (`store.rs`, `validate.rs`): write canonical Turtle to `/library` through the
   Phase 1 actor and the SQLite index in the same event transaction; validate first. *Check:* a violation
   returns `ShapeViolation` and writes nothing.
4. **Duplicate + orphan detection.** *Check:* duplicate `content_hash` rejected; `orphans` fails on a
   dangling `mm:testedBy`.
5. **Skill library** (`skill.rs`): register/verify/retrieve; `Verified` required for retrieval.
   *Check:* `skill_verify.rs` — draft excluded until tests pass.
6. **Frames** (`frame.rs`): `Frame_child = Frame_parent ⊕ Δ`; record `frame_instances` with `missing`.
   *Check:* compose reports missing slots deterministically.
7. **Applicability + activation** (`applicability.rs`): deterministic score + `activation_score` blend.
   *Check:* matches `gold_applicability.jsonl` and `gold_activation.json` exactly.
8. **Case + analogy** (`case_db.rs`): retrieve by `case_utility`; build `case_map` via
   `structural_similarity`. *Check:* `case_retrieval.rs` gold.
9. **Experience compiler** (`experience.rs`): insights → technique/policy drafts. *Check:*
   `experience_compile.rs` — repeats yield candidate entries with low confidence.
10. **Policy genome** (`policy.rs`, `genome.rs`): immutable versions; `evolve()` retains only gains.
    *Check:* `policy_versioning.rs`, `genome_evolve.rs`.
11. **Seed extraction** (`seed.rs`): `kg-llm` + SHACL gate + rejection ledger. *Check:*
    malformed input exits non-zero, valid input commits.
12. **Plugin modules + `codex`.** Register modules, `codex scan`/`verify`. *Check:* `codex verify` exit 0.

---

## 6. Logging and observability

Global fields apply (`trace_id` = episode/job ULID). Phase 7 codes:

| Event code | Required fields |
|---|---|
| `library.entry.upsert` | `entry_iri, kind, version, content_hash, activation_score, result` |
| `library.entry.validated` | `entry_iri, shapes, violation_count` |
| `library.entry.rejected` | `entry_iri, reason, shape_violations[], job_ulid` |
| `library.dup.detected` | `entry_iri, existing_iri, content_hash` |
| `skill.register` / `skill.verify` | `skill_iri, version, verification` / `skill_iri, tests_ref, exit_code, verification` |
| `skill.retrieve` | `query_hash, skill_iri, score, verification` |
| `library.extract.job.start/.end` | `job_ulid, source, accepted, rejected, model, tokens, cost` |
| `library.applicable.rank` | `episode_ulid, state_hash, ranked[], score_components{activation,fitness,contraindication_penalty}` |
| `library.case.retrieve` | `query_hash, case_iri, similarity, correspondences, utility_components{outcome,transferability,evidence}` |
| `experience.compile` | `job_ulid, trajectories, insights, techniques[], policies[]` |
| `library.frame.compose` | `instance_ulid, frame_iri, parents[], slots_known, slots_missing` |
| `library.frame.switch` | `episode_ulid, from_frame, to_frame, reason` |
| `policy.version.create` | `policy_iri, version, parent_version, scope, confidence` |
| `policy.fitness.update` | `policy_iri, version, trials, successes, mean_utility` |
| `genome.evolve.start/.generation/.end` | `name, generations, budget` / `generation, retained, best_fitness` / `name, best_iri, generations` |

**Audit (immutable, transactional with the mutation):** `library.entry.upsert`, `skill.verify`,
`policy.version.create`, `policy.fitness.update`, `genome.evolve.end`. Operational detail (ranking,
retrieval, extraction) goes to the JSONL sink. `mm-cli logs verify` checks schema, one audit per commit,
gapless chained sequence, and redaction.

---

## 7. Testing

| Type | Test | Assertion |
|---|---|---|
| Round-trip | `entry_roundtrip.rs` | every entry type encodes→decodes to an identical `content_hash`; unknown property is a hard error |
| SHACL gate | `shacl_gate.rs` | missing `applicableWhen`/`evidence`/`example` rejected with the shape id; well-formed commits |
| Skill verify | `skill_verify.rs` | retrieval returns only `Verified` skills; a failing test keeps it `Draft`/`Failing` |
| Case retrieval | `case_retrieval.rs` | gold precedents returned; structure mapping (not surface) orders analogies |
| Applicability gold | `applicability_gold.rs` | exact match to `gold_applicability.jsonl`; activation matches `gold_activation.json` |
| Policy versioning | `policy_versioning.rs` | versions monotonic, `parent_version` chains, no in-place update path, fitness appends |
| Genome | `genome_evolve.rs` | a generation retains only fitness gains; every candidate is an immutable version |
| Experience | `experience_compile.rs` | repeated insights produce candidate entries with confidence below threshold |
| Extraction | `extraction_rejection.rs` | `malformed.jsonl` → rejections + non-zero exit; `raw.jsonl` commits |
| Orphans / property | `orphans` + proptest | dangling reference fails; `content_hash` stable across insertion order |
| Logs | `logs verify` | schema-valid, gapless audit, clean redaction |

Fixtures: `bench/library/{state_uncertain_strategy.json, gold_applicability.jsonl, gold_cases.jsonl,
gold_activation.json, trajectories/*.jsonl, evolution/*.json, extraction/*.jsonl}`.

---

## 8. Pass gate

Run as plain commands; earlier phases' gates must still pass.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 1. Seed library valid: one entry per class, 0 SHACL violations, no orphans.
mm-cli library import ontology/seed/library_seed.ttl
mm-cli library validate --graph library          # exit 0, 0 violations
for k in Doctrine Principle Heuristic Technique Pattern Case AntiPattern Skill Policy Frame Evaluation; do
    mm-cli library list --kind "$k" | grep -q . || exit 1 ; done
mm-cli library orphans --fail-if-any             # exit 0

# 2. A malformed LLM-extracted entry is rejected, not coerced.
mm-cli library extract --source bench/library/extraction/malformed.jsonl \
    --out bench/library/extraction/validated.jsonl ; test $? -ne 0

# 3. Skill library: only verified skills are retrievable.
mm-cli skill verify https://metamind.dev/library/skill/api-capability-check@1   # exit 0
mm-cli skill retrieve --query "verify external api before use" --top 3          # returns only Verified

# 4. Deterministic selection matches gold.
mm-cli library applicable --state bench/library/state_uncertain_strategy.json --top 5 \
    --gold bench/library/gold_applicability.jsonl        # exit 0 on exact match

# 5. Experience compiler + evolving policy genome.
mm-cli experience compile --trajectories bench/library/trajectories/success.jsonl \
    --out bench/library/compiled.jsonl                   # emits drafts, exits 0
mm-cli policy evolve prefer_simpler_solution --generations 3 \
    --budget bench/library/evolution/budget.toml         # retains only gains

# 6. Policy versions immutable, parented, fitnessed; frames compose deterministically.
mm-cli policy history https://metamind.dev/policy/prefer_simpler_solution@2
mm-cli frame compose --frames problem_solving,software,debugging,high_stakes --episode 01J0000000000000000000000A

# 7. Logging verification.
mm-cli logs verify
```

**Pass criteria:** all commands exit 0; `library validate` reports **0** violations; each of the eleven
entry kinds has ≥1 committed seed entry; malformed extraction **exits non-zero**; `skill retrieve` returns
only `Verified` skills; applicability/activation match gold exactly; `experience compile` emits only
sub-threshold drafts; `policy evolve` adds only fitness-improving immutable versions; every policy
resolves by stable URI with `version`/`parent`/`fitness`; `logs verify` is green.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| One-philosophy hammer (a single doctrine dominates) | `applicableWhen`/`inapplicableWhen` profiles, explicit disagreement support, ranked selection not defaults; multiple simultaneous frames |
| Skill library rot (skills break as code changes) | Verification is re-run in the Phase 10 sandbox; skills carry `tests_ref` and downgrade to `Failing` |
| Experience over-generalization | `compile_experience` emits **drafts** with low confidence; promotion only through Phase 11 with evidence |
| Structure mapping is expensive/floods matches | Store correspondences with a score threshold; cap `case_map` edges per case |
| Genome evolution burns budget / drifts | Hard `EvoBudget`; retain only fitness gains; every candidate immutable and rollback-able |
| Library bloat/duplication | `content_hash` dedup + `orphans` check on ingest; contradiction surfaces as an explicit Phase 6 record |
| LLM extraction hallucination | SHACL + orphan checks are a hard commit gate; unvalidated entries are never written |
| Copied reference drift | `mmc:copiedFrom` + per-origin `COPYING.md`; re-copy is a reviewed change-set (Phase 11) |

---

## 10. References (code-available)

- Voyager (skill library) — https://github.com/MineDojo/Voyager
- ExpeL (experiential learning) — https://github.com/LeapLabTHU/ExpeL
- Reflexion (verbal RL) — https://github.com/noahshinn/reflexion
- Agent Workflow Memory (procedural memory) — https://github.com/zorazrw/agent-workflow-memory
- Case-Based Reasoning — https://github.com/sipemu/case-based-reasoning · CaBRNet — https://github.com/aiser-team/cabrnet
- ANASIME (structure mapping) — https://github.com/Tijl/ANASIME
- DEAP (evolutionary computation) — https://github.com/DEAP/deap
- OpenEvolve (open-ended program evolution) — https://github.com/codelion/openevolve
- Cognitive-architecture structuring — Soar https://github.com/SoarGroup/Soar · ACT-R https://github.com/asmaloney/ACT-R · LIDA https://github.com/CognitiveComputingResearchGroup/lida-framework
- (already integrated) Oxigraph, sqlx, rdf-codec, rdf-shacl, kg-llm, kg-extract, kg-validate, analogy-ir, evolution-ir, learning-ir, mechanism-ir as per parent plan §7
