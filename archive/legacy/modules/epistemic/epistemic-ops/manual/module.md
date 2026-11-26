# epistemic-ops

**URI** `https://metamind.dev/code/module/epistemic/epistemic-ops`
**Phase** 6 (epistemic discipline)
**Capability** `mm:EpistemicOps`

## What it is for

Every Metamind module is a nexus-style plugin: a directory with a `plugin.toml`
manifest, a stable `uri`, and the T-Box functions it adds to the monad.

Phase 6 introduces *epistemic discipline* — the layer that keeps **what the being
thinks** apart from **what the world contains**. The mechanism lives in
`crates/mm-epistemic`: a monotone-guarded status lattice, a justification graph
whose retraction is dependency-directed, contradictions as explicit objects, and a
validation barrier that is the only writer permitted to touch `/world`. This module
owns the **contract**: which operations exist and what a caller may rely on without
opening a store.

It is declarative on purpose. Duplicating the guard here would create a second
place for it to drift, and the whole point of `mm-epistemic::guard` is that there is
exactly one place a promotion may be refused.

## T-Box functions

| Function | Handler | Meaning |
|---|---|---|
| `epistemic.claim` | `handlers::claim` | Ingest a typed claim at the status its kind starts at |
| `epistemic.assume` | `handlers::assume` | Hold a proposition without support, with its cost of being wrong |
| `epistemic.predict` | `handlers::predict` | Expect a proposition with a caller-supplied probability |

## Contract

- `functions()` and `surfaces()` name the same three operations in the same order.
  Each surface carries the arguments `mm-epistemic` validates and one sentence a
  caller can rely on, so a handler wired to this surface cannot accept something the
  engine would refuse.
- **No silent promotion.** `epistemic.claim` records a claim at the status its kind
  starts at; a rise from there is a separate, guarded transition, and the forbidden
  ones (`Assumed→Observed`, `Inferred→Observed`, `Predicted→Observed`,
  `Simulated→Verified`, and any movement out of `Fictional`/`Unknown`) are refused
  regardless of evidence.
- **Only `OBSERVED`/`VERIFIED` reach `/world`.** The validation barrier is the sole
  writer of the world graph, and it admits a claim only when its status is one of
  those two *and* it carries evidence.
- **An assumption names its risk.** `epistemic.assume` requires
  `consequence_if_false` and `verification_cost`, because an assumption is the one
  kind of claim that may be held with no support, which is exactly why its cost of
  being wrong must be stated.
- **A probability is the caller's.** `epistemic.predict` takes the probability as an
  argument; nothing in this layer computes one, so a prediction ledger is
  reproducible from the caller's input alone.

## Ownership

Phase 6 owns this module and `crates/mm-epistemic`. Phase 7's
`LibraryManager::record_fitness` consumes `mm_epistemic::Outcome` and must not
define a parallel outcome type (`INDEX.md` D2). Phase 6 references Phase 5's
`entity_edges` and adds only its own dependency edges; it must not duplicate that
table (`INDEX.md` D4). Later phases may *read* this module — the code graph
registers its URI and T-Box functions — but must not change its contract.
