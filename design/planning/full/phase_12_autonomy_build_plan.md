# Phase 12 — Autonomy / Self-Bootstrap Closed Loop (Extended Build Plan)

> Extended from: `design/planning/phase_12_autonomy_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 12 · Codename Metamind (`mm`)

This is the research-grounded, streamlined revision. It keeps the same scope and pass gate but locks in
the mechanisms: a **ten-stage closed developmental loop** over an event-sourced run, a **five-timescale
scheduler**, a **numeric self-model** (Actual/Model/Ideal divergence), **architectural-debt scanning and
reversible cognitive GC** that never touches the immutable developmental ledger, a **design writer** whose
revisions flow through the same `ChangeSet` + promotion pipeline as code, and a **hot module loader** that
swaps in promoted capabilities without restarting the process. The pass gate is the qualification
scenario: a novel goal drives the whole loop to a working, tested, promoted, hot-loaded capability with
**no human code edits**.

---

## 0. Research foundation (code-available)

Every idea below is borrowed from a project whose source is available. Only ideas with code are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| What a self-improving agent can actually evolve | Awesome-Self-Evolving-Agents — https://github.com/XMUDeepLIT/Awesome-Self-Evolving-Agents | A taxonomy of evolution targets: model, context/prompt, tools, workflow, architecture | `ChangeSet` kinds cover exactly these; GC's escalation ladder orders interventions cheapest-first |
| Trustworthiness of self-evolution | Awesome-Reliable-Self-Evolving-Agents — https://github.com/wkqdzkd/Awesome-Reliable-Self-Evolving-Agents | Reliability axes: safety, controllability, boundedness, auditability | Every self-change is reversible + journaled; identity invariants and permissions are never auto-evolved |
| Experience → feedback → self-modification as the canonical loop | Self-evolving agents survey — https://arxiv.org/html/2507.21046v1 | Organizing axis: runs outpace a single context window and accumulate experience across runs | Fixed ten-stage loop over persistent state; all learning is written to the event log, never held in context |
| Durable workflow with replay from history | Temporal — https://github.com/temporalio/temporal | Event-sourced workflow state, replay determinism, activity retry | `LoopState` is a projection of the event log; `loop replay` reproduces a run from `(code_version, config_hash, events)` |
| Journaled, idempotent long-running handlers | Restate — https://github.com/restatedev/restate | Idempotent handlers keyed by invocation id; resume after crash | Each stage is an idempotent handler keyed `(run_id, idx)`; a crashed loop resumes at the last committed stage |
| Append-only events + deterministic projections | cqrs-es — https://github.com/j5ik2o/cqrs-es-example-rs · event-sourcing — https://crates.io/crates/event-sourcing | Rebuild state by folding events; no mutable "current state" table | Loop/self-model/debt state are folds over `mm-eventlog`; no authoritative mutable state |
| Agent-run tracing and eval-as-code | Langfuse — https://github.com/langfuse/langfuse | Structured traces for agent runs; an evaluation harness that is code, not a spreadsheet | The qualification scenario is a committed eval dataset under `bench/qualification/`; every run emits a trace |
| Standard LLM/agent telemetry attribute names | OpenTelemetry GenAI semantic conventions — https://opentelemetry.io/docs/specs/semconv/gen-ai/ | Conventional span/attribute names for model and agent calls | `mm-log` exports traces using GenAI semconv names; no bespoke field names where a convention exists |
| Bounded recursive self-improvement with a lineage archive | Darwin Gödel Machine — https://github.com/jennyzzt/dgm | Candidates judged by benchmarks; an archive of stepping-stone versions | Promoted module versions are archived; the loop may branch from any archived version, never from an unnamed state |
| Live module (re)load without process restart | `nexus-core` plugin lifecycle (nexus) | Plugin load/unload with a stable `uri`; activate a new version while old ones keep serving | `ModuleLoader` hot-loads a promoted `ModuleVersion`; rollback restores the previous version |
| Multi-objective fitness + multi-lens agreement for candidates | `evolution-ir`, `ensemble-ir` (rust_symbolic) | Fitness vectors and agree/disagree across evaluators | Self-evaluation is a numeric vector; a candidate must not regress any frozen dimension |

---

## 1. Objective and scope

Close the developmental loop so the being can **continue its own design and build process**. Given a
novel goal, the runtime drives the full cycle — experience → event log → meta-analysis → capability gap →
change set → self-engineering → test/benchmark → shadow → promotion gate → new version — and the result is
a working, tested, promoted, hot-loadable capability, produced **without human code edits**, with complete
RDF provenance and deterministic replay.

Five organs are added on top of Phases 1–11:

1. **`mm-runtime::loop_controller`** — the ten-stage `ClosedLoop` with deterministic budget enforcement.
2. **`mm-runtime::timescale`** — the five-timescale scheduler (Action → Episode → Project → Goal → Identity, §31).
3. **`mm-runtime::self_model`** — numeric Actual/Model/Ideal divergence reports (§62).
4. **`mm-runtime::debt` + `gc`** — architectural-debt detection and reversible cognitive GC that protects the immutable developmental ledger (§85–86).
5. **`mm-runtime::design_writer` + `module_loader`** — self-design through the change-set/promotion pipeline, and hot-loading of promoted modules over the nexus lifecycle (§67–69, §103–107).

**In scope:** loop controller, timescale scheduler, self-model, debt scanner + GC, design writer, module
hot-loader, the qualification scenario, and end-to-end replay.

**Out of scope / never automatic:** opening mutation privileges beyond Phase 11 (identity invariants stay
immutable), any change to an `mm:Invariant` resource, and redesigning the promotion gate inside a cycle
(gate changes are themselves change-sets judged by the frozen gate).

---

## 2. Architecture and invariants

```
 EXPERIENCE ─► EVENT LOG ─► META-ANALYSIS ─► CAPABILITY GAP ─► CHANGE SET
      ▲                                                             │
      │                                                             ▼
 NEW VERSION ◄─ PROMOTION GATE ◄─ SHADOW ◄─ TEST/BENCHMARK ◄─ SELF-ENGINEERING (sandbox + Pi)
      │                  │
      │                  └── reject(reason) ─► evolution_journal
      ▼
 HOT LOAD (nexus lifecycle) ─► /code graph ─► next experience

 Timescales: Action(ms) · Episode(s) · Project(h) · Goal(day) · Identity(never)
 Self-model: Actual (event-log behavior) vs Model (self-claims) vs Ideal (declared principles)
 Debt/GC:    scan → evidence → escalation ladder → reversible action (ledger-protected guard)
```

**Invariants (deterministic, checked every run):**

- **I1 — LLM proposes, the loop decides.** No stage outcome depends on unvalidated model text; the
  promotion gate is code.
- **I2 — The developmental ledger is immutable.** No GC action, design revision, or module load may
  modify or delete an event or a resource marked `mm:protectsLedger true`.
- **I3 — Replay determinism.** `loop replay` reproduces a run byte-identically from
  `(code_version, config_hash, event log)` with zero live provider calls.
- **I4 — Budgets are hard.** A stage that would exceed `BudgetEnvelope` is denied, not truncated.
- **I5 — Production is written only by promotion.** Design docs and code reach the tree only through a
  promoted `ChangeSet`.

---

## 3. Deliverables and workspace layout

```
crates/mm-runtime/
├── src/lib.rs                  # ClosedLoop, LoopState, LoopStage, LoopReport, LoopGoal, BudgetEnvelope
├── src/loop_controller.rs      # ten-stage orchestration + budget envelopes
├── src/timescale.rs            # TimescaleScheduler, Timescale, TimescaleKind
├── src/self_model.rs           # SelfModel, SelfModelReport, Divergence
├── src/debt.rs                 # DebtScanner, DebtFinding, DebtKind
├── src/gc.rs                   # Collector, GcAction, GcActionKind, ledger-protection guard
├── src/design_writer.rs        # DesignWriter: design/ revisions via ChangeSet
├── src/module_loader.rs        # ModuleLoader over the nexus plugin lifecycle
├── src/bin/mm-loop.rs          # long-running loop process (BACKGROUND service)
└── tests/                      # unit + behaviour tests
crates/mm-store-sqlite/migrations/0012_loop.sql
crates/mm-cli/src/cmd/{loop,self_model,debt,gc}.rs
ontology/mm.ttl                 # + SelfModel, DebtFinding, GcAction, DesignRevision, ModuleLoad
ontology/shapes/self_model.ttl  # + shapes
modules/cognition/closed-loop/{plugin.toml,src/lib.rs,manual/module.md,tests/}
pi/skills/mm-design-revise/SKILL.md
bench/qualification/{goal_novel.json,budget.toml,baseline.json}
tests/e2e/{qualification,replay_determinism,hot_load,gate_bypass_adversarial}.rs
data/                           # runtime: graph/, events/, logs/, sandbox/ (gitignored)
```

`mm-cli` gains: `loop run|status|artifacts|replay`, `self-model report`, `debt scan`, `gc --apply`,
`module load`, `design revise`.

---

## 4. Detailed specifications

Named graph `https://metamind.dev/graph/self`. ULID IRIs `<https://metamind.dev/data/{ulid}>`; module and
design IRIs use the stable forms from parent plan §4 (`.../code/module/{path}`, `.../design/{doc}#{anchor}`).

### 4.1 Core types

```rust
// mm-runtime::lib
pub enum LoopStage { Experience, EventLog, MetaAnalysis, CapabilityGap, ChangeSet,
                     SelfEngineering, TestBenchmark, Shadow, PromotionGate, NewVersion }

pub struct LoopGoal { pub id: Ulid, pub description: String, pub novel: bool,
                      pub success_criteria: Vec<String> }

pub struct BudgetEnvelope { pub meta_analysis: f64, pub improvement: f64, pub evolution: f64,
                            pub metacognitive: f64, pub wall_ms: u64 }

pub struct LoopState { pub run_id: Ulid, pub stage: LoopStage, pub iteration: u32,
                       pub artifacts: Vec<ArtifactRef>, pub budget_used: BudgetEnvelope }

pub struct LoopReport { pub run_id: Ulid, pub stages: Vec<StageOutcome>,
                        pub promoted: Option<PromotionId>, pub budget_used: BudgetEnvelope }

pub trait ClosedLoop {
    async fn run(&self, goal: LoopGoal, budget: BudgetEnvelope) -> Result<LoopReport, LoopError>;
    async fn step(&self, state: &mut LoopState, stage: LoopStage) -> Result<StageOutcome, LoopError>;
}
```

```rust
// timescale.rs — §31
pub enum TimescaleKind { Action, Episode, Project, Goal, Identity }
pub struct Timescale { pub id: Ulid, pub kind: TimescaleKind, pub tick: Duration, pub handler: HandlerId }
pub trait TimescaleScheduler { async fn tick(&self, now: Timestamp) -> Result<Vec<Tick>, SchedError>; }

// self_model.rs — §62
pub trait SelfModel { async fn measure(&self, run: Ulid) -> Result<SelfModelReport, SelfModelError>; }
pub struct SelfModelReport { pub id: Ulid, pub actual_model: f32, pub actual_ideal: f32,
                             pub model_ideal: f32, pub dims: Vec<Divergence> }

// debt.rs / gc.rs — §85–86
pub enum DebtKind { UnusedCapability, DuplicatePolicy, ConflictingPolicy, StaleMemory,
                    UnusedSchema, ExpensiveWorkflow, ObsoleteTechnique, Complexity }
pub trait DebtScanner { async fn scan(&self) -> Result<Vec<DebtFinding>, DebtError>; }
pub struct DebtFinding { pub id: Ulid, pub kind: DebtKind, pub subject: NamedNode,
                         pub severity: f32, pub evidence: Vec<EvidenceId> }

pub enum GcActionKind { TempReasoning, Context, Data, Policy, Prompt, Skill, Code, Architecture, Model }
pub struct GcAction { pub id: Ulid, pub kind: GcActionKind, pub reversible: bool,
                      pub ledger_impact: LedgerImpact }
pub trait Collector { async fn collect(&self, finding: &DebtFinding) -> Result<GcAction, GcError>; }

// design_writer.rs / module_loader.rs
pub trait DesignWriter { async fn revise(&self, doc: DesignDocRef, cs: &ChangeSet)
    -> Result<Revision, DesignError>; }
pub trait ModuleLoader { async fn hot_load(&self, mv: &ModuleVersion)
    -> Result<LoadReceipt, LoadError>; }
```

### 4.2 Migration `crates/mm-store-sqlite/migrations/0012_loop.sql`

All ids `TEXT(26)` (ULID); no mutable "current" rows — these tables are projections of the event log.

```sql
CREATE TABLE loop_runs (
  id TEXT(26) PRIMARY KEY, goal_ulid TEXT(26) NOT NULL, goal_text TEXT NOT NULL,
  novel INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL
    CHECK (status IN ('running','completed','failed','aborted')),
  config_hash TEXT NOT NULL, code_version TEXT NOT NULL,
  started_at TEXT NOT NULL, ended_at TEXT);
CREATE TABLE loop_iterations (
  id TEXT(26) PRIMARY KEY, run_id TEXT(26) NOT NULL REFERENCES loop_runs(id),
  idx INTEGER NOT NULL, stage TEXT NOT NULL, outcome TEXT NOT NULL,
  started_at TEXT NOT NULL, ended_at TEXT, UNIQUE (run_id, idx));
CREATE TABLE self_model_reports (
  id TEXT(26) PRIMARY KEY, run_id TEXT(26) REFERENCES loop_runs(id),
  actual_model REAL NOT NULL, actual_ideal REAL NOT NULL, model_ideal REAL NOT NULL,
  created_at TEXT NOT NULL);
CREATE TABLE divergence_metrics (
  id TEXT(26) PRIMARY KEY, report_id TEXT(26) NOT NULL REFERENCES self_model_reports(id),
  dimension TEXT NOT NULL, value REAL NOT NULL);
CREATE TABLE debt_findings (
  id TEXT(26) PRIMARY KEY, run_id TEXT(26), kind TEXT NOT NULL, subject_uri TEXT NOT NULL,
  severity REAL NOT NULL, evidence_json TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE gc_actions (
  id TEXT(26) PRIMARY KEY, finding_id TEXT(26) REFERENCES debt_findings(id),
  action TEXT NOT NULL, subject_uri TEXT NOT NULL, reversible INTEGER NOT NULL,
  ledger_impact TEXT NOT NULL, protects_ledger INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
CREATE TABLE module_loads (
  id TEXT(26) PRIMARY KEY, module_uri TEXT NOT NULL, version TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('loaded','rolled_back','failed')),
  loaded_at TEXT NOT NULL);
CREATE TABLE design_revisions (
  id TEXT(26) PRIMARY KEY, doc_uri TEXT NOT NULL, revision INTEGER NOT NULL,
  change_set_id TEXT(26) NOT NULL, created_at TEXT NOT NULL, UNIQUE (doc_uri, revision));
CREATE TABLE timescales (
  name TEXT PRIMARY KEY, kind TEXT NOT NULL, tick_ms INTEGER NOT NULL, last_tick_ulid TEXT(26));
```

### 4.3 Ontology + SHACL

Classes `mm:SelfModel, mm:SelfModelReport, mm:Divergence, mm:DebtFinding, mm:GcAction, mm:ModuleLoad,
mm:DesignRevision`; predicates `mm:hasDivergence, mm:subjectOf, mm:severity, mm:generatedRevision,
mm:loadedModule, mm:protectsLedger, mm:inRun`. Shapes in `ontology/shapes/self_model.ttl` require: a
`mm:SelfModelReport` has exactly three scalar divergences; a `mm:GcAction` names a subject and asserts
`mm:protectsLedger true` whenever its subject is a protected ledger resource; a `mm:DesignRevision` links
exactly one `mm:ChangeSet`.

### 4.4 The ten stages (each an idempotent handler keyed `(run_id, idx)`)

| # | Stage | Does | Idempotency key |
|---|---|---|---|
| 1 | `Experience` | Build the episode from the goal + current capability graph | `(run_id, 0)` |
| 2 | `EventLog` | Commit the episode and all inputs as events | append-only |
| 3 | `MetaAnalysis` | Run the Phase 11 trigger/diagnosis pass on the episode | `(run_id, 2)` |
| 4 | `CapabilityGap` | Derive the gap (capability/data/policy/prompt/code) | `(run_id, 3)` |
| 5 | `ChangeSet` | Assemble a typed `ChangeSet` from the gap | `(run_id, 4)` |
| 6 | `SelfEngineering` | Sandbox + Pi authoring (Phase 11) | `(run_id, 5)` |
| 7 | `TestBenchmark` | Run regression + the frozen benchmark | `(run_id, 6)` |
| 8 | `Shadow` | Compare candidate against baseline in shadow | `(run_id, 7)` |
| 9 | `PromotionGate` | Deterministic promote/reject with a written reason | `(run_id, 8)` |
| 10 | `NewVersion` | Register the version and hot-load it | `(run_id, 9)` |

**GC escalation ladder (cheapest effective intervention first).** `TempReasoning → Context → Data →
Policy → Prompt → Skill → Code → Architecture → Model`. The collector picks the lowest rung that resolves
the finding; any rung at or above `Code` is itself a `ChangeSet` through the promotion gate.

### 4.5 Promotion gate (reused, not re-specified)

Phase 12 reuses the Phase 11 `PromotionGate` unchanged. It rejects if any holds, with a specific recorded
reason: a regression test fails; the candidate is worse than baseline beyond `noise_margin`; risk exceeds
the evolution risk budget; a hard prohibition fires (Phase 9 firewall); required evidence is missing; the
change touches identity invariants or permissions. A rejected design revision or module load leaves the
tree byte-identical.

### 4.6 Design writer and module loader

- `DesignWriter::revise` never writes a doc directly: it emits a `ChangeSet` whose payload is the diff,
  registers the resulting `mm:DesignRevision` and updates the phase plan. Rejection ⇒ byte-identical doc.
- `ModuleLoader::hot_load` activates a promoted `ModuleVersion` over the nexus plugin lifecycle, records
  `mm:ModuleLoad`, exposes its T-Box functions immediately, and **keeps prior versions serving** until the
  new one is verified; rollback restores the previous version. The loop itself is module
  `modules/cognition/closed-loop`, `uri = https://metamind.dev/code/module/cognition/closed-loop`,
  keeping the Phase 8 `plugin.toml` contract (stable `uri`, `version`, `owned_by_phase`, `capability`).

### 4.7 External references copied in and integrated

Per parent plan §7, external code is **reference only**: the needed source is copied into `vendor/<origin>/`
as workspace members and integrated behind `mm-*` traits, with `mmc:copiedFrom` (repo, revision, license)
emitted by the Phase 2 scan and a `vendor/<origin>/COPYING.md`.

| Need | Reference | Copy / integration |
|---|---|---|
| Hot module swap without restart | `nexus-core` / `nexus-launcher` plugin loader | copied to `vendor/nexus/`; `mm-runtime::module_loader` drives the plugin lifecycle |
| Fitness of candidate versions during promotion | `evolution-ir` | copied to `vendor/rust_symbolic/`; gate scoring input |
| Multi-lens self-evaluation | `ensemble-ir` | copied; self-model dimensions scored by agreement across lenses |
| Deterministic graph views of code/debt | `kg-diagram` | copied to `vendor/rust_extract/`; Mermaid views of the debt graph |
| Canonical RDF + SHACL validation of self/debt graphs | `rdf-codec`, `rdf-shacl` | copied to `vendor/rust_symbolic/`; ontology/shapes validation |
| Causal root-cause of capability gaps | `causal-ir` | copied; diagnoses which substrate to change |
| Immutable developmental ledger semantics | `temporal-store` | copied as a pattern; backs `mm:protectsLedger` GC guards |
| Pi-authored module and design generation | Phase 11 `mm-pi` | local; Pi RPC drives authoring, sessions ingested for provenance |

---

## 5. Build sequence

1. **Skeleton + schema.** Create `crates/mm-runtime` with `lib.rs`, `LoopStage`/`LoopState`/`LoopGoal`/
   `BudgetEnvelope`, and `crates/mm-store-sqlite/migrations/0012_loop.sql`; wire into `[workspace] members`;
   add empty `cmd/{loop,self_model,debt,gc}.rs`. *Check:* `cargo test -p mm-runtime` passes trait-level tests.
2. **Loop controller.** Implement `LoopController` running the ten stages in order, delegating to Phase 11
   services; enforce `BudgetEnvelope` with deterministic arithmetic; abort on overrun. *Check:* a dry-run
   loop over a tiny fixture records all ten stages and never exceeds budget.
3. **Timescale scheduler.** Implement the five timescales (§31); each tick dispatches its handler and
   appends a `timescale.tick` event; persist `timescales`. *Check:* a fast test clock drives N ticks per
   timescale deterministically.
4. **Self-model.** Implement `SelfModel::measure` comparing Actual (event-log behavior), Model (self-claims),
   and Ideal (declared principles), producing numeric `SelfModelReport` + `Divergence` rows and
   `mm:SelfModelReport` triples. *Check:* a known Actual/Ideal gap yields the expected divergence within tolerance.
5. **Debt scanner + GC.** Implement `DebtScanner` over the code graph, memory, policies, and schemas;
   implement `Collector` with the escalation ladder. GC refuses any subject carrying the immutable-ledger
   marker and records `mm:protectsLedger true`. *Check:* seeded unused capability / duplicate policy / stale
   memory are found; a protected subject is never collected.
6. **Design writer.** Implement `DesignWriter::revise` so design-doc changes flow through `ChangeSet` +
   promotion (never a direct write) and register `mm:DesignRevision`. *Check:* revising a fixture doc creates
   a `ChangeSet`; rejection leaves the doc byte-identical.
7. **Module hot-loader.** Implement `ModuleLoader::hot_load` over the nexus lifecycle; loading a promoted
   `ModuleVersion` registers `mm:ModuleLoad`, exposes its T-Box functions immediately, and leaves prior
   modules running. *Check:* hot-load a fixture module and call its function without a restart.
8. **Qualification run.** Add `bench/qualification/{goal_novel.json,budget.toml,baseline.json}` and
   `tests/e2e/qualification.rs`. The scenario: a novel goal (absent from every fixture corpus) must produce
   a design revision, a Pi-authored module, tests, a benchmark comparison, a promotion decision with a
   written reason, and a successful hot-load — with no human edits.
9. **Replay + provenance verification.** Implement `loop replay` and `loop artifacts`: replay from
   `(code_version, config_hash, event log)` must reproduce the run byte-identically, and `loop artifacts`
   must return `mmc:PiSession → mmc:Edit → mmc:ModuleVersion → mm:ChangeSet → mm:Promotion`.
10. **CI wiring.** Add the qualification scenario and the full Phase 1–12 gate suite to CI; `mm-loop` runs as
    a BACKGROUND service for long runs with readiness, budget-stop, and log checks.

---

## 6. Logging and observability

All records go through `mm-log` (parent plan §10) with `trace_id` = run ULID, and GenAI-semconv attribute
names where a convention exists. Phase-12 event codes and required fields:

| Event code | Fields |
|---|---|
| `loop.run.start` / `loop.run.end` | `run_id`, `goal_ulid`, `novel`, `config_hash`, `code_version`, `budget`, `status` |
| `loop.iteration.start` / `.end` | `run_id`, `idx`, `stage`, `outcome`, `latency_ms` |
| `loop.budget.debit` / `loop.budget.deny` | `run_id`, `bucket`, `amount`, `remaining` |
| `loop.stage.resume` | `run_id`, `idx`, `stage` (crash recovery) |
| `timescale.tick` | `name`, `kind`, `tick_ms`, `handler`, `last_tick_ulid` |
| `self_model.report` | `run_id`, `actual_model`, `actual_ideal`, `model_ideal` |
| `self_model.divergence` | `report_id`, `dimension`, `value` |
| `debt.scan` / `debt.finding` | `kind`, `subject_uri`, `severity`, `evidence` |
| `gc.action` | `finding_id`, `action`, `subject_uri`, `reversible`, `ledger_impact`, `protects_ledger` |
| `gc.refuse` | `finding_id`, `subject_uri`, `reason` (protected-ledger or non-reversible) |
| `design.revision` | `doc_uri`, `revision`, `change_set_id` |
| `module.load` / `module.rollback` | `module_uri`, `version`, `status`, `latency_ms` |
| `invariant.check` | `invariant_uri`, `result`, `subject_uri` |

Operational records are best-effort; every state mutation, promotion, module load, design revision, and GC
action is a transactional audit record. `mm-cli logs verify` must pass for a qualification run (schema,
audit completeness, gapless sequence, ULID correlation, redaction, replay equivalence).

---

## 7. Testing

| Area | Test | Assertion |
|---|---|---|
| Loop ordering | `loop_controller` unit | the ten stages run in order; a stage failure aborts deterministically |
| Budget | proptest | sum of debits never exceeds `limit_amount`; denial leaves the ledger unchanged |
| Resume | kill-and-resume | `loop.stage.resume` continues at the last committed `(run_id, idx)` with no duplicate side effects |
| Timescale | fast clock | N ticks per timescale are deterministic; Identity never ticks during a run |
| Self-model | fixture with known gap | divergence equals expected within tolerance; snapshot-stable |
| GC | seeded debt + protected subject | findings found; escalation picks the cheapest rung; protected subject never collected |
| Design writer | revise fixture doc | creates a `ChangeSet`; rejection leaves the doc byte-identical |
| Hot-load | `tests/e2e/hot_load.rs` | promoted module's function is callable without restart; prior version still serving during load |
| Replay | `tests/e2e/replay_determinism.rs` | `loop replay` byte-identical from `(code, config, event log)` with zero live provider calls |
| Adversarial | `tests/e2e/gate_bypass_adversarial.rs` | crafted prompts/change-sets cannot bypass the gate; invariant violations impossible |
| Qualification | `tests/e2e/qualification.rs` | novel goal ⇒ design revision + module + tests + benchmark + reasoned decision + hot-load, no human edits |
| Conformance | RDF | self/debt graph SHACL validation; `codex verify` green after a design revision |

Fixtures: `bench/qualification/{goal_novel.json,budget.toml,baseline.json}`,
`bench/debt/seeded_debt_01.json`, `bench/self_model/gap_01.json`, plus recorded Phase-11 Pi sessions.

---

## 8. Pass gate

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

cargo run -p mm-cli -- loop status --run "$RUN"         # status=completed
cargo run -p mm-cli -- loop artifacts --run "$RUN"       # full provenance chain present
cargo run -p mm-cli -- self-model report --run "$RUN"    # divergence report emitted
cargo run -p mm-cli -- debt scan                         # actionable findings listed
cargo run -p mm-cli -- gc --apply                        # protected subjects untouched
cargo run -p mm-cli -- module load --uri "$MODULE@$VER"  # hot-loaded without restart
cargo run -p mm-cli -- codex verify                      # code graph still consistent
cargo run -p mm-cli -- being verify                      # identity invariants intact
cargo run -p mm-cli -- logs verify                       # audit chain intact after the run

# Deterministic reproduction of the entire run.
cargo run -p mm-cli -- loop replay --run "$RUN"          # byte-identical artifacts
```

**Pass criteria (definition of "functional prototype"):**

1. The novel-goal run, with **no human code edits**, produces (a) a design-doc revision, (b) a Pi-authored
   module, (c) passing tests, (d) a benchmark comparison, (e) a promotion decision with a written reason,
   and (f) a successful hot-load.
2. `loop artifacts` returns the complete chain `mmc:PiSession → mmc:Edit → mmc:ModuleVersion →
   mm:ChangeSet → mm:Promotion` in RDF.
3. `loop replay` reproduces the run deterministically from the event log.
4. `self-model report` emits a numeric divergence report; `debt scan` + `gc --apply` complete with
   actionable findings and no protected subject collected.
5. `logs verify` passes (schema, audit completeness, gapless sequence, ULID correlation, redaction).
6. The whole workspace builds with zero warnings and the full Phase 1–12 regression suite is green.
7. No identity invariant was violated during the run (deterministic `invariant.check` verified).

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Long autonomous runs consume unbounded resources | `BudgetEnvelope` enforced deterministically with hard deny; `mm-loop` runs as a monitored BACKGROUND service with readiness + budget-stop |
| Replay diverges because of Pi/LLM nondeterminism | Pi sessions are recorded and replayed from the event log; `loop replay` uses recorded artifacts, never live provider calls |
| A crash mid-run corrupts state or double-applies a stage | Stages are idempotent handlers keyed `(run_id, idx)`; state is a fold over the append-only event log; `loop.stage.resume` |
| GC deletes something needed | Immutable-ledger marker + `mm:protectsLedger` + `gc.refuse`; every GC action is reversible and logged; reversibility is checked before apply |
| Self-model report is a narrative, not a measurement | Divergences are numeric, computed from event-log behavior vs declared principles, snapshot-tested |
| Design writer directly edits production docs | Design revisions go through `ChangeSet` + promotion only; rejected revisions leave docs byte-identical |
| Hot-load destabilizes the running process | Nexus plugin lifecycle with rollback to the prior `ModuleVersion`; prior versions keep serving during load |
| Reward hacking / self-confirming evaluation | Frozen harness within a cycle; held-out qualification bench; adversarial + regression suites; gate changes are themselves change-sets |
| Gate bypass via crafted prompts | Adversarial e2e suite asserts the promotion gate and identity invariants cannot be bypassed by model output |
| Telemetry becomes bespoke and unqueryable | `mm-log` traces use OpenTelemetry GenAI semconv names, so runs are inspectable with standard tooling |
| Self-improvement outruns understanding | Every promoted version is an archived stepping stone in `evolution_journal`; the loop may branch only from named, journaled versions |

---

## 10. References (code-available)

- Awesome-Self-Evolving-Agents — https://github.com/XMUDeepLIT/Awesome-Self-Evolving-Agents
- Awesome-Reliable-Self-Evolving-Agents — https://github.com/wkqdzkd/Awesome-Reliable-Self-Evolving-Agents
- Self-evolving agents survey — https://arxiv.org/html/2507.21046v1
- Temporal (durable workflow replay) — https://github.com/temporalio/temporal
- Restate (journaled idempotent handlers) — https://github.com/restatedev/restate
- cqrs-es (append-only events + projections) — https://github.com/j5ik2o/cqrs-es-example-rs
- event-sourcing (crate) — https://crates.io/crates/event-sourcing
- Langfuse (agent tracing + eval-as-code) — https://github.com/langfuse/langfuse
- OpenTelemetry GenAI semantic conventions — https://opentelemetry.io/docs/specs/semconv/gen-ai/
- Darwin Gödel Machine (bounded self-improvement, lineage archive) — https://github.com/jennyzzt/dgm
- Gödel Agent (bounded recursive self-modification) — https://github.com/Arvid-pku/Godel_Agent
- nexus plugin lifecycle (hot module load) — parent plan §7 `vendor/nexus/`
- `evolution-ir`, `ensemble-ir`, `rdf-codec`, `rdf-shacl`, `causal-ir`, `temporal-store` (rust_symbolic) — parent plan §7 `vendor/rust_symbolic/`
- Oxigraph (RDF store for self/debt graphs) — https://github.com/oxigraph/oxigraph
- Design sources: `digital_mind_design1.md` §31, §62, §67–69, §85–86, §103–110; `companion_loop1/2.md`; `personality_simulator3.md`; `meta_analysis2.md`; parent plan §11 (Pi contract)
