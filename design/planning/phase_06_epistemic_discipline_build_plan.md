# Phase 6 Build Plan — Epistemic discipline

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 6 · Codename Metamind (`mm`)
> Supporting specs read: `design/sanity_layer1.md`, `design/congitive_elements3.md`,
> `design/congitive_elements4.md`; master design §18–25, §28, §36–38, §51–52, §87–90, §108–110.

---

## 1. Objective and scope

Make it structurally impossible for the being to confuse **what it thinks** with **what the world
contains**. Phase 6 introduces the epistemic layer: claims, evidence, observations, inferences,
hypotheses, assumptions, and predictions, each carrying an explicit `EpistemicStatus`; an assumption
ledger with a deterministic verification priority; explicit contradiction records; and an epistemic
dependency graph whose invalidation cascades to dependents.

**In scope**
- `mm-epistemic` crate: the typed epistemic objects, status machinery, promotion guard, assumption ledger,
  contradiction detection, dependency graph, and a deterministic validation barrier before commit.
- Separate `/world` and `/epistemic` named graphs, with a queryable guarantee that `/world` holds only
  `OBSERVED`/`VERIFIED` facts.
- One nexus-style module exposing epistemic operations to the DSL (`modules/epistemic/epistemic-ops`).
- SQLite persistence + RDF mirror, logging, and the golden/adversarial fixtures for the gate.

**Out of scope (later phases)**
- Metacognitive program compilation (Phase 8); the sanity *firewall* outcome enum and Jev-style decisions
  (Phase 9) — Phase 6 only produces the typed facts those layers consume.
- Causal counterfactual *computation* (Phase 11 diagnosis); Phase 6 only labels `HYPOTHETICAL`/
  `SIMULATED`/`FICTIONAL` content correctly so it can never contaminate world state.
- Learned calibration thresholds (Phase 11); Phase 6 defines the prediction ledger rows.

---

## 2. Prerequisites and dependencies

| Depends on | Artifact required |
|---|---|
| Phase 1 | workspace, `mm-core` (ULID/`Id::iri`, `Timestamp`, `MmError`), `mm-log`, `mm-store-sqlite` (WAL + migrations), `mm-store-graph` (Oxigraph actor, named graphs), `mm-eventlog` (append→apply→commit + replay), `ontology/mm.ttl` v0, `mm-cli doctor/replay/graph validate` |
| Phase 2 | `mm-codex` registry, `mmc:copiedFrom` provenance, `codex verify` |
| Phase 3 | `mm-llm` `LlmClient` + `StructuredOut<T>` (used to *propose* claims; never to decide status) |
| Phase 4 | `mm-being` identity invariants ("never promote assumption to observation") enforced in code |
| Phase 5 | `mm-memory` prediction/mistake records that Phase 6 prediction outcomes feed |

External references (copied in, not linked — see §6): `provenance-ir`, `epistemic-ir`, `event-ir`,
`kg-validate`.

---

## 3. Deliverables (exact paths)

Crate (all `#![forbid(unsafe_code)]`, zero clippy warnings):

```
crates/mm-epistemic/
├── Cargo.toml
├── src/lib.rs                 # re-exports; crate docs
├── src/status.rs              # EpistemicStatus + legal transitions
├── src/proposition.rs         # Proposition, ClaimKind
├── src/claim.rs               # Claim, Evidence, Observation, Inference, Hypothesis
├── src/assumption.rs          # Assumption, AssumptionLedger, verification_priority()
├── src/prediction.rs          # Prediction, PredictionOutcome
├── src/contradiction.rs       # Contradiction, ContradictionDetector
├── src/dependency.rs          # DependencyGraph, cascade invalidation
├── src/guard.rs               # PromotionGuard (deterministic, no model input)
├── src/validate.rs            # ValidationBarrier
├── src/rdf.rs                 # ToRdf/FromRdf via rdf-codec; canonical serialization
├── src/engine.rs              # EpistemicEngine (ingest/verify/invalidate/query)
├── src/error.rs               # EpistemicError
└── tests/{promotion_guard.rs, contradiction.rs, dependency.rs, rdf_roundtrip.rs, replay.rs}
```

Copied reference crates: `vendor/rust_symbolic/provenance-ir/`, `vendor/rust_symbolic/epistemic-ir/`,
`vendor/rust_symbolic/event-ir/`, `vendor/rust_extract/kg-validate/` (each with a `COPYING.md`).

Other deliverables:

```
ontology/mm.ttl                                   # + epistemic classes/predicates
ontology/shapes/epistemic_shapes.ttl              # hand-written SHACL for the fragment rdf-shacl does not generate
crates/mm-store-sqlite/migrations/0006_epistemic.sql
modules/epistemic/epistemic-ops/{plugin.toml,src/lib.rs,manual/module.md,tests/}
bench/golden/verification_priority.csv
bench/epistemic/promotion_guard/forbidden.jsonl
bench/adversarial/contradictions/*.jsonl
bench/epistemic/cascade/*.json
tests/e2e/epistemic_gate.rs
```

New `mm-cli` subcommands: `epistemic ingest`, `epistemic verify`, `epistemic promotable`,
`epistemic contradictions`, `epistemic assumptions`, `epistemic dependency`, `epistemic invalidate`,
`epistemic world`.

---

## 4. Data model and ontology deltas

### SQLite (`migrations/0006_epistemic.sql`) — all ids `TEXT(26)`, ULID

```sql
CREATE TABLE claims (
  id TEXT(26) PRIMARY KEY, kind TEXT NOT NULL,
  subject TEXT NOT NULL, predicate TEXT NOT NULL, object TEXT NOT NULL,
  status TEXT NOT NULL, confidence REAL NOT NULL,
  valid_from TEXT, valid_until TEXT, source_ulid TEXT,
  created_ulid TEXT NOT NULL
);
CREATE TABLE evidence (
  id TEXT(26) PRIMARY KEY, claim_id TEXT(26) NOT NULL REFERENCES claims(id),
  kind TEXT NOT NULL, source_uri TEXT, span_start INTEGER, span_end INTEGER,
  content_hash TEXT NOT NULL, reliability REAL NOT NULL, created_ulid TEXT NOT NULL
);
CREATE TABLE observations (
  id TEXT(26) PRIMARY KEY, claim_id TEXT(26) NOT NULL REFERENCES claims(id),
  authoritative INTEGER NOT NULL, observed_at TEXT NOT NULL,
  source_ulid TEXT NOT NULL, created_ulid TEXT NOT NULL
);
CREATE TABLE assumptions (
  id TEXT(26) PRIMARY KEY, proposition_uri TEXT NOT NULL,
  status TEXT NOT NULL, confidence REAL NOT NULL,
  consequence_if_false TEXT NOT NULL, verification_cost REAL NOT NULL,
  decision_dependence REAL NOT NULL, valid_from TEXT, valid_until TEXT,
  created_ulid TEXT NOT NULL
);
CREATE TABLE predictions (
  id TEXT(26) PRIMARY KEY, proposition_uri TEXT NOT NULL, probability REAL NOT NULL,
  horizon_secs INTEGER NOT NULL, conditions_json TEXT NOT NULL,
  created_ulid TEXT NOT NULL, outcome_status TEXT, outcome_value TEXT, resolved_at TEXT
);
CREATE TABLE contradictions (
  id TEXT(26) PRIMARY KEY, claim_a TEXT(26) NOT NULL, claim_b TEXT(26) NOT NULL,
  reason TEXT NOT NULL, evidence_a TEXT, evidence_b TEXT,
  status TEXT NOT NULL, created_ulid TEXT NOT NULL
);
CREATE TABLE dependencies (
  id TEXT(26) PRIMARY KEY, dependent TEXT NOT NULL, dependency TEXT NOT NULL,
  kind TEXT NOT NULL, criticality REAL NOT NULL, created_ulid TEXT NOT NULL,
  UNIQUE(dependent, dependency)
);
CREATE TABLE epistemic_transitions (
  id TEXT(26) PRIMARY KEY, subject TEXT NOT NULL, from_status TEXT, to_status TEXT,
  reason TEXT NOT NULL, evidence_ulid TEXT, created_ulid TEXT NOT NULL
);
CREATE INDEX idx_claims_spo ON claims(subject, predicate, object);
CREATE INDEX idx_deps_dependency ON dependencies(dependency);
```

### Ontology (`ontology/mm.ttl`, namespace `mm:` = `https://metamind.dev/ontology#`)

- **Classes:** `mm:Claim`, `mm:Observation`, `mm:Inference`, `mm:Hypothesis`, `mm:Assumption`,
  `mm:Prediction`, `mm:Evidence`, `mm:Contradiction`, `mm:Dependency`.
- **Object/datatype properties:** `mm:status`, `mm:confidence`, `mm:evidenceFor`, `mm:supports`,
  `mm:contradicts`, `mm:derivedFrom`, `mm:dependsOn`, `mm:observedAt`, `mm:validFrom`, `mm:validUntil`,
  `mm:consequenceIfFalse`, `mm:verificationCost`, `mm:decisionDependence`, `mm:source`,
  `mm:propositionSubject`, `mm:propositionPredicate`, `mm:propositionObject`.
- **Named graphs:** `https://metamind.dev/graph/epistemic` (all epistemic objects) and
  `https://metamind.dev/graph/world` (**only** `OBSERVED`/`VERIFIED`). The validation barrier is the
  only writer permitted to touch `/world`.

`ontology/shapes/epistemic_shapes.ttl` enforces: exactly one `mm:status` per claim; `mm:confidence` in
`[0,1]`; every `mm:Assumption` has `mm:consequenceIfFalse`, `mm:verificationCost`,
`mm:decisionDependence`; a `mm:Dependency` is not self-referential.

---

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

```rust
// status.rs
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EpistemicStatus { Observed, Verified, Reported, Inferred, Assumed,
                           Hypothetical, Predicted, Simulated, Fictional, Unknown }

// guard.rs  — deterministic; takes NO model output
pub struct PromotionDenied { pub from: EpistemicStatus, pub to: EpistemicStatus, pub reason: &'static str }
pub fn can_promote(from: EpistemicStatus, to: EpistemicStatus, ev: &[Evidence]) -> Result<(), PromotionDenied>;

// proposition.rs
pub struct Proposition { pub subject: NamedNode, pub predicate: NamedNode, pub object: Term }
pub enum ClaimKind { Fact, Observation, Inference, Hypothesis, Prediction, Simulation, Fiction }

// claim.rs
pub struct Claim { pub id: Ulid, pub kind: ClaimKind, pub proposition: Proposition,
                   pub status: EpistemicStatus, pub confidence: f32,
                   pub evidence: Vec<Ulid>, pub valid_from: Option<Timestamp>,
                   pub valid_until: Option<Timestamp> }
pub struct Evidence { pub id: Ulid, pub kind: EvidenceKind, pub source_uri: Option<NamedNode>,
                      pub span: Option<(u32,u32)>, pub content_hash: [u8;32], pub reliability: f32 }

// assumption.rs
pub struct Assumption { pub id: Ulid, pub proposition: Proposition,
                        pub status: EpistemicStatus, pub confidence: f32,
                        pub consequence_if_false: RiskLevel, pub verification_cost: f64,
                        pub decision_dependence: f64, pub dependencies: Vec<Ulid> }
pub fn verification_priority(a: &Assumption, p_false: f64) -> f64;

// dependency.rs
pub struct DependencyGraph { /* ... */ }
impl DependencyGraph {
    pub fn add(&mut self, dependent: Ulid, dependency: Ulid, kind: DepKind, criticality: f32) -> Result<()>;
    pub fn is_acyclic(&self) -> bool;
    pub fn transitive_dependents(&self, root: Ulid) -> Vec<Ulid>;
}

// contradiction.rs
pub struct Contradiction { pub id: Ulid, pub claim_a: Ulid, pub claim_b: Ulid,
                           pub reason: String, pub status: ContradictionStatus }
pub trait ContradictionDetector { fn check(&self, new: &Claim, existing: &[Claim]) -> Vec<Contradiction>; }

// engine.rs (async, tokio)
#[async_trait::async_trait]
pub trait EpistemicEngine {
    async fn ingest(&self, candidate: Claim, evidence: Vec<Evidence>) -> Result<Ulid, EpistemicError>;
    async fn verify(&self, id: Ulid, ev: Evidence) -> Result<(), EpistemicError>;
    async fn invalidate(&self, id: Ulid, reason: &str) -> Result<Vec<Ulid>, EpistemicError>;
    async fn set_status(&self, id: Ulid, to: EpistemicStatus) -> Result<(), EpistemicError>;
}
```

CLI (all emit structured logs and return non-zero on violation):

```bash
mm-cli epistemic ingest    --file bench/epistemic/claims.jsonl
mm-cli epistemic promotable --fixture bench/epistemic/promotion_guard/forbidden.jsonl
mm-cli epistemic contradictions --fixture bench/adversarial/contradictions/
mm-cli epistemic assumptions --sort priority --limit 20
mm-cli epistemic dependency  cascade --root <ulid>
mm-cli epistemic invalidate  <ulid> --reason "source retracted"
mm-cli epistemic world       --query '<sparql>'
mm-cli graph validate        --graph epistemic
```

`modules/epistemic/epistemic-ops/plugin.toml` (nexus contract):

```toml
[plugin]
name = "mm-epistemic-ops"
uri = "https://metamind.dev/code/module/epistemic/epistemic-ops"
version = "0.1.0"
[metadata]
category = "epistemic"
owned_by_phase = 6
capability = "mm:EpistemicDiscipline"
[tbox.functions]
"epistemic.claim" = { source = "handlers::claim" }
"epistemic.assume" = { source = "handlers::assume" }
"epistemic.predict" = { source = "handlers::predict" }
```

---

## 6. External references copied in and integration

Copy the needed source into `vendor/…` as Metamind-owned workspace members; record `mmc:copiedFrom`
(origin repo, path, revision, license) in the Phase 2 scan and a `COPYING.md` per tree.

| Reference | Copied to | Integration |
|---|---|---|
| `provenance-ir` (`KnowledgeClaim`, `KnowledgeSource`, `SourceKind`, `Contradiction`, trust update) | `vendor/rust_symbolic/provenance-ir` | Registers the canonical claim/source schemas `mm-epistemic` reuses |
| `epistemic-ir` (`BeliefState`, `Evidence`, `RevisionOperation`, `KripkeFrame`) | `vendor/rust_symbolic/epistemic-ir` | Underpins status transitions and contradiction classification |
| `event-ir` (`Initiates`/`Terminates`, fluents, state transitions) | `vendor/rust_symbolic/event-ir` | Temporal validity + invalidation as state transitions on the event log |
| `kg-validate` (`EvidenceValidator`, `ClaimValidator`, conformance) | `vendor/rust_extract/kg-validate` | Pattern for the validation barrier that admits only well-formed claims |

All four sit behind `mm-epistemic` traits; the rest of the runtime never names their internal types.

---

## 7. Step-by-step implementation tasks

1. **Scaffold** `crates/mm-epistemic` and `modules/epistemic/epistemic-ops`; add both to the workspace
   members. Copy the four reference crates into `vendor/` with `COPYING.md`.
2. **`status.rs` + `guard.rs`.** Define `EpistemicStatus` and the legal-transition table. Implement
   `can_promote` so the forbidden promotions are impossible regardless of evidence:
   `Assumed→Verified`, `Assumed→Observed`, `Inferred→Observed`, `Predicted→Observed`,
   `Simulated→Verified`, `Fictional→*`, and any promotion of `Unknown`. Only `Observed` (authoritative
   observation) and `Verified` (external/tool evidence) may write to `/world`.
3. **`proposition.rs` + `claim.rs`.** Typed `Proposition` and `Claim`/`Evidence` with append-only
   provenance; constructors validate confidence ∈ [0,1] and temporal ordering.
4. **`assumption.rs`.** `AssumptionLedger` + deterministic `verification_priority = p_false ×
   risk_weight(consequence_if_false) × decision_dependence / max(verification_cost, EPS)`. The arithmetic
   is pure Rust; `p_false` comes from the caller (LLM/Phase 9), never computed here.
5. **`prediction.rs`.** Prediction ledger rows with `probability`, `horizon_secs`, `conditions`, and a
   nullable outcome; resolution writes `epistemic_transitions` and (Phase 11) feeds calibration.
6. **`contradiction.rs`.** `ContradictionDetector::check` compares a candidate against existing claims by
   indexed `(subject, predicate)` buckets; on conflict it emits an explicit `Contradiction` (both claim
   ids + evidence) and **never** reconciles silently. Set both claims `Uncertain`/`Reported` and log.
7. **`dependency.rs`.** DAG over ULIDs. `add` rejects cycles; `cascade_invalidate` marks every transitive
   dependent suspect in one pass, emitting one event per affected node.
8. **`validate.rs`.** `ValidationBarrier` runs before any commit: `rdf-shacl` shapes + symbolic checks
   (`kg-validate` conformance pattern). Rejections are hard errors with reasons, never silent skips.
9. **`rdf.rs`.** `ToRdf`/`FromRdf` for every type via `rdf-codec` (`RdfContext`, `Namespace`, `new_ulid`);
   canonical serialization so re-parses hash identically.
10. **`engine.rs`.** Async orchestration: validate → append event → mutate SQLite → mirror to
    `/epistemic` (and `/world` only for allowed statuses) → commit event. All writes use the Phase 1
    append/apply/commit protocol.
11. **CLI + module.** Wire `mm-cli epistemic …`, register the nexus module, add `mm-cli epistemic world`
    to enforce the world/epistemic separation.
12. **Fixtures.** Populate `bench/golden/verification_priority.csv`,
    `bench/epistemic/promotion_guard/forbidden.jsonl`, `bench/adversarial/contradictions/`,
    `bench/epistemic/cascade/`.
13. **Wire the invariant.** Add the Phase 4 identity invariant "no promotion without satisfying evidence"
    to call `guard::can_promote`; a violation is a deterministic error.
14. **Docs.** `manual/module.md` describing epistemic operations for the LLM.

---

## 8. Detailed logging requirements

All through `mm-log` (§10 of the parent plan): structured records with `trace_id` (ULID), `target`, and
the fields below. Status transitions, contradictions, and invalidation cascades are **audit records**
(append-only, immutable, gapless sequence). Event codes:

| Event code | Fields |
|---|---|
| `epistemic.claim.ingest` | `claim_id`, `kind`, `status`, `confidence`, `graph` |
| `epistemic.evidence.attach` | `claim_id`, `evidence_id`, `content_hash`, `reliability` |
| `epistemic.promotion.reject` | `claim_id`, `from`, `to`, `reason` |
| `epistemic.status.transition` | `subject`, `from_status`, `to_status`, `evidence_id`, `reason` |
| `epistemic.assumption.create` | `assumption_id`, `consequence_if_false`, `verification_cost` |
| `epistemic.assumption.priority` | `assumption_id`, `p_false`, `risk`, `decision_dependence`, `verification_cost`, `priority` |
| `epistemic.contradiction.detect` | `contradiction_id`, `claim_a`, `claim_b`, `reason` |
| `epistemic.dependency.edge` | `dependent`, `dependency`, `kind`, `criticality` |
| `epistemic.dependency.cascade` | `root`, `affected_ids`, `count` |
| `epistemic.validation.fail` | `candidate_id`, `violations`, `cause_chain` |
| `epistemic.world.query` | `query`, `result_count`, `graphs` |

Redaction applies as always; proposition free-text is length-bounded.

---

## 9. Testing plan

| Test | Type | Assertion |
|---|---|---|
| `tests/promotion_guard.rs` | table-driven unit | every forbidden transition returns `Err(PromotionDenied)`; only `Observed`/`Verified` reach `/world` |
| `tests/contradiction.rs` | fixture integration | each seeded adversarial pair yields exactly one `Contradiction` record with both sides + evidence |
| `tests/dependency.rs` | property (`proptest`) | graph is acyclic after any accepted `add`; `cascade_invalidate` marks exactly the transitive dependents |
| `tests/verification_priority.rs` | golden | matches `bench/golden/verification_priority.csv` within `1e-9` |
| `tests/rdf_roundtrip.rs` | property | encode→decode→encode has identical content hash |
| `tests/replay.rs` | integration | replaying the event log reproduces statuses, contradictions, and cascade results byte-identically |
| `tests/world_separation.rs` | integration | SPARQL over `/world` returns only `OBSERVED`/`VERIFIED`; injecting an `ASSUMED` fact is rejected |
| `tests/temporal.rs` | unit | bitemporal validity honored: a claim valid only in the past is excluded from current queries |
| SHACL | conformance | `mm-cli graph validate --graph epistemic` → 0 violations |
| Logging | integration | `mm-cli logs verify` passes; every transition has exactly one audit record |

---

## 10. Pass gate

Run each as a plain command; the phase passes only if **all** exit 0 and the earlier phases' gates still pass.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mm-epistemic

# Forbidden promotions are all rejected by deterministic code (0 allowed).
cargo run -p mm-cli -- epistemic promotable \
    --fixture bench/epistemic/promotion_guard/forbidden.jsonl
# expected: "allowed=0 rejected=<N>", exit 0

# Contradictions produce explicit records, never silent reconciliation.
cargo run -p mm-cli -- epistemic contradictions --fixture bench/adversarial/contradictions/
# expected: created == seeded pairs, exit 0

# Leaf invalidation cascades to every transitive dependent.
cargo run -p mm-cli -- epistemic invalidate <ULID> --reason "source retracted"
# expected: affected set == bench/epistemic/cascade/expected.json

# VerificationPriority matches the golden table.
cargo test -p mm-epistemic --test verification_priority

# World graph holds only OBSERVED|VERIFIED.
cargo run -p mm-cli -- epistemic world --query 'SELECT ?s WHERE { GRAPH <https://metamind.dev/graph/world> { ?s <https://metamind.dev/ontology#status> ?st } }'
# expected: every ?st ∈ {OBSERVED, VERIFIED}

cargo run -p mm-cli -- graph validate --graph epistemic   # 0 SHACL violations
cargo run -p mm-cli -- logs verify                        # schema + audit completeness + redaction
cargo run -p mm-cli -- replay --check                     # deterministic replay
```

**Pass criteria:** `allowed=0` on the forbidden-promotion fixture; one contradiction record per seeded
pair; cascade set equals the golden expectation; `verification_priority` exact; `/world` contains only
`OBSERVED|VERIFIED`; 0 SHACL violations; `logs verify` green; replay identical.

---

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Over-modeling epistemic machinery the LLM could supply | Keep the layer to typed status + guard + ledger + DAG; no ontology of concepts (design §108) |
| Status inflation (everything becomes `VERIFIED`) | Promotion requires evidence satisfying `can_promote`; `/world` writer is the only path and is audited |
| Pairwise contradiction detection is O(n²) | Index by `(subject, predicate)` buckets; only same-subject/predicate candidates are compared |
| Contradiction spam / false positives | Contradictions are explicit records with status; materiality is scored in Phase 9, not here |
| World/epistemic leakage | Named-graph separation + `world_separation.rs` test + audited writer |
| Nondeterminism from float priority | Fixed formula, `f64`, golden CSV with tolerance; no model input to the arithmetic |
| Cascade performance at scale | DAG stored once; transitive set computed iteratively, one event per node |

---

## 12. Design traceability

- **`digital_mind_design1.md`** §18 (common sense/sanity primitives), §19 (bad-idea detection),
  §20 (comparison integrity — contract consumed in Phase 9), §21 (supposition & epistemic control:
  statuses, assumption ledger, `VerificationPriority`), §22 (dependency & uncertainty propagation),
  §23 (contradiction detection), §24 (missing-step detection — dependency prerequisites), §25
  (constraints & feasibility inputs), §28 (prediction ledger), §36–38 (imagination/causal/counterfactual
  labelling), §51 (world model vs model state), §52 (external state is authoritative), §87–90
  (RDF/OWL layer, runtime model, typed ops, backend mapping), §108–110 (what not to overbuild; the
  single invariant).
- **`sanity_layer1.md`** §2–§10 (assumption checks, constraint detection, dependency awareness,
  missing-step), §14 (decomposed confidence), §16 ("what would change my mind"), §21 (dumb-idea firewall
  inputs).
- **`congitive_elements3.md`** §4 (counterfactual labelling), §5 (predictive processing / prediction
  error), §19 (self-other boundary: owner/source/perspective on every belief), §21 (error taxonomy),
  §25 (structured internal events such as `BeliefContradictionDetected`).
- **`congitive_elements4.md`** §3 (memory beliefs with confidence/evidence/contradictions), §4 (REAL →
  FICTIONAL epistemic status as a hard primitive), §5 (epistemic budget: `ErrorCost × Uncertainty −
  VerificationCost`), §6 (prediction ledger), §9 (autobiographical timeline), §16 (identity invariants).
- **Parent plan** §13 Phase 6 (deliverables/steps/gate), §4 (ULID/URI), §5 (stores/named graphs),
  §6 (ontology/SHACL), §7 (copy-in integration), §8 (module contract), §9–§10 (testing/logging).
