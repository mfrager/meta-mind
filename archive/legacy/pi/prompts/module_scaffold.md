# Task: scaffold a module

**Goal.** {{goal}}

**Module IRI.** `{{module_uri}}`

**Skill to follow.** `{{skill}}` — read it first, in full. It lists the exact files to
create, what each one must contain, and the checklist to walk before you report.

**Paths you may change.** Nothing outside this list:

{{allowed_paths}}

## What "done" means

The task is done when all of these are true inside your sandbox worktree:

1. The module directory exists at the path the module IRI is derived from, with
   `Cargo.toml`, `plugin.toml`, `src/lib.rs`, `tests/`, and `manual/module.md`.
2. `plugin.toml` declares the module's `name`, its path-derived `uri`, its `category`,
   its phase, its single `mm:` capability, and one `tbox.functions` entry per handler with
   the handler's Rust path.
3. `src/lib.rs` embeds the manifest, exposes the T-Box constants and the typed API,
   refuses malformed and impossible inputs with typed errors, and calls existing kernel
   arithmetic rather than reimplementing it.
4. `tests/` covers the capability: the manifest is consistent with the crate, each
   handler answers and refuses correctly, and any property the doc comment claims is
   asserted.
5. `manual/module.md` documents the module for an operator: what it is for, its
   functions and their inputs and outputs, its contract, and its ownership.
6. Nothing outside the allowed paths was touched — no root manifest, no ontology, no
   migration, no production file, no test that already existed.

Then write the report the system prompt asks for: files, commands, done and not done,
risks. If the goal cannot be met inside the allowed paths, or if the module needs a
workspace-member entry you are not allowed to add, stop and say so in the report — a
precise refusal is a better result than a change that cannot be built.
