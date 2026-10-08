# Phase 12 Build Plan — Self-bootstrap closed loop (the functional prototype)

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 12 · Codename Metamind (`mm`)
> Source designs: §62–69, §85–86, §103–110 of `design/digital_mind_design1.md`; §11 Pi contract of the
> parent plan; supporting specs `companion_loop1.md`, `companion_loop2.md`, `personality_simulator3.md`,
> `meta_analysis2.md`.

## 1. Objective and scope

Close the developmental loop so the being can **continue its own design and build process**. Given a
novel design goal, the runtime drives the full cycle — experience → event log → meta-analysis →
capability gap → change set → self-engineering → test/benchmark → shadow → promotion gate → new
version — and the result is a working, tested, promoted, hot-loadable capability, produced **without
human code edits**, with complete RDF provenance and deterministic replay.

This phase adds four organs on top of Phases 1–11:
1. **Closed-loop controller** (`mm-runtime`) with deterministic multi-timescale scheduling and budget
   enforcement (§31, §103).
2. **Self-model divergence** — Actual vs Model vs Ideal, with periodic reports (§62).
3. **Architectural debt detection + cognitive garbage collection**, protecting the immutable
   developmental ledger (§85–86).
4. **Self-design** — the runtime writes/revises its own Markdown design docs and phase plan, registered
   in the code graph, every revision a `ChangeSet` (§67–69, §105).

**In scope:** the loop controller, timescale scheduler, self-model, debt/GC, design writer, module
hot-loader, the qualification scenario, and end-to-end replay.
**Out of scope:** opening new mutation privileges beyond Phase 11 (identity invariants stay immutable),
and any change to `mm:Invariant` resources (§4 invariant enforcement remains deterministic).

## 2. Prerequisites and dependencies

| Requires | Artifact | Why |
|---|---|---|
| Phase 1 | `mm-eventlog` replay, `mm-store-*`, `mm-log`, event-log sequence | loop replay and audit |
| Phase 2 | `mm-codex` code graph, stable module URIs, `codex verify` | design revisions and module loads register here |
| Phase 4 | `mm-being` identity invariants, budgets | invariant checks and budget envelopes |
| Phase 7–8 | `mm-library`, `mm-metacog` program compiler | gap → program → change plan |
| Phase 9 | `mm-firewall`, `mm-decision` | promotion-gate bounded judgments |
| Phase 10 | `mm-tools` executor, sandbox, rollback | safe execution of change sets |
| Phase 11 | `mm-metaanalysis`, `mm-selfeng`, `mm-pi`, `PromotionGate` | the loop's inner stages already exist |
| Host | `pi 0.85.1`, OpenAI-compatible endpoint via `OPENAI_BASE_URL`/`OPENAI_API_KEY` | Pi RPC + LLM |

## 3. Deliverables (exact paths)

```
crates/mm-runtime/
├── src/lib.rs                       # ClosedLoop, LoopState, LoopStage, LoopReport
├── src/loop_controller.rs           # mm-loop orchestration + budget envelopes
├── src/timescale.rs                 # TimescaleScheduler, TimescaleKind
├── src/self_model.rs                # SelfModel, SelfModelReport, Divergence
├── src/debt.rs                      # DebtScanner, DebtFinding, DebtKind
├── src/gc.rs                        # Collector, GcAction, ledger protection
├── src/design_writer.rs             # DesignWriter: design/ revisions via ChangeSet
├── src/module_loader.rs             # ModuleLoader over the nexus plugin lifecycle
├── src/bin/mm-loop.rs               # long-running loop process (BACKGROUND)
└── tests/                           # unit + behaviour tests
#   this phase's SQLite migration lives at crates/mm-store-sqlite/migrations/0012_loop.sql (§4)
crates/mm-cli/src/cmd/{loop,self_model,debt,gc}.rs
ontology/mm.ttl                      # + self/debt/GC classes (§4)
ontology/shapes/self_model.ttl       # + shapes
tests/e2e/qualification.rs           # the committed qualification scenario
tests/e2e/replay_determinism.rs
tests/e2e/hot_load.rs
tests/e2e/gate_bypass_adversarial.rs
bench/qualification/goal_novel.json  # novel goal fixture
bench/qualification/budget.toml      # budget envelope
bench/qualification/baseline.json    # benchmark baseline
pi/skills/mm-design-revise/SKILL.md  # Pi skill for design-doc revisions
data/                                # runtime: graph/, events/, logs/, sandbox/ (gitignored)
```

## 4. Data model and ontology deltas

**SQLite (`crates/mm-store-sqlite/migrations/0012_loop.sql`).**

```sql
CREATE TABLE loop_runs (
  id           TEXT(26) PRIMARY KEY,
  goal_ulid    TEXT(26) NOT NULL,
  goal_text    TEXT NOT NULL,
  novel        INTEGER NOT NULL DEFAULT 0,
  status       TEXT NOT NULL,              -- running|completed|failed|aborted
  config_hash  TEXT NOT NULL,
  code_version TEXT NOT NULL,
  started_at   TEXT NOT NULL,
  ended_at     TEXT
);
CREATE TABLE loop_iterations (
  id         TEXT(26) PRIMARY KEY,
  run_id     TEXT(26) NOT NULL REFERENCES loop_runs(id),
  idx        INTEGER NOT NULL,
  stage      TEXT NOT NULL,                -- LoopStage
  outcome    TEXT NOT NULL,
  started_at TEXT NOT NULL,
  ended_at   TEXT
);
CREATE TABLE self_model_reports (
  id TEXT(26) PRIMARY KEY, run_id TEXT(26) REFERENCES loop_runs(id),
  actual_model REAL NOT NULL, actual_ideal REAL NOT NULL, model_ideal REAL NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE divergence_metrics (
  id TEXT(26) PRIMARY KEY, report_id TEXT(26) NOT NULL REFERENCES self_model_reports(id),
  dimension TEXT NOT NULL, value REAL NOT NULL
);
CREATE TABLE debt_findings (
  id TEXT(26) PRIMARY KEY, run_id TEXT(26), kind TEXT NOT NULL,
  subject_uri TEXT NOT NULL, severity REAL NOT NULL, evidence_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE gc_actions (
  id TEXT(26) PRIMARY KEY, finding_id TEXT(26) REFERENCES debt_findings(id),
  action TEXT NOT NULL, subject_uri TEXT NOT NULL,
  reversible INTEGER NOT NULL, ledger_impact TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE module_loads (
  id TEXT(26) PRIMARY KEY, module_uri TEXT NOT NULL, version TEXT NOT NULL,
  status TEXT NOT NULL, loaded_at TEXT NOT NULL
);
CREATE TABLE design_revisions (
  id TEXT(26) PRIMARY KEY, doc_uri TEXT NOT NULL, revision INTEGER NOT NULL,
  change_set_id TEXT(26) NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE timescales (
  name TEXT PRIMARY KEY, tick_ms INTEGER NOT NULL, last_tick_ulid TEXT(26)
);
```

**Ontology (`mm:` = `https://metamind.dev/ontology#`; graph `https://metamind.dev/graph/self`).**
Classes `mm:SelfModel, mm:SelfModelReport, mm:Divergence, mm:DebtFinding, mm:GcAction,
mm:ModuleLoad, mm:DesignRevision`. Predicates `mm:hasDivergence, mm:subjectOf, mm:severity,
mm:generatedRevision, mm:loadedModule, mm:protectsLedger, mm:inRun`. Instance IRIs are ULIDs
(`https://metamind.dev/data/{ulid}`); module/design URIs are the stable forms from §4 of the parent
plan (`.../code/module/{path}`, `.../design/{doc}#{anchor}`). SHACL shapes in
`ontology/shapes/self_model.ttl` require each `mm:GcAction` to name a subject and assert
`mm:protectsLedger true` for any action whose subject is a protected ledger resource.

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

`crates/mm-runtime/src/lib.rs`:

```rust
pub enum LoopStage { Experience, EventLog, MetaAnalysis, CapabilityGap, ChangeSet,
                     SelfEngineering, TestBenchmark, Shadow, PromotionGate, NewVersion }

pub struct LoopGoal { pub id: Ulid, pub description: String, pub novel: bool,
                      pub success_criteria: Vec<String> }

pub struct BudgetEnvelope { pub meta_analysis: f64, pub improvement: f64,
                            pub evolution: f64, pub metacognitive: f64, pub wall_ms: u64 }

pub trait ClosedLoop {
    async fn run(&self, goal: LoopGoal, budget: BudgetEnvelope)
        -> Result<LoopReport, LoopError>;
    async fn step(&self, state: &mut LoopState, stage: LoopStage)
        -> Result<StageOutcome, LoopError>;
}
pub struct LoopReport { pub run_id: Ulid, pub stages: Vec<StageOutcome>,
                        pub promoted: Option<PromotionId>, pub budget_used: BudgetEnvelope }
```

`timescale.rs`:

```rust
pub enum TimescaleKind { Action, Episode, Project, Goal, Identity }
pub struct Timescale { pub id: Ulid, pub kind: TimescaleKind, pub tick: Duration, pub handler: HandlerId }
pub trait TimescaleScheduler { async fn tick(&self, now: Timestamp) -> Result<Vec<Tick>, SchedError>; }
```

`self_model.rs` / `debt.rs` / `gc.rs` / `design_writer.rs` / `module_loader.rs`:

```rust
pub trait SelfModel { async fn measure(&self, run: Ulid) -> Result<SelfModelReport, SelfModelError>; }
pub struct SelfModelReport { pub id: Ulid, pub actual_model: f32, pub actual_ideal: f32,
                             pub model_ideal: f32, pub dims: Vec<Divergence> }

pub enum DebtKind { UnusedCapability, DuplicatePolicy, ConflictingPolicy, StaleMemory,
                    UnusedSchema, ExpensiveWorkflow, ObsoleteTechnique, Complexity }
pub trait DebtScanner { async fn scan(&self) -> Result<Vec<DebtFinding>, DebtError>; }
pub struct DebtFinding { pub id: Ulid, pub kind: DebtKind, pub subject: NamedNode,
                         pub severity: f32, pub evidence: Vec<EvidenceId> }

pub trait Collector { async fn collect(&self, finding: &DebtFinding) -> Result<GcAction, GcError>; }
pub struct GcAction { pub id: Ulid, pub kind: GcActionKind, pub reversible: bool,
                      pub ledger_impact: LedgerImpact }

pub trait DesignWriter { async fn revise(&self, doc: DesignDocRef, cs: &ChangeSet)
    -> Result<Revision, DesignError>; }

pub trait ModuleLoader { async fn hot_load(&self, mv: &ModuleVersion)
    -> Result<LoadReceipt, LoadError>; }
```

**CLI (`mm-cli`).**

```
mm-cli loop run --goal "<text>" | --goal-file bench/qualification/goal_novel.json
                [--budget-file bench/qualification/budget.toml] [--novel]
mm-cli loop status --run <ulid>
mm-cli loop artifacts --run <ulid>
mm-cli loop replay --run <ulid>
mm-cli self-model report [--run <ulid>]
mm-cli debt scan
mm-cli gc --apply [--finding <ulid>]
mm-cli module load --uri <module-uri>@<version>
mm-cli design revise --doc <doc-uri> --change-set <ulid>
```

Hot-loaded modules keep the Phase 8 `plugin.toml` contract (stable `uri`, `version`, `owned_by_phase`,
`capability`, T-Box functions). The loop itself is exposed as module
`modules/cognition/closed-loop` with `uri = https://metamind.dev/code/module/cognition/closed-loop`.

## 6. External references copied in and integration

Following §7 of the parent plan, these are references only and are copied into `vendor/` as workspace
members with `mmc:copiedFrom` provenance and a `COPYING.md`:

| Reference | Source | Copied-in use |
|---|---|---|
| `nexus-launcher`, `nexus-core` plugin loader | nexus | hot-load promoted modules through the plugin lifecycle without restart |
| `evolution-ir` | rust_symbolic | fitness of candidate design/module versions during promotion |
| `ensemble-ir` | rust_symbolic | multi-lens self-evaluation (agree/disagree across benchmarks) |
| `kg-diagram` | rust_extract | deterministic Mermaid views of the code/debt graph |
| `rdf-codec`, `rdf-shacl` | rust_symbolic | canonical RDF emission + SHACL validation of self/debt graphs |
| `temporal-store` semantics | rust_symbolic | immutable developmental ledger for GC protection |
| `mm-pi` (Phase 11) | local | Pi RPC drives module and design-doc generation |
| `decision-ir`, `causal-ir` | rust_symbolic | gate scoring and root-cause diagnosis of capability gaps |

## 7. Step-by-step implementation tasks

1. **Skeleton + schema.** Create `crates/mm-runtime` with `lib.rs`, the `LoopStage`/`LoopState` types
   and `crates/mm-store-sqlite/migrations/0012_loop.sql`; wire it into `[workspace] members`. Add empty `cmd/{loop,self_model,
   debt,gc}.rs` to `mm-cli`. Check: `cargo test -p mm-runtime` passes trait-level unit tests.

2. **Loop controller.** Implement `LoopController` executing the ten stages in order, delegating each
   stage to the Phase 11 services (meta-analysis, change-set assembly, Pi, promotion gate). Enforce
   `BudgetEnvelope` with deterministic arithmetic; abort on overrun. Check: a dry-run loop over a tiny
   fixture records all ten stages and never exceeds budget.

3. **Timescale scheduler.** Implement `TimescaleScheduler` with the five timescales (§31); each tick
   dispatches its handler and appends a `timescale.tick` event. Persist `timescales`. Check: a fast
   test clock drives N ticks per timescale deterministically.

4. **Self-model.** Implement `SelfModel::measure` comparing Actual (event-log behavior), Model (the
   being's own claims about itself), and Ideal (declared operating principles), producing
   `SelfModelReport` + `Divergence` rows and `mm:SelfModelReport` triples. Check: a fixture with a
   known Actual/Ideal gap yields the expected divergence within tolerance.

5. **Debt scanner + GC.** Implement `DebtScanner` over the code graph, memory, policies, and schemas;
   implement `Collector` with the escalation ladder (change temporary reasoning → context → data →
   policy → prompt → skill → code → architecture → model), always choosing the cheapest effective
   intervention (§107). GC must refuse to delete any subject carrying the immutable-ledger marker and
   record `mm:protectsLedger true`. Check: seeded unused capability / duplicate policy / stale memory
   are found; a protected subject is never collected.

6. **Design writer.** Implement `DesignWriter::revise` so design-doc changes flow through the same
   `ChangeSet` + promotion pipeline as code (never a direct write), register the new revision in the
   code graph as `mm:DesignRevision`, and update the phase plan. Check: revising a fixture doc creates a
   `ChangeSet`, and rejection leaves the doc byte-identical.

7. **Module hot-loader.** Implement `ModuleLoader::hot_load` over the nexus plugin lifecycle; loading a
   promoted `ModuleVersion` registers `mm:ModuleLoad`, makes its T-Box functions immediately available,
   and leaves prior modules running. Check: hot-load a fixture module and call its function without
   restarting the process.

8. **Qualification run.** Add `bench/qualification/goal_novel.json`, `budget.toml`, `baseline.json`, and
   `tests/e2e/qualification.rs`. The scenario: a novel design goal (not in any fixture corpus) must
   produce a design revision, a Pi-authored module, tests, a benchmark comparison, a promotion decision
   with a written reason, and a successful hot-load — with no human edits.

9. **Replay + provenance verification.** Implement `loop replay` and `loop artifacts`: replay from
   `(code_version, config_hash, event log)` must reproduce the run byte-identically, and the artifacts
   query must return the full chain `mmc:PiSession → mmc:Edit → mmc:ModuleVersion → mm:ChangeSet →
   mm:Promotion`.

10. **CI wiring.** Add the qualification scenario and the full Phase 1–12 gate suite to CI; the loop
    process (`mm-loop`) runs as a BACKGROUND service for long runs, with readiness and log checks.

## 8. Detailed logging requirements

All records go through `mm-log` (§10 of the parent plan) with `trace_id` = run ULID, and the following
event codes/fields (in addition to the global schema):

| Event code | Fields |
|---|---|
| `loop.run.start` / `loop.run.end` | `run_id`, `goal_ulid`, `novel`, `config_hash`, `code_version`, `budget`, `status` |
| `loop.iteration.start` / `.end` | `run_id`, `idx`, `stage`, `outcome`, `latency_ms` |
| `loop.budget.debit` | `run_id`, `bucket`, `amount`, `remaining` |
| `timescale.tick` | `name`, `kind`, `tick_ms`, `handler`, `last_tick_ulid` |
| `self_model.report` | `run_id`, `actual_model`, `actual_ideal`, `model_ideal` |
| `self_model.divergence` | `report_id`, `dimension`, `value` |
| `debt.scan` / `debt.finding` | `kind`, `subject_uri`, `severity`, `evidence` |
| `gc.action` | `finding_id`, `action`, `subject_uri`, `reversible`, `ledger_impact`, `protects_ledger` |
| `module.load` | `module_uri`, `version`, `status`, `latency_ms` |
| `design.revision` | `doc_uri`, `revision`, `change_set_id` |
| `invariant.check` | `invariant_uri`, `result`, `subject_uri` |

Operational records are best-effort; every state mutation, promotion, module load, and design revision
is a transactional audit record. `mm-cli logs verify` must pass for a qualification run.

## 9. Testing plan

- **Unit:** `LoopController` stage ordering; budget arithmetic; timescale tick math; divergence metrics;
  debt classification; GC escalation-ladder choice; design-writer change-set translation.
- **Property (`proptest`):** budget never exceeded under randomized envelopes; GC never selects a
  protected subject; loop replay equals recorded state.
- **Golden/replay (`insta`):** `bench/qualification` run snapshot; `loop replay` byte-identical output.
- **Adversarial (`tests/e2e/gate_bypass_adversarial.rs`):** prompts and change sets crafted to bypass the
  promotion gate are rejected; identity-invariant violations are impossible.
- **End-to-end (`tests/e2e/qualification.rs`, `hot_load.rs`):** the novel-goal scenario, provenance
  query, and hot-load without restart.
- **Conformance:** self/debt graph SHACL validation; `codex verify` stays green after a design revision.

## 10. Pass gate

Run as plain commands; all must exit 0 and earlier phases' gates must still pass.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p mm-cli -- doctor
cargo run -p mm-cli -- logs verify

# The qualification scenario: novel goal, no human edits.
RUN=$(cargo run -p mm-cli -- loop run \
        --goal-file bench/qualification/goal_novel.json \
        --budget-file bench/qualification/budget.toml --novel | jq -r .run_id)

cargo run -p mm-cli -- loop status --run "$RUN"          # status=completed
cargo run -p mm-cli -- loop artifacts --run "$RUN"        # full provenance chain present
cargo run -p mm-cli -- self-model report --run "$RUN"     # divergence report emitted
cargo run -p mm-cli -- debt scan                          # actionable findings listed
cargo run -p mm-cli -- gc --apply                         # protected subjects untouched
cargo run -p mm-cli -- codex verify                       # code graph still consistent
cargo run -p mm-cli -- being verify                       # identity invariants intact

# Deterministic reproduction of the entire run.
cargo run -p mm-cli -- loop replay --run "$RUN"           # byte-identical artifacts
```

**Pass criteria (definition of "functional prototype").**
1. The novel-goal run, with **no human code edits**, produces (a) a design doc revision, (b) a
   Pi-authored module, (c) passing tests, (d) a benchmark comparison, (e) a promotion decision with a
   written reason, and (f) a successful hot-load.
2. `loop artifacts` returns the complete chain `mmc:PiSession → mmc:Edit → mmc:ModuleVersion →
   mm:ChangeSet → mm:Promotion` in RDF.
3. `loop replay` reproduces the run deterministically from the event log.
4. `self-model report` emits a divergence report; `debt scan` + `gc --apply` complete with actionable
   findings and no protected subject collected.
5. `logs verify` passes (schema, audit completeness, gapless sequence, ULID correlation, redaction).
6. The whole workspace builds with zero warnings and the full Phase 1–12 regression suite is green.
7. No identity invariant was violated during the run (deterministic `invariant.check` verified).

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Long autonomous runs consume unbounded resources | `BudgetEnvelope` enforced deterministically; `mm-loop` runs as a monitored BACKGROUND service with readiness + budget-stop |
| Replay diverges because of Pi/LLM nondeterminism | Pi sessions are recorded and replayed from the event log; `loop replay` uses recorded session artifacts, not live Pi calls |
| GC deletes something needed | Immutable-ledger marker + `mm:protectsLedger`; GC is reversible and logged; reversibility checked before apply |
| Self-model report is a narrative, not a measurement | Divergences are numeric, computed from event-log behavior vs declared principles, and snapshot-tested |
| Design writer directly edits production docs | Design revisions go through `ChangeSet` + promotion only; rejected revisions leave docs byte-identical |
| Hot-load destabilizes the running process | Nexus plugin lifecycle with rollback to the prior `ModuleVersion`; prior modules keep running during load |
| Gate bypass via crafted prompts | Adversarial e2e suite asserts the promotion gate and invariants cannot be bypassed by model output |

## 12. Design traceability

| Build-plan area | Design reference |
|---|---|
| Closed development loop | `digital_mind_design1.md` §103 (continuous automatic integration) |
| Multi-timescale scheduling | §31 |
| Self-model Actual/Model/Ideal | §62 |
| Meta-analysis triggers and diagnosis | §63 |
| Improvement/evolution budgets | §64–65, §43 |
| Evolution journal | §66 |
| Developmental stages / mutation privileges | §67 |
| Continuous integration of capabilities | §68 |
| Architectural debt | §85 |
| Cognitive garbage collection | §86 |
| Developmental control plane | §104 |
| The being learns its own architecture | §105 |
| Architecture-aware metacognition | §106 |
| Self-engineering research program | §107 |
| Bootstrap phases 9–10 | §101–102 |
| The one architectural invariant | §109 |
| Final system | §110 |
| Companion/goal agency continued | `companion_loop1.md`, `companion_loop2.md` |
| Contextual character in reports | `personality_simulator3.md` |
| Metacognitive allocation of Loop stages | `meta_analysis2.md` |
| Pi as code editor | parent plan §11 |
