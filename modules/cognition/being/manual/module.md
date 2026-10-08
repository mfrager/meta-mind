# being

**URI** `https://metamind.dev/code/module/cognition/being`
**Phase** 4 (being substrate)
**Capability** `mm:BeingState`

## What it is for

Every Metamind module is a nexus-style plugin: a directory with a `plugin.toml`
manifest, a stable `uri`, and the T-Box functions it adds to the monad.

Phase 4 introduces the *persistent self* — the small state that survives model
changes, context-window boundaries, and conversations. That state lives in
`crates/mm-being`, where the deterministic guard can enforce the four core
invariants. This module owns the **contract**: what the self is, which surfaces it
exposes, and what a caller may rely on without opening a store.

It is declarative on purpose. Duplicating the enforcement rules here would create a
second place for them to drift, and the whole point of the guard is that there is
exactly one.

## T-Box functions

| Function | Handler | Meaning |
|---|---|---|
| `being.identity` | `handlers::identity` | The immutable core: stable IRI, creation instant, and the four invariants |
| `being.personality` | `handlers::personality` | The constitution — values, dispositions, constraints — a model elaborates |
| `being.affect` | `handlers::affect` | The appraisal-driven control variable that asserts nothing |
| `being.beliefs` | `handlers::beliefs` | The evidence-gated user model |
| `being.relationships` | `handlers::relationships` | Event-sourced relationship projections |
| `being.goals` | `handlers::goals` | Goals (desire) and commitments (intention) |
| `being.resources` | `handlers::resources` | Accounts, ledger, and policies |

## Contract

- `surfaces()` returns one [`Surface`] per T-Box function, in the same order as
  `SURFACE_FUNCTIONS`. A surface names the tables it reads and writes, the
  invariant it must respect, and one sentence a caller can rely on.
- `being.affect` writes only `affect_*` tables. `affects_internal_state` and
  `affects_reasoning` are unsettable and always `false`, and the affect state
  carries no status-bearing predicate — an impulse can move the being, never
  inform it.
- `being.identity` and `being.goals` carry `no_history_rewrite`: a revision appends
  a row, and a terminal status is never rewritten.
- `being.beliefs` carries `no_assumption_to_observation`: `OBSERVED` requires an
  observation record.
- The manifest is embedded with `include_str!`, so a malformed `plugin.toml` is a
  compile error, not a load-time surprise. `manifest_at` exists to prove the file
  on disk still matches the binary.

## Ownership

Phase 4 owns this module. Phase 5 (memory) and later phases may *read* it — the
code graph registers its URI and T-Box functions — but must not change its
contract. The state it describes is guarded in `crates/mm-being`, which is the only
place a mutation may be proposed and the only place the guard runs.
