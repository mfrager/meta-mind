# Skill: `mm-design-revise`

Revise a Metamind design document and author the module the revision describes — with
every write going through a change set, never straight into the production tree.

A design revision and a module are two halves of one change. The revision is the *claim*
about how the system should work; the module is the *thing that makes the claim true*;
and the change set is what the promotion gate judges. Producing one without the other is
the failure this skill exists to prevent: a document describing a capability nobody
built, or a module nobody specified.

## When to use it

When a goal names a design document to revise and a capability to add — usually a
capability gap (`bench/gaps/gap_*.json`) or a novel goal
(`bench/qualification/goal_novel.json`) whose `target_uri`, `target_path`, `capability`,
`function` and `design_doc` fields say exactly what is wanted. The goal's identity is the
work's identity: create the module *at* that path with *that* uri, and revise *that*
document, or say why the goal's target is wrong rather than inventing a second home for
the capability.

## The two rules that shape everything here

1. **Nothing is written into the production tree directly.** Every file below is an edit
   *inside the sandbox worktree* or a patch in a change set's payload. The change set is
   built first, the sandbox materialises it, the tests and the frozen benchmark judge it,
   and only a promotion writes anything anywhere durable. A revision that writes the
   document first and proposes the change afterwards has already done the thing the gate
   exists to prevent.
2. **A revision carries its reason.** The change set's `reason` is the revision's own
   summary and its `hypothesis` is what the revision asserts will change. The gate's rule 5
   refuses a change set whose rollback plan is empty, so every revision names how it is
   undone.

## The files, and what each one must contain

Work inside the sandbox worktree; every path below is relative to its root. Only the
paths the task contract's `allowed_paths` names are yours to touch.

### 1. `design/<...>/<doc>.md` — the revision

The document the goal names. Revise it rather than replacing it: keep the existing
structure, add the new capability's section, and say in that section

- what the capability is for, in one paragraph a reader can act on;
- the T-Box function's name, its input shape and its output shape;
- what it **refuses** — the malformed payload and the well-shaped impossible value — and
  why refusing is the right answer rather than a default;
- what it deliberately does **not** do, and which existing crate owns that instead.

A revision whose numbers are asserted somewhere (`bench/...`) should name the fixture, so
the module's test can be graded against committed data rather than against a literal
somebody typed into the document.

### 2. `modules/<category>/<name>/plugin.toml` — the module's identity

```toml
[plugin]
name = "<name>"
uri = "https://metamind.dev/code/module/<category>/<name>"
version = "0.1.0"

[metadata]
category = "<category>"
owned_by_phase = <the phase that owns this work>
capability = "mm:<CapabilityName>"

[tbox.functions]
"<namespace>.<verb>" = { source = "handlers::<fn>" }

[monad.operations]
name = "cognition"
arity = 1

[build]
rust_edition = "2021"
```

- **`uri` is path-derived**: it is `mm_core::iri::module("<category>/<name>")` and nothing
  else. The goal's `target_uri` and the path-derived value must be the same string; if
  they are not, the goal's target is what is wrong and you say so.
- **`name` is the directory name.**
- **Every `tbox.functions` entry names a real handler** by its Rust path
  (`handlers::<fn>`). The code graph resolves that path.
- **One capability per module**, `mm:<Something>`, distinct from every other module's.

### 3. `modules/<category>/<name>/src/lib.rs` — the module's body

- `#![forbid(unsafe_code)]` at the top.
- A module doc comment saying *why* the module exists, what it owns, what it refuses, and
  what it deliberately does not do because that lives elsewhere.
- Public constants for the embedded manifest (`include_str!(\"../plugin.toml\")`), the T-Box
  function names, `SURFACE_FUNCTIONS`, `CAPABILITY`, `MODULE_PATH`, and the tables it reads
  and writes (empty arrays when it touches none — an empty list is a contract too).
- A typed error enum with two kinds: a *payload-shape* refusal and a *refusal of
  something understood*, the second carrying a stable code. Never `unwrap` a caller's
  document.
- A `handlers` module with one function per T-Box entry, taking and returning
  `serde_json::Value`, returning `Result`.
- Unit tests in the same file.

**Do not reimplement kernel arithmetic.** If the capability is defined in an existing
crate, call it and say in the doc comment why the module is a thin adapter. A second
implementation of a documented formula is a second definition of it.

### 4. `modules/<category>/<name>/tests/behaviour.rs` — the coverage

The test the manifest's capability points at. At minimum it asserts that the embedded
manifest declares this module, its phase, its capability and exactly the functions the
crate exposes, that the declared `uri` equals the path-derived one, that each handler
answers a realistic input with the values it promises, and that each handler refuses both
a malformed payload and a well-shaped impossible value, naming the function and the code
and never panicking. A capability with no test is refused by `codex verify`; the test is
not optional paperwork.

### 5. `modules/<category>/<name>/manual/module.md` — the manual

Header (`URI`, `Phase`, `Capability`), what it is for and which goal it answers, a table of
the T-Box functions with their inputs and outputs, a `Contract` section with the refusals
and the invariants, and an `Ownership` section naming the phase that owns the module.

## Before you report the task done

Walk this list and put the answer for each in your report:

1. The design document was revised *and* the module exists, and nothing outside
   `allowed_paths` was touched.
2. `plugin.toml`'s `uri` is the path-derived one for the directory, and it equals the
   goal's `target_uri`.
3. Every declared T-Box function has a handler whose path the manifest names.
4. The capability has at least one test in `tests/`.
5. The module reuses existing arithmetic instead of duplicating it, and the doc comment
   says where the real implementation lives.
6. Nothing was written into the production tree directly: the document and the module are
   a change set's payload, and the promotion is what makes them real.
7. No new dependency was introduced, and nothing in the module touches the network or
   writes outside its own directory.

## Worked example: `cognition/goal-attainment`

Phase 12's qualification run. The goal named
`https://metamind.dev/code/module/cognition/goal-attainment`, so the module was created at
`modules/cognition/goal-attainment` with that uri, category `cognition`,
`owned_by_phase = 12`, `capability = "mm:GoalAttainment"`, and one T-Box function:

- `cognition.goal_attainment_progress` → `handlers::goal_attainment_progress`

It takes a goal with the evidence recorded for it and returns the progress fraction, the
counts behind it and the evidence it stood on. It refuses a malformed payload and a
well-shaped impossible value (evidence that is not evidence, a target of zero), and it
returns a number and its inputs rather than a narrative, because the self-model reads it.
The revision of `design/qualification/goal_attainment.md` is the change set's other half;
`tests/behaviour.rs` is the coverage `codex verify` checks.
