# Phase 12 dual-run report — cost and latency

**Status: executed against a live provider on 2026-11-23. The legacy half is
complete; the CM v3 half is partial because the shared provider key hit its
quota mid-run.** Two code defects that made a live CM turn impossible were found
and fixed before the run (below), and the regression that had hidden them is
now covered by a test.

The endpoint was configured in this environment through `OPENAI_BASE_URL` /
`OPENAI_API_KEY` (a LiteLLM proxy on the LAN); neither the URL nor the key is
reproduced here. The routed model was `gemini/gemini-2.5-flash`. Earlier
versions of this report and of `components-and-status.md` said no provider had
ever been exercised here; that was true when written and is no longer.

## Defects found and fixed before the run

A live CM turn could not run at all until these were fixed. Both are on
`integration/cme-rdf-backend` in the working tree.

1. **The routed model was dropped.** `TurnExecutor::execute` chooses a model
   from its `ConversationRouter`, but `llm_request` built the request without
   `model`, so `OpenAiClient` failed every turn with
   `no model was requested and no default_model is configured`
   (`config/llm.toml` deliberately has no `[defaults].default_model`). The
   routed model is now pinned on the request
   (`crates/mm-cme/src/executor.rs`), with regression test
   `the_routed_model_is_pinned_on_the_request` in
   `crates/mm-cme/tests/phase4.rs`.
2. **The CM path wrote no `llm_calls` rows.** It called the raw
   `OpenAiClient`, so the SQL below would have shown the legacy path and
   nothing else, making the comparison impossible. `crates/mm-serve/src/state.rs`
   now wraps the CM client in `LlmService`, and `impl LlmClient for LlmService`
   (`crates/mm-llm/src/service.rs`) gives every CM call the same routing,
   cache, provenance, and accounting as a legacy call. The `metamind.think`
   model also falls back to `config/routing_table.toml`'s default, since
   `config/llm.toml` sets none.

Both were validated by live turns: the CM route now makes provider calls and
records one `llm_calls` row per call, exactly like the legacy route.

## What was run

Two fresh data roots, the same six prompts in the same order, both servers on
loopback, one turn at a time with 15 s between turns:

```bash
# Legacy route.
MM_CM_V3=false MM_CME_ENABLED=false \
  target/debug/mm-serve --data-dir /tmp/p12_final_legacy --port 39124

# New CM contract route.
MM_CM_V3=true MM_CME_ENABLED=true \
  target/debug/mm-serve --data-dir /tmp/p12_final_cm --port 39125
```

Prompts: kernel status; unavailable services; current goals; memory classes;
what the sanity firewall does; how many tools are registered.

### Legacy result — complete

All six turns completed (`0.50–1.87 s` client wall clock). Aggregate for
`purpose = converse`:

| calls | tokens_in | tokens_out | cost_micros | avg_ms | max_ms | cached | failed |
|---|---|---|---|---|---|---|---|
| 8 | 4,522 | 330 | 3,600 | 815.8 | 1,375 | 0 | 4 |

The four failed calls are upstream `429`/`503` responses that hit the console's
intro call and were absorbed by its fallback: the six turns still completed.
Per successful call this is roughly 1,130 input tokens, 900 micros, 1.1 s.

### New CM v3 result — partial

The paired CM window ran after the legacy window on the same key and every turn
was refused upstream (`429 RateLimitError: You exceeded your current quota`,
and one `400` carrying the same 429 body). The CM route itself was working:
the same deployment produced four successful live turns earlier on a fresh data
root, and those are the only successful CM rows available.

Successful-call rows:

| status | model | tokens_in | tokens_out | cost_micros | latency_ms |
|---|---|---|---|---|---|
| ok | gemini-2.5-flash | 6,590 | 92 | 1,650 | 1,351 |
| ok | gemini-2.5-flash | 6,804 | 46 | 1,650 | 1,165 |
| ok | gemini-2.5-flash | 6,596 | 115 | 1,650 | 1,417 |
| ok | gemini-2.5-flash | 6,572 | 93 | 1,650 | 1,733 |
| provider_error | gemini/gemini-2.5-flash | 0 | 0 | 0 | 1,426 |
| provider_error | gemini/gemini-2.5-flash | 0 | 0 | 0 | 2,274 |

Aggregate for that window:

| calls | tokens_in | tokens_out | cost_micros | avg_ms | max_ms | cached | failed |
|---|---|---|---|---|---|---|---|
| 6 | 26,562 | 346 | 6,600 | 1,561.0 | 2,274 | 0 | 2 |

Per successful call the CM path used about 6,640 input tokens and 1,650 micros
at 1.1–1.7 s. That is roughly 5.9× the legacy input tokens per call, driven by
the CM system prompt and the 17-tool schema block. It is **not** a verdict on
cost per completed turn: the CM sample is four turns, the prompt sets are not
paired, and the failed rows are quota noise rather than model behaviour.

The CM half must be rerun for the comparison below to mean anything.

## Acceptance thresholds for the first full run

These are still the thresholds the plan intended. With four successful CM calls
and an exhausted key, **none of them is evaluated**; treat this table as the
checklist for the rerun, not as a result.

| Measure | Threshold | Current state |
|---|---|---|
| New-path cost per completed turn | ≤ legacy path, or an explained one-time increase | not evaluated (N=4, unpaired) |
| New-path p50 latency | ≤ legacy p50 + 250 ms | not evaluated |
| New-path p95 latency | ≤ legacy p95 + 750 ms | not evaluated |
| Failed/schema-rejected calls | no increase over legacy | not evaluated; both windows dominated by upstream quota |
| Tool rounds per turn | bounded by the turn loop policy, no runaway | not exercised live (no tool round was taken) |
| Cache hit rate | not lower than legacy on the same prompt set | 0 cached on both; cache not exercised by distinct prompts |

Record the raw `llm_calls` rows with the rerun; do not average away a long tail.
If a threshold fails, set `MM_CM_V3=false` (and `MM_CME_ENABLED=false` if the
whole CME route is implicated), keep the data root for diagnosis, and file the
failing prompt/trace ids with the report.

## How to complete the measurement

The only missing ingredient is provider quota. The procedure above is correct
and now measurable end to end.

1. Use a key with headroom (or wait for the quota window to reset). The failure
   mode is unmistakable: `429`/`503` bodies from the proxy appear inside the
   `failed` SSE frame and as `provider_error` rows.
2. Run the six prompts against one route, then the other, on separate data
   roots. Throttle to roughly one turn every 20 s; the legacy route makes one
   or two calls per turn and the CM route one.
3. Fill the per-purpose table below for both roots, and record the successful
   and failed rows separately. A minimum of ~20 prompts per route is needed
   before p50/p95 mean anything.
4. Update the threshold table with a pass/fail per measure. If the CM path is
   more expensive only because of prompt size, say so with the per-call token
   split rather than calling it a regression.

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
mm-cli --data-dir <root> llm stats --json
```

## What this report claims and does not claim

- It claims both routes have now made real provider calls in this environment,
  and that both record them in `llm_calls`.
- It claims the legacy route completed the six-prompt set; the CM route did not,
  because of the provider quota, not because of a CM defect.
- It does not claim a cost or latency verdict, and it does not treat the
  four-call CM sample as a dual-run result.
- It does not use scripted/mock latency as provider latency.

The Phase 12 default (`MM_CM_V3=true`) still rests on passing parity tests and
an explicit rollback flag, not on a measured cost win. The rerun is the
remaining validation.
