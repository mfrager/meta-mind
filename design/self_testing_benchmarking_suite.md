# MetaMind Self-Testing, Benchmarking & Bootstrapping Suite

**Status:** Design Proposal  
**Location:** `design/self_testing_benchmarking_suite.md`  
**Related:** `bootstrap_process1.md`, `bootstrap_process2.md`, `PHASE_INDEX.md`, `tests/e2e/`

---

## 1. Purpose & Scope

This design specifies an **internal benchmarking, self-testing, and bootstrapping suite** that:

- **Tests every crate/module** in the MetaMind workspace in increasing complexity tiers
- **Uses live LLM minimally** — only where semantic cognition is genuinely required to validate integration
- **Generates detailed, structured reports** per test and per suite run
- **Serves as the regression backbone** for the self-engineering loop (bootstrap_process2.md §3)
- **Excludes audio/voice** — no TTS, ASR, Realtime API, or prosody tests
- **Enables bootstrapping** — the suite itself is a capability the being can run, evaluate, and improve

The suite lives under `bench/self_test/` and `tests/self_test/` and is invoked via `mm-cli self-test` and `mm-cli bench self`.

---

## 2. Architectural Principles

| Principle | Rationale |
|-----------|-----------|
| **Deterministic first, semantic second** | Unit/integration tests use no LLM; only "semantic integration" tiers call the LLM |
| **Minimal LLM calls per test** | One call max per semantic test; cached via `mm-llm` disk cache |
| **Self-describing tests** | Each test declares: crate, tier, LLM usage, expected duration, fixtures, success criteria |
| **Report as first-class artifact** | Reports are RDF + JSON, queryable, versioned, fed to meta-analysis |
| **Bootstrappable** | The suite can test itself; new tests generated from mistakes (bootstrap_process2.md §9) |
| **Tiered complexity** | 5 tiers from zero-LLM unit → full cognitive episode |

---

## 3. Module Map (Crates Under Test)

| Crate | Domain | Primary Test Tiers |
|-------|--------|-------------------|
| `mm-core` | Kernel, config, ULID, events | 1, 2 |
| `mm-eventlog` | Event sourcing, audit chain | 1, 2, 3 |
| `mm-store-sqlite` | Tabular store, migrations | 1, 2 |
| `mm-store-graph` / `mm-data` | RDF/SPARQL, cubecast | 1, 2, 3 |
| `mm-log` | Structured logging | 1 |
| `mm-llm` | LLM client, cache, routing, structured output | 1, 2, **4** |
| `mm-epistemic` | Claims, evidence, assumption ledger, contradictions | 1, 2, 3, **4** |
| `mm-memory` | 10 memory classes, consolidation, retrieval | 1, 2, 3, **4** |
| `mm-being` | Identity, personality, goals, relationships | 1, 2, 3, **4** |
| `mm-metacog` | Cognitive program compiler, OperationValue, budgets | 1, 2, 3, **4** |
| `mm-decision` / `mm-firewall` | Jev decisions, sanity firewall, comparison integrity | 1, 2, 3, **4** |
| `mm-library` | Cognitive library, frames, skills, policy genome | 1, 2, 3, **4** |
| `mm-tools` | Tool registry, permissions, executor, verification | 1, 2, 3 |
| `mm-codex` | Code metadata, registry, capability packages | 1, 2, 3 |
| `mm-mistakes` | Mistake records, regression test compiler | 1, 2, 3 |
| `mm-metaanalysis` | Meta-analysis budgets, prediction calibration | 1, 2, 3, **4** |
| `mm-selfeng` | Self-engineering loop, sandbox, promotion | 1, 2, 3, **4** |
| `mm-pi` | Pi RPC, qualification, module scaffolding | 1, 2, 3 |
| `mm-runtime` | Closed loop, self-model divergence | 1, 2, 3, **4** |
| `mm-sm` | Hierarchical state machines | 1, 2, 3 |
| `mm-cme` | Conversation engine (text path only) | 1, 2, 3, **4** |
| `mm-serve` | HTTP/SSE, API contracts | 1, 2 |
| `mm-cli` | Command dispatch, doctor, REPL | 1, 2, 3 |

**Tier 4** = semantic integration tests (use live LLM). All others are zero-LLM.

---

## 4. Five Test Tiers (Increasing Complexity)

```
┌─────────────────────────────────────────────────────────────────┐
│ TIER 0: INFRASTRUCTURE  (cargo test --workspace --lib)          │
│   • Rust unit tests, doc tests, property tests (proptest)       │
│   • No binary, no stores, no LLM                                │
│   • Runs in < 30s                                               │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ TIER 1: CRATE ISOLATION  (mm-cli self-test --tier=1)            │
│   • Each crate tested in throwaway data dir                     │
│   • Real stores (SQLite + Oxigraph), real config                │
│   • Deterministic fixtures from bench/*/gold/                   │
│   • No LLM calls — mocks/stubs for mm-llm trait                 │
│   • ~2-5 min total                                              │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ TIER 2: CROSS-CRATE INTEGRATION  (--tier=2)                     │
│   • Multi-crate scenarios: eventlog → memory → epistemic        │
│   • Kernel boot → ontology load → being init → goal create      │
│   • CLI-driven via mm-cli (same as tests/e2e/)                  │
│   • Fixtures from bench/*/standard/, bench/*/gold/              │
│   • Zero LLM — mm-llm mocked                                    │
│   • ~5-10 min                                                   │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ TIER 3: ADVERSARIAL & PROPERTY  (--tier=3)                      │
│   • Adversarial fixtures from bench/*/adversarial/              │
│   • Property-based: random event sequences, concurrent access   │
│   • Chaos: kill -9 mid-transaction, corrupt WAL, bad RDF        │
│   • Fuzz: schema validation, ULID parsing, SPARQL injection     │
│   • Zero LLM                                                    │
│   • ~10-20 min                                                  │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ TIER 4: SEMANTIC INTEGRATION  (--tier=4)                        │
│   • **Only tier using live LLM** — 1-2 calls per test max       │
│   • Tests: LLM→structured output→firewall→memory→epistemic     │
│   • Cognitive episode: observe → plan → act → verify → learn    │
│   • Fixtures: bench/episodes/gold/, bench/calibration/labeled.jsonl│
│   • Cached via mm-llm disk cache (replay mode)                  │
│   • ~5-15 min (depends on LLM latency)                          │
└─────────────────────────────────────────────────────────────────┘
```

---

## 5. Test Specification Format

Each test is a TOML file under `bench/self_test/specs/`:

```toml
# bench/self_test/specs/mm_memory_episodic_recall.tier1.toml
[test]
id = "mm_memory_episodic_recall"
crate = "mm-memory"
tier = 1
name = "Episodic memory round-trip: store → recall → verify"
description = "Proves the episodic store writes, indexes, and retrieves an episode with full provenance."

[fixtures]
input = "bench/memory/episodes.jsonl"      # 50 golden episodes
expected = "bench/memory/gold.jsonl"        # expected recall results

[execution]
timeout_seconds = 30
parallel_safe = true
llm_calls = 0
mock_llm = true

[success_criteria]
# All must pass
- "recall_count == input_count"
- "provenance_chain_intact == true"
- "temporal_order_preserved == true"
- "confidence_scores_in_range == true"

[report]
include = ["latency_p50", "latency_p99", "store_size_bytes", "index_size_bytes"]
format = "json+rdf"
```

```toml
# bench/self_test/specs/mm_epistemic_assumption_ledger.tier4.toml
[test]
id = "mm_epistemic_assumption_ledger"
crate = "mm-epistemic"
tier = 4
name = "Assumption ledger: assumption → dependency → cascade on falsification"
description = "Validates the full epistemic chain with live LLM generating claims."

[fixtures]
input = "bench/epistemic/cascade/graph.json"
llm_prompt = "bench/llm/schemas.json#assumption_extraction_v1"

[execution]
timeout_seconds = 120
parallel_safe = false
llm_calls = 1
mock_llm = false
llm_cache_mode = "replay"  # or "fresh" for calibration runs

[success_criteria]
- "assumptions_extracted >= 3"
- "dependency_dag_acyclic == true"
- "cascade_on_falsification == true"
- "epistemic_status_never_upgrades_silently == true"
- "verification_priority_computed == true"

[report]
include = ["llm_tokens_used", "llm_latency_ms", "assumptions_count", "cascade_depth", "verification_priorities"]
format = "json+rdf"
```

---

## 6. Report Schema

Every test produces a report written to `bench/self_test/reports/<test_id>_<timestamp>.<ulid>.json`:

```json
{
  "test_id": "mm_memory_episodic_recall",
  "crate": "mm-memory",
  "tier": 1,
  "run_id": "01JXK2M...",           // ULID
  "timestamp": "2026-11-21T14:32:10Z",
  "git_commit": "7f92a1...",
  "self_version": {                 // bootstrap_process2.md §4
    "code": "7f92a1",
    "ontology": "31",
    "memory": "82",
    "policies": "14",
    "prompt": "9",
    "model": "qwen-x"
  },
  "environment": {
    "rustc": "1.85.0",
    "target": "x86_64-unknown-linux-gnu",
    "llm_provider": "mock",         // or "openai:gpt-4o-mini"
    "llm_cache_mode": "replay"
  },
  "fixtures": {
    "input_hash": "sha256:...",
    "expected_hash": "sha256:..."
  },
  "execution": {
    "duration_ms": 1247,
    "llm_calls": 0,
    "llm_tokens": 0,
    "llm_cost_usd": 0.0,
    "peak_memory_mb": 42,
    "cpu_time_ms": 890
  },
  "results": {
    "passed": true,
    "assertions": [
      { "name": "recall_count", "expected": 50, "actual": 50, "passed": true },
      { "name": "provenance_chain_intact", "expected": true, "actual": true, "passed": true }
    ],
    "metrics": {
      "latency_p50_ms": 3.2,
      "latency_p99_ms": 18.7,
      "store_size_bytes": 1048576,
      "index_size_bytes": 262144
    }
  },
  "regressions": [],                // vs baseline if --compare-baseline
  "new_failures": [],
  "artifacts": {
    "store_dump": "bench/self_test/artifacts/01JXK2M.../sqlite.dump",
    "graph_dump": "bench/self_test/artifacts/01JXK2M.../oxigraph.nq",
    "trace": "bench/self_test/artifacts/01JXK2M.../trace.jsonl"
  }
}
```

**RDF Mirror** — Same data as named graph `mm:selfTestReport/<run_id>` in cubecast for SPARQL queries.

---

## 7. Suite Runner: `mm-cli self-test`

```bash
# Run all tiers sequentially (default)
mm-cli self-test

# Run specific tier
mm-cli self-test --tier 2

# Run specific crate(s)
mm-cli self-test --crate mm-memory,mm-epistemic --tier 1

# Run single test
mm-cli self-test --test mm_memory_episodic_recall

# Compare against baseline (bench/self_test/baselines/)
mm-cli self-test --compare-baseline v184

# Generate aggregate report
mm-cli self-test --report aggregate --output bench/self_test/reports/aggregate_<timestamp>.json

# Calibration run (fresh LLM calls, update bench/calibration/reference.json)
mm-cli self-test --tier 4 --calibrate

# Bootstrapping mode: being runs self-test on itself, feeds results to meta-analysis
mm-cli self-test --bootstrap --meta-analyze
```

**Exit codes:** 0 = all passed, 1 = failures, 2 = infrastructure error, 3 = baseline regression.

---

## 8. Baseline Management

```
bench/self_test/
├── specs/                    # Test specifications (TOML)
├── baselines/                # Golden reports per self-version
│   ├── v184/
│   │   ├── mm_memory_episodic_recall.json
│   │   ├── mm_epistemic_assumption_ledger.json
│   │   └── aggregate.json
│   └── v185/
├── reports/                  # Timestamped run reports
├── artifacts/                # Store dumps, traces per run
└── calibration/
    ├── reference.json        # Tier 4 LLM reference outputs
    └── conformal.json        # Conformal prediction intervals
```

**Baseline promotion** (bootstrap_process2.md §3):
1. Run suite on candidate self-version
2. Compare aggregate metrics vs current baseline
3. If **global fitness** improves (bootstrap_process2.md §14): promote
4. New baseline = candidate reports; old baseline archived

---

## 9. Per-Crate Test Plans (Representative)

### 9.1 mm-core (Tiers 1-2)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_core_ulid_monotonic` | 1 | ULIDs sort correctly under clock drift |
| `mm_core_config_load` | 1 | TOML config loads, validates, merges env |
| `mm_core_event_roundtrip` | 1 | Event serialize → deserialize → commit |
| `mm_core_audit_chain` | 2 | Append N events → audit chain verifies |
| `mm_core_doctor_idempotent` | 2 | `mm-cli doctor` twice → no duplicate ontology |

### 9.2 mm-eventlog (Tiers 1-3)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_eventlog_append_commit` | 1 | Single event append + commit |
| `mm_eventlog_concurrent_append` | 3 | 10 writers, 100 events each, no lost commits |
| `mm_eventlog_replay_determinism` | 2 | Replay from genesis → identical state |
| `mm_eventlog_abort_preserves_audit` | 2 | Aborted events have audit records |
| `mm_eventlog_wal_corruption_recovery` | 3 | Corrupt WAL → recovery replays cleanly |

### 9.3 mm-store-graph / mm-data (Tiers 1-3)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_data_ontology_load` | 1 | T-Box loads, >50 triples, invariants OK |
| `mm_data_sparql_basic` | 1 | SELECT/ASK/CONSTRUCT/DESCRIBE work |
| `mm_data_shacl_validation` | 2 | Shapes validate, violations reported |
| `mm_data_named_graph_isolation` | 2 | Graphs don't leak triples |
| `mm_data_cubecast_migration` | 3 | Oxigraph → Cubecast cutover preserves data |
| `mm_data_concurrent_sparql` | 3 | 20 parallel queries, no deadlocks |

### 9.4 mm-llm (Tiers 1-2, 4)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_llm_cache_hit_miss` | 1 | Disk cache: miss → fetch → hit → replay |
| `mm_llm_structured_output_schema` | 1 | JSON schema validation, retry on failure |
| `mm_llm_routing_table` | 2 | Task → model selection per routing_table.toml |
| `mm_llm_cost_accounting` | 2 | Token counts, USD costs tracked per call |
| `mm_llm_semantic_integration` | 4 | **Live LLM**: prompt → structured output → validate |

### 9.5 mm-epistemic (Tiers 1-4)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_epistemic_claim_create` | 1 | Claim with evidence, status, confidence |
| `mm_epistemic_assumption_ledger` | 2 | Assumption recorded, never auto-promotes |
| `mm_epistemic_contradiction_detection` | 2 | New fact contradicts assumption → conflict |
| `mm_epistemic_dependency_cascade` | 3 | A→B→C, falsify A → B,C confidence drops |
| `mm_epistemic_verification_priority` | 3 | Priority = P(false)×Consequence×Dep/Cost |
| `mm_epistemic_llm_assumption_extraction` | 4 | **Live LLM**: text → assumptions → ledger |

### 9.6 mm-memory (Tiers 1-4)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_memory_episodic_roundtrip` | 1 | Store 50 episodes → recall all |
| `mm_memory_semantic_consolidation` | 2 | 100 episodes → pattern → semantic memory |
| `mm_memory_retrieval_hybrid` | 2 | Keyword + embedding → ranked results |
| `mm_memory_forgetting_utility` | 3 | Low-utility memories pruned, protected kept |
| `mm_memory_adversarial_contradiction` | 3 | Contradictory facts → flagged, not merged |
| `mm_memory_llm_compression` | 4 | **Live LLM**: episodes → compressed segment |

### 9.7 mm-being (Tiers 1-4)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_being_identity_invariants` | 1 | Identity never fabricates, never rewrites history |
| `mm_being_personality_contextual` | 2 | Context vector → behavioral mode weights |
| `mm_being_goal_lifecycle` | 2 | Create → progress → complete → archive |
| `mm_being_relationship_dimensions` | 3 | Multi-dim relationship evolves over episodes |
| `mm_being_self_model_divergence` | 4 | **Live LLM**: actual vs model vs ideal → gaps |

### 9.8 mm-metacog (Tiers 1-4)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_metacog_operation_value` | 1 | OperationValue formula computes correctly |
| `mm_metacog_cognitive_budget` | 2 | Budget allocates, escalates, stops correctly |
| `mm_metacog_program_compilation` | 3 | Scan → program → LLM executes → verifies |
| `mm_metacog_triage_depth` | 3 | Stakes/uncertainty → correct tier selection |
| `mm_metacog_llm_cognitive_scan` | 4 | **Live LLM**: situation → metacognitive scan |

### 9.9 mm-decision / mm-firewall (Tiers 1-4)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_firewall_gate_outcomes` | 1 | 7 outcomes map to correct actions |
| `mm_firewall_bad_idea_taxonomy` | 2 | Wrong/Bad/Dumb/Dangerous/Wasteful/Misaligned detected |
| `mm_decision_jev_bounded` | 2 | Jev decisions: probabilities, choices, scores |
| `mm_comparison_integrity` | 3 | Apples-to-apples: normalization, purpose-driven |
| `mm_firewall_llm_sanity_scan` | 4 | **Live LLM**: plan → firewall → gate decision |

### 9.10 mm-library (Tiers 1-4)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_library_frame_composition` | 1 | Frame A ⊕ B = composed frame |
| `mm_library_precedent_retrieval` | 2 | Problem → similar cases → transferability |
| `mm_library_policy_genome_version` | 3 | Policy vN+1 fitness > vN → promote |
| `mm_library_skill_execution` | 3 | Skill: invoke → verify → record outcome |
| `mm_library_llm_frame_construction` | 4 | **Live LLM**: observe → construct frame |

### 9.11 mm-tools (Tiers 1-3)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_tools_registry_permissions` | 1 | Tool requires grant → permit/forbid |
| `mm_tools_executor_reversibility` | 2 | Reversible tool → rollback works |
| `mm_tools_verification_cargo` | 3 | `cargo check` as verification tool |
| `mm_tools_adversarial` | 3 | Path traversal, oversize, scope bypass → rejected |

### 9.12 mm-codex (Tiers 1-3)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_codex_registry_verify` | 1 | `mmc codex verify` passes on clean repo |
| `mm_codex_capability_package` | 2 | Capability: identity, impl, tests, benchmarks |
| `mm_codex_golden_snapshots` | 3 | Golden files match canonical output |

### 9.13 mm-mistakes (Tiers 1-3)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_mistakes_regression_compiler` | 1 | Incident → minimal repro → test added |
| `mm_mistakes_root_cause` | 2 | Mistake → root cause → corrective rule |
| `mm_mistakes_recurrence_risk` | 3 | Near-miss → risk score → prevention |

### 9.14 mm-metaanalysis (Tiers 1-4)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_metaanalysis_prediction_ledger` | 1 | Prediction recorded → outcome → calibration |
| `mm_metaanalysis_calibration_conformal` | 2 | Conformal intervals cover at nominal rate |
| `mm_metaanalysis_budget_allocation` | 3 | Meta/Improvement/Evolution budgets enforced |
| `mm_metaanalysis_llm_self_review` | 4 | **Live LLM**: review decisions → lessons |

### 9.15 mm-selfeng (Tiers 1-4)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_selfeng_sandbox_isolation` | 1 | Candidate runs in sandbox, no prod access |
| `mm_selfeng_change_set` | 2 | Code+data+schema+policy versioned together |
| `mm_selfeng_promotion_pipeline` | 3 | Test → historical → adversarial → regress → compare |
| `mm_selfeng_llm_candidate_generation` | 4 | **Live LLM**: failure → hypothesis → candidate |

### 9.16 mm-pi (Tiers 1-3)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_pi_qualification_novel` | 2 | Novel goal → qualification → capability gap |
| `mm_pi_module_scaffold` | 3 | Scaffold → build → test → integrate |

### 9.17 mm-runtime (Tiers 1-4)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_runtime_closed_loop` | 3 | Episode → meta-analysis → improvement → verify |
| `mm_runtime_self_model_sync` | 4 | **Live LLM**: actual behavior → model update |

### 9.18 mm-sm (Tiers 1-3)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_sm_orthogonal_regions` | 1 | 6 regions transition independently |
| `mm_sm_history_pseudostate` | 2 | Deep history restores sub-state |
| `mm_sm_parent_child_spawn` | 2 | Turn SM spawns Chunk SMs |
| `mm_sm_rdf_serialization` | 3 | SM definition + runtime → RDF roundtrip |

### 9.19 mm-cme (Tiers 1-4, text path only)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_cme_turn_planning` | 1 | TurnPlan with chunk directives validates |
| `mm_cme_proactive_injection` | 2 | Proactive queue → inject at plan time |
| `mm_cme_speculative_execution` | 3 | ASR-LLM overlap (mocked) → latency gain |
| `mm_cme_llm_turn_execution` | 4 | **Live LLM**: user msg → turn → chunks → done |

### 9.20 mm-serve / mm-cli (Tiers 1-3)
| Test ID | Tier | Description |
|---------|------|-------------|
| `mm_serve_health_endpoint` | 1 | GET /health → 200, version, stores OK |
| `mm_serve_sse_stream` | 2 | SSE frames: text, card, control |
| `mm_cli_doctor` | 1 | Doctor boots kernel, reports versions |
| `mm_cli_repl_basic` | 2 | REPL: send → receive → history |

---

## 10. Bootstrapping Integration

The suite **is itself a capability** the being can invoke:

```rust
// mm-selfeng/src/bootstrap_runner.rs
pub struct SelfTestCapability {
    pub identity: "self_test_suite",
    pub purpose: "Validate own integrity, detect regressions, calibrate",
    pub implementation: "mm-cli self-test --bootstrap --meta-analyze",
    pub data_requirements: ["bench/self_test/specs/", "bench/self_test/baselines/"],
    pub activation_conditions: [
        "scheduled: daily 03:00",
        "on: code_change",
        "on: baseline_regression",
        "on: meta_analysis_trigger"
    ],
    pub tests: ["bench/self_test/specs/*.toml"],
    pub benchmarks: ["bench/self_test/benchmarks/"],
    pub known_failure_modes: [
        "LLM provider unavailable → tier 4 skipped, reported",
        "Disk full → artifact cleanup, retry",
        "Concurrent run → lock conflict, wait or skip"
    ],
    pub evidence: "bench/self_test/reports/aggregate_latest.json",
    pub version: 1,
    pub fitness: 0.0  // computed after each run
}
```

**Bootstrap loop** (bootstrap_process2.md §3):
1. Being runs `mm-cli self-test --bootstrap`
2. Results → meta-analysis → lessons → candidate policies
3. If regression: `mm-selfeng` creates ChangeSet, tests in sandbox
4. Promotion → new baseline → new self-version

---

## 11. Calibration & Conformance (Tier 4)

Tier 4 tests use **conformal prediction** (bench/calibration/) to bound LLM variability:

```toml
# bench/self_test/specs/mm_epistemic_llm_assumption_extraction.tier4.toml
[calibration]
reference_file = "bench/calibration/reference.json"
conformal_file = "bench/calibration/conformal.json"
coverage = 0.95
max_deviation = 0.15  # 15% metric drift triggers investigation
```

**Calibration run** (`--calibrate`):
- Runs Tier 4 with `llm_cache_mode = "fresh"`
- Updates `reference.json` (median over 10 runs)
- Updates `conformal.json` (quantile intervals)
- **Only maintainers run this** — not in CI

---

## 12. CI Integration

```yaml
# .github/workflows/self-test.yml
name: Self-Test Suite
on: [push, pull_request, schedule: "0 3 * * *"]

jobs:
  tier0:
    runs-on: ubuntu-latest
    steps: [checkout, cargo test --workspace --lib]

  tier1:
    needs: tier0
    runs-on: ubuntu-latest
    steps:
      - checkout
      - cargo build --workspace --release
      - run: mm-cli self-test --tier 1 --report junit

  tier2:
    needs: tier1
    runs-on: ubuntu-latest
    steps:
      - checkout
      - cargo build --workspace --release
      - run: mm-cli self-test --tier 2

  tier3:
    needs: tier2
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - checkout
      - cargo build --workspace --release
      - run: mm-cli self-test --tier 3

  tier4:
    # MANUAL ONLY - requires LLM credentials, costs money
    # Triggered via workflow_dispatch or on calibration tag
    runs-on: ubuntu-latest
    if: github.event_name == 'workflow_dispatch'
    env:
      OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
    steps:
      - checkout
      - cargo build --workspace --release
      - run: mm-cli self-test --tier 4 --llm-provider openai:gpt-4o-mini
```

---

## 13. Implementation Phases

| Phase | Deliverable | Depends On |
|-------|-------------|------------|
| **13.1** | `bench/self_test/specs/` schema + parser | mm-core, mm-data |
| **13.2** | Tier 0/1 runner in `mm-cli self-test` | mm-cli, mm-eventlog, mm-store-* |
| **13.3** | Report writer (JSON + RDF) + artifact collector | mm-data, mm-log |
| **13.4** | Tier 2 cross-crate fixtures + runner | All crates Tier 1 |
| **13.5** | Tier 3 adversarial/property harness | Tier 2, proptest, chaos tools |
| **13.6** | Tier 4 LLM integration (mock + live) | mm-llm, bench/calibration/ |
| **13.7** | Baseline management + comparison CLI | mm-selfeng, mm-metaanalysis |
| **13.8** | Bootstrap capability registration | mm-selfeng, mm-pi |
| **13.9** | CI workflow + calibration docs | All above |
| **13.10** | Full per-crate test specs (Section 9) | Domain experts per crate |

**Estimated effort:** 4-6 weeks for Phases 13.1-13.7 (core suite), 2-3 weeks for 13.8-13.10 (completion).

---

## 14. Success Criteria

| Metric | Target |
|--------|--------|
| Tier 0+1 runtime | < 3 min |
| Tier 2 runtime | < 10 min |
| Tier 3 runtime | < 20 min |
| Tier 4 runtime (cached) | < 5 min |
| Tier 4 LLM calls per run | ≤ 20 total |
| Baseline regression detection | 100% (no false negatives) |
| Self-test capability fitness | > 0.9 after 3 bootstrap cycles |
| Report queryability (SPARQL) | All fields queryable |
| Bootstrappable | Being can run, analyze, improve own suite |

---

## 15. Non-Goals (Explicit)

- ❌ Audio/voice testing (TTS, ASR, Realtime, prosody)
- ❌ Multi-agent coordination tests
- ❌ Load/stress testing beyond Tier 3 chaos
- ❌ UI/frontend testing (covered by frontend/e2e/)
- ❌ External API integration tests (except LLM provider)
- ❌ Security penetration testing (separate program)

---

## 16. Appendix: Test Spec Template

```toml
# bench/self_test/specs/<crate>_<feature>_tier<N>.toml
[test]
id = ""                    # unique, snake_case
crate = ""                 # mm-<crate>
tier = 0                   # 0-4
name = ""                  # human-readable
description = ""           # 1-2 sentences

[fixtures]
input = ""                 # path relative to repo root
expected = ""              # optional golden output
llm_prompt = ""            # for tier 4 only

[execution]
timeout_seconds = 60
parallel_safe = true
llm_calls = 0
mock_llm = true
llm_cache_mode = "replay"  # replay | fresh | bypass

[success_criteria]
# Array of expressions evaluated post-run
# Available vars: actual, expected, metrics, artifacts
- ""

[report]
include = []               # extra metrics to emit
format = "json+rdf"        # json | json+rdf | rdf
```

---

*This design enables the being to **trust its own substrate** — every crate verified, every integration proven, every regression caught, every improvement measured. The suite is not a test harness; it's the being's **immune system and mirror**.*