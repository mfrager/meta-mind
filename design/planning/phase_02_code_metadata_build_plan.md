# Phase 2 Build Plan — Modular code system and code-metadata registry

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 2 · Codename Metamind (`mm`)
> Depends on: Phase 1 (kernel, `mm-log`, stores, event log, `mm-cli`, `modules/` skeleton, `vendor/`).

---

## 1. Objective and scope

**Objective.** Make the codebase itself a first-class, queryable RDF artifact. Every module and source
file gets a **stable RDF identifier**, recorded in a dedicated named graph
(`https://metamind.dev/graph/code`), so the system can perform meta-analysis of its own code (§62, §85,
§105) and so the Pi-driven self-build (Phase 11) has a machine-readable map of the system's own modules:
what exists, what it depends on, what it exposes, what it tests, which phase owns it, and where every
copied-in file came from.

**In scope.**
- The `mmc:` code-metadata vocabulary (`ontology/code.ttl`) and its SHACL shapes.
- `crates/mm-codex`: a deterministic workspace scanner + emitter + verifier + meta-query engine.
- Generated per-module `metadata.ttl`, `modules/registry.json`, committed `codex.lock`.
- `mm-cli codex {scan, verify, graph, diff, meta}`.
- The `module_index` SQLite tables and the `/code` graph upsert.
- `mmc:copiedFrom` provenance for every file copied from an external reference (§7).

**Out of scope.** Runtime cognition, any self-modification, and Pi invocation (Phases 11–12). Phase 2
*describes and verifies* the code; it does not change it. It must be complete before Phase 11, because
self-modification needs the metadata graph as its map and its audit surface.

---

## 2. Prerequisites and dependencies

From **Phase 1** (must be green):
- `mm-core`: `Id`/`Ulid` factory, `Timestamp`, `MmError`, `Config`, content-hash helpers, and the async
  traits `Tabular`, `Graph`, `EventSink`.
- `mm-log`: structured logging, JSONL + audit sinks, ULID/span correlation, `Redactor`.
- `mm-store-sqlite` (sqlx, WAL, `sqlx::migrate!`), `mm-store-graph` (Oxigraph 0.5 actor, named-graph
  manager, async SPARQL), `mm-eventlog` (append→apply→commit + replay).
- `ontology/mm.ttl` + generated shapes; `mm-cli doctor|replay|graph validate`.
- `modules/` skeleton with one bootstrap module; `vendor/` tree + `COPYING.md` convention.

**Reference components copied in this phase (§6):** nexus plugin contract and `plugin.toml` shape
(`monad-llm`), `nexus-rust-ast` / `monad-rust` (Rust source parsing for T-Box/`#[tbox_fn]` extraction),
`rust_extract::kg-diagram` (deterministic derived graph views + render manifest), `rdf-codec`
(canonical RDF emission, `Namespace`, `RdfContext`), `rdf-shacl` (`check_ontology` → `generate_shacl` →
`validate`), plus `sha2`, `toml`, `serde`, `walkdir`, `rayon` (all already present in the reference
workspaces).

---

## 3. Deliverables (exact paths)

```
crates/mm-codex/
├── Cargo.toml
├── src/lib.rs              # public API re-exports
├── src/model.rs            # ModuleRecord, FileRecord, CapabilityRecord, DependencyEdge, CodexReport
├── src/scan.rs             # Codex::scan — deterministic workspace walk
├── src/manifest.rs         # plugin.toml parsing + validation (nexus shape)
├── src/cargo_meta.rs       # Cargo.toml workspace + crate parsing
├── src/rust_ast.rs         # T-Box fn/trait + pub item extraction (via copied nexus-rust-ast)
├── src/hash.rs             # sha256 file/dir content hashing; canonical content hash
├── src/emit.rs             # ModuleRecord -> canonical Turtle (rdf-codec), metadata.ttl
├── src/registry.rs         # registry.json + SQLite module_index upsert
├── src/verify.rs           # VerifyFailure rules; dependency-cycle detection
├── src/meta.rs             # MetaQuery over /code via SPARQL
├── src/graph_view.rs       # derived Mermaid view (kg-diagram conventions) + render manifest
├── src/lock.rs             # codex.lock read/write/compare
└── tests/
    ├── scan_fixtures.rs
    ├── idempotence.rs
    ├── verify_rules.rs
    └── meta_queries.rs

ontology/code.ttl                       # mmc: vocabulary (T-Box)
ontology/shapes/mmc-shapes.ttl          # SHACL shapes for the code graph
crates/mm-cli/src/cmd/codex.rs          # codex {scan,verify,graph,diff,meta,lock}
crates/mm-store-sqlite/migrations/0002_codex.sql
modules/registry.json                   # GENERATED, committed
codex.lock                              # GENERATED, committed (drift detection)
modules/<category>/<name>/metadata.ttl  # GENERATED per module, .gitignore'd
vendor/nexus/COPYING.md                 # + copied nexus source used by mm-codex
vendor/rust_extract/COPYING.md          # + copied kg-diagram source
vendor/rust_symbolic/COPYING.md         # + copied rdf-codec, rdf-shacl source
bench/codex/fixtures/                   # valid + negative fixture module trees
bench/codex/golden/                     # insta golden metadata.ttl / registry.json / lock
```

`.gitignore` adds `modules/**/metadata.ttl`; `modules/registry.json` and `codex.lock` are committed.

---

## 4. Data model and ontology deltas

`mmc:` = `https://metamind.dev/code#` (schema). Instances live in the `/code` graph. Design-document
anchors and phases use `https://metamind.dev/design/{doc}#{anchor}` and `https://metamind.dev/phase/{n}`.

`ontology/code.ttl` (extract):

```turtle
@prefix mmc: <https://metamind.dev/code#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

mmc:Module a rdfs:Class .
mmc:RustCrate a rdfs:Class ; rdfs:subClassOf mmc:Module .
mmc:NexusPlugin a rdfs:Class ; rdfs:subClassOf mmc:Module .
mmc:SourceFile a rdfs:Class .
mmc:Interface a rdfs:Class .            # a T-Box function / trait / exported type
mmc:TestSet a rdfs:Class .
mmc:Capability a rdfs:Class .
mmc:ModuleVersion a rdfs:Class .
mmc:ChangeSet a rdfs:Class .
mmc:PiSession a rdfs:Class .
mmc:Edit a rdfs:Class .
mmc:ToolCall a rdfs:Class .

# predicates
# path uri version dependsOn exports requires implementsCapability hasTest testCovers
# contentHash ownedByPhase promotedFrom registryIndex copiedFrom
# category crateName pluginType sourceFile inModule tboxFunction generatedFrom
```

Stable URI builders (single funnel in `mm-core`):
- module: `https://metamind.dev/code/module/{relative-path}`
- version: `https://metamind.dev/code/module/{relative-path}@{semver}`
- file: `https://metamind.dev/code/file/{relative-path}@{sha256}`
- phase: `https://metamind.dev/phase/{n}`

Sample generated `modules/cognition/case-match/metadata.ttl`:

```turtle
<https://metamind.dev/code/module/cognition/case-match> a mmc:NexusPlugin ;
    mmc:path "modules/cognition/case-match" ;
    mmc:uri "https://metamind.dev/code/module/cognition/case-match" ;
    mmc:version "0.1.0" ;
    mmc:category "cognition" ;
    mmc:ownedByPhase <https://metamind.dev/phase/7> ;
    mmc:dependsOn <https://metamind.dev/code/module/crates/mm-core> ;
    mmc:implementsCapability mm:CaseRetrieval ;
    mmc:hasTest <https://metamind.dev/code/module/cognition/case-match#tests> ;
    mmc:sourceFile <https://metamind.dev/code/file/modules/cognition/case-match/src/lib.rs@ab12…> .
```

SQLite (migration `0002_codex.sql`):

```sql
CREATE TABLE module_index (
  id            TEXT(26) PRIMARY KEY,
  module_uri    TEXT NOT NULL UNIQUE,
  rel_path      TEXT NOT NULL UNIQUE,
  version       TEXT NOT NULL,
  category      TEXT,
  crate_name    TEXT,
  plugin_type   TEXT,
  owned_phase   INTEGER NOT NULL,
  content_hash  TEXT NOT NULL,
  first_seen_ulid TEXT(26) NOT NULL,
  last_seen_ulid  TEXT(26) NOT NULL
);
CREATE TABLE module_file (
  id            TEXT(26) PRIMARY KEY,
  module_uri    TEXT NOT NULL REFERENCES module_index(module_uri),
  rel_path      TEXT NOT NULL,
  content_hash  TEXT NOT NULL,
  lang          TEXT NOT NULL,
  UNIQUE(rel_path)
);
CREATE TABLE capability_index (
  id            TEXT(26) PRIMARY KEY,
  capability    TEXT NOT NULL,
  module_uri    TEXT NOT NULL,
  test_count    INTEGER NOT NULL DEFAULT 0,
  UNIQUE(capability, module_uri)
);
CREATE TABLE module_dep (
  from_uri TEXT NOT NULL,
  to_uri   TEXT NOT NULL,
  kind     TEXT NOT NULL,           -- crate | module | reference
  PRIMARY KEY(from_uri, to_uri, kind)
);
CREATE TABLE codex_run (
  id         TEXT(26) PRIMARY KEY,
  started_at TEXT NOT NULL,
  modules    INTEGER NOT NULL,
  files      INTEGER NOT NULL,
  violations INTEGER NOT NULL,
  graph_hash TEXT NOT NULL
);
```

SHACL shapes (`ontology/shapes/mmc-shapes.ttl`): `ModuleShape` (exactly one `mmc:path`, one `mmc:uri`,
one `mmc:version`, one `mmc:ownedByPhase`; ≥1 `mmc:implementsCapability`), `SourceFileShape` (exactly
one `mmc:inModule`), `CapabilityShape` (≥1 `mmc:hasTest`), `DependencyShape` (acyclic; `mmc:dependsOn`
targets resolve to a registered module).

---

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

`crates/mm-core/src/codex.rs` (URI funnel — added in this phase):

```rust
pub fn module_iri(rel_path: &str) -> NamedNode;
pub fn module_version_iri(rel_path: &str, semver: &str) -> NamedNode;
pub fn file_iri(rel_path: &str, sha256: &str) -> NamedNode;
pub fn phase_iri(n: u8) -> NamedNode;
```

`crates/mm-codex/src/model.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModuleRecord {
    pub id: Ulid, pub uri: String, pub rel_path: String, pub version: String,
    pub category: Option<String>, pub plugin_type: PluginType, pub crate_name: Option<String>,
    pub owned_phase: u8, pub capabilities: Vec<CapabilityRecord>,
    pub depends_on: Vec<String>, pub files: Vec<FileRecord>, pub content_hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileRecord { pub id: Ulid, pub rel_path: String, pub sha256: String, pub lang: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityRecord { pub name: String, pub tests: Vec<String> }
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DependencyEdge { pub from: String, pub to: String, pub kind: DepKind }
#[derive(Debug, Clone, Default, Serialize)]
pub struct CodexReport {
    pub scan_id: Ulid, pub modules: usize, pub files: usize,
    pub violations: Vec<VerifyFailure>, pub graph_hash: String,
}
```

`crates/mm-codex/src/lib.rs`:

```rust
pub struct Codex { /* root, graph handle, sqlite handle, log, config */ }
impl Codex {
    pub async fn open(root: &Path, cfg: CodexConfig) -> Result<Self, CodexError>;
    pub async fn scan(&self) -> Result<CodexReport, CodexError>;
    pub async fn verify(&self) -> Result<(), Vec<VerifyFailure>>;
    pub async fn write_registry(&self) -> Result<PathBuf, CodexError>;   // registry.json
    pub async fn write_lock(&self, out: &Path) -> Result<(), CodexError>; // codex.lock
    pub async fn diff_lock(&self, lock: &Path) -> Result<Vec<ChangeSet>, CodexError>;
    pub async fn graph_mermaid(&self, opts: ViewOptions) -> Result<String, CodexError>;
    pub async fn meta(&self, q: MetaQuery) -> Result<MetaAnswer, CodexError>;
}
pub enum VerifyFailure {
    OrphanFile { rel_path: String },
    DuplicateUri { uri: String, a: String, b: String },
    DependencyCycle { cycle: Vec<String> },
    CapabilityWithoutTest { capability: String, module_uri: String },
    MissingStableUri { module_uri: String },
    MissingOwningPhase { module_uri: String },
    VersionDrift { module_uri: String, recorded: String, content_hash: String },
}
```

`mm-cli` subcommands:

```text
mm-cli codex scan   [--root <dir>] [--no-emit] [--update-lock]
mm-cli codex verify [--root <dir>] [--json]
mm-cli codex graph  [--format mermaid|html] [--out <file>]
mm-cli codex diff   [--lock codex.lock]
mm-cli codex meta   --query <name> [--module <uri>] [--phase <n>]
mm-cli codex lock   [--check]
```

`plugin.toml` (extended nexus shape — the authoritative module manifest):

```toml
[plugin]
name = "mm-case-match"
uri  = "https://metamind.dev/code/module/cognition/case-match"   # STABLE
version = "0.1.0"
[metadata]
category = "cognition"
owned_by_phase = 7
capability = "mm:CaseRetrieval"
[source]
origin = "metamind"            # or "nexus" / "rust_extract" / "rust_symbolic" for copied code
copied_from = "https://github.com/mfrager/…@<revision>"   # required when origin != metamind
license = "MIT"
```

---

## 6. External references copied in and integration

Per §7 of the parent plan, all of the following are **copied into `vendor/` and integrated as
Metamind-owned source** (no submodules or path dependencies), each with a `COPYING.md` recording origin
repo, revision, and license, and a `mmc:copiedFrom` triple emitted by this phase's scanner.

| Reference | Copied into | Used for |
|---|---|---|
| nexus plugin manifest contract + `plugin.toml` (`plugins/ai/llm`) | `vendor/nexus/plugin-manifest/` | manifest schema, `uri` convention, T-Box function declarations |
| `nexus-rust-ast` + `plugins/foundation/rust-ast/monad-rust` | `vendor/nexus/rust-ast/` | parse Rust to extract `#[tbox_fn]`/traits/public items |
| `rust_extract::kg-diagram` | `vendor/rust_extract/kg-diagram/` | deterministic Mermaid view + render manifest for `codex graph` |
| `rust_symbolic::rdf-codec` | `vendor/rust_symbolic/rdf-codec/` | canonical Turtle emission, `Namespace`, `RdfContext`, `new_ulid` |
| `rust_symbolic::rdf-shacl` | `vendor/rust_symbolic/rdf-shacl/` | `check_ontology` → `generate_shacl` → `validate` for the code graph |
| `mm-log` audit conventions | (already in-tree, Phase 1) | registry/lock writes as audit records |

Integration notes: `mm-codex` depends only on `mm-core`, `mm-log`, `mm-store-graph`, `mm-store-sqlite`,
and the vendored crates; the extracted `nexus-rust-ast` API is wrapped behind `mm-codex::rust_ast` so the
runtime never depends on the reference's internal types. Originals kept as read-only references under
`design/` or `reference/` (not compiled).

---

## 7. Step-by-step implementation tasks

1. **Copy in references.** Create `vendor/{nexus,rust_extract,rust_symbolic}/`, add the needed crates as
   workspace members, write `COPYING.md` per origin. *Check:* `cargo build -p kg-diagram -p rdf-codec -p rdf-shacl` green.
2. **Add URI funnel** in `crates/mm-core/src/codex.rs` (`module_iri`, `module_version_iri`, `file_iri`,
   `phase_iri`) + unit tests for stability. *Check:* `cargo test -p mm-core codex`.
3. **Author `ontology/code.ttl`** with all `mmc:` classes/predicates; load it into `/code` at start.
4. **Generate shapes** with `rdf-shacl::generate_shacl` and hand-write `mmc-shapes.ttl` for the rules the
   generator cannot express (exactly-one, ≥1-test, acyclicity). *Check:* empty code graph validates, 0 violations.
5. **Migration `0002_codex.sql`** for `module_index/module_file/capability_index/module_dep/codex_run`.
6. **`model.rs` + `manifest.rs`:** parse `plugin.toml`; enforce required `[plugin].uri`, `version`,
   `[metadata].owned_by_phase`; classify `PluginType`. *Check:* fixture manifests parse; missing `uri` errors.
7. **`cargo_meta.rs`:** read the workspace `Cargo.toml`, enumerate members and each crate's
   `[dependencies]` to build `kind="crate"` dependency edges (workspace-internal only).
8. **`rust_ast.rs`:** wrap the vendored nexus Rust parser to extract `#[tbox_fn]` functions, exported
   traits, and `pub` items into `Interface` records and `capability`/`hasTest` links. Parse failure is a
   hard error (never a silent skip).
9. **`hash.rs`:** sha256 per file; a canonical module `content_hash` = hash of sorted `(rel_path, sha256)`
   pairs (independent of walk order/mtime). *Check:* property test stable under reordering.
10. **`scan.rs`:** deterministic walk (sorted paths) of `modules/**`, `crates/**`, `vendor/**`; assign
    every file to exactly one module (its owning crate/module); build `ModuleRecord`s; upsert `/code`
    graph + SQLite tables; emit per-module `metadata.ttl`; record `mmc:copiedFrom` for `vendor/**` and
    any manifest with `origin != "metamind"`.
11. **`registry.rs`:** write `modules/registry.json` (stable key order, ULIDs, content hashes); upsert
    `module_index`.
12. **`lock.rs`:** write `codex.lock` (module → version + content_hash + capability set); `diff` returns a
    `ChangeSet` list (added/removed/changed modules, version-bump drift).
13. **`verify.rs`:** implement every `VerifyFailure` rule and cycle detection over `module_dep`. *Check:*
    negative fixtures each fail with their specific variant.
14. **`meta.rs`:** `MetaQuery` enum (`TotalModules`, `ModulesPerPhase`, `CapabilitiesWithoutTests`,
    `DependencyCycles`, `VersionDrift`, `Churn`, `OrphanFiles`, `CopiedFrom`), answered via SPARQL over
    `/code` (+ SQLite for churn). *Check:* meta query tests against fixtures.
15. **`graph_view.rs`:** derived Mermaid module/dependency diagram via vendored `kg-diagram`; emit a
    render manifest (every node/edge traces to a triple or is counted hidden).
16. **Wire `mm-cli codex`** subcommands; add `codex scan --update-lock` to the CI step; backfill metadata
    for the Phase 1 bootstrap module and every `mm-*` crate. *Check:* `codex scan` then `codex verify` on
    the real workspace.

---

## 8. Detailed logging requirements

All records go through `mm-log` (parent plan §10) with global fields (`ts`, `level`, `target`, `event`,
`msg`, `trace_id`, …). The scan opens a `codex.scan` span whose ULID is the `trace_id` for every record
below; the ULID is stored in `codex_run.id`.

| Event code | Level | Key fields | Sink |
|---|---|---|---|
| `codex.scan.start` | INFO | `root`, `config_hash`, `scan_id` | console, JSONL |
| `codex.copying.reference` | INFO | `origin`, `revision`, `license`, `dest` | JSONL, `/provenance` |
| `codex.module.discovered` | DEBUG | `module_uri`, `rel_path`, `version`, `owned_phase`, `content_hash` | JSONL |
| `codex.file.discovered` | TRACE | `file_path`, `sha256`, `lang`, `module_uri` | JSONL |
| `codex.dependency.edge` | DEBUG | `from_uri`, `to_uri`, `kind` | JSONL |
| `codex.capability.registered` | INFO | `capability`, `module_uri`, `test_count` | JSONL, SQLite |
| `codex.copied_from.record` | INFO | `module_uri`, `copied_from`, `revision` | `/code`, `/provenance` |
| `codex.verify.fail` | WARN | `rule`, `module_uri`, `detail`, `reason` | console, JSONL, SQLite |
| `codex.drift.detected` | WARN | `module_uri`, `recorded_version`, `content_hash` | console, JSONL |
| `codex.registry.write` | INFO (audit) | `path`, `modules`, `files`, `graph_hash` | audit + JSONL |
| `codex.lock.write` / `codex.lock.diff` | INFO | `path`, `added`, `removed`, `changed` | audit + JSONL |
| `codex.graph.emit` | INFO | `format`, `nodes`, `edges`, `hidden`, `out` | JSONL |
| `codex.scan.end` | INFO | `scan_id`, `modules`, `files`, `violations`, `graph_hash`, `latency_ms` | console, JSONL, SQLite |

Audit (immutable): `codex.registry.write`, `codex.lock.write`, and every `codex.verify.fail` are written
transactionally with the event that produced them, keyed by monotonic sequence. Redaction applies to
absolute filesystem paths where they may contain user names; source content is never logged (only
hashes) unless `MM_CODEX_LOG_SOURCE=1` in dev builds.

---

## 9. Testing plan

- **Unit.** `manifest.rs` (valid/missing-key/precise errors), `cargo_meta.rs` (member + dep extraction),
  `rust_ast.rs` (T-Box/trait/pub extraction on fixtures), `hash.rs` (order independence), `emit.rs`
  (canonical Turtle), `lock.rs` (diff kinds).
- **Property (`proptest`).** `file_iri`/`module_iri` stable for equal inputs; module `content_hash`
  invariant under walk order and mtime change; scanner output dependency graph is a DAG iff no cycle.
- **Golden (`insta`).** `bench/codex/golden/`: canonical `metadata.ttl` for a fixture module,
  `registry.json`, and `codex.lock`.
- **SHACL conformance.** Valid fixture → 0 violations; broken fixtures (missing uri, orphan file,
  capability without test) → ≥1 violation with the expected shape.
- **Negatives (`verify_rules.rs`).** Each `VerifyFailure` has a fixture that triggers exactly it:
  orphan file, duplicate URI, dependency cycle, capability without tests, missing owning phase, version
  drift (content changed without version bump).
- **Idempotence (`idempotence.rs`).** Two consecutive scans of the same tree produce identical `/code`
  graph content hash and identical `registry.json`.
- **Meta (`meta_queries.rs`).** Each `MetaQuery` returns the expected set against fixtures.
- **E2E.** `mm-cli codex scan` → `codex verify` → `codex meta --query TotalModules` on the real workspace.

---

## 10. Pass gate

Objective, runnable. All commands exit 0 unless stated; `set -o pipefail`; capture exit status explicitly.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 1. Scan + verify the real workspace
mm-cli codex scan
mm-cli codex verify                 # exits 0

# 2. Code graph validates
mm-cli graph validate --graph code  # 0 SHACL violations

# 3. Coverage: every file belongs to exactly one module
mm-cli codex meta --query OrphanFiles            # prints "0 orphan files"
mm-cli codex meta --query TotalModules
mm-cli codex meta --query ModulesPerPhase        # non-empty for phases 1..12 present
mm-cli codex meta --query CapabilitiesWithoutTests
mm-cli codex meta --query VersionDrift           # lists modules changed w/o version bump

# 4. Negative cases — each must fail with a SPECIFIC message
for c in orphan_file duplicate_uri dependency_cycle capability_without_test; do
  mm-cli codex scan --root bench/codex/fixtures/bad_$c
  if mm-cli codex verify --root bench/codex/fixtures/bad_$c; then
    echo "FAIL: $c not detected"; exit 1
  else
    echo "ok: $c rejected"
  fi
done

# 5. Idempotence — identical graph hash across two scans
h1=$(mm-cli codex scan --json | jq -r .graph_hash)
h2=$(mm-cli codex scan --json | jq -r .graph_hash)
test "$h1" = "$h2" || { echo "FAIL: scan not idempotent"; exit 1; }

# 6. Lock drift detection
mm-cli codex lock --check            # exits 0 when codex.lock matches

# 7. Logging
mm-cli logs verify
```

**Pass criteria.** Workspace builds clean with zero warnings; `codex verify` exits 0 on the real
workspace and non-zero with a rule-specific message on every negative fixture; code graph SHACL-validates
with 0 violations; `codex meta` answers total modules, modules per phase, capabilities without tests, and
version drift; two scans yield an identical graph hash; `codex.lock --check` is clean; `mm-cli logs verify`
passes (schema, gapless audit sequence, ULID correlation, redaction).

---

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Rust AST parsing is fragile across editions | Use the vendored `nexus-rust-ast`; treat parse failure as a hard error; fall back to manifest-level metadata only for `vendor/**` third-party trees, and mark those records `mmc:generatedFrom "manifest"` |
| Metadata drifts from source | `codex verify` runs in CI; `codex.lock` is committed; `VersionDrift` query and `codex_lock --check` fail on change without a version bump |
| Scanner is slow on large trees | Parallel sorted walk, per-file hash cached by `(path, size, mtime)`; incremental re-hash of changed modules only |
| Code graph polluted by runtime facts | Dedicated `/code` named graph; every other graph is separate by construction |
| False-positive dependency cycles | Cycle detection unit-tested on acyclic + known-cyclic fixtures; `kind` distinguishes crate/module/reference edges |
| Generated artifacts committed by mistake | `modules/**/metadata.ttl` git-ignored; only `registry.json` and `codex.lock` committed and regenerated deterministically |
| Copying reference code blurs ownership | `COPYING.md` per origin + `mmc:copiedFrom` per file; `vendor/**` records carry origin; re-copy is a reviewed change-set |

---

## 12. Design traceability

| Design source | Section / theme | Phase 2 artifact |
|---|---|---|
| `digital_mind_design1.md` | §55–56 self-bootstrapping, capability objects | module ↔ capability records; `mmc:Capability` |
| `digital_mind_design1.md` | §57–58 self-engineering, code/data co-evolution | module version + `CapabilityVersion` fields in `ModuleRecord` |
| `digital_mind_design1.md` | §62 self-model; §85 architecture debt; §105 learn its own architecture | `codex meta` (cycles, orphans, unused capabilities, churn) |
| `digital_mind_design1.md` | §87 RDF/OWL data model; §88 runtime model; §89–90 ops/backend map | `mmc:` vocabulary in `/code`; class/relation naming aligned with §87 |
| `digital_mind_design1.md` | §108–110 guardrails and invariants | separate `/code` graph; no runtime facts conflated; generated files not hand-edited |
| `meta_analysis1.md` | self-scientist, ROI, improvement/evolution budgets | meta queries supply the measurement base for later budgets |
| `meta_analysis2.md` | metacognitive controller, criticality propagation | module dependency graph as the structural substrate for later analysis |
| `bootstrap_procses2.md` | SELF as mutable system; code/data versioned together | stable URIs + version + content hash enabling change-sets |
| parent plan `implementation_plan1.md` | §4 identifier scheme; §6 ontology; §7 copy-in; §8 module contract; §10 logging | URI funnel, `mmc:`, `vendor/`, `plugin.toml`, per-phase log records |
