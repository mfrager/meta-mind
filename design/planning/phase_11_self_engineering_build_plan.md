# Phase 11 Build Plan — Self-engineering: meta-analysis, calibration, mistake learning, and Pi-driven module construction

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 11 · Codename Metamind (`mm`)
> Design of record: `design/digital_mind_design1.md` §43, §55–72, §84–86, §103–110;
> supporting: `bootstrap_procses2.md`, `meta_analysis1.md`, `meta_analysis2.md`, `companion_loop1.md`.

---

## 1. Objective and scope

Close the improvement loop so the being can observe itself, form theories about itself, experiment on
its own behavior, and turn those experiments into promoted capabilities. This phase delivers the four
organs that make that loop real:

1. **`mm-metaanalysis`** — event-triggered introspection, an explicit error taxonomy, an immutable
   prediction ledger, and calibration mathematics (Brier, log loss, ECE, reliability, selective risk).
2. **Mistake→regression-test compiler** — every meaningful failure becomes a permanent test in
   `bench/regression/` (design §60; `bootstrap_procses2.md` §9, §11).
3. **`mm-selfeng`** — `ChangeSet` assembly, a git-worktree sandbox, benchmark/shadow evaluation, the
   `PromotionGate`, rollback, the evolution journal, and the four deterministic budgets.
4. **`mm-pi`** — the async Pi RPC client and the Pi-session ingester that gives every generated line of
   code provenance. Pi is the **only** code editor (parent plan §11); the runtime never writes
   production source.

**In scope:** prediction ledger + calibration; adaptive meta-analysis + taxonomy; mistake→test; change
sets; sandbox build/test; benchmark + shadow; promotion/reject with written reason; rollback; budgets;
Pi RPC control; Pi session ingestion; the first gap→module→promote workflow.

**Out of scope (deferred):** hot-loading a promoted module into a running process (Phase 12), generating
design documents and phase plans (Phase 12), architectural evolution beyond policy/code change sets
(Phase 12), and any modification of identity invariants or permissions (never automatic, per
`meta_analysis1.md` §8).

---

## 2. Prerequisites and dependencies

Must be green before Phase 11 starts:

- Phases 1–10 pass gates, especially: the event log + replay (P1); the code-metadata graph `mmc:` (P2);
  the LLM substrate with offline replay (P3); memory + mistake records (P5); the epistemic dependency
  graph and contradiction records (P6); the cognitive library + policy genome (P7); the metacognitive
  controller (P8); `DecisionCore` + firewall outcomes (P9); the deterministic executor, tool registry,
  permission engine, action ledger, rollback, and sandbox primitives (P10).
- External: Pi `0.85.1` present at `~/.nvm/.../bin/pi`; `git` available; `OPENAI_BASE_URL` /
  `OPENAI_API_KEY` set for the runtime and Pi (no endpoint URL is hard-coded).

---

## 3. Deliverables (exact paths)

```
crates/mm-metaanalysis/
├── Cargo.toml
├── src/lib.rs              # public re-exports
├── src/ledger.rs           # Prediction, PredictionOutcome, immutable append
├── src/calibration.rs      # CalibrationReport, Calibrator trait, Brier/log-loss/ECE/bins
├── src/taxonomy.rs         # ErrorClass, ErrorTaxonomy, classification
├── src/triggers.rs         # Trigger detection from the event log
├── src/diagnosis.rs        # LLM-backed diagnosis over a bounded context
├── src/lessons.rs          # Lesson extraction + write-back to the cognitive library
└── tests/                  # ledger, calibration golden, taxonomy

crates/mm-mistakes/          # mistake→test compiler (design §60)
├── src/lib.rs
├── src/reproduce.rs         # minimize an incident to a reproducing case
├── src/emit.rs              # emit a runnable regression test (Rust + fixture)
└── tests/

crates/mm-selfeng/
├── Cargo.toml
├── src/lib.rs
├── src/changeset.rs         # ChangeSet, Patch, Migration, PolicyDelta, RollbackPlan
├── src/sandbox.rs           # git worktree under data/sandbox/<ulid>/
├── src/benchmark.rs         # local fitness
├── src/shadow.rs            # shadow run + comparison to baseline
├── src/promotion.rs         # PromotionGate trait + default policy gate
├── src/rollback.rs          # apply/verify rollback
├── src/journal.rs           # EvolutionEvent append (Phase 12 reads it)
├── src/budget.rs            # BudgetKind, BudgetLedger (deterministic)
└── tests/

crates/mm-pi/
├── Cargo.toml
├── src/lib.rs
├── src/protocol.rs          # PiCommand / PiEvent wire types (strict JSONL)
├── src/rpc.rs               # PiClient (spawn, prompt, steer, follow_up, abort, new_session)
├── src/framing.rs           # LF-only splitter (strip trailing \r; never U+2028/U+2029)
├── src/session.rs           # PiSession, PiEdit, PiToolCall (version:3, parentId chain)
├── src/ingest.rs            # JSONL → event log + /code graph
├── fixtures/                # recorded sessions (protocol fixtures)
└── tests/

crates/mm-store-sqlite/migrations/
└── 0011_self_engineering.sql

ontology/
├── mm.ttl                   # + Prediction, EvolutionEvent, ChangeSet, Lesson, MetaAnalysis
├── code.ttl                 # + PiSession, Edit, ToolCall, generatedBy, parentSession
└── shapes/phase11_{metaanalysis,changeset,pisession}.ttl

pi/
├── system_prompt.md         # module contract, forbidden actions, required outputs
├── skills/mm-module-scaffold/SKILL.md
├── prompts/module_scaffold.md
└── extensions/metadata_emit.ts   # emit metadata.ttl after a module is written

modules/cognition/calibration/   # thin capability module over mm-metaanalysis (nexus plugin)
bench/regression/  bench/calibration/  bench/promotion/  bench/pi/
```

`mm-cli` gains: `meta analyze|queue|lessons`, `calibrate`, `changeset new|show`, `sandbox run`,
`promote|reject <ulid>`, `budget show`, `pi run|ingest`, `regression run`.

---

## 4. Data model and ontology deltas

Named graphs: `/provenance` (decisions, changes, promotions, Pi edits) and `/code` (module metadata).
ULID IRIs `<https://metamind.dev/data/{ulid}>`; source-anchored URIs
`https://metamind.dev/design/digital_mind_design1.md#60`.

`0011_self_engineering.sql`:

```sql
CREATE TABLE predictions (
  id TEXT(26) PRIMARY KEY,
  proposition TEXT NOT NULL,
  probability REAL NOT NULL CHECK (probability >= 0.0 AND probability <= 1.0),
  horizon_seconds INTEGER NOT NULL,
  conditions_json TEXT NOT NULL,
  episode_ulid TEXT(26),
  created_at TEXT NOT NULL
);
CREATE TABLE prediction_outcomes (
  prediction_ulid TEXT(26) PRIMARY KEY REFERENCES predictions(id),
  observed INTEGER NOT NULL CHECK (observed IN (0,1)),
  resolved_at TEXT NOT NULL,
  evidence_ulid TEXT(26) NOT NULL
);
CREATE TABLE calibration_runs (
  id TEXT(26) PRIMARY KEY,
  subject TEXT NOT NULL,
  n INTEGER NOT NULL,
  brier REAL NOT NULL,
  log_loss REAL NOT NULL,
  ece REAL NOT NULL,
  coverage REAL,
  selective_risk REAL,
  baseline_brier REAL,
  created_at TEXT NOT NULL
);
CREATE TABLE meta_analyses (
  id TEXT(26) PRIMARY KEY,
  trigger TEXT NOT NULL,
  episode_ulid TEXT(26),
  diagnosis TEXT NOT NULL,
  error_classes_json TEXT NOT NULL,
  recurrence REAL NOT NULL,
  impact REAL NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE lessons (
  id TEXT(26) PRIMARY KEY,
  analysis_ulid TEXT(26) NOT NULL REFERENCES meta_analyses(id),
  text TEXT NOT NULL,
  confidence REAL NOT NULL,
  evidence_count INTEGER NOT NULL,
  domains_json TEXT NOT NULL,
  policy_effect TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE change_sets (
  id TEXT(26) PRIMARY KEY,
  reason TEXT NOT NULL,
  hypothesis TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  rollback_json TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN
    ('draft','sandboxed','built','tested','benchmarked','shadowed','promoted','rejected')),
  created_at TEXT NOT NULL
);
CREATE TABLE promotions (
  id TEXT(26) PRIMARY KEY,
  changeset_ulid TEXT(26) NOT NULL REFERENCES change_sets(id),
  decision TEXT NOT NULL CHECK (decision IN ('promote','reject')),
  reason TEXT NOT NULL,
  evidence_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE regression_tests (
  id TEXT(26) PRIMARY KEY,
  mistake_ulid TEXT(26),
  path TEXT NOT NULL UNIQUE,
  fails_before_ulid TEXT(26),
  passes_after_ulid TEXT(26),
  created_at TEXT NOT NULL
);
CREATE TABLE evolution_journal (
  id TEXT(26) PRIMARY KEY,
  date TEXT NOT NULL,
  reason TEXT NOT NULL,
  evidence_json TEXT NOT NULL,
  hypothesis TEXT NOT NULL,
  changeset_ulid TEXT(26),
  outcome TEXT NOT NULL,
  decision TEXT NOT NULL,
  self_version TEXT NOT NULL
);
CREATE TABLE budgets (
  id TEXT(26) PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('meta_analysis','improvement','evolution','metacognitive')),
  period TEXT NOT NULL,
  limit_amount REAL NOT NULL,
  spent_amount REAL NOT NULL DEFAULT 0,
  UNIQUE (kind, period)
);
CREATE TABLE pi_sessions (
  id TEXT(26) PRIMARY KEY,
  session_file TEXT NOT NULL,
  changeset_ulid TEXT(26),
  model TEXT,
  cwd TEXT,
  started_at TEXT NOT NULL,
  ended_at TEXT,
  event_count INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE pi_events (
  id TEXT(26) PRIMARY KEY,
  session_ulid TEXT(26) NOT NULL REFERENCES pi_sessions(id),
  seq INTEGER NOT NULL,
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  UNIQUE (session_ulid, seq)
);
```

Ontology additions (`ontology/mm.ttl`):

```turtle
mm:Prediction     a owl:Class ; rdfs:subClassOf mm:Claim .
mm:MetaAnalysis   a owl:Class .
mm:Lesson         a owl:Class .
mm:ChangeSet      a owl:Class .
mm:EvolutionEvent a owl:Class .
mm:hasOutcome     a owl:ObjectProperty ; rdfs:domain mm:Prediction .
mm:promotedFrom   a owl:ObjectProperty .
mm:policyEffect   a owl:DatatypeProperty .
mmc:PiSession     a owl:Class .
mmc:Edit          a owl:Class .
mmc:ToolCall      a owl:Class .
mmc:generatedBy   a owl:ObjectProperty .
mmc:parentSession a owl:ObjectProperty .
mmc:sessionFile   a owl:DatatypeProperty .
```

SHACL (generated via `rdf-shacl::generate_shacl`, hand-written where the fragment does not fit):
a `mm:Prediction` requires exactly one `mm:probability` in `[0,1]`; a `mm:ChangeSet` requires `mm:reason`,
`mm:hypothesis`, and a `mm:rollbackPlan`; a `mmc:PiSession` requires `mmc:sessionFile` and its events must
have gapless `mmc:seq`.

---

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

```rust
// mm-metaanalysis
pub struct Prediction { pub id: Ulid, pub proposition: String, pub probability: f32,
    pub horizon: Duration, pub conditions: Vec<String>, pub created_at: Timestamp,
    pub outcome: Option<PredictionOutcome> }
pub struct CalibrationReport { pub n: u32, pub brier: f64, pub log_loss: f64, pub ece: f64,
    pub bins: Vec<ReliabilityBin>, pub coverage: f64, pub selective_risk: f64 }
pub trait Calibrator { fn score(&self, labeled: &[Labeled]) -> CalibrationReport;
    fn threshold_for(&self, class: &str, target_precision: f64) -> f64; }
pub enum ErrorClass { Knowledge, Retrieval, Interpretation, Comparison, Assumption, Causal,
    Planning, Decision, Execution, Verification, Social, Resource, Policy, Code, Data, Model }
pub enum Trigger { PredictionError, UserCorrection, RepeatedFailure, Contradiction,
    UnexpectedOutcome, GoalFailure, NearMiss, NovelSuccess }
pub struct MetaAnalysis { pub id: Ulid, pub trigger: Trigger, pub episode: Option<Ulid>,
    pub diagnosis: String, pub error_classes: Vec<ErrorClass>, pub recurrence: f32,
    pub impact: f32, pub lessons: Vec<Lesson> }
pub async fn analyze(episode: Ulid, ctx: &MetaContext) -> Result<MetaAnalysis>;

// mm-mistakes
pub struct RegressionTest { pub id: Ulid, pub path: PathBuf, pub fails_before: Ulid,
    pub passes_after: Ulid }
pub async fn compile_mistake(mistake: Ulid, repo: &Path) -> Result<RegressionTest>;

// mm-selfeng
pub struct ChangeSet { pub id: Ulid, pub reason: String, pub hypothesis: String,
    pub code: Vec<Patch>, pub schema_migrations: Vec<MigrationId>, pub data_migrations: Vec<MigrationId>,
    pub memory_transforms: Vec<TransformId>, pub policies: Vec<PolicyDelta>, pub prompts: Vec<PromptDelta>,
    pub tests: Vec<TestId>, pub benchmarks: Vec<BenchmarkId>, pub rollback: RollbackPlan }
pub enum PromotionDecision { Promote, Reject { reason: String } }
pub trait PromotionGate { fn evaluate(&self, cs: &ChangeSet, ev: &EvidenceBundle) -> PromotionDecision; }
pub trait Sandbox { async fn prepare(&self, cs: &ChangeSet) -> Result<SandboxDir>;
    async fn build(&self, dir: &SandboxDir) -> Result<BuildResult>;
    async fn test(&self, dir: &SandboxDir) -> Result<TestResult>; }
pub enum BudgetKind { MetaAnalysis, Improvement, Evolution, Metacognitive }
pub trait BudgetLedger { async fn debit(&self, kind: BudgetKind, amount: f64, trace: Ulid) -> Result<()>;
    async fn remaining(&self, kind: BudgetKind) -> Result<f64>; }

// mm-pi
pub enum PiCommand { Prompt{ id: String, message: String, streaming_behavior: Option<String> },
    Steer{ id: String, message: String }, FollowUp{ id: String, message: String },
    Abort{ id: String }, ClearQueue{ id: String }, NewSession{ id: String } }
pub enum PiEvent { Response{ id: Option<String>, command: String, success: bool },
    Agent{ raw: serde_json::Value } }
pub struct PiClient;
impl PiClient {
    pub async fn spawn(cfg: &PiConfig) -> Result<Self>;
    pub async fn send(&mut self, cmd: PiCommand) -> Result<()>;
    pub async fn prompt(&mut self, id: &str, message: &str) -> Result<()>;
    pub async fn steer(&mut self, id: &str, message: &str) -> Result<()>;
    pub async fn abort(&mut self, id: &str) -> Result<()>;
    pub async fn new_session(&mut self, id: &str) -> Result<()>;
    pub fn events(&mut self) -> impl futures::Stream<Item = PiEvent> + '_;
}
pub async fn ingest_session(path: &Path) -> Result<PiSessionGraph>;   // version:3, parentId chain
```

CLI (all use exit codes for gates): `mm-cli meta analyze --episode <ulid>`,
`mm-cli calibrate --bench <jsonl>`, `mm-cli changeset new --from-gap <json>`,
`mm-cli sandbox run <changeset-ulid>`, `mm-cli promote <ulid>` / `mm-cli reject <ulid> --reason <s>`,
`mm-cli budget show`, `mm-cli pi run --task <json>` / `mm-cli pi ingest <session.jsonl>`,
`mm-cli regression run --suite bench/regression`.

`modules/cognition/calibration/plugin.toml`:

```toml
[plugin]
name = "mm-calibration"
uri = "https://metamind.dev/code/module/cognition/calibration"
version = "0.1.0"
[metadata]
category = "cognition"
owned_by_phase = 11
capability = "mm:Calibration"
[tbox.functions]
"cognition.calibrate" = { source = "handlers::calibrate" }
[monad.operations]
name = "cognition"
arity = 1
[build]
rust_edition = "2021"
```

---

## 6. External references copied in and integration

Per parent plan §7, external code is **reference only**: the needed source is copied into
`vendor/<origin>/` as workspace members and integrated behind `mm-*` traits, with `mmc:copiedFrom`
(repo, revision, license) emitted by the Phase 2 scan and a `vendor/<origin>/COPYING.md`.

| Need | Reference | Copy/integration |
|---|---|---|
| Diagnosis + lesson extraction | `kg-llm` (`LlmClient`, `CachedLlmClient`, schemas) | copied into `vendor/rust_extract/`; `mm-metaanalysis::diagnosis` calls it via `mm-llm` |
| Root-cause / intervention reasoning | `causal-ir` (`CausalEngine`, `Counterfactual`) | copied into `vendor/rust_symbolic/`; `mm-selfeng` uses it to localize the cause substrate |
| Policy/gene fitness, mutation | `learning-ir`, `evolution-ir` (`EvolutionEngine`, fitness) | copied; `mm-selfeng` policy deltas |
| Metadata validation | `rdf-shacl` (`generate_shacl`, `validate`) | copied; validates `ChangeSet`/`PiSession` graphs |
| Change-set artifact + replay | `rust_extract` versioned JSONL artifact pattern | copied as a pattern into `mm-selfeng::changeset` |
| Module load (Phase 12) | nexus plugin lifecycle (`nexus-core`) | copied in Phase 12; Phase 11 only registers metadata |

**Pi is an external process, not copied source.** `mm-pi` invokes the installed `pi` binary over its
documented RPC protocol (parent plan §11); it is recorded as an external tool dependency with its
version in the `evolution_journal`/`config`, not vendored.

---

## 7. Step-by-step implementation tasks

1. **Prediction ledger + calibration.** Implement `mm-metaanalysis::ledger` (append-only, ULID, no
   update/delete) and `calibration::Calibrator` (Brier, log loss, ECE with configurable bins,
   reliability, selective risk/coverage, `threshold_for`). Require every consequential decision/prediction
   to write a `predictions` row. Check: golden vectors vs `bench/calibration/labeled.jsonl`.
2. **Event-triggered meta-analysis.** `triggers.rs` watches the event log for the eight triggers and
   computes `MetaAnalysisPriority = Novelty + Failure + Impact + Recurrence + Uncertainty`
   (`meta_analysis1.md` §3). `diagnosis.rs` runs one bounded LLM pass over the episode context (never the
   whole log) and classifies into `ErrorClass`. Check: a seeded failure produces one `meta_analyses` row
   with ≥1 error class.
3. **Mistake→test compiler.** `mm-mistakes` minimizes an incident into a reproducing case, emits a test
   plus fixture into `bench/regression/`, records `regression_tests`, and asserts fail-before/pass-after.
   New tests are retained forever. Check: seeded bug reproduces, fails before, passes after.
4. **ChangeSet + sandbox.** `changeset.rs` assembles a typed `ChangeSet` (code/schema/data/memory/policy/
   prompt/tests/benchmarks/rollback). `sandbox.rs` creates a git worktree at `data/sandbox/<changeset-ulid>/`,
   runs `cargo build`/`cargo test`, and records results. The production tree is never written. Check:
   filesystem audit shows writes only under the sandbox.
5. **Promotion pipeline + gate.** `benchmark.rs` (local fitness per `bootstrap_procses2.md` §14),
   `shadow.rs` (compare to baseline), `promotion.rs` (`PromotionGate` default policy: reject on any
   regression, on risk above `RiskBudget`, or on hard-prohibition hit — `meta_analysis1.md` §8), and
   `rollback.rs`. A promotion updates the Phase 2 code-metadata graph and appends to `evolution_journal`.
   Check: negative fixtures reject with a written reason.
6. **`mm-pi`.** `framing.rs` (LF-only splitter; strip trailing `\r`), `protocol.rs`, and `rpc.rs`
   (spawn `pi --mode rpc --provider <p> --model <m> --session-dir data/sandbox/pi`, restricted `--tools`,
   id-correlated commands). `session.rs`/`ingest.rs` parse Pi JSONL `version:3` with the `parentId` chain
   and write `pi_sessions`/`pi_events` plus `/code` RDF (`mmc:PiSession`, `mmc:Edit`, `mmc:ToolCall`).
   Check: recorded-session replay is deterministic and ingestion round-trips to identical canonical RDF.
7. **First generated-module workflow.** Wire gap→`ChangeSet`→Pi scaffolds the module from
   `pi/skills/mm-module-scaffold`→tests→benchmark→gate. `pi/extensions/metadata_emit.ts` writes
   `metadata.ttl`. Check: end-to-end produces a registered module or a rejection with reason, no human edits.
8. **Budgets.** `budget.rs` enforces the four budgets (`meta_analysis`, `improvement`, `evolution`,
   `metacognitive`) with deterministic arithmetic and a hard deny when exhausted (`meta_analysis1.md` §1,
   §10). Every debit is an audit record. Check: an overspend attempt is denied and logged.

---

## 8. Detailed logging requirements

All records go through `mm-log` with the global fields (`ts`, `level`, `target`, `event`, `trace_id`,
`span_id`, …). Phase-11 event codes and required fields:

- `meta.trigger` — `trigger`, `episode_id`, `priority_components{novelty,failure,impact,recurrence,uncertainty}`.
- `meta.diagnose` — `analysis_id`, `error_classes[]`, `model`, `prompt_hash`, `latency_ms`, `cost`.
- `meta.lesson` — `lesson_id`, `confidence`, `evidence_count`, `domains[]`, `policy_effect`.
- `calib.report` — `subject`, `n`, `brier`, `log_loss`, `ece`, `coverage`, `baseline_brier`.
- `mistake.test.create` — `mistake_id`, `test_path`, `fails_before`, `passes_after`.
- `regression.run` — `suite`, `passed`, `failed`, `duration_ms`, `first_failure`.
- `changeset.create` / `changeset.stage` — `change_set_id`, `status`, `reason`, `hypothesis`.
- `sandbox.prepare` / `sandbox.build` / `sandbox.test` — `change_set_id`, `dir`, `exit_code`, `warnings`.
- `bench.run` / `shadow.start` — `change_set_id`, `baseline`, `candidate`, `delta`, `n`.
- `gate.decide` — `change_set_id`, `decision`, `reason`, `evidence{}`, `risk`, `budget_kind`.
- `promote.commit` / `promote.reject` — `change_set_id`, `module_uri`, `self_version`, `reason`.
- `rollback.apply` — `change_set_id`, `restored_state_hash`.
- `budget.debit` / `budget.deny` — `kind`, `amount`, `remaining`, `period`.
- `pi.spawn` / `pi.command` / `pi.event` — `session_id`, `command`, `id`, `success`, `seq`.
- `pi.session.ingest` / `pi.edit` — `session_id`, `session_file`, `event_count`, `file`, `tool`.
- `copy.integrate` — `origin`, `revision`, `license`, `mmc:copiedFrom` IRI.

Every state-changing record is also an append-only audit record; `mm-cli logs verify` must pass.

---

## 9. Testing plan

- **Pi protocol** (`crates/mm-pi/tests`): replay recorded fixtures; assert LF-only framing including a
  `\r\n` case and a payload containing `U+2028`/`U+2029` that must NOT split; id correlation for
  `prompt`/`steer`/`follow_up`/`abort`/`new_session`; `success:false` handling; abort drains the queue.
- **Session ingestion**: ingest a fixture with a `parentId` chain; assert canonical RDF equals a golden
  file and `pi_events.seq` is gapless; a truncated session is a hard error.
- **Sandbox isolation**: build/test in a worktree and assert (via filesystem audit) zero writes outside
  `data/sandbox/`; a candidate that tries to write the production tree is rejected.
- **ChangeSet**: schema validation for every component; a `ChangeSet` missing `rollback` is rejected;
  a round-trip through JSON is stable.
- **Promotion gate**: negative fixtures (regression introduced, risk > budget, hard prohibition,
  no benchmark evidence) each reject with a specific reason; a clean candidate promotes.
- **Calibration**: Brier/log-loss/ECE match reference values within 1e-9; ECE bin-count invariance for
  equal edges; `threshold_for` returns the precision-target threshold on a labeled set.
- **Mistake→test**: the seeded bug's regression test fails before and passes after the fix; the test is
  retained and discovered by `regression run`.
- **Budgets**: property test that the sum of debits can never exceed `limit_amount`; denial leaves the
  ledger unchanged; audit sequence remains gapless.
- **Replay/determinism**: a full sandbox→benchmark→gate cycle replays identically from
  `(code version, event log, config)` with zero provider calls in replay mode.

---

## 10. Pass gate

Run as plain commands; every command must exit 0 (an earlier gate failing is a Phase 11 failure):

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Calibration improves over the recorded baseline on a labeled cycle (objective thresholds).
# The corpus is the phase's own `predictions.jsonl`, which `compute_reference.py` builds and
# `reference.json` grades — not Phase 9's `labeled.jsonl`, whose calibrated Brier floor is
# ~0.212 under any temperature.
mm-cli calibrate --bench bench/calibration/predictions.jsonl \
  --assert-brier-le 0.20 --assert-ece-le 0.10 --assert-improves-baseline

# Event-triggered meta-analysis produces a diagnosis + lesson
mm-cli meta analyze --episode bench/episodes/seeded_failure_01.json --assert-lessons-ge 1

# Mistake becomes a retained regression test that fails before and passes after
mm-cli regression run --suite bench/regression \
  --assert-fail-before-pass-after seeded_bug_01

# Gap → ChangeSet → Pi-authored module → sandbox build/test → benchmark → gate
mm-cli changeset new --from-gap bench/gaps/gap_01.json
# `--offline` replays the task's recorded session (deterministic, no provider); without it
# the contract goes to a live session, which needs `--provider` and `--model`.
mm-cli pi run --task bench/pi/module_scaffold_01.json --offline --assert-session-ingested
mm-cli sandbox run "$CHANGESET" --assert-isolated
mm-cli promote "$CHANGESET" --assert-reason-present
mm-cli codex verify

# No production writes outside promotion
mm-cli audit production-tree --assert-unmodified-outside-promotion

# Budgets cannot be exceeded
mm-cli budget show --assert-no-overspend

# Logging + provenance
mm-cli logs verify
```

Objective criteria:

- The seeded gap yields a `ChangeSet`; Pi generates the module; the sandbox builds and tests it; a
  benchmark compares it to a baseline; the gate emits `promote` or `reject` **with a written reason** —
  with no human code edits.
- The seeded bug's regression test in `bench/regression/` fails before and passes after the fix and is
  retained.
- The production tree is unmodified except by a promotion (filesystem audit + event log agree).
- A promoted module is registered in the code-metadata graph with code+data+schema+policy+prompt versions
  and `codex verify` stays green.
- Calibration Brier/log-loss/ECE improve over the recorded baseline across one labeled cycle.
- No budget is exceeded; every meta-analysis, change, and promotion is journaled immutably.
- `mm-cli logs verify` passes (schema, audit completeness, gapless sequence, ULID correlation, redaction,
  replay equivalence).

---

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Pi output is nondeterministic | Sandbox-only execution; deterministic build/test/benchmark gate; full session ingestion for provenance; a candidate is judged by evidence, never by the LLM |
| RPC framing bugs corrupt streams | LF-only splitter with `\r` stripping; fixtures with `U+2028`/`U+2029`; id correlation; recorded-session replay tests |
| Session file format drift (Pi upgrades) | `version:3` header checked; unknown record kinds are hard errors, not skipped; re-ingest is a change-set |
| Self-modification bypasses the gate | Production is never written by Pi; promotion is the only writer; `audit production-tree` gate |
| Calibration over-trust (Jev confidence treated as truth) | Probabilities are scored against outcomes; thresholds learned empirically; confidence carries no authority (design §49) |
| Budget runaway / runaway introspection | Deterministic `BudgetLedger`; hard deny; event-triggered (not scheduled) analysis |
| Awarding success without evidence | Promotion requires benchmark + regression evidence; hard prohibitions short-circuit to reject |
| Sandbox escape | Capability-bounded sandbox (Phase 10 primitives); filesystem audit test |
| Identity/permission drift | Identity invariants and permissions are never auto-evolved (`meta_analysis1.md` §8) |

---

## 12. Design traceability

| Design | This plan |
|---|---|
| `digital_mind_design1.md` §43 budgets, §55–58 self-bootstrap/capabilities, §59 sandbox/promotion, §60 mistakes→tests, §63 meta-analysis, §66 evolution journal, §68 CI of capabilities, §70 code improvement, §71 data improvement, §72 change sets, §84 error taxonomy, §85 architecture debt, §86 GC, §103 continuous loop, §104 control plane, §108 overbuild guardrails, §109–110 invariants | §1–§11 |
| `bootstrap_procses2.md` §3 self-engineering loop, §4 code/data versioning, §7 capability packages, §8 experimental modification, §9–11 tests from mistakes, §13 escalation levels, §14 two fitness measures, §16 backlog | §1, §4, §5, §7, §9 |
| `meta_analysis1.md` §1 three+ budgets, §2 ROI, §3 adaptive meta-analysis, §4 improvement queue, §5 experiments, §6 policy genome, §7–8 safe mutation/risk budget, §9 developmental stages, §11 lessons, §13 three selves, §14 evolution journal, §15 loop | §2, §5, §7, §10 |
| `meta_analysis2.md` §1 cognitive state, §2 scan, §5 information gain, §14 metacognitive budget | §7, §8 |
| `companion_loop1.md` §1 loop, §11 question cost, §16 policy learning | §7 (budget/policy hooks) |
| Parent plan `implementation_plan1.md` §7 copy-in, §9 testing, §10 logging, §11 Pi contract, §13 Phase 11 | §2–§10 |
