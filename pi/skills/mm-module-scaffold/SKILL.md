# Skill: `mm-module-scaffold`

Scaffold a new Metamind cognition module so that it registers, validates and is
considered *covered* rather than merely present.

A module is a nexus-style plugin: a directory under `modules/<category>/<name>/` with a
`plugin.toml` manifest, a stable path-derived `uri`, a crate that embeds its manifest, and
tests that cover the capability it declares. `mm-cli codex scan` reads these; a module
that invents its own layout is not discovered, and a capability with no test is refused by
`codex verify`.

## When to use it

When the task is to add a module for a capability the system does not have — usually
because a gap was recorded (`bench/gaps/gap_*.json`) naming a `target_uri` and a
`target_path`. The gap's target is the module's identity: create the module *at* that
path with *that* uri, or say why the gap's target is wrong rather than creating a second
home for the capability.

## The files, and what each one must contain

Work inside the sandbox worktree; every path below is relative to its root.

### 1. `modules/<category>/<name>/Cargo.toml`

A workspace member. Mirror an existing module's manifest exactly: the package name is the
directory name, and the metadata block below is what the scanner reads.

```toml
[package]
name = "<name>"
description = "Metamind cognition module: <the contract in one line>"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
# Only what the module's code actually uses, from `[workspace.dependencies]` in the
# root manifest. Do not add a new dependency: the root manifest is not in your
# allowed paths.
```

The root `Cargo.toml` must list the new directory under `[workspace] members`. If it is
not already listed and the root manifest is not in your allowlist, stop and report it —
a module that is not a workspace member does not build, and that is the coordinator's
edit to make.

### 2. `modules/<category>/<name>/plugin.toml`

```toml
[plugin]
name = "<name>"
uri = "https://metamind.dev/code/module/<category>/<name>"
version = "0.1.0"

[metadata]
category = "<category>"
owned_by_phase = <phase>
capability = "mm:<CapabilityName>"

[tbox.functions]
"<namespace>.<verb>" = { source = "handlers::<fn>" }

[monad.operations]
name = "cognition"
arity = 1

[build]
rust_edition = "2021"
```

Rules the scanner and the verifier enforce:

- **`uri` is path-derived.** It is
  `mm_core::iri::module("<category>/<name>")` and nothing else. A test asserts the
  embedded manifest contains it, because a drifting uri is a second identity for one
  module.
- **`name` is the directory name.**
- **Every `tbox.functions` entry names a real handler** by its Rust path
  (`handlers::<fn>`). The code graph resolves that path; a name that does not exist is a
  `codex verify` failure.
- **One capability per module**, named `mm:<Something>`, and it must differ from every
  other module's.

### 3. `modules/<category>/<name>/src/lib.rs`

- `#![forbid(unsafe_code)]` at the top.
- A module doc comment that says *why* the module exists, what it owns, what it refuses,
  and what it deliberately does **not** do because that lives elsewhere. Every decision
  that is not visible in a signature belongs here.
- Public constants for the manifest text (`include_str!("../plugin.toml")`), the T-Box
  function names, `SURFACE_FUNCTIONS`, `CAPABILITY`, the module path, and the tables it
  reads and writes (empty arrays when it touches none — an empty list is a contract too).
- A typed error enum with two kinds: a *payload-shape* refusal and a *refusal of
  something understood*, the second carrying a stable code. A module boundary that
  unwraps turns a caller's bad document into a dead process.
- A `handlers` module with one function per T-Box entry, taking and returning
  `serde_json::Value` (or typed values) and returning `Result`.
- Unit tests in the same file, and an integration test in `tests/`.

**Do not reimplement kernel arithmetic.** If the capability is defined in an existing
crate (`mm-decision`, `mm-metacog`, `mm-library`, …), call it, and say in the doc comment
why the module is a thin adapter. A second implementation of a documented formula is a
second definition of it.

### 4. `modules/<category>/<name>/tests/<name>_behavior.rs` (or `tests/behavior.rs`)

The tests that make the capability *covered*. At minimum:

- the embedded manifest declares this module, its phase, its capability, and exactly the
  functions the crate exposes, with the declared `uri` equal to the path-derived one;
- each handler answers a realistic input with the numbers or structure it promises;
- each handler refuses a malformed payload **and** a well-shaped impossible value,
  naming the function and the code, and never panics;
- any property the doc comment claims about the capability, asserted against a committed
  fixture rather than a hand-written literal where one exists.

### 5. `modules/<category>/<name>/manual/module.md`

The manual an operator reads. Header (`URI`, `Phase`, `Capability`), what it is for and
which gap it closes, a table of the T-Box functions with their inputs and outputs, a
`Contract` section with the refusals and the invariants, and an `Ownership` section
naming the phase that owns the module and what later phases may read.

## Before you report the task done

Walk this list and put the answer for each in your report:

1. Every file above exists, and nothing outside the allowlist was touched.
2. `plugin.toml`'s `uri` is the path-derived one for the directory.
3. The root manifest lists the module as a workspace member (or you reported that you
   could not add it).
4. No new dependency was introduced.
5. Every declared T-Box function has a handler whose path the manifest names.
6. The capability has at least one test in `tests/`.
7. The module reuses existing arithmetic instead of duplicating it, and the doc comment
   says where the real implementation lives.
8. Nothing in the module writes outside its own directory, touches production, or
   depends on the network.

## Worked example: `cognition/calibration`

Phase 11's first generated module. The gap said *no module can summarize a drifting
calibration bin*, and targeted `https://metamind.dev/code/module/cognition/calibration`,
so the module was created at `modules/cognition/calibration` with that uri, category
`cognition`, `owned_by_phase = 11`, `capability = "mm:CalibrationSummary"`, and two T-Box
functions:

- `cognition.calibration_summary` → `handlers::calibration_summary`
- `cognition.calibration_drift` → `handlers::calibration_drift`

`src/lib.rs` calls `mm_decision::calibration` for the Brier score, the log loss, the ECE
and the temperature fit rather than reimplementing them, exposes the typed API
(`summarize`, `drifting_classes`, `score_at`, `fitted_temperatures`, `reliability_bins`,
`parse_jsonl`), declares `READS` and `WRITES` as empty because it is pure arithmetic, and
carries a typed refusal for an out-of-range probability. `tests/behavior.rs` grades it
against `bench/calibration/predictions.jsonl` and the independently computed
`bench/calibration/reference.json`. The `drifting` field is the gap's answer, which is
why it is a sorted list of classes and not a boolean.
