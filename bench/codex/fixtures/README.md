# `bench/codex/fixtures` — negative fixtures for `mm-cli codex verify`

Each directory here is a self-contained mini-workspace with **exactly one planted
defect**. They exist so the pass gate can prove that `codex verify` fails *for the
right reason*: a fixture that trips several rules at once would pass a checker that
merely reports "something is wrong".

The gate drives them like this:

```bash
for c in orphan_file duplicate_uri dependency_cycle capability_without_test; do
  if mm-cli codex verify --root bench/codex/fixtures/bad_$c; then
    echo "FAIL: $c not detected"; exit 1
  else echo "ok: $c rejected"; fi
done
```

| Directory | Rule it must trigger | Why it is otherwise clean |
|---|---|---|
| `good/` | *none* — must exit 0 | A two-crate workspace that is well formed in every respect: versioned members, an owning phase and a capability each, one dependency edge, and a `#[test]` in both crates. |
| `bad_orphan_file/` | `orphan_file` | `good/` plus `crates/stray.rs`. That file sits directly under `crates/` and inside no workspace member, so no module owns it. Every module is otherwise complete. |
| `bad_duplicate_uri/` | `duplicate_uri` | Two nexus modules, `modules/alpha/` and `modules/beta/`, whose `plugin.toml` files declare the **same** `[plugin].uri`. Each has its own capability and its own test, so the shared URI is the only thing wrong. |
| `bad_dependency_cycle/` | `dependency_cycle` | `crates/alpha` depends on `crates/beta` and `crates/beta` depends back on `crates/alpha` — declared in both `[dependencies]` tables *and* exercised by a `use` in each `src/lib.rs`, so the cycle is present whichever edge kind the scanner derives. Both crates have tests, so no other rule applies. |
| `bad_capability_without_test/` | `capability_without_test` | One crate that claims capability `mm:Untested` while the tree contains no `#[test]` and no `mod tests`. |

## Conventions these fixtures encode

- **Every module declares an owning phase.** A crate does it in
  `[package.metadata.metamind] owned_by_phase = N`; a nexus plugin does it in
  `[metadata] owned_by_phase = N`. Without it the scan itself fails, because a
  module with no owning phase has no identity to record — so a fixture that
  omitted it would be testing the wrong failure.
- **A module's URI drops the `modules/` prefix.** A module at `modules/alpha`
  declares `https://metamind.dev/code/module/alpha`, matching the path-derived
  binding used everywhere else. `bad_duplicate_uri` relies on the scanner
  comparing the *declared* `[plugin].uri` values, not the directory-derived IRIs.
- **Crate versions are inherited** (`version.workspace = true` from
  `[workspace.package]`) in the crate fixtures, which is the shape Metamind's own
  workspace uses. The plugin fixtures use literal versions because they are not
  workspace members.
- **Only indexed extensions** (`.rs`, `.toml`, `.sql`, `.ttl`, `.json`, `.jsonl`,
  `.md`) are part of the scan, and `target/` and dot-directories are ignored, so
  nothing here is noise.
