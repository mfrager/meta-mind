# Legacy Code Archive

This directory holds code that is no longer part of the running MetaMind system.
It exists so the retired implementation remains inspectable and recoverable
without continuing to pay for its build, test, and maintenance surface.

**Status:** skeleton created in the design phase. The actual source trees are
moved here in Phase 2 of
[`design/streamlined_core_four_phase_plan.md`](../../design/streamlined_core_four_phase_plan.md).

## Rules

1. **Not a workspace member.** Nothing under `archive/legacy/` is listed in the
   root `Cargo.toml`, imported by a kept crate, or compiled by CI.
2. **Reference only.** Archived crates keep their original `Cargo.toml` and
   source so they can be read and diffed. They are not expected to build from
   this directory.
3. **Build the history, not the copy.** To run the legacy system, use the freeze
   tag recorded in `MANIFEST.md`:
   ```bash
   git worktree add /tmp/metamind-legacy legacy-freeze-YYYY-MM-DD
   ```
   That worktree is the last commit where the retired code compiled and its
   tests passed.
4. **Tests move with their code.** A retired crate's tests and fixtures are
   archived beside it. Kept tests never reference archived modules.
5. **No live imports.** A source guard fails CI if a kept crate names an
   archived crate outside a comment or this directory.
6. **Frozen content.** After the freeze, only `MANIFEST.md` may change. Fixes
   belong in the kept core, not in the archive.
7. **No secrets or runtime data.** The archive contains source and fixtures
   only. `data/`, `.env`, and generated databases are never copied here.

## Layout

```
archive/legacy/
  README.md
  MANIFEST.md                 # what moved, why, and where it came from
  crates/<crate>/             # retired Rust crates, tests included
  surfaces/mm-serve/<path>/   # retired modules of the kept server crate
  frontend/<path>/            # retired frontend directories and their tests
  config/<file>               # retired configuration files
  tests/<path>/               # retired integration/e2e/bench trees
```

A path mirrors its original location. For example
`crates/mm-data/src/memory/` becomes
`archive/legacy/crates/mm-data/src/memory/`, and
`frontend/src/components/inspector/` becomes
`archive/legacy/frontend/src/components/inspector/`.

## Kept system

The archive boundary is defined by the six crates and the reduced surfaces in
the plan:

| Kept crate | Responsibility |
|---|---|
| `mm-core` | IDs, time, hashing, minimal config and errors |
| `mm-prompt` | Deterministic prompt templates and instances |
| `mm-llm` | Provider-neutral model transport |
| `mm-agentstream` | Append-only JSONL event log, projections, LLM gateway |
| `mm-cme` | Interpret → Process → Verify turn pipeline and its six tools |
| `mm-serve` | HTTP/SSE surface and frontend wiring |

Anything not in that table is legacy.
