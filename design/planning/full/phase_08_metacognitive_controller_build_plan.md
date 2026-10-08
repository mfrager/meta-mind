# Phase 8 — Metacognitive Controller and Cognitive-Program Compiler (Extended Build Plan)

> Extended from: `design/planning/phase_08_metacognitive_controller_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 8 · Codename Metamind (`mm`)

This revision keeps Phase 8's scope and gate, but locks in the mechanisms from current, code-available
work: reasoning as a **graph of operations**, deliberate search over thoughts, **budget-forced** test-time
compute, self-critique/refine, automatic reasoning-structure selection, and parallel DAG decomposition.

---

## 0. Research foundation (code-available)

Only ideas whose source is available are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| Reasoning as a graph of operations, executed with an LLM | Graph of Thoughts — https://github.com/spcl/graph-of-thoughts | `Graph of Operations` (GoO) structure + an executor that aggregates/branches | A `CognitiveProgram` is an `OpGraph` (nodes = ops, edges = data deps), not a flat list |
| Deliberate search over intermediate thoughts with self-evaluation | Tree of Thoughts — https://github.com/princeton-nlp/tree-of-thought-llm | Branching expansion + `evaluate` of partial states + backtracking | `Search` is an **optional** op available at higher tiers; never forced |
| Budget forcing for adaptive test-time compute | s1 — https://github.com/simplescaling/s1 | "Force" extra/less compute against a budget; stop when enough | `BudgetForcer` raises/lowers allowed ops from `OperationValue` vs budget; implements minimum sufficient cognition |
| Taxonomy of compute policies | Awesome-Inference-Time-Scaling — https://github.com/ThreeSR/Awesome-Inference-Time-Scaling | Catalogue of policies (parallel sampling, sequential revise, tree/graph search) | The tier ladder selects a *policy*, not a fixed script |
| Iterative generate → critique → refine | Self-Refine — https://github.com/madaan/self-refine | Self-critique and refinement as first-class ops with a stopping test | `Critique`/`Simplify` ops with their own `OperationValue` |
| LLM composes its own reasoning structure from atomic modules | Self-Discover — https://github.com/catid/self-discover | Select + adapt reasoning modules into a task-specific structure | Program compilation selects frames/techniques and *composes* them per episode |
| Programs/prompts as optimizable modules | DSPy — https://github.com/stanfordnlp/dspy | Modular program representation that can later be optimized | Programs/ops/traces are recorded so Phase 11 can optimize them; Phase 8 only records |
| Parallel DAG/task planning | LLM Compiler — https://github.com/SqueezeBits/llm-compiler | Split a plan into a dependency DAG and run independent nodes in parallel | `lower()` emits a `ComputationDAG`; the scheduler runs independent ops in parallel |
| Interleaved reason + act | ReAct — https://github.com/ysymyth/ReAct | The reason→act loop as a node type | `Act` follows a `Decide` in the graph with an explicit dependency edge |
| Self-* operation taxonomy | Self-* survey index — https://github.com/iaar-shanghai/icsfsurvey | Vocabulary of self-correct/refine/improve ops to enumerate | `OpClass` enum is derived from this taxonomy |

---

## 1. Objective and scope

Build the system's executive: a **metacognitive controller** that (a) inspects a provisional model of a
problem, (b) decides *what kind of cognition is needed and how much is sufficient*, and (c) compiles a
temporary **cognitive program** — a typed graph of cognitive operations — then lowers it to an executable
nexus computation DAG. It is neither a checklist nor a reasoning engine: it allocates cognition, bounds it
with a budget, records a replayable trace, and **stops the moment remaining uncertainty is no longer
decision-relevant**.

**In scope.** `CognitiveEpisode`; the `CognitiveOp` algebra and `OpClass` taxonomy; one broad metacognitive
scan; the tier/policy selector; the deterministic `OperationValue` score and greedy selection; the
minimum-sufficient-cognition loop with explicit stopping conditions; `BudgetForcer`; compilation to a typed
program and lowering to `ExecutionDocument` (`dag_hash`); `CognitiveBudget`; replayable traces.

**Out of scope (other phases).** Firewall/risk decisions (9); tool execution/permissions (10); calibration
and self-improvement (11); multi-timescale scheduling and the closed loop (12). Phase 8 *produces* the
program and trace; other phases consume them.

**Definition of done:** the §8 pass gate runs green as plain commands.

---

## 2. Architecture

The controller is a compiler: `episode + scan → program graph → budgeted execution → trace`.

```
            goal · context · epistemic state · frames/techniques · budget
                                     │
                                 [ SCAN ]         one broad pass (StructuredOut<ScanResult>)
                                     │
                              [ TRIER ]            stakes × uncertainty × irreversibility × novelty → Tier 0..5
                                     │
                          [ PROGRAM COMPILER ]     select frames/techniques (Self-Discover); add ops;
                                     │             build OpGraph; attach StoppingConditions
                                     │
                     [ BudgetForcer / SELECT ]     score every candidate op; force compute up/down
                                     │
                     [ LOWER → ExecutionDocument ] Graph-of-Operations → ComputationDAG (+ dag_hash)
                                     │
                       [ EXECUTE (nexus DAG) ]     independent ops run in parallel (LLM Compiler style)
                                     │
                       [ PROVISIONAL MODEL UPDATE ] re-ask: is remaining uncertainty decision-relevant?
                                     │
                          stop ◄────────┴────────► continue (next op)      [ TRACE → SQLite + /epistemic ]
```

**Invariants**

1. **Spend the least cognition that can change the decision.** No op runs without a positive marginal
   `OperationValue` or an un-met stopping condition.
2. **One scan, many issues.** The scan is a single structured LLM pass (§18 "one broad scan"), never a
   set of independent per-check prompts.
3. **Deterministic selection.** Costs and scores are deterministic; ties break by `(order, op tag)`.
4. **Program is a graph.** Steps form an `OpGraph` with explicit dependencies; the DAG engine may execute
   independent nodes in parallel.
5. **Budget is enforced, never clamped.** Over-debit returns `BudgetError::Exceeded` → a stop cause.
6. **Every deliberation is replayable.** Same inputs + trace ⇒ byte-identical program and scores.

---

## 3. Deliverables and workspace layout

```
crates/mm-metacog/
├── Cargo.toml                 # deps: mm-core, mm-log, mm-llm, mm-library, mm-epistemic, rdf-codec
├── src/lib.rs                 # pub use surface (§4.5)
├── src/episode.rs             # CognitiveEpisode, Context, Timescale
├── src/op.rs                  # CognitiveOp, OpClass, CostClass
├── src/graph.rs               # OpGraph, OpNode, edges, topological order
├── src/scan.rs                # ScanRequest/ScanResult + ScanDriver over StructuredOut
├── src/tier.rs                # Tier 0..5 policy ladder + selector
├── src/program.rs             # ProgramCompiler, CognitiveProgram, StoppingCondition
├── src/value.rs               # OperationValue + score() + greedy argmax + tie-break
├── src/budget.rs              # CognitiveBudget, BudgetForcer, BudgetDebit/BudgetError
├── src/loop_.rs               # minimum-sufficient-cognition loop
├── src/lower.rs               # to_execution_document() -> ComputationDAG + dag_hash
├── src/trace.rs               # ProgramTrace persistence + replay
├── src/rdf.rs                 # ToRdf/FromRdf (rdf-codec) for episode/program/op
└── tests/metacog_tests.rs

modules/cognition/metacog/     # THE MODULAR CODE SYSTEM unit for this phase
├── plugin.toml                # stable uri + capability mm:CognitiveControl + owned_by_phase = 8
├── src/lib.rs                 # registers cognition.plan / cognition.op_value
├── manual/module.md
└── tests/behavior.rs

crates/mm-store-sqlite/migrations/0008_metacog.sql
ontology/shapes/episode.ttl
bench/episodes/{trivial,standard,hard}/*.json
bench/episodes/gold/{programs,operation_value}.jsonl
bench/episodes/operation_value.csv
bench/episodes/thresholds.json
tests/e2e/episode_roundtrip.rs
```

CLI: `mm-cli episode {run, replay, verify, value, budget-audit, program}`.

---

## 4. Detailed specifications

### 4.1 Operation algebra and graph

```rust
// src/op.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpClass {
    Observe, Recall, Clarify, Classify, Decompose, Compare, Analogize, SearchPrecedent,
    Invert, Predict, Simulate, Verify, Critique, Refine, RedTeam, Search,
    CheckConstraints, CheckAssumptions, CheckContradictions, CheckCausality,
    Simplify, Synthesize, Decide, Act, Measure, Learn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostClass { Free, Cheap, Moderate, Expensive, Tool }   // -> f64 via COST_TABLE

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
    Refine { candidate: CandidateId },        // Self-Refine
    Search { over: SearchSpace, budget: u8 }, // ToT-style, tier-gated
    Simplify { solution: CandidateId },
    Decide { candidates: Vec<CandidateId> },
    Act { action: ActionId },                 // ReAct: depends on a Decide
    Evaluate { outcome: OutcomeId },
    Learn { lesson: LessonId },
}
impl CognitiveOp { pub fn class(&self) -> OpClass; pub fn cost_class(&self) -> CostClass; }
```

```rust
// src/graph.rs — reasoning as a Graph of Operations (GoT)
pub type NodeId = u16;
pub struct OpNode { pub id: NodeId, pub op: CognitiveOp, pub value: OperationValue }
pub struct OpGraph { pub nodes: Vec<OpNode>, pub edges: Vec<(NodeId, NodeId)> } // edges = data deps
impl OpGraph {
    pub fn topological(&self) -> Vec<NodeId>;          // deterministic (by id on ties)
    pub fn parallel_layers(&self) -> Vec<Vec<NodeId>>; // LLM Compiler style: run layer in parallel
    pub fn add(&mut self, op: CognitiveOp, after: &[NodeId]) -> NodeId;
}
```

### 4.2 Episode and context (design §88)

```rust
// src/episode.rs
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
    pub timescale: Timescale,     // design §31; an episode is the "minutes" scale
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tier { T0, T1, T2, T3, T4, T5 }   // policy ladder
```

### 4.3 Scan, tier, value, budget

```rust
// src/scan.rs — ONE broad pass, strict schema; violations rejected (Phase 3 rule)
pub struct ScanRequest { pub episode: EpisodeRef, pub context: Context }
pub struct ScanResult {
    pub issues: Vec<ScanIssue>,
    pub uncertainties: Vec<Uncertainty>,
    pub comparison_requirements: Vec<ComparisonSpec>,
    pub failure_modes: Vec<FailureMode>,
    pub simpler_alternatives: Vec<CandidateId>,
    pub stakes: f64, pub irreversibility: f64, pub verification_value: f64,
}
pub trait ScanDriver: Send + Sync {
    async fn scan(&self, req: &ScanRequest) -> Result<ScanResult, MetacogError>;
}

// src/tier.rs — a compute *policy* is selected, not a script (Awesome-ITSc taxonomy)
impl Tier {
    pub fn select(ep: &CognitiveEpisode, scan: &ScanResult) -> Tier;  // f(stakes, uncertainty, irreversibility, novelty)
    pub fn policy(self) -> ComputePolicy;   // allowed OpClass set + budget multipliers + parallel search width
}

// src/value.rs — deterministic arithmetic (design §11; meta_analysis2 §5)
pub struct OperationValue {
    pub expected_error_reduction: f64, // 0..1
    pub decision_importance: f64,      // 0..1
    pub probability_change: f64,       // 0..1
    pub cost: f64,                     // > 0
}
impl OperationValue {
    pub const EPS: f64 = 1e-9;
    /// score = EER * importance * p_change / max(cost, EPS)
    pub fn score(&self) -> f64 { self.expected_error_reduction * self.decision_importance
        * self.probability_change / self.cost.max(Self::EPS) }
}
pub fn select_next(graph: &OpGraph, ready: &[NodeId]) -> Option<NodeId>; // max score, tie -> (id)

// src/budget.rs — budget forcing (s1)
pub struct CognitiveBudget {
    pub max_ops: u32, pub max_llm_calls: u32, pub max_cost: f64, pub max_wall_ms: u64,
    pub spent_ops: u32, pub spent_llm_calls: u32, pub spent_cost: f64, pub spent_wall_ms: u64,
}
impl CognitiveBudget {
    pub fn debit(&mut self, d: BudgetDebit) -> Result<(), BudgetError>;   // never clamps
    pub fn exhausted(&self) -> Option<StopCause>;
}
pub struct BudgetForcer { pub headroom: f64 }          // 0..1 fraction of budget held in reserve
impl BudgetForcer {
    /// s1-style: if score density is high and headroom allows, permit an extra op;
    /// if the remaining budget is low, forbid low-value ops. Deterministic.
    pub fn permit(&self, next: f64, budget: &CognitiveBudget, policy: ComputePolicy) -> Decision;
}
```

### 4.4 Program, compiler, loop, lowering

```rust
// src/program.rs
pub enum StoppingCondition {
    RemainingUncertaintyBelow(f64), ExpectedValueBelow(f64),
    RequiredConfidenceMet(f64), BudgetExhausted, IrreversibleActionReached,
}
pub struct ProgramStep { pub order: u8, pub node: NodeId, pub op: CognitiveOp, pub value: OperationValue }
pub struct CognitiveProgram {
    pub id: ProgramId, pub episode_id: EpisodeId, pub tier: Tier,
    pub graph: OpGraph, pub steps: Vec<ProgramStep>,
    pub frames: Vec<FrameId>, pub doctrines: Vec<DoctrineId>, pub techniques: Vec<TechniqueId>,
    pub reference_classes: Vec<RefClassId>, pub assumptions_to_verify: Vec<AssumptionId>,
    pub comparisons: Vec<ComparisonSpec>, pub candidate_actions: Vec<CandidateId>,
    pub evaluation_criteria: Vec<CriterionId>, pub required_tools: Vec<ToolId>,
    pub stopping: Vec<StoppingCondition>,
}
pub trait ProgramCompiler: Send + Sync {
    fn compile(&self, ep: &CognitiveEpisode, scan: &ScanResult, budget: &CognitiveBudget)
        -> Result<CognitiveProgram, MetacogError>;   // Self-Discover: compose frames/techniques per episode
}

// src/loop_.rs
pub trait MetacognitiveController: Send + Sync {
    async fn run(&self, ep: CognitiveEpisode, ctx: Context) -> Result<EpisodeOutcome, MetacogError>;
}
// scan -> tier -> compile -> repeat { select by max OperationValue; BudgetForcer.permit;
//   execute ready layer (parallel); update provisional model; evaluate stopping } -> trace

// src/lower.rs — GoO -> ComputationDAG; deterministic
pub fn to_execution_document(p: &CognitiveProgram, tools: &ToolCatalog)
    -> Result<ExecutionDocument, LowerError>;
pub fn dag_hash(p: &CognitiveProgram, tools: &ToolCatalog) -> String; // sha256 hex; stable
```

### 4.5 SQLite (`crates/mm-store-sqlite/migrations/0008_metacog.sql`)

```sql
CREATE TABLE episodes (
    id            TEXT(26) PRIMARY KEY,
    goal          TEXT NOT NULL,
    tier          INTEGER NOT NULL CHECK (tier BETWEEN 0 AND 5),
    stakes        REAL NOT NULL CHECK (stakes BETWEEN 0.0 AND 1.0),
    uncertainty   REAL NOT NULL CHECK (uncertainty BETWEEN 0.0 AND 1.0),
    reversibility REAL NOT NULL CHECK (reversibility BETWEEN 0.0 AND 1.0),
    status        TEXT NOT NULL,          -- open | programmed | stopped | closed
    budget_json   TEXT NOT NULL,
    trace_id      TEXT(26) NOT NULL,
    created_ulid  TEXT(26) NOT NULL,
    system_from   TEXT NOT NULL, system_to TEXT,
    valid_from    TEXT NOT NULL, valid_to  TEXT
);
CREATE TABLE programs (
    id TEXT(26) PRIMARY KEY,
    episode_id TEXT(26) NOT NULL REFERENCES episodes(id),
    version INTEGER NOT NULL,
    tier INTEGER NOT NULL,
    program_json TEXT NOT NULL,           -- canonical serde (nodes+edges+steps)
    dag_hash TEXT NOT NULL,
    operation_value_json TEXT NOT NULL,
    created_ulid TEXT(26) NOT NULL,
    UNIQUE (episode_id, version)
);
CREATE TABLE program_traces (
    id TEXT(26) PRIMARY KEY,
    program_id TEXT(26) NOT NULL REFERENCES programs(id),
    seq INTEGER NOT NULL,
    node_id INTEGER NOT NULL,
    op TEXT NOT NULL,
    op_class TEXT NOT NULL,
    selected INTEGER NOT NULL,            -- 1 executed, 0 considered+skipped
    value_json TEXT NOT NULL,             -- {eer, importance, p_change, cost, score}
    outcome TEXT NOT NULL,                -- executed | skipped | failed | stopped
    budget_after_json TEXT NOT NULL,
    stopping_reason TEXT,
    at_ulid TEXT(26) NOT NULL,
    UNIQUE (program_id, seq)
);
CREATE INDEX idx_programs_episode ON programs(episode_id);
CREATE INDEX idx_traces_program_seq ON program_traces(program_id, seq);
```

### 4.6 Ontology and SHACL (`ontology/shapes/episode.ttl`)

Classes: `mm:CognitiveEpisode ⊑ mm:Episode`, `mm:CognitiveProgram`, `mm:CognitiveOp`,
`mm:OperationValue`, `mm:StoppingCondition`.
Predicates: `mm:compiledProgram, mm:hasOperation, mm:opClass, mm:opOrder, mm:opDependsOn, mm:targets,
mm:stoppingCondition, mm:budget, mm:budgetSpent, mm:tier, mm:stakes, mm:uncertainty, mm:reversibility,
mm:valueEer, mm:valueImportance, mm:valueChangeProbability, mm:valueCost, mm:score, mm:activatedFrame,
mm:activatedDoctrine, mm:activatedTechnique, mm:referenceClass, mm:requiredTool, mm:evaluationCriterion`.
Emitted into `https://metamind.dev/graph/epistemic`, linked to `/provenance` via `mm:derivedFrom`;
instance IRIs `<https://metamind.dev/data/{ulid}>`.

Shapes: `CognitiveEpisodeShape` (exactly one goal/tier in `[0..5]`/budget; stakes, uncertainty,
reversibility in `[0,1]`); `CognitiveProgramShape` (≥1 op, contiguous `mm:opOrder` from 1, acyclic
`mm:opDependsOn`); `CognitiveOpShape` (`Decide`/`Act` targets resolve in `mm-epistemic`/`mm-being`).

### 4.7 Module and CLI

`modules/cognition/metacog/plugin.toml`: `uri = "https://metamind.dev/code/module/cognition/metacog"`,
`owned_by_phase = 8`, `capability = "mm:CognitiveControl"`, functions `cognition.plan` /
`cognition.op_value`, with named tests.

CLI:
- `mm-cli episode run --input <episode.json> [--tier auto|0..5] [--budget max_ops=12,max_cost=0.5]`
- `mm-cli episode replay --episode <ulid> | --all [--compare <dir>]`
- `mm-cli episode verify --artifacts <dir> --gold bench/episodes/gold`
- `mm-cli episode value --gold-table bench/episodes/operation_value.csv`
- `mm-cli episode budget-audit --artifacts <dir> --thresholds bench/episodes/thresholds.json`
- `mm-cli episode program --episode <ulid> --format turtle|json`

---

## 5. Build sequence

1. **Scaffold** `crates/mm-metacog/` + `modules/cognition/metacog/`; workspace members; `codex scan`. *Check:* `cargo build -p mm-metacog`.
2. **`op.rs`** enum + `OpClass` + `CostClass` + `class()`/`cost_class()`. *Check:* golden enum↔tag round-trip; class coverage.
3. **`graph.rs`** `OpGraph` + deterministic topo order + `parallel_layers`. *Check:* acyclic/order property tests.
4. **`episode.rs`** `CognitiveEpisode` + builder pulling ids from `mm-being`/`mm-epistemic`/`mm-library`. *Check:* validates against `episode.ttl`.
5. **`scan.rs`** one broad `ScanResult` via `mm-llm::StructuredOut`; reject violations. *Check:* malformed scan rejected, not coerced.
6. **`tier.rs`** T0–T5 selector + `ComputePolicy`. *Check:* boundary table goldens.
7. **`value.rs`** deterministic `score()` + `CostClass→f64` + stable tie-break. *Check:* monotonicity proptest; `operation_value.csv` exact.
8. **`budget.rs`** `CognitiveBudget` + `BudgetForcer`. *Check:* over-debit errors; forcer never overspends.
9. **`program.rs`** compile scan→program (Self-Discover composition; add `CheckAssumption` for top `mm:VerificationPriority`; add `Compare`/`FindSimilar`/`Search` per policy; attach stopping). *Check:* determinism proptest.
10. **`loop_.rs`** scan→tier→compile→select→execute-ready-layer→update→stop. *Check:* trivial ≤ k ops; budget never exceeded.
11. **`lower.rs`** GoO→`ExecutionDocument` + `dag_hash`. *Check:* lower→re-lower identical hash.
12. **`trace.rs`** persist ordered traces; `replay()` asserts byte-identical program/scores. *Check:* `diff_count == 0`.
13. **`rdf.rs`** `ToRdf`/`FromRdf`; emit to `/epistemic`; SHACL-validate before commit. *Check:* 0 violations clean, ≥1 broken.
14. **CLI** wire `episode {run,replay,verify,value,budget-audit,program}`. *Check:* each subcommand exits 0 on fixtures.
15. **Bootstrap module + capability tests.** *Check:* `codex verify` green.
16. **Commit benchmark corpus** (`bench/episodes/*`, gold programs, `operation_value.csv`, `thresholds.json`). *Check:* `episode verify` green.

---

## 6. Logging and observability

All records carry the global fields (`ts, level, target, event, msg, trace_id, span_id, episode_id,
program_id, result, latency_ms, cost`). `trace_id` = the episode ULID, so `mm-cli logs trace <ulid>`
reconstructs the whole deliberation.

| Event code | Fields |
|---|---|
| `metacog.episode.open` | `episode_id`, `goal`, `timescale`, `budget_json`, `trace_id` |
| `metacog.scan.begin` / `.issue` / `.end` | `llm_call_id, prompt_hash` / `issue_kind, materiality, target` / `issue_count, stakes, irreversibility, verification_value` |
| `metacog.tier.select` | `episode_id`, `tier`, `policy`, `reason` |
| `metacog.program.compile` | `program_id`, `episode_id`, `node_count`, `edge_count`, `dag_hash`, `required_tools` |
| `metacog.op.consider` | `program_id`, `node_id`, `op`, `op_class`, `value_components_json` |
| `metacog.op.select` | `program_id`, `seq`, `node_id`, `op`, `score`, `tie_break` |
| `metacog.op.execute` | `program_id`, `seq`, `node_id`, `dag_node_id`, `llm_call_id?` |
| `metacog.op.outcome` | `program_id`, `seq`, `outcome`, `latency_ms`, `cost` |
| `metacog.budget.debit` / `.exhausted` | `episode_id, resource, amount, remaining` / `episode_id, resource` |
| `metacog.force.decide` | `episode_id`, `decision` (permit/add/reserve), `score`, `headroom` |
| `metacog.stop` | `episode_id`, `stopping_reason`, `remaining_uncertainty`, `decision_relevant` |
| `metacog.trace.persist` | `program_id`, `row_count`, `first_seq`, `last_seq` |
| `metacog.program.lower` | `program_id`, `dag_hash`, `node_count` |
| `metacog.replay` | `episode_id`, `match`, `diff_count` |

Audit records (immutable, sequence-numbered): one per `metacog.op.execute` and per `metacog.trace.persist`.
Operational traces are retention-bounded; `mm-cli logs verify` must pass.

---

## 7. Testing

| Type | Test | Assertion |
|---|---|---|
| Unit | `op.rs` class coverage | every `CognitiveOp` maps to exactly one `OpClass`; cost table total |
| Unit | `budget.rs` | over-debit returns `Exceeded`, never clamps; `BudgetForcer` respects headroom |
| Unit | `tier.rs` | boundary goldens for the selector |
| Property (`proptest`) | `score` monotonicity | ↑ in eer/importance/p_change, ↓ in cost |
| Property | compile determinism | same `ScanResult` ⇒ identical `CognitiveProgram` |
| Property | graph | `topological`/`parallel_layers` deterministic and acyclic |
| Golden | programs | `bench/episodes/{trivial,standard,hard}` ⇒ gold programs |
| Golden | arithmetic | `bench/episodes/operation_value.csv` exact match |
| Replay | traces | byte-identical program + scores + traces (`diff_count == 0`) |
| Min-cognition | thresholds | trivial ≤ `k` ops; hard > trivial; both within budget |
| Scan schema | adversarial | malformed/over-broad `ScanResult` rejected |
| Conformance | SHACL | clean 0 violations; malformed (tier 7, ops out of order) ≥1 |
| E2E | `episode_roundtrip.rs` | run→persist→replay identical |
| Module | `codex verify` | capability `mm:CognitiveControl` names tests |

Fixtures: `bench/episodes/*`, `bench/episodes/gold/*`, `bench/episodes/operation_value.csv`,
`bench/episodes/thresholds.json`.

---

## 8. Pass gate

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

mm-cli episode run --input bench/episodes/standard --out data/artifacts/episodes
mm-cli episode verify --artifacts data/artifacts/episodes --gold bench/episodes/gold
mm-cli episode value --gold-table bench/episodes/operation_value.csv         # exact match
mm-cli episode budget-audit --artifacts data/artifacts/episodes --thresholds bench/episodes/thresholds.json
mm-cli episode replay --all --compare data/artifacts/episodes

mm-cli codex verify
mm-cli graph validate --graph epistemic
mm-cli logs verify
```

Objective criteria:

1. Build/clippy/tests exit 0 with **zero warnings**.
2. Every episode emits a typed, SHACL-valid program with ≥1 op and correct ordering/acyclic deps.
3. `episode value` matches `operation_value.csv` exactly (all rows).
4. `budget-audit` reports **no** episode exceeded any budget dimension and every trivial episode used ≤ `k` ops.
5. `episode replay --all` reproduces byte-identical programs, scores, and traces (`diff_count == 0`).
6. `codex verify`, `graph validate --graph epistemic`, and `logs verify` exit 0.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Degenerates into a fixed checklist | Tier policy + greedy `OperationValue` are data-dependent; benchmark includes materially different episodes |
| Overthinking | Hard budget + `BudgetForcer` reserve + stopping conditions + trivial ≤ k gate |
| Underthinking on high-stakes episodes | Tier ladder raises required verification with stakes/irreversibility; `< k` scoped to trivial only |
| Non-determinism (LLM scan, cost table) | Deterministic costs; stable tie-breaks; scan is one cached `StructuredOut`; golden replay gates |
| Graph search explodes | `Search` is tier-gated and budget-bounded; width from `ComputePolicy` |
| Nexus type coupling | `lower.rs` is the only seam; all other code uses `mm-metacog` types |
| Trace growth | Operational traces retention-bounded; only audit rows immutable; Phase 12 GC owns compaction |

---

## 10. References (code-available)

- Graph of Thoughts — https://github.com/spcl/graph-of-thoughts
- Tree of Thoughts — https://github.com/princeton-nlp/tree-of-thought-llm
- s1 (budget forcing) — https://github.com/simplescaling/s1
- Awesome-Inference-Time-Scaling — https://github.com/ThreeSR/Awesome-Inference-Time-Scaling
- Self-Refine — https://github.com/madaan/self-refine
- Self-Discover — https://github.com/catid/self-discover
- DSPy — https://github.com/stanfordnlp/dspy
- LLM Compiler — https://github.com/SqueezeBits/llm-compiler
- ReAct — https://github.com/ysymyth/ReAct
- Self-* survey index — https://github.com/iaar-shanghai/icsfsurvey
