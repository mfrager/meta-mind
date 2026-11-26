# comparison-integrity

**URI** `https://metamind.dev/code/module/cognition/comparison-integrity`
**Phase** 9 (sanity firewall, decision core, comparison integrity)
**Capability** `mm:ComparisonIntegrity`

## What it is for

Every Metamind module is a nexus-style plugin: a directory with a `plugin.toml`
manifest, a stable `uri`, and the T-Box functions it adds to the monad.

Phase 9 adds the two organs between *reasoning* and *action*: the bounded-decision
primitive and the sanity firewall. This module owns the third thing that phase
introduced, design §20's **comparison integrity** — because most decisions that look
wrong in hindsight are not reasoning failures at all, they are *invalid
comparisons*: a latency under one load set against a latency under another, a cost in
one currency against a cost in another, a benchmark of one model against its
successor.

The mechanism lives in `mm-decision::compare`, where the validity rules, the refusal
verdicts and the normalization arithmetic are deterministic. This module owns the
**contract**: which surface exists, the tables it reads and writes, the invariant it
must respect, and one sentence a caller may rely on without opening a store.

It is declarative on purpose. Duplicating the checks or the unit conversions here
would create a second place for them to drift, and the whole point of the subsystem
is that there is exactly one refusal.

## T-Box functions

| Function | Handler | Meaning |
|---|---|---|
| `cognition.compare_contract` | `handlers::compare_contract` | Check a pair of comparison contracts *before* any comparison is made |

## Contract

- `surfaces()` returns one [`Surface`] per T-Box function, in the same order as
  `SURFACE_FUNCTIONS`. A surface names the tables it reads and writes, the invariant
  it must respect, and one sentence a caller can rely on.
- **A comparison is checked, never assumed.** `check_contract` returns `valid`,
  `non_comparable` or `rejected`, and a mismatch is never coerced into a number: a
  `NonComparable`/`Rejected` pair has no normalized comparison at all, and the
  firewall turns that into `REPLAN` rather than into a comparison nobody validated.
- **A conversion stays inside its dimension.** `normalize` converts only between
  units whose declared dimension agrees, using the contracts' own `to_base` ratios,
  applies the `ceil((n+1)·coverage)`-style rules of the compare module's own
  arithmetic unchanged, and records every conversion it applied, so a reader can see
  what happened to the numbers.
- **The verdict vocabulary is re-exported, not redefined.** `verdict_names()` and
  `violation_codes()` read the compare module's `COMPARISON_VERDICTS` and
  `VIOLATION_CODES`, so a caller branching on the strings cannot drift from the type.
- The manifest is embedded with `include_str!`, so a malformed `plugin.toml` is a
  compile error, not a load-time surprise. `manifest_at` exists to prove the file on
  disk still matches the binary.
- `bench/comparison/fixtures.jsonl` is the corpus `mm-cli compare check` grades:
  matched pairs must be `Valid`, and every violation code must be produced by a
  fixture that names it.

## Ownership

Phase 9 owns this module, `crates/mm-decision`, and `crates/mm-firewall`. Phase 10
(tool execution) consults comparisons before acting; Phase 11 (calibration and local
models) reads the recorded verdicts rather than re-deriving them. Later phases may
*read* this module — the code graph registers its URI and T-Box functions — but must
not change its contract.
