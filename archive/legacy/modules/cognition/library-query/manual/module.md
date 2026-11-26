# library-query

**URI** `https://metamind.dev/code/module/cognition/library-query`
**Phase** 7 (cognitive library)
**Capability** `mm:TechniqueSelection`

## What it is for

Every Metamind module is a nexus-style plugin: a directory with a `plugin.toml`
manifest, a stable `uri`, and the T-Box functions it adds to the monad.

Phase 7 introduces the *cognitive library* — a persistent, versioned repertoire of
ways of thinking (doctrines, principles, heuristics, techniques, patterns, cases,
anti-patterns, skills, policies, frames, evaluations, insights, workflows) stored in
the `/library` graph and indexed in SQLite. The mechanism lives in
`crates/mm-library`, where the applicability arithmetic, the structure-mapped case
retrieval, and the verification gate on skills are deterministic. This module owns
the **contract**: which surfaces exist and what a caller may rely on without opening
a store.

It is declarative on purpose. Duplicating the ranking or the verification gate here
would create a second place for it to drift, and the whole point of the gate in
`mm-library` is that there is exactly one.

## T-Box functions

| Function | Handler | Meaning |
|---|---|---|
| `cognition.library_applicable` | `handlers::library_applicable` | Rank the techniques applicable to a state, deterministically |
| `cognition.skill_retrieve` | `handlers::skill_retrieve` | Retrieve skills; only a `Verified` one is returned |
| `cognition.case_retrieve` | `handlers::case_retrieve` | Retrieve precedents, explained by structural correspondences |

## Contract

- `surfaces()` returns one [`Surface`] per T-Box function, in the same order as
  `SURFACE_FUNCTIONS`. A surface names the tables it reads and writes, the
  invariant it must respect, and one sentence a caller can rely on.
- **Selection is ranked, not defaulted.** `applicability()` is pure arithmetic over
  the `ApplicabilityState` the caller passes in — no clock, no model output, no
  hidden default — and the ranking it returns is recorded in `applicability_runs`.
- **An unverified skill is not retrievable.** A `Draft` or `Failing` skill is never
  returned by `cognition.skill_retrieve`, whatever its score. Promotion to
  `Verified` happens only through `skill verify`.
- **An analogy is structural.** `structural_similarity` maps *relations* between two
  case graphs rather than matching surface text, and the correspondences are stored
  in `case_map` with their scores.
- The manifest is embedded with `include_str!`, so a malformed `plugin.toml` is a
  compile error, not a load-time surprise. `manifest_at` exists to prove the file on
  disk still matches the binary.

## Ownership

Phase 7 owns this module and `crates/mm-library`. Phase 8 (metacognitive controller)
reads this module to activate frames and select techniques by applicability; Phase 9
(the decision firewall) reads cases as precedent. Later phases may *read* this
module — the code graph registers its URI and T-Box functions — but must not change
its contract.
