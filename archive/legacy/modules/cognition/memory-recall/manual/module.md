# memory-recall

**URI** `https://metamind.dev/code/module/cognition/memory-recall`
**Phase** 5 (persistent memory)
**Capability** `mm:MemoryStore`

## What it is for

Every Metamind module is a nexus-style plugin: a directory with a `plugin.toml`
manifest, a stable `uri`, and the T-Box functions it adds to the monad.

Phase 5 introduces the *long-term memory organ* — the ten memory classes of design
§32 as typed, provenance-bearing traces, with deterministic hybrid retrieval,
episodic→semantic→summary consolidation, deliberate time-decayed forgetting, and
mistake/near-miss retention. The mechanism lives in `crates/mm-memory`, where the
retention curve, the admission gate, and the forget refusals are deterministic.
This module owns the **contract**: which surfaces exist and what a caller may rely
on without opening a store.

It is declarative on purpose. Duplicating the enforcement here would create a
second place for it to drift, and the whole point of the guard in `mm-memory` is
that there is exactly one.

## T-Box functions

| Function | Handler | Meaning |
|---|---|---|
| `memory.remember` | `handlers::remember` | Admit a memory: validated, provenance-bearing, duplicate-flagged |
| `memory.recall` | `handlers::recall` | Lexical ▸ vector ▸ graph ▸ rerank, deterministic |
| `memory.consolidate` | `handlers::consolidate` | Episodes into patterns, patterns into a summary tree |
| `memory.forget` | `handlers::forget` | Retention decay; archives, refuses, never deletes |
| `memory.mistakes` | `handlers::mistakes` | Mistakes and near misses with corrective rules |
| `memory.procedures` | `handlers::procedures` | Skill records and habit promotion |
| `memory.summaries` | `handlers::summaries` | The summary tree and its communities |

## Contract

- `surfaces()` returns one [`Surface`] per T-Box function, in the same order as
  `SURFACE_FUNCTIONS`. A surface names the tables it reads and writes, the
  invariant it must respect, and one sentence a caller can rely on.
- **Nothing is deleted.** `memory.forget` archives and writes only `memories`. A
  protected record, a developmental record, and a record serving an open commitment
  are refused, and every refusal is logged with what protected it.
- **Nothing is stored without provenance.** `memory.remember` refuses a nil
  provenance node, and `/memory` SHACL requires exactly one `mm:source` on every
  memory.
- **Recall is deterministic.** Fixed channel weights
  (`0.35·lexical + 0.30·vector + 0.20·graph + 0.10·recency + 0.05·importance`,
  times `0.5 + 0.5·confidence`), a fixed ULID tie-break, and no clock other than
  the instant the caller passes in.
- **Consolidation keeps its lineage.** Sources are archived only after the target
  commits, and every source still resolves through `mm:consolidatedFrom` /
  `prov:wasDerivedFrom`.
- **A mistake always names a rule.** One is derived from the failure mode when the
  caller supplies none, so a lesson can never be stored without something to do
  about it.
- The manifest is embedded with `include_str!`, so a malformed `plugin.toml` is a
  compile error, not a load-time surprise. `manifest_at` exists to prove the file
  on disk still matches the binary.

## Ownership

Phase 5 owns this module and `crates/mm-memory`. Phase 6 (epistemic ledger)
references `entity_edges`, which Phase 5 owns (`INDEX.md` D4) and Phase 6 must not
duplicate. Phase 11 compiles mistakes into regression tests through the `mistakes`
table's `id` column (`INDEX.md` D5). Later phases may *read* this module — the code
graph registers its URI and T-Box functions — but must not change its contract.
