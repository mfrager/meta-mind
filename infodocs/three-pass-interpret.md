# Three-Pass Interpret (Phase 2)

> **Status:** implemented, on by default · **Date:** 2026-11-23 · **Flag:** `MM_THREE_PASS_V2=true` (set `false` for the single-loop path)
> **Plan:** [`design/five_phase_turn_process_plan.md`](../design/five_phase_turn_process_plan.md) Phase 2

Phase 2 makes Interpret a real pass. Pure conversation completes after one model
call; a work request registers checks and a Process brief, then the Process
pass runs. Verify is deliberately stubbed to approve the completed
reply. The legacy path is unchanged while the flag is off.

## 1. Contract

| Piece | Contract |
|---|---|
| Runner | `mm_cme::ThreePassRunner` — prompt rendering, scope, budget, transcript, control protocol |
| Context | `PassContext` — clean log, state block, request, variables, model, envelope; no Process/Verify data in Interpret |
| Outcome | `PassOutcome` — text, prompt ids/hashes, tool emissions, budget snapshot, finish, checks, Process brief, updated envelope |
| Finishes | `fast_path`, `action`, `approved`, `rejected`, `protocol_error`, `exhausted`, `error` |
| Telemetry | `mm_cme::PassRecord` on `Turn`/`TurnOutcome`, plus `pass_start` / `pass_token` / `pass_complete` SSE frames |
| Config | `config/three_pass.toml`; `config/prompts/fragments/interpret_repair.toml` and `default_action_check.toml` |

`PassKind::Interpret` maps to `PassScope::Interpret`, which advertises six tools:
the four `conversation.*` tools plus `metamind.verify` and `metamind.approve`.

## 2. Interpret flow

1. Render `cm.interpret.v1` with the clean log as transcript, the canonical
   `[STATE]` block, the request, the visible tool schemas, and the quick ledger.
2. Accept exactly one ending per round:
   - `metamind.approve {success:true, reply}` → fast path;
   - `metamind.verify {checks, process?}` → action, then Process;
   - `conversation.*` → run immediately and continue the loop.
3. `metamind.approve` must be the only call in its round. A non-empty reply of at
   most 8,000 characters is required.
4. `metamind.verify` must be the only call in its round. Checks are validated by
   the Phase 1 registry (count, labels, backend, threshold, parameter size) and
   the count is capped by `interpret.max_checks`. An empty list is legal for an
   action turn: the server adds the implicit `action` LLM-judge check.
5. `process` is optional. Omitted values derive `{task: request, constraints: [],
   suggested_tools: none, budget_hint: process.budget_class}`. Suggested tools
   are intersected with the Process scope.
6. Any other tool gets `TOOL_NOT_AVAILABLE` and is never dispatched.
7. Text with no control call triggers one protocol-repair turn. A second failure
   ends with `INTERPRET_PROTOCOL`; no partial reply is committed.

## 3. Fast path

- One model call, quick ledger only, no Process or Verify events.
- The reply is emitted as the final assistant message; `PassToken` carries
  `final:true`.
- Conversation state changes only through the `conversation.*` calls the model
  actually made. There is no implicit topic or depth guessing.
- The turn records one `PassRecord { kind: interpret, outcome: fast_path }`.

## 4. Action path

- Interpret records `action` with its checks and brief; the runner's `envelope`
  is returned for later passes.
- Process is the reusable `ProcessPass`, restricted to `PassScope::Process`
  (17 work tools, no control tools). Its prompt is `cm.process.v1`, with the
  Interpret task, constraints, suggested tools, budget, and optional retry
  context rendered into dedicated sections.
- The legacy path runs the same pass over `cm.turn.v1` with no scope or brief;
  Phase 3's parity fixtures pin that behavior.
- A completed Process reply records `PassRecord { kind: process, outcome:
  completed }`, followed by the Phase 2 Verify stub `approved`.
- The reply contract, chunking, and tool emissions are the same as the legacy
  turn path. Process text is the reply; no Verifier rewrite exists yet.

## 5. Context and clean log

The executor rebuilds the clean log from the agentstream before Interpret through
`CleanLogProvider`. Only `UserMessage` and `AgentMessage` with `final:true`
contribute; tool results, pass text, verification reasoning, and rejected
attempts never do. The log is injected as transcript messages, not concatenated
into the system prompt, so prompt-hash changes stay attributable.

`[STATE]` is rendered into the Interpret system prompt. The clean log is bounded
to 16,000 characters (~4,000 tokens).

## 6. Limits

- Verify is a stub: checks are registered but not executed.
- Retries, staged writes, and `approve(false)` are Phase 4. Phase 3 can render a
  retry context, but the executor does not schedule a retry yet.
- Think recursive three-pass mode is Phase 4; `metamind.think` still uses the
  existing `ThinkService` inside Process.
- The fast path is one call only in tests today; the live route still depends on
  a configured model.
- `MM_PROCESS_V1` remains unrelated and off.
