# Phase 2 — Modular Code System and Code-Metadata Registry (Extended Build Plan)

> Extended from: `design/planning/phase_02_code_metadata_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 2 · Codename Metamind (`mm`)

This is the research-grounded, streamlined revision. It keeps the same scope and gate but locks in the
specific mechanisms — **stable symbol identifiers (SCIP style)**, **content-addressed file identifiers
(SWHID style)**, and a **Kythe/Joern-style code graph** — that make the codebase a queryable artifact and
give Phase 11 a precise map to modify.

---

## 0. Research foundation (code-available)

Every idea below is borrowed from a project whose source is available. Only ideas with code are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| Stable, human-readable symbol IDs | SCIP — https://github.com/scip-code/scip · https://sourcegraph.com/blog/announcing-scip | One stable identifier string per definition/reference, stable across files and time | Every symbol gets `https://metamind.dev/code/symbol/{module}#{scip-style-descriptor}` in `mmc:Symbol` |
| Content-addressed software artifacts | SWHID — https://github.com/ocamlpro/swhid · https://swhid.org/faq/ | Intrinsic, registry-free identifiers derived from bytes (git object hashes) | File nodes carry `mmc:contentHash` (sha256) and optional `mmc:swhid`; the file IRI embeds the hash |
| Queryable graph over code (AST+CFG+call graph) | Joern CPG — https://github.com/joernio/joern | Model code as a graph and answer structural questions with queries | The `/code` graph is queried with SPARQL; `codex meta` are graph queries, not bespoke code |
| Code-index schema (nodes/edges) | Kythe — https://github.com/kythe/kythe | Nodes (`file`, `definition`, `reference`) and edges (`defines`, `references`, `childOf`) | `mmc:` adopts exactly these classes/edges for source graphs |
| Incremental concrete syntax trees | tree-sitter — https://github.com/tree-sitter/tree-sitter | Incremental parsing, error-tolerant trees, stable node ranges | Scanner parses changed files incrementally; byte ranges recorded for every symbol |
| Real Rust semantic model (HIR) | rust-analyzer (`ra_ap_*`) — https://github.com/rust-lang/rust-analyzer | Symbol/interface/reference extraction from semantics, not regex | `mm-codex` extracts interfaces via `ra_ap_*`; parse/analysis failure is a hard error |
| Structural code search for drift | ast-grep — https://github.com/ast-grep/ast-grep · `syn` — https://github.com/dtolnay/syn | Structural (AST) pattern matching | `codex diff`/drift checks use AST patterns; declaration parsing via `syn` |

---

## 1. Objective and scope

**Objective.** Make the codebase a first-class, queryable RDF artifact. Every module, file, symbol,
symbol reference, capability, and test gets a **stable RDF identifier** in a dedicated named graph
(`https://metamind.dev/graph/code`), so the being can perform meta-analysis of its own code (design §62,
§85, §105) and so self-modification (Phase 11) has a machine-readable map: what exists, what it depends
on, what it exposes, what it tests, which phase owns it, and where every copied-in file came from.

**In scope.** `ontology/code.ttl` (`mmc:`) + SHACL shapes; `crates/mm-codex` (deterministic scanner,
symbol extractor, emitter, verifier, meta engine); generated per-module `metadata.ttl`,
`modules/registry.json`, committed `codex.lock`; `mm-cli codex {scan,verify,graph,diff,meta,lock}`; the
`module_*`/`symbol_*` SQLite tables; `mmc:copiedFrom` provenance for copied-in files (parent plan §7).

**Out of scope.** Runtime cognition, any self-modification, and Pi invocation (Phases 11–12). Phase 2
*describes and verifies* the code; it does not change it. It must be green before Phase 11.

**Definition of done:** the §8 pass gate runs green as plain commands.

---

## 2. Architecture and identifiers

The scanner reads the workspace, extracts modules/files/symbols, emits canonical Turtle into the `/code`
graph, mirrors an index into SQLite for fast queries and churn, and emits `registry.json` + `codex.lock`.
The `/code` graph is separate from every runtime graph, so meta-analysis never touches runtime facts.

```
            workspace tree (crates/**, modules/**, vendor/**)
                       │  walk (sorted, incremental)
        ┌──────────────┼───────────────────────────┐
        ▼              ▼                            ▼
  Cargo.toml /     ra_ap_* + syn              tree-sitter
  plugin.toml      (symbols, interfaces,      (incremental parse,
  (manifests)       references)                byte ranges)
        └──────────────┼───────────────────────────┘
                       ▼
                 mm-codex::model  ──►  mm-codex::emit (rdf-codec, canonical)
                       │                          │
        ┌──────────────┴──────────────┐           ▼
        ▼                             ▼      /code named graph  (Oxigraph)
  mm-store-sqlite index          registry.json · codex.lock · metadata.ttl
  (module_*, symbol_*, deps)     (committed, deterministic)
```

**Stable identifier scheme** (built by one funnel in `mm-core::codex`; see §4):

| Entity | IRI | Stability |
|---|---|---|
| Module | `https://metamind.dev/code/module/{rel-path}` | path-stable |
| Module version | `https://metamind.dev/code/module/{rel-path}@{semver}` | immutable per release |
| Source file | `https://metamind.dev/code/file/{rel-path}@{sha256}` | content-addressed (SWHID spirit) |
| Symbol | `https://metamind.dev/code/symbol/{rel-path}#{scip-descriptor}` | SCIP-style, survives edits |
| Capability | `https://metamind.dev/code/capability/{name}` | name-stable |
| Phase | `https://metamind.dev/phase/{n}` | fixed |

---

## 3. Deliverables and workspace layout

```
crates/mm-codex/
├── Cargo.toml                         # deps: mm-core, mm-log, mm-store-*, ra_ap_*, syn, tree-sitter, sha2, walkdir
├── src/lib.rs                         # Codex facade + re-exports
├── src/model.rs                       # ModuleRecord, FileRecord, SymbolRecord, CapabilityRecord, DependencyEdge, CodexReport
├── src/scan.rs                        # deterministic incremental workspace walk
├── src/manifest.rs                    # plugin.toml + Cargo.toml parsing/validation
├── src/rust_analyze.rs                # ra_ap_* + syn symbol/interface/reference extraction
├── src/parse.rs                       # tree-sitter incremental trees + byte ranges
├── src/symbol.rs                      # SCIP-style descriptor builder; symbol IRI
├── src/hash.rs                        # sha256 file/dir; canonical module content_hash; optional SWHID
├── src/emit.rs                        # records -> canonical Turtle via rdf-codec; metadata.ttl
├── src/registry.rs                    # registry.json + SQLite index upsert
├── src/verify.rs                      # VerifyFailure rules; dependency-cycle detection
├── src/meta.rs                        # MetaQuery over /code (SPARQL) + SQLite (churn)
├── src/drift.rs                       # ast-grep/syn structural drift checks
├── src/graph_view.rs                  # derived Mermaid view (kg-diagram conventions) + manifest
├── src/lock.rs                        # codex.lock read/write/diff
└── tests/{scan_fixtures.rs,idempotence.rs,verify_rules.rs,meta_queries.rs,symbol_stability.rs}

ontology/code.ttl                        # mmc: vocabulary (T-Box)
ontology/shapes/mmc-shapes.ttl           # SHACL shapes for the code graph
crates/mm-cli/src/cmd/codex.rs           # codex {scan,verify,graph,diff,meta,lock}
crates/mm-store-sqlite/migrations/0002_codex.sql
modules/registry.json                    # GENERATED, committed
codex.lock                               # GENERATED, committed
modules/<category>/<name>/metadata.ttl   # GENERATED per module (.gitignore)
vendor/{nexus,rust_extract,rust_symbolic}/COPYING.md
bench/codex/{fixtures,golden,ast-patterns}/
```

`.gitignore` adds `modules/**/metadata.ttl`; `modules/registry.json` and `codex.lock` are committed.

---

## 4. Detailed specifications

### 4.1 `mmc:` vocabulary (`ontology/code.ttl`)

`mmc:` = `https://metamind.dev/code#`. Instances live in the `/code` graph. Node/edge names follow Kythe;
the symbol model follows SCIP.

```turtle
@prefix mmc: <https://metamind.dev/code#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

# nodes (Kythe-style)
mmc:Module        a rdfs:Class .
mmc:RustCrate     a rdfs:Class ; rdfs:subClassOf mmc:Module .
mmc:NexusPlugin   a rdfs:Class ; rdfs:subClassOf mmc:Module .
mmc:SourceFile    a rdfs:Class .          # file node
mmc:Symbol        a rdfs:Class .          # definition node
mmc:Reference     a rdfs:Class .          # reference node
mmc:Interface     a rdfs:Class ; rdfs:subClassOf mmc:Symbol .   # T-Box fn / trait / exported type
mmc:TestSet       a rdfs:Class .
mmc:Capability    a rdfs:Class .
mmc:ModuleVersion a rdfs:Class .
mmc:ChangeSet     a rdfs:Class .
mmc:PiSession     a rdfs:Class .
mmc:Edit          a rdfs:Class .
mmc:ToolCall      a rdfs:Class .

# edges / properties
# path uri version contentHash swhid swhidKind
# dependsOn kind exports requires implementsCapability
# hasTest testCovers coveredBy
# ownedByPhase promotedFrom registryIndex copiedFrom copiedRevision copiedLicense
# defines references childOf                                        (Kythe edges)
# language byteStart byteEnd descriptor                            (symbol fields)
```

### 4.2 URI funnel (`crates/mm-core/src/codex.rs`)

```rust
pub fn module_iri(rel_path: &str) -> NamedNode;                     // .../module/{path}
pub fn module_version_iri(rel_path: &str, semver: &str) -> NamedNode;
pub fn file_iri(rel_path: &str, sha256: &str) -> NamedNode;         // .../file/{path}@{sha256}
pub fn symbol_iri(rel_path: &str, descriptor: &str) -> NamedNode;   // .../symbol/{path}#{descriptor}
pub fn capability_iri(name: &str) -> NamedNode;                     // .../capability/{name}
pub fn phase_iri(n: u8) -> NamedNode;
```

**SCIP-style descriptors.** `mm-codex::symbol::descriptor` renders a stable descriptor from the
definition's canonical path, e.g. `crates/mm-codex#Codex#scan().` and `crates/mm-codex#VerifyFailure#`.
The descriptor is derived from the semantic path (module scope + item path + kind suffix), not from a
byte offset, so it survives reformatting and line moves. `mmc:byteStart`/`byteEnd` are recorded separately
for navigation only. A symbol's IRI is stable across edits; a *moved* symbol keeps its descriptor unless
it changes scope.

**SWHID adoption.** We adopt the SWHID *idea* (intrinsic, content-derived identity) but keep Metamind URIs
as the canonical identifiers. Where a git object hash is available, the scanner records
`mmc:swhid "swh:1:cnt:<sha1>"` for files/directories so provenance can interoperate with external
archives; the canonical file IRI still carries `sha256`.

### 4.3 Scanner (`scan.rs` + `rust_analyze.rs` + `parse.rs`)

- **Walk:** sorted `walkdir` over `crates/**`, `modules/**`, `vendor/**`; a file belongs to exactly one
  module (its crate root or nearest `plugin.toml` ancestor).
- **Manifests:** parse `Cargo.toml` (workspace members + intra-workspace deps) and `plugin.toml`
  (nexus shape; requires `[plugin].uri`, `version`, `[metadata].owned_by_phase`). Missing keys are hard
  errors.
- **Symbols:** `ra_ap_hir`/`ra_ap_ide` (via a small `ra_ap_*` facade) yield definitions and references;
  `syn` parses item declarations for T-Box functions and exported traits. Analysis failure is a hard
  error — never a silent skip. For `vendor/**` third-party trees where full analysis is impractical,
  records are marked `mmc:generatedFrom "manifest"` and only manifest-level metadata is emitted.
- **Incremental:** tree-sitter parses only files whose `(path, size, mtime, sha256)` changed since the
  last `codex_run`; the per-file `symbol_index` rows are rebuilt only for changed files.

### 4.4 SQLite (migration `0002_codex.sql`)

```sql
CREATE TABLE module_index (
  id              TEXT(26) PRIMARY KEY,
  module_uri      TEXT NOT NULL UNIQUE,
  rel_path        TEXT NOT NULL UNIQUE,
  version         TEXT NOT NULL,
  category        TEXT,
  crate_name      TEXT,
  plugin_type     TEXT,
  owned_phase     INTEGER NOT NULL,
  content_hash    TEXT NOT NULL,
  copied_from     TEXT,
  first_seen_ulid TEXT(26) NOT NULL,
  last_seen_ulid  TEXT(26) NOT NULL
);
CREATE TABLE module_file (
  id           TEXT(26) PRIMARY KEY,
  module_uri   TEXT NOT NULL REFERENCES module_index(module_uri),
  rel_path     TEXT NOT NULL UNIQUE,
  content_hash TEXT NOT NULL,
  swhid        TEXT,
  lang         TEXT NOT NULL
);
CREATE TABLE symbol_index (
  id           TEXT(26) PRIMARY KEY,
  symbol_uri   TEXT NOT NULL UNIQUE,
  descriptor   TEXT NOT NULL,
  kind         TEXT NOT NULL,          -- interface | trait | fn | type | const | module
  module_uri   TEXT NOT NULL,
  file_path    TEXT NOT NULL,
  byte_start   INTEGER NOT NULL,
  byte_end     INTEGER NOT NULL,
  is_public    INTEGER NOT NULL,
  content_hash TEXT NOT NULL,
  UNIQUE(descriptor, module_uri)
);
CREATE TABLE symbol_reference (
  from_symbol  TEXT NOT NULL,
  to_symbol    TEXT NOT NULL,
  file_path    TEXT NOT NULL,
  byte_start   INTEGER NOT NULL,
  PRIMARY KEY(from_symbol, to_symbol, file_path, byte_start)
);
CREATE TABLE capability_index (
  id         TEXT(26) PRIMARY KEY,
  capability TEXT NOT NULL,
  module_uri TEXT NOT NULL,
  test_count INTEGER NOT NULL DEFAULT 0,
  UNIQUE(capability, module_uri)
);
CREATE TABLE module_dep (
  from_uri TEXT NOT NULL, to_uri TEXT NOT NULL,
  kind TEXT NOT NULL,                  -- crate | module | reference
  PRIMARY KEY(from_uri, to_uri, kind)
);
CREATE TABLE codex_run (
  id         TEXT(26) PRIMARY KEY,
  started_at TEXT NOT NULL,
  modules    INTEGER NOT NULL,
  files      INTEGER NOT NULL,
  symbols    INTEGER NOT NULL,
  violations INTEGER NOT NULL,
  graph_hash TEXT NOT NULL,
  changed_files INTEGER NOT NULL
);
```

### 4.5 SHACL shapes (`ontology/shapes/mmc-shapes.ttl`)

Generated where possible by `rdf-shacl::generate_shacl`; hand-written shapes cover the rest:
`ModuleShape` (exactly one `mmc:path`, `mmc:uri`, `mmc:version`, `mmc:ownedByPhase`; ≥1
`mmc:implementsCapability`), `FileShape` (exactly one `mmc:inModule`, one `mmc:contentHash`), `SymbolShape`
(exactly one `mmc:descriptor`, one `mmc:definedIn`), `CapabilityShape` (≥1 `mmc:hasTest`, every
`mmc:testCovers` target a `mmc:Symbol`), `DependencyShape` (acyclic; `mmc:dependsOn` targets resolve),
`CopiedShape` (`mmc:copiedFrom` present iff the path is under `vendor/**` or origin ≠ `metamind`).

### 4.6 Emission and CLI

`metadata.ttl` per module (canonical, via `rdf-codec`):

```turtle
<https://metamind.dev/code/module/cognition/case-match> a mmc:NexusPlugin ;
    mmc:path "modules/cognition/case-match" ;
    mmc:uri  "https://metamind.dev/code/module/cognition/case-match" ;
    mmc:version "0.1.0" ;
    mmc:ownedByPhase <https://metamind.dev/phase/7> ;
    mmc:dependsOn <https://metamind.dev/code/module/crates/mm-core> ;
    mmc:implementsCapability <https://metamind.dev/code/capability/CaseRetrieval> ;
    mmc:hasTest <https://metamind.dev/code/symbol/modules/cognition/case-match#tests> ;
    mmc:sourceFile <https://metamind.dev/code/file/modules/cognition/case-match/src/lib.rs@ab12…> .
<https://metamind.dev/code/symbol/modules/cognition/case-match#case_match().> a mmc:Interface ;
    mmc:descriptor "case_match()" ;
    mmc:definedIn <https://metamind.dev/code/file/modules/cognition/case-match/src/lib.rs@ab12…> ;
    mmc:ownedByPhase <https://metamind.dev/phase/7> .
```

```text
mm-cli codex scan   [--root <dir>] [--incremental|--full] [--no-emit] [--update-lock] [--json]
mm-cli codex verify [--root <dir>] [--json]
mm-cli codex graph  [--format mermaid|html] [--out <file>] [--focus <module-uri>]
mm-cli codex diff   [--lock codex.lock]
mm-cli codex meta   --query <name> [--module <uri>] [--phase <n>]
mm-cli codex lock   [--check]
```

`plugin.toml` gains source provenance fields:

```toml
[source]
origin      = "metamind"                       # or nexus | rust_extract | rust_symbolic
copied_from = "https://github.com/<origin>@<revision>"   # required when origin != "metamind"
license     = "MIT"
```

---

## 5. Build sequence

1. **Copy-in + workspace.** Create `vendor/{nexus,rust_extract,rust_symbolic}/`, add needed crates as
   members, write `COPYING.md` per origin. *Check:* `cargo build -p rdf-codec -p rdf-shacl` green.
2. **URI funnel + symbol descriptors** in `mm-core::codex` and `mm-codex::symbol` with stability unit
   tests. *Check:* `cargo test -p mm-core codex`.
3. **`ontology/code.ttl`** with Kythe/SCIP-named classes/edges; load into `/code` at startup.
4. **Shapes.** Generate via `rdf-shacl`, hand-write the exactly-one / ≥1-test / acyclic / copied rules.
   *Check:* empty `/code` validates, 0 violations.
5. **Migration `0002_codex.sql`** (all tables above).
6. **`manifest.rs` + `hash.rs`.** Manifest validation; sha256 + optional SWHID; canonical module
   `content_hash` = hash of sorted `(rel_path, sha256)`. *Check:* order-independence property.
7. **`parse.rs` + `rust_analyze.rs`.** tree-sitter incremental parse; `ra_ap_*`/`syn` symbol, interface,
   and reference extraction. *Check:* fixture extraction matches golden symbols.
8. **`symbol.rs`.** SCIP-style descriptor builder + `symbol_iri`; `symbol_index`/`symbol_reference`
   upsert. *Check:* descriptor stability under reformatting/reordering.
9. **`scan.rs`.** Deterministic incremental walk; assign files to modules; upsert `/code` + SQLite; emit
   `metadata.ttl`; emit `mmc:copiedFrom` for `vendor/**` and non-`metamind` origins.
10. **`registry.rs`.** Write stable `registry.json`; upsert `module_index`.
11. **`lock.rs`.** Write/compare `codex.lock` (module → version + content_hash + capability set).
12. **`verify.rs`.** All `VerifyFailure` rules + cycle detection. *Check:* each negative fixture fails
    with its specific variant.
13. **`drift.rs`.** ast-grep/`syn` structural drift checks (e.g. a public signature changed without a
    version bump). *Check:* drifted fixture flagged.
14. **`meta.rs`.** `MetaQuery` (`TotalModules`, `ModulesPerPhase`, `SymbolsPerModule`,
    `CapabilitiesWithoutTests`, `DependencyCycles`, `VersionDrift`, `Churn`, `OrphanFiles`,
    `CopiedFrom`, `UnreferencedSymbols`) answered by SPARQL over `/code` (+ SQLite for churn).
15. **`graph_view.rs`.** Derived Mermaid module/dependency/symbol view via vendored `kg-diagram`; render
    manifest traces every node/edge to a triple or counts it hidden.
16. **Wire `mm-cli codex`**, add `codex scan --update-lock` to CI, backfill metadata for every `mm-*`
    crate and the Phase 1 bootstrap module. *Check:* `codex scan` then `codex verify` on the real tree.

---

## 6. Logging and observability

All records go through `mm-log` (parent plan §10). The scan opens a `codex.scan` span whose ULID is the
`trace_id` and is stored in `codex_run.id`.

| Event code | Level | Key fields | Sink |
|---|---|---|---|
| `codex.scan.start` | INFO | `root`, `mode`, `config_hash`, `scan_id` | console, JSONL |
| `codex.copying.reference` | INFO | `origin`, `revision`, `license`, `dest` | JSONL, `/provenance` |
| `codex.module.discovered` | DEBUG | `module_uri`, `rel_path`, `version`, `owned_phase`, `content_hash` | JSONL |
| `codex.file.discovered` | TRACE | `file_path`, `sha256`, `swhid`, `lang`, `module_uri` | JSONL |
| `codex.symbol.extracted` | TRACE | `symbol_uri`, `descriptor`, `kind`, `byte_start`, `byte_end` | JSONL |
| `codex.dependency.edge` | DEBUG | `from_uri`, `to_uri`, `kind` | JSONL |
| `codex.capability.registered` | INFO | `capability`, `module_uri`, `test_count` | JSONL, SQLite |
| `codex.copied_from.record` | INFO | `module_uri`, `copied_from`, `revision` | `/code`, `/provenance` |
| `codex.verify.fail` | WARN | `rule`, `module_uri`, `detail` | console, JSONL, SQLite |
| `codex.drift.detected` | WARN | `module_uri`, `recorded_version`, `content_hash`, `pattern` | console, JSONL |
| `codex.registry.write` | INFO (audit) | `path`, `modules`, `files`, `symbols`, `graph_hash` | audit + JSONL |
| `codex.lock.write` / `codex.lock.diff` | INFO | `path`, `added`, `removed`, `changed` | audit + JSONL |
| `codex.graph.emit` | INFO | `format`, `nodes`, `edges`, `hidden`, `out` | JSONL |
| `codex.scan.end` | INFO | `scan_id`, `modules`, `files`, `symbols`, `changed_files`, `violations`, `graph_hash`, `latency_ms` | console, JSONL, SQLite |

Audit (immutable): `codex.registry.write`, `codex.lock.write`, and every `codex.verify.fail`.
**Source content is never logged — only hashes and byte ranges** (unless `MM_CODEX_LOG_SOURCE=1` in dev
builds); absolute paths are redacted. `mm-cli logs verify` checks schema, gaplessness, ULID correlation,
and redaction.

---

## 7. Testing

- **Unit.** `manifest.rs`, `rust_analyze.rs` (T-Box/trait/pub extraction), `symbol.rs` (descriptors),
  `hash.rs`, `emit.rs`, `lock.rs`.
- **Property (`proptest`).** `file_iri`/`symbol_iri` stable for equal inputs; module `content_hash`
  invariant under walk order, mtime change, and reformatting; dependency graph is a DAG iff no cycle.
- **Symbol stability (`symbol_stability.rs`).** Moving a function within a file, reordering items, and
  reformatting produce the same descriptor and symbol IRI; renaming produces a new IRI.
- **Golden (`insta`).** `bench/codex/golden/`: canonical `metadata.ttl`, `registry.json`, `codex.lock`,
  and the `/code` graph hash for a fixture tree.
- **SHACL.** Valid fixture → 0 violations; broken fixtures (missing uri, orphan file, capability without
  test) → ≥1 violation of the expected shape.
- **Negatives.** One fixture per `VerifyFailure`: orphan file, duplicate URI, dependency cycle,
  capability without tests, missing owning phase, version drift.
- **Idempotence.** Two consecutive scans → identical `/code` graph hash and identical `registry.json`;
  an incremental scan of an unchanged tree changes zero rows.
- **Meta.** Each `MetaQuery` returns the expected set against fixtures.
- **E2E.** `codex scan` → `codex verify` → `codex meta --query TotalModules` on the real workspace.

---

## 8. Pass gate

Objective, runnable. `set -o pipefail`; capture exit status explicitly.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 1. Scan + verify the real workspace
mm-cli codex scan
mm-cli codex verify                     # exits 0

# 2. Code graph validates
mm-cli graph validate --graph code      # 0 SHACL violations

# 3. Coverage / meta
mm-cli codex meta --query OrphanFiles             # "0 orphan files"
mm-cli codex meta --query TotalModules
mm-cli codex meta --query ModulesPerPhase         # non-empty for phases present
mm-cli codex meta --query CapabilitiesWithoutTests
mm-cli codex meta --query VersionDrift

# 4. Negative cases — each must fail with a SPECIFIC message
for c in orphan_file duplicate_uri dependency_cycle capability_without_test; do
  if mm-cli codex verify --root bench/codex/fixtures/bad_$c; then
    echo "FAIL: $c not detected"; exit 1
  else echo "ok: $c rejected"; fi
done

# 5. Idempotence — identical graph hash across two scans
h1=$(mm-cli codex scan --json | jq -r .graph_hash)
h2=$(mm-cli codex scan --json | jq -r .graph_hash)
test "$h1" = "$h2" || { echo "FAIL: scan not idempotent"; exit 1; }

# 6. Lock drift + logging
mm-cli codex lock --check               # exits 0 when codex.lock matches
mm-cli logs verify
```

**Pass criteria.** Zero-warning build; `codex verify` exits 0 on the real workspace and non-zero with a
rule-specific message on every negative fixture; `/code` SHACL-validates with 0 violations; `codex meta`
answers total modules, modules per phase, capabilities without tests, and version drift; symbol IRIs are
stable under move/reformat; two scans yield an identical graph hash; `codex.lock --check` clean;
`mm-cli logs verify` passes.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Rust semantic analysis is heavy / fragile | Incremental parse of changed files only; treat analysis failure as a hard error; manifest-only records for `vendor/**`, marked `mmc:generatedFrom "manifest"` |
| Symbol descriptors drift across refactors | Descriptors derive from scope+item path+kind (not offsets); `symbol_stability.rs` asserts invariance under move/reformat |
| Metadata drifts from source | `codex verify` + committed `codex.lock` in CI; `VersionDrift` and `codex lock --check` fail on change without a version bump |
| Scanner slow on large trees | Sorted parallel walk; per-file hash cached by `(path, size, mtime)`; incremental re-index |
| Code graph polluted by runtime facts | Dedicated `/code` named graph; all other graphs separate by construction |
| False-positive dependency cycles | Cycle detection tested on acyclic + known-cyclic fixtures; `kind` distinguishes crate/module/reference |
| Generated artifacts committed by mistake | `modules/**/metadata.ttl` git-ignored; only `registry.json` and `codex.lock` committed and regenerated deterministically |
| Copied reference code blurs ownership | `COPYING.md` per origin + `mmc:copiedFrom` per file; `vendor/**` records carry origin; re-copy is a reviewed change-set |

---

## 10. References (code-available)

- SCIP — https://github.com/scip-code/scip · https://sourcegraph.com/blog/announcing-scip
- Software Heritage SWHID — https://github.com/ocamlpro/swhid · https://swhid.org/faq/
- Joern code property graph — https://github.com/joernio/joern
- Kythe — https://github.com/kythe/kythe
- tree-sitter — https://github.com/tree-sitter/tree-sitter
- rust-analyzer — https://github.com/rust-lang/rust-analyzer
- ast-grep — https://github.com/ast-grep/ast-grep
- `syn` — https://github.com/dtolnay/syn
- Oxigraph — https://github.com/oxigraph/oxigraph
- sqlx — https://github.com/launchbadge/sqlx
- Reference components copied in (parent plan §7): nexus plugin manifest + `nexus-rust-ast`,
  `rust_extract::kg-diagram`, `rust_symbolic::rdf-codec`, `rust_symbolic::rdf-shacl`.
