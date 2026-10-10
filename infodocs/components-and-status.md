# Metamind — system components, capabilities, and status

> **What this document is.** A component-by-component account of what exists in this repository, what each
> part can do, where it is deliberately limited, what is tested today and what is not, how the
> implementation holds up, and what comes next. It is written from the tree, not from the plans: every
> claim was read out of the code or observed by running the built CLI.
>
> **Companion documents.** [`user-guide.md`](./user-guide.md) — how to build, run, and use the system.
> [`open-gaps.md`](./open-gaps.md) — the backlog of work the twelve phases left unbuilt.
>
> **State when written.** `HEAD = 0204d73` ("Initial 12 phase build complete"), working tree clean,
> 2026-10-09. See ["How this was verified"](#how-this-was-verified) for the commands that reproduce every
> number below.

---

## 1. The system in numbers

| Dimension | Value | Where it comes from |
|---|---|---|
| Workspace crates | **21** (`mm-core` … `mm-runtime`) | `crates/*/Cargo.toml` |
| Nexus modules | **10** under `modules/` | `modules/*/*/plugin.toml` |
| Copied-in reference crates | **7** under `vendor/rust_symbolic/` | `vendor/rust_symbolic/COPYING.md` |
| SQLite migrations | **12** (`0001_kernel` … `0012_loop`), one per phase | `crates/mm-store-sqlite/migrations/` |
| SQLite tables (fresh data dir) | **110** | `sqlite_master` on a fresh `--data-dir` |
| Named RDF graphs | **11** declared; a fresh boot keeps the ontology in the default graph, and named graphs fill as data is written | `crates/mm-core/src/iri.rs:59` |
| Ontology files | **8** plus 1 seed document | `ontology/*.ttl` |
| SHACL shape files | **11** | `ontology/shapes/*.ttl` |
| Ontology triples loaded at boot | **1450** | `mm-cli doctor` |
| Structured log event codes | **197** | `crates/mm-log/src/codes.rs` |
| CLI subcommands | **42** plus `help` | `mm-cli --help` |
| Test functions in the tree | **1776** `#[test]`/`#[tokio::test]` attributes | static count |
| Tests executed by `cargo test --workspace` | **1747 passed, 0 failed, 0 ignored**, 152 test binaries | measured run |
| Integration test files | **59** (`crates/*/tests/`, `modules/*/*/tests/`) | file count |
| End-to-end gate files | **15** targets in `tests/e2e/` (+ `lib.rs` helpers) | `tests/e2e/Cargo.toml` |
| Benchmark/fixture corpora | **71** directories / **134** files under `bench/` (42 corpora at the top two levels) | `find bench -type d \| wc -l` |
| CI | GitHub Actions, one `phase-1 gate` job + a Phase 12 gate block | `.github/workflows/ci.yml` |

The governing invariant, restated from the parent plan and enforced throughout the tree:

> The LLM proposes; the control plane structures; the decision core makes bounded judgments; symbolic
> layers prove; deterministic code enforces; the environment decides truth; memory preserves;
> meta-analysis improves. **Production is written only by promotion.**

Two consequences show up in almost every component below: (a) anything that cannot be decided says
`Unavailable` and escalates instead of inventing a value, and (b) nothing in the production tree changes
without a change set, a sandbox, a gate, and a promotion row.

---

## 2. Layout

```
metamind/
├── crates/                 # 21 Rust crates (see §3)
├── modules/                # 10 nexus modules: plugin.toml + src + tests + manual/module.md
├── vendor/rust_symbolic/   # 7 copied-in reference crates + COPYING.md
├── ontology/               # 8 T-Box documents + 11 SHACL shapes + seed/library_seed.ttl
├── bench/                  # 30 fixture corpora (episodes, calibration, firewall, promotion, …)
├── pi/                     # the code agent's assets: system_prompt.md, 2 skills, 1 prompt, 1 extension
├── tests/e2e/              # the end-to-end gates
├── config/metamind.toml    # the kernel configuration
├── design/                 # the design corpus + planning/ (parent plan, 12 short + 12 full plans)
├── data/                   # the default data dir (SQLite, RocksDB graph, logs, sandboxes)
├── codex.lock              # generated: every module's version + content hash
└── .github/workflows/ci.yml
```

`data/` is the only mutable state, and it is not authoritative: the append-only event log is (§4).

---

## 3. Component catalogue

Each component below lists **Purpose**, **What it can do**, **Limits**, **Tested by**, and **Quality**.
Test counts are the `#[test]`/`#[tokio::test]` functions in that crate; the executed total is 1747 (§8).

### 3.1 Kernel and substrate (Phases 1–3)

#### `mm-core` — kernel core (Phase 1, 37 tests, 0 integration files)

- **Purpose.** Identity, time, configuration, hashing, IRI construction, and the three store traits every
  other crate programs against.
- **Can do.** Lowercase Crockford ULIDs with a persisted watermark; `Timestamp` with strict RFC3339
  handling (9 fractional digits out, UTC-only in — a local offset is rejected rather than guessed);
  `content_hash`/`hash_fields` (separator-based, so `["ab","c"]` ≠ `["a","bc"]`); the single IRI funnel
  (`data`, `mm`, `mmc`, `graph`, `module`, `vendor`, `design`) and `NAMED_GRAPHS`; `Config` loading with
  every path resolved against the repository root; `Param`/`Params`, `NewEvent`, `AuditRecord`, and the
  `Tabular`/`Graph`/`EventSink` traits.
- **Limits.** `Config::with_env_overrides` (which honours `MM_DATA_DIR`) exists but is **never called** —
  the CLI does not apply it, so `MM_DATA_DIR` does nothing in practice even though the module docs and
  `README.md` say it overrides discovery. The README also says `NAMED_GRAPHS` lists "the seven runtime
  graphs" while `iri.rs:59` declares eleven — documentation drift, not code drift.
- **Tested by.** In-file unit tests (IRI parsing, time round-trips, hashing collisions, config paths).
- **Quality.** Small, fully documented surface; no `unsafe`; no I/O beyond config reading. The two drift
  items above are worth a five-minute fix.

#### `mm-log` — structured logging (Phase 1, 20 tests, 0 integration files)

- **Purpose.** The one way anything in the system reports what it did: structured records, stable event
  codes, sinks, redaction, and an audit stream.
- **Can do.** 197 event codes in a stable `<area>.<action>` namespace; records carry `trace_id`
  correlation, target, level, fields; sinks are console + JSONL (`data/logs/mm.jsonl`); audit records
  forward to the immutable audit chain (`audit_log` table + optional `audit.jsonl`); redaction is applied
  before a record reaches a sink; `mm-cli logs verify` re-hashes and validates the whole thing.
- **Limits.** Deliberately in-process: no remote sink, no log rotation, no sampling. The audit chain is
  SQLite-backed, so its integrity is as strong as the local file.
- **Tested by.** Golden-record snapshots (`crates/mm-log/tests/golden_records.rs` + an `insta` snapshot of
  every code), schema checks, and the `logs verify` step present in every phase gate.
- **Quality.** High. The golden snapshot pins every event code's shape, which is what makes log-based
  assertions in tests stable.

#### `mm-store-sqlite` — tabular store (Phase 1, 20 tests, 1 integration file)

- **Purpose.** The `sqlx`/SQLite pool, the phase-numbered migration runner, bitemporal as-of reads, and the
  audit chain.
- **Can do.** WAL mode; 12 migrations applied in order (`schema_versions` holds 12 rows); 110 tables on a
  fresh data dir; a single pool handed to every crate; as-of reads by event/commit sequence; audit append
  with chain verification; the expected-table guard used by `doctor`.
- **Limits.** One writer, like SQLite itself. There is no sharding or replication; `store.pool` is exposed
  by design so crates like `mm-tools` and `mm-eventlog` can own their own projections in the same file.
- **Tested by.** `crates/mm-store-sqlite/tests/migrations.rs` (up/down per migration, expected tables),
  down-migration coverage in `migrations_down/`, plus every other crate's tests as clients.
- **Quality.** High. Migrations include their `down` counterparts, which many projects skip.

#### `mm-store-graph` — RDF store (Phase 1, 17 tests, 0 integration files)

- **Purpose.** Oxigraph 0.5 behind a single-writer async actor, named graphs, canonical hashing, SHACL
  validation.
- **Can do.** `replace_turtle`, SPARQL, per-graph load/clear, canonical quad hashing (so a graph's hash is
  reproducible), SHACL validation with named violations; 11 named graphs; the store is `rocksdb-pkg-config`
  backed (`data/graph/`).
- **Limits.** The actor serialises writes — correct, not fast, under heavy write concurrency. RocksDB is a
  system dependency (`librocksdb-dev`), which is why CI installs it before building.
- **Tested by.** Unit tests plus the graph-shape gates (`graph validate --graph <name>`) that every phase
  runs, and the `mm-cli doctor` graph invariant check.
- **Quality.** High; the single-writer actor removes the whole class of "two quads writers" bugs.

#### `mm-eventlog` — append-only event log (Phase 1, 15 tests, 1 integration file)

- **Purpose.** The authoritative history: events are appended, never edited, and state is a fold over them.
- **Can do.** Bitemporal commit (provisional → committed / aborted); deterministic replay with checkpoints
  (`store_checkpoints`); a state hash after each fold, which is what makes replay provable; `mm-cli replay
  --from/--until/--as-of`.
- **Limits.** Replay is single-threaded and re-reads the log; there is no compaction story beyond
  checkpoints. The log grows without bound by design (production history is not to be rewritten).
- **Tested by.** `crates/mm-eventlog/tests/replay.rs`, `tests/e2e/replay_determinism.rs` (same hash across
  runs *and* across processes), and `logs verify`'s replay-determinism check.
- **Quality.** High — determinism is asserted rather than assumed, at three levels (unit, e2e, CLI gate).

#### `mm-cli` — operator CLI (Phase 1, 38 tests, 0 integration files)

- **Purpose.** The single operator surface: 42 subcommands that open the kernel exactly as the runtime
  does.
- **Can do.** Every capability of every phase is reachable here — see the command reference in the
  [user guide](./user-guide.md#7-command-reference). Global `--config`/`MM_CONFIG` and `--data-dir`; most
  commands accept `--json`; commands that consult a decision core accept `--core`/`--rules`/`--head`.
- **Limits.** It is a thin façade: it holds no state of its own, and a handful of commands (`tool run`,
  `action rollback`) mutate the *repository's* `data/sandbox`, not the `--data-dir` you pass.
- **Tested by.** `tests/e2e/cli_*.rs`, and effectively every phase gate, which is a sequence of CLI
  commands.
- **Quality.** High: it is the most-exercised crate in the tree and stays out of the way.

#### `mm-codex` — code metadata (Phase 2, 138 tests, 6 integration files)

- **Purpose.** The deterministic scanner that turns the workspace into the `/code` graph: modules, files,
  symbols, references, capabilities, provenance.
- **Can do.** `scan` (rebuild `/code`, regenerate `modules/registry.json`), `verify` (every code-metadata
  rule; currently `39 modules conform`), `graph` (module dependency graph), `diff` (interface drift, or a
  lock diff), `meta` (TotalModules, ModulesPerPhase, SymbolsPerModule, CapabilitiesWithoutTests,
  DependencyCycles, VersionDrift, Churn, OrphanFiles, CopiedFrom, UnreferencedSymbols), `lock`
  (write/check `codex.lock`).
- **Limits.** Rust only as a *parser* target (`syn`), plus the nexus `plugin.toml`/TOML manifests; a module
  written in another language would be seen as files without symbols. Capabilities are read from
  `[package.metadata.metamind]`/`plugin.toml`, never from a tool's `module_uri` — which is why the dangling
  tool-module IRIs in [`open-gaps.md`](./open-gaps.md#g3) go unreported.
- **Tested by.** 6 integration files including `verify_rules.rs` (planted-defect fixtures in
  `bench/codex/fixtures/`), `golden.rs` (a golden `/code` graph), `idempotence.rs` (scan twice ⇒ identical
  output), and `meta_queries.rs`.
- **Quality.** Very high. The negative fixtures make the rule set falsifiable, and the lock file makes
  unversioned change detectable.

#### `mm-llm` — LLM substrate (Phase 3, 99 tests, 6 integration files)

- **Purpose.** Typed, cached, accountable model access with schema-valid output and offline replay.
  How a call is made, how prompts are assembled, and how tools relate to the model is the subject of
  [`llm-integration.md`](./llm-integration.md).
- **Can do.** One provider adapter (`openai.rs`) plus a mock; a strict `StructuredOut` path with schema
  validation; grammar/decoder backends (`llguidance`, `xgrammar`); a cache index with re-hashing; routing;
  accounting (`llm_calls`); provenance and redaction; replay of recorded sessions with the network guard
  armed.
- **Limits.** **No live-provider call has ever been made in this environment.** With `OPENAI_BASE_URL`/
  `OPENAI_API_KEY` unset, every hosted path degrades to `Unavailable` (`mm-cli conformance` prints exactly
  that). Grammar quality is measured against fixtures, not against a real model's outputs. There are no
  `#[ignore]`d live tests and no recorded golden traffic from a real provider.
- **Tested by.** `accounting.rs`, `cache_replay.rs`, `grammar_conformance.rs`, `no_hardcoded_endpoint.rs`
  (endpoints come only from the environment), `routing.rs`, `schema_reject.rs` (every malformed fixture is
  rejected for the right reason), plus `bench/llm/` fixtures.
- **Quality.** High for the substrate, unproven against reality — an honest split that the docs state
  rather than hide.

### 3.2 Being and memory (Phases 4–5)

#### `mm-being` — identity substrate (Phase 4, 96 tests, 6 integration files)

- **Purpose.** Who the being is and what it must never do: identity invariants, core blocks, personality,
  affect, user model, relationships, goals, commitments, resource accounts.
- **Can do.** `being show/verify/invariants/block/belief/goal/commitment/relationship/affect/budget`;
  four enforced invariants (`no_fabricated_autobiography`, `no_assumption_to_observation`,
  `no_action_without_evidence`, `no_history_rewrite`); affect as a separate projection from personality; a
  budget ledger with policies; SHACL validation of `/being`.
- **Limits.** Invariants are enforced on the paths that exist — there is no ambient enforcement at the
  SQLite level, so a direct write to the DB outside the API is only caught by `being verify`/audit, not
  prevented.
- **Tested by.** 6 integration files (`identity_invariants.rs`, `belief_promotion.rs`,
  `commitment_lifecycle.rs`, `affect_separation.rs`, `budgets.rs`, `rdf_roundtrip.rs`), `cli_being.rs`, and
  `being verify` (which reports **22 of 22** adversarial cases denied against
  `bench/being/invariants.jsonl`).
- **Quality.** High, with a real adversarial corpus rather than happy-path checks.

#### `mm-memory` — long-term memory (Phase 5, 126 tests, 7 integration files)

- **Purpose.** Ten memory classes, hybrid retrieval, consolidation, and time-decayed forgetting, mirrored
  into `/memory`.
- **Can do.** `memory add/get/recall/consolidate/forget/stats/eval/verify/mistake/procedure/index`;
  lexical + vector + graph + recency + importance scoring (each term visible in `recall` output); FTS5
  index; communities and summary trees; protected records that forgetting may not touch; `memory eval`
  graded over a gold set with committed thresholds; SQLite↔`/memory` reconciliation.
- **Limits.** The vector index is a packed local index, not an embedding service: vectors are produced
  deterministically, so semantic recall quality is bounded by that. Forgetting is a dry run by default and
  never hard-deletes.
- **Tested by.** `adversarial.rs`, `consolidation.rs`, plus 4 more integration files;
  `tests/e2e/cli_memory.rs` asserts a fixture round-trip, consolidation reconciliation, the dry-run rule,
  and that `memory eval` meets its thresholds.
- **Quality.** High; the reconciliation check (SQLite vs graph) is the kind of invariant most projects
  skip.

### 3.3 Epistemic discipline and the cognitive library (Phases 6–7)

#### `mm-epistemic` — claims, evidence, justification (Phase 6, 88 tests, 8 integration files)

- **Purpose.** The discipline that separates what is observed from what is assumed from what is merely
  claimed, with provenance (PROV-O) and a justification graph.
- **Can do.** Typed claims with statuses; a monotone promotion guard (status can only go up, and only with
  evidence); assumptions with a verification priority; contradictions recorded, never silently resolved;
  dependency edges; `invalidate` (withdraw support and retract what rested on it); `/world` restricted to
  OBSERVED or VERIFIED.
- **Limits.** Two documented degradations: contradiction *detection* stays structural because no ASP/SMT
  backend is vendored (`FormalCheck::Unavailable`, with the `FormalBackend` seam in place), and the
  contradiction engine is indexed rather than complete — it compares claims within a `(subject, predicate)`
  bucket by design.
- **Tested by.** 10 integration files (`promotion_guard.rs`, `contradiction.rs`, `data_tests.rs`,
  `justification.rs`, `world_separation.rs`, `verification_priority.rs`, `replay.rs`, …) plus
  `tests/e2e/epistemic_gate.rs` (the Phase 6 gate, including a report that must never reach `/world`).
- **Quality.** Very high. This is the crate with the most adversarial coverage relative to its size, and
  both degradations are recorded at the site.

#### `mm-library` — cognitive library (Phase 7, 120 tests, 9 integration files)

- **Purpose.** Techniques, policies, cases, frames, insights and doctrines as typed, versioned entries
  behind a SHACL commit gate; plus a verified skill library and structure-mapped case retrieval.
- **Can do.** `library import/validate/orphans/list/applicable/extract`, `skill register/verify/retrieve`,
  `case retrieve`, `experience compile`, `policy history/fitness/check/evolve`, `frame compose/missing`;
  fitness/outcome recording against the Phase 6 `Outcome` type (one type, not two); immutable policy
  versions; a policy genome with retained-gain evolution; sub-threshold drafts from trajectories.
- **Limits.** `experience compile` produces *draft candidates*, never promoted entries — revision is a
  later, gated step. Applicability ranking is structural (activation conditions), not learned.
- **Tested by.** 10 integration files including `shacl_gate.rs`, `applicability_gold.rs` (gold table),
  `skill_verify.rs`, `genome_evolve.rs`, `case_retrieval.rs`, plus `tests/e2e/library_gate.rs` (the Phase 7
  gate and import idempotence).
- **Quality.** High; the SHACL gate plus gold tables make both correctness and refusals testable.

### 3.4 Control and judgment (Phases 8–9)

#### `mm-metacog` — metacognitive controller (Phase 8, 84 tests, 2 integration files)

- **Purpose.** How a question gets *structured*: cognitive episodes, the operation algebra, `OpGraph`
  compilation, the tier/policy ladder, and deterministic op selection.
- **Can do.** Episode compile/run; operations (`Refine`, `Search`, …) lowered into an `OpGraph` with a
  retained ordered-step projection; deterministic tie-breaks; budget-aware op choice; program traces for
  replay; `mm-cli episode run/replay/verify/value/budget-audit/program`.
- **Limits.** The LLM-facing scan (structured proposal) needs a provider; without one the controller runs
  its deterministic half. Tier selection is a ladder with fixed costs, not a learned policy.
- **Tested by.** Unit tests, `crates/mm-metacog/tests/trace_store.rs`, the `metacog` module's behaviour
  tests, and `tests/e2e/episode_roundtrip.rs` (corpus runs → persists → replays byte-identically, plus
  tampered-program and tampered-trace refusals).
- **Quality.** High. "Tamper with the trace and it is refused" is exactly the right test for this layer.

#### `mm-decision` — bounded decisions (Phase 9, 140 tests, 1 integration file)

- **Purpose.** One bounded question shape, three interchangeable backings (rules/local/hosted), one
  conformance suite, and deterministic risk arithmetic.
- **Can do.** `decide`, `conformance`, `compare check`, `risk analyze`, `calibration fit/conformal`;
  confidence bounded by construction; a state digest so a cached answer cannot be replayed against the
  wrong state; eight risk measures checked against a reference table (`bench/risk/reference_values.json`);
  split-conformal thresholds per class; `LocalCore` over a distilled head; hosted core over `mm-llm`.
- **Limits.** **`--core local` has no producer**: a head can be *loaded* but nothing *fits* one, so the
  local core is `Unavailable` in practice (see [`open-gaps.md` G1](./open-gaps.md#g1)). **Factuality is
  library-only**: `check_factuality` has no caller outside its tests, `search_augmented` is unimplemented,
  and the `factuality_checks` table has no writer (G2). The hosted core needs credentials nobody has
  configured here.
- **Tested by.** `crates/mm-decision/tests/conformance.rs` (the one suite across cores, including the
  legal-refusal path), unit tests for every risk measure, and `tests/e2e/phase09_firewall.rs` (the Phase 9
  gate, which asserts that local and hosted are *reported unavailable* rather than counted as passes).
- **Quality.** High for what is reachable; the unreachable paths are documented rather than faked.

#### `mm-firewall` — sanity firewall (Phase 9, 62 tests, 0 integration files)

- **Purpose.** The pre-action check: deterministic prohibition rails first, bounded judgments second,
  escalation when uncertain.
- **Can do.** `firewall eval` over a template or a corpus with precision/recall thresholds; ordered
  prohibitions with short-circuit; precedence aggregation; `VERIFY_FIRST`/`PROCEED`/`HUMAN_REVIEW`
  outcomes; uncertainty and calibration pressure as signals; a report with per-signal magnitudes.
- **Limits.** It consumes `factuality_support` as an *input*; nothing in the tree computes it, so that
  signal is inert unless an operator supplies it (G2). With no core configured it runs its deterministic
  half only — by design, and it says so.
- **Tested by.** Unit tests including the low-factuality → `VERIFY_FIRST` path, corpus grading against
  `bench/firewall/gold.jsonl`, and the Phase 9 e2e gate.
- **Quality.** High. Refusal semantics are explicit and tested.

### 3.5 Action (Phase 10)

#### `mm-tools` — tool execution (Phase 10, 184 tests, 0 integration files)

- **Purpose.** The only way the system touches the outside world: MCP-shaped tool protocol, default-deny
  authorization, tiered sandboxes, exactly-once execution, and an append-only action ledger.
- **Can do.** 9 registered tools (`fs.read/write/list`, `process.exec`, `graph.query`, `tabular.query`,
  `http.fetch`, `verify.run`, `pi_editor.open`); `tool list/describe/run`; `action ledger/show/rollback`;
  `verify run` obligations (`cargo-build`, `cargo-test`, `static-analysis`, `symbolic`, `math-residual`);
  `mcp list/call/serve` (stdio JSON-RPC); `policy check` using the same engine as the executor;
  idempotency keys (a verbatim retry deduplicates); hash-chained ledger; byte-identical rollback from
  snapshots; permission grants with path/command scoping.
- **Limits.** Four, all documented at the site:
  1. **Tier 2/3 have no runtime.** `gvisor`/`microvm` are off by default and their production default is
     `AbsentRuntime`, which refuses with `TierUnavailable`. So `pi_editor.open` (tier `MicroVm`) can never
     run, and any future tier-2 tool refuses until a runtime is installed (G5).
  2. **`pi_editor.open` is a stub** whose client was superseded by `mm-pi` + `mm selfeng` (G4).
  3. **The nine tool `module_uri` IRIs dangle** — the modules they name do not exist, and neither
     `codex verify` nor SHACL can see it (G3).
  4. **JSON ULIDs are uppercase in this crate's output.** `action ledger --json` and `tool run --json`
     print `01M4HVQH…` because `mm_tools::action::ActionResult.action_id` is a bare `mm_core::Ulid` with no
     `serde_ulid` attribute, while the human-readable path goes through `mm_core::ulid_string()`
     (`crates/mm-cli/src/tools_cmd.rs:357`). Every other surface prints lowercase. Fixing it is a
     `#[serde(with = "mm_core::serde_ulid")]` on three fields.
  The tool sandbox root is the repository's `data/sandbox`, **not** the `--data-dir` you pass to the CLI.
- **Tested by.** 184 unit tests — the largest suite in the tree — covering the permission matrix, sandbox
  lifecycle, idempotency, ledger chain, rollback, MCP framing, and each tool; plus
  `tests/e2e/phase10_tools.rs` (the Phase 10 gate) and `bench/tools/` fixtures including an adversarial
  undeclared-network case.
- **Quality.** Very high — this is the most safety-critical crate and the best covered.

### 3.6 Self-engineering (Phase 11)

#### `mm-metaanalysis` — self-observation (Phase 11, 51 tests, 1 integration file)

- **Purpose.** Measure, diagnose, and learn: the prediction ledger, calibration scoring, trigger/diagnosis,
  and lessons.
- **Can do.** `calibrate --bench` (Brier, log loss, ECE, coverage, selective risk, with baseline
  comparison and `--assert-*` thresholds); `meta analyze/queue/lessons`; trigger extraction
  (`repeated_failure`, …), error classification, ROI-ish budgets; append-only predictions.
- **Limits.** Diagnoses are rule-based over recorded evidence; no learned classifier sits behind them.
- **Tested by.** Unit tests, the `calibration` module (16 tests), `tests/e2e/selfeng_cycle.rs`, and the
  golden `bench/golden/` numbers.
- **Quality.** High; the assertion flags make it usable as a gate, not just a report.

#### `mm-mistakes` — mistake→regression-test compiler (Phase 11, 20 tests, 0 integration files)

- **Purpose.** Turn an incident into something a machine checks forever: minimize it, emit a test plus its
  fixture into `bench/regression/`.
- **Can do.** `regression run --suite bench/regression` (each case must fail before and pass after);
  minimization; fixture emission; linkage to the Phase 5 `mistakes.id` row.
- **Limits.** Emits Rust/`bench` artifacts for Rust-shaped incidents; the reducer is structural, so a
  non-reproducible incident yields a case with no `fail-before` half.
- **Tested by.** Unit tests plus the seeded case in `bench/regression/seeded_bug_01/` (observed:
  `fails_before true passes_after true`).
- **Quality.** High for a small crate.

#### `mm-selfeng` — change sets, sandbox, gate (Phase 11, 71 tests, 0 integration files)

- **Purpose.** The only path from an idea to production: typed change sets, a git-worktree sandbox,
  benchmark and shadow comparison, and a deterministic promotion gate.
- **Can do.** `changeset new/show`, `sandbox run --apply/--build/--test/--assert-isolated`, `promote
  --assert-reason-present`, `reject --reason`, `budget show`, `audit production-tree`; every promotion
  carries a written reason and a new self version; isolation is asserted (nothing outside the sandbox
  changes).
- **Limits.** The sandbox is a worktree plus a filesystem audit, not a network-isolated VM; a candidate
  that needs network is refused rather than contained. The gate is deterministic and evidence-based — it
  never *adds* judgment the operators did not configure.
- **Tested by.** Unit tests plus `tests/e2e/selfeng_cycle.rs` (the Phase 11 gate: promote with a reason,
  reject with a reason and write the lineage) and `tests/e2e/gate_bypass_adversarial.rs` (five ways of
  trying to buy a promotion, all refused).
- **Quality.** Very high. The adversarial file is the strongest single piece of evidence in the tree that
  the promotion path is not bypassable.

#### `mm-pi` — Pi code-agent client (Phase 11, 44 tests, 0 integration files)

- **Purpose.** The only code editor: JSONL framing, the id-correlated RPC protocol, session ingestion.
- **Can do.** `PiClient` spawn/prompt/steer/follow-up/abort/clear-queue/new-session/next-event/shutdown;
  ingestion of a recorded session into rows + RDF; `mm-cli pi run --task … --offline` replays the recorded
  session deterministically (observed: 8 events, 3 edits, 2 tool calls, 30 triples); a live run needs a
  provider name and the `pi` binary.
- **Limits.** LF-only framing and the documented RPC subset, so a Pi version with new message kinds would
  need a client update. Live runs need credentials plus the binary; here `pi 0.85.1` is installed at
  `/home/mfrager/.nvm/versions/node/v26.3.0/bin/pi`, but the offline path is what every gate uses.
- **Tested by.** Unit tests including a fake-`pi` script over the real framing, plus
  `tests/e2e/pi_replay.rs` (a recorded session replays to the same rows and RDF; a truncated session is
  refused rather than partly ingested).
- **Quality.** High; the truncation refusal is the detail that matters.

### 3.7 Autonomy (Phase 12)

#### `mm-runtime` — the closed loop (Phase 12, 56 tests, 0 integration files)

- **Purpose.** The ten-stage developmental loop, the five-timescale scheduler, the numeric self-model, debt
  scanning, reversible GC, the design writer, and module hot-loading.
- **Can do.** `loop run/status/artifacts/replay/resume`, `self-model report`, `debt scan`, `gc --apply`,
  `module load/rollback`, `design revise/history`, and `bin/mm-loop` as a supervised process with
  readiness output and ticking clocks. A run commits all ten stages, replays byte-identically, promotes a
  version, hot-loads it, and writes a design revision — observed end-to-end on the qualification goal
  (see the [user guide](./user-guide.md#56-the-flagship-run-the-whole-loop)) in ~0.33 s using the recorded
  Pi session.
- **Limits.** The loop's Pi stage uses the recorded session unless a live provider is configured; the five
  clocks require a long-running process to matter; GC refuses anything that is part of the immutable
  developmental ledger (observed: 5 proposed, 1 refused, `protected_untouched: true`).
- **Tested by.** Unit tests plus `tests/e2e/phase12_autonomy.rs` (the full gate + a run that cannot produce
  its capability recording the refusal), `hot_load.rs` (a promoted capability loaded without a restart),
  and `gate_bypass_adversarial.rs`.
- **Quality.** Very high: the same run is asserted at three levels — stage outcomes, replay digest, and
  hot-load — and the refusal path is tested too.

### 3.8 Nexus modules (10)

Each module is a workspace member with `plugin.toml`, `src/lib.rs`, `tests/behavior.rs`, and
`manual/module.md`, and is registered in `modules/registry.json`.

| Module | Phase | Capability | T-Box function(s) | Tests |
|---|---|---|---|---|
| `system/kernel-bootstrap` | 1 | `mm:KernelBootstrap` | boot/self-check | 8 |
| `cognition/being` | 4 | `mm:BeingState` | `being.identity`, `.personality`, `.affect`, `.beliefs`, `.goals`, `.relationships`, `.resources` | 9 |
| `cognition/memory-recall` | 5 | `mm:MemoryStore` | `memory.recall` | 9 |
| `epistemic/epistemic-ops` | 6 | `mm:EpistemicOps` | claim/promotion ops | 6 |
| `cognition/library-query` | 7 | `mm:TechniqueSelection` | `library.applicable` | 9 |
| `cognition/frame-activate` | 7 | `mm:FrameActivation` | `frame.compose` | 10 |
| `cognition/metacog` | 8 | `mm:CognitiveControl` | `cognition.plan`, `cognition.op_value` | 9 |
| `cognition/comparison-integrity` | 9 | `mm:ComparisonIntegrity` | comparison checks | 11 |
| `cognition/calibration` | 11 | `mm:CalibrationSummary` | calibration summary | 16 |
| `cognition/closed-loop` | 12 | `mm:ClosedLoopAutonomy` | loop stages | 17 |

**Limits.** None of these module dirs are the tool layer: the Phase 10 plan named
`modules/tools/{fs-read,process-exec,verify-build}/`, which do not exist (G3). Modules are Rust crates
compiled into the workspace, not dynamically loaded binaries — "hot-load" means the runtime's active
*version pointer* changes without a restart, not that a new `.so` is dlopen'd.

### 3.9 Copied-in reference code (`vendor/rust_symbolic/`)

Seven crates (`decision-ir`, `logic-ir`, `logic-types`, `math-core`, `math-types`, `rdf-codec`,
`rdf-shacl`) copied into the repository as workspace members with `COPYING.md` recording repo, revision,
and license. They supply the exact risk/decision semantics (`decision-ir`) and the algebraic/RDF plumbing
the `mm-*` crates wrap. **Limit:** they are reference code — no submodules, no external path dependencies,
and `mm-*` interfaces are the only public surface.

### 3.10 Assets: Pi skills, prompts, and fixtures

`pi/system_prompt.md`, `pi/skills/mm-module-scaffold/SKILL.md`,
`pi/skills/mm-design-revise/SKILL.md`, `pi/prompts/module_scaffold.md`,
`pi/extensions/metadata_emit.ts`, and the recorded sessions under `crates/mm-pi/fixtures/`. These are the
inputs to the code-agent stage; the recorded session is what makes the qualification run deterministic.

---

## 4. Storage, ontology, and observability

### 4.1 The 110 tables, by phase

A fresh `mm-cli doctor` creates 110 tables. They group by the phase that owns the migration:

Each table is created by exactly one migration (verified against the `CREATE TABLE` statements):

| Migration | Tables it creates |
|---|---|
| `0001_kernel` | `events`, `audit_log`, `store_checkpoints`, `facts`, `schema_versions` |
| `0002_codex` | `module_index`, `module_file`, `symbol_index`, `symbol_reference`, `capability_index`, `module_dep`, `codex_run` |
| `0003_llm` | `llm_calls`, `llm_cache_index`, `llm_semantic_cache`, `llm_repair_attempts`, `llm_routing`, `llm_grammars` |
| `0004_being` | `identity`, `invariants`, `core_blocks`, `personality_values`, `personality_dispositions`, `personality_context_modifiers`, `affect_state`, `affect_impulses`, `motivations`, `user_beliefs`, `relationships`, `relationship_events`, `goals`, `goal_transitions`, `commitments`, `commitment_transitions`, `resource_accounts`, `resource_ledger`, `budget_policies` |
| `0005_memory` | `memories`, `memory_cues`, `memory_entities`, `memory_links`, `memory_access`, `memory_consolidations`, `memory_summaries`, `memory_communities`, `entity_edges`, `mistakes` (+ the FTS5 shadow tables) |
| `0006_epistemic` | `claims`, `evidence`, `observations`, `assumptions`, `predictions`, `contradictions`, `dependencies`, `epistemic_transitions` |
| `0007_library` | `library_entries`, `skills`, `workflows`, `cases`, `case_map`, `insights`, `policies`, `policy_versions`, `policy_fitness`, `policy_population`, `frame_instances`, `applicability_runs` |
| `0008_metacog` | `episodes`, `programs`, `program_traces` |
| `0009_decision_firewall` | `decisions`, `comparisons`, `firewall_runs`, `calibration`, `conformal_thresholds`, `factuality_checks` |
| `0010_tools` | `tool_registry`, `policy_sets`, `permission_grants`, `tool_calls`, `idempotency_keys`, `action_ledger`, `observed_payloads`, `rollback_snapshots` |
| `0011_self_engineering` | `prediction_ledger`, `prediction_outcomes`, `calibration_runs`, `meta_analyses`, `change_sets`, `promotions`, `regression_tests`, `evolution_journal`, `budgets`, `pi_sessions`, `pi_events` |
| `0012_loop` | `loop_runs`, `loop_iterations`, `self_model_reports`, `divergence_metrics`, `debt_findings`, `gc_actions`, `module_loads`, `design_revisions`, `timescales` |

(`_sqlx_migrations` and the FTS5 shadow tables account for the remainder.)

### 4.2 Named graphs and shapes

Eleven graphs are declared in `crates/mm-core/src/iri.rs:59`: `being`, `memory`, `epistemic`, `library`,
`world`, `provenance`, `code`, `decision`, `tools`, `selfeng`, `self`. A boot populates `/code` and the
ontology; `graph validate --graph <name>` runs the matching shape file (`ontology/shapes/*.ttl`, 11 files).
The `/world` graph is the strict one: only OBSERVED or VERIFIED claims may appear there.

### 4.3 Logging contract

197 codes, `console` + `jsonl` sinks, `trace_id` correlation, redaction before sink, and an immutable
audit chain. `mm-cli logs verify` is a gate step in **every** phase and checks ten things at once
(observed on a small run): audit chain, audit completeness, event sequence, record schema, redaction,
replay determinism, llm accounting, being correlation, memory correlation, epistemic correlation.

---

## 5. Capability matrix (what works end to end today)

| Capability | Status | Evidence |
|---|---|---|
| Boot a kernel, validate its invariants, verify its logs | ✅ works | `doctor`, `logs verify`, `replay --from 0` |
| Code-metadata scan/verify/lock | ✅ works | `codex verify` → `39 modules conform` |
| Being state, goals, invariants, adversarial denials | ✅ works | `being verify` → 22/22 denied |
| Memory add/recall/consolidate/forget/eval | ✅ works | `memory eval` meets thresholds; e2e `cli_memory` |
| Epistemic promotion guard, contradictions, world separation | ✅ works | e2e `epistemic_gate` |
| Library import/validate/applicability/skills/cases | ✅ works | 21 entries imported, 0 violations |
| Metacognitive episode compile/run/replay | ✅ works | e2e `episode_roundtrip`, byte-identical replay |
| Bounded decisions (rules core) + risk + calibration + conformance | ✅ works | `conformance: rules 6 passed, 0 failed` |
| Sanity firewall (deterministic half, corpus-graded) | ✅ works | `firewall eval`, gold corpus |
| Tools: 9 tools, default-deny, idempotency, ledger, rollback, MCP | ✅ works | `tool list/run/describe`, `action ledger --verify` |
| Verification obligations (build/test/static/symbolic/residual) | ✅ works | `verify run cargo-build --subject mm-core` → proven |
| Mistake→regression compile and run | ✅ works | `regression run` → fails-before true, passes-after true |
| Change sets, sandbox, promotion gate, audit | ✅ works | `promote` + `audit production-tree` → 1 promotion, 0 outside |
| Ten-stage closed loop, replay, self-model, debt, GC, hot-load | ✅ works | §3.7, and the e2e gate |
| Pi authoring (recorded session) | ✅ works | `pi run --offline` → 8 events, 3 edits |
| Pi authoring (live provider) | ⚠️ needs credentials + binary | `--provider` required; not run here |
| Decision core `local` | ❌ no producer for a head | `conformance` reports `local not run (unavailable)` |
| Decision core `hosted` | ❌ needs provider config | `conformance` reports `hosted not run (unavailable)` |
| Search-augmented factuality + persisted factuality rows | ❌ unimplemented | `factuality.rs:408`; no writer of `factuality_checks` |
| Sandbox tiers 2/3 | ❌ no runtime | `TierUnavailable`; features off |
| `pi_editor.open` | ❌ stub | refuses on tier + irreversibility |
| Tool-layer module provenance | ⚠️ dangling IRIs | nine `module/tools/*` IRIs with no module record |

---

## 6. Quality and completeness

**Strengths.**

- **Determinism is asserted, not hoped for.** Event-log replay hashes, `episode replay`, `loop replay`, and
  `logs verify`'s replay check all compare digests; `tests/e2e/replay_determinism.rs` proves the same hash
  across separate processes.
- **Refusals are first-class.** Every un-runnable path returns `Unavailable` with a reason and is *tested
  as a refusal* (`conformance` counts an unavailable core as "not run", not as a pass). This is the single
  best property of the codebase.
- **Negative fixtures.** `bench/codex/fixtures/` (one planted defect each), `bench/llm/malformed/`,
  `bench/being/invariants.jsonl`, `bench/tools/adversarial/`, and `gate_bypass_adversarial.rs` make the
  rules falsifiable.
- **Adversarial promotion coverage.** Five separate attempts to buy a promotion are refused by the
  deterministic gate.
- **Doc discipline.** Nearly every crate carries a module-level essay explaining the design and its
  deviations (`mm-tools/src/lib.rs` "Two deviations from the phase plan, and why" is the model).
- **Zero warnings, zero `unsafe`.** `#![forbid(unsafe_code)]` workspace-wide; `clippy -D warnings` and
  `cargo fmt --check` are CI steps.

**Weaknesses and drift.**

- **Three real gaps and three environment-gated ones.** See [`open-gaps.md`](./open-gaps.md): the local
  decision head (G1), factuality wiring (G2), tool modules (G3), the Pi editor stub (G4), sandbox tiers
  (G5), live models (G6).
- **Documentation drift in `mm-core`.** `README.md` says `NAMED_GRAPHS` lists "the seven runtime graphs"
  (there are eleven) and advertises `MM_DATA_DIR` overriding discovery, which no caller applies.
- **ULID case inconsistency.** `mm-tools`' JSON emits uppercase ULIDs (see §3.5 limit 4) — the rest of the
  CLI is lowercase, and `mm_core::serde_ulid` exists precisely to make this uniform.
- **One misclassified error, and an inconsistency around it.** Registering a skill that already exists
  (the seed at `ontology/seed/library_seed.ttl:198` ships `library/skill/api-capability-check@1`, so
  re-registering `bench/library/skills/manifest.json` after importing the seed does exactly this) reports
  `mm-cli: internal error: … (duplicate) (mm.internal)` with exit 2. A duplicate is a user-level refusal,
  not an internal error. The same situation through `library import` is handled well — the document's
  duplicates are *rejected* entry by entry and the run reports `0 accepted, 21 rejected` with exit 0 — so
  the two paths disagree about what a duplicate means and how it should be reported.
- **Breadth over depth in a few places.** Several crates ship a deterministic half of a
  model-facing feature (metacog's scan, factuality, the hosted core) and defer the model half; that is
  honest but means the system's "intelligent" behaviour is largely unexercised here.
- **Test volume, not test time.** 152 binaries take ~78 s of CPU in aggregate (longest single binary ~31 s),
  which is healthy; but there are no soak/long-run tests and no concurrency/scale tests.

**Measured quality gates (all green on the current tree):**

| Check | Result |
|---|---|
| `cargo build --workspace` | 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 |
| `cargo fmt --all -- --check` | 0 |
| `cargo test --workspace` | **1747 passed, 0 failed, 0 ignored** (152 binaries) |
| `mm-cli codex verify` | `39 modules conform` (0) |
| `mm-cli codex lock --check` | current, delta 0 |
| `mm-cli doctor` | kernel ready, graph invariants ok, audit chain intact |
| `mm-cli logs verify` | 10/10 checks pass |

---

## 7. What is tested, and what is not

### 7.1 Tested

| Layer | What it is | Scale |
|---|---|---|
| Unit tests | In-file, per crate and per module | 1776 attributes; 1747 executed |
| Integration tests | `crates/*/tests/`, `modules/*/*/tests/` | 59 files |
| Snapshot tests | `insta` golden records (every log code) and a golden `/code` graph | `mm-log`, `mm-codex` |
| Property tests | `proptest` on hashing, IRI parsing, and numeric paths | several crates |
| End-to-end gates | `tests/e2e/` — one or more per phase | 15 targets, 24 tests |
| Adversarial suites | Promotion bypass, sandbox escape, memory adversarial, malformed LLM output, invariant corpus | 5 corpora |
| Determinism tests | Event-log replay, episode replay, loop replay, Pi session replay | 4 e2e files |
| CLI gates | `mm-cli` commands run as subprocesses and their exit codes asserted | all gates |
| CI | build → clippy → fmt → test → phase-1 gate commands → Phase 12 gate → supervised loop | `.github/workflows/ci.yml` |

Phase-level mapping: 1 `kernel_boot`/`replay_determinism`/`cli_doctor`; 4 `cli_being`; 5 `cli_memory`;
6 `epistemic_gate`; 7 `library_gate`; 8 `episode_roundtrip`; 9 `phase09_firewall`; 10 `phase10_tools`;
11 `selfeng_cycle` + `pi_replay`; 12 `phase12_autonomy` + `hot_load` + `gate_bypass_adversarial`. Phases 2
and 3 have no e2e file: they are covered by their integration suites and the CLI-driven `codex`/`llm`
commands.

### 7.2 Not tested (and why)

| Untested | Why it is untested | How to close it |
|---|---|---|
| Live LLM calls (hosted core, metacog scan, Pi live authoring) | No `OPENAI_*` credentials in this environment; no `#[ignore]`d live tests exist | Set the env vars (see the [user guide](./user-guide.md#4-configuration)) and run `decide --core hosted`, `conformance --core hosted`, `pi run` without `--offline`; record the redaction/accounting results |
| Local decision core | Nothing fits a head (G1) | Implement the fitter, then run the conformance suite with `--head` |
| Search-augmented factuality | No retriever seam; no caller (G2) | Implement + wire, then assert a `factuality_checks` row |
| Sandbox tiers 2/3 | No gVisor/Firecracker runtime installed (G5) | Install a runtime, enable the feature, run a tier-2 tool end to end |
| `pi_editor.open` | Stub; needs tier 3 (G4/G5) | Implement against `mm-pi` once tier 3 exists |
| Cross-platform behaviour | Everything is developed and tested on Linux with RocksDB | Build/test on macOS/Windows; the RocksDB dependency is the first risk |
| Long-running/soak behaviour | The loop is tested for single runs; the five clocks need a long-lived process | Run `bin/mm-loop` for hours with `--ticks` high and watch `logs verify`/GC/budget drift |
| Concurrency at scale | Single-writer stores are correct but untested under load | A load test against the SQLite pool and the graph actor |
| Data migration/upgrade from an older `data/` dir | Only clean-boot and per-migration up/down are covered | Replay an older dump through the 12 migrations |
| Token/cost realism | Accounting is exercised with mocks only | A live run with the provider's usage reporting |

---

## 8. Future plans

The twelve-phase plan is **complete**: every phase's deliverables exist, its gate passes, and the tree is
green. What remains is not "phase 13" as designed work but a short, explicit backlog plus the deliberate
degradations. In priority order:

1. **Close G1 — the local decision head.** The traces exist (`decisions`), the consumer exists
   (`LocalCore`), the CLI flag exists. What is missing is a fitter and a command. This is the smallest
   change that turns a dead path into a working one, and it makes the `local` core reachable in
   `conformance`.
2. **Close G2 — factuality.** Two halves: implement `SearchAugmented` behind a retriever seam, and give
   `SelfConsistency`/`AtomicPrecision` an operator path that persists `factuality_checks` rows and feeds
   `factuality_support` into a firewall run. Until then a whole signal in the firewall is inert.
3. **Close G3 — tool-module provenance.** Either create the module directories (option A) or correct
   `PHASE_INDEX.md`/the Phase 10 plans and add a `codex verify` rule that a tool's `module_uri` must
   resolve (option B). Option B also prevents recurrence.
4. **Decide G4/G5 — the Pi editor stub and the tier-2/3 runtimes.** Either implement, or record in the
   plans that authoring ships through `mm-pi` and that higher tiers require an operator-installed runtime.
   These are coupled: the tool cannot run before tier 3 exists.
5. **G6 — an operator verification pass** against a live provider, to convert "unproven" into "observed".
6. **Housekeeping.** Fix the `MM_DATA_DIR` dead code (call `with_env_overrides`, or remove the claim),
   correct the `NAMED_GRAPHS` sentence in `mm-core/README.md`, and add
   `#[serde(with = "mm_core::serde_ulid")]` to the three `mm-tools` ULID fields so JSON is lowercase
   everywhere.

Beyond the backlog, the natural growth directions are the ones the design corpus already sketched: more
cognitive capabilities authored by the loop itself (each arriving through the promotion gate), a trained
local head replacing hosted calls for high-volume classes, deeper sandbox isolation, and a Pi client
tracking newer Pi protocol revisions. Every one of those arrives through the same door: change set →
sandbox → benchmark → gate → promotion → hot-load.

---

## How this was verified

```bash
cd ~/Build/metamind
git rev-parse --short HEAD            # 0204d73
cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --workspace                # 1747 passed, 0 failed, 0 ignored, 152 binaries

# component inventories
ls crates/*/Cargo.toml | wc -l                                    # 21
find modules -name plugin.toml | wc -l                            # 10
ls crates/mm-store-sqlite/migrations/*.sql | wc -l                # 12
rg -c '^\[\[module\]\]' codex.lock                                # 39
rg -o '"[a-z_]+\.[a-z_.]+"' crates/mm-log/src/codes.rs -N --no-filename | wc -l   # 197
rg -n 'NAMED_GRAPHS' -A12 crates/mm-core/src/iri.rs | head        # 11 graphs

# a fresh kernel, and its schema
D=$(mktemp -d); cargo run -q -p mm-cli -- --data-dir "$D" doctor
python3 -c "import sqlite3;print(len(sqlite3.connect('$D/metamind.db').execute(\"select name from sqlite_master where type='table'\").fetchall()))"   # 110

# gates
cargo run -q -p mm-cli -- codex verify        # 39 modules conform
cargo run -q -p mm-cli -- codex lock --check  # current, delta 0
```

Every output quoted in this document (the `conformance` refusals, the promotion JSON, the loop's ten
stages, `debt scan`'s six findings, `gc`'s refusal, the uppercase ULIDs in `action ledger --json`) came
from runs made while writing it; the transcripts are reproducible with the commands in the
[user guide](./user-guide.md).
