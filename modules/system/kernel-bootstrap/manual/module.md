# kernel-bootstrap

**URI** `https://metamind.dev/code/module/system/kernel-bootstrap`
**Phase** 1 (foundation)
**Capability** `mm:KernelBootstrap`

## What it is for

Every Metamind module is a nexus-style plugin: a directory with a `plugin.toml`
manifest, a stable `uri`, and the T-Box functions it adds to the monad. Phase 1
has no cognition to put in a module, so this is the smallest module that still
exercises the whole contract.

Its job is to state, in one place, what the kernel does at boot — and to refuse
to start when a store is not writable rather than skipping a step.

## T-Box functions

| Function | Handler | Meaning |
|---|---|---|
| `system.boot_plan` | `handlers::boot_plan` | Given whether every store answered, the ordered boot steps or a single refusal |

## Contract

- `system.boot_plan(true)` returns `OpenTabular`, `OpenGraph`, `LoadOntology`,
  `VerifyAudit`, `RecordBoot`, in that order.
- `system.boot_plan(false)` returns exactly one `Refuse` step. A store that is not
  writable stops the boot; it is never skipped and never logged as ready.
- The manifest is embedded with `include_str!`, so a malformed `plugin.toml` is a
  compile error, not a load-time surprise. `manifest_at` exists to prove the file
  on disk still matches the binary.

## Ownership

Phase 1 owns this module. Phase 2 may *read* it (the code graph registers its URI
and T-Box functions) but must not change its contract.
