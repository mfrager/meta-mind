# Three-Pass Process (Phase 3)

> **Status:** implemented, off by default · **Date:** 2026-11-23 · **Flag:** `MM_THREE_PASS_V2=false`
> **Plan:** [`design/five_phase_turn_process_plan.md`](../design/five_phase_turn_process_plan.md) Phase 3

Phase 3 extracts the model/tool loop into one reusable Process pass. The legacy
`cm.turn.v1` path and the three-pass `cm.process.v1` path run the same code; the
difference is the prompt strategy, the 17-tool scope, and the Interpret brief.
Verify remains the Phase 2 stub.

## 1. Contract

| Piece | Contract |
|---|---|
| Pass | `mm_cme::ProcessPass` — render, reserve, call, dispatch tools, re-render |
| Request | `ProcessRequest` — strategy, optional scope, Interpret brief, retry context, clean log, reply policy |
| Result | `ProcessRun` — final reply, Process ledger, rounds, tool steps |
| Context | `ProcessContext` — task, ≤8 constraints (≤300 chars each), advisory tools, budget hint |
| Retry | `RetryContext` — attempt, prior reply digest, failed labels/reasoning/confidence, exact error |
| Telemetry | `PassRecord { kind: process }` with the final prompt hash and budget |
| Config | `three_pass.process.budget_class`, `three_pass.process.strategy` (`cm.process.v1`) |

The executor keeps ownership of `Turn` mutation: chunk streaming, tool cards,
prompt pinning, and pass history are applied around the pass, not inside it.

## 2. Legacy parity

`TurnExecutor::execute` calls `ProcessPass` in both modes:

- Legacy: `cm.turn.v1`, no pass scope, no brief or clean-log transcript,
  `self.budget_class`, and the existing `PROMPT_UNAVAILABLE` and tool-cap errors.
- V2: `cm.process.v1`, `PassScope::Process`, the Interpret brief and clean log,
  `three_pass.process.budget_class`, child budget reservation, and
  `PROCESS_INCOMPLETE` when no final text is produced.

The legacy prompt fixture and the integration test pin the prompt contract, tool
sequence, final reply, and pass history. `crates/mm-cme/tests/phase3_process.rs`
is the parity harness.

## 3. Process prompt

The `cm.process.v1` strategy receives a dedicated brief:

```
[TASK]        one-sentence restatement from Interpret
[CONSTRAINTS] bullet list, each ≤ 300 chars, max 8
[TOOLS]       suggested subset, advisory only
[BUDGET]      class + remaining (advisory; enforcement is server-side)
[RETRY]       only on a retry: attempt n/5, prior reply digest,
              failed checks with reasoning/confidence, exact error
```

The brief is bounded when it is parsed from `metamind.verify`; unknown or
out-of-scope suggested tools are dropped. Retry context is appended as a
user-role message prefixed `RETRY_CONTEXT`, never as system text, and never
contains an internal judge prompt or a secret.

## 4. Scope and delegation

- Process advertises all 17 work tools and no control tools.
- `agent.run` and `metamind.think` are available only in Process (and Think for
  `agent.run`); Verify and Interpret cannot reach them.
- `metamind.verify` and `metamind.approve` are denied before dispatch in Process.
- `agent.run`, `metamind.think`, and `metamind.plan` reserve a child token
  ceiling from the Process ledger before they run. If the remainder cannot cover
  the ceiling, the call is refused with a `BUDGET` tool result and never
  dispatched. An unknown child usage count keeps the reservation.
- Tool emissions stay in the Process transcript and are recorded with their
  `ToolCallEmission` exactly as before.

## 5. Reply identity

- The final assistant text from Process is the reply. There is no Verify rewrite
  and no post-hoc summarizer.
- An empty final text in V2 fails with `PROCESS_INCOMPLETE`; nothing is
  synthesized.
- Chunks are spawned once per turn. Retry text will replace the pending bubble
  when Phase 4 schedules retries.
- Budget exhaustion mid-loop is a hard `BUDGET` failure; no Process record is
  added, so the turn timeline cannot imply Verify ran.

## 6. Limits

- Verify does not execute checks or approve/retry; that is Phase 4.
- Retry context is rendered and tested, but no retry attempt is scheduled yet.
- Staged writes, `VERIFY_EXHAUSTED`, recursive think, and the inspector pass
  timeline are Phase 4: see [`three-pass-verify.md`](./three-pass-verify.md).
- Think still uses the existing single-service `ThinkService`; recursive
  three-pass thinking is Phase 4 P4.7.
- `MM_THREE_PASS_V2` remains off by default and the legacy path remains the
  default runtime.
