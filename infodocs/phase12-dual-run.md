# Phase 12 dual-run report — cost and latency

**Status: not executed against a live provider in this environment.**

This report is deliberately empty of numbers the environment could not measure.
The repository has no `OPENAI_BASE_URL`/`OPENAI_API_KEY` configured, and the
project's own baseline documents that no live-provider call has ever been made
here ([`components-and-status.md`](./components-and-status.md), `mm-llm`). A
scripted controller can exercise both code paths, but its latency measures local
orchestration, not provider cost or model latency, so it is not reported as a
dual-run result.

## What was compared instead

Two non-live checks ran in Phase 12:

1. **Contract parity.** The conversation-manager surface is exactly the seventeen
   reviewed schemas (`crates/mm-serve/schemas/tools.json`), and the legacy
   console controller remains reachable behind the flag-off route. The legacy
   route's tests (`crates/mm-serve/tests/parity.rs`) and the CME route's tests
   (`phase7.rs`, `phase2.rs`, `phase4.rs`, `phase5.rs`, `phase6.rs`,
   `phase7_pi.rs`) both pass.
2. **Flag fallback.** `FeatureFlags::default()` now enables `MM_CM_V3`, and
   `MM_CM_V3=false` removes the CM tool attachment without changing the legacy
   `/api/chat` route. `MM_CME_ENABLED=false` keeps the legacy route in force.

The default was therefore chosen for **behavioral parity and rollback safety**,
not for a measured cost win. That distinction matters: if the first real
dual-run shows a regression, the rollback is a flag flip.

## How to run the real dual-run

Use two fresh data directories so neither run sees the other's memory, sessions,
or cache.

```bash
# Same prompts, same model specs, two data roots.
DATA_OLD=$(mktemp -d)
DATA_NEW=$(mktemp -d)

# Legacy route.
MM_CM_V3=false MM_CME_ENABLED=false \
  cargo run -q -p mm-serve -- --data-dir "$DATA_OLD" --port 8899

# New CM contract route.
MM_CM_V3=true MM_CME_ENABLED=true \
  cargo run -q -p mm-serve -- --data-dir "$DATA_NEW" --port 8900
```

Drive the same scripted conversation through both (the repository's Playwright
suite is the natural driver). Then read the accounting table:

```sql
-- per run
SELECT purpose,
       COUNT(*)                         AS calls,
       SUM(tokens_in)                   AS tokens_in,
       SUM(tokens_out)                  AS tokens_out,
       SUM(cost_micros)                 AS cost_micros,
       AVG(latency_ms)                  AS avg_latency_ms,
       MAX(latency_ms)                  AS max_latency_ms,
       SUM(CASE WHEN cached = 1 THEN 1 ELSE 0 END) AS cached_calls,
       SUM(CASE WHEN status <> 'ok' THEN 1 ELSE 0 END) AS failed_calls
FROM llm_calls
GROUP BY purpose
ORDER BY purpose;
```

`llm_calls` is migration `0003_llm.sql`; it records `cost_micros`, `latency_ms`,
token counts, cache hits, routing changes, and failures. The CLI summary is:

```bash
mm-cli --data-dir "$DATA_NEW" llm stats --json
```

## Acceptance thresholds for the first real run

These are the thresholds the plan intended; they are not claims about the current
build.

| Measure | Threshold |
|---|---|
| New-path cost per completed turn | ≤ legacy path, or an explained one-time increase |
| New-path p50 latency | ≤ legacy p50 + 250 ms |
| New-path p95 latency | ≤ legacy p95 + 750 ms |
| Failed/schema-rejected calls | no increase over legacy |
| Tool rounds per turn | bounded by the turn loop policy, no runaway |
| Cache hit rate | not lower than legacy on the same prompt set |

Record the raw `llm_calls` rows with the report; do not average away a long tail.
If a threshold fails, set `MM_CM_V3=false` (and `MM_CME_ENABLED=false` if the whole
CME route is implicated), keep the data root for diagnosis, and file the failing
prompt/trace ids with the report.

## What this report does not claim

- It does not claim the new path is cheaper or faster.
- It does not claim a live provider has ever been exercised.
- It does not treat scripted/mock latency as provider latency.

The Phase 12 default is a reversible product decision backed by passing parity
tests and an explicit rollback flag, not by fabricated benchmark numbers.
