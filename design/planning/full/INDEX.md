# Metamind Extended Build Plans — Index

This directory (`design/planning/full/`) holds the **research-grounded, extended** build plan for each of
the twelve phases. Each document keeps the scope, migration number, and pass gate of its counterpart in
`design/planning/phase_NN_*_build_plan.md`, and adds:

- **§0 Research foundation (code-available)** — the ideas we borrow, each from a project whose **source is
  available**, with the decision locked in for this phase.
- **§4 Detailed specifications** — concrete types, DDL, ontology/shapes, interfaces, and the copied-in
  reference crates.
- **§6 Logging and observability** — the phase's `mm-log` event codes and required fields.
- **§7 Testing**, **§8 Pass gate**, **§9 Risks** — including the phase's `mm-cli logs verify` gate.

Every document uses the same eleven sections (`§0`–`§10`); only `§2`'s suffix varies
(`Architecture`, `Architecture and invariants`, `Architecture and identifiers`).

> Authority: `design/planning/implementation_plan1.md` (the parent plan) is normative for conventions,
> logging, the Pi contract, and traceability. Where a full plan and the parent disagree, the parent wins
> and the full plan is corrected.

---

## 1. The twelve extended plans

| Phase | Document | Objective in one line | Migration | Primary crates | Primary module(s) |
|---|---|---|---|---|---|
| 1 | [`phase_01_foundation_build_plan.md`](phase_01_foundation_build_plan.md) | Kernel: workspace, IDs/time, SQLite + RDF stores, append-only event log, `mm-log`, `mm-cli doctor/logs verify` | `0001_kernel.sql` | `mm-core`, `mm-log`, `mm-store-sqlite`, `mm-store-graph`, `mm-eventlog`, `mm-cli` | `modules/system/kernel-bootstrap` |
| 2 | [`phase_02_code_metadata_build_plan.md`](phase_02_code_metadata_build_plan.md) | Code/metadata graph: stable URIs, symbol/reference edges, `codex scan|verify`, provenance | `0002_codex.sql` | `mm-codex` (+ `mm-core::codex`) | `modules/registry.json` (generated) |
| 3 | [`phase_03_llm_substrate_build_plan.md`](phase_03_llm_substrate_build_plan.md) | LLM substrate: provider abstraction, structured output, caching/routing, accounting | `0003_llm.sql` | `mm-llm` | — |
| 4 | [`phase_04_being_substrate_build_plan.md`](phase_04_being_substrate_build_plan.md) | Being substrate: identity invariants, budgets, personality/state primitives, `being verify` | `0004_being.sql` | `mm-being` | `modules/cognition/being` |
| 5 | [`phase_05_memory_build_plan.md`](phase_05_memory_build_plan.md) | Memory: episodic/semantic/entity graph, summaries, communities, retrieval, `/memory` graph | `0005_memory.sql` | `mm-memory` | — |
| 6 | [`phase_06_epistemic_discipline_build_plan.md`](phase_06_epistemic_discipline_build_plan.md) | Epistemic discipline: claims/evidence/contradiction, PROV-O, dependency & entity edges, outcome type | `0006_epistemic.sql` | `mm-epistemic` | `modules/epistemic/epistemic-ops` |
| 7 | [`phase_07_cognitive_library_build_plan.md`](phase_07_cognitive_library_build_plan.md) | Cognitive library: techniques, policies, `ActivationCondition`, fitness/outcome recording | `0007_library.sql` | `mm-library` | `modules/cognition/{library-query,frame-activate}` |
| 8 | [`phase_08_metacognitive_controller_build_plan.md`](phase_08_metacognitive_controller_build_plan.md) | Metacognitive controller: program (`OpGraph`) compilation, ops (`Refine`/`Search`), allocation | `0008_metacog.sql` | `mm-metacog` | `modules/cognition/metacog` |
| 9 | [`phase_09_decision_firewall_build_plan.md`](phase_09_decision_firewall_build_plan.md) | Bounded decision core + sanity firewall + comparison integrity; calibration & conformal abstention | `0009_decision_firewall.sql` | `mm-decision`, `mm-firewall` | `modules/cognition/comparison-integrity` |
| 10 | [`phase_10_tools_execution_build_plan.md`](phase_10_tools_execution_build_plan.md) | Tools & execution: capability-bounded sandbox, MCP tool interface, rollback, audit | `0010_tools.sql` | `mm-tools` | tool modules under `modules/tools/` |
| 11 | [`phase_11_self_engineering_build_plan.md`](phase_11_self_engineering_build_plan.md) | Self-engineering: prediction ledger, mistake→test compiler, typed change sets, promotion gate, Pi authoring | `0011_self_engineering.sql` | `mm-metaanalysis`, `mm-mistakes`, `mm-selfeng`, `mm-pi` | `modules/cognition/calibration` |
| 12 | [`phase_12_autonomy_build_plan.md`](phase_12_autonomy_build_plan.md) | Autonomy: ten-stage closed loop, timescales, self-model divergence, debt/GC, design writer, module hot-load | `0012_loop.sql` | `mm-runtime` | `modules/cognition/closed-loop` |

**Migration rule (locked):** the migration number **is** the phase number, and every migration lives under
`crates/mm-store-sqlite/migrations/NNNN_<name>.sql` so the shared DB evolves in one ordered history.
Verified unique: `0001`–`0012`, one per phase, no duplicates.

---

## 2. Cross-cutting conventions (identical in all twelve)

- **Codename** Metamind (`mm`); crate prefix `mm-`; Rust async (`tokio`), `#![forbid(unsafe_code)]`,
  zero warnings (`clippy -D warnings`) in every phase's gate.
- **IDs / IRIs:** lowercase Crockford ULIDs, exposed as IRIs `https://metamind.dev/data/{ulid}`.
  Stable path-derived URIs for modules (`.../code/module/{path}`) and design anchors
  (`.../design/{doc}#{anchor}`).
- **State:** SQLite via `sqlx` 0.8 (WAL, `journal_mode=WAL`) for tabular state; Oxigraph 0.5 for RDF;
  an **append-only event log** whose state is a deterministic fold (no authoritative mutable state table).
- **External code is reference only:** copied into `vendor/<origin>/` as workspace members with
  `vendor/<origin>/COPYING.md` and `mmc:copiedFrom` (repo, revision, license) emitted by the Phase 2 scan.
  No git submodules, no path dependencies, no `[patch]`.
- **Modular code:** nexus-style plugin modules with `plugin.toml` (stable `uri`, `version`,
  `owned_by_phase`, `capability`, T-Box functions).
- **Logging:** all records via `mm-log` (structured, ULID `trace_id` correlation, five sinks, redaction);
  `mm-cli logs verify` is a pass-gate item in **every** phase.
- **Endpoints:** never hard-coded — the LLM endpoint comes from `OPENAI_BASE_URL` / `OPENAI_API_KEY`; Pi is
  invoked from the installed binary (`pi 0.85.1`) over its documented RPC.
- **Invariant:** the LLM proposes; the control plane structures; the decision core makes bounded judgments;
  symbolic layers prove; deterministic code enforces; the environment decides truth; memory preserves;
  meta-analysis improves. **Production is written only by promotion.**

---

## 3. Deferred cross-phase integration points (must be settled when the owning phase lands)

These were flagged while writing the plans. Each is a real seam; the resolution below is the intended one.

| # | Item | Owned by | Notes / intended resolution |
|---|---|---|---|
| D1 | `ActivationCondition` | Phase 7 (`mm-library`) | Phase 7 needs it for **both** techniques and policies. Settle **one shared type in `mm-core`** and have `mm-library` re-export it; do not define two. |
| D2 | Library fitness/outcome type | Phase 6 → Phase 7 | `LibraryManager::record_fitness` takes the **Phase 6 `mm_epistemic::Outcome`**; Phase 7 must not define a parallel outcome type. |
| D3 | `FirewallInput` / `DecisionState` composition | Phase 9 | Phase 9 composes types from Phases 6/7/8/10. `LocalCore` (Phase 11) and `Factuality::SearchAugmented` (Phase 10) are seams that return `Unavailable` until those phases land — the firewall must degrade to rules + escalate, never fabricate. |
| D4 | `entity_edges` ownership | Phase 5 vs Phase 6 | Both plans add entity/dependency edges. **Owner: Phase 5** (`0005_memory.sql`); Phase 6 references `entity_edges` and adds only its own epistemic/dependency edges on top — no duplicate table. |
| D5 | `regression_tests.mistake_ulid` FK target | Phase 5 → Phase 11 | Phase 11 assumes the Phase 5 `mistakes` table and its PK name; confirm the PK column name when Phase 5 lands and align the FK (Phase 11 also owns the `budgets` table naming). |
| D6 | Program representation | Phase 8 | Phase 8 renamed the metacognitive program to **`OpGraph`**; the ordered step list is retained as a projection. Added ops `Refine`, `Search` and event `metacog.force.decide`. Later phases consume `OpGraph`, not the old "program steps" shape. |
| D7 | Loop SQLite path | Phase 12 | The loop's tables live in `crates/mm-store-sqlite/migrations/0012_loop.sql` (shared migrations path), **not** under `mm-runtime/`. Corrected from the original Phase 12 plan. |
| D8 | Promotion gate | Phase 11 → Phase 12 | Phase 12 **reuses** the Phase 11 `PromotionGate` unchanged; it does not re-specify promotion. GC actions at or above `Code` on the escalation ladder flow through that same gate. |

---

## 4. How to read a plan

1. **§0** — why each mechanism exists, and its code-available source. If an idea had no available code, it
   was dropped.
2. **§1–§3** — scope, architecture/invariants, and the exact files the phase produces.
3. **§4** — the specification to build to: types, DDL, ontology, interfaces, copied-in crates.
4. **§5–§8** — the ordered build sequence, logging contract, tests, and the executable pass gate.
5. **§9–§10** — risks/mitigations and the reference list.

Every pass gate ends with `mm-cli logs verify`, and Phase 12's gate additionally proves the whole loop
deterministically replays and that a novel goal reaches a promoted, hot-loaded capability **with no human
code edits**.
