# Copied-in reference code — origin `rust_symbolic`

Per the parent plan §7, external code is **reference only**: the source we depend on is *copied into this
repository* and integrated behind `mm-*` interfaces. There are no git submodules and no path dependencies
pointing outside this repository.

## Origin

| Field | Value |
|---|---|
| Source workspace | `~/Scripts/rust_symbolic` |
| Source repository | `git@github.com:mfrager/blanc-scripts.git` |
| Revision (`git rev-parse HEAD`) | `c03aee5fb3a9947da1c7f21e491aa52f43d6f09d` |
| Copied at | Phase 1 (`design/planning/full/phase_01_foundation_build_plan.md`) |
| License | **unspecified** — the origin workspace (and these crates) contain no `LICENSE` or `COPYING` file, so no license text could be copied. This is local, unpublished source. Resolve licensing before any redistribution. |

## What was copied

| Vendored crate | Origin path | Copied-in use |
|---|---|---|
| `rdf-codec` | `crates/rdf-codec` | Canonical, order-independent N-Triples serialization and content hashing (`io::to_canonical_ntriples`, `io::canonical_hash`) plus Turtle parse/serialize and the cRDF/SEM lanes |
| `rdf-shacl` | `crates/rdf-shacl` | Native SHACL validator (`validate::validate_turtle`, `ShaclReport`) and the OWL→SHACL generator/ontology checker |
| `math-core` | `crates/math-core` | Transitive dependency of `rdf-codec`/`rdf-shacl` (re-exports `oxigraph`) |
| `math-types` | `crates/math-types` | Transitive dependency of `math-core` |
| `decision-ir` | `crates/decision-ir` | Risk measures behind `mm-decision::risk`: `decision_ir::Outcome`/`Action` are mapped into `RiskProfile`, and `DecisionEngine::risk` supplies the variance, VaR and CVaR a profile reports (Phase 9) |
| `logic-ir` | `crates/logic-ir` | Transitive dependency of `decision-ir` |
| `logic-types` | `crates/logic-types` | Transitive dependency of `decision-ir` |

## Modifications made to the copied source

Documented so the copy is auditable against the origin revision.

1. **Manifests rewritten.** `version.workspace` / `edition.workspace` were replaced with concrete values
   (`0.1.0`, `2021`), the `ulid = { workspace = true }` and `oxigraph = { workspace = true }` inherited
   dependencies were expanded to concrete versions, and a vendored lint policy was added
   (`unsafe_code = "forbid"`; `clippy::all` allowed because vendored code is not held to Metamind's
   zero-warning bar).
2. **`rdf-codec/src/bin/` removed** — the origin's example generators depend on crates outside the copied
   subtree.
3. **`rdf-codec/src/sem/*_tests.rs` and `tests.rs` removed**, and their now-dangling `mod` declarations
   removed from `rdf-codec/src/sem/mod.rs`, with a comment pointing at the reason. These suites depend on
   fixtures from the wider origin workspace.
4. **`rdf-shacl/tests/pipeline.rs` removed** — its dev-dependency `math-ontology` is outside the copied
   subtree.
5. **No source logic was modified.** Only imports-free test/bin entry points were dropped; every remaining
   `src` file is byte-identical to the origin revision.
6. **`decision-ir`, `logic-ir` and `logic-types` copied in Phase 9**, by the same rules: `src/` and the
   manifest only, no `tests/`, and the `[dev-dependencies]` sections removed from the manifests because
   their dev-dependency closure (`logic-fragment`, `logic-semantics`, `logic-parser`) is outside the
   integrated subset. Their two path dependencies inside the subset (`rdf-codec`, `math-types`) resolve to
   the copies already present. The origin revision is the one recorded above, so the copied crates and the
   already-vendored ones are the same revision.
7. **The wider solver set was not copied.** The plan's §4.12 also lists `solver-ir`, `backend-registry`,
   `logic-planner`, `ensemble-ir` and `causal-ir`; those have a 28-crate path-dependency closure and
   `backend-smt-z3` links Z3, so the SAT/SMT half is a separate vendoring job. `mm-decision::risk` therefore
   exposes `decision-ir` only, which is the crate this phase's gate needs.

## How Metamind verifies it

The vendored surface Metamind actually relies on is covered by our own tests, not by the origin's suite:

- `mm-store-graph` canonical-RDF tests: 10k-triple encode → canonical serialize → re-parse ⇒ identical
  `canonical_hash` (the canonicalization property the replay/diffing invariant depends on).
- `mm-store-graph` SHACL conformance tests: a clean `being` graph yields zero violations and a broken one
  yields at least one.
