# Metamind — Implementation Plan (12 Phases)

> **Source design:** [`digital_mind_design1.md`](./digital_mind_design1.md)
> **Supporting designs:** `bootstrap_procses1.md`, `bootstrap_procses2.md`, `meta_analysis1/2.md`,
> `sanity_layer1.md`, `thought_systems1/2.md`, `congitive_elements1-4.md`, `state_structure1.md`,
> `conceptual_frames1.md`, `companion_loop1/2.md`, `personality_simulator1-3.md`.
> **Codename:** the running being is **Metamind** (`mm`). Crate prefix `mm-`, ontology prefix `mm:`.
> **Goal of this plan:** build the system in **12 gated phases**, from a deterministic kernel to a
> functional prototype that can **continue its own design and build process** with **Pi** as its code
> editor, driven by the cognitive runtime.

---

# 0. Review findings and how this plan responds

The design is unusually disciplined about *not* recreating the LLM. Its single organizing rule (§109) is:

> The LLM proposes cognition; the control plane structures it; Jev makes bounded judgments; symbolic
> systems prove what can be proven; deterministic systems enforce what must be enforced; the environment
> decides what is actually true; memory preserves what matters; meta-analysis determines how the whole
> system should improve.

The plan keeps that rule as its architecture invariant. Review surpluses and gaps we must close:

| Design says | Status | Plan response |
|---|---|---|
| Persistent substrate + LLM + cognitive library + metacognitive executive | Sound | Directly implemented across Phases 1, 3–8 |
| "PostgreSQL for authoritative state; RDF/OWL + graph store" (§3) | Under-specified for this environment | **SQLite** (tabular, WAL, `sqlx`) + **Oxigraph** (RDF, RocksDB) — user-mandated stores |
| Jev is the bounded-decision primitive (§47–49) | Depends on a hosted/again unavailable model | Abstract behind a `DecisionCore` trait with **3 interchangeable impls**: deterministic rules, local head, hosted. Calibration is mandatory from day one. |
| Self-modification must never overwrite production (§59) | Listed, not mechanized | Phase 11 adds an explicit **sandbox → change-set → benchmark → promotion** pipeline with rollback. |
| Bootstrap stages §92–102 (11 conceptual) | No gates, no pass criteria | Mapped onto **12 phases**, each with an objective pass gate. |
| "Meta-analysis of its own codebase" (§62, §85, §105) | Mentioned, no data model | Phase 2 adds a **code-metadata registry** with **stable URI RDF identifiers** in a dedicated named graph. |
| Code editing is assumed to be done by "the LLM" | No code-editor subsystem named | **Pi** (`@earendil-works/pi-coding-agent`) is the code editor, driven over its **JSONL RPC protocol** from the async Rust runtime. |
| "Modular code" | Implied by capability objects | Nexus **plugin module contract** (manifest with stable `uri`) is the unit of code; every module is registered in Phase 2. |
| ULIDs for IDs (§87, `rdf-codec` convention) | Consistent across utilities | Adopted verbatim: lowercase ULID Crockford strings; IRIs `<https://metamind.dev/data/{ulid}>`. |
| Strong testing at every phase | Implied | §9 testing strategy + per-phase pass gates; replay/determinism required from Phase 1. |

**Non-goals** (design §108): no giant symbolic common-sense base, no hand-coded personality matrix, no
premature giant ontology, not every thought persistent, no LLM arithmetic, no Jev prose, no symbolic NL
understanding, no dozen independent critics, no deterministic planner for every task, no untested
self-modification, model confidence never overrides authoritative external state.

---

# 1. Environment and prerequisites (verified on the build machine)

| Dependency | Verified | Notes |
|---|---|---|
| Rust | `rustc 1.96.1`, `cargo 1.96.1` | Edition 2021; `rust-toolchain.toml` pinned |
| SQLite | `sqlite3 3.46.1`, `libsqlite3.so` | WAL mode; `sqlx` 0.8 (`sqlite`, `runtime-tokio`) as used by `nexus` `monad-sql` |
| Oxigraph | `librocksdb.so.9.11` present | `oxigraph = "0.5"`, feature `rocksdb-pkg-config` (mandatory, per `rust_symbolic` build directive) |
| ONNX Runtime | `libonnxruntime.so.1.23` | For `kg-embed` BGE-small embeddings |
| Pi | `pi 0.85.1` at `~/.nvm/.../bin/pi` | `@earendil-works/pi-coding-agent`; provider `dojo_dev` (OpenAI-compatible, `http://bobs-mac.local:4000/v1`) |
| LLM provider | OpenAI-compatible endpoint (env `OPENAI_BASE_URL`, `OPENAI_API_KEY`) | Same endpoint Pi and the runtime use |

System packages already installed. The workspace must build with **zero warnings** and
`#![forbid(unsafe_code)]` in all `mm-*` crates (matching `kg-core`/`kg-embed` convention).

---

# 2. Architecture at a glance

```
                         ┌──────────────────────────────────────────────┐
                         │  mm-runtime (async tokio)  — the executive   │
                         │  episode loop · scheduler · budgets          │
                         └───────────────┬──────────────────────────────┘
        ┌────────────────────────────────┼─────────────────────────────────────┐
        ▼                ▼               ▼                ▼                     ▼
  ┌───────────┐   ┌───────────┐   ┌────────────┐   ┌───────────┐        ┌──────────────┐
  │ mm-being  │   │ mm-memory │   │ mm-epistemic│   │ mm-library│        │ mm-codex     │
  │ identity  │   │ classes + │   │ claims /    │   │ frames /  │        │ code module  │
  │ personality│  │ consolidate│  │ assumptions │   │ techniques│        │ metadata     │
  └─────┬─────┘   └─────┬─────┘   └──────┬─────┘   └─────┬─────┘        └──────┬───────┘
        │               │                │               │                     │
        └───────────────┴────────────────┴───────────────┴─────────────────────┘
                                        │
                              ┌─────────┴──────────┐
                              ▼                    ▼
                    ┌──────────────────┐  ┌──────────────────┐
                    │  mm-metacog      │  │  mm-firewall /   │
                    │  program compiler│  │  mm-decision     │
                    └────────┬─────────┘  └────────┬─────────┘
                             ▼                     ▼
                   ┌────────────────────────────────────────┐
                   │ mm-tools · mm-selfeng · mm-pi          │
                   │ typed ops · sandbox · promotion · Pi   │
                   └────────────────────────────────────────┘
                             │                         │
              ┌──────────────┴───────┐      ┌──────────┴───────────┐
              ▼                      ▼      ▼                      ▼
      ┌───────────────┐     ┌──────────────┐  ┌───────────────────────────┐
      │ SQLite (sqlx) │     │ Oxigraph     │  │ Pi subprocess (RPC, JSONL) │
      │ tabular state │     │ RDF graphs   │  │ read/bash/edit/write tools │
      └───────────────┘     └──────────────┘  └───────────────────────────┘
```

The one invariant enforced throughout: **LLM proposes; control plane structures; Jev decides bounded
questions; symbolic proves; deterministic enforces; environment observes; memory preserves;
meta-analysis improves.**

---

# 3. Workspace layout

`~/Build/metamind/` becomes a Cargo workspace; `design/` stays as-is.

```
~/Build/metamind/
├── Cargo.toml                 # [workspace] members = crates/* + modules/*/…
├── rust-toolchain.toml        # pinned stable
├── design/                    # source designs + this plan
├── crates/                    # the runtime, one responsibility per crate
│   ├── mm-core                # IDs (ULID), time, errors, config, shared traits
│   ├── mm-store-sqlite        # tabular store (sqlx + migrations)
│   ├── mm-store-graph         # Oxigraph actor + rdf-codec integration + SHACL
│   ├── mm-eventlog            # append-only bitemporal event log + replay
│   ├── mm-codex               # code-module metadata registry (stable URI RDF)
│   ├── mm-llm                 # LLM substrate (LlmClient adapters, cache, routing)
│   ├── mm-being               # identity, personality, affect, user, relations, goals
│   ├── mm-memory              # 5 memory classes, retrieval, consolidation, forgetting
│   ├── mm-epistemic           # claims, evidence, assumptions, predictions, dependency DAG
│   ├── mm-library             # cognitive library + conceptual frames + policy genome
│   ├── mm-metacog             # metacognitive scan + cognitive-program compiler
│   ├── mm-decision            # DecisionCore trait, risk, comparison integrity, calibration math
│   ├── mm-firewall            # unified sanity / epistemic firewall
│   ├── mm-tools               # typed CognitiveOps, tool registry, permissions, executor
│   ├── mm-metaanalysis        # prediction ledger, calibration, error taxonomy, mistake→test
│   ├── mm-selfeng             # change sets, sandbox, benchmark, shadow, promotion, rollback
│   ├── mm-pi                  # Pi RPC client + session ingester
│   ├── mm-runtime             # the async cognitive runtime (main binary)
│   └── mm-cli                 # operator CLI (doctor, replay, codex, promote, ask, inspect)
├── modules/                   # THE MODULAR CODE SYSTEM — nexus-style plugin modules
│   ├── cognition/<name>/      # plugin.toml + src/ + tests/ + manual/
│   ├── tools/<name>/
│   ├── io/<name>/
│   └── registry.json          # generated index of modules (see Phase 2)
├── ontology/
│   ├── mm.ttl                 # Metamind T-Box (being, memory, epistemic, library, code)
│   ├── code.ttl               # mmc: code-metadata vocabulary
│   └── shapes/*.ttl           # SHACL shapes (generated OWL→SHACL where possible)
├── pi/                        # Pi system prompt, skills, prompt templates, extensions
├── data/                      # runtime data (gitignored): metamind.db, graph/, events/, sandbox/
├── bench/                     # benchmark, adversarial, and historical replay corpora (committed)
└── tests/                     # cross-crate integration + end-to-end suites
```

**Build/verify loop** (used in every phase):

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                 # unit + integration
cargo run -p mm-cli -- doctor          # environment + store readiness + ontology load
```

---

# 4. Identifier scheme — ULIDs and stable URIs

Two id kinds, deliberately separated:

1. **Instance identity = ULID** (lowercase Crockford base32, 26 chars, monotonic factory). Every runtime
   entity gets one. IRIs use `rdf-codec`'s convention: `<https://metamind.dev/data/{ulid}>`.
   SQLite stores them as `TEXT(26) PRIMARY KEY`; RDF stores them as the IRI. Content-addressed artifacts
   (schemas, prompts, embeddings) may use `rdf_codec::ulid_from_content` for reproducibility.
2. **Stable, path-derived URIs = human-diffable** for things whose identity is a name, not a moment:
   - Code module: `https://metamind.dev/code/module/{relative-path}`
   - Module version: `https://metamind.dev/code/module/{relative-path}@{semver}`
   - Source file: `https://metamind.dev/code/file/{relative-path}@{sha256}`
   - Design doc anchor: `https://metamind.dev/design/{doc}#{anchor}`
   - Phase: `https://metamind.dev/phase/{n}`

`mm-core` exposes one funnel: `Id::new_ulid() -> Ulid`, `Id::iri(&Ulid) -> NamedNode`,
`codex::module_iri(path)`, `codex::file_iri(path, hash)`. No ad-hoc string IDs anywhere else.

---

# 5. Storage topology

Three stores, each with an explicit role; no authoritative fact lives in two places.

| Store | Technology | Holds | Access pattern |
|---|---|---|---|
| **Tabular** | SQLite (WAL) via `sqlx` 0.8, async; migrations in `crates/mm-store-sqlite/migrations/` | identity + invariants, goals/commitments, memory index rows, prediction ledger, decision log, tool registry + permission grants, resource accounts, event-log index, module-registry index, calibration rows | async `sqlx` pool |
| **Graph** | Oxigraph 0.5 (RocksDB, `rocksdb-pkg-config`) behind `mm-store-graph` | world model, cognitive library, dependency/provenance graphs, **code-metadata graph**, ontology | single-writer **actor** (Oxigraph `Store` is sync) exposed as an async handle; reads via SPARQL |
| **Event log** | append-only table in SQLite + mirrored PROV triples in Oxigraph | every mutation as an immutable, ordered, replayable record | append + deterministic replay |

Rules:
- **Write path is transactional per store, coordinated by the event log.** An event is appended first
  (with provisional status), the store mutation is applied, then the event is marked committed — so a
  crash yields either "not applied" or "applied once" on replay.
- **`mm-store-graph` is an actor.** Because `oxigraph::store::Store` is synchronous, all writes funnel
  through one `tokio` task with an `mpsc` mailbox; readers use a bounded read-only handle. This gives
  serialized writes and makes the store replayable. (No `unsafe`, no global statics.)
- **Named graphs** keep concerns separate: `.../graph/being`, `/memory`, `/epistemic`, `/library`,
  `/world`, `/provenance`, `/code`. Meta-analysis over code never has to filter runtime facts.
- **SQLite schema conventions:** every table has `id TEXT(26) PRIMARY KEY`, `created_ulid`, and
  `valid_from`/`valid_until` where bitemporality applies; `mm-store-sqlite` enforces
  `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;` on open.

Reused: `rust_symbolic::temporal-store` semantics (immutable record versions, replay oracle) inform the
event log contract; `rust_symbolic::rdf-codec` provides `RdfContext`, `Namespace`, `ToRdf`/`FromRdf`,
`new_ulid`, `ulid_from_content`.

---

# 6. Ontology and validation strategy

- `ontology/mm.ttl` defines the T-Box: classes `Being, Identity, User, Relationship, Goal, Commitment,
  Memory/EpisodicMemory/SemanticMemory/ProceduralMemory/Belief, Claim, Evidence, Observation, Assumption,
  Inference, Prediction, Decision, Action, Outcome, Policy, Doctrine, Principle, Heuristic, Pattern,
  Case, AntiPattern, Capability, Experiment, Evaluation, Failure, NearMiss, EvolutionEvent, Frame,
  Technique, Skill` and the relations listed in design §87 (`dependsOn, supports, contradicts,
  derivedFrom, observedIn, caused, predicts, verifiedBy, similarTo, analogousTo, applicableWhen,
  inapplicableWhen, implementedBy, requires, improves, replaces, testedBy, triggeredBy`).
- `ontology/code.ttl` defines `mmc:` (see Phase 2).
- **SHACL shapes are generated from the OWL/class structure using `rust_symbolic::rdf-shacl`
  (`check_ontology` → `generate_shacl` → `validate`) wherever the fragment fits; hand-written shapes
  cover the rest.** Validation is a hard gate: unsupported constructs are errors, never silent skips.
- Every RDF emitter/decoder goes through `rdf-codec` so serialization is **canonical and deterministic**
  (content-hash stable across re-parses) — this is what makes replay and diffing possible.
- Instance graphs are validated before commit: `mm-cli graph validate --graph being`.

---

# 7. Component reuse matrix

Reuse is via **path dependencies inside the Metamind workspace where possible** (workspace-local
vendoring or `[patch]`/git submodule). Components are adapted behind `mm-*` traits so the runtime never
couples to a utility's internal types.

| Need (design §) | Reused component | Source | Adaptation |
|---|---|---|---|
| Modular code system, plugin lifecycle, T-Box/A-Box | `nexus-core`, `nexus-abi`, `nexus-launcher`, plugin contract | nexus | `modules/*` are nexus plugins; the runtime loads capability modules through nexus lifecycle |
| Execution engine for cognitive programs | `monad-executor`, `monad-scheduler`, `monad-context`, `dsl-compiler`, `nexus-monad-types` | nexus | Cognitive programs compile to the DAG/`ExecutionDocument` form; deterministic nodes run here |
| LLM→executable block | `nexus-mpyir` (`MpyirEngine`) | nexus | Used for "one-shot LLM-produced DSL" tool compositions |
| LLM backends | `nexus-agentstream`; `rust_symbolic::llm-agentstream` | nexus / rust_symbolic | Behind `mm-llm`'s `LlmClient` trait |
| LLM client + cache + structured output + replay | `kg-llm` (`LlmClient`, `MockLlmClient`, `CachedLlmClient`, `OpenAiLlmClient`, schemas) | rust_extract | Primary LLM substrate; `mm-llm` wraps it and adds routing/accounting |
| Embeddings / semantic retrieval | `kg-embed` (`Embedder`, `BgeSmallEmbedder`, BGE-small, packed index) | rust_extract | Memory + case retrieval |
| Entity/event resolution, KG extraction | `kg-extract`, `kg-resolve`, `kg-core` | rust_extract | World-model ingestion; `kg-core` IR + versioned JSONL artifacts inspire event/artifact formats |
| Deterministic validation + RDF emission + ontology | `kg-validate` (`EvidenceValidator`, `ClaimValidator`, RDF emission, conformance) | rust_extract | Epistemic validation barrier in Phase 6 |
| RDF codec / ULID IRIs / namespaces | `rdf-codec` (`RdfContext`, `ToRdf`/`FromRdf`, `new_ulid`) | rust_symbolic | Core graph serialization for all `mm-*` RDF types |
| OWL→SHACL validation | `rdf-shacl` (`check_ontology`, `generate_shacl`, `validate`) | rust_symbolic | Ontology gate |
| Memory classes, consolidation, trajectories | `memory-ir` (`MemoryEngine`, `MemoryKind`, `Consolidation`) | rust_symbolic | `mm-memory` reference implementation + RDF shape |
| Claims/sources/trust/conflicts | `provenance-ir` (`KnowledgeClaim`, `KnowledgeSource`, `Contradiction`, trust update) | rust_symbolic | `mm-epistemic` foundation |
| Belief/knowledge dynamics | `epistemic-ir` (Kripke frames, belief update, info gain) | rust_symbolic | Epistemic propagation + hypotheses |
| Event calculus / process mining | `event-ir` (fluents, `Initiates`/`Terminates`, process discovery, drift) | rust_symbolic | Event log semantics + temporal validity |
| Bitemporal storage / replay | `temporal-store` + `temporal-ir` | rust_symbolic | Event-log immutability/replay contract |
| Decision & utility theory, risk (VaR/CVaR), Bayesian decisions, MDP | `decision-ir` (`DecisionEngine`, `RiskMeasure`, `BayesianProblem`, `Mdp`) | rust_symbolic | Deterministic core of `mm-decision` |
| Causal reasoning | `causal-ir` (SCM, intervention, counterfactual, do-calculus) | rust_symbolic | Phase 6+ causal graph + Phase 11 diagnosis |
| Case-based / analogical reasoning | `analogy-ir` (`Case`, `Correspondence`, `TransferCandidate`) | rust_symbolic | Phase 7 precedent retrieval |
| Symbolic/SAT/SMT/TPTP + planning + proof | `logic-ir`, `solver-ir`, `backend-registry`, `logic-planner` (`Planner`, proof manager, sandbox) | rust_symbolic | Formal verification, constraint feasibility, proof certificates |
| Multi-model ensembles, disagreement | `ensemble-ir` (BMA, consensus, disagreement) | rust_symbolic | Phase 9 multi-lens decisions |
| Learning / policy / evolution primitives | `learning-ir`, `evolution-ir`, `mechanism-ir` | rust_symbolic | Phase 7 policy genome + Phase 12 evolution |
| Math engine | `math-*` (`math-manager`, `math-runtime` residual verifier, `math-solve`) | rust_symbolic | Deterministic arithmetic + verification |
| LLM tool catalog / orchestration | `llm-interface` (`tool_catalog`, `orchestrate`, `repair`) | rust_symbolic | Pattern for `mm-tools` tool catalog + repair loop |
| Derived graph views / diagrams | `kg-diagram` (deterministic Mermaid views + render manifest) | rust_extract | Codebase + architecture views for meta-analysis |
| SPARQL over Oxigraph | nexus `monad-sparql` plugin | nexus | Module-level graph access |
| SQLite over `sqlx` | nexus `monad-sql` plugin | nexus | Module-level tabular access |

Vendoring: add each source repo as a **git submodule under `vendor/`** (or path dependency via
`../..`) and expose only the needed crates through `mm-*` re-exports. A `[patch]` section is avoided
unless a genuine bug fix is required upstream.

---

# 8. Module and plugin contract (the modular code system)

A **module** is the unit of code the system can create, version, test, and promote. Modules follow the
nexus plugin contract so the runtime can load them dynamically.

```
modules/cognition/case-match/
├── plugin.toml            # [plugin] name, uri (STABLE), version, description
├── src/lib.rs             # #![forbid(unsafe_code)]
├── manual/module.md       # operator/LLM-readable manual (nexus monad-manual convention)
├── tests/                 # unit + behavior tests (module-local)
└── metadata.ttl           # generated code metadata (see Phase 2; never hand-edited)
```

`plugin.toml` (extending the nexus shape already used by `monad-llm`):

```toml
[plugin]
name = "mm-case-match"
uri = "https://metamind.dev/code/module/cognition/case-match"   # stable identifier
version = "0.1.0"
[metadata]
category = "cognition"
owned_by_phase = 7
capability = "mm:CaseRetrieval"
[tbox.functions]
"cognition.case_match" = { source = "handlers::case_match" }
[monad.operations]
name = "cognition"
arity = 1
[build]
rust_edition = "2021"
```

Contract rules (enforced by `mm-cli codex verify`):
1. Every source file under the workspace belongs to exactly one module; orphan files fail CI.
2. Every module declares a **stable `uri`**, a `version`, an owning phase, and ≥1 capability.
3. Every capability names its tests; a capability without tests fails the gate.
4. Modules depend only on `mm-core` + other declared modules (no cycles).
5. The module's `manual/module.md` and T-Box functions are authoritative for the LLM's use of it.

---

# 9. Testing strategy (applies to every phase)

| Layer | Tool | Requirement |
|---|---|---|
| Unit tests | `cargo test` per crate | Every public function; edge cases include malformed input |
| Property tests | `proptest` | ULID uniqueness/monotonicity; RDF round-trip canonicalization; event-log replay equals state |
| Golden / replay | `insta` snapshots + versioned JSONL artifacts | Deterministic outputs compared byte-for-byte; offline replay makes **zero** provider calls |
| Ontology conformance | `rdf-shacl` + `kg-validate` conformance | Every emitted graph validates; unsupported constructs are hard errors |
| Adversarial | committed adversarial corpus under `bench/` | Promotion and safety decisions tested against deliberate bad inputs |
| Benchmark | `bench/` harness with baseline comparison | Every self-improvement must beat or match a baseline on a labeled set |
| End-to-end | `tests/e2e/*.rs` + `mm-cli` subprocess tests | Each phase's gate is an actual runnable command |
| Determinism | event-log replay + content hashes | Any phase run can be reproduced from `(code version, event log, config)` |

**Rule:** no phase is complete until its pass gate runs green as a plain command and the earlier phases'
gates still pass. Do not weaken assertions to pass; fix the cause.

---

# 10. Pi integration contract (the code-editor system)

Pi is the **only** code editor. The cognitive runtime never writes production source directly; it
drives Pi and then verifies Pi's output deterministically.

- **Transport.** `mm-pi` spawns `pi --mode rpc --provider <p> --model <m> --session-dir data/sandbox/pi`
  as a child process and speaks the documented **strict JSONL protocol** over stdin/stdout:
  commands `prompt`, `steer`, `follow_up`, `abort`, `clear_queue`, `new_session`, each carrying an
  optional `id`; responses carry `type:"response"`; agent events stream as JSONL. Framing splits on
  `\n` only (strip trailing `\r`), never on Unicode line separators.
- **Prompting.** One Pi session per change-set. The runtime sends a typed task prompt (goal, target
  module path, allowed files, required outputs, forbidden actions) and streams events, correlating by
  `id`. `--tools` is restricted per task; `--system-prompt`/`--append-system-prompt` carry the
  Metamind module contract; `pi/` holds reusable skills (e.g. `mm-module-scaffold`) and prompt
  templates that Pi expands via `/skill:` and `/template`.
- **Session ingestion.** Pi persists sessions as JSONL (header `version:3, id, cwd` then
  `parentId`-chained records). `mm-pi` ingests the session file into the **event log** and the
  **code-metadata graph** as `mmc:PiSession`, `mmc:Edit`, `mmc:ToolCall` resources keyed by ULID —
  so every generated line has provenance.
- **Isolation.** Pi runs only inside `data/sandbox/<changeset-ulid>/` (a git worktree/copy), with an
  isolated session dir. It may not touch the production tree; promotion copies verified artifacts.
- **Model/provider.** Uses the same OpenAI-compatible endpoint (default provider `dojo_dev`). The
  runtime's LLM and Pi share `.env`/`pi/` config so routing can move workloads between them.
- **Fallback.** If subprocess RPC is unavailable, the Node SDK (`AgentSession`) can be embedded via a
  thin Node sidecar exposing the same JSONL contract; the `mm-pi` interface is unchanged.

---

# 11. Phase dependency graph

```
P1 Foundation ──┬── P2 Codex/module metadata ──────────────┐
                ├── P3 LLM substrate ──────────────────────┤
                ├── P4 Being substrate ────────────────────┤
                └── P5 Memory ─────────────────────────────┤
                                                           ▼
                                    P6 Epistemic discipline
                                                           │
                                                           ▼
                                    P7 Cognitive library + frames + policy genome
                                                           │
                                                           ▼
                                    P8 Metacognitive controller + program compiler
                                                           │
                                                           ▼
                                    P9 Sanity firewall + DecisionCore + comparison integrity
                                                           │
                                                           ▼
                                    P10 Tool execution + verification + external world
                                                           │
                                                           ▼
                                    P11 Self-engineering + calibration + promotion + Pi code-gen
                                                           │
                                                           ▼
                                    P12 Self-bootstrap closed loop (functional prototype)
```

P1 is the hard prerequisite. P2–P5 can proceed in parallel after P1. P6 needs P4+P5. Each later phase
adds exactly one new organ and must not regress earlier gates.

---

# 12. The 12 phases

Each phase has: **Goal · Reused components · New code/ontology · Steps · Testing · Pass gate**.

---

## Phase 1 — Foundation: deterministic kernel, dual stores, ontology v0

**Goal.** A buildable workspace with a deterministic kernel: ULID identity, SQLite + Oxigraph behind
async traits, an append-only event log with replay, `ontology v0`, and the test harness every later
phase depends on. No self-modification, no cognition.

**Reused components.** `rdf-codec` (ULIDs, namespaces, RDF encode/decode), `temporal-store` (immutability/
replay contract), nexus plugin lifecycle skeleton (`nexus-abi`/`nexus-core` patterns) for `modules/`.

**New code / ontology.**
- `mm-core`: `Id`, `Ulid` factory, `Timestamp` (integer seconds+nanos, as in `nexus-abi`
  `HiResTimestamp`), `MmError`, `Config`, and async traits `Tabular`, `Graph`, `EventSink`.
- `mm-store-sqlite`: `sqlx` SQLite pool, WAL, migrations (identity, events, and empty forward tables);
  `sqlx::migrate!` applied on open.
- `mm-store-graph`: Oxigraph **actor** (single writer, async handle), named-graph manager, `rdf-codec`
  context wiring, `mm-cli graph validate`.
- `mm-eventlog`: append/commit protocol, monotonic sequence, `replay(until) -> State`; PROV mirroring.
- `ontology/mm.ttl` (T-Box v0 with the §87 classes/relations) + `ontology/shapes/`.
- `mm-cli doctor`: opens both stores, loads ontology, prints versions, runs a self-check.

**Steps.**
1. Create the workspace, `rust-toolchain.toml`, shared lints (`forbid(unsafe_code)`, clippy deny), CI skeleton.
2. Implement `mm-core` IDs/timestamps/config/errors + determinism helpers (content hashing).
3. Implement `mm-store-sqlite` with migrations; add the `events` table with (seq, ulid, type, payload JSON, status).
4. Implement `mm-store-graph` actor over Oxigraph; wire named graphs; expose async SPARQL.
5. Implement `mm-eventlog` append→apply→commit protocol and replay to a state snapshot.
6. Author `ontology/mm.ttl`; generate SHACL shapes with `rdf-shacl`; add `graph validate`.
7. Add the module skeleton under `modules/` (one bootstrap module) and the workspace members glob.
8. Implement `mm-cli doctor` and a `mm-cli replay` smoke command.

**Testing.** Unit tests for IDs/config/errors; property tests for ULID uniqueness + monotonicity and
RDF canonical round-trip; a crash-injection test proving replay yields exactly-once application;
migration up-then-down; `doctor` integration test; SHACL validates an empty instance graph (0 violations).

**Pass gate.**
- `cargo build --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo test --workspace` green.
- `mm-cli doctor` exits 0 and reports sqlite/oxigraph versions, ontology triples, and both stores writable.
- Property tests: 100k ULIDs all unique; 10k-triple graph export→reimport gives an identical content hash.
- Event-log replay from an empty log reconstructs a mutated state byte-identically; a killed writer replays with no double-apply.
- `mm-cli graph validate` reports 0 SHACL violations for a clean instance and ≥1 for a deliberately broken one.

---

## Phase 2 — Modular code system and code-metadata registry

**Goal.** Make the codebase itself a first-class, queryable RDF artifact. Every module and source file
gets a **stable RDF identifier** in a dedicated code graph, enabling the meta-analysis (§62/§85/§105)
and the Pi-driven self-build later. This must exist before any self-modification.

**Reused components.** nexus plugin contract + `plugin.toml` (`monad-llm` shows the `uri` convention);
nexus `monad-rust`/`nexus-rust-ast` and `rust_extract::::kg-diagram` for derived views; `rdf-codec`
for canonical emission; `rdf-shacl` for shape validation.

**New code / ontology.**
- `ontology/code.ttl`: vocabulary `mmc:` (`path, uri, version, dependsOn, exports, requires,
  implementsCapability, hasTest, testCovers, contentHash, ownedByPhase, promotedFrom, registryIndex`).
- `mm-codex`: workspace scanner that parses `Cargo.toml`, `plugin.toml`, T-Box macros, test files;
  computes file `sha256`; emits `metadata.ttl` per module and upserts into graph
  `https://metamind.dev/graph/code`; maintains `modules/registry.json` and `codex.lock`.
- `mm-cli codex {scan, verify, graph, diff, meta}`.
- Meta-analysis queries (`codex meta`): module count, dependency acyclic check, orphan files,
  capabilities without tests, phase ownership map, churn.

**Steps.**
1. Define `mmc:` vocabulary and SHACL shapes (module has exactly one `path`, one stable `uri`, a version,
   an owning phase; source file belongs to exactly one module).
2. Implement the scanner; assign stable URIs via `mm-core` (`codex::module_iri`, `codex::file_iri`).
3. Emit module RDF + `registry.json`; commit `codex.lock` for CI drift detection.
4. Add `codex verify` that fails on: unregistered file, duplicate URI, dependency cycle, capability without tests.
5. Add `codex graph` producing a derived Mermaid view (via `kg-diagram` conventions) and `codex meta`
   SPARQL reports.
6. Wire the scan into the build (a `build.rs`-free `mm-cli` step) so metadata is refreshed deterministically.
7. Backfill metadata for the bootstrap module and every `mm-*` crate.

**Testing.** Scanner unit tests on fixture trees; SHACL conformance on the code graph; golden test that
`codex scan` is idempotent (running twice yields identical triples); negative tests for each `verify` rule.

**Pass gate.**
- `mm-cli codex scan` registers every workspace file under exactly one module; `codex verify` exits 0.
- The code graph SHACL-validates with 0 violations.
- Adding a stray source file, a duplicate module URI, a dependency cycle, or a capability with no test
  makes `codex verify` exit non-zero with a specific message.
- `codex meta` answers, via SPARQL, at least: total modules, modules per phase, capabilities without
  tests, and modules whose code changed without a version bump (drift).
- `codex scan` is idempotent (identical content hash across two runs).

---

## Phase 3 — LLM cognitive substrate

**Goal.** One typed, cached, accountable LLM interface used by everything. The LLM must never be called
ad-hoc; every call is schema-validated, budgeted, logged, and replayable offline.

**Reused components.** `kg-llm` (`LlmClient`, `MockLlmClient`, `CachedLlmClient`, `OpenAiLlmClient`,
structured-output schema, request hashing); `nexus-agentstream` and `rust_symbolic::llm-agentstream`
as alternative backends; `nexus` `monad-llm` for DSL-level access.

**New code / ontology.**
- `mm-llm`: `LlmClient` re-export + `RoutingPolicy`, `CallAccount` (tokens/cost/latency), `StructuredOut<T>`
  with hard schema validation, and a replay mode.
- SQLite tables: `llm_calls` (ulid, purpose, model, prompt_hash, tokens_in/out, cost, latency, status).
- `ontology` additions: `mm:LlmCall` node linked into the provenance graph.

**Steps.**
1. Wrap `kg-llm` behind `mm-llm`; add the accounting layer and purpose tags (interpret, plan, extract,
   critique, summarize, code-review…).
2. Implement routing: deterministic policy now (cost/latency/complexity thresholds); leave the
   `DecisionCore` seam for Phase 9.
3. Enforce structured outputs: reject schema violations (no silent coercion); record repair attempts.
4. Add disk cache keyed by request hash; replay mode performs zero provider calls.
5. Record every call in `llm_calls` + provenance; expose `mm-cli llm stats`.
6. Add adversarial fixtures (malformed JSON, truncated output, wrong types) to `bench/llm`.

**Testing.** Mock/cached replay determinism; schema-violation rejection; accounting correctness on
fixtures; adversarial malformed payloads never crash and never produce a typed value; concurrency
(bounded parallel calls) preserves accounting.

**Pass gate.**
- Offline replay of a recorded session makes **zero** network calls and yields identical outputs.
- 100% of structured-output violations are rejected (not coerced); adversarial fixtures all handled.
- Every call has a complete accounting row (tokens, cost, latency, purpose, prompt hash).
- Routing selects the cheapest model meeting a declared requirement on a labeled toy workload, deterministically.

---

## Phase 4 — Being substrate: identity, personality, affect, user, relationships, goals

**Goal.** The persistent substrate that survives model changes: identity + invariants, personality
traits (persistent) vs affect (transient), user model with epistemic status, relationship dimensions,
goals/commitments/desires, and a resource economy with enforced budgets.

**Reused components.** `provenance-ir` (claim/source/trust) and `epistemic-ir` (belief) for the user
model's status; `event-ir` for goal/commitment lifecycle events; SQLite for authoritative rows;
Oxigraph for relational structure.

**New code / ontology.**
- `mm-being`: `Identity` + `Invariant` enforcement, `Personality` (trait/state split), `Affect`,
  `UserModel` (`Belief<T>` with `EpistemicStatus`, confidence, evidence, validity — design §7),
  `RelationshipState` (familiarity, trust, reciprocity, openness, cooperation, reliance, unresolved
  issues), `Goal`/`Commitment` (§9), `ResourceState` + budgets.
- SQLite: `identity`, `invariants`, `personality_traits`, `affect_state`, `user_beliefs`,
  `relationships`, `goals`, `commitments`, `resource_accounts`.
- Ontology: the corresponding classes + `mm:Invariant` as non-evolvable.

**Steps.**
1. Implement identity with immutable creation record, lineage, current version, and invariants
   ("never fabricate autobiographical memory", "never promote assumption to observation", "never claim
   action without execution evidence", "never silently rewrite history").
2. Implement personality as persistent traits + contextual modifiers; affect as short-lived deterministic
   state transitions influenced by appraisal inputs.
3. Implement the user model as probabilistic beliefs with epistemic status; never auto-promote to fact.
4. Implement relationships as multi-dimensional, event-sourced state.
5. Implement goals/commitments with lifecycle (active/fulfilled/revoked/superseded) and deadlines.
6. Implement the resource economy: compute/context/LLM-calls/money/tool-execution budgets enforced in
   code; LLM never sets spending.
7. Persist everything to SQLite with RDF mirror; add `mm-cli being show/verify`.

**Testing.** Invariant enforcement tests (attempted violations rejected by deterministic code); belief
status preservation; goal/commitment lifecycle monotonicity; budget enforcement cannot be exceeded;
affect trait/state separation; SHACL conformance of the being graph.

**Pass gate.**
- Deterministic tests prove each identity invariant cannot be violated through the public API.
- A user statement enters as `REPORTED`/`INFERRED` and can never become `OBSERVED` without an evidence
  record that satisfies the promotion rule.
- Commitments are monotonic: fulfilled/revoked/superseded transitions are append-only; no silent rewrite.
- Budget enforcement: a scripted overspend is rejected; resource rows reconcile with `llm_calls` totals.
- `mm-cli being verify` exits 0 with invariants intact and 0 SHACL violations.

---

## Phase 5 — Persistent memory architecture

**Goal.** The ten memory classes from §32 as typed, provenance-bearing traces with deterministic
retrieval, episodic→semantic consolidation, deliberate forgetting, and mistake/near-miss retention.

**Reused components.** `memory-ir` (`MemoryEngine`, `MemoryKind`, `Consolidation`, `ReasoningTrajectory`);
`kg-embed` (BGE-small) for semantic retrieval; `temporal-store` for validity intervals.

**New code / ontology.**
- `mm-memory`: `Memory` record (content, type, source, timestamp, confidence, importance, validity,
  provenance, related entities, retrieval cues, access history, outcomes); deterministic `MemoryUtility`
  (`FutureBehaviorImpact × RetrievalProbability × Reliability / StorageCost`); consolidation pipeline;
  forgetting with immutable-ledger protection; `Mistake`/`NearMiss` records.
- SQLite: `memories`, `memory_links`, `memory_access`, `mistakes`; a vector index (packed,
  `kg-embed` convention).
- Ontology: `mm:Memory` subclasses + retrieval cues; provenance links.

**Steps.**
1. Implement memory records + typed storage across the 10 classes.
2. Implement hybrid retrieval (keyword + embedding) with deterministic scoring and a gold set.
3. Implement consolidation: raw episode → repeated pattern → semantic memory → compressed rule;
   preserve provenance, allow archive/delete of low-value episodes.
4. Implement forgetting via the utility formula, never touching the developmental ledger.
5. Implement mistake/near-miss records with `failure_mode`, `root_cause`, `missed_signal`,
   `corrective_rule`, `recurrence_risk`.
6. Mirror into the `/memory` named graph; add `mm-cli memory {add,get,consolidate,stats}`.

**Testing.** Gold-set retrieval precision/recall; consolidation reduces count while preserving
provenance; forgetting never deletes protected records; embedding fallback only when keyword fails
(matching `kg-resolve` behavior); property test that memory utility is monotone in its inputs.

**Pass gate.**
- Retrieval precision@k and recall@k on the committed gold set meet the phase threshold (set explicitly
  in `bench/memory/gold.jsonl`; the run fails below it).
- Consolidation of a synthetic episode stream reduces memory count while a SPARQL query can still
  reconstruct every source episode through provenance.
- Forgetting removes only records whose utility is below threshold and leaves all protected/immutable
  records intact.
- A seeded mistake yields a retrievable `Mistake` record that names a `corrective_rule`.
- `mm-cli memory stats` reconciles SQLite rows with `/memory` triples exactly.

---

## Phase 6 — Epistemic discipline

**Goal.** Separate what the being *thinks* from what the world *contains*. Claims, evidence,
observations, inferences, hypotheses, assumptions, predictions with explicit `EpistemicStatus`;
assumption ledger; contradiction detection; epistemic dependency graph with propagation and
"never silently promote" enforcement.

**Reused components.** `provenance-ir` (claims/sources/trust/conflicts), `epistemic-ir` (belief/Kripke),
`event-ir` (temporal validity), `kg-validate` (`EvidenceValidator`, `ClaimValidator`, RDF emission,
conformance), `rdf-shacl`.

**New code / ontology.**
- `mm-epistemic`: `Claim`, `Evidence`, `Observation`, `Inference`, `Hypothesis`, `Prediction`,
  `Assumption` (`Assumption` struct from §21 with `consequence_if_false`, `verification_cost`,
  `dependencies`), `EpistemicStatus` enum (`OBSERVED, VERIFIED, REPORTED, INFERRED, ASSUMED,
  HYPOTHETICAL, PREDICTED, SIMULATED, FICTIONAL, UNKNOWN`); contradiction records;
  dependency DAG with cascade invalidation; `VerificationPriority` formula
  (`ProbabilityFalse × ConsequenceIfFalse × DecisionDependence / VerificationCost`).
- SQLite: `claims`, `evidence`, `assumptions`, `predictions`, `contradictions`, `dependencies`.
- Ontology: the epistemic classes + relations `supports, contradicts, derivedFrom, evidenceFor, dependsOn`.

**Steps.**
1. Implement the claim/evidence/observation model with append-only provenance.
2. Implement the epistemic-status rules + a promotion guard (no promotion without satisfying evidence).
3. Implement the assumption ledger and `VerificationPriority` (deterministic arithmetic).
4. Implement contradiction detection on ingest against facts/assumptions/commitments/plans/predictions/
   memories/policies; create explicit contradiction records (never silent reconciliation).
5. Implement the dependency graph: `dependsOn(C,B)`, `dependsOn(B,A)`; when a leaf is invalidated,
   mark downstream as suspect via `event-ir`-style state transitions.
6. Add the `kg-validate`-style validation barrier before any claim commit; mirror to `/epistemic`.

**Testing.** Promotion-guard tests (each forbidden promotion rejected); contradiction fixtures from
`bench/adversarial`; dependency cascade tests; temporal validity honored under bitemporal queries;
SHACL + symbolic consistency; property test that the dependency graph is a DAG after commit.

**Pass gate.**
- The forbidden promotions (`assumption→fact`, `inference→observation`, `prediction→event`,
  `simulation→reality`) are all rejected by deterministic code.
- Inserting a contradictory claim creates a `mm:Contradiction` record with both sides and evidence;
  nothing is silently reconciled.
- Invalidating a leaf assumption marks all transitive dependents suspect within one event-log replay.
- `VerificationPriority` matches reference values on a golden table.
- World graph and model graph are provably separate: a SPARQL query over `/world` returns only
  `OBSERVED|VERIFIED` facts.

## Phase 7 — Cognitive library, conceptual frames, and the policy genome

**Goal.** A persistent, versioned repertoire of *ways of thinking* — doctrines, principles, heuristics,
techniques, patterns, cases, anti-patterns, skills, policies — plus temporary **conceptual frames** and a
**policy genome**. This is where the system stops rediscovering reasoning and starts reusing it.

**Reused components.** `analogy-ir` (`Case`, `Correspondence`, `TransferCandidate`), `evolution-ir`
(populations, fitness), `mechanism-ir`, `learning-ir`; `rust_extract`'s extraction pattern (`kg-extract`
+ `kg-llm` + `kg-validate`) for LLM-populated, schema-validated library entries; `rdf-shacl`.

**New code / ontology.**
- `mm-library`: entry types `Doctrine, Principle, Heuristic, Technique, Pattern, Case, AntiPattern,
  Skill, Policy, Frame, Evaluation` with `applicableWhen`/`inapplicableWhen`, `contraindications`,
  `examples`/`counterexamples`, `evidence` (§13, §15, §16).
- `Frame` registry supporting **multiple simultaneous frames** (§14): software architecture + economic
  optimization + risk + coordination.
- `Policy` genome (§44): `id, version, scope, activation, behavior, evidence, fitness, confidence, parent`.
- Seed corpus task: LLM extracts structured entries (via `kg-extract` pattern) from the reasoning lenses
  in §15–16; `rdf-shacl` validates each against the library shapes before commit.
- Ontology: library classes + relations (`applicableWhen, improves, replaces, derivedFrom, testedBy`).

**Steps.**
1. Define library classes and SHACL shapes; require every entry to have purpose, activation conditions,
   contraindications, evidence, and ≥1 example.
2. Implement the LLM extraction pipeline for seed doctrines/policies/techniques; validate, then commit to `/library`.
3. Implement case records (positive, failed, unusual, edge cases — §17) with outcome quality and transferability.
4. Implement frame generation/selection storage and multi-frame episodes.
5. Implement the policy genome with versioning, parentage, and fitness records; no policy mutation without a version.
6. Add applicability queries (`mm-cli library applicable --state ...`) and case retrieval via `analogy-ir`.

**Testing.** SHACL conformance on all seed entries; no orphan references (every `mm:evidence`/`mm:technique`
ID resolves); case-retrieval gold set; policy version monotonicity; duplicate-detection on ingest.

**Pass gate.**
- The seed library loads with ≥1 entry per class and 0 SHACL violations; every referenced id resolves.
- An LLM-extracted entry that violates a shape is rejected, not coerced.
- `library applicable` returns a ranked technique set for a benchmark state; ranking is deterministic.
- Every policy has version/parent/fitness and is retrievable by its stable URI; no policy can be mutated in place.

---

## Phase 8 — Metacognitive controller and cognitive-program compiler

**Goal.** The central new organ (§11, §12): a general metacognitive scan that decides *what kind of
cognition is needed*, then compiles a temporary **cognitive program** — optimizing for **minimum
sufficient cognition**, not maximum.

**Reused components.** `decision-ir` (bounded selection), `logic-planner` (`Planner` cost model),
`nexus` `monad-executor` + `dsl-compiler` + `nexus-monad-types` to execute the compiled program as a DAG,
`nexus-mpyir` for LLM-authored one-shot blocks.

**New code / ontology.**
- `mm-metacog`: `CognitiveEpisode` (§88) and `CognitiveOp` enum (§89: Recall, Observe, FormHypothesis,
  Compare, FindSimilar, CheckAssumption, Verify, Invert, Simulate, Critique, Simplify, Decide, Act,
  Evaluate, Learn); the scan → program compiler; `OperationValue = ExpectedErrorReduction ×
  DecisionImportance × ProbabilityOfChangingDecision / OperationCost`; `CognitiveBudget` (§43).
- Program output (typed): active frames, doctrines, techniques, reference classes, assumptions to verify,
  comparisons, candidate actions, evaluation criteria, required tools, stopping conditions (§12).
- Compiler target: the nexus `ComputationDAG`/`ExecutionDocument` form, so deterministic nodes run on
  `monad-executor` and semantic nodes call the LLM.
- SQLite: `episodes`, `programs`, `program_traces`.

**Steps.**
1. Implement the broadcast LLM scan (one broad pass, not twelve prompts) + bounded classifications.
2. Implement program compilation to a typed program and its lowering to a computation DAG.
3. Implement the operation-value score with deterministic arithmetic and budget checks.
4. Implement minimum-sufficient-cognition: trivial episodes stop early; stopping conditions enforced.
5. Record full traces for replay and later distillation (Phase 11).
6. Add `mm-cli episode run --input ...` and `episode replay`.

**Testing.** Golden programs for benchmark episodes; deterministic arithmetic golden table for
`OperationValue`; budget never exceeded; trivial episodes produce ≤ k operations; traces replay identically.

**Pass gate.**
- For a committed benchmark episode set, the controller emits typed, schema-valid cognitive programs.
- `OperationValue` matches a golden table exactly (deterministic arithmetic).
- A set of trivial episodes compiles to ≤ k operations each; a hard episode compiles to a richer program
  — both within budget.
- Every episode produces a replayable trace; replay yields identical programs.

---

## Phase 9 — Sanity firewall, decision core (Jev-style), and comparison integrity

**Goal.** The pre-action safety gate (§46) and the bounded-decision primitive (§47–49). A `DecisionCore`
abstraction answers typed questions (choice/score/yes-no) over shared state; a `SanityFirewall` aggregates
assumptions, comparisons, contradictions, feasibility, missing steps, risk, reversibility, and authority
into one outcome; comparison integrity (§20) gets its own subsystem.

**Reused components.** `decision-ir` (`DecisionEngine`, `RiskMeasure` VaR/CVaR, `BayesianProblem`, `Mdp`),
`ensemble-ir` (multi-lens disagreement), `logic-ir`/`backend-registry`/`logic-planner` for constraint
feasibility and formal checks, `causal-ir` for risk structure.

**New code / ontology.**
- `mm-decision`: `DecisionCore` trait with **three interchangeable impls** — (a) deterministic rule set,
  (b) local classifier/model head, (c) hosted decision model — all sharing one conformance suite;
  typed `DecisionQuestion` (choice/score/noul) and calibration schema.
- `mm-firewall`: unified scan producing `PROCEED | PROCEED_WITH_CAUTION | VERIFY_FIRST | ASK_USER |
  REPLAN | HUMAN_REVIEW | REJECT` (§46); hard deterministic prohibitions; bounded judgments via `DecisionCore`.
- Comparison integrity: comparison contract (objective/objects/dimensions/units/timeframe/conditions/
  constraints/evidence) and validity checks (same class/purpose/units/timeframe/conditions/definitions/
  scope); normalization deterministic.
- SQLite: `decisions` (features + answer + confidence + timestamp), `comparisons`, `firewall_runs`.
- Ontology: `mm:Decision`, `mm:Comparison`, `mm:Risk` with outcomes for calibration.

**Steps.**
1. Define `DecisionCore` + conformance tests (determinism, typing, confidence in [0,1]).
2. Implement the deterministic-rules impl first; add local and hosted adapters behind the same trait.
3. Implement risk measures (maximum loss, tail probability, ruin, variance, downside asymmetry,
   reversibility, optionality) via `decision-ir`.
4. Implement comparison integrity + deterministic normalization.
5. Implement the sanity firewall aggregating Phase 6 outputs + Phase 7/8 state; log every run.
6. Log every bounded decision with features + answer for later calibration (Phase 11).

**Testing.** DecisionCore conformance across all impls; adversarial labeled set for firewall outcomes
with precision/recall thresholds; comparison mismatch fixtures; risk measures vs reference values;
every decision and firewall run logged with enough features to calibrate.

**Pass gate.**
- All three `DecisionCore` impls pass one identical conformance suite.
- The firewall meets precision/recall thresholds on the committed adversarial set; every hard prohibition
  short-circuits to `REJECT` regardless of model output.
- Comparisons with mismatched units/timeframe/conditions are rejected or explicitly flagged non-comparable
  (golden fixtures).
- Risk measures match reference values (VaR/CVaR/ruin) within tolerance.
- 100% of decisions and firewall runs are logged with features + outcome, keyed by ULID.

---

## Phase 10 — Tool execution, verification, and the external world

**Goal.** Turn proposals into authorized, observable action. Typed `CognitiveOp`s are mapped to a
deterministic executor; the tool registry and permission engine are authoritative; execution returns
**authoritative observations**; verification tools (compiler/tests/symbolic) run; rollback exists.

**Reused components.** `logic-planner::sandbox` (capabilities/filesystem/network), `backend-registry`
(SAT/SMT/TPTP), `math-runtime` (residual verifier), `llm-interface` (tool catalog/orchestration/repair),
nexus `monad-process`/`monad-file`/`monad-sparql`/`monad-sql`/`monad-resource` for concrete tools.

**New code / ontology.**
- `mm-tools`: `ActionResult` (§52: action, status, observation, evidence); deterministic `Executor`;
  `ToolRegistry` with declared inputs/outputs/permissions/reversibility; `PermissionEngine` (grants,
  scopes, approvals); action ledger; rollback; external observation records.
- Tool set (v1): filesystem (read/write/move), process (build/test), graph (SPARQL), tabular (SQL),
  HTTP (fetch), and the Pi editor tool (Phase 11).
- SQLite: `tool_calls`, `permission_grants`, `action_ledger`, `observations`.
- Ontology: `mm:Action`, `mm:Observation`, `mm:Tool`, `mm:Permission`; observations linked to evidence.

**Steps.**
1. Implement the tool registry with permissions + reversibility metadata.
2. Implement the deterministic executor with the sandbox (`logic-planner::sandbox` conventions).
3. Guarantee that observations come from execution evidence, never inferred from plans (§52).
4. Implement the action ledger (append-only) and rollback for reversible actions.
5. Implement verification tools: cargo build/test, static analysis, symbolic/SMT checks, residual math verify.
6. Add `mm-cli tool list/run`, `mm-cli action ledger`, `mm-cli action rollback <ulid>`.

**Testing.** Unauthorized tool denied; sandbox blocks undeclared fs/network; plan-vs-observation separation;
rollback restores prior state byte-identically; ledger immutability; seeded logical inconsistency caught
by the symbolic verifier; every action yields an authoritative observation record.

**Pass gate.**
- A tool call without a matching permission grant is denied by deterministic code (never by model judgment).
- Sandbox capability tests: undeclared filesystem/network access fails; declared access succeeds.
- Executor observations are sourced only from execution evidence; a fabricated "planned" observation is
  rejected by the epistemic layer.
- Reversible actions roll back to a byte-identical prior state; the action ledger rejects mutation.
- The symbolic verifier detects a committed seeded inconsistency and reports a proof obligation failure.

---

## Phase 11 — Self-engineering: meta-analysis, calibration, mistake learning, and Pi-driven module construction

**Goal.** Close the improvement loop. Event-triggered meta-analysis diagnoses behavior; the prediction
ledger is calibrated; mistakes become regression tests; candidate improvements produce **change sets**;
**Pi** writes the new module code under the runtime's control; the promotion pipeline (sandbox → tests →
regression → adversarial → benchmark → shadow → promote/reject) decides — never the LLM. This is the
largest phase and the point at which the system begins building new modules itself.

**Reused components.** `mm-pi` (new) wrapping Pi's RPC; `kg-llm` for diagnosis; `causal-ir` for root-cause
and intervention reasoning; `learning-ir`/`evolution-ir` for policy/gene fitness; nexus plugin loader to
load a promoted module; `rust_extract`'s versioned JSONL artifact + replay pattern for change sets;
`rdf-shacl` to validate emitted metadata.

**New code / ontology.**
- `mm-metaanalysis` (§63, §84): triggers (prediction error, user correction, repeated failure,
  contradiction, unexpected outcome, goal failure, near miss, novel success); the diagnosis prompt;
  `ErrorTaxonomy` (knowledge/retrieval/interpretation/comparison/assumption/causal/planning/decision/
  execution/verification/social/resource/policy/code/data/model); prediction ledger + calibration
  (Brier, log loss, ECE, reliability, selective risk/coverage); recurrence + impact metrics.
- `mm-selfeng` (§58, §59, §68, §72): `ChangeSet` (code patches, schema migrations, data migrations,
  memory transformations, policy changes, prompt changes, new tests, new benchmarks, rollback procedure);
  sandbox; benchmark/shadow runner; **PromotionGate** with explicit policy; rollback; evolution journal
  segmented into meta-analysis / improvement / evolution / metacognitive budgets (§43).
- **Mistake→test compiler** (§60): incident → reproduction → regression test → fix → test retained.
- `mm-pi`: async child-process client for `pi --mode rpc` (strict JSONL framing, `prompt`/`steer`/
  `follow_up`/`abort`/`new_session`, id correlation); session ingester (Pi JSONL v3, `parentId` chain)
  into the event log + `/code` graph as `mmc:PiSession`, `mmc:Edit`, `mmc:ToolCall`.
- Pi assets under `pi/`: system prompt encoding the module contract, `mm-module-scaffold` skill,
  prompt templates per module kind, an extension to emit `metadata.ttl`.
- SQLite: `predictions`, `prediction_outcomes`, `calibration`, `meta_analyses`, `change_sets`,
  `promotions`, `evolution_journal`, `budgets`.
- Ontology: `mm:Prediction`, `mm:EvolutionEvent`, `mm:ChangeSet`, `mmc:*` for Pi provenance.

**Steps.**
1. Implement the prediction ledger and calibration math; require every consequential prediction to be logged.
2. Implement event-triggered meta-analysis + error taxonomy + recurrence/impact scoring.
3. Implement the mistake→regression-test compiler; new tests enter `bench/regression/` forever.
4. Implement `ChangeSet` assembly and the sandbox (git worktree under `data/sandbox/<ulid>/`).
5. Implement the promotion pipeline and gate; promotions update the code-metadata graph in Phase 2.
6. Implement `mm-pi`: RPC client, restricted-tools profile, isolated session dir; ingest sessions into RDF.
7. Implement the first generated-module workflow: gap → Pi scaffolds module → tests → benchmark → promote.
8. Enforce budgets (meta-analysis/improvement/evolution/metacognitive) with deterministic arithmetic.
**Testing.** Pi RPC protocol tests against a recorded session (deterministic framing, id correlation,
steer/abort); session ingestion round-trip into RDF; sandbox isolation; change-set schema; promotion gate
negative cases; calibration metrics vs reference values; a seeded bug whose regression test fails before
the fix and passes after.

**Pass gate (the self-engineering gate).**
- End-to-end: from a seeded capability gap, the system produces a `ChangeSet`, Pi generates the module,
  the sandbox builds + tests it, a benchmark compares it to a baseline, and the gate either promotes or
  rejects **with a written reason** — with no human code edits.
- A seeded bug yields a regression test in `bench/regression/` that **fails before** and **passes after**
  the fix, and the test is retained.
- The production tree is never written outside promotion (verified by filesystem audit + event log).
- A promoted module is registered in the code-metadata graph with code+data+schema+policy+prompt versions,
  and `codex verify` stays green.
- Calibration: Brier/log-loss/ECE are computed on the prediction ledger and improve over the recorded
  baseline across one labeled cycle.
- Budgets cannot be exceeded; every meta-analysis, change, and promotion is journaled immutably.

---

## Phase 12 — Self-bootstrap closed loop (the functional prototype)

**Goal.** Reach the point where the being can **continue its own design and build process**: given a
design goal, it runs the full loop — meta-analysis → capability gap → change set → Pi-authored module →
test/benchmark → promotion → new capability — and updates its own design documents and phase plan, with
full provenance and reproducible replay. Also implemented: self-model divergence (§62), architectural
debt/GC (§85–86), multi-timescale scheduling (§31), and the continuous dev loop of §103.

**Reused components.** Everything above; `evolution-ir` for population/fitness; `ensemble-ir` for
multi-lens self-evaluation; `nexus-launcher`/plugin loader to hot-load newly promoted modules.

**New code / ontology.**
- `mm-runtime`: the closed `mm-loop` command — scheduler, budgets, multi-timescale orchestration (action /
  episode / project / goal / identity), and the `EXPERIENCE → EVENT LOG → META-ANALYSIS → CHANGE →
  SELF-ENGINEERING → TEST/BENCHMARK → SHADOW → PROMOTION GATE → NEW VERSION` loop of §103.
- Self-model (§62): Actual vs Model vs Ideal; divergence metrics; a periodic divergence report.
- Architectural housekeeping (§85–86): complexity, unused capabilities, duplicate/conflicting policies,
  stale memories, unused schemas, expensive workflows, obsolete techniques; GC that respects the
  immutable developmental ledger.
- `mm-cli loop run --goal ...`, `mm-cli self-model report`, `mm-cli debt scan`, `mm-cli gc`.
- Self-design: the runtime generates/updates Markdown design docs and the phase plan in `design/`,
  registering them in the code graph; every generated design revision is a `ChangeSet` with provenance.

**Steps.**
1. Implement the continuous loop controller with deterministic scheduling and budget enforcement.
2. Implement self-model divergence measurement and reporting.
3. Implement architectural debt detection + GC with immutable-ledger protection.
4. Implement design-doc generation/update through the same change-set/promotion pipeline.
5. Implement hot-loading of promoted modules via the nexus plugin lifecycle (no restart).
6. Run the qualification task (below) and record the full provenance graph.

**Testing.** A committed qualification scenario exercised in CI: the system must produce a working,
reviewed capability with no human edits; `mm-cli replay` reproduces the run from `(code version, event
log, config)`; the entire Phase 1–12 regression suite stays green; adversarial inputs cannot bypass the
promotion gate.

**Pass gate (definition of "functional prototype").**
- Given a **novel design goal** (not in the training fixtures), the system, without human code edits: (a)
  writes a design document, (b) generates a new module with Pi, (c) passes the full promotion pipeline,
  and (d) hot-loads the capability.
- `mm-cli replay` reproduces the entire qualification run deterministically from the event log.
- The complete provenance of the new capability is queryable in RDF: `mmc:PiSession → mmc:Edit →`
  `mmc:ModuleVersion → mm:ChangeSet → mm:Promotion`.
- A self-model divergence report is produced; a debt scan + GC run completes and reports actionable findings.
- The whole workspace builds with zero warnings; the full Phase 1–12 regression suite is green.
- No identity invariant was violated during the run (checked deterministically).

---

# 13. Pass-gate summary

| Phase | Organ added | Pass gate in one line |
|---|---|---|
| 1 | Deterministic kernel + dual stores + event log | Builds clean; ULID/property tests; store round-trips; replay exactly-once; ontology loads, SHACL 0 violations |
| 2 | Modular code system + code-metadata RDF | Every file in exactly one module; `codex verify` green; SHACL 0; negative drift tests fail; idempotent scan |
| 3 | LLM substrate | Offline replay 0 calls; 100% schema-violation rejection; full accounting; deterministic routing |
| 4 | Being substrate | Identity invariants unbreakable; no belief auto-promotion; commitments monotonic; budgets enforced |
| 5 | Memory | Gold retrieval thresholds met; consolidation preserves provenance; forgetting protects ledger |
| 6 | Epistemic discipline | Forbidden promotions all rejected; contradictions explicit; dependency cascade works; world/model separated |
| 7 | Cognitive library + frames + policy genome | Seed library SHACL-clean; applicability ranked deterministically; policies versioned |
| 8 | Metacognitive controller + program compiler | Typed programs on benchmarks; deterministic OperationValue; budget respected; replayable |
| 9 | Sanity firewall + DecisionCore + comparison integrity | 3 impls conform; firewall thresholds met; hard prohibitions short-circuit; comparisons validated |
| 10 | Tool execution + verification | Unauthorized denied; sandbox enforced; observations authoritative; rollback works; ledger immutable |
| 11 | Self-engineering + Pi module construction | Gap→ChangeSet→Pi→test/benchmark→promote/reject with reason; mistake→test; calibration improves; budgets held |
| 12 | Closed-loop autonomy | Novel goal → design+code+test+promote+hot-load with no human edits; replayable; suite green; no invariant violated |

Every gate is run as a plain command (through `mm-cli` or `cargo`), and earlier gates must still pass.

---

# 14. Design traceability

| Design section | Phase |
|---|---|
| §1–2 architecture, backend philosophy | all (invariant) |
| §3–4 being substrate, identity | 4 |
| §5–8 personality, affect, user, relationships | 4 |
| §9–10 goals, planning/execution | 4, 8, 10 |
| §11–12 metacognitive controller, program compiler | 8 |
| §13–17 library, frames, doctrines, techniques, analogy | 7 (+6) |
| §18–19 sanity, bad-idea detection | 9 |
| §20 comparison integrity | 9 |
| §21–25 supposition, dependency, contradiction, missing steps, constraints | 6 (+9) |
| §26–27 risk, reversibility/optionality | 9 |
| §28–30 prediction, curiosity, attention | 6, 11 |
| §31 multi-timescale, §32–35 memory, forgetting, mistakes, habits | 5 (+12) |
| §36–39 imagination, causal, counterfactual, theory of mind | 6, 11 (causal) |
| §40–42 norms, values, resource economy | 4 |
| §43–45 budgets, policy genome, portfolios | 7, 8, 11 |
| §46–49 firewall, Jev, local models, calibration | 9, 11 |
| §50–51 symbolic layer, world model | 6, 10 |
| §52 tool execution | 10 |
| §53–54 conversation bootstrap, corrections | 4, 5, 11 |
| §55–58 self-bootstrap, capabilities, self-engineering, co-evolution | 2, 11 |
| §59–61 sandbox/promotion, mistakes→tests, data quality | 10, 11 |
| §62–66 self-model, meta-analysis, budgets, evolution journal | 11, 12 |
| §67–69 developmental stages, CI, technique discovery | 7, 11, 12 |
| §70–72 code/data improvement, change sets | 11 |
| §73–76 routing, depth, multiple passes | 3, 9 |
| §77–79 self-criticism, red team, verification strategy | 9, 10 |
| §80–83 invariants, escalation, capability map, self-prediction | 4, 9, 11 |
| §84–86 error taxonomy, architecture debt, GC | 11, 12 |
| §87–90 ontology, runtime model, ops, backend map | 1, 6, 8, 10 |
| §91–102 bootstrap phases 0–10 | 1–11 (mapped) |
| §103–107 continuous integration, control plane, self-knowledge | 11, 12 |
| §108 what not to overbuild | guardrails (§16) |
| §109–110 invariants, final system | all |

---

# 15. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Jev/hosted decision model unavailable | `DecisionCore` trait with deterministic-rules + local + hosted impls sharing one conformance suite (Phase 9); distillation path (design §48) starts from decision traces |
| Oxigraph `Store` is synchronous, blocking async runtime | Dedicated single-writer actor with bounded read handles (Phase 1) |
| Pi edits cause nondeterminism/regressions | Sandbox-only execution, restricted `--tools`, full session ingestion, deterministic build/test before promotion (Phase 11) |
| Self-modification damages the system | Immutable identity invariants, budgets, change-set rollback, promotion gate, event-log replay; production never written directly (Phases 4, 10, 11) |
| Ontology/SHACL drift from code | Code metadata generated from source + `codex verify` in CI (Phase 2) |
| LLM cost/latency explosion | Routing, caching, budget enforcement, Jev→local distillation, minimum-sufficient-cognition (Phases 3, 8, 9) |
| Reused utilities drift from their origin repos | Vendor via submodule; wrap behind `mm-*` traits; pin revisions; upstream bug fixes via `[patch]` only when necessary |
| Over-building symbolic common sense | §108 guardrails; keep symbolic layer small and focused (design §50) |

---

# 16. Guardrails and definition of done

**Anti-overbuild guardrails** (design §108, enforced in review): no giant commonsense DB; no hand-coded
personality matrix; no premature giant ontology; not every thought persisted; no LLM arithmetic; no Jev
prose; no symbolic NL understanding; no dozen independent critics; no deterministic planner for every
task; no untested self-modification; model confidence never overrides authoritative external state.

**Definition of done for the functional prototype (Phase 12):** the system, given a novel design goal,
produces a working, tested, promoted, hot-loadable capability **without human code edits**, updating its own
design documents, with complete RDF provenance, deterministic replay, enforced identity invariants and
budgets, and a green workspace-wide regression suite. That is the point at which the system can continue
the design and build process on its own.

**Build order and checkpointing:** complete phases strictly in order; after each gate, commit the
code-metadata scan, the event log, and the benchmark baselines so the phase is independently replayable.

