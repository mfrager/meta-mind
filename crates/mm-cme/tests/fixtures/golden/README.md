# Golden three-pass fixtures

Reference captures of the three-pass turn pipeline at the legacy freeze
(`legacy-freeze-2026-11-25`, commit `29614dc`). They are the "before" side of the
streamlined-core cutover: Phase 2 must reproduce the prompt inputs, pass
ordering, and verdicts, while writing them as agentstream JSONL instead of the
old LLM activity feed.

## Provenance

Produced by `crates/mm-cme/tests/golden_capture.rs`, which drives the real
`TurnExecutor` + `ThreePassRunner` with `mm_llm::MockClient`. The mock serves
scripted model output and records the requests verbatim. No provider is called
and no network access is needed.

| File | Scenario |
|---|---|
| `fast_path.json` | Interpret answers with `metamind.approve`; no Process or Verify pass |
| `action_approved.json` | Interpret registers no checks; Process answers; the implicit action check is judged; Verify approves |
| `action_retry_then_approved.json` | Verify rejects attempt 1; Process retries and stages `data.create`; Verify approves and commits |
| `events/<name>.jsonl` | The same turn projected into the agentstream v2 envelope |

## What a capture contains

- `request`: the session id, turn id, and user input.
- `toolScope`: every tool name visible to Interpret, Process, and Verify. These
  are the 17 work tools and 2 control tools of the legacy catalog; Phase 2
  reduces them to the six `conversation.*` / `metamind.verify` /
  `metamind.approve` tools.
- `calls[]`: one entry per model call, in order.
  - `messages`: the complete rendered array exactly as sent — the system
    fragments with the `[STATE]` block, the clean log, and the user input. This
    is the payload the frontend prompt viewer must show; a hash is never a
    substitute.
  - `tools`: the complete `ToolSchema` objects offered for that call, including
    `parameters`.
  - `pass`, `round`, `trigger`, `tool`: the call attribution that the
    agentstream gateway persists on `llm.request`.
  - `response`: text, tool calls, usage, and model, or `error` for a transport
    failure.
- `outcome`: `pass_history` (pass events), `verification` results,
  `staged_writes`, `committed_writes`, the final reply, and the error if the turn
  failed.

The JSONL projection is the target wire shape from §4 of
`design/streamlined_core_four_phase_plan.md`: one record per line with the v2
envelope (`v`, `seq`, `id`, `sessionId`, `turnId`, `at`, `type`, `source`,
`model`, `data`). It includes `turn.started`, `pass.started`,
`llm.request/response`, `tool.call`, `verification.result`, `pass.completed`,
and `turn.completed`. `llm.delta` is intentionally absent: deltas are live-only
and never persisted.

## Determinism

The test compares the live pipeline against these files byte for byte. To keep
that stable:

- Wall-clock fields (`elapsed_ms`, `wall_ms`) are written as `0`.
- The mock's per-call ULID (`call_id`) is written as `"call"`; the fixture's
  own `callId` is the stable `call-<index>`.
- Event ids are `evt-<seq>` and `at` is a fixed base plus `seq`.

Two limitations are deliberate:

- The `[STATE]` block is a pinned literal in the test, not rendered from
  `mm-data`. The shape matches `mm_cme::state_block`; the values are fixed so
  the fixture is stable without a data layer.
- `toolScope` names the legacy 19-tool catalog. It is a record of what used to
  be visible, not a proposal for the new scope.

## Regenerating

Only after an intentional pipeline or prompt change:

```bash
REGENERATE_GOLDEN=1 cargo test -p mm-cme --test golden_capture
```

Review the diff before committing. A regenerated fixture that silently drops a
prompt fragment or a tool is exactly the regression this file exists to catch.
