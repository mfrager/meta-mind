# closed-loop

**URI** `https://metamind.dev/code/module/cognition/closed-loop`
**Phase** 12 (autonomy / self-bootstrap)
**Capability** `mm:ClosedLoopAutonomy`

## What it is for

Every Metamind module is a nexus-style plugin: a directory with a `plugin.toml`
manifest, a stable `uri`, and the T-Box functions it adds to the monad.

This module is the developmental loop's own registration. Phase 12's loop runs ten stages
over an event-sourced run — experience → event log → meta-analysis → capability gap →
change set → self-engineering → test/benchmark → shadow → promotion gate → new version —
and the *orchestration* of those stages lives in `mm-runtime::loop_controller`, next to
the budget arithmetic, the sandbox and the promotion call. This module deliberately owns
none of that.

What it owns is the part that can be checked without a store:

- the **order** of the ten stages, as a constant a recorded run is compared against; and
- a deterministic **summary** of a run's stage records, including `first_refusal` — the
  first stage that was refused or failed, in run order.

The summary is the module's reason to exist. Reading `10 stages, 9 ok` tells an operator
nothing about what happened; `refused: 1, first_refusal: promotion_gate: refused` tells
them the loop ran to the end and the gate said no, which is a decision, not a crash.

It is deliberately host-independent: `READS` and `WRITES` are empty, because the module is
pure arithmetic over records handed to it. The rows that persist a run — `loop_runs`,
`loop_iterations`, `module_loads`, `design_revisions` — are `mm-runtime`'s, and a second
writer of them here would be a second opinion about the loop's record.

## T-Box functions

| Function | Handler | Input | Output |
|---|---|---|---|
| `cognition.loop_status` | `handlers::loop_status` | `stages`: a JSON array of `{idx, stage, outcome, artifact}` | the `LoopSummary`: `{stages, ok, refused, failed, first_refusal, complete}` |
| `cognition.loop_stage_order` | `handlers::loop_stage_order` | none | `{stages, count, function}` |

Rust callers can use the same arithmetic with types — `summarize`, `order_matches`,
`expected_stage`, `stages_from_json`, `STAGES`, `OUTCOMES` — without going through JSON.

## Contract

- **`complete` is stricter than "all ten stages ran".** It is true only when all ten
  stages are present exactly once *and* every outcome is `ok`. A run whose promotion gate
  refused ran every stage and produced nothing, and calling that complete would report the
  loop's shape rather than its result.
- **`refused` and `failed` are different outcomes.** A stage the budget envelope denied is
  not a stage that broke. The phase's rule is that a refusal is a decision, so the summary
  counts them separately and `first_refusal` names which one happened first.
- **A malformed record is refused, never repaired.** An unknown stage name, an index that
  repeats, an eleventh index, a record whose name does not match its index, and an outcome
  outside `ok|refused|failed` are all refusals naming the offending record: `json` when
  the payload is not the shape the function takes, `validation` when a value is understood
  and impossible. Re-ordering or de-duplicating a caller's records would silently assert
  that the loop's own record is well formed, which is the property the summary exists to
  check.
- **The order is data, not a computation.** `STAGES` is the list the controller walks and
  `order_matches` compares a recorded run against it. Deriving the order from the records
  would make every run correct by construction.
- The manifest is embedded with `include_str!` and parsed by a test, so a malformed
  `plugin.toml`, or a `uri` that has drifted from this directory, fails a test rather than
  a load.

## Ownership

Phase 12 owns this module. The loop it describes is Phase 12's (`mm-runtime`), the
self-engineering stages it sequences are Phase 11's (`mm-selfeng`, `mm-pi`), and the
gate it ends at is Phase 11's deterministic promotion gate. Later phases may *read* this
module — the code graph registers its URI, its two T-Box functions and the tests that
cover its capability — but must not change its contract: a stage cannot be inserted,
renamed or reordered without changing `STAGES`, and every recorded run in the event log
is compared against exactly that list.
