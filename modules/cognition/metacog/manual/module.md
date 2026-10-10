# metacog

**URI** `https://metamind.dev/code/module/cognition/metacog`
**Phase** 8 (metacognitive controller)
**Capability** `mm:CognitiveControl`

## What it is for

Every Metamind module is a nexus-style plugin: a directory with a `plugin.toml`
manifest, a stable `uri`, and the T-Box functions it adds to the monad.

Phase 8 introduces the *metacognitive controller* — the system's executive. It
inspects a provisional model of a problem, decides what kind of cognition is needed
and how much is sufficient, compiles a temporary cognitive program (a typed graph of
cognitive operations), and lowers it to an executable computation DAG with a stable
`dag_hash`. The mechanism lives in `crates/mm-metacog`, where the one broad scan, the
tier policy, the deterministic operation value, the compiler, the budget forcing and
the replayable trace are pure functions with their own tests. This module owns the
**door**: the two surfaces a caller can invoke without linking the controller.

It is a thin adapter on purpose. Duplicating the scoring or the composition here
would create a second place for it to drift, and the whole point of the compiler in
`mm-metacog` is that there is exactly one.

## T-Box functions

| Function | Handler | Input | Output |
|---|---|---|---|
| `cognition.plan` | `handlers::plan` | an `mm_metacog::CognitiveEpisode` as JSON and an `mm_metacog::ScanResult` as JSON | the compiled `mm_metacog::CognitiveProgram` as JSON, including its content-derived `id` |
| `cognition.op_value` | `handlers::op_value` | `expected_error_reduction`, `decision_importance`, `probability_change`, `cost` | `{eer, importance, p_change, cost, score, canonical}` |

## Contract

- **The module never mints compute of its own.** The tier's compute policy and the
  budget come from the episode that was handed in; `plan` cannot raise either. A
  door that could would be a second, unaudited source of compute, and the phase's
  whole first invariant — spend the least cognition that can change the decision —
  would stop being enforceable.
- **A refusal is typed, and never a partial program.** A payload that is not the
  shape the function takes is a `json` refusal; a payload the controller understands
  and refuses is a `refused` refusal carrying the controller's own code
  (`validation`, `scan`, `graph`, `compile`, …). A caller that received a program
  with no operations could not tell a refusal from a successful compile of nothing,
  so neither function returns one.
- **Nothing panics on malformed input.** Both functions are total over
  `serde_json::Value`; unwrapping a caller's payload at a module boundary would turn
  a bad document into a dead process.
- **`op_value` is the controller's own expression**, `eer * importance * p_change /
  max(cost, EPS)`, with an epsilon floor so a free operation is never a division by
  zero. An input outside `[0,1]`, or a negative or non-finite cost, is refused
  rather than clamped: a clamped score would let a caller's bug read as a
  legitimate ranking.
- **The program is deterministic.** `id` is derived from the program's own canonical
  content, so compiling the same episode and scan twice yields byte-identical
  programs — which is exactly what `mm-cli episode replay` compares. A caller may
  therefore cache a program by `(episode, scan)` without a validity window.
- The manifest is embedded with `include_str!`, so a malformed `plugin.toml` is a
  compile error, and a `uri` that has drifted from this directory fails a test.

## Ownership

Phase 8 owns this module, `crates/mm-metacog`, and the `episodes`, `programs` and
`program_traces` tables of `0008_metacog.sql`. Phase 9 (decision firewall) consumes
the compiled program's `Act` operations before they run; Phase 10 executes the tool
bound ones; Phase 11 optimizes the programs and traces this phase records; Phase 12
schedules episodes and closes the loop. Later phases may *read* this module — the
code graph registers its URI and T-Box functions — but must not change its contract.
