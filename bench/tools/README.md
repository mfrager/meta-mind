# `bench/tools` — Phase 10 fixtures

Three kinds of fixture live here, and each one is *data a test or the gate reads*, not
prose about what the code does.

## 1. Clause sets for the symbolic verifier

| File | What it is | Expected verdict |
|---|---|---|
| `seeded_inconsistency.logic` | two complementary unit clauses, `p(a)` and `not p(a)` | `refuted`, with the pair named as the counterexample |
| `no_refutation.logic` | two unrelated unit clauses | `inconclusive` — **not** `proven` |

The second file is the important one. `mm-cli verify run symbolic` reports
`inconclusive` rather than `proven` when its refutation search finds nothing, because
unit resolution is sound but incomplete: a search that did not refute a clause set has
not proved it consistent. A fixture that only exercised the `refuted` path would let the
verifier look more capable than it is.

## 2. Adversarial authorization and sandbox cases (`adversarial/`)

One JSON object per case, loaded by `tests/e2e/phase10_tools.rs`. The shape is:

```json
{
  "id": "missing_grant",
  "description": "one sentence a reader can act on",
  "principal": "system",
  "tool": "process.exec",
  "args": { "cmd": "cargo test" },
  "generate": { "field": "text", "repeat": "x", "count": 2000000 },
  "construct": "oversize",
  "expect": {
    "status": "denied",
    "code": "permission.no_grant",
    "records": ["tool.permission.check"]
  }
}
```

* `generate` is optional: it expands `args[field]` to `repeat` repeated `count` times, so
  an oversize payload can be specified without inlining two megabytes of JSON.
* `construct` is optional and names a case that is *not* an action at all:
  `oversize` (the payload must be refused by the capability guard) and
  `fabricated_observation` (an observation must not be constructible without execution
  evidence). Those two are asserted at the layer that refuses them rather than through
  the CLI, because the refusal happens before an action exists.
* `expect.status` is the `ActionResult::status`; `expect.code` is the machine-readable
  code of the refusal; `expect.records` are the ledger events the refusal must have
  written. Every denial writes a record: "nothing happened" is a fact the ledger has to
  be able to attest to.

Every case here is a *denial* that deterministic code produces — never a judgment a
model makes. That is the phase's second invariant, and these are its fixtures.

## 3. Tool selection (`selection/`)

`selection.jsonl` is the selection split and `eval.jsonl` the evaluation split, mirroring
ToolBench's split between fitting a selector and measuring it. One record per line:

```json
{"query": "read the deployment state file", "tools": ["fs.read", "fs.write", "graph.query"], "expected": "fs.read"}
```

The split exists so a selector's accuracy is measured on queries it was not tuned on. A
selector that scores well on the selection split and badly on the evaluation split has
memorised the first file, and the two files are separate so that is visible.
