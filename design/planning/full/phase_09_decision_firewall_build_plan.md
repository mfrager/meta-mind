# Phase 9 — Sanity Firewall, Decision Core, and Comparison Integrity (Extended Build Plan)

> Extended from: `design/planning/phase_09_decision_firewall_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 9 · Codename Metamind (`mm`)

This is the research-grounded, streamlined revision. It keeps the same scope and gate as the Phase 9 plan
but locks in the mechanisms — calibrated bounded decisions, conformal thresholds, uncertainty signals, and
a programmable safety firewall — that Phase 10/11 depend on.

The phase adds the two organs between *reasoning* and *action*: the bounded-decision primitive
(`DecisionCore`, the Jev-style interface) and the unified **sanity firewall** (design §46). It also adds
**comparison integrity** (design §20) as its own subsystem, because most "wrong" decisions are really
invalid comparisons.

---

## 0. Research foundation (code-available)

Every idea below is borrowed from a project whose source is available. Only ideas with code are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| Conformal prediction | MAPIE — https://github.com/scikit-learn-contrib/MAPIE | Split-conformal sets and quantiles that give a chosen empirical coverage | Firewall `VERIFY_FIRST`/`ASK_USER` cutoffs and decision `abstain` are **conformal thresholds** stored in `conformal_thresholds` |
| Calibration metrics + temperature scaling | netcal — https://github.com/fabiankueppers/calibration-framework | Brier, log-loss, ECE, reliability diagrams, temperature scaling | `mm-decision::calibration` computes ECE/Brier and fits one temperature per decision class |
| Semantic entropy | semantic_uncertainty — https://github.com/jlko/semantic_uncertainty | Uncertainty from entropy over meaning clusters (hallucination signal) | `Uncertainty::SemanticEntropy` is a first-class firewall signal |
| Fine-grained UQ | kernel-language-entropy — https://github.com/AlexanderVNikitin/kernel-language-entropy | Kernel-based entropy estimator | Alternative `Uncertainty::KernelEntropy` scorer behind the same trait |
| UQ scorer library | UQLM — https://github.com/cvs-health/uqlm | A catalogue of pluggable UQ scorers | `UncertaintyScorer` registry; scorers are selectable and logged |
| Programmable rails | NeMo Guardrails — https://github.com/NVIDIA-NeMo/Guardrails | Input/output/execution rails guarding an LLM pipeline | Firewall is a rail system: deterministic prohibition rails run before any model call |
| Validators + re-ask | Guardrails AI — https://github.com/guardrails-ai/guardrails | Declarative validators and structured re-ask loops | Hosted core validates structured output through `Validator`s; no silent coercion |
| Bounded safety classifier | Llama Guard / Purple Llama — https://github.com/meta-llama/PurpleLlama | A small classifier returning a bounded safety label | Optional `SafetyRail` classifier; label is a signal, never the final authority |
| Self-consistency factuality | SelfCheckGPT — https://github.com/potsawee/selfcheckgpt | Sample-based hallucination detection | `Factuality::SelfConsistency` check for unsupported claims |
| Atomic factual precision | FActScore — https://github.com/shmsw25/FActScore | Decompose text into atomic facts, score support | `Factuality::AtomicPrecision` for claim-level support scoring |
| Search-augmented factuality | SAFE / long-form-factuality — https://github.com/google-deepmind/long-form-factuality | Decompose → search → judge each atomic fact | `Factuality::SearchAugmented` verification operation (uses Phase 10 tools) |
| Risk-measure definitions | empyrical — https://github.com/quantopian/empyrical | Exact VaR / CVaR / downside deviation definitions | `analyze_risk` mirrors these definitions so reference values match |

---

## 1. Objective and scope

**In scope**
- `mm-decision`: a `DecisionCore` trait answering typed bounded questions (`choice` / `score` / `yes-no`)
  over shared state, with **three interchangeable implementations** (deterministic rules, local head,
  hosted) passing **one identical conformance suite**, plus calibration and conformal abstention.
- Deterministic **risk measures** built on `decision-ir` (expected loss, maximum loss, tail probability,
  ruin probability, variance, downside asymmetry, reversibility, optionality).
- **Uncertainty signals** (semantic entropy, optional kernel entropy, self-consistency) with a fixed
  `UncertaintyScorer` interface.
- **Comparison integrity**: a comparison contract, validity checks, and deterministic normalization.
- `mm-firewall`: a unified scan over Phase 6–8 + Phase 10 inputs emitting one of
  `PROCEED | PROCEED_WITH_CAUTION | VERIFY_FIRST | ASK_USER | REPLAN | HUMAN_REVIEW | REJECT`, with hard
  deterministic prohibitions that short-circuit to `REJECT` regardless of any model output.
- Full feature-and-outcome logging of every decision and firewall run for Phase 11 calibration.

**Out of scope:** distilling frequent decisions into local models (Phase 11) — this phase only defines the
local adapter and consumes decision traces; prediction-ledger calibration metrics (Phase 11) — this phase
only *records* features/outcomes; tool execution and rollback (Phase 10); new library entries (Phase 7).

The rule everything must uphold:

> **Hard prohibitions are deterministic and short-circuit to `REJECT`; model output can only ever make the
> firewall more cautious, never less.**

---

## 2. Architecture

```
 FirewallInput (assumptions, contradictions, comparisons, constraints, missing steps,
                risk, reversibility, authorization, precedent, simpler alternative, stakes)
        │
        ▼
 ┌──────────────────────────┐   any hit ⇒ REJECT (no model consulted)
 │ 1. Hard prohibition rails│ ─────────────────────────────────────────────► REJECT
 │   (deterministic, ordered)│
 └────────────┬─────────────┘
              │ no prohibition
              ▼
 ┌──────────────────────────┐   decisions carry features for later calibration
 │ 2. Bounded judgments     │◄── DecisionCore { rules | local | hosted }
 │   safety/risk/comparison │
 │   precedent/alternative  │
 └────────────┬─────────────┘
              ▼
 ┌──────────────────────────┐   calibrate confidence; conformal coverage target
 │ 3. Uncertainty + calibr. │◄── UncertaintyScorer (semantic/kernel/self-consistency)
 │   + conformal thresholds │
 └────────────┬─────────────┘
              ▼
 ┌──────────────────────────┐
 │ 4. Deterministic         │   precedence: prohib > review > verify > ask >
 │    aggregation           │   replan > caution > proceed
 └────────────┬─────────────┘
              ▼
      FirewallOutcome + ReasonCode[]  ──► decisions / comparisons / firewall_runs (+ RDF mirror)
```

Two producers of judgment, one authority: the `DecisionCore` proposes bounded answers with calibrated
confidence; the **deterministic aggregation** chooses the outcome. Uncertainty and conformal coverage
only push the outcome toward *more* caution.

### 2.1 Decision cores

| Core | Backing | Availability | Use |
|---|---|---|---|
| `RulesCore` | `rules/decision_rules.toml` (table-driven) | always | safety-critical, cheap, high-volume bounded calls |
| `LocalCore` | distilled decision head (Phase 11) | if a head exists | high-volume stable decisions |
| `HostedCore` | `mm-llm` structured output | if provider reachable | novel/unbounded bounded-questions |

All three must pass `tests/conformance.rs`; an impl that fails cannot be selected.

---

## 3. Deliverables and workspace layout

```
~/Build/metamind/
├── crates/mm-decision/
│   ├── src/{lib,core,question,rules,local,hosted}.rs
│   ├── src/risk.rs                 # RiskProfile + analyze_risk (decision-ir)
│   ├── src/compare.rs              # ComparisonContract, check_contract, normalize
│   ├── src/uncertainty.rs          # UncertaintyScorer: semantic/kernel/self-consistency
│   ├── src/calibration.rs          # Brier/ECE, temperature scaling, FeatureVector keys
│   ├── src/conformal.rs            # split-conformal quantiles + coverage targets
│   ├── src/factuality.rs           # atomic precision, search-augmented checks
│   ├── src/rdf.rs                  # ToRdf/FromRdf for decision/comparison/risk/uncertainty
│   └── rules/decision_rules.toml
├── crates/mm-decision/tests/conformance.rs   # ONE suite, run against all three cores
├── crates/mm-firewall/
│   └── src/{lib,scan,rails,prohibitions,aggregate,outcome,report}.rs
├── crates/mm-cli/src/cmd/{decide,firewall,compare,risk,calibration,conformance}.rs
├── crates/mm-store-sqlite/migrations/0009_decision_firewall.sql
├── ontology/mm.ttl                 # + mm:Decision, mm:Comparison, mm:Risk, mm:FirewallRun, mm:Uncertainty
├── ontology/shapes/decision.ttl    # SHACL shapes
├── modules/cognition/comparison-integrity/{plugin.toml,src/lib.rs,tests/,manual/module.md}
└── bench/
    ├── risk/reference_values.json
    ├── comparison/fixtures.jsonl
    ├── adversarial/firewall_cases.jsonl
    ├── firewall/gold.jsonl
    └── calibration/{labeled.jsonl,conformal.jsonl}
```

---

## 4. Detailed specifications

### 4.1 `DecisionCore`

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoreId { Rules, Local, Hosted }

pub struct DecisionState { pub episode: Ulid, pub facts: Vec<ClaimRef>, pub assumptions: Vec<AssumptionRef>,
                           pub constraints: Vec<ConstraintRef>, pub context: serde_json::Value }

#[derive(Clone, Debug)]
pub enum DecisionQuestion {
    Choice { id: Ulid, prompt: String, options: Vec<OptionId>, allow_none: bool },
    Score  { id: Ulid, prompt: String, scale: ScoreScale },   // e.g. 0..1 with anchors
    YesNo  { id: Ulid, prompt: String },
}
#[derive(Clone, Debug)]
pub enum AnswerValue { Option(OptionId), Score(f32), Bool(bool) }

#[derive(Clone, Debug)]
pub struct DecisionAnswer {
    pub question: Ulid,
    pub answer: AnswerValue,
    pub confidence: f32,          // [0,1] by construction
    pub features: FeatureVector,  // logged for calibration
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

`DecisionAnswer::confidence` is the *pre-calibration* value. `calibrate()` applies the class temperature
before the answer reaches the firewall.

### 4.2 Calibration and conformal abstention

```rust
pub struct Calibrator { pub temperature: f32, pub ece: f32, pub brier: f32 }
impl Calibrator {
    pub fn fit(pred: &[f32], labels: &[bool]) -> Self;         // one temperature per decision class
    pub fn calibrate(&self, p: f32) -> f32;                    // logit(p)/T -> sigmoid
}
pub struct ConformalSet { pub q_hat: f32, pub coverage: f32, pub n: usize }
impl ConformalSet {
    /// Split-conformal quantile at the requested coverage (MAPIE semantics).
    pub fn fit(scores: &[f32], target: f32) -> Self;
    pub fn admit(&self, calibrated_p: f32) -> bool;            // false => abstain/verify
}
```

Rules: a class with no calibration data uses `temperature = 1.0` and must be reported as *uncalibrated*;
the firewall treats uncalibrated high-confidence answers as `VERIFY_FIRST`, never `PROCEED`.

### 4.3 Risk (`decision-ir`)

```rust
pub struct RiskProfile {
    pub expected_loss: f64, pub max_loss: f64, pub tail_probability: f64, pub ruin_probability: f64,
    pub variance: f64, pub downside_asymmetry: f64, pub reversibility: f32, pub optionality: f32,
}
pub fn analyze_risk(outcomes: &[decision_ir::Outcome], measure: decision_ir::RiskMeasure) -> RiskProfile;
```

Definitions mirror empyrical so reference values match:

```
VaR_alpha      = -quantile_alpha(loss)                      // alpha-quantile of the loss distribution
CVaR_alpha     =  E[loss | loss >= VaR_alpha]               // expected shortfall
ruin_prob      =  P(sum(loss) >= capital)                   // probability of depleting the risk budget
downside_asym  =  E[max(0, -r)] - E[max(0, r)]             // signed downside deviation
variance       =  Var(loss)
```

Dispersion-spread risk means `max_loss` and `ruin_probability` are reported separately from `expected_loss`.

### 4.4 Comparison integrity

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

`check_contract` verifies same object class, purpose, units (dimensional consistency), timeframe,
operating conditions, definitions, and scope. Mismatches are `NonComparable`/`Rejected`; they are **never
coerced** into a comparison.

### 4.5 Uncertainty and factuality

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UncertaintyKind { SemanticEntropy, KernelEntropy, SelfConsistency }
pub trait UncertaintyScorer: Send + Sync {
    fn kind(&self) -> UncertaintyKind;
    async fn score(&self, claim: &str, samples: &[String]) -> Result<f32, DecisionError>; // [0,1]
}
pub enum FactualityCheck { SelfConsistency, AtomicPrecision, SearchAugmented }
pub struct FactualityReport { pub check: FactualityCheck, pub support: f32, pub atoms: Vec<AtomicFact> }
```

Semantic entropy is computed by clustering samples by meaning (equivalence via the LLM) and taking the
entropy over clusters; higher entropy ⇒ more `VERIFY_FIRST` pressure.

### 4.6 Firewall

```rust
pub enum FirewallOutcome { Proceed, ProceedWithCaution, VerifyFirst, AskUser, Replan, HumanReview, Reject }

pub struct FirewallInput {
    pub episode: Ulid, pub assumptions: Vec<AssumptionRef>, pub contradictions: Vec<Ulid>,
    pub comparisons: Vec<Ulid>, pub constraints: Vec<ConstraintRef>, pub missing_steps: Vec<MissingStep>,
    pub risks: RiskProfile, pub reversibility: Reversibility, pub authorization: AuthorizationRef,
    pub precedent: Option<Ulid>, pub simpler_alternative: Option<Ulid>, pub stakes: Stakes,
    pub uncertainty: Option<Vec<f32>>,
}
pub struct FirewallReport { pub id: Ulid, pub outcome: FirewallOutcome, pub reason_codes: Vec<ReasonCode>,
                            pub decisions: Vec<Ulid>, pub hard_prohibition: Option<HardProhibition> }

#[async_trait::async_trait]
pub trait SanityFirewall {
    async fn evaluate(&self, input: &FirewallInput, ctx: &mut FirewallCtx) -> Result<FirewallReport, FirewallError>;
}
```

Hard prohibition table (deterministic, ordered, checked **before** any `DecisionCore` call):

| Prohibition id | Trigger |
|---|---|
| `identity_invariant` | would violate a Phase 4 identity invariant |
| `unauthorized_tool` | action targets a tool with no matching permission grant (Phase 10) |
| `irreversible_without_approval` | irreversible action with no approval record |
| `unresolved_hard_contradiction` | an unresolved `mm:Contradiction` on a decision-critical claim |
| `spend_over_budget` | projected spend exceeds the Phase 4 budget |
| `schema_invalid_action` | action fails its declared schema |

Aggregation precedence (deterministic): any prohibition ⇒ `Reject`; critical unverified assumption or
catastrophic `ruin_probability`/`max_loss` ⇒ `HumanReview`; failed factuality/uncertainty above the
conformal cutoff ⇒ `VerifyFirst`; invalid comparison ⇒ `Replan`; missing authorization/intent ⇒ `AskUser`;
bounded residual risk above the caution conformal cutoff ⇒ `ProceedWithCaution`; otherwise `Proceed`.

### 4.7 SQLite — `0009_decision_firewall.sql`

```sql
CREATE TABLE decisions (
  id TEXT(26) PRIMARY KEY, question_kind TEXT NOT NULL, question_json TEXT NOT NULL,
  answer_json TEXT NOT NULL,
  confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
  calibrated_confidence REAL,
  features_json TEXT NOT NULL, core_impl TEXT NOT NULL,
  latency_ms INTEGER NOT NULL, cost REAL NOT NULL DEFAULT 0.0,
  created_ulid TEXT(26) NOT NULL, episode_ulid TEXT(26),
  outcome_json TEXT, outcome_ulid TEXT(26)
);
CREATE INDEX decisions_episode ON decisions(episode_ulid);
CREATE INDEX decisions_core ON decisions(core_impl, question_kind);

CREATE TABLE comparisons (
  id TEXT(26) PRIMARY KEY, objective TEXT NOT NULL, contract_json TEXT NOT NULL,
  verdict TEXT NOT NULL, violations_json TEXT NOT NULL, normalized_json TEXT,
  created_ulid TEXT(26) NOT NULL
);

CREATE TABLE firewall_runs (
  id TEXT(26) PRIMARY KEY, episode_ulid TEXT(26) NOT NULL, inputs_json TEXT NOT NULL,
  outcome TEXT NOT NULL, reason_codes_json TEXT NOT NULL, decision_ulids TEXT NOT NULL,
  hard_prohibition TEXT, created_ulid TEXT(26) NOT NULL
);
CREATE INDEX firewall_runs_episode ON firewall_runs(episode_ulid);

CREATE TABLE calibration (
  id TEXT(26) PRIMARY KEY, decision_class TEXT NOT NULL, temperature REAL NOT NULL,
  ece REAL NOT NULL, brier REAL NOT NULL, n INTEGER NOT NULL, created_ulid TEXT(26) NOT NULL
);
CREATE INDEX calibration_class ON calibration(decision_class, created_ulid);

CREATE TABLE conformal_thresholds (
  id TEXT(26) PRIMARY KEY, decision_class TEXT NOT NULL, target_coverage REAL NOT NULL,
  q_hat REAL NOT NULL, n INTEGER NOT NULL, created_ulid TEXT(26) NOT NULL
);
CREATE INDEX conformal_class ON conformal_thresholds(decision_class);

CREATE TABLE factuality_checks (
  id TEXT(26) PRIMARY KEY, claim_ulid TEXT(26) NOT NULL, check_kind TEXT NOT NULL,
  support REAL NOT NULL, atoms_json TEXT, created_ulid TEXT(26) NOT NULL
);
```

Conventions: ULID `TEXT(26)` keys, `created_ulid` correlation, WAL + `foreign_keys=ON` (Phase 1).

### 4.8 RDF / OWL (`ontology/mm.ttl`, `mm:` = `https://metamind.dev/ontology#`)

Classes `mm:Decision, mm:Comparison, mm:Risk, mm:RiskMeasure, mm:FirewallRun, mm:ReasonCode,
mm:Uncertainty`; predicates `mm:hasQuestion, mm:questionKind, mm:selectedOption, mm:hasConfidence,
mm:calibratedConfidence, mm:decidedBy, mm:hasFeature, mm:hasOutcome, mm:hasRisk, mm:usesMeasure,
mm:measuredValue, mm:comparedTo, mm:comparisonValid, mm:hasViolation, mm:firewallOutcome,
mm:hasReasonCode, mm:consultedDecision, mm:hasUncertainty`. Instances are
`<https://metamind.dev/data/{ulid}>`; records live in `https://metamind.dev/graph/decision`, with audit
links mirrored into `https://metamind.dev/graph/provenance`.

### 4.9 SHACL (`ontology/shapes/decision.ttl`)

- Every `mm:Decision`: exactly one `mm:questionKind`, `mm:decidedBy`, `mm:hasConfidence` ∈ `[0,1]`.
- Every `mm:FirewallRun`: exactly one `mm:firewallOutcome` and ≥1 `mm:hasReasonCode`; a `REJECT` due to a
  prohibition MUST carry `mm:hardProhibition`.
- Every `mm:Uncertainty`: exactly one `mm:measuredValue` ∈ `[0,1]` and a `mm:usesMeasure`.
- Generated with `rdf-shacl` (`check_ontology` → `generate_shacl` → `validate`); unsupported constructs
  are hard errors.

### 4.10 CLI

```
mm-cli decide --question <file|json> [--core rules|local|hosted] [--calibrate] [--episode <ulid>]
mm-cli firewall eval --episode <ulid> [--input <file>] [--json]
mm-cli compare check --fixtures <file|-> | --a <file> --b <file>
mm-cli risk analyze --outcomes <file> [--measure var|cvar|ruin|max_loss] [--tolerance <f64>]
mm-cli calibration fit --labeled bench/calibration/labeled.jsonl [--class <name>]
mm-cli calibration conformal --scores bench/calibration/conformal.jsonl --coverage 0.9
mm-cli conformance --core rules --core local --core hosted
```

### 4.11 Module `plugin.toml` (`modules/cognition/comparison-integrity/`)

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

### 4.12 External references copied in and integrated

Copy only what is needed into `vendor/<origin>/…` as workspace members; record origin repo, revision, and
license in `vendor/<origin>/COPYING.md`, and emit `mmc:copiedFrom` during the Phase 2 scan. No git
submodules, no external path dependencies, no `[patch]`.

| Reference (origin) | Copied-in crate | Integration |
|---|---|---|
| `decision-ir` (`rust_symbolic`) | `vendor/rust_symbolic/decision-ir` | `DecisionEngine`, `RiskMeasure`, `Outcome`, `BayesianProblem`, `Mdp`, `MultiObjectiveProblem` behind `mm-decision::risk` |
| `ensemble-ir` (`rust_symbolic`) | `vendor/rust_symbolic/ensemble-ir` | multi-lens disagreement as firewall evidence |
| `causal-ir` (`rust_symbolic`) | `vendor/rust_symbolic/causal-ir` | risk structure + counterfactual downside in `FirewallInput` |
| `logic-ir` + `solver-ir` + `backend-registry` + `logic-planner` | `vendor/rust_symbolic/{logic-ir,solver-ir,backend-registry,logic-planner}` | hard-constraint feasibility via SAT/SMT; proof obligations; sandboxed calls |
| `rdf-codec` + `rdf-shacl` | `vendor/rust_symbolic/{rdf-codec,rdf-shacl}` | canonical RDF encode/decode; SHACL generation + validation |
| `kg-llm` (`rust_extract`) | `vendor/rust_extract/kg-llm` | structured-output client behind `mm-llm` for `HostedCore`, entropy sampling |

Adaptation rule: `mm-decision` never exposes a `decision-ir` type publicly; it maps to `RiskProfile`,
`ComparisonCheck`, and `DecisionAnswer`.

---

## 5. Build sequence

1. **Copy-in** the reference crates (§4.12); add `vendor/<origin>/COPYING.md`; add to `[workspace] members`.
   *Check:* `cargo build -p mm-decision`.
2. **`mm-decision` skeleton**: `core.rs`, `question.rs`; `DecisionState`/`DecisionQuestion`/`DecisionAnswer`
   with confidence bounded by construction; `#![forbid(unsafe_code)]`. *Check:* unit tests.
3. **Conformance suite** (`tests/conformance.rs`): typing, determinism, confidence ∈ [0,1], stable
   tie-breaks. *Check:* fails against a deliberately nondeterministic stub.
4. **`RulesCore`** from `rules/decision_rules.toml`; ordered tie-break by rule id. *Check:* golden matches.
5. **`LocalCore`** adapter (loads a distilled head or returns `Unavailable`). *Check:* `Unavailable` path.
6. **`HostedCore`** via `mm-llm` `StructuredOut<DecisionAnswer>`; reject schema violations. *Check:* malformed
   reply rejected.
7. **Risk** (`risk.rs`): all eight measures; VaR/CVaR/ruin per empyrical. *Check:* `risk analyze --tolerance 1e-6`.
8. **Comparison** (`compare.rs`): contract, `check_contract`, deterministic `normalize`. *Check:* fixtures.
9. **Uncertainty** (`uncertainty.rs`): semantic entropy + optional kernel entropy + self-consistency behind
   one trait. *Check:* determinism on fixed samples.
10. **Calibration** (`calibration.rs`): Brier/ECE + temperature scaling per class. *Check:* ECE improves on
    the labeled set after fitting.
11. **Conformal** (`conformal.rs`): split-conformal `q_hat` + `admit`; thresholds stored per class.
    *Check:* empirical coverage within tolerance on held-out data.
12. **Factuality** (`factuality.rs`): atomic precision + search-augmented check (tool seam for Phase 10).
13. **`mm-firewall`**: `rails.rs` + `prohibitions.rs` (ordered table) → `scan.rs` (Phase 6–8 + 10 signals) →
    `aggregate.rs` (precedence) → `outcome.rs`/`report.rs`. *Check:* prohibition short-circuit test.
14. **Persistence + RDF**: write `decisions`/`comparisons`/`firewall_runs`/`calibration`/`conformal_thresholds`
    /`factuality_checks`; mirror `/decision` + `/provenance` audit. *Check:* `graph validate --graph decision`.
15. **CLI** (§4.10) + fixtures (`bench/…`) + E2E `tests/e2e/phase09_firewall.rs`. *Check:* gate commands.

---

## 6. Logging and observability

All via `mm-log` (structured records, ULID `trace_id` correlation, five sinks, redaction).

| Event code | Required fields |
|---|---|
| `decision.core.answer` | `question_id`, `question_kind`, `core_impl`, `answer`, `confidence`, `calibrated_confidence`, `latency_ms`, `feature_keys` |
| `decision.core.unavailable` | `core_impl`, `reason` |
| `decision.rules.match` | `rule_id`, `matched` (bool), `state_hash` |
| `decision.hosted.call` | `model`, `prompt_hash`, `tokens_in`, `tokens_out`, `cost`, `schema_ok` |
| `decision.abstain` | `question_id`, `calibrated_confidence`, `q_hat`, `target_coverage` |
| `decision.log.write` | `decision_id`, `episode_id`, `outcome_present` (bool) |
| `risk.analyze` | `measure`, `expected_loss`, `max_loss`, `tail_probability`, `ruin_probability`, `variance`, `downside_asymmetry`, `reversibility`, `optionality` |
| `compare.contract` | `objective`, `objects`, `dimensions`, `units`, `timeframe` |
| `compare.check` | `verdict`, `violation_codes` |
| `compare.normalize` | `applied_conversions`, `timeframe_shift` |
| `uncertainty.score` | `kind`, `value`, `n_samples` |
| `factuality.check` | `claim_id`, `check_kind`, `support`, `atoms` |
| `calibration.fit` | `decision_class`, `n`, `temperature`, `ece_before`, `ece_after`, `brier` |
| `conformal.adjust` | `decision_class`, `target_coverage`, `q_hat`, `n`, `empirical_coverage` |
| `firewall.scan.start` | `episode_id`, `input_digest` |
| `firewall.prohibition.shortcircuit` | `prohibition`, `outcome=REJECT`, `evidence` |
| `firewall.aggregate` | `signals`, `outcome`, `reason_codes`, `consulted_decision_ids` |
| `firewall.report` | `firewall_run_id`, `outcome`, `hard_prohibition` |

**Audit records (immutable)** are written transactionally with each `decisions`, `comparisons`,
`firewall_runs`, `calibration`, `conformal_thresholds` insert; every RDF mirror; every hard-prohibition
short-circuit. Prompts/responses are hashed, never logged verbatim (redaction). `mm-cli logs verify` checks
schema validity, one audit record per committed mutation, gapless audit sequence, redaction, and
replay-equivalence.

---

## 7. Testing

| Type | Test | Assertion |
|---|---|---|
| Conformance | `tests/conformance.rs` vs 3 cores | typing, determinism, confidence bounds, stable tie-breaks |
| Golden | `OperationValue`/decision snapshots (`insta`) | aggregation outputs stable for fixed inputs |
| Reference | `bench/risk/reference_values.json` | every measure matches hand-computed VaR/CVaR/ruin within `1e-6` |
| Fixtures | `bench/comparison/fixtures.jsonl` | matched pairs `Valid`; mismatches `NonComparable`/`Rejected` |
| Adversarial | `bench/adversarial/firewall_cases.jsonl` + `bench/firewall/gold.jsonl` | precision ≥ 0.95, recall ≥ 0.90; 100% of hard-prohibition cases `REJECT` |
| Calibration | `bench/calibration/labeled.jsonl` | ECE/Brier computed; temperature fit lowers ECE |
| Conformal | `bench/calibration/conformal.jsonl` | empirical coverage within ±0.03 of target |
| Completeness | SQL audit | 100% of `decisions.features_json` and `firewall_runs.reason_codes_json` non-null |
| E2E | `tests/e2e/phase09_firewall.rs` | episode → firewall run → recorded decision → RDF mirror |

---

## 8. Pass gate

```bash
cd ~/Build/metamind
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 1) One conformance suite passes for all three impls
cargo run -p mm-cli -- conformance --core rules --core local --core hosted

# 2) Firewall thresholds; every hard prohibition => REJECT
cargo run -p mm-cli -- firewall eval --episode bench/ep.json \
    --corpus bench/adversarial/firewall_cases.jsonl --report bench/firewall/report.json
# PASS: precision >= 0.95 and recall >= 0.90 on unsafe outcomes; 100% of hard-prohibition cases REJECT.

# 3) Risk measures match reference values
cargo run -p mm-cli -- risk analyze --outcomes bench/risk/reference_values.json --tolerance 1e-6

# 4) Comparisons match expected verdicts
cargo run -p mm-cli -- compare check --fixtures bench/comparison/fixtures.jsonl

# 5) Calibration + conformal coverage
cargo run -p mm-cli -- calibration fit --labeled bench/calibration/labeled.jsonl
cargo run -p mm-cli -- calibration conformal --scores bench/calibration/conformal.jsonl --coverage 0.9

# 6) 100% of decisions and firewall runs logged with features + outcome
sqlite3 data/metamind.db \
 "SELECT (SELECT COUNT(*) FROM decisions WHERE features_json IS NULL)
       + (SELECT COUNT(*) FROM firewall_runs WHERE reason_codes_json IS NULL);"   # PASS: 0

# 7) Logging verification
cargo run -p mm-cli -- logs verify
```

Objective gate conditions: (a) all three cores conform; (b) firewall precision/recall met and hard
prohibitions unconditionally `REJECT`; (c) risk measures within tolerance; (d) comparison fixtures all
correct; (e) ECE improves and conformal empirical coverage within ±0.03 of target; (f) logging
completeness `0`; (g) `mm-cli logs verify` passes.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| `DecisionCore` drift between implementations | One shared conformance suite; a failing impl cannot be selected |
| Uncalibrated confidence treated as truth | `temperature = 1.0` classes are flagged *uncalibrated*; firewall escalates them to `VERIFY_FIRST` |
| Hosted decision model unavailable | `HostedCore` returns `Unavailable`; aggregation falls back to rules + escalate — never fabricate a decision |
| Conformal threshold overfits a small calibration set | Minimum `n` per class; coverage recomputed on held-out data; thresholds versioned with `created_ulid` |
| Semantic-entropy sampling cost | Bound `n_samples` per class; cache by prompt hash; optionally route to `LocalCore` |
| Firewall over-blocking (false `REJECT`) | Explicit precedence policy; near-miss cases added to the adversarial corpus; recall measured, not assumed |
| Invalid comparison silently accepted | `check_contract` is mandatory before any compare; mismatches never coerced |
| Risk numbers mistaken for truth | Measures are computed from logged inputs, not model-generated; reported as estimates |
| Reusing a cached decision across changed state | Decision cache key includes a state digest; wrong-state hits are rejected (mirrors semantic-cache safety) |

---

## 10. References (code-available)

- MAPIE (conformal prediction) — https://github.com/scikit-learn-contrib/MAPIE
- netcal (calibration) — https://github.com/fabiankueppers/calibration-framework
- semantic entropy — https://github.com/jlko/semantic_uncertainty
- kernel language entropy — https://github.com/AlexanderVNikitin/kernel-language-entropy
- UQLM (uncertainty quantification) — https://github.com/cvs-health/uqlm
- NeMo Guardrails — https://github.com/NVIDIA-NeMo/Guardrails
- Guardrails AI — https://github.com/guardrails-ai/guardrails
- Llama Guard / Purple Llama — https://github.com/meta-llama/PurpleLlama
- SelfCheckGPT — https://github.com/potsawee/selfcheckgpt
- FActScore — https://github.com/shmsw25/FActScore
- SAFE / long-form factuality — https://github.com/google-deepmind/long-form-factuality
- empyrical (VaR/CVaR definitions) — https://github.com/quantopian/empyrical

### Design traceability

| Design / spec | Handled by |
|---|---|
| `digital_mind_design1.md` §18 (sanity layer) | §4.6, §5.13, §7 adversarial |
| §19 (bad-idea detection taxonomy) | prohibition table + adversarial classes |
| §20 (comparison integrity) | §4.4, §5.8, §7 fixtures |
| §21 (assumption ledger) | `FirewallInput.assumptions`, §5.13 |
| §22 (dependency & uncertainty propagation) | Phase 6 inputs + §4.6 |
| §23–24 (contradictions, missing steps) | §4.6 scan signals + reason codes |
| §25 (constraints & feasibility) | `constraints`, §4.12 SAT/SMT |
| §26 (risk & failure analysis) | §4.3, §5.7, reference values |
| §27 (reversibility & optionality) | `RiskProfile.reversibility`/`optionality` |
| §46 (sanity firewall outcomes) | §4.6, aggregation precedence |
| §47–49 (Jev; local models; calibration mandatory) | §4.1–§4.2, §5.3–5.6, §5.10–5.11 |
| §50 (symbolic layer) | §4.12 logic/SAT/SMT for hard constraints |
| §77–79 (self-criticism, red team, verification strategy) | adversarial corpus + uncertainty/factuality |
| `sanity_layer1.md` (taxonomy, reference class, missing step, error budget, stop conditions) | §4.6, §5.13 |
| `congitive_elements1.md` (beliefs/uncertainty, agency boundaries) | firewalls authorization, §4.1 |
| `congitive_elements2.md` (arbitration, self-evaluation) | §4.2 calibration, §4.6 precedence |
| `congitive_elements4.md` (epistemic budget, identity invariants) | `stakes`, prohibitions, features |
