# Phase 0 Baseline — Conversation Engine + RDF Backend

**Recorded**: 2026-11-21
**Base commit**: `c0110de`
**Branch**: `integration/cme-rdf-backend` (created locally; not pushed)

## Verification

| Check | Result |
|-------|--------|
| `cargo test --workspace --no-fail-fast` | 1806 passed, 0 failed (158 test binaries), exit 0 |
| `cubecast`: `cargo build --workspace --features application` | Success (1m 07s) |
| `mm-serve` boot to `/api/health` `200` | 246 ms (debug build, empty data dir) |
| `mm-serve` idle RSS after boot | 47,464 kB (VmRSS = VmHWM) |

Boot and memory were sampled once with `target/debug/mm-serve --data-dir <temp> --port 18899`
and `/api/health` polled at 100 ms intervals. The release profile and a warm data
directory will differ; record those when Phase 7 adds the load gate.

## Plan targets (unchanged)

| Metric | Baseline measured | Target |
|--------|-------------------|--------|
| Workspace tests | pass | pass |
| `mm-serve` boot time | 246 ms (debug/fresh) | < 3 s |
| Chat turn latency (p99) | not measured in Phase 0 | < 2 s |
| Idle memory | 47 MB (debug/fresh) | < 300 MB |
| Concurrent sessions | not measured in Phase 0 | 200 |

Turn latency and concurrency need the synthetic/load harness from Phase 7; no
plausible Phase 0 substitute exists, so they stay unmeasured rather than guessed.

## Test gates repaired to reach a green baseline

Two committed gates were failing on this checkout for environmental reasons
rather than product regressions:

- `tests/e2e/cli_memory.rs` — the corpus has fixed `valid_from` instants, so the
  literal `--window 30d` stopped covering it as the fixture aged and
  consolidation became a no-op. The window is now derived from the fixture's
  oldest episode (plus slack), preserving the assertion that consolidation
  reduces the corpus.
- `tests/e2e/phase09_firewall.rs` — the gate asserts `hosted` is unavailable, but
  the repository `.env` configures a provider on this machine, so conformance
  built and ran it. The gate now blanks `OPENAI_BASE_URL` / `OPENAI_API_KEY` for
  that subprocess (`run_cli_ok_with_env`), so the assertion tests the kernel's
  unconfigured-provider behaviour instead of the developer's environment.

## Scaffolding

Created (tracked with `.gitkeep`; no crates or workspace members added yet):

```
crates/mm-sm/src/
crates/mm-data/src/
crates/mm-cme/src/{regions,driver,planner,executor,routing,observability,report,dashboard}/
```

Phase 1 turns `crates/mm-sm` into a real workspace member; Phases 2 and 3 do the
same for `mm-data` and `mm-cme`.
