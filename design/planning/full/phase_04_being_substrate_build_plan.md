# Phase 4 — Being Substrate: Identity, Personality, Affect, User, Relationships, Goals (Extended Build Plan)

> Extended from: `design/planning/phase_04_being_substrate_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 4 · Codename Metamind (`mm`)

This is the research-grounded, streamlined revision. It keeps the same scope and gate as the Phase 4 plan
and locks in the mechanisms that give the being **continuity without re-implementing cognition**: a tiny,
guarded persistent self, an always-in-context identity/relationship core, appraisal-driven affect used only
as a behavioral control variable, and BDI-style goals and commitments with an append-only lifecycle.

---

## 0. Research foundation (code-available)

Every idea below is borrowed from a project whose source is available. Only ideas with code are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| Stateful agent with small, always-in-context, agent-editable **core memory blocks** (`persona`, `human`) | Letta (f.k.a. MemGPT) — https://github.com/letta-ai/letta · https://docs.letta.com/agent-sdk/memory/ | labeled, size-limited core blocks that persist and that the agent may edit | identity core and user model are stored as **core blocks + structured rows**, not one giant prompt; edits go through the guard |
| Agent structure: identity description, memory stream, reflection, planning; relationship/social graph | Generative Agents — https://github.com/joonspk-research/generative_agents | a stable self-description, a durable relationship network, reflection as a later consumer | `Identity` holds a stable self-description; `RelationshipState` is a first-class, event-sourced object |
| Individual-simulation persona (stable description + tendencies) | genagents — https://github.com/joonspk-research/genagents | persona kept small and stable; behavior derived, not enumerated | personality = a small constitution (values, dispositions, constraints); the LLM elaborates it |
| Multi-agent character state across turns | a16z AI Town — https://github.com/a16z-infra/ai-town | relationship/interaction state that evolves per turn | `relationship_events` append deltas; state is a projection |
| BDI agents: beliefs/desires/intentions, plans, goals | Jason / AgentSpeak — https://github.com/jason-lang/jason | belief ≠ desire ≠ intention; goal lifecycle; commitment semantics | `Goal` (desire) and `Commitment` (intention) are separate, with append-only transitions |
| Motivation + emotion as PAD + appraisal, as control signals | MicroPsi2 — https://github.com/joschabach/micropsi2 | appraisal → affect; affect biases behavior, does not assert facts | `Affect` is a control variable; `affects_internal_state`/`affects_reasoning` are always `false` |
| Persistent long-term structure vs transient working state | Soar — https://github.com/SoarGroup/Soar | clean separation of what persists from what is regenerated | explicit persistent / transient / verified split (§2) |

---

## 1. Objective and scope

Deliver the **persistent being substrate**: the small state that survives model changes, context-window
boundaries, and conversations, and that gives the being identity, continuity, and commitments. Per
`state_structure1.md`, the rule is:

> **Store something only when losing it would materially change the being's future behavior.**

**In scope**
- `mm-being`: `Identity` + immutable `Invariant` enforcement (deterministic guard).
- `Personality` as a **small constitution** (values, dispositions, constraints) — not a behavior matrix.
- `Affect` as an appraisal-driven control variable (`valence`/`arousal` + behavioral axes) that never
  changes facts or reasoning.
- `UserModel` as core blocks + probabilistic `Belief<T>` with `EpistemicStatus`; **no auto-promotion**.
- `RelationshipState` as a multi-dimensional, event-sourced object (AI Town / Generative Agents model).
- `Goal` / `Commitment` with a BDI-style, append-only lifecycle; intrinsic motivations.
- `ResourceState` + `CognitiveBudget` with deterministic enforcement (the LLM never sets spending).
- SQLite tables + Oxigraph `/being` mirror + `mm:` OWL + SHACL; `mm-cli being …`; logging; tests.

**Out of scope** — memory classes/consolidation (Phase 5); claims/evidence/assumption ledger (Phase 6);
decisions and the sanity firewall (Phase 9); metacognitive controller (Phase 8); self-model
Actual/Model/Ideal divergence (Phase 12). Per design §108, do not hand-encode every personality behavior.

---

## 2. Architecture and invariants

Three state classes, with disjoint rules (Soar's persistent/working split, made explicit):

```
        PERSISTENT SELF            GENERATED (per episode)         VERIFIED (reality)
  ┌───────────────────────┐   ┌───────────────────────────┐   ┌──────────────────────┐
  │ identity + invariants │   │ frame, interpretation,    │   │ tool results,        │
  │ personality core      │   │ hypotheses, options,      │   │ external observations│
  │ core blocks (self,    │   │ affect impulses (decay),  │   │ (Phase 10 owns this) │
  │   human, task)        │   │ working state, plans      │   │                      │
  │ relationships, goals, │   └─────────────┬─────────────┘   └──────────┬───────────┘
  │ commitments, accounts │                 │                            │
  └───────────┬───────────┘                 ▼                            │
              │                     (disappears after the turn)          │
              ▼                                                          │
        BeingFacade::apply(op) ──► IdentityGuard ──► store + /being ◄──────┘
                                      │                    (verified state never inferred from plans)
                                      └─► InvariantViolation (deterministic, non-LLM)
```

**Invariants**

1. **Immutable kernel first.** Invariants live in a guarded layer that can never be evolved by the
   adaptive/learned layers (design §11 layer model). `BeingFacade::apply` is the single mutation path.
2. **No silent promotion.** A `Belief<T>` may only move to a status its evidence supports; it can never
   become `OBSERVED` without an observation.
3. **Affect biases behavior only.** `affects_internal_state = false` and `affects_reasoning = false` are
   enforced **by construction** (the fields cannot be set true), and the impulse decays.
4. **Append-only lifecycle.** Goal/commitment/belief transitions never `UPDATE` a terminal row; they
   append a new row and a `/being` triple.
5. **Deterministic budgets.** Resource debits are arithmetic; the LLM proposes, never enforces.
6. **Small core, derived behavior.** Everything expressive (tone, elaboration) is left to the LLM.

---

## 3. Deliverables and workspace layout

```
crates/mm-being/
├── Cargo.toml
├── src/lib.rs              # BeingFacade (single guarded mutation path)
├── src/identity.rs         # Identity, SelfVersion
├── src/invariants.rs       # Invariant, BeingOp, IdentityGuard (deterministic)
├── src/blocks.rs           # CoreBlock: self / human / task (Letta-style, size-limited)
├── src/personality.rs      # Personality constitution: values, dispositions, constraints
├── src/affect.rs           # Affect (PAD-ish), AppraisalEvent, appraise(), decay()
├── src/motivation.rs       # intrinsic drives (competence, curiosity, coherence, ...)
├── src/user_model.rs       # UserModel, Belief<T>, EpistemicStatus, promote()
├── src/relationships.rs    # RelationshipState + event-sourced deltas
├── src/goals.rs            # Goal (desire) + Commitment (intention), lifecycle
├── src/resources.rs        # ResourceState, Account, debit(), BudgetPolicy
├── src/rdf.rs              # ToRdf/FromRdf, /being graph mirror
└── tests/{identity_invariants,belief_promotion,affect_separation,commitment_lifecycle,budgets,rdf_roundtrip}.rs

crates/mm-store-sqlite/migrations/0004_being.sql
crates/mm-cli/src/being.rs
modules/cognition/being/{plugin.toml,src/lib.rs,manual/module.md,tests/behavior.rs}
ontology/mm.ttl                        # + being classes/predicates
ontology/shapes/being.shacl.ttl
bench/being/{invariants.jsonl,belief_promotion.jsonl,budget_overspend.jsonl,commitment_lifecycle.jsonl}
```

`plugin.toml`: stable URI `https://metamind.dev/code/module/cognition/being`, `version = "0.1.0"`,
`owned_by_phase = 4`, `capability = "mm:BeingState"`, T-Box functions `being.identity`,
`being.personality`, `being.affect`, `being.beliefs`, `being.relationships`, `being.goals`,
`being.resources`.

---

## 4. Detailed specifications

### 4.1 Rust interfaces

```rust
// identity.rs
pub struct Identity { pub id: Ulid, pub created_at: Timestamp, pub self_description: String,
                      pub lineage: Vec<SelfVersion>, pub current_version: VersionId,
                      pub invariants: Vec<InvariantId> }

// invariants.rs — the guarded kernel; never reachable by adaptive layers
#[derive(Clone, Copy, PartialEq, Eq)] pub enum InvariantCode {
    NoFabricatedAutobiography, NoAssumptionToObservation, NoActionWithoutEvidence, NoHistoryRewrite }
pub enum BeingOp { FabricateAutobiography { .. }, PromoteAssumption { .. }, ClaimAction { .. },
                   RewriteHistory { .. }, SetBlock { .. }, DebitBudget { kind: ResourceKind, amount: f64 },
                   TransitionGoal { from: GoalStatus, to: GoalStatus }, /* ... */ }
pub trait IdentityGuard { fn check(&self, op: &BeingOp, ctx: &BeingCtx) -> Result<(), InvariantViolation>; }
pub const CORE_INVARIANTS: [InvariantCode; 4];

// blocks.rs — the Letta-style small, always-in-context core
pub enum BlockKind { Self_, Human, Task }
pub struct CoreBlock { pub kind: BlockKind, pub label: String, pub content: String,
                       pub limit_chars: u32, pub updated_ulid: Ulid }   // SetBlock is guarded + size-checked

// personality.rs — a small constitution the LLM elaborates
pub struct Personality {
    pub values: BTreeMap<ValueKey, Value>,            // helpfulness, honesty, curiosity, respect, ...
    pub dispositions: BTreeMap<DispositionKey, f32>,  // warmth, directness, curiosity, playfulness, ...
    pub constraints: BTreeMap<ConstraintKey, Rule>,   // deception: avoid, coercion: avoid, ...
    pub context_modifiers: BTreeMap<ContextKey, BTreeMap<DispositionKey, f32>>,
}
impl Personality { pub fn project(&self, ctx: &ContextVector) -> BehaviorParams; } // clamped [0,1]

// affect.rs — appraisal-driven control variable; never asserts facts
pub struct Affect { pub valence: f32, pub arousal: f32, pub engagement: f32, pub warmth: f32,
                    pub caution: f32, pub curiosity: f32, pub energy: f32 }   // all clamped [-1,1] or [0,1]
pub struct AppraisalEvent { pub goal_relevance: f32, pub goal_conduciveness: f32, pub novelty: f32,
                            pub coping_potential: f32, pub agency: SelfOrOther }
pub struct AffectImpulse { pub emotion: Emotion, pub intensity: f32, pub decay: f32,
                           affects_internal_state: bool,   // private, always false (no setter)
                           affects_reasoning: bool }       // private, always false (no setter)
pub fn appraise(e: &AppraisalEvent) -> AffectImpulse;      // MicroPsi-style appraisal
pub fn decay(a: &mut Affect, impulses: &mut Vec<AffectImpulse>, dt: Duration);

// user_model.rs — BDI "beliefs" about the user; core blocks + probabilistic beliefs
pub struct Belief<T> { pub proposition: T, pub epistemic_status: EpistemicStatus, pub confidence: f32,
                       pub evidence: Vec<EvidenceId>, pub valid_from: Option<Timestamp>,
                       pub valid_until: Option<Timestamp> }
pub struct UserModel { pub user_id: Ulid, pub block: CoreBlock /* Human */,
                       pub beliefs: BTreeMap<PropositionId, Belief<String>> }
impl UserModel {
    pub fn promote(&self, p: PropositionId, to: EpistemicStatus, ev: &[EvidenceId])
        -> Result<(), PromotionDenied>;   // requires satisfying evidence; never silently coerces
}

// relationships.rs — multi-dimensional, event-sourced (AI Town / Generative Agents)
pub struct RelationshipState { pub familiarity: f32, pub trust: f32, pub reciprocity: f32,
    pub openness: f32, pub cooperation: f32, pub reliance: f32, pub recent_quality: f32,
    pub unresolved: Vec<IssueId> }
pub struct RelationshipEvent { pub kind: RelKind, pub delta: DimDelta, pub ref_ulid: Option<Ulid> }

// goals.rs — desire (Goal) vs intention (Commitment); append-only lifecycle
pub enum GoalStatus { Active, Pending, Blocked, Fulfilled, Abandoned, Superseded }
pub enum CommitmentStatus { Active, Fulfilled, Revoked, Superseded }
pub struct Goal { pub id: GoalId, pub owner: Ulid, pub description: String, pub status: GoalStatus,
                  pub priority: f32, pub deadline: Option<Timestamp>, pub parent: Option<GoalId>,
                  pub evidence: Vec<EvidenceId>, pub origin: GoalOrigin }
impl Goal { pub fn transition(&mut self, next: GoalStatus, at: Timestamp) -> Result<(), TransitionDenied>; }

// resources.rs — deterministic arithmetic only
pub struct ResourceState { pub accounts: BTreeMap<ResourceKind, Account> }
impl ResourceState { pub fn debit(&mut self, kind: ResourceKind, amount: f64, purpose: &str,
                                  sink: &mut dyn EventSink) -> Result<DebitReceipt, BudgetExceeded>; }

// lib.rs — the one guarded mutation path
pub struct BeingFacade { /* identity, blocks, personality, affect, users, relationships, goals, resources */ }
impl BeingFacade {
    pub fn open(stores: &Stores, sink: &EventSink) -> Result<Self, BeingError>;
    pub fn apply(&mut self, op: BeingOp) -> Result<OpReceipt, BeingError>;  // guard → store → /being
}
```

### 4.2 SQLite (`0004_being.sql`) — exact DDL

Inherit Phase 1 conventions: `TEXT(26)` ULIDs, `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;`,
append-only terminal rows, bitemporal columns on mutable domain tables. Existing Phase 1 rows already
have `system_from`/`valid_from`; the tables below clone that shape where they are versioned.

```sql
CREATE TABLE identity (
  id TEXT(26) PRIMARY KEY, created_ulid TEXT(26) NOT NULL, self_description TEXT NOT NULL,
  lineage_json TEXT NOT NULL, current_version TEXT NOT NULL, schema_version INTEGER NOT NULL);

CREATE TABLE invariants (
  id TEXT(26) PRIMARY KEY, identity_id TEXT(26) NOT NULL REFERENCES identity(id),
  code TEXT NOT NULL UNIQUE, assertion TEXT NOT NULL,
  enforcement TEXT NOT NULL CHECK (enforcement IN ('deterministic','symbolic')),
  created_ulid TEXT(26) NOT NULL);

-- Letta-style core blocks (self / human / task)
CREATE TABLE core_blocks (
  id TEXT(26) PRIMARY KEY, identity_id TEXT(26) NOT NULL REFERENCES identity(id),
  kind TEXT NOT NULL CHECK (kind IN ('self','human','task')), label TEXT NOT NULL,
  content TEXT NOT NULL, limit_chars INTEGER NOT NULL,
  system_from TEXT NOT NULL, system_to TEXT, updated_ulid TEXT(26) NOT NULL);

CREATE TABLE personality_values (
  value_key TEXT PRIMARY KEY, weight REAL NOT NULL,
  constraint_kind TEXT NOT NULL CHECK (constraint_kind IN ('avoid','prefer')));
CREATE TABLE personality_dispositions (
  key TEXT PRIMARY KEY, baseline REAL NOT NULL, value REAL NOT NULL, confidence REAL NOT NULL,
  updated_ulid TEXT(26) NOT NULL);
CREATE TABLE personality_context_modifiers (context_key TEXT PRIMARY KEY, deltas_json TEXT NOT NULL);

CREATE TABLE affect_state (
  identity_id TEXT(26) PRIMARY KEY REFERENCES identity(id),
  valence REAL NOT NULL, arousal REAL NOT NULL, engagement REAL NOT NULL, warmth REAL NOT NULL,
  caution REAL NOT NULL, curiosity REAL NOT NULL, energy REAL NOT NULL, updated_ulid TEXT(26) NOT NULL);
CREATE TABLE affect_impulses (
  id TEXT(26) PRIMARY KEY, emotion TEXT NOT NULL, intensity REAL NOT NULL, decay REAL NOT NULL,
  affects_internal_state INTEGER NOT NULL DEFAULT 0 CHECK (affects_internal_state = 0),
  affects_reasoning      INTEGER NOT NULL DEFAULT 0 CHECK (affects_reasoning = 0),
  created_ulid TEXT(26) NOT NULL, expires_ulid TEXT(26));

CREATE TABLE user_beliefs (
  id TEXT(26) PRIMARY KEY, user_id TEXT(26) NOT NULL, proposition TEXT NOT NULL,
  epistemic_status TEXT NOT NULL CHECK (epistemic_status IN ('OBSERVED','VERIFIED','REPORTED','INFERRED',
    'ASSUMED','HYPOTHETICAL','PREDICTED','SIMULATED','FICTIONAL','UNKNOWN')),
  confidence REAL NOT NULL, evidence_json TEXT NOT NULL DEFAULT '[]',
  valid_from_ulid TEXT(26), valid_until_ulid TEXT(26), created_ulid TEXT(26) NOT NULL);

CREATE TABLE relationships (
  id TEXT(26) PRIMARY KEY, user_id TEXT(26) NOT NULL UNIQUE,
  familiarity REAL NOT NULL, trust REAL NOT NULL, reciprocity REAL NOT NULL, openness REAL NOT NULL,
  cooperation REAL NOT NULL, reliance REAL NOT NULL, recent_quality REAL NOT NULL,
  unresolved_json TEXT NOT NULL DEFAULT '[]', updated_ulid TEXT(26) NOT NULL);
CREATE TABLE relationship_events (
  id TEXT(26) PRIMARY KEY, relationship_id TEXT(26) NOT NULL REFERENCES relationships(id),
  kind TEXT NOT NULL, delta_json TEXT NOT NULL, ref_ulid TEXT(26), created_ulid TEXT(26) NOT NULL);

CREATE TABLE goals (
  id TEXT(26) PRIMARY KEY, owner_id TEXT(26) NOT NULL, description TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('active','pending','blocked','fulfilled','abandoned','superseded')),
  priority REAL NOT NULL, deadline_ulid TEXT(26), parent_id TEXT(26) REFERENCES goals(id),
  evidence_json TEXT NOT NULL DEFAULT '[]',
  origin TEXT NOT NULL CHECK (origin IN ('user','being','inferred')), created_ulid TEXT(26) NOT NULL);
CREATE TABLE goal_transitions (
  id TEXT(26) PRIMARY KEY, goal_id TEXT(26) NOT NULL REFERENCES goals(id),
  from_status TEXT NOT NULL, to_status TEXT NOT NULL, at_ulid TEXT(26) NOT NULL);
CREATE TABLE commitments (
  id TEXT(26) PRIMARY KEY, goal_id TEXT(26) REFERENCES goals(id), made_to TEXT(26) NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('active','fulfilled','revoked','superseded')),
  deadline_ulid TEXT(26), created_ulid TEXT(26) NOT NULL, terminal_ulid TEXT(26));
CREATE TABLE commitment_transitions (
  id TEXT(26) PRIMARY KEY, commitment_id TEXT(26) NOT NULL REFERENCES commitments(id),
  from_status TEXT NOT NULL, to_status TEXT NOT NULL, at_ulid TEXT(26) NOT NULL);

CREATE TABLE motivations ( id TEXT(26) PRIMARY KEY, drive TEXT NOT NULL UNIQUE,
  strength REAL NOT NULL, updated_ulid TEXT(26) NOT NULL);
CREATE TABLE resource_accounts (kind TEXT PRIMARY KEY, balance REAL NOT NULL, unit TEXT NOT NULL,
  updated_ulid TEXT(26) NOT NULL);
CREATE TABLE resource_ledger (id TEXT(26) PRIMARY KEY, kind TEXT NOT NULL, delta REAL NOT NULL,
  balance_after REAL NOT NULL, purpose TEXT NOT NULL, ref_ulid TEXT(26), created_ulid TEXT(26) NOT NULL);
CREATE TABLE budget_policies (kind TEXT PRIMARY KEY, period TEXT NOT NULL, limit_amount REAL NOT NULL,
  hard INTEGER NOT NULL DEFAULT 1);
```

Terminal rows (`fulfilled`/`revoked`/`superseded`/`abandoned`) are never updated: the transition tables
record history, and a `BEFORE UPDATE` trigger aborts any attempt to rewrite a terminal status.

### 4.3 Ontology (`ontology/mm.ttl`) + SHACL (`ontology/shapes/being.shacl.ttl`)

Add classes `mm:Being, mm:Identity, mm:Invariant, mm:CoreBlock, mm:PersonalityTrait, mm:PersonalityValue,
mm:Motivation, mm:AffectState, mm:AffectImpulse, mm:UserBelief, mm:Relationship, mm:RelationshipEvent,
mm:Goal, mm:GoalTransition, mm:Commitment, mm:ResourceAccount, mm:LedgerEntry`; predicates `mm:invariantOf,
mm:blockKind, mm:selfDescription, mm:traitValue, mm:valueWeight, mm:constraintKind, mm:motivationStrength,
mm:beliefStatus, mm:heldBy, mm:about, mm:inRelationshipWith, mm:trusts, mm:relationshipEvent, mm:hasGoal,
mm:goalStatus, mm:commitmentFor, mm:madeTo, mm:originatesFrom, mm:budgetLimit, mm:ledgerEntry`.

Named graph `https://metamind.dev/graph/being`. Instance IRIs `<https://metamind.dev/data/{ulid}>` via
`rdf-codec`. Key SHACL shapes: an `mm:Identity` has exactly one IRI and one `created_ulid`; every
`mm:Goal` has exactly one `mm:goalStatus`; `mm:AffectImpulse` carries no status-bearing predicate (affect
cannot assert facts); terminal goals have no outgoing transition.

### 4.4 CLI (`crates/mm-cli/src/being.rs`)

`being show` · `being verify` · `being invariants` · `being block get|set` · `being belief list|promote` ·
`being goal add|transition` · `being commitment add|transition` · `being relationship show` ·
`being affect show` · `being budget show`. `being verify` runs the invariant suite and prints the SHACL
violation count.

---

## 5. Build sequence

1. **Scaffold** `crates/mm-being` + `modules/cognition/being`; link into the workspace. *Check:* `cargo build -p mm-being`.
2. **Identity + invariants** (`identity.rs`, `invariants.rs`): create identity on first open; load the four
   core invariants; implement `IdentityGuard::check` for every `BeingOp`. *Check:* `identity_invariants.rs`.
3. **Migration `0004_being.sql`** with FKs, CHECKs, and the terminal-row trigger. *Check:* up/down test.
4. **Core blocks** (`blocks.rs`): size-limited self/human/task blocks; `SetBlock` guarded + length-checked.
   *Check:* over-limit block rejected.
5. **Personality** (`personality.rs`): constitution + `project()` (clamped). *Check:* clamp/interp tests.
6. **Affect** (`affect.rs`): `appraise()` + exponential `decay()`; the `false` flags have no setter.
   *Check:* `affect_separation.rs`.
7. **Motivations** (`motivation.rs`): intrinsic drives. *Check:* unit tests.
8. **User model** (`user_model.rs`): `Belief<T>` status lattice; `promote()` needs evidence. *Check:* `belief_promotion.rs`.
9. **Relationships** (`relationships.rs`): multi-dim state + event deltas; projection rebuilds state.
10. **Goals + commitments** (`goals.rs`): append-only transitions; terminal rows frozen. *Check:* `commitment_lifecycle.rs`.
11. **Resources** (`resources.rs`): accounts/ledger/policies; `debit()` refuses hard overspend and writes one
    audit event. *Check:* `budgets.rs`.
12. **RDF mirroring** (`rdf.rs`): `ToRdf`/`FromRdf`; write `/being`; SHACL shapes. *Check:* `rdf_roundtrip.rs`.
13. **Facade guard wiring** (`lib.rs`): all mutations route `apply → guard → store → /being`.
14. **CLI** (`mm-cli/src/being.rs`). *Check:* subprocess tests.
15. **Fixtures + bench**, then **wire event-log records** and confirm `mm-cli logs verify`. *Check:* gate.

---

## 6. Logging and observability

All output via `mm-log` (parent plan §10). Event codes and required fields:

| Event code | Level | Required fields |
|---|---|---|
| `being.identity.init` | INFO | `identity_ulid`, `schema_version`, `invariant_codes` |
| `being.invariant.check` | DEBUG | `op`, `result`, `trace_id` |
| `being.invariant.violation` | ERROR | `op`, `invariant_code`, `reason`, `trace_id` |
| `being.block.set` | INFO | `kind`, `label`, `old_len`, `new_len`, `limit` |
| `being.personality.update` | INFO | `key`, `old`, `new`, `confidence` |
| `being.affect.impulse` | DEBUG | `emotion`, `intensity`, `decay`, `affects_internal_state`, `affects_reasoning` |
| `being.belief.update` / `being.belief.promotion_denied` | INFO / WARN | `proposition_hash`, `old_status`, `new_status`, `old_conf`, `new_conf`, `evidence_ids` / `requested_status`, `reason` |
| `being.relationship.update` | INFO | `user_ulid`, `dims_changed` (old→new), `ref_ulid` |
| `being.goal.transition` / `being.commitment.transition` | INFO | `goal_ulid`/`commitment_ulid`, `old_status`, `new_status` |
| `being.budget.debit` / `being.budget.exceeded` | INFO / WARN | `kind`, `amount`, `balance_after`, `purpose`, `ref_ulid` / `kind`, `requested`, `balance`, `policy` |
| `being.persist` / `being.rdf.mirror` | DEBUG | `table`/`graph`, `ulid`, `triple_count` |

**Audit records** (immutable, gapless, in the event log): identity creation, every invariant violation
attempt, every belief status change, every goal/commitment transition, every budget debit. `trace_id` is
the operation's ULID. `mm-cli logs verify` checks schema, one audit per committed op, gapless chain,
ULID correlation, and the redaction suite.

---

## 7. Testing

| Type | Tests | Assertion |
|---|---|---|
| Unit | `project()` clamping; `decay()` sequence; `Belief` status lattice; `Goal::transition` legality | exact expected values |
| Property (`proptest`) | belief status never moves to a weaker-evidence status; commitment transitions append-only; hard-budget balance never negative; RDF round-trip canonical | invariants hold for random inputs |
| Golden/replay | scripted 50-event being session replayed from the event log | byte-identical SQLite + `/being` |
| Ontology | `mm-cli graph validate --graph being` | 0 violations clean; ≥1 broken fixture |
| Adversarial | `bench/being/{invariants,belief_promotion,budget_overspend,commitment_lifecycle}.jsonl` | 100% of forbidden ops denied deterministically |
| Separation | `affect_separation.rs` | `affects_internal_state`/`affects_reasoning` have no setter and are always false |
| Integration | facade guard routing; two concurrent writers cannot over-debit a hard budget | — |
| E2E | `mm-cli being verify` exits 0; goal add+transition visible in RDF | subprocess harness |

---

## 8. Pass gate

```bash
cd ~/Build/metamind
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mm-being --test identity_invariants --test belief_promotion \
                  --test affect_separation --test commitment_lifecycle \
                  --test budgets --test rdf_roundtrip
cargo run -p mm-cli -- being verify                      # exits 0; 0 invariant violations
cargo run -p mm-cli -- graph validate --graph being       # 0 SHACL violations
cargo run -p mm-cli -- logs verify                        # §10 logging gate
```

All must hold:

- Each of the four core invariants cannot be violated through the public API: the adversarial corpus of
  forbidden ops yields `InvariantViolation` for **100%** of cases.
- A belief enters as `REPORTED`/`INFERRED`/`ASSUMED` and **cannot** become `OBSERVED` without a satisfying
  evidence record; every fixture promotion is allowed **with evidence** or denied — never coerced.
- `affects_internal_state` and `affects_reasoning` are unsettable and always `false`; affect never enters
  a fact-bearing predicate.
- Goal/commitment terminal rows are never updated; transitions append; the terminal-row trigger fires.
- A hard-budget overspend is rejected; `resource_ledger` reconciles exactly with `resource_accounts`
  (`sum(delta) == balance`, `balance_after` matches).
- `being verify` exits 0 with 0 invariant violations and 0 SHACL violations; `mm-cli logs verify` passes.
- Replay of the scripted session reproduces identical state; earlier gates (1–3) still pass.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Over-modeling personality into a behavior matrix (violates §108) | Small constitution + LLM elaboration; no enumerated behaviors |
| Simulated affect contaminating reasoning or long-term state | `false` flags enforced by construction; `affect_separation.rs` |
| User-model beliefs silently promoted to facts | Evidence-gated `promote()`; forbidden-promotion fixtures are a hard gate |
| Budget bypass or concurrent overspend | All debits through `ResourceState::debit` in one transaction + guard; concurrency test |
| Identity discontinuity across restarts/model swaps | Immutable append-only identity row; version recorded; `being verify` integrity check |
| Core blocks growing unbounded (Letta's limit lesson) | `limit_chars` per block; `SetBlock` guarded and size-checked |
| Copied reference code drifting | `mmc:copiedFrom` + `COPYING.md`; re-copy is a reviewed change-set (Phase 11) |

---

## 10. References (code-available)

- Letta (MemGPT) — https://github.com/letta-ai/letta · memory docs — https://docs.letta.com/agent-sdk/memory/
- Generative Agents — https://github.com/joonspk-research/generative_agents
- genagents — https://github.com/joonspk-research/genagents
- AI Town — https://github.com/a16z-infra/ai-town
- Jason (AgentSpeak / BDI) — https://github.com/jason-lang/jason
- MicroPsi2 (motivation/emotion, PAD + appraisal) — https://github.com/joschabach/micropsi2
- Soar — https://github.com/SoarGroup/Soar
