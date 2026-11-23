# Three-pass Verify: checks, staged writes, retries, and recursive think

**Status:** Phase 4 implemented 2026-11-23. `MM_THREE_PASS_V2` remains off by
default; the default flip is a separate change after the live gate.

Phase 4 completes the turn architecture started in Phases 2 and 3. Interpret
plans the checks, Process does the work, and Verify runs the checks, reviews the
reply, and either approves it or sends Process back with the failure.

## 1. What ships

| Piece | Where |
|---|---|
| Phase A check runner (bounded parallel, timeouts, declaration order) | `mm_cme::ThreePassRunner::run_checks` |
| Staged write journal + commit replay | `mm_cme::verification::MutationJournal`, `ThreePassRunner::commit` |
| Phase B review (`cm.verify.v1`) with approve/reject | `ThreePassRunner::run_verify` |
| Retry loop, budget pair reservation, exhaustion | `TurnExecutor::execute_three_pass` |
| Three-pass orchestration and turn outcome mapping | `TurnExecutor::execute_three_pass` |
| Pass timeline frames + inspector timeline | `ChatFrame::{PassStart,PassComplete,Verification}`, `pass-timeline.tsx` |
| Verified indicator on the reply | `ChatFrame::FinalResponse.verified` |
| Recursive think | `ThreePassRunner::run_think`, `ThinkService::with_runtime` |
| Pass events and writes in the agentstream | `CmeRuntime::log_turn_frames` |
| Think default check | `config/prompts/fragments/think_check.toml` |

## 2. Turn shape

```
Interpret  → checks + ProcessContext, or fast path
Process    → reply_text + tool emissions + durable effects
Phase A    → one VerificationResult per check, in declaration order
Phase B    → review with staged writes; approve(true|false)
approve    → commit journal, finalize the Process reply
reject     → discard journal, retry Process with RetryContext
exhausted  → Failed(VERIFY_EXHAUSTED), effects reported, reply withheld
```

Only the Process reply is user-visible. Verify never rewrites it. Exactly one
user message and one approved assistant reply enter the clean log per turn.

## 3. Phase A

- Checks run concurrently up to `verify.max_parallel_checks` (default 4) and the
  result vector keeps declaration order.
- Each check has `verify.check_timeout_ms` (default 20 s). A timeout, a backend
  error, and an unknown backend all become `pass:false` with `error` set; they
  never abort the other checks and never pass.
- Checks see the final Process reply, the clean log, `[STATE]`, and the user
  request. They do not see the Process transcript or raw tool arguments.
- No checks is legal only for a turn the runner itself defaulted (the implicit
  `action` check); a review still runs, because Verify is also the bookkeeping
  pass.

## 4. Staged writes (D4)

Verify runs `WriteMode::Staged`:

- Read-only tools execute immediately.
- Mutating calls (`data.create/update/delete`, `data.namespace`,
  `metamind.memory` remember/forget, `metamind.plan` create/update,
  `metamind.act` non-control) are validated and appended to a
  `MutationJournal` with the revision observed at staging time. They execute
  nothing; the tool result says `staged:true` and echoes the intent.
- `approve(true)` replays the journal in order through the real dispatcher. A
  revision conflict gets exactly one retry against fresh state; a second
  failure ends the turn `VERIFY_COMMIT_FAILED` and retains the journal.
- `approve(false)` discards the journal and emits `WriteDiscarded`.

Process effects are **not** staged. Process does real work, and a rejected turn
does not undo it. The dispatcher records those effects (`ProcessMutationLog`) so
`RetryContext` tells the next attempt what already exists; the exhaustion
message names what changed. The system never claims nothing happened.

## 5. Phase B review

- `cm.verify.v1` sees the clean log, state, request, Process reply, Phase A
  results, retry count, and staged intent previews.
- Verify scope: 16 tools (all work tools except `agent.run` and
  `metamind.think`) plus `metamind.approve`. Control tools and delegation are
  denied before argument parsing.
- The model must end with exactly one `metamind.approve`. Text without an
  approve gets one protocol repair, then `VERIFY_PROTOCOL`.
- A reviewer may not approve while a Phase A check failed unless
  `verify.allow_override=true` (default false). The refusal is
  `VERIFICATION_OVERRIDE_DENIED`; an allowed override records
  `approved_override`.

## 6. Retries and outcomes

- `verify.max_retries` (default 5) means at most six Process attempts.
- A retry reserves a full Process+Verify budget pair up front. If the envelope
  cannot cover it, the turn ends `BUDGET` instead of running a half-retry.
- Interpret is not re-run on a retry; its checks are fixed for the turn.
- Retry context contains the attempt number, the prior reply digest, the failed
  check labels/reasoning/confidence, the reviewer's error, and the prior
  attempt's durable effects. It is a user-role message prefixed
  `RETRY_CONTEXT`.
- `VERIFY_EXHAUSTED` withholds the unverified reply and names the failed check
  and the durable changes already made.

| Outcome | Turn state |
|---|---|
| fast path, or approved | `completed` |
| `INTERPRET_PROTOCOL`, `VERIFY_PROTOCOL`, `VERIFY_COMMIT_FAILED`, `BUDGET`, `PROCESS_INCOMPLETE` | `failed` with the typed error |
| retries exhausted | `failed` with `VERIFY_EXHAUSTED: …` |
| interrupted between attempts | `interrupted`; staged journal discarded |

## 7. Recursive think (P4.7)

Set `[think] three_pass = true` in `config/three_pass.toml` (and have
`MM_THREE_PASS_V2` on) to route `metamind.think` through the runner:

- The thinker gets its own session (`think/<parent_turn>/<ulid>`), the Think
  tool scope (read-only data, `metamind.log`, recall-only memory, `agent.run`,
  the two control tools), and no user-state writes.
- It skips a separate Interpret model call: the question is the task, so the
  runner injects the step-style check from `think_check.toml`.
- Process runs `cm.think.v1`; Verify reviews under the same Think scope, so a
  write attempted by the reviewer is denied by the dispatcher, not merely
  discouraged by the prompt.
- The Verify journal is never committed. A thought returns findings, not
  durable state changes.
- The result keeps the `ThinkResult` shape used by the single-service path, so
  callers do not branch. With the flag off, `ThinkService` behaves exactly as
  before.

## 8. Telemetry and UI

- Each pass records `{pass, attempt, prompt_hash, budget, outcome, checks,
  verification, staged, committed}` on the turn.
- SSE adds `pass_start`, `pass_complete` (with `promptHash`), and `verification`
  frames. Only `final:true` pass tokens are assistant content.
- The inspector Trace tab shows a pass timeline: pass, attempt, outcome,
  remaining budget, prompt hash, and each check's label/pass/confidence.
- The chat shows one reply bubble with a subtle `verified` chip when the final
  Verify approved a turn with checks. Retries add timeline entries, not bubbles.
- Agentstream adds `pass_started`, `pass_completed`, `verification_result`,
  `retry_scheduled`, `write_staged`, `write_committed`, and `write_discarded`.

## 9. Configuration

```toml
[verify]
max_retries = 5            # MM_VERIFY_MAX_RETRIES may only lower this
check_timeout_ms = 20000
max_parallel_checks = 4
allow_override = false

[think]
three_pass = false
```

## 10. Validation

`crates/mm-cme/tests/phase4_verify.rs` covers: implicit check injection and
approval; a failed check retrying Process and committing the journal once; the
staged create never executing before approval; exhaustion withholding the reply
without committing; override denied by default and allowed when configured; a
commit failure retaining the journal; declaration-order preservation, timeout,
backend error, and unknown backend mapping; and recursive think running under
the Think scope with delegated read-only tools and no writes.

`crates/mm-serve/tests/phase4_verify.rs` covers the commit path: a staged write
is validated without executing, an approval applies it exactly once, a
revision conflict is retried once against fresh state (`retried:true`), and a
commit that cannot converge fails with `VERIFY_COMMIT_FAILED` naming the intent.
The `mm-serve` unit tests cover the pass timeline frames, including the prompt
hash and the `final:true` fast-path token.

The live three-pass dual-run was **not** executed in this change. Provider
quota was not confirmed available; the plan forbids replacing a live record with
scripted latency. See `phase12-dual-run.md` §"Three-pass turn run" for the
command and what to record.
