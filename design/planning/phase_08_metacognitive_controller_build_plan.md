# Phase 8 Build Plan — Metacognitive controller and cognitive-program compiler

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 8 · Codename Metamind (`mm`)

## 1. Objective and scope

Build the central new organ of the system: a **general metacognitive controller** that (a) inspects a
provisional model of a problem, (b) decides *what kind of cognition is needed* and *how much is
sufficient*, and (c) **compiles a temporary cognitive program** — a typed, ordered set of cognitive
operations — then lowers it to an executable nexus computation DAG. The controller is not a checklist
and not a reasoning engine: it allocates cognitive operations, bounds them with a budget, records a
replayable trace, and stops the moment remaining uncertainty is no longer decision-relevant.

In scope for Phase 8:

- The `CognitiveEpisode` working object and the `CognitiveOp` algebra (design §88, §89).
- One broad metacognitive scan (a single LLM pass returning many issues, not twelve separate prompts).
- The escalate-by-tier selector (Tier 0–5) and bounded classification of scan outputs.
- The deterministic `OperationValue` score and its greedy "next operation" selection.
- The **minimum sufficient cognition** loop with explicit `StoppingCondition`s.
- Compilation to a typed `CognitiveProgram`, persistence, and lower- ing to a nexus
  `ComputationDAG`/`ExecutionDocument` via copied-in `monad-executor`/`dsl-compiler`.
- `CognitiveBudget` enforcement (the metacognitive budget from design §43).
- Full traces for replay and for later Phase 11 distillation.

Out of scope (owned by other phases): the actual fairness/risk decisions and the pre-action firewall
(Phase 9); the tool executor and permissions (Phase 10); calibration and self-improvement (Phase 11);
multi-timescale scheduling and the closed loop (Phase 12). Phase 8 *produces* the program and the trace;
other phases consume them.

## 2. Prerequisites and dependencies

| Requires | Why | Artifact |
|---|---|---|
| Phase 1 | stores, event log, ULIDs, `mm-log`, ontology pipeline | `mm-core`, `mm-store-sqlite`, `mm-store-graph`, `mm-eventlog`, `mm-log` |
| Phase 2 | the module + code-metadata contract the new module must satisfy | `mm-codex`, `mmc:` graph |
| Phase 3 | the only sanctioned way to call the LLM (cached, schema-validated, accounted) | `mm-llm::LlmClient`, `StructuredOut<T>` |
| Phase 4 | goal/identity context the episode references | `mm-being::GoalId` |
| Phase 5 | `Recall` retrieval backing | `mm-memory` |
| Phase 6 | epistemic state the episode consumes (`Claim`, `Assumption`, `Prediction`) | `mm-epistemic` |
| Phase 7 | frames, doctrines, techniques the program activates | `mm-library` |

Later phases must not regress this gate; the controller's traces are inputs to Phases 9–12.

## 3. Deliverables (exact paths)

```
crates/mm-metacog/
├── Cargo.toml                 # deps: mm-core, mm-log, mm-llm, mm-library, mm-epistemic, rdf-codec
├── src/lib.rs                 # pub use surface (see §5)
├── src/episode.rs             # CognitiveEpisode + Context
├── src/op.rs                  # CognitiveOp enum + ProgramStep + OpClass
├── src/scan.rs                # ScanRequest/ScanResult schema + LtraceScan client
├── src/tier.rs                # Tier escalation ladder (T0..T5) + selector
├── src/program.rs             # CognitiveProgram + StoppingCondition + compiler
├── src/value.rs               # OperationValue + score() + greedy selection
├── src/budget.rs              # CognitiveBudget + debits + exhaustion
├── src/loop_.rs               # minimum-sufficient-cognition loop
├── src/lower.rs               # to_execution_document -> nexus ComputationDAG
├── src/trace.rs               # ProgramTrace persistence + replay
├── src/rdf.rs                 # ToRdf/FromRdf (rdf-codec) for episode/program/op
└── tests/metacog_tests.rs     # crate-level unit/integration

modules/cognition/metacog/     # THE MODULAR CODE SYSTEM unit for this phase
├── plugin.toml                # name + STABLE uri + capability + owned_by_phase = 8
├── src/lib.rs                 # registers cognition.plan / cognition.op_value
├── manual/module.md           # LLM/operator-readable manual
└── tests/behavior.rs

crates/mm-store-sqlite/migrations/0008_metacog.sql
ontology/shapes/episode.ttl                      # SHACL for mm:CognitiveEpisode/Program/Op
bench/episodes/{trivial,standard,hard}/*.json    # committed benchmark episode corpus
bench/episodes/gold/{programs,operation_value}.jsonl
bench/episodes/operation_value.csv               # golden deterministic arithmetic table
tests/e2e/episode_roundtrip.rs                   # end-to-end run/replay
```

Deliverable CLI surface: `mm-cli episode {run, replay, verify, value, budget-audit, program}`.

## 4. Data model and ontology deltas

### 4.1 SQLite (`migrations/0008_metacog.sql`)

```sql
CREATE TABLE episodes (
    id            TEXT(26) PRIMARY KEY,
    goal          TEXT     NOT NULL,          -- mm:GoalId ULID or free goal text id
    tier          INTEGER  NOT NULL CHECK (tier BETWEEN 0 AND 5),
    stakes        REAL     NOT NULL CHECK (stakes >= 0.0 AND stakes <= 1.0),
    uncertainty   REAL     NOT NULL CHECK (uncertainty >= 0.0 AND uncertainty <= 1.0),
    reversibility REAL     NOT NULL CHECK (reversibility >= 0.0 AND reversibility <= 1.0),
    status        TEXT     NOT NULL,          -- open | programmed | stopped | closed
    budget_json   TEXT     NOT NULL,          -- CognitiveBudget snapshot at open
    trace_id      TEXT(26) NOT NULL,
    created_ulid  TEXT(26) NOT NULL,
    valid_from    INTEGER  NOT NULL,
    valid_until   INTEGER
);

CREATE TABLE programs (
    id                 TEXT(26) PRIMARY KEY,
    episode_id         TEXT(26) NOT NULL REFERENCES episodes(id),
    version            INTEGER  NOT NULL,
    tier               INTEGER  NOT NULL,
    program_json       TEXT     NOT NULL,     -- canonical CognitiveProgram (serde)
    dag_hash           TEXT     NOT NULL,     -- sha256 of canonical lowered DAG
    operation_value_json TEXT   NOT NULL,     -- per-step scores at compile time
    created_ulid       TEXT(26) NOT NULL,
    UNIQUE (episode_id, version)
);

CREATE TABLE program_traces (
    id               TEXT(26) PRIMARY KEY,
    program_id       TEXT(26) NOT NULL REFERENCES programs(id),
    seq              INTEGER  NOT NULL,
    op               TEXT     NOT NULL,       -- CognitiveOp variant tag
    op_class         TEXT     NOT NULL,       -- OpClass (see §5)
    selected         INTEGER  NOT NULL,       -- 1 = executed, 0 = considered and skipped
    value_json       TEXT     NOT NULL,       -- {eer, importance, p_change, cost, score}
    outcome          TEXT     NOT NULL,       -- executed | skipped | failed | stopped
    budget_after_json TEXT    NOT NULL,
    stopping_reason  TEXT,
    at_ulid          TEXT(26) NOT NULL,
    UNIQUE (program_id, seq)
);

CREATE INDEX idx_programs_episode   ON programs(episode_id);
CREATE INDEX idx_traces_program_seq ON program_traces(program_id, seq);
```

### 4.2 RDF / ontology (`ontology/mm.ttl` + `ontology/shapes/episode.ttl`)

New classes (namespace `mm:` = `https://metamind.dev/ontology#`):

- `mm:CognitiveEpisode` ⊑ `mm:Episode`
- `mm:CognitiveProgram`
- `mm:CognitiveOp` (individual per program step)
- `mm:OperationValue` (bounded 0..1 components + `mm:score`)
- `mm:StoppingCondition`

New predicates:

`mm:compiledProgram`, `mm:hasOperation`, `mm:opClass`, `mm:opOrder`, `mm:targets`,
`mm:stoppingCondition`, `mm:budget`, `mm:budgetSpent`, `mm:tier`, `mm:stakes`, `mm:uncertainty`,
`mm:reversibility`, `mm:criticality`, `mm:valueEer`, `mm:valueImportance`, `mm:valueChangeProbability`,
`mm:valueCost`, `mm:score`, `mm:activatedFrame`, `mm:activatedDoctrine`, `mm:activatedTechnique`,
`mm:referenceClass`, `mm:requiredTool`, `mm:evaluationCriterion`.

Graph placement: episodes/programs emit into the named graph
`https://metamind.dev/graph/epistemic` (they are epistemic artifacts about the being's own reasoning),
linked to `/provenance` via `mm:derivedFrom`. Instance IRIs: `<https://metamind.dev/data/{ulid}>`.

`ontology/shapes/episode.ttl` (SHACL, generated where possible with `rdf-shacl`):

- `mm:CognitiveEpisodeShape`: exactly one `mm:goal`, exactly one `mm:tier` in `[0..5]`, ≥0
  `mm:assumption`, one `mm:budget`; `mm:stakes`/`mm:uncertainty`/`mm:reversibility` in `[0,1]`.
- `mm:CognitiveProgramShape`: exactly one `mm:compiledProgram` inverse, ≥1 `mm:hasOperation`, all ops
  ordered by contiguous `mm:opOrder` starting at 1.
- `mm:CognitiveOpShape`: one `mm:opClass` drawn from the enum; `Decide`/`Act` ops MUST target a candidate
  or action id that resolves in `mm-epistemic`/`mm-being`.

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

### 5.1 Core types

```rust
// src/op.rs
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpClass {
    Observe, Recall, Clarify, Classify, Decompose, Compare, Analogize, SearchPrecedent,
    Invert, Predict, Simulate, Verify, Critique, RedTeam, CheckConstraints, CheckAssumptions,
    CheckContradictions, CheckCausality, Simplify, Synthesize, Decide, Act, Measure, Learn,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CognitiveOp {
    Recall { query: String },
    Observe { source: SourceId },
    FormHypothesis { proposition: PropositionId },
    Compare { candidates: Vec<CandidateId> },
    FindSimilar { case: CaseId },
    CheckAssumption { assumption: AssumptionId },
    Verify { claim: ClaimId },
    Invert { goal: GoalId },
    Simulate { scenario: ScenarioId },
    Critique { candidate: CandidateId },
    Simplify { solution: CandidateId },
    Decide { candidates: Vec<CandidateId> },
    Act { action: ActionId },
    Evaluate { outcome: OutcomeId },
    Learn { lesson: LessonId },
}

impl CognitiveOp { pub fn class(&self) -> OpClass; pub fn cost_hint(&self) -> CostClass; }

// src/episode.rs  (mirrors design §88)
pub struct CognitiveEpisode {
    pub id: EpisodeId,
    pub goal: GoalId,
    pub context: Context,
    pub claims: Vec<ClaimId>,
    pub assumptions: Vec<AssumptionId>,
    pub uncertainties: Vec<UncertaintyId>,
    pub constraints: Vec<ConstraintId>,
    pub active_frames: Vec<FrameId>,
    pub active_doctrines: Vec<DoctrineId>,
    pub active_techniques: Vec<TechniqueId>,
    pub candidates: Vec<CandidateId>,
    pub evidence: Vec<EvidenceId>,
    pub risks: Vec<RiskId>,
    pub decision: Option<DecisionId>,
    pub budget: CognitiveBudget,
    pub tier: Tier,
    pub timescale: Timescale,          // design §31; episode = "minutes" scale
}

// src/program.rs  (mirrors design §12)
pub struct ProgramStep { pub order: u8, pub op: CognitiveOp, pub targets: Vec<NodeId>, pub value: OperationValue }
pub struct CognitiveProgram {
    pub id: ProgramId,
    pub episode_id: EpisodeId,
    pub tier: Tier,
    pub steps: Vec<ProgramStep>,
    pub frames: Vec<FrameId>,
    pub doctrines: Vec<DoctrineId>,
    pub techniques: Vec<TechniqueId>,
    pub reference_classes: Vec<RefClassId>,
    pub assumptions_to_verify: Vec<AssumptionId>,
    pub comparisons: Vec<ComparisonSpec>,
    pub candidate_actions: Vec<CandidateId>,
    pub evaluation_criteria: Vec<CriterionId>,
    pub required_tools: Vec<ToolId>,
    pub stopping: Vec<StoppingCondition>,
}

#[derive(Clone, Debug)]
pub enum StoppingCondition {
    RemainingUncertaintyBelow(f64),
    ExpectedValueBelow(f64),
    RequiredConfidenceMet(f64),
    BudgetExhausted,
    IrreversibleActionReached,
}

// src/value.rs  (deterministic arithmetic — design §11, meta_analysis2 §5)
pub struct OperationValue {
    pub expected_error_reduction: f64, // 0..1, bounded classification
    pub decision_importance: f64,      // 0..1
    pub probability_change: f64,       // 0..1
    pub cost: f64,                     // > 0, from logic-planner cost model
}
impl OperationValue {
    /// score = EER * importance * p_change / cost ; cost floored to EPS.
    pub fn score(&self) -> f64;
}

// src/budget.rs
pub struct CognitiveBudget {
    pub max_ops: u32,
    pub max_llm_calls: u32,
    pub max_cost: f64,
    pub max_wall_ms: u64,
    pub spent_ops: u32,
    pub spent_llm_calls: u32,
    pub spent_cost: f64,
    pub spent_wall_ms: u64,
}
impl CognitiveBudget { pub fn debit(&mut self, d: BudgetDebit) -> Result<(), BudgetError>; pub fn exhausted(&self) -> Option<StopCause>; }
```

### 5.2 Controller trait and compiler

```rust
// src/scan.rs — one broad pass, strict structured output
pub struct ScanRequest { pub episode: EpisodeRef, pub context: Context }
pub struct ScanResult {
    pub issues: Vec<ScanIssue>,             // unsupported assumptions, invalid comparisons, ...
    pub uncertainties: Vec<Uncertainty>,    // with materiality 0..1
    pub comparison_requirements: Vec<ComparisonSpec>,
    pub failure_modes: Vec<FailureMode>,
    pub simpler_alternatives: Vec<CandidateId>,
    pub stakes: f64,
    pub irreversibility: f64,
    pub verification_value: f64,
}

pub trait ScanDriver: Send + Sync {         // impl over mm-llm::StructuredOut<ScanResult>
    async fn scan(&self, req: &ScanRequest) -> Result<ScanResult, MetacogError>;
}

// src/program.rs
pub trait ProgramCompiler: Send + Sync {
    fn tier(&self, ep: &CognitiveEpisode, scan: &ScanResult) -> Tier;
    fn compile(&self, ep: &CognitiveEpisode, scan: &ScanResult, budget: &CognitiveBudget)
        -> Result<CognitiveProgram, MetacogError>;
}

// src/loop_.rs — minimum sufficient cognition (design §11; meta_analysis2 §12/§15)
pub trait MetacognitiveController: Send + Sync {
    async fn run(&self, ep: CognitiveEpisode, ctx: Context) -> Result<EpisodeOutcome, MetacogError>;
}
```

### 5.3 DAG lowering

```rust
// src/lower.rs
use nexus_monad_types::{ComputationDAG, ExecutionDocument, NodeOperation, Value};
pub fn to_execution_document(p: &CognitiveProgram, tools: &ToolCatalog)
    -> Result<ExecutionDocument, LowerError>;
/// Deterministic: same program => same ExecutionDocument => same dag_hash.
pub fn dag_hash(p: &CognitiveProgram, tools: &ToolCatalog) -> String; // sha256 hex
```

### 5.4 Plugin contract (`modules/cognition/metacog/plugin.toml`)

```toml
[plugin]
name = "mm-metacog"
uri = "https://metamind.dev/code/module/cognition/metacog"
version = "0.1.0"
description = "Metacognitive scan and cognitive-program compiler."
[metadata]
category = "cognition"
owned_by_phase = 8
capability = "mm:CognitiveControl"
[tbox.functions]
"cognition.plan"     = { source = "handlers::cognition_plan" }
"cognition.op_value" = { source = "handlers::cognition_op_value" }
[monad.operations]
name = "cognition"
arity = 1
[build]
rust_edition = "2021"
```

### 5.5 CLI

- `mm-cli episode run --input <episode.json> [--tier auto|0..5] [--budget max_ops=12,max_cost=0.5]`
- `mm-cli episode replay --episode <ulid> | --all`
- `mm-cli episode verify --artifacts data/artifacts/episodes --gold bench/episodes/gold`
- `mm-cli episode value --gold-table bench/episodes/operation_value.csv`
- `mm-cli episode budget-audit --artifacts data/artifacts/episodes`
- `mm-cli episode program --episode <ulid> --format turtle|json`

## 6. External references copied in and integration

External code is reference-only; the needed parts are copied into `vendor/` as workspace members and
integrated (never a submodule or path dependency). Each copy is recorded with `mmc:copiedFrom`
(origin repo, path, revision, license) during the Phase 2 scan.

| Reference | Copied into | Integrated as |
|---|---|---|
| `decision-ir` (Utility / bounded argmax) | `vendor/rust_symbolic/crates/decision-ir` | deterministic greedy selection in `value.rs`; Phase 9 reuses the full engine |
| `logic-planner` (`Planner`, cost model) | `vendor/rust_symbolic/crates/logic-planner` | `cost_hint` → numeric `OperationValue::cost` |
| `monad-executor`, `dsl-compiler`, `nexus-monad-types`, `monad-scheduler` | `vendor/nexus/…` | `lower.rs` targets `ComputationDAG`/`ExecutionDocument`; deterministic ops execute here |
| `nexus-mpyir` (`MpyirEngine`) | `vendor/nexus/nexus-mpyir` | one-shot LLM-produced DSL blocks used by composite techniques |
| `kg-llm` via `mm-llm` | — (Phase 3) | the single broad scan call and any semantic op execution |
| `rdf-codec` | `vendor/rust_symbolic/crates/rdf-codec` | canonical, deterministic RDF for episode/program/op |

Adaptation rule: nexus types are wrapped behind `mm-metacog` types so the runtime never couples to
`nexus-monad-types` directly; `lower.rs` is the single seam.

## 7. Step-by-step implementation tasks

1. **Scaffold crate + module.** Create `crates/mm-metacog/` and `modules/cognition/metacog/` per the
   contract; add to the workspace members glob; `#![forbid(unsafe_code)]`; run `mm-cli codex scan`.
2. **Implement `op.rs`.** The `CognitiveOp` enum, `OpClass`, `class()`, `cost_hint()`; golden test of
   enum ↔ tag round-trip and class coverage (every `CognitiveOp` maps to exactly one `OpClass`).
3. **Implement `episode.rs`.** `CognitiveEpisode` + `Timescale`; builder that pulls ids from
   `mm-being`/`mm-epistemic`/`mm-library`; validate against `episode.ttl`.
4. **Implement `scan.rs`.** One broad `ScanResult` schema sent to `mm-llm` as `StructuredOut<ScanResult>`;
   reject schema violations; never run independent per-check prompts (design §18 "one broad scan").
5. **Implement `tier.rs`.** The T0–T5 escalation ladder (design §75; meta_analysis2 §13) selected from
   `stakes × uncertainty × irreversibility × novelty`. Tier caps the permitted op set and budget.
6. **Implement `value.rs`.** Bounded, deterministic `OperationValue::score`; a `CostClass → f64` table;
   greedy argmax selection with stable tie-breaks (by `order`, then op tag) for determinism.
7. **Implement `budget.rs`.** `CognitiveBudget` with typed `BudgetDebit`; `debit` returns
   `BudgetError::Exceeded` rather than clamping; `exhausted()` maps to `StoppingCondition::BudgetExhausted`.
8. **Implement `program.rs`.** Compile `ScanResult` → `CognitiveProgram`: activate frames/doctrines/
   techniques from `mm-library` by applicability, add `CheckAssumption` ops for the highest
   `mm:VerificationPriority` assumptions (Phase 6), add `Compare`/`FindSimilar` when the scan requires
   them, and attach stopping conditions.
9. **Implement `loop_.rs`.** The fundamental loop: scan → compile → for each step, select by max
   `OperationValue`, execute, update the provisional model, re-evaluate "is the remaining uncertainty
   decision-relevant?" (design §11 / meta_analysis2 §15), stop on any `StoppingCondition`.
10. **Implement `lower.rs`.** Lower the program to `ExecutionDocument`; deterministic `dag_hash`; a
    round-trip test program → DAG → re-lower → identical hash.
11. **Implement `trace.rs`.** Persist `program_traces` rows in order; `replay(episode)` re-runs compile
    from stored inputs and asserts byte-identical programs and scores.
12. **Implement `rdf.rs`.** `ToRdf`/`FromRdf` for episode/program/op/value; emit into
    `/graph/epistemic`; SHACL-validate before commit.
13. **Wire CLI.** `mm-cli episode run/replay/verify/value/budget-audit/program`.
14. **Register the module capability + tests.** `plugin.toml` → `mmc:Capability` = `mm:CognitiveControl`
    with named tests so `mm-cli codex verify` passes.
15. **Commit the benchmark corpus** (`bench/episodes/{trivial,standard,hard}`, gold programs,
    `operation_value.csv`) with expected outputs.

## 8. Detailed logging requirements

All records go through `mm-log` with the global fields (`ts`, `level`, `target`, `event`, `msg`,
`trace_id`, `span_id`, `episode_id`, `program_id`, `result`, `latency_ms`, `cost`). Phase 8 must emit,
at minimum:

| Event code | Fields |
|---|---|
| `metacog.episode.open` | `episode_id`, `goal`, `timescale`, `budget_json`, `trace_id` |
| `metacog.scan.begin` | `episode_id`, `llm_call_id`, `prompt_hash` |
| `metacog.scan.issue` | `episode_id`, `issue_kind`, `materiality`, `target` |
| `metacog.scan.end` | `episode_id`, `issue_count`, `stakes`, `irreversibility`, `verification_value` |
| `metacog.tier.select` | `episode_id`, `tier`, `reason` |
| `metacog.program.compile` | `program_id`, `episode_id`, `step_count`, `dag_hash`, `required_tools` |
| `metacog.op.consider` | `program_id`, `op`, `op_class`, `value_components_json` |
| `metacog.op.select` | `program_id`, `seq`, `op`, `op_class`, `score`, `tie_break` |
| `metacog.op.execute` | `program_id`, `seq`, `op`, `dag_node_id`, `llm_call_id?` |
| `metacog.op.outcome` | `program_id`, `seq`, `outcome`, `latency_ms`, `cost` |
| `metacog.budget.debit` | `episode_id`, `resource`, `amount`, `remaining` |
| `metacog.budget.exhausted` | `episode_id`, `resource` |
| `metacog.stop` | `episode_id`, `stopping_reason`, `remaining_uncertainty`, `decision_relevant` |
| `metacog.trace.persist` | `program_id`, `row_count`, `first_seq`, `last_seq` |
| `metacog.program.lower` | `program_id`, `dag_hash`, `node_count` |
| `metacog.replay` | `episode_id`, `match` (bool), `diff_count` |

The `trace_id` is the episode ULID so `mm-cli logs trace <episode-ulid>` reconstructs the whole
deliberation. Audit records (one per `metacog.op.execute` and per `metacog.trace.persist`) are immutable
and sequence-numbered. `mm-cli logs verify` must pass.

## 9. Testing plan

- **Unit.** `op.rs` class coverage; `value.rs` score formula and tie-break stability; `budget.rs` rejects
  over-debits; `tier.rs` ladder boundaries.
- **Property (`proptest`).** `score` is monotone increasing in `eer`/`importance`/`p_change` and
  decreasing in `cost`; compile is deterministic (same `ScanResult` → identical `CognitiveProgram`);
  budget spent never exceeds max.
- **Golden / replay.** `bench/episodes/{trivial,standard,hard}` compile to golden programs;
  `program_traces` replay byte-identically; `dag_hash` stable across runs and re-parses.
- **Arithmetic golden table.** `bench/episodes/operation_value.csv` columns
  `eer,importance,p_change,cost,expected_score`; `mm-cli episode value` must match exactly.
- **Minimum sufficient cognition.** Every trivial episode compiles to ≤ `k` ops (k committed in
  `bench/episodes/thresholds.json`); a hard episode compiles to more, and both stay within budget.
- **Scan schema.** Malformed / over-broad `ScanResult` payloads are rejected, not coerced (Phase 3 rule).
- **Ontology conformance.** Emitted episode/program graphs pass `episode.ttl` SHACL with 0 violations;
  a deliberately malformed program (ops out of order, tier 7) fails with ≥1 violation.
- **End-to-end.** `tests/e2e/episode_roundtrip.rs` runs one episode, persists, replays, and asserts
  identical program + trace.
- **Module contract.** `mm-cli codex verify` stays green; capability `mm:CognitiveControl` names tests.

## 10. Pass gate

Run as plain commands; earlier phase gates must still pass; each includes `mm-cli logs verify`.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Typed, schema-valid programs for the benchmark episode set
mm-cli episode run --input bench/episodes/standard --out data/artifacts/episodes
mm-cli episode verify --artifacts data/artifacts/episodes --gold bench/episodes/gold

# Deterministic OperationValue arithmetic
mm-cli episode value --gold-table bench/episodes/operation_value.csv   # exact match

# Minimum sufficient cognition + budgets
mm-cli episode budget-audit --artifacts data/artifacts/episodes --thresholds bench/episodes/thresholds.json

# Replay determinism
mm-cli episode replay --all --compare data/artifacts/episodes

mm-cli codex verify
mm-cli graph validate --graph epistemic
mm-cli logs verify
```

Objective criteria:

1. `cargo build`/`clippy` clean (zero warnings) and `cargo test` green.
2. Every episode in the committed set emits a typed, SHACL-valid `CognitiveProgram` with ≥1 step and
   correct ordering.
3. `mm-cli episode value` matches `operation_value.csv` exactly (all rows).
4. `budget-audit` reports **no** episode exceeded any budget dimension, and every trivial episode used
   ≤ `k` operations.
5. `episode replay --all` reproduces byte-identical programs, scores, and traces (`diff_count == 0`).
6. `mm-cli codex verify`, `mm-cli graph validate --graph epistemic`, and `mm-cli logs verify` all exit 0.

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Controller degenerates into a fixed checklist | Tier selection + greedy `OperationValue` must be data-dependent; benchmark includes episodes whose programs differ materially |
| Overthinking (paralysis by metacognition) | Hard budget + `StoppingCondition`s; trivial-episode ≤ k gate |
| Underthinking on high-stakes episodes | Tier ladder raises required verification with stakes/irreversibility; `< k` check is scoped to trivial episodes only |
| Non-determinism from LLM scan or cost table | Deterministic costs + stable tie-breaks; scan is one cached `StructuredOut`; golden replay gates |
| Nexus type coupling | `lower.rs` is the only seam; all other code uses `mm-metacog` types |
| Scan produces many low-value issues | Materiality filter at §5.2 + ranking by `OperationValue`; only material issues become ops |
| Trace table growth | Operational traces retention-bounded; only audit rows (`op.execute`, `trace.persist`) are immutable; Phase 12 GC owns compaction |

## 12. Design traceability

| Design section | What it supplies to Phase 8 |
|---|---|
| `digital_mind_design1.md` §11 | The metacognitive controller, the scan, minimum sufficient cognition |
| §12 | Cognitive program / reasoning compiler and its outputs |
| §31 | Multi-timescale cognition (`Timescale` field; episode = minutes scale) |
| §43 | Cognitive budgeting (metacognitive budget) |
| §75–76 | Reasoning-depth routing, tier ladder, variable number of LLM passes/roles |
| §77–79 | Self-criticism, red team, verification strategy (as selectable ops, not fixed passes) |
| §87–90 | Ontology classes, runtime model, typed `CognitiveOp`s, backend mapping |
| §108–110 | Guardrails (don't overbuild), the invariant, the final architecture |
| `meta_analysis2.md` §1–§8 | Cognitive state, scan, triage, op toolbox, information gain, issue graph, criticality |
| `meta_analysis2.md` §11–§15 | Controller-as-compiler, minimum sufficient cognition, escalation ladder, budgets, core loop |
| `meta_analysis1.md` §1–§3 | Budget framing and event-triggered introspection (trace inputs to Phase 11) |
| `thought_systems1.md` §14–§15 | Technique-selection probability and applicability profiles feeding op selection |
