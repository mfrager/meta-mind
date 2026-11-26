# frame-activate

**URI** `https://metamind.dev/code/module/cognition/frame-activate`
**Phase** 7 (cognitive library, conceptual frames, policy genome)
**Capability** `mm:FrameActivation`

## What it is for

Every Metamind module is a nexus-style plugin: a directory with a `plugin.toml`
manifest, a stable `uri`, and the T-Box functions it adds to the monad.

Phase 7 introduces the *cognitive library*: a versioned repertoire of reusable
ways of thinking (doctrines, principles, heuristics, techniques, patterns, cases,
anti-patterns, skills, policies) plus **conceptual frames** and the policy genome.
A frame is a reusable structure — roles, slots, typical actions, failure modes —
and a frame instance is that structure bound to one episode.

The mechanism lives in `crates/mm-library`, where the slot merge and the
missing-slot report are deterministic. This module owns the **contract**: which
surfaces exist and what a caller may rely on without opening a store. It is
declarative on purpose; duplicating the enforcements here would create a second
place for them to drift.

## T-Box functions

| Function | Handler | Meaning |
|---|---|---|
| `cognition.frame_compose` | `handlers::frame_compose` | Bind base frames to an episode: `Frame_child = Frame_parent ⊕ Δ` |
| `cognition.frame_missing` | `handlers::frame_missing` | The slots an instance left unfilled |
| `cognition.frame_switch` | `handlers::frame_switch` | Record a change of active frame, with its reason |

## Contract

- `surfaces()` returns one [`Surface`] per T-Box function, in the same order as
  `SURFACE_FUNCTIONS`. A surface names the tables it reads and writes, the
  invariant it must respect, and one sentence a caller can rely on.
- **No undeclared parent.** `cognition.frame_compose` refuses a base frame that is
  not already in the library, and the instance it writes records every parent it
  was built from. Composition cannot invent a frame.
- **A gap is reported, never fabricated.** `cognition.frame_missing` writes
  nothing: a slot the bases do not fill is returned as missing rather than guessed.
- **A switch is explicable.** `cognition.frame_switch` records the frame it left,
  the frame it entered, and the reason, so an activation change can always be
  reconstructed from the record.
- The manifest is embedded with `include_str!`, so a malformed `plugin.toml` is a
  compile error, not a load-time surprise. `manifest_at` exists to prove the file
  on disk still matches the binary.

## Ownership

Phase 7 owns this module, `modules/cognition/library-query`, and
`crates/mm-library`. Phase 8 (metacognitive controller) *reads* these surfaces to
select the frames a program runs under; it does not change the contract.
