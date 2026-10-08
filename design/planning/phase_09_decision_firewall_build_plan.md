# Phase 9 Build Plan — Sanity firewall, decision core (Jev-style), and comparison integrity

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 9 · Codename Metamind (`mm`)
> Supports the single architecture invariant: LLM proposes, the control plane structures, **Jev decides
> bounded questions**, symbolic proves, deterministic enforces, the environment observes, memory
> preserves, meta-analysis improves.

This phase adds the two organs between *reasoning* and *action*: the **bounded-decision primitive**
(`DecisionCore`, the Jev-style interface) and the **unified sanity / epistemic firewall** (§46). It also
adds **comparison integrity** (§20) as its own subsystem, because most "wrong" decisions are really
invalid comparisons.

---

## 1. Objective and scope

**In scope**
- `mm-decision`: a `DecisionCore` trait answering typed, bounded questions (`choice` / `score` / `yes-no`)
  over shared state, with **three interchangeable implementations** — deterministic rules, a local model
  head, and a hosted decision model — that all pass **one identical conformance suite**.
- Deterministic **risk measures** built on `decision-ir` (expected loss, maximum loss, tail probability,
  ruin probability, variance, downside asymmetry, reversibility, optionality).
- **Comparison integrity**: a comparison contract, validity checks, and deterministic normalization.
- `mm-firewall`: a unified scan over Phase 6–8 + Phase 10 inputs that emits one of
  `PROCEED | PROCEED_WITH_CAUTION | VERIFY_FIRST | ASK_USER | REPLAN | HUMAN_REVIEW | REJECT`, with hard
  deterministic prohibitions that short-circuit to `REJECT` regardless of any model output.
- Full feature-and-outcome logging of every decision and firewall run for Phase 11 calibration.

**Out of scope**
- Distilling frequent decisions into smaller local models (Phase 11) — this phase only defines the local
  adapter and the decision traces it consumes.
- Prediction-ledger calibration metrics (Phase 11); this phase only *records* the features/outcomes.
- Tool execution and rollback (Phase 10).
- New cognitive techniques or library entries (Phase 7).

---

## 2. Prerequisites and dependencies

| Requires | From | Why |
|---|---|---|
| `mm-core` (`Id`, `Ulid`, `Timestamp`, `MmError`, async traits) | Phase 1 | IDs, IRIs, errors |
| `mm-log` + `mm-cli logs verify` | Phase 1 / §10 | structured logging, audit |
| `mm-store-sqlite` (sqlx, WAL, migrations) | Phase 1 | decisions / comparisons / firewall_runs |
| `mm-store-graph` (Oxigraph actor) + `rdf-codec` + `rdf-shacl` | Phase 1 | ontology + provenance mirror |
| `mm-eventlog` | Phase 1 | audit records for decisions and firewall runs |
| `mm-epistemic` (assumptions, contradictions, dependency DAG) | Phase 6 | firewall inputs |
| `mm-library` (precedent / cases / techniques) + `analogy-ir` | Phase 7 | precedent + simpler-alternative checks |
| `mm-metacog` (episode, budget, program) | Phase 8 | firewall runs inside an episode; budget for bounded calls |
| `mm-llm` (`LlmClient`, structured output, replay) | Phase 3 | hosted core + optional local head |

**Reference components to copy in** (see §6): `decision-ir`, `ensemble-ir`, `causal-ir`,
`logic-ir`/`solver-ir`/`backend-registry`/`logic-planner`.

---

## 3. Deliverables (exact paths)

```
~/Build/metamind/
├── crates/mm-decision/
│   ├── src/lib.rs                 # public surface + re-exports
│   ├── src/core.rs                # DecisionCore trait, CoreId, DecisionState
│   ├── src/question.rs            # DecisionQuestion, DecisionAnswer, AnswerValue, ScoreScale
│   ├── src/rules.rs               # DeterministicRulesCore
│   ├── src/local.rs               # LocalModelCore adapter
│   ├── src/hosted.rs              # HostedDecisionCore (mm-llm)
│   ├── src/risk.rs                # RiskProfile + analyze_risk (decision-ir)
│   ├── src/compare.rs             # ComparisonContract, check_contract, normalize
│   ├── src/calibration.rs         # features + outcome schema, calibration batch keys
│   ├── src/rdf.rs                 # ToRdf/FromRdf for decision/comparison/risk
│   ├── rules/decision_rules.toml  # table-driven deterministic rules
│   └── tests/conformance.rs       # ONE suite, run against all three cores
├── crates/mm-firewall/
│   ├── src/lib.rs
│   ├── src/scan.rs                # aggregate assumption/contradiction/constraint/step/risk/authority
│   ├── src/outcome.rs             # FirewallOutcome + ReasonCode
│   ├── src/prohibitions.rs        # hard prohibitions (deterministic, short-circuit)
│   └── src/report.rs              # FirewallReport
├── crates/mm-cli/src/cmd/{decide.rs,firewall.rs,compare.rs,risk.rs,conformance.rs}
├── crates/mm-store-sqlite/migrations/0009_decision_firewall.sql
├── ontology/mm.ttl                # + mm:Decision, mm:Comparison, mm:Risk
├── ontology/shapes/decision.ttl   # SHACL shapes
├── modules/cognition/comparison-integrity/{plugin.toml,src/lib.rs,tests/,manual/module.md}
├── bench/
│   ├── risk/reference_values.json
│   ├── comparison/fixtures.jsonl
│   ├── adversarial/firewall_cases.jsonl
│   └── firewall/gold.jsonl
└── tests/e2e/phase09_firewall.rs
```

---

## 4. Data model and ontology deltas

### 4.1 SQLite — `0009_decision_firewall.sql`

```sql
CREATE TABLE decisions (
  id            TEXT(26) PRIMARY KEY,          -- ULID
  question_kind TEXT NOT NULL,                 -- choice | score | yes_no
  question_json TEXT NOT NULL,
  answer_json   TEXT NOT NULL,
  confidence    REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
  features_json TEXT NOT NULL,                 -- FeatureVector for calibration
  core_impl     TEXT NOT NULL,                 -- rules | local | hosted
  latency_ms    INTEGER NOT NULL,
  cost          REAL NOT NULL DEFAULT 0.0,
  created_ulid  TEXT(26) NOT NULL,
  episode_ulid  TEXT(26),
  outcome_json  TEXT,                          -- filled later (Phase 11 calibration)
  outcome_ulid  TEXT(26)
);
CREATE INDEX decisions_episode ON decisions(episode_ulid);
CREATE INDEX decisions_core ON decisions(core_impl, question_kind);

CREATE TABLE comparisons (
  id            TEXT(26) PRIMARY KEY,
  objective     TEXT NOT NULL,
  contract_json TEXT NOT NULL,
  verdict       TEXT NOT NULL,                 -- valid | non_comparable | rejected
  violations_json TEXT NOT NULL,
  normalized_json TEXT,
  created_ulid  TEXT(26) NOT NULL
);

CREATE TABLE firewall_runs (
  id                TEXT(26) PRIMARY KEY,
  episode_ulid      TEXT(26) NOT NULL,
  inputs_json       TEXT NOT NULL,
  outcome           TEXT NOT NULL,             -- PROCEED .. REJECT
  reason_codes_json TEXT NOT NULL,
  decision_ulids    TEXT NOT NULL,             -- JSON array of decision ULIDs consulted
  hard_prohibition  TEXT,                      -- non-null iff short-circuited
  created_ulid      TEXT(26) NOT NULL
);
CREATE INDEX firewall_runs_episode ON firewall_runs(episode_ulid);
```

Conventions: ULID `TEXT(26)` primary keys, `created_ulid` correlation, WAL + `foreign_keys=ON` (Phase 1).

### 4.2 RDF / OWL (`ontology/mm.ttl`, `mm:` = `https://metamind.dev/ontology#`)

Add classes `mm:Decision`, `mm:Comparison`, `mm:Risk`, `mm:RiskMeasure`, `mm:FirewallRun`,
`mm:ReasonCode`; predicates `mm:hasQuestion`, `mm:questionKind`, `mm:selectedOption`, `mm:hasConfidence`,
`mm:decidedBy`, `mm:hasFeature`, `mm:hasOutcome`, `mm:hasRisk`, `mm:usesMeasure`, `mm:measuredValue`,
`mm:comparedTo`, `mm:comparisonValid`, `mm:hasViolation`, `mm:firewallOutcome`, `mm:hasReasonCode`,
`mm:consultedDecision`. Instance IRIs are `<https://metamind.dev/data/{ulid}>`; the decision/comparison
records live in the named graph `https://metamind.dev/graph/decision` and audit links mirror into
`https://metamind.dev/graph/provenance`.

### 4.3 SHACL (`ontology/shapes/decision.ttl`)

- Every `mm:Decision` has exactly one `mm:questionKind`, one `mm:decidedBy`, one `mm:hasConfidence` in
  `[0,1]`, and (unless unresolved) one `mm:hasOutcome`.
- Every `mm:FirewallRun` has exactly one `mm:firewallOutcome` and ≥1 `mm:hasReasonCode`.
- A `mm:FirewallRun` whose outcome is `REJECT` due to a prohibition MUST carry `mm:hardProhibition`.
- Generated with `rdf-shacl` (`check_ontology` → `generate_shacl` → `validate`) where the fragment fits;
  unsupported constructs are hard errors.

---

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

### 5.1 `DecisionCore`

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoreId { Rules, Local, Hosted }

pub struct DecisionState { pub episode: Ulid, pub facts: Vec<ClaimRef>, pub assumptions: Vec<AssumptionRef>,
                           pub constraints: Vec<ConstraintRef>, pub context: serde_json::Value }

#[derive(Clone, Debug)]
pub enum DecisionQuestion {
    Choice { id: Ulid, prompt: String, options: Vec<OptionId>, allow_none: bool },
    Score  { id: Ulid, prompt: String, scale: ScoreScale }, // e.g. 0..1 with anchors
    YesNo  { id: Ulid, prompt: String },
}

#[derive(Clone, Debug)]
pub enum AnswerValue { Option(OptionId), Score(f32), Bool(bool) }

#[derive(Clone, Debug)]
pub struct DecisionAnswer {
    pub question: Ulid,
    pub answer: AnswerValue,
    pub confidence: f32,            // MUST be in [0,1]
    pub features: FeatureVector,    // logged for calibration
    pub core: CoreId,
    pub latency_ms: u32,
}

#[async_trait::async_trait]
pub trait DecisionCore: Send + Sync {
    fn id(&self) -> CoreId;
    /// Deterministic: identical (question, state) => identical (answer, confidence, features).
    async fn answer(&self, q: &DecisionQuestion, s: &DecisionState) -> Result<DecisionAnswer, DecisionError>;
}
```

Implementations: `rules::DeterministicRulesCore` (table-driven, deterministic tie-break),
`local::LocalModelCore` (loads a distilled head if present, else reports `Unavailable`),
`hosted::HostedDecisionCore` (typed structured output through `mm-llm`, no prose).

### 5.2 Risk

```rust
pub struct RiskProfile {
    pub expected_loss: f64, pub max_loss: f64, pub tail_probability: f64, pub ruin_probability: f64,
    pub variance: f64, pub downside_asymmetry: f64, pub reversibility: f32, pub optionality: f32,
}
pub fn analyze_risk(outcomes: &[decision_ir::Outcome], measure: decision_ir::RiskMeasure) -> RiskProfile;
```

### 5.3 Comparison integrity

```rust
pub struct ComparisonContract {
    pub objective: String, pub objects: Vec<ObjRef>, pub dimensions: Vec<Dimension>,
    pub units: Vec<Unit>, pub timeframe: Timeframe, pub conditions: Vec<Condition>,
    pub constraints: Vec<ConstraintRef>, pub evidence: Vec<Ulid>,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ComparisonVerdict { Valid, NonComparable, Rejected }
pub struct ComparisonCheck { pub verdict: ComparisonVerdict, pub violations: Vec<ComparisonViolation>,
                             pub normalized: Option<NormalizedComparison> }
pub fn check_contract(a: &ComparisonContract, b: &ComparisonContract) -> ComparisonCheck;
pub fn normalize(a: &ComparisonContract, b: &ComparisonContract) -> Option<NormalizedComparison>;
```

### 5.4 Firewall

```rust
pub enum FirewallOutcome { Proceed, ProceedWithCaution, VerifyFirst, AskUser, Replan, HumanReview, Reject }

pub struct FirewallInput {
    pub episode: Ulid, pub assumptions: Vec<AssumptionRef>, pub contradictions: Vec<Ulid>,
    pub comparisons: Vec<Ulid>, pub constraints: Vec<ConstraintRef>, pub missing_steps: Vec<MissingStep>,
    pub risks: RiskProfile, pub reversibility: Reversibility, pub authorization: AuthorizationRef,
    pub precedent: Option<Ulid>, pub simpler_alternative: Option<Ulid>, pub stakes: Stakes,
}
pub struct FirewallReport { pub id: Ulid, pub outcome: FirewallOutcome, pub reason_codes: Vec<ReasonCode>,
                            pub decisions: Vec<Ulid>, pub hard_prohibition: Option<HardProhibition> }

#[async_trait::async_trait]
pub trait SanityFirewall {
    async fn evaluate(&self, input: &FirewallInput, ctx: &mut FirewallCtx) -> Result<FirewallReport, FirewallError>;
}
```

Hard prohibitions are checked **before** any `DecisionCore` call and short-circuit to `Reject`.

### 5.5 CLI

```
mm-cli decide --question <file|json> [--core rules|local|hosted] [--episode <ulid>]
mm-cli firewall eval --episode <ulid> [--input <file>] [--json]
mm-cli compare check --fixtures <file|-> | --a <file> --b <file>
mm-cli risk analyze --outcomes <file> [--measure var|cvar|ruin|max_loss] [--tolerance <f64>]
mm-cli conformance --core rules --core local --core hosted
```

### 5.6 Module `plugin.toml` (`modules/cognition/comparison-integrity/`)

```toml
[plugin]
name = "mm-comparison-integrity"
uri = "https://metamind.dev/code/module/cognition/comparison-integrity"
version = "0.1.0"
[metadata]
category = "cognition"
owned_by_phase = 9
capability = "mm:ComparisonIntegrity"
[tbox.functions]
"cognition.compare_contract" = { source = "handlers::compare_contract" }
[monad.operations]
name = "cognition"
arity = 1
[build]
rust_edition = "2021"
```

---

## 6. External references copied in and integration

Copy only what is needed into `vendor/<origin>/…` as workspace members; record origin repo, revision,
and license in `vendor/<origin>/COPYING.md`, and emit `mmc:copiedFrom` during the Phase 2 scan. No git
submodules, no external path dependencies, no `[patch]`.

| Reference (origin) | Copied-in crate | Integration |
|---|---|---|
| `decision-ir` (`rust_symbolic`) | `vendor/rust_symbolic/decision-ir` | `mm-decision` uses `DecisionEngine`, `RiskMeasure`, `Outcome`, `BayesianProblem`, `Mdp`, `MultiObjectiveProblem` for risk and bounded selection; wrapped behind `mm-decision::risk` |
| `ensemble-ir` (`rust_symbolic`) | `vendor/rust_symbolic/ensemble-ir` | multi-lens disagreement for firewall evidence (`DisagreementEdge`, BMA) |
| `causal-ir` (`rust_symbolic`) | `vendor/rust_symbolic/causal-ir` | risk structure + counterfactual downside in `FirewallInput` |
| `logic-ir` + `solver-ir` + `backend-registry` + `logic-planner` (`rust_symbolic`) | `vendor/rust_symbolic/{logic-ir,solver-ir,backend-registry,logic-planner}` | hard-constraint feasibility via SAT/SMT; proof obligations; sandboxed calls |
| `rdf-codec` + `rdf-shacl` (`rust_symbolic`) | `vendor/rust_symbolic/{rdf-codec,rdf-shacl}` | canonical RDF encode/decode; SHACL generation + validation |
| `kg-llm` (`rust_extract`) | `vendor/rust_extract/kg-llm` | structured-output client behind `mm-llm` used by `HostedDecisionCore` |

Adaptation rule: `mm-decision` never exposes a `decision-ir` type in its public API; it maps to
`RiskProfile`, `ComparisonCheck`, and `DecisionAnswer`.

---

## 7. Step-by-step implementation tasks

1. **Copy-in** the reference crates of §6; add `COPYING.md` per origin; add them to `[workspace] members`.
2. **`mm-decision` skeleton**: `core.rs`, `question.rs`; implement `DecisionState`, `DecisionQuestion`,
   `DecisionAnswer`, `FeatureVector`; enforce confidence bounded by construction; `#![forbid(unsafe_code)]`.
3. **Conformance suite** (`tests/conformance.rs`): canned questions/states; asserts typing, determinism
   (same input ⇒ identical answer+confidence+features), confidence ∈ [0,1], and stable tie-breaks.
4. **`DeterministicRulesCore`**: load `rules/decision_rules.toml`; evaluate conditions over
   `DecisionState`; deterministic ordered tie-break (rule id); `Unavailable` if no rule matches.
5. **`LocalModelCore`**: adapter that loads a distilled decision head from a model path if present; else
   returns `DecisionError::Unavailable`. No model training here (Phase 11).
6. **`HostedDecisionCore`**: send a typed prompt through `mm-llm` with `StructuredOut<DecisionAnswer>`;
   reject schema violations (no coercion); record tokens/cost/latency.
7. **Risk** (`risk.rs`): `analyze_risk` over `decision_ir::Outcome`; compute all eight measures;
   `max_loss`/`tail_probability`/`ruin_probability` from the loss distribution; reversibility/optionality
   from `Reversibility` input.
8. **Comparison integrity** (`compare.rs`): contract types; `check_contract` verifying same class,
   purpose, units, timeframe, conditions, definitions, scope; `normalize` performing deterministic unit
   conversion and timeframe alignment; never silently compare unlike objects.
9. **`mm-firewall`**: `scan.rs` gathers Phase 6 (assumptions, contradictions, missing steps),
   Phase 7 (precedent, simpler alternative), Phase 8 (budget/stopping), Phase 10 (authorization,
   reversibility); `prohibitions.rs` holds the hard table (`unauthorized_tool`, `identity_invariant`,
   `irreversible_without_approval`, `unresolved_hard_contradiction`, `spend_over_budget`,
   `schema_invalid_action`); `outcome.rs` maps signals → outcome with reason codes.
10. **Aggregation policy**: deterministic precedence — any hard prohibition ⇒ `Reject`; critical
    unverified assumption or catastrophic downside ⇒ `HumanReview`/`VerifyFirst`; invalid comparison ⇒
    `Replan`; ambiguity requiring user intent ⇒ `AskUser`; bounded residual risk ⇒ `ProceedWithCaution`;
    otherwise `Proceed`.
11. **Persistence**: write `decisions`, `comparisons`, `firewall_runs`; mirror RDF to `/decision` and
    provenance audit records; attach `outcome` when later supplied.
12. **Calibration schema** (`calibration.rs`): canonical `FeatureVector` keys per decision class so
    Phase 11 can compute Brier/log-loss/ECE; expose `mm-cli decide` logging by default.
13. **CLI**: implement the five subcommands of §5.5.
14. **Fixtures**: author `bench/risk/reference_values.json`, `bench/comparison/fixtures.jsonl`,
    `bench/adversarial/firewall_cases.jsonl`, `bench/firewall/gold.jsonl`.
15. **E2E** (`tests/e2e/phase09_firewall.rs`): episode → firewall run → recorded decision → RDF mirror.

---

## 8. Detailed logging requirements

All via `mm-log` (structured records, ULID `trace_id` correlation, five sinks, redaction). Event codes and
required fields for this phase:

| Event code | Required fields |
|---|---|
| `decision.core.answer` | `question_id`, `question_kind`, `core_impl`, `answer`, `confidence`, `latency_ms`, `feature_keys` |
| `decision.core.unavailable` | `core_impl`, `reason` |
| `decision.rules.match` | `rule_id`, `matched` (bool), `state_hash` |
| `decision.hosted.call` | `model`, `prompt_hash`, `tokens_in`, `tokens_out`, `cost`, `schema_ok` |
| `decision.log.write` | `decision_id`, `episode_id`, `outcome_present` (bool) |
| `risk.analyze` | `measure`, `expected_loss`, `max_loss`, `tail_probability`, `ruin_probability`, `variance`, `downside_asymmetry`, `reversibility`, `optionality` |
| `compare.contract` | `objective`, `objects`, `dimensions`, `units`, `timeframe` |
| `compare.check` | `verdict`, `violation_codes` |
| `compare.normalize` | `applied_conversions`, `timeframe_shift` |
| `firewall.scan.start` | `episode_id`, `input_digest` |
| `firewall.prohibition.shortcircuit` | `prohibition`, `outcome=REJECT`, `evidence` |
| `firewall.aggregate` | `signals`, `outcome`, `reason_codes`, `consulted_decision_ids` |
| `firewall.report` | `firewall_run_id`, `outcome`, `hard_prohibition` |

Audit records (append-only, transactional with the mutation): each `decisions`, `comparisons`, and
`firewall_runs` insert; every RDF mirror; every hard-prohibition short-circuit. Errors carry full cause
chains; prompts/responses are hashed, never logged verbatim (redaction).

---

## 9. Testing plan

- **Conformance (property + unit):** `tests/conformance.rs` runs the identical suite against all three
  cores — typing, determinism, confidence bounds, tie-break stability.
- **Risk vs reference:** `bench/risk/reference_values.json` compares each measure to hand-computed values
  within tolerance (VaR/CVaR/ruin).
- **Comparison fixtures:** `bench/comparison/fixtures.jsonl` contains matched pairs and deliberate
  mismatches (units, timeframe, conditions, scope, class) with expected verdicts.
- **Firewall adversarial:** `bench/adversarial/firewall_cases.jsonl` (spend over budget, unauthorized
  tool, irreversible action, unresolved contradiction, novel-without-justification, simpler alternative
  available, overengineering, unsupported claim) with labels in `bench/firewall/gold.jsonl`; measure
  precision/recall of the safety outcomes and require every hard-prohibition case to be `REJECT`.
- **Calibration completeness:** assert 100% of `decisions` rows carry non-empty `features_json`, and
  100% of `firewall_runs` carry `reason_codes_json`.
- **Golden snapshots:** `insta` snapshots of aggregation outcomes for fixed inputs.
- **E2E:** the CLI commands of §5.5 exercised as subprocesses.

---

## 10. Pass gate

Run as plain commands; all must pass, and earlier phases' gates must still pass.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 1) One conformance suite passes for all three impls
cargo test -p mm-decision --test conformance
cargo run -p mm-cli -- conformance --core rules --core local --core hosted

# 2) Firewall meets thresholds; every hard prohibition => REJECT
cargo run -p mm-cli -- firewall eval --corpus bench/adversarial/firewall_cases.jsonl \
    --report bench/firewall/report.json
# PASS: precision >= 0.95 and recall >= 0.90 on unsafe outcomes; 100% of hard-prohibition cases REJECT.

# 3) Risk measures match reference values
cargo run -p mm-cli -- risk analyze --outcomes bench/risk/reference_values.json --tolerance 1e-6
# PASS: exit 0.

# 4) Comparisons: matched pairs Valid; mismatches NonComparable|Rejected
cargo run -p mm-cli -- compare check --fixtures bench/comparison/fixtures.jsonl
# PASS: every fixture matches its expected verdict.

# 5) 100% of decisions and firewall runs logged with features + outcome
sqlite3 data/metamind.db \
 "SELECT (SELECT COUNT(*) FROM decisions WHERE features_json IS NULL)
       + (SELECT COUNT(*) FROM firewall_runs WHERE reason_codes_json IS NULL);"
# PASS: prints 0.

# 6) Logging verification
cargo run -p mm-cli -- logs verify
# PASS: schema-valid records, gapless audit sequence, ULID correlation, redaction clean,
#       replay identical with logging on/off.
```

Objective gate conditions: (a) all three cores conform; (b) firewall precision/recall thresholds met and
hard prohibitions unconditionally `REJECT`; (c) risk measures match within tolerance; (d) comparison
fixtures all correct; (e) logging completeness `0`; (f) `mm-cli logs verify` passes.

---

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| `DecisionCore` drift between implementations | One shared conformance suite; any impl failing it cannot be selected |
| Hosted decision model unavailable | `HostedDecisionCore` returns `Unavailable`; aggregation falls back to rules + escalate (`HumanReview`) per policy — never fabricate a decision |
| Rules overfit the fixtures | Rules kept coarse and audited; conformance + adversarial sets separate; thresholds calibrated in Phase 11 |
| Firewall over-blocking (false `REJECT`) | Precedence policy is explicit; near-miss/over-block cases added to the adversarial corpus; recall measured, not assumed |
| Invalid comparison silently accepted | `check_contract` is mandatory before any compare; mismatches must be `NonComparable`/`Rejected`, never coerced |
| Risk numbers mistaken for truth | Deterministic inputs logged; measures are computed, not model-generated; results flagged as estimates |
| Calibration unusable later | `FeatureVector` keys fixed per decision class now; decision rows never lack features |

---

## 12. Design traceability

| Design / spec | Handled by |
|---|---|
| `digital_mind_design1.md` §18 (sanity layer) | §5.4, §7.9–7.10, §9 adversarial |
| §19 (bad-idea detection taxonomy) | §7.9 prohibitions + adversarial classes |
| §20 (comparison integrity) | §5.3, §7.8, §9 fixtures |
| §21 (supposition / assumption ledger) | §5.4 `assumptions`, §7.9 scan |
| §22 (dependency & uncertainty propagation) | §7.9 inputs from Phase 6 |
| §23–24 (contradictions, missing steps) | §7.9 scan signals + reason codes |
| §25 (constraints & feasibility) | §5.4 `constraints`, §6 `logic-*` SAT/SMT |
| §26 (risk & failure analysis) | §5.2, §7.7, reference values |
| §27 (reversibility & optionality) | `RiskProfile.reversibility`/`optionality`, §7.9 |
| §28–30 (prediction, curiosity, attention) | decision logging feeds §7.12 / Phase 11 |
| §46 (sanity firewall outcomes) | §5.4, §7.10 |
| §47–49 (when Jev replaces an LLM; local models; calibration mandatory) | §5.1, §7.3–7.6, §7.12 |
| §50 (symbolic layer) | §6 logic/SAT/SMT for hard constraints |
| §77–79 (self-criticism, red team, verification strategy) | adversarial corpus + precedence policy |
| `sanity_layer1.md` (firewall, taxonomy, reference class, missing step, error budget, stop conditions, decomposed confidence, doubt generator, anti-patterns) | §7.9–7.12, §9 |
| `congitive_elements1.md` (beliefs/uncertainty, agency boundaries, metacognition) | §5.4 authorization, §5.1 bounded questions |
| `congitive_elements2.md` (internal arbitration, resource/executive function, self-evaluation) | §7.10 precedence, §7.12 features |
| `congitive_elements4.md` (epistemic budget, prediction ledger, identity invariants) | §5.4 stakes, §7.12, prohibitions table |
