# Phase 11 — Self-Engineering (Extended Build Plan)

> Extended from: `design/planning/phase_11_self_engineering_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 11 · Codename Metamind (`mm`)

This is the research-grounded, streamlined revision. It keeps the same scope and pass gate but locks in
the mechanisms: an immutable prediction ledger with calibration, an event-triggered meta-analysis, a
mistake→regression-test compiler, typed change sets executed in a sandbox, a deterministic promotion gate,
and **Pi as the only code editor**, driven over RPC and fully ingested for provenance.

---

## 0. Research foundation (code-available)

Every idea below is borrowed from a project whose source is available. Only ideas with code are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| Code mutation as hypothesis + empirical validation + lineage archive | Darwin Gödel Machine — https://github.com/jennyzzt/dgm | Self-modification is a *candidate* judged by benchmarks, with an archive of stepping-stone versions | Every change is a `ChangeSet` evaluated by the promotion gate; accepted changes append to `evolution_journal` as a lineage |
| Bounded recursive self-improvement | Gödel Agent — https://github.com/Arvid-pku/Godel_Agent | A self-modification loop that edits its own policy/code under limits | Mutation scope is capped by the evolution budget and never touches identity invariants/permissions |
| Improving the improver under a fixed harness | STOP (Self-Taught Optimizer) — https://github.com/microsoft/stop | Optimize the improvement procedure itself, but only against a frozen evaluation | The promotion gate and benchmark harness are frozen within a cycle; gate changes are themselves change-sets |
| Agent–computer interface; issue→patch→test loop | SWE-agent — https://github.com/swe-agent/swe-agent · mini-swe-agent — https://github.com/swe-agent/mini-swe-agent | Minimal, inspectable edit/run loop | Pi is driven with a restricted toolset and an explicit task contract |
| Real-issue repair evaluation | SWE-bench — https://github.com/SWE-bench/SWE-bench | Evaluation methodology (resolve rate, regression checks) | Promotion compares candidate vs baseline on a held-out local bench + the regression suite |
| Agentic edit/test loop + event stream | OpenHands — https://github.com/All-Hands-AI/OpenHands | Streaming agent events as first-class records | Pi JSONL events are ingested into the event log + `/code` graph; nothing is lost |
| Repo-map context selection | Aider — https://github.com/Aider-AI/aider | Select the smallest relevant context for an edit | Pi tasks carry a bounded file allowlist; the runtime never sends the whole tree |
| Program-analysis-guided localization | AutoCodeRover — https://github.com/nus-apr/auto-code-rover | Localize before editing using the code graph | Phase 2 `mmc:` symbol/reference graph feeds localization |
| Failure→lesson→reuse | Reflexion — https://github.com/noahshinn/reflexion | Verbal lessons stored and retrieved | Lessons land in the cognitive library (Phase 7); mistakes become tests |

---

## 1. Objective and scope

Close the improvement loop: the being observes itself, forms falsifiable theories, experiments on its own
behavior, and turns successful experiments into promoted capabilities. Four organs:

1. **`mm-metaanalysis`** — prediction ledger + calibration (Brier/log-loss/ECE/coverage/selective risk), an
   error taxonomy, event-triggered diagnosis, and lesson extraction.
2. **`mm-mistakes`** — the mistake→regression-test compiler (design §60).
3. **`mm-selfeng`** — `ChangeSet` assembly, git-worktree sandbox, benchmark + shadow, the `PromotionGate`,
   rollback, the evolution journal, and the four deterministic budgets.
4. **`mm-pi`** — the async Pi RPC client and the Pi-session ingester (parent plan §11). Pi is the **only**
   code editor.

**In scope:** ledger + calibration; adaptive meta-analysis + taxonomy; mistake→test; change sets; sandbox
build/test; benchmark + shadow; promote/reject with a written reason; rollback; budgets; Pi RPC control;
Pi session ingestion; the first gap→module→promote workflow.

**Out of scope (Phase 12):** hot-loading a promoted module into a running process; generating design docs
and phase plans; architectural evolution beyond policy/code change sets. **Never automatic:** identity
invariants and permissions.

---

## 2. Architecture

```
 TRIGGER (prediction error · correction · repeat failure · contradiction · near miss · novel success)
      │
      ▼
 mm-metaanalysis ── diagnosis (1 bounded LLM pass) ── error taxonomy ── lesson ──► library (P7)
      │                                                                             │
      ▼                                                                             ▼
 capability/data/policy gap ───────────────────────────────────────────────► ChangeSet (typed)
      │                                                                             │
      └──────────────────────────────► mm-selfeng ◄──────────────────────────────────┘
                                          │
        sandbox (git worktree) → build → unit → regression → adversarial → benchmark → shadow
                                          │
                                   PromotionGate (deterministic policy)
                                          │
                             ┌────────────┴────────────┐
                        promote                      reject(+written reason)
                             │                            │
                     update /code graph            evolution_journal
                     append evolution_journal
                             │
                        Pi (rpc) writes code only inside the sandbox; sessions ingested to /code
```

**Rule:** the promotion gate decides — never the LLM; **production is never written directly**; every
accepted change is immutable in the journal.

---

## 3. Deliverables and workspace layout

```
crates/mm-metaanalysis/src/{ledger,calibration,taxonomy,triggers,diagnosis,lessons}.rs
crates/mm-mistakes/src/{reproduce,emit}.rs
crates/mm-selfeng/src/{changeset,sandbox,benchmark,shadow,promotion,rollback,journal,budget}.rs
crates/mm-pi/src/{protocol,rpc,framing,session,ingest}.rs + fixtures/  (recorded sessions)
crates/mm-store-sqlite/migrations/0011_self_engineering.sql
ontology/mm.ttl (+ Prediction, MetaAnalysis, Lesson, ChangeSet, EvolutionEvent)
ontology/code.ttl (+ PiSession, Edit, ToolCall, generatedBy, parentSession)
ontology/shapes/phase11_{metaanalysis,changeset,pisession}.ttl
pi/system_prompt.md · pi/skills/mm-module-scaffold/SKILL.md · pi/prompts/module_scaffold.md
pi/extensions/metadata_emit.ts
modules/cognition/calibration/{plugin.toml,src/lib.rs,manual/module.md,tests/}
bench/{regression,calibration,promotion,pi,gaps}/
tests/e2e/{selfeng_cycle.rs, pi_replay.rs}
```

`mm-cli` gains: `meta analyze|queue|lessons`, `calibrate`, `changeset new|show`, `sandbox run`,
`promote|reject <ulid>`, `budget show`, `pi run|ingest`, `regression run`, `audit production-tree`.

---

## 4. Detailed specifications

Named graphs: `/provenance` (decisions, changes, promotions, Pi edits) and `/code` (module metadata).
ULID IRIs `<https://metamind.dev/data/{ulid}>`; design anchors `https://metamind.dev/design/digital_mind_design1.md#60`.

### 4.1 Core types

```rust
// mm-metaanalysis
pub struct Prediction { pub id: Ulid, pub proposition: String, pub probability: f32,
    pub horizon: Duration, pub conditions: Vec<String>, pub created_at: Timestamp,
    pub outcome: Option<PredictionOutcome> }                 // ledger is append-only; no update/delete
pub struct CalibrationReport { pub n: u32, pub brier: f64, pub log_loss: f64, pub ece: f64,
    pub bins: Vec<ReliabilityBin>, pub coverage: f64, pub selective_risk: f64 }
pub trait Calibrator { fn score(&self, labeled: &[Labeled]) -> CalibrationReport;
    fn threshold_for(&self, class: &str, target_precision: f64) -> f64; }
pub enum ErrorClass { Knowledge, Retrieval, Interpretation, Comparison, Assumption, Causal, Planning,
    Decision, Execution, Verification, Social, Resource, Policy, Code, Data, Model }
pub enum Trigger { PredictionError, UserCorrection, RepeatedFailure, Contradiction, UnexpectedOutcome,
    GoalFailure, NearMiss, NovelSuccess }
pub async fn analyze(episode: Ulid, ctx: &MetaContext) -> Result<MetaAnalysis>;

// mm-mistakes — always fail-before / pass-after
pub struct RegressionTest { pub id: Ulid, pub path: PathBuf, pub fails_before: Ulid, pub passes_after: Ulid }
pub async fn compile_mistake(mistake: Ulid, repo: &Path) -> Result<RegressionTest>;

// mm-selfeng
pub struct ChangeSet { pub id: Ulid, pub reason: String, pub hypothesis: String, pub code: Vec<Patch>,
    pub schema_migrations: Vec<MigrationId>, pub data_migrations: Vec<MigrationId>,
    pub memory_transforms: Vec<TransformId>, pub policies: Vec<PolicyDelta>, pub prompts: Vec<PromptDelta>,
    pub tests: Vec<TestId>, pub benchmarks: Vec<BenchmarkId>, pub rollback: RollbackPlan }
pub enum PromotionDecision { Promote, Reject { reason: String } }
pub trait PromotionGate { fn evaluate(&self, cs: &ChangeSet, ev: &EvidenceBundle) -> PromotionDecision; }
pub trait Sandbox { async fn prepare(&self, cs: &ChangeSet) -> Result<SandboxDir>;
    async fn build(&self, d: &SandboxDir) -> Result<BuildResult>;
    async fn test(&self, d: &SandboxDir) -> Result<TestResult>; }
pub enum BudgetKind { MetaAnalysis, Improvement, Evolution, Metacognitive }
pub trait BudgetLedger { async fn debit(&self, k: BudgetKind, amount: f64, trace: Ulid) -> Result<()>;
    async fn remaining(&self, k: BudgetKind) -> Result<f64>; }

// mm-pi — strict LF-only JSONL framing (split on '\n' only; strip a trailing '\r')
pub enum PiCommand { Prompt { id: String, message: String, streaming_behavior: Option<String> },
    Steer { id: String, message: String }, FollowUp { id: String, message: String },
    Abort { id: String }, ClearQueue { id: String }, NewSession { id: String } }
pub enum PiEvent { Response { id: Option<String>, command: String, success: bool },
    Agent { raw: serde_json::Value } }
pub struct PiClient;
impl PiClient {
    pub async fn spawn(cfg: &PiConfig) -> Result<Self>;          // pi --mode rpc --session-dir data/sandbox/pi
    pub async fn send(&mut self, cmd: PiCommand) -> Result<()>;
    pub async fn prompt(&mut self, id: &str, message: &str) -> Result<()>;
    pub async fn steer(&mut self, id: &str, message: &str) -> Result<()>;
    pub async fn abort(&mut self, id: &str) -> Result<()>;
    pub async fn new_session(&mut self, id: &str) -> Result<()>;
    pub fn events(&mut self) -> impl futures::Stream<Item = PiEvent> + '_;
}
pub async fn ingest_session(path: &Path) -> Result<PiSessionGraph>;   // version:3, parentId chain
```

### 4.2 Migration `0011_self_engineering.sql` (key tables; all ids `TEXT(26)`)

```sql
CREATE TABLE predictions (id TEXT(26) PRIMARY KEY, proposition TEXT NOT NULL,
  probability REAL NOT NULL CHECK (probability BETWEEN 0 AND 1), horizon_seconds INTEGER NOT NULL,
  conditions_json TEXT NOT NULL, episode_ulid TEXT(26), created_at TEXT NOT NULL);
CREATE TABLE prediction_outcomes (prediction_ulid TEXT(26) PRIMARY KEY REFERENCES predictions(id),
  observed INTEGER NOT NULL CHECK (observed IN (0,1)), resolved_at TEXT NOT NULL, evidence_ulid TEXT(26) NOT NULL);
CREATE TABLE calibration_runs (id TEXT(26) PRIMARY KEY, subject TEXT NOT NULL, n INTEGER NOT NULL,
  brier REAL NOT NULL, log_loss REAL NOT NULL, ece REAL NOT NULL, coverage REAL, selective_risk REAL,
  baseline_brier REAL, created_at TEXT NOT NULL);
CREATE TABLE meta_analyses (id TEXT(26) PRIMARY KEY, trigger TEXT NOT NULL, episode_ulid TEXT(26),
  diagnosis TEXT NOT NULL, error_classes_json TEXT NOT NULL, recurrence REAL NOT NULL, impact REAL NOT NULL,
  created_at TEXT NOT NULL);
CREATE TABLE change_sets (id TEXT(26) PRIMARY KEY, reason TEXT NOT NULL, hypothesis TEXT NOT NULL,
  payload_json TEXT NOT NULL, rollback_json TEXT NOT NULL, status TEXT NOT NULL CHECK (status IN
  ('draft','sandboxed','built','tested','benchmarked','shadowed','promoted','rejected')), created_at TEXT NOT NULL);
CREATE TABLE promotions (id TEXT(26) PRIMARY KEY, changeset_ulid TEXT(26) NOT NULL REFERENCES change_sets(id),
  decision TEXT NOT NULL CHECK (decision IN ('promote','reject')), reason TEXT NOT NULL,
  evidence_json TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE regression_tests (id TEXT(26) PRIMARY KEY, mistake_ulid TEXT(26), path TEXT NOT NULL UNIQUE,
  fails_before_ulid TEXT(26), passes_after_ulid TEXT(26), created_at TEXT NOT NULL);
CREATE TABLE evolution_journal (id TEXT(26) PRIMARY KEY, date TEXT NOT NULL, reason TEXT NOT NULL,
  evidence_json TEXT NOT NULL, hypothesis TEXT NOT NULL, changeset_ulid TEXT(26), outcome TEXT NOT NULL,
  decision TEXT NOT NULL, self_version TEXT NOT NULL);
CREATE TABLE budgets (id TEXT(26) PRIMARY KEY, kind TEXT NOT NULL CHECK (kind IN
  ('meta_analysis','improvement','evolution','metacognitive')), period TEXT NOT NULL,
  limit_amount REAL NOT NULL, spent_amount REAL NOT NULL DEFAULT 0, UNIQUE (kind, period));
CREATE TABLE pi_sessions (id TEXT(26) PRIMARY KEY, session_file TEXT NOT NULL, changeset_ulid TEXT(26),
  model TEXT, cwd TEXT, started_at TEXT NOT NULL, ended_at TEXT, event_count INTEGER NOT NULL DEFAULT 0);
CREATE TABLE pi_events (id TEXT(26) PRIMARY KEY, session_ulid TEXT(26) NOT NULL REFERENCES pi_sessions(id),
  seq INTEGER NOT NULL, kind TEXT NOT NULL, payload_json TEXT NOT NULL, UNIQUE (session_ulid, seq));
```

### 4.3 Ontology + SHACL

```turtle
mm:Prediction a owl:Class ; rdfs:subClassOf mm:Claim .
mm:MetaAnalysis  a owl:Class .
mm:Lesson        a owl:Class .
mm:ChangeSet     a owl:Class .
mm:EvolutionEvent a owl:Class .
mm:hasOutcome    a owl:ObjectProperty ; rdfs:domain mm:Prediction .
mmc:PiSession    a owl:Class .  mmc:Edit a owl:Class .  mmc:ToolCall a owl:Class .
mmc:generatedBy  a owl:ObjectProperty .  mmc:parentSession a owl:ObjectProperty .
mmc:sessionFile  a owl:DatatypeProperty .
```

Shapes (via `rdf-shacl::generate_shacl`, hand-written where the fragment does not fit): a `mm:Prediction`
has exactly one `mm:probability ∈ [0,1]`; a `mm:ChangeSet` requires `mm:reason`, `mm:hypothesis`,
`mm:rollbackPlan`; a `mmc:PiSession` requires `mmc:sessionFile` and gapless `mmc:seq`.

### 4.4 Promotion-gate policy (deterministic, ordered)

Reject if any holds, with a specific reason recorded: **(1)** any regression test fails; **(2)** the
candidate is worse than baseline on the frozen bench beyond `noise_margin`; **(3)** risk exceeds the
evolution risk budget; **(4)** a hard prohibition is hit (Phase 9 firewall); **(5)** required evidence is
missing (no benchmark or no rollback); **(6)** the change touches identity invariants or permissions.
Otherwise promote.

### 4.5 External references copied in and integrated

Per parent plan §7, external code is **reference only**: the needed source is copied into `vendor/<origin>/`
as workspace members and integrated behind `mm-*` traits, with `mmc:copiedFrom` (repo, revision, license)
emitted by the Phase 2 scan and a `vendor/<origin>/COPYING.md`.

| Need | Reference | Copy / integration |
|---|---|---|
| Diagnosis + lesson extraction | `kg-llm` (`LlmClient`, `CachedLlmClient`, schemas) | copied to `vendor/rust_extract/`; `mm-metaanalysis::diagnosis` calls it via `mm-llm` |
| Root-cause / intervention reasoning | `causal-ir` (`CausalEngine`, `Counterfactual`) | copied to `vendor/rust_symbolic/`; `mm-selfeng` localizes the cause substrate |
| Policy/gene fitness, mutation | `learning-ir`, `evolution-ir` | copied; `mm-selfeng` policy deltas |
| Metadata / graph validation | `rdf-shacl`, `rdf-codec` | copied; validates `ChangeSet`/`PiSession` graphs |
| Change-set artifact + replay | `rust_extract` versioned JSONL artifact pattern | copied as a pattern into `mm-selfeng::changeset` |
| Module load (Phase 12) | nexus plugin lifecycle (`nexus-core`) | copied in Phase 12; Phase 11 only registers metadata |

**Pi is an external process, not copied source.** `mm-pi` invokes the installed `pi` binary over its
documented RPC protocol (parent plan §11); it is recorded as a versioned external tool dependency in the
evolution journal/config, never vendored. The four budgets are enforced in `budget.rs` as
`limit_amount - spent_amount >= 0`; a debit that would go negative is denied with `budget.deny` and
leaves the ledger unchanged.

---

## 5. Build sequence

1. **Ledger + calibration.** `ledger` (append-only) and `Calibrator` (Brier, log-loss, ECE, reliability,
   coverage/selective risk, `threshold_for`). Require consequential predictions to write a row.
   *Check:* golden vectors vs `bench/calibration/labeled.jsonl`.
2. **Event-triggered meta-analysis.** `triggers` watches the event log for the eight triggers and computes
   `MetaAnalysisPriority = Novelty + Failure + Impact + Recurrence + Uncertainty`. `diagnosis` runs one
   bounded LLM pass over the episode context and classifies `ErrorClass`. *Check:* a seeded failure yields
   one `meta_analyses` row with ≥1 class.
3. **Mistake→test compiler.** `reproduce` minimizes an incident; `emit` writes a test + fixture into
   `bench/regression/`, records `regression_tests`, asserts fail-before/pass-after. *Check:* seeded bug
   reproduces, fails before, passes after.
4. **ChangeSet + sandbox.** `changeset` assembles the typed set; `sandbox` creates a git worktree at
   `data/sandbox/<ulid>/`, runs `cargo build`/`cargo test`. *Check:* filesystem audit shows writes only
   under `data/sandbox/`.
5. **Promotion pipeline + gate.** `benchmark`, `shadow`, `promotion` (§4.4 policy), `rollback`. Promotion
   updates the Phase 2 `/code` graph and appends `evolution_journal`. *Check:* negative fixtures reject
   with a written reason.
6. **`mm-pi`.** `framing` (LF-only; strip `\r`), `protocol`, `rpc` (spawn `pi --mode rpc --provider <p>
   --model <m> --session-dir data/sandbox/pi`, restricted `--tools`, id-correlated). `session`/`ingest`
   parse Pi JSONL `version:3` (`parentId` chain) into `pi_sessions`/`pi_events` + `/code` RDF.
   *Check:* recorded-session replay deterministic; ingestion round-trips to identical canonical RDF.
7. **First generated-module workflow.** Wire gap→`ChangeSet`→Pi scaffolds from `pi/skills/mm-module-scaffold`
   →tests→benchmark→gate; `pi/extensions/metadata_emit.ts` writes `metadata.ttl`. *Check:* end-to-end
   produces a registered module or a rejection with reason, no human edits.
8. **Budgets.** `budget` enforces the four budgets with deterministic arithmetic; exhaust ⇒ hard deny; each
   debit is an audit record. *Check:* overspend denied and logged.

---

## 6. Logging and observability

All records go through `mm-log` with the global fields. Phase-11 event codes and required fields:

- `meta.trigger` — trigger, episode_id, priority_components{novelty,failure,impact,recurrence,uncertainty}.
- `meta.diagnose` — analysis_id, error_classes[], model, prompt_hash, latency_ms, cost.
- `meta.lesson` — lesson_id, confidence, evidence_count, domains[], policy_effect.
- `calib.report` — subject, n, brier, log_loss, ece, coverage, baseline_brier.
- `mistake.regress` — mistake_id, test_path, fails_before, passes_after.
- `changeset.create` / `changeset.stage` — change_set_id, status, reason, hypothesis.
- `sandbox.prepare` / `sandbox.build` / `sandbox.test` — change_set_id, dir, exit_code, warnings.
- `bench.run` / `shadow.start` — change_set_id, baseline, candidate, delta, n.
- `gate.decide` — change_set_id, decision, reason, evidence{}, risk, budget_kind.
- `promote.commit` / `promote.reject` — change_set_id, module_uri, self_version, reason.
- `rollback.apply` — change_set_id, restored_state_hash.
- `budget.debit` / `budget.deny` — kind, amount, remaining, period.
- `pi.spawn` / `pi.command` / `pi.event` — session_id, command, id, success, seq.
- `pi.session.ingest` / `pi.edit` — session_id, session_file, event_count, file, tool.
- `copy.integrate` — origin, revision, license, `mmc:copiedFrom` IRI.

Every state-changing record is also an append-only audit record; `mm-cli logs verify` must pass (schema,
audit completeness, gapless sequence, ULID correlation, redaction, replay equivalence).

---

## 7. Testing

| Area | Test | Assertion |
|---|---|---|
| Pi protocol | replay fixtures | LF-only framing; a `\r\n` case and a payload containing `U+2028`/`U+2029` must NOT split; id correlation for prompt/steer/follow_up/abort/new_session; `success:false` handled |
| Pi ingestion | session with `parentId` chain | canonical RDF equals golden; `pi_events.seq` gapless; a truncated session is a hard error |
| Sandbox | worktree build/test | filesystem audit: zero writes outside `data/sandbox/`; production write ⇒ rejected |
| ChangeSet | schema validation | missing rollback ⇒ rejected; JSON round-trip stable |
| Promotion gate | negative fixtures | each of the six reject rules yields its specific reason; a clean candidate promotes |
| Calibration | labeled set | Brier/log-loss/ECE match reference within 1e-9; `threshold_for` hits the target precision |
| Mistake→test | seeded bug | regression test fails before, passes after, and is retained/discovered by `regression run` |
| Budgets | proptest | sum of debits never exceeds `limit_amount`; denial leaves ledger unchanged |
| Determinism | full cycle replay | sandbox→bench→gate replays identically from `(code, event log, config)` with zero provider calls |

Fixtures: `bench/calibration/labeled.jsonl`, `bench/episodes/seeded_failure_01.json`,
`bench/gaps/gap_01.json`, `bench/pi/module_scaffold_01.json`, `bench/regression/seeded_bug_01`,
`crates/mm-pi/fixtures/recorded_session_*.jsonl`.

---

## 8. Pass gate

Run as plain commands; every command must exit 0 (an earlier gate failing is a Phase 11 failure):

```bash
cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace

mm-cli calibrate --bench bench/calibration/labeled.jsonl \
  --assert-brier-le 0.20 --assert-ece-le 0.10 --assert-improves-baseline
mm-cli meta analyze --episode bench/episodes/seeded_failure_01.json --assert-lessons-ge 1
mm-cli regression run --suite bench/regression --assert-fail-before-pass-after seeded_bug_01

mm-cli changeset new --from-gap bench/gaps/gap_01.json          # sets $CHANGESET
mm-cli pi run --task bench/pi/module_scaffold_01.json --assert-session-ingested
mm-cli sandbox run "$CHANGESET" --assert-isolated
mm-cli promote "$CHANGESET" --assert-reason-present
mm-cli codex verify

mm-cli audit production-tree --assert-unmodified-outside-promotion
mm-cli budget show --assert-no-overspend
mm-cli logs verify
```

Objective criteria:

- The seeded gap yields a `ChangeSet`; Pi generates the module; the sandbox builds and tests it; a
  benchmark compares it to a baseline; the gate emits `promote` or `reject` **with a written reason** —
  with no human code edits.
- The seeded bug's regression test in `bench/regression/` fails before and passes after the fix, retained.
- The production tree is unmodified except by a promotion (filesystem audit + event log agree).
- A promoted module is registered in `/code` with code+data+schema+policy+prompt versions; `codex verify`
  stays green.
- Calibration improves over the recorded baseline across one labeled cycle.
- No budget is exceeded; every meta-analysis, change, and promotion is journaled immutably.
- `mm-cli logs verify` passes.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Pi output is nondeterministic | Sandbox-only execution; deterministic build/test/benchmark gate; full session ingestion; a candidate is judged by evidence, never by the LLM |
| Reward hacking / self-confirming evaluation | Frozen harness within a cycle; held-out bench; adversarial + regression suites; gate changes are themselves change-sets |
| Unbounded recursive self-modification | Evolution budget hard cap; identity invariants/permissions never auto-evolved; every mutation is a change-set with rollback |
| RPC framing bugs corrupt streams | LF-only splitter with `\r` stripping; `U+2028`/`U+2029` fixtures; id correlation; recorded-session replay |
| Session format drift (Pi upgrade) | `version:3` header checked; unknown record kinds are hard errors; re-ingest is a change-set |
| Self-modification bypasses the gate | Production is never written by Pi; promotion is the only writer; `audit production-tree` gate |
| Calibration over-trust (confidence ≠ truth) | Probabilities scored against outcomes; thresholds learned empirically; confidence carries no authority |
| Budget runaway / runaway introspection | Deterministic `BudgetLedger`; hard deny; event-triggered analysis (never scheduled) |
| Sandbox escape | Capability-bounded sandbox (Phase 10 primitives) + filesystem-audit test |

---

## 10. References (code-available)

- Darwin Gödel Machine — https://github.com/jennyzzt/dgm
- Gödel Agent — https://github.com/Arvid-pku/Godel_Agent
- STOP (Self-Taught Optimizer) — https://github.com/microsoft/stop
- SWE-agent — https://github.com/swe-agent/swe-agent · mini-swe-agent — https://github.com/swe-agent/mini-swe-agent
- SWE-bench — https://github.com/SWE-bench/SWE-bench
- OpenHands — https://github.com/All-Hands-AI/OpenHands
- Aider — https://github.com/Aider-AI/aider
- AutoCodeRover — https://github.com/nus-apr/auto-code-rover
- Reflexion — https://github.com/noahshinn/reflexion
