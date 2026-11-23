# Three-Pass Substrate (Phase 1)

> **Status:** implemented, behavior-neutral · **Date:** 2026-11-23 · **Flags:** `MM_THREE_PASS_V2=false`, `MM_PROCESS_V1=false`
> **Plan:** [`design/five_phase_turn_process_plan.md`](../design/five_phase_turn_process_plan.md) Phase 1

Phase 1 lands the contracts every later pass and process step uses. No route, prompt,
or tool behavior changes while both flags are off: `CmTools::schemas()` still
advertises the original seventeen work tools, and `TurnExecutor::execute` is
untouched.

## 1. What exists now

| Area | Contract |
|---|---|
| Verification | `VerificationBackend` trait, `BackendRegistry`, `VerifyCheck`, `VerificationResult`, `VerifyContext`, `ProcessResult` |
| First backend | `LlmJudgeBackend` (`llm_judge`) with a strict JSON result and threshold/clamp handling |
| Tool catalog | 17 work tools + 2 control tools (`metamind.verify`, `metamind.approve`) = 19 |
| Pass scopes | `PassScope::{Interpret, Process, Verify, Think}`; server-enforced by the dispatcher |
| Prompts | `cm.interpret.v1`, `cm.process.v1`, `cm.verify.v1`, `cm.think.v1` plus eleven fragments |
| Budgets | `TurnEnvelope` totals the passes, caps retries, and reserves before a retry runs |
| Events | `pass_started`, `pass_completed`, `verification_registered`, `verification_result`, `retry_scheduled`, `write_staged`, `write_committed`, `write_discarded` |
| Clean log | `mm_data::CleanLog` projection of user messages and approved final replies |
| Config | `config/three_pass.toml`, parsed and validated by `mm_cme::ThreePassConfig` |
| Flags | `MM_THREE_PASS_V2`, `MM_PROCESS_V1` (both default off) |

## 2. Verification

A check is validated when it is registered, not when it runs:

- label 1–64 chars, unique within the turn; at most 8 checks;
- description/details ≤ 2,000 chars; serialized `backend_params` ≤ 8 KB;
- threshold finite and within `[0, 1]`;
- backend must be registered, and the backend validates its own parameters.

The LLM judge requires `evaluation_prompt` and `expected_output_schema` and
declares the three result fields it parses (`pass`, `reasoning`, `confidence`).
It renders the prompt through the same strict `{{slot}}` renderer as the prompt
tree (`user_request`, `process_reply`, `check_details`, `state_summary`), makes a
`Purpose::Critique` call through the configured `LlmClient` with `CacheMode::Bypass`,
and validates the returned JSON against the declared schema subset. Malformed
output is an error, never a silently failed check. Confidence is clamped to
`[0, 1]`; a check passes only when the judge says pass **and** confidence meets
the check's threshold.

The judge does not have a client of its own: `LlmJudgeBackend::new(client, model)`
takes the same `Arc<dyn LlmClient>` the turn uses, so accounting and routing apply.
Without a client it returns an honest `Model` error.

## 3. Pass scopes

| Tool | Interpret | Process | Verify | Think |
|---|---|---|---|---|
| `conversation.topic` / `.depth` / `.status` / `.frame` | ✓ | ✓ | ✓ | ✗ |
| `agent.run` | ✗ | ✓ | ✗ | ✓ |
| `data.create` / `.update` / `.delete` | ✗ | ✓ | ✓ | ✗ |
| `data.query` / `.search` / `.schema` | ✗ | ✓ | ✓ | ✓ |
| `data.namespace` | ✗ | ✓ | ✓ | ✗ |
| `metamind.plan` | ✗ | ✓ | ✓ | ✗ |
| `metamind.think` | ✗ | ✓ | ✗ | ✗ |
| `metamind.memory` | ✗ | ✓ | ✓ | recall only |
| `metamind.log` | ✗ | ✓ | ✓ | ✓ |
| `metamind.act` | ✗ | ✓ | ✓ | ✗ |
| `metamind.verify` | ✓ | ✗ | ✗ | ✓ |
| `metamind.approve` | ✓ (true) | ✗ | ✓ (false/true) | ✓ |

Counts: Interpret 6, Process 17, Verify 16, Think 8. Two independent layers
enforce this: `ToolCatalog::schemas_for(scope)` controls what the model sees, and
`CmTools::with_pass_scope(scope)` rejects anything else with `TOOL_NOT_AVAILABLE`
before arguments are parsed. The second layer is what catches prompt drift.

Control-tool arguments are `metamind.verify {checks, process?}` and
`metamind.approve {success, reply?, error?}`. Phase 1 defines and validates the
schemas. Phase 2's Interpret runner consumes `metamind.verify` and
`metamind.approve` directly; Verify's `approve(false)` handler arrives in Phase 4.
See [`three-pass-interpret.md`](./three-pass-interpret.md).

## 4. Prompts

The four strategies reuse the existing fragments (`identity`, `character`,
`state_ownership`, `budget_contract`, `honesty_contract`, `safety_contract`, and
so on) and add pass-specific fragments:

- `interpret_contract`, `verification_guide`, `fast_path_criteria`;
- `process_execution`, `tool_selection`;
- `verify_contract`, `verification_review`, `state_update_guidance`, `retry_guidance`;
- `think_contract`, `deep_analysis`.

No prompt text lives in Rust. A new strategy must declare every slot its fragments
use; the existing `every_strategy_renders_with_standard_values` test renders the
whole tree, so an undeclared slot fails the build.

## 5. Budgets

`TurnEnvelope::standard(max_retries)` is quick + standard + quick: 48 rounds,
136 steps, 184,000 tokens, 810,000 ms. Each pass still gets its own class ledger.
`absorb()` accounts a finished ledger and refuses an overrun; `reserve_retry()`
reserves the retry's token ceiling up front and enforces the retry cap;
`release_reservation()` returns it after the retry is accounted. The envelope is
the hard ceiling above the per-pass classes, not a replacement for them.

`config/three_pass.toml` sets `envelope.max_tokens = 184000` to match. A lower
value is a supported tighter ceiling; a higher value is clamped by the envelope
constructor.

## 6. Events and clean log

The eight new event types are additive. Existing readers ignore unknown variants
(and the `Custom` fallback remains). A clean log is built by
`CleanLog::from_events(&events, max_chars)`:

- `UserMessage` contributes its `text` or `query`;
- `AgentMessage` contributes only when `payload.final == true`;
- tool results, pass text, verification reasoning, and rejected attempts never
  enter;
- the newest turns win a size bound (default 16,000 chars) and `truncated` says
  whether older turns were dropped.

Traces written before this change have no `final` marker, so they yield an empty
clean log rather than a polluted one. `agent_message()` now sets `final: true`.

## 7. Configuration and flags

`mm_cme::ThreePassConfig::load_or_default(path)`:

- missing file → defaults;
- unreadable or invalid file → error (never a silent fallback);
- unknown keys are rejected;
- `max_checks` 1–8, `max_retries` ≤ 10, `max_parallel_checks` 1–8,
  `check_timeout_ms` 1,000–120,000, `envelope.max_tokens` ≥ 8,000.

`MM_VERIFY_MAX_RETRIES` may only lower the file's `max_retries`; an invalid or
too-large value is an error.

`MM_THREE_PASS_V2` and `MM_PROCESS_V1` are in `FeatureFlags` and default to
`false`. Phase 1 adds the switches; Phases 2–3 read `MM_THREE_PASS_V2`, and
Phase 5 owns `MM_PROCESS_V1`.

## 8. Limits of this phase

- Phase 1 alone runs no pass; Phases 2–3 add Interpret and Process, and Phase 4
  adds Verify and retries.
- The control tools are protocol markers consumed by the runner, not handlers in
  the work-tool dispatcher; the legacy path still returns its typed
  `NOT_AVAILABLE` envelope.
- Staged writes and retries are Phase 4; the Process pipeline is Phase 5.
- The judge's JSON-schema support is the deliberate subset needed for a flat
  verdict object; it is not a general JSON Schema engine.
- `CleanLog` requires the `final` marker; the Phase 2 clean-log logging writes it.

## 9. Verification

```
cargo test -p mm-cme
cargo test -p mm-data
cargo test -p mm-serve --tests
cargo clippy -p mm-cme -p mm-data -p mm-serve --all-targets -- -D warnings
cargo fmt --all -- --check
```

Phase 1 adds tests for backend registration and validation, judge parsing and
thresholds, the 17+2 catalog and per-pass projections, dispatcher scope denial,
the four strategies rendering, envelope accounting and retry reservation, config
parsing/validation, and the clean-log projection. The legacy Phase 2 catalog test
still asserts exactly seventeen advertised schemas with no scope set.
