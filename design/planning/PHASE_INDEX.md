# Metamind — Phase Build Plan Index

> Companion index for the **12 standalone phase build plans**. The authoritative phase definitions,
> workspace conventions, logging contract, and overall gates live in
> [`implementation_plan1.md`](./implementation_plan1.md). Each `phase_NN_*_build_plan.md` is
> self-contained, follows the same 12-section template (`## 1. Objective and scope` … `## 12. Design
> traceability`), and ends in an objective `## 10. Pass gate` that includes `mm-cli logs verify`.

## Files

| Phase | Build plan | SQLite migration | Primary crates | Nexus module(s) |
|---|---|---|---|---|
| 1 | `phase_01_foundation_build_plan.md` | `0001_kernel.sql` | `mm-core`, `mm-log`, `mm-store-sqlite`, `mm-store-graph`, `mm-eventlog`, `mm-cli` | `modules/system/kernel-bootstrap` |
| 2 | `phase_02_code_metadata_build_plan.md` | `0002_codex.sql` | `mm-codex` | — (scanner; example module `case-match`) |
| 3 | `phase_03_llm_substrate_build_plan.md` | `0003_llm.sql` | `mm-llm` | — |
| 4 | `phase_04_being_substrate_build_plan.md` | `0004_being.sql` | `mm-being` | `modules/cognition/being` |
| 5 | `phase_05_memory_build_plan.md` | `0005_memory.sql` | `mm-memory` | `modules/cognition/memory-recall` |
| 6 | `phase_06_epistemic_discipline_build_plan.md` | `0006_epistemic.sql` | `mm-epistemic` | `modules/epistemic/epistemic-ops` |
| 7 | `phase_07_cognitive_library_build_plan.md` | `0007_library.sql` | `mm-library` | `modules/cognition/library-query`, `modules/cognition/frame-activate` |
| 8 | `phase_08_metacognitive_controller_build_plan.md` | `0008_metacog.sql` | `mm-metacog` | `modules/cognition/metacog` |
| 9 | `phase_09_decision_firewall_build_plan.md` | `0009_decision_firewall.sql` | `mm-decision`, `mm-firewall` | `modules/cognition/comparison-integrity` |
| 10 | `phase_10_tools_execution_build_plan.md` | `0010_tools.sql` | `mm-tools` | `modules/tools/fs-read`, `modules/tools/process-exec`, `modules/tools/verify-build` |
| 11 | `phase_11_self_engineering_build_plan.md` | `0011_self_engineering.sql` | `mm-metaanalysis`, `mm-mistakes`, `mm-selfeng`, `mm-pi` | `modules/cognition/calibration` |
| 12 | `phase_12_autonomy_build_plan.md` | `0012_loop.sql` | `mm-runtime` | `modules/cognition/closed-loop` |

All SQLite migrations live under `crates/mm-store-sqlite/migrations/`. **Migration number = phase
number** (one migration per phase), so files sort in build order and never collide.

## Cross-phase shared conventions (reconciled)

- **Identity:** ULID lowercase Crockford; instance IRIs `https://metamind.dev/data/{ulid}`; stable
  path-derived URIs for modules/files/design docs/phases.
- **Rust:** async (`tokio`), `#![forbid(unsafe_code)]`, zero warnings.
- **Stores:** SQLite via `sqlx` (WAL) for tabular state; Oxigraph 0.5 (`rocksdb-pkg-config`) behind the
  single-writer async actor; append-only event log with deterministic replay.
- **Named graphs:** `/being`, `/memory`, `/epistemic`, `/world`, `/library`, `/provenance`, `/code`.
- **Code metadata:** `mmc:` vocabulary; every module declares a stable `uri`; copied-in reference code
  lives under `vendor/<origin>/` with a `COPYING.md` and `mmc:copiedFrom` provenance (no submodules,
  no external path dependencies).
- **Modules:** nexus-style `plugin.toml` (stable `uri`, version, owning phase, ≥1 capability that names
  its tests) plus `manual/module.md`.
- **Logging:** `mm-log` structured records; event-code convention `<area>.<action>`; audit records are
  immutable and gapless; `mm-cli logs verify` runs inside every phase gate (parent plan §10).
- **External access:** LLM and Pi endpoints come from the environment only — no hard-coded URLs.
- **LLM / code editor:** cognition calls the LLM through `mm-llm`; all code editing goes through Pi RPC
  (`pi --mode rpc`), sandboxed (Phases 11–12).

## Cross-phase items to settle at build time

These are deliberate integration points; the later phase's build plan owns them:

- `ActivationCondition` is introduced in Phase 7 for both techniques and policies — settle one shared
  type in `mm-core` when it lands.
- `mm-library`'s fitness/outcome hook references an outcome type owned by Phase 6 — bind it to
  `mm-epistemic` (or `mm-core`) once Phase 6 exists.
- Phase 9's `FirewallInput` composes types from Phases 6–8 and 10 — confirm their names as each lands.
- Phase 11's regression-test link assumes the Phase 5 `mistakes` primary-key name — verify against
  `0005_memory.sql`.
