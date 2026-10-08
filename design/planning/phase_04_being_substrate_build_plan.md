# Phase 4 Build Plan — Being substrate: identity, personality, affect, user, relationships, goals

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 4 · Codename Metamind (`mm`)

---

## 1. Objective and scope

Deliver the **persistent being substrate**: the state that survives model changes, context-window
boundaries, and conversations. It is deliberately small (design `state_structure1.md`: *store only what
losing would materially change the being's future behavior*).

**In scope**
- `mm-being` crate: `Identity` + immutable `Invariant` enforcement (deterministic).
- `Personality` as a small structured core (traits + values + contextual modifiers), **not** a giant
  prompt or an enumerated behavior matrix.
- `Affect` as a two-state model (stable internal baseline + short-lived expression impulses) that
  influences *presentation*, never factual reasoning.
- `UserModel` as probabilistic `Belief<T>` with explicit `EpistemicStatus`; **no auto-promotion to fact.**
- `RelationshipState` as a multi-dimensional, event-sourced object.
- `Goal` / `Commitment` with append-only lifecycle; intrinsic motivations.
- `ResourceState` + `CognitiveBudget` with **deterministic** enforcement (LLM never sets spending).
- SQLite tables + Oxigraph `/being` mirror + ontology classes + SHACL shapes.
- `mm-cli being …` commands, logging records, tests, and the Phase 4 pass gate.

**Out of scope** — memory classes and consolidation (Phase 5); claims/evidence/assumption ledger
(Phase 6); decisions and the sanity firewall (Phase 9); self-model divergence Actual/Model/Ideal (Phase 12).
Per design §108, do not hand-encode every personality behavior; keep the core tiny and let the LLM
elaborate it.

---

## 2. Prerequisites and dependencies

| Requires | Why |
|---|---|
| Phase 1 (`mm-core` IDs/timestamps, event log, `mm-store-sqlite`, `mm-store-graph` actor, `ontology v0`, `mm-log`) | All persistence, ULIDs, logging, SHACL |
| Phase 2 (`mm-codex`) | `mm-being` registered as a module with a stable URI; `mmc:copiedFrom` provenance for copied files |
| Phase 3 (`mm-llm`) | Only for optional trait/personality elaboration; the substrate must work without any LLM call |

Reference components copied in (per parent plan §7): `rdf-codec` (`ToRdf`/`FromRdf`, `Namespace`,
`new_ulid`), `provenance-ir` (`EpistemicStatus`, `KnowledgeClaim`), `epistemic-ir` (`BeliefState`),
`temporal-store` semantics (validity intervals + replay), `nexus-abi`/`nexus-core` plugin patterns.

---

## 3. Deliverables (exact paths)

```
crates/mm-being/
├── Cargo.toml
├── src/lib.rs                 # re-exports, BeingFacade
├── src/identity.rs            # Identity, SelfVersion, VersionId
├── src/invariants.rs          # Invariant, InvariantKind, BeingOp, IdentityGuard
├── src/personality.rs         # Personality, Trait, Values, contextual modifiers
├── src/affect.rs              # Affect, AffectImpulse, appraise(), decay()
├── src/motivation.rs          # Motivation, Drive
├── src/user_model.rs          # UserModel, Belief<T>, EpistemicStatus
├── src/relationships.rs       # RelationshipState, RelationshipEvent
├── src/goals.rs               # Goal, Commitment, status transitions
├── src/resources.rs           # ResourceState, CognitiveBudget, Account, DebitReceipt
├── src/rdf.rs                 # ToRdf/FromRdf impls, mm-being named graph
├── src/error.rs               # BeingError, InvariantViolation, BudgetExceeded
└── tests/
    ├── identity_invariants.rs
    ├── belief_promotion.rs
    ├── affect_separation.rs
    ├── commitment_lifecycle.rs
    ├── budgets.rs
    └── rdf_roundtrip.rs

crates/mm-store-sqlite/migrations/0004_being.sql
crates/mm-cli/src/being.rs
modules/cognition/being/{plugin.toml,src/lib.rs,manual/module.md,tests/behavior.rs}
ontology/shapes/being.shacl.ttl
bench/being/{invariants.jsonl,belief_promotion.jsonl,budget_overspend.jsonl,commitment_lifecycle.jsonl}
```

The `mm-being` module's `plugin.toml` declares the stable URI
`https://metamind.dev/code/module/cognition/being`, `version = "0.1.0"`, `owned_by_phase = 4`,
`capability = "mm:BeingState"`, and T-Box functions `being.identity`, `being.personality`,
`being.beliefs`, `being.relationships`, `being.goals`, `being.resources`.

---

## 4. Data model and ontology deltas

### 4.1 SQLite (`0004_being.sql`) — exact DDL

```sql
CREATE TABLE identity (
  id                TEXT(26) PRIMARY KEY,
  created_ulid      TEXT(26) NOT NULL,
  lineage_json      TEXT     NOT NULL,
  current_version   TEXT     NOT NULL,
  schema_version    INTEGER  NOT NULL
);

CREATE TABLE invariants (
  id          TEXT(26) PRIMARY KEY,
  identity_id TEXT(26) NOT NULL REFERENCES identity(id),
  code        TEXT     NOT NULL UNIQUE,          -- e.g. no_fabricated_autobiography
  assertion   TEXT     NOT NULL,
  enforcement TEXT     NOT NULL CHECK (enforcement IN ('deterministic','symbolic')),
  created_ulid TEXT(26) NOT NULL
);

CREATE TABLE personality_traits (
  id          TEXT(26) PRIMARY KEY,
  trait_key   TEXT     NOT NULL UNIQUE,           -- warmth, directness, curiosity, ...
  baseline    REAL     NOT NULL,
  value       REAL     NOT NULL,
  confidence  REAL     NOT NULL,
  updated_ulid TEXT(26) NOT NULL
);

CREATE TABLE personality_values (
  value_key   TEXT PRIMARY KEY,                   -- helpfulness, honesty, curiosity, ...
  weight      REAL NOT NULL,
  constraint_kind TEXT NOT NULL CHECK (constraint_kind IN ('avoid','prefer'))
);

CREATE TABLE personality_modifiers (
  context_key TEXT PRIMARY KEY,                   -- friend, technical_review, conflict, ...
  deltas_json TEXT NOT NULL                       -- {"directness": 0.17, "humor": -0.04, ...}
);

CREATE TABLE affect_state (
  identity_id TEXT(26) PRIMARY KEY REFERENCES identity(id),
  valence     REAL NOT NULL,
  arousal     REAL NOT NULL,
  engagement  REAL NOT NULL,
  warmth      REAL NOT NULL,
  caution     REAL NOT NULL,
  curiosity   REAL NOT NULL,
  energy      REAL NOT NULL,
  updated_ulid TEXT(26) NOT NULL
);

CREATE TABLE affect_impulses (
  id          TEXT(26) PRIMARY KEY,
  emotion     TEXT     NOT NULL,                  -- mock_frustration, amusement, ...
  intensity   REAL     NOT NULL,
  decay       REAL     NOT NULL,
  affects_internal_state INTEGER NOT NULL DEFAULT 0,   -- invariant: always 0
  affects_reasoning      INTEGER NOT NULL DEFAULT 0,   -- invariant: always 0
  created_ulid TEXT(26) NOT NULL,
  expires_ulid TEXT(26)
);

CREATE TABLE user_beliefs (
  id          TEXT(26) PRIMARY KEY,
  user_id     TEXT(26) NOT NULL,
  proposition TEXT     NOT NULL,
  epistemic_status TEXT NOT NULL CHECK (epistemic_status IN
    ('OBSERVED','VERIFIED','REPORTED','INFERRED','ASSUMED','HYPOTHETICAL','PREDICTED','SIMULATED','FICTIONAL','UNKNOWN')),
  confidence  REAL     NOT NULL,
  evidence_json TEXT   NOT NULL DEFAULT '[]',
  valid_from_ulid  TEXT(26),
  valid_until_ulid TEXT(26),
  created_ulid TEXT(26) NOT NULL
);

CREATE TABLE relationships (
  id          TEXT(26) PRIMARY KEY,
  user_id     TEXT(26) NOT NULL UNIQUE,
  familiarity REAL NOT NULL, trust REAL NOT NULL, reciprocity REAL NOT NULL,
  openness    REAL NOT NULL, cooperation REAL NOT NULL, reliance REAL NOT NULL,
  recent_quality REAL NOT NULL,
  unresolved_json TEXT NOT NULL DEFAULT '[]',
  updated_ulid TEXT(26) NOT NULL
);

CREATE TABLE relationship_events (
  id TEXT(26) PRIMARY KEY, relationship_id TEXT(26) NOT NULL REFERENCES relationships(id),
  kind TEXT NOT NULL, delta_json TEXT NOT NULL, ref_ulid TEXT(26), created_ulid TEXT(26) NOT NULL
);

CREATE TABLE goals (
  id          TEXT(26) PRIMARY KEY,
  owner_id    TEXT(26) NOT NULL,
  description TEXT     NOT NULL,
  status      TEXT     NOT NULL CHECK (status IN ('active','pending','blocked','fulfilled','abandoned','superseded')),
  priority    REAL     NOT NULL,
  deadline_ulid TEXT(26),
  parent_id   TEXT(26) REFERENCES goals(id),
  evidence_json TEXT   NOT NULL DEFAULT '[]',
  origin      TEXT     NOT NULL CHECK (origin IN ('user','being','inferred')),
  created_ulid TEXT(26) NOT NULL
);

CREATE TABLE commitments (
  id          TEXT(26) PRIMARY KEY,
  goal_id     TEXT(26) REFERENCES goals(id),
  made_to     TEXT(26) NOT NULL,
  status      TEXT     NOT NULL CHECK (status IN ('active','fulfilled','revoked','superseded')),
  deadline_ulid TEXT(26),
  created_ulid TEXT(26) NOT NULL,
  terminal_ulid TEXT(26)
);

CREATE TABLE motivations (
  id TEXT(26) PRIMARY KEY, drive TEXT NOT NULL UNIQUE,   -- competence, curiosity, coherence, ...
  strength REAL NOT NULL, updated_ulid TEXT(26) NOT NULL
);

CREATE TABLE resource_accounts (
  kind TEXT PRIMARY KEY, balance REAL NOT NULL, unit TEXT NOT NULL, updated_ulid TEXT(26) NOT NULL
);

CREATE TABLE resource_ledger (
  id TEXT(26) PRIMARY KEY, kind TEXT NOT NULL, delta REAL NOT NULL, balance_after REAL NOT NULL,
  purpose TEXT NOT NULL, ref_ulid TEXT(26), created_ulid TEXT(26) NOT NULL
);

CREATE TABLE budget_policies (
  kind TEXT PRIMARY KEY, period TEXT NOT NULL, limit_amount REAL NOT NULL, hard INTEGER NOT NULL DEFAULT 1
);
```

Conventions (Phase 1): `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;`; all ids are lowercase ULIDs
(`TEXT(26)`); status/commitment transitions are **append-only** (never `UPDATE` a terminal row).

### 4.2 Ontology (`ontology/mm.ttl`) and SHACL (`ontology/shapes/being.shacl.ttl`)

Add classes: `mm:Being, mm:Identity, mm:Invariant, mm:PersonalityTrait, mm:PersonalityValue,
mm:Motivation, mm:AffectState, mm:AffectImpulse, mm:UserBelief, mm:Relationship, mm:Goal,
mm:Commitment, mm:ResourceAccount` and predicates: `mm:invariantOf, mm:traitValue, mm:valueWeight,
mm:motivationStrength, mm:beliefStatus, mm:heldBy, mm:about, mm:inRelationshipWith, mm:trusts,
mm:hasGoal, mm:commitmentFor, mm:madeTo, mm:originatesFrom, mm:budgetLimit, mm:ledgerEntry`.
Named graph: `https://metamind.dev/graph/being`. Instance IRIs are `<https://metamind.dev/data/{ulid}>`
via `rdf-codec`; all instance graphs are validated before commit.

---

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

```rust
// identity.rs
pub struct Identity { pub id: Ulid, pub created_at: Timestamp, pub lineage: Vec<SelfVersion>,
                      pub current_version: VersionId, pub invariants: Vec<InvariantId> }
pub struct Invariant { pub id: InvariantId, pub code: InvariantCode, pub assertion: String,
                       pub enforcement: Enforcement }            // Enforcement: deterministic | symbolic

// invariants.rs — every mutation passes through the guard
pub enum BeingOp { FabricateAutobiography, PromoteAssumption, ClaimActionWithoutEvidence,
                   RewriteHistory, ExceedBudget { kind: ResourceKind }, ... }
pub trait IdentityGuard { fn check(&self, op: &BeingOp, ctx: &BeingCtx)
                              -> Result<(), InvariantViolation>; }
pub const CORE_INVARIANTS: [InvariantCode; 4];   // the four from design §4

// personality.rs
pub struct Personality { pub traits: BTreeMap<TraitKey, Trait>, pub values: BTreeMap<ValueKey, Value> }
pub struct Trait { pub baseline: f32, pub value: f32, pub confidence: f32 }
pub fn project(&self, ctx: &ContextVector) -> BehaviorParams;      // contextual deltas, clamped [0,1]

// affect.rs — two-state model; expression never mutates the baseline
pub struct Affect { pub valence: f32, pub arousal: f32, pub engagement: f32, pub warmth: f32,
                    pub caution: f32, pub curiosity: f32, pub energy: f32 }
pub struct AffectImpulse { pub emotion: Emotion, pub intensity: f32, pub decay: f32,
                           pub affects_internal_state: bool /* always false */,
                           pub affects_reasoning: bool /* always false */ }
pub fn appraise(event: &AppraisalEvent) -> AffectImpulse;
pub fn decay(a: &mut Affect, impulses: &mut Vec<AffectImpulse>);

// user_model.rs
pub struct Belief<T> { pub proposition: T, pub epistemic_status: EpistemicStatus, pub confidence: f32,
                       pub evidence: Vec<EvidenceId>, pub valid_from: Option<Timestamp>,
                       pub valid_until: Option<Timestamp> }
pub struct UserModel { pub user_id: Ulid, pub beliefs: BTreeMap<PropositionId, Belief<String>> }
impl UserModel { pub fn promote(&self, prop: PropositionId, to: EpistemicStatus, e: &[EvidenceId])
                     -> Result<(), PromotionDenied>; }

// relationships.rs
pub struct RelationshipState { pub familiarity: f32, pub trust: f32, pub reciprocity: f32,
    pub openness: f32, pub cooperation: f32, pub reliance: f32, pub recent_quality: f32,
    pub unresolved: Vec<IssueId> }

// goals.rs
pub enum GoalStatus { Active, Pending, Blocked, Fulfilled, Abandoned, Superseded }
pub struct Goal { pub id: GoalId, pub owner: Ulid, pub description: String, pub status: GoalStatus,
                  pub priority: f32, pub deadline: Option<Timestamp>, pub parent: Option<GoalId>,
                  pub evidence: Vec<EvidenceId>, pub origin: GoalOrigin }
pub struct Commitment { pub id: CommitmentId, pub goal: Option<GoalId>, pub made_to: Ulid,
                        pub status: CommitmentStatus, pub deadline: Option<Timestamp> }
impl Goal { pub fn transition(&mut self, next: GoalStatus, at: Timestamp) -> Result<(), TransitionDenied>; }

// resources.rs — deterministic arithmetic only
pub struct ResourceState { pub accounts: BTreeMap<ResourceKind, Account> }
impl ResourceState { pub fn debit(&mut self, kind: ResourceKind, amount: f64, purpose: &str,
                                  r: &mut dyn EventSink) -> Result<DebitReceipt, BudgetExceeded>; }

// lib.rs
pub struct BeingFacade { /* identity, personality, affect, users, relationships, goals, resources */ }
impl BeingFacade {
    pub fn open(stores: &Stores, sink: &EventSink) -> Result<Self, BeingError>;
    pub fn apply(&mut self, op: BeingOp) -> Result<OpReceipt, BeingError>;  // guarded
}
```

CLI (`crates/mm-cli/src/being.rs`): `mm-cli being show`, `being verify`, `being invariants`,
`being belief list|promote`, `being goal add|transition`, `being commitment add|transition`,
`being relationship show`, `being budget show`. `being verify` runs the invariant suite and prints the
SHACL violation count.

---

## 6. External references copied in and integration

| Reference | Copied into | How integrated |
|---|---|---|
| `rdf-codec` (`RdfContext`, `Namespace`, `ToRdf`/`FromRdf`, `new_ulid`) | `vendor/rdf-codec`, wrapped in `mm-being/src/rdf.rs` | canonical, deterministic RDF for every being type; ULID IRIs |
| `provenance-ir` (`EpistemicStatus`, `KnowledgeClaim`, source trust) | `vendor/provenance-ir` | `Belief<T>.epistemic_status` and promotion rules |
| `epistemic-ir` (`BeliefState`) | `vendor/epistemic-ir` | optional belief-update math for the user model |
| `temporal-store` semantics | `vendor/temporal-store` (patterns only) | `valid_from`/`valid_until` + event-log replay |
| nexus `plugin.toml` / T-Box conventions | `vendor/nexus-core` | `modules/cognition/being/plugin.toml` with stable `uri` |

Each copied file records `mmc:copiedFrom` (origin repo, path, revision, license) during the Phase 2 scan;
`vendor/<origin>/COPYING.md` is added. Adapted behind `mm-*` traits. No submodules or path dependencies.

---

## 7. Step-by-step implementation tasks

1. **Scaffold** `crates/mm-being` (Cargo.toml, `#![forbid(unsafe_code)]`, clippy deny) and add it to the
   workspace; register the `modules/cognition/being` plugin manifest. *Check:* `cargo build -p mm-being`.
2. **Identity + invariants** (`identity.rs`, `invariants.rs`): create identity on first open; load the four
   core invariants; implement `IdentityGuard::check` for every `BeingOp`. *Check:* `identity_invariants.rs`.
3. **Migration `0004_being.sql`**: all tables in §4.1 with FKs and CHECK constraints; up/down test.
4. **Personality** (`personality.rs`): small trait core, values, contextual modifiers, `project()`;
   persist + mirror to `/being`. *Check:* unit tests for clamp/interpolation.
5. **Affect** (`affect.rs`): baseline + `AffectImpulse` with `affects_internal_state=false` and
   `affects_reasoning=false` enforced by construction; `appraise()` and exponential `decay()`.
6. **Motivations** (`motivation.rs`): the eight intrinsic drives from `congitive_elements1.md`.
7. **User model** (`user_model.rs`): `Belief<T>` with status lattice; `promote()` requires satisfying
   evidence and never crosses into `OBSERVED` without an observation. *Check:* `belief_promotion.rs`.
8. **Relationships** (`relationships.rs`): multi-dimensional state; event-sourced updates with deltas.
9. **Goals + commitments** (`goals.rs`): append-only lifecycle; no mutation of terminal rows.
10. **Resources + budgets** (`resources.rs`): accounts, ledger, hard/soft policies; `debit()` refuses to
    go negative for hard budgets; every debit writes an audit event. *Check:* `budgets.rs`.
11. **RDF mirroring** (`rdf.rs`): `ToRdf`/`FromRdf` for every type; write to `/being`; SHACL shapes.
12. **Facade + guard wiring** (`lib.rs`): all mutations route through `BeingFacade::apply` → guard → store.
13. **CLI** (`mm-cli/src/being.rs`): the §5 subcommands; `being verify` aggregates invariant + SHACL checks.
14. **Fixtures + bench corpus**: `bench/being/*.jsonl` (invariant bypass attempts, forbidden promotions,
    budget overspend, commitment lifecycle).
15. **Wire event-log records** (§8) and confirm `mm-cli logs verify` passes for being operations.

---

## 8. Detailed logging requirements

All output via `mm-log` (parent plan §10). Event codes and required fields:

| Event code | Level | Required fields |
|---|---|---|
| `being.identity.init` | INFO | `identity_ulid`, `schema_version`, `invariant_codes` |
| `being.invariant.check` | DEBUG | `op`, `result`, `trace_id` |
| `being.invariant.violation` | ERROR | `op`, `invariant_code`, `reason`, `trace_id` |
| `being.personality.update` | INFO | `trait_key`, `old`, `new`, `confidence` |
| `being.affect.impulse` | DEBUG | `emotion`, `intensity`, `decay`, `affects_internal_state`, `affects_reasoning` |
| `being.belief.update` | INFO | `proposition_hash`, `old_status`, `new_status`, `old_conf`, `new_conf`, `evidence_ids` |
| `being.belief.promotion_denied` | WARN | `proposition_hash`, `requested_status`, `reason` |
| `being.relationship.update` | INFO | `user_ulid`, `dims_changed` (old→new), `ref_ulid` |
| `being.goal.transition` | INFO | `goal_ulid`, `old_status`, `new_status`, `origin` |
| `being.commitment.transition` | INFO | `commitment_ulid`, `old_status`, `new_status` |
| `being.budget.debit` | INFO | `kind`, `amount`, `balance_after`, `purpose`, `ref_ulid` |
| `being.budget.exceeded` | WARN | `kind`, `requested`, `balance`, `policy` |
| `being.persist` / `being.rdf.mirror` | DEBUG | `table`/`graph`, `ulid`, `triple_count` |

**Audit records** (immutable, in the event log): identity creation, every invariant violation attempt,
every belief status change, every goal/commitment transition, every budget debit. `trace_id` is the ULID
of the operation; a gap in the audit sequence is a hard error.

---

## 9. Testing plan

| Type | Tests |
|---|---|
| Unit | trait `project()` clamping; affect `decay()` sequences; `Belief` status lattice; `Goal::transition` legality |
| Property (`proptest`) | belief status never moves to a weaker-evidence status; commitment transitions are append-only; `ResourceState` balance for hard budgets is never negative; RDF round-trip canonicalization |
| Golden/replay | a scripted 50-event being session replayed from the event log yields byte-identical SQLite + `/being` content |
| Ontology | `mm-cli graph validate --graph being` → 0 SHACL violations; broken fixture → ≥1 |
| Adversarial | `bench/being/invariants.jsonl` and `belief_promotion.jsonl`: every forbidden op/promotion is rejected deterministically |
| Integration | facade `apply()` routes through the guard; concurrency test: two writers cannot over-debit a hard budget |
| E2E (`mm-cli`) | `being verify` exits 0; `being goal add` + `transition` reflected in RDF; subprocess test harness |

---

## 10. Pass gate

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mm-being --test identity_invariants --test belief_promotion \
                  --test affect_separation --test commitment_lifecycle \
                  --test budgets --test rdf_roundtrip
cargo run -p mm-cli -- being verify            # exits 0; prints 0 invariant violations
cargo run -p mm-cli -- graph validate --graph being   # 0 SHACL violations
cargo run -p mm-cli -- logs verify             # §10 logging gate
```

Objective criteria (all must hold):
- Each of the four core identity invariants cannot be violated through the public API — the adversarial
  corpus of forbidden ops produces `InvariantViolation` for 100% of cases.
- A belief enters as `REPORTED`/`INFERRED`/`ASSUMED` and **cannot** become `OBSERVED` without an evidence
  record satisfying the promotion rule; every fixture promotion is either allowed **with evidence** or
  denied, never silently coerced.
- Commitments/goals are monotonic: terminal rows are never updated; transitions append new rows.
- A hard-budget overspend is rejected; `resource_ledger` reconciles exactly with `resource_accounts`
  balances (sum of deltas == balance).
- `being verify` exits 0 with 0 invariant violations and 0 SHACL violations.
- `mm-cli logs verify` passes: schema-valid records, gapless audit sequence, ULID correlation intact,
  redaction suite clean.
- Replay of the scripted session reproduces identical state; earlier phase gates (1–3) still pass.

---

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Over-modeling personality into a behavior matrix (violates §108) | Keep a small trait core + LLM elaboration (`state_structure1.md`); no enumerated behaviors |
| Simulated affect contaminating reasoning or long-term state | Enforce `affects_internal_state=false` and `affects_reasoning=false` by construction; `affect_separation.rs` |
| User-model beliefs silently promoted to facts | Promotion guard requires evidence; forbidden-promotion fixtures are a hard gate |
| Budget bypass or concurrent overspend | All debits route through `ResourceState::debit` with a transaction + guard; concurrency test |
| Identity discontinuity across restarts or model swaps | Identity row is immutable and append-only; version recorded; `being verify` checks integrity |
| Copied reference code drifting | `mmc:copiedFrom` + `COPYING.md`; re-copy is a reviewed change-set (Phase 11) |

---

## 12. Design traceability

| Design source | Section |
|---|---|
| `digital_mind_design1.md` | §3 Being Substrate, §4 Identity, §5 Personality, §6 Affect, §7 User Model, §8 Relationships, §9 Goals/Intentions/Commitments/Desires, §10 Planning (goal inputs) |
| `digital_mind_design1.md` | §41 Normative Reasoning and Values, §42 Resource Economy, §53 Conversation as Bootstrap Environment, §54 User Corrections as Development Signals |
| `digital_mind_design1.md` | §80 Formal Invariants, §81 Human Escalation, §82 Self-Assessment/Capability Map, §83 Prediction of Own Behavior |
| `digital_mind_design1.md` | §108 What Should Not Be Overbuilt, §109 Most Important Architectural Invariant, §110 Final System |
| `state_structure1.md` | minimal persistent state; generated vs persistent vs verified state; identity continuity |
| `personality_simulator1.md` | trait/role/motivation/state layers; behavior IR; personality as intermediate representation |
| `personality_simulator2.md` | two-state affect; stable positive baseline; expression impulses and decay; emotion must not affect reasoning |
| `personality_simulator3.md` | contextual profiles; context vectors; mode mixtures; relationship-conditioned behavior; behavioral vs linguistic dimensions |
| `congitive_elements1.md` | persistent self-model; intrinsic motivations; values; beliefs/uncertainty; attention/working memory context; social cognition; agency boundaries |
| Parent plan | §3 layout, §4 ULIDs/URIs, §5 stores, §6 ontology, §7 copy-in, §8 module contract, §9 testing, §10 logging, §13 Phase 4 gate |
