# Phase 1 Build Plan — Foundation: deterministic kernel, dual stores, ontology v0

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 1 · Codename Metamind (`mm`)

---

## 1. Objective and scope

Build the deterministic, non-cognitive kernel the entire being stands on: a compiling Rust workspace
with ULID identity, an append-only event log with deterministic replay, **SQLite** (tabular state) and
**Oxigraph** (RDF state) behind one async abstraction, `ontology v0`, structured logging, and the test
harness every later phase is gated on. This is the design's "Bootstrap Phase 0: Deterministic Kernel"
(design §92), with PostgreSQL replaced by SQLite per the parent plan §0/§5.

**In scope**
- Cargo workspace, toolchain pin, shared lints, CI skeleton.
- `mm-core`: ULIDs, timestamps, errors, config, async store traits, content hashing.
- `mm-log`: `tracing` subscriber, structured JSONL + immutable audit sinks, redaction.
- `mm-store-sqlite`: `sqlx` pool, WAL pragmas, migrations, `Tabular` impl.
- `mm-store-graph`: Oxigraph single-writer async actor + named graphs, `Graph` impl.
- `mm-eventlog`: append→apply→commit protocol and `replay`.
- `ontology/mm.ttl` (T-Box v0) + generated SHACL shapes.
- `mm-cli`: `doctor`, `replay`, `graph validate`, `logs verify`.
- One bootstrap plugin module + generated module registry skeleton.

**Out of scope (later phases)**
- Any LLM call, memory, being state, cognition, or self-modification.
- Real content in the forward tables (only their migration scaffolding exists).

**Definition of done** — the Phase 1 pass gate in §10 runs green as plain commands.

---

## 2. Prerequisites and dependencies

Verified on the build machine (parent plan §1):

| Dependency | Version | Use |
|---|---|---|
| Rust | 1.96.1, edition 2021 | workspace |
| SQLite | 3.46.1, `libsqlite3.so` | tabular store (`sqlx` 0.8) |
| Oxigraph | 0.5 + `librocksdb.so.9.11` | RDF store (`rocksdb-pkg-config`) |
| Tokio | 1.x | async runtime |

Phase 1 depends on no earlier phase. It is the hard prerequisite for Phases 2–12.

Reference code to be copied in (see §6): `rdf-codec` (ULID IRIs, namespaces, canonical
serialization), `temporal-store` (immutability/replay semantics), `nexus-core::logging`
(structured-logger conventions).

---

## 3. Deliverables (exact paths)

```
~/Build/metamind/
├── Cargo.toml                                   # [workspace], resolver=2, members = crates/* + modules/*/*
├── rust-toolchain.toml                          # channel = "1.96.1"
├── .cargo/config.toml                           # RUSTFLAGS for zero-warning CI
├── clippy.toml
├── rustfmt.toml
├── .github/workflows/ci.yml                     # build, clippy -D warnings, test, gates
├── crates/
│   ├── mm-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                           # #![forbid(unsafe_code)]
│   │       ├── id.rs                            # UlidFactory, iri()
│   │       ├── time.rs                          # Timestamp (HiRes)
│   │       ├── error.rs                         # MmError
│   │       ├── config.rs                        # Config
│   │       ├── hash.rs                          # content_hash()
│   │       └── store.rs                         # Tabular/Graph/EventSink traits
│   ├── mm-log/
│   │   ├── Cargo.toml
│   │   └── src/{lib.rs, record.rs, sinks.rs, redact.rs, codes.rs}
│   ├── mm-store-sqlite/
│   │   ├── Cargo.toml
│   │   ├── migrations/0001_kernel.sql
│   │   └── src/{lib.rs, pool.rs, tabular.rs}
│   ├── mm-store-graph/
│   │   ├── Cargo.toml
│   │   └── src/{lib.rs, actor.rs, graphs.rs, shacl.rs}
│   ├── mm-eventlog/
│   │   ├── Cargo.toml
│   │   └── src/{lib.rs, record.rs, log.rs, replay.rs}
│   └── mm-cli/
│       ├── Cargo.toml
│       └── src/{main.rs, doctor.rs, graph_cmd.rs, replay_cmd.rs, logs_cmd.rs}
├── modules/
│   └── system/kernel-bootstrap/{plugin.toml, src/lib.rs, manual/module.md, tests/smoke.rs}
├── ontology/
│   ├── mm.ttl                                   # T-Box v0 (design §87)
│   ├── code.ttl                                 # mmc: namespace stub (filled in Phase 2)
│   └── shapes/{mm-shapes.ttl}
└── tests/e2e/{kernel_boot.rs, replay_determinism.rs, cli_doctor.rs}
```

---

## 4. Data model and ontology deltas

### 4.1 SQLite (migration `0001_kernel.sql`)

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE events (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT(26) NOT NULL UNIQUE,          -- ULID
    kind         TEXT NOT NULL,
    payload      TEXT NOT NULL,                     -- canonical JSON
    status       TEXT NOT NULL CHECK (status IN ('provisional','committed','aborted')),
    correlation  TEXT(26),                          -- trace_id ULID
    created_at   TEXT NOT NULL,                     -- RFC3339 ns
    hash         TEXT NOT NULL                      -- sha256(kind|payload|created_at)
);
CREATE INDEX events_status_seq ON events(status, seq);
CREATE UNIQUE INDEX events_hash ON events(hash);

CREATE TABLE audit_log (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    record_id    TEXT(26) NOT NULL UNIQUE,          -- ULID of the audit record
    event_code   TEXT NOT NULL,                     -- e.g. 'eventlog.commit'
    level        TEXT NOT NULL,
    target       TEXT NOT NULL,
    trace_id     TEXT(26),
    payload      TEXT NOT NULL,
    prev_hash    TEXT,
    hash         TEXT NOT NULL                     -- chained hash
);
CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;

CREATE TABLE store_checkpoints (
    seq          INTEGER PRIMARY KEY,               -- last applied event seq
    state_hash   TEXT NOT NULL,
    created_at   TEXT NOT NULL
);

-- forward scaffolding only; populated by later phases
CREATE TABLE schema_versions (name TEXT PRIMARY KEY, version INTEGER NOT NULL, applied_at TEXT NOT NULL);
```

### 4.2 RDF / ontology

- Namespaces: `mm:` = `https://metamind.dev/ontology#`, `mmc:` = `https://metamind.dev/code#`,
  `mmd:` = `https://metamind.dev/data/`, `sh:` = `http://www.w3.org/ns/shacl#`,
  `prov:` = `http://www.w3.org/ns/prov#`.
- Named graphs: `https://metamind.dev/graph/{being,memory,epistemic,library,world,provenance,code}`.
- Instance IRIs: `https://metamind.dev/data/{ulid}` (lowercase Crockford).
- `ontology/mm.ttl` declares the T-Box v0 classes from design §87 (`Being, Identity, User,
  Relationship, Goal, Commitment, Memory*, Belief, Claim, Evidence, Observation, Assumption, Inference,
  Prediction, Decision, Action, Outcome, Policy, Doctrine, Principle, Heuristic, Pattern, Case,
  AntiPattern, Capability, Experiment, Evaluation, Failure, NearMiss, EvolutionEvent, Frame, Technique,
  Skill`) and relations (`dependsOn, supports, contradicts, derivedFrom, observedIn, caused, predicts,
  verifiedBy, similarTo, analogousTo, applicableWhen, inapplicableWhen, implementedBy, requires,
  improves, replaces, testedBy, triggeredBy`). Only `mm:Being` and `mm:Identity` receive instance data
  before Phase 4.
- SHACL shapes are **generated** from `mm.ttl` with `rdf-shacl::check_ontology` →
  `generate_shacl` → `validate`; hand-written shapes in `ontology/shapes/mm-shapes.ttl` cover the
  kernel's structural constraints (every `mm:Identity` must have exactly one stable IRI and a creation
  timestamp).

---

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

### 5.1 `mm-core`

```rust
pub struct UlidFactory { /* monotonic within a process; persisted high-water mark */ }
impl UlidFactory {
    pub fn new() -> Self;
    pub fn open(path: &Path) -> Result<Self, MmError>;  // restores last ULID across restarts
    pub fn next(&self) -> Ulid;
}

pub mod iri {
    pub const MM:  &str = "https://metamind.dev/ontology#";
    pub const MMC: &str = "https://metamind.dev/code#";
    pub const DATA:&str = "https://metamind.dev/data/";
    pub fn data(u: &Ulid) -> NamedNode;              // https://metamind.dev/data/{ulid}
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp { pub seconds: u64, pub nanos: u32 }
impl Timestamp { pub fn now() -> Self; pub fn to_rfc3339(&self) -> String; }

#[derive(Debug, thiserror::Error)]
pub enum MmError {
    #[error("store error: {0}")]   Store(String),
    #[error("graph error: {0}")]   Graph(String),
    #[error("event error: {0}")]   Event(String),
    #[error("config error: {0}")]  Config(String),
    #[error("codec error: {0}")]   Codec(String),
    #[error("internal error: {0}")] Internal(String),
}

pub fn content_hash(bytes: &[u8]) -> String;         // sha256 hex, used for content-addressed artifacts

#[async_trait::async_trait]
pub trait Tabular: Send + Sync {
    async fn execute(&self, sql: &str) -> Result<u64, MmError>;
    async fn query_json(&self, sql: &str) -> Result<Vec<serde_json::Value>, MmError>;
}

#[async_trait::async_trait]
pub trait Graph: Send + Sync {
    async fn insert(&self, graph: &str, quad: Quad) -> Result<(), MmError>;
    async fn sparql(&self, graph: &str, query: &str) -> Result<Vec<serde_json::Value>, MmError>;
    async fn validate(&self, graph: &str) -> Result<ShaclReport, MmError>;
}

#[async_trait::async_trait]
pub trait EventSink: Send + Sync {
    async fn append(&self, e: NewEvent) -> Result<Ulid, MmError>;
    async fn commit(&self, id: &Ulid) -> Result<(), MmError>;
}
```

### 5.2 `mm-log`

```rust
pub struct LogRecord {          // serialized one-per-line to data/logs/*.jsonl
    pub ts: String, pub level: String, pub target: String, pub event: String,
    pub msg: String, pub trace_id: Option<String>, pub span_id: Option<String>,
    pub result: Option<String>, pub latency_ms: Option<u64>,
    pub fields: serde_json::Map<String, serde_json::Value>,
}
pub fn init(cfg: &Config) -> LogGuard;                // console + JSONL + audit + provenance layers
pub fn audit(record: LogRecord) -> Result<(), MmError>; // transactional audit sink
pub struct Redactor;                                   // strips keys/tokens/passwords/PII before any sink
```

### 5.3 `mm-store-graph` actor

```rust
pub struct GraphHandle { tx: tokio::sync::mpsc::Sender<GraphCmd> }
pub fn spawn_blocking_actor(cfg: &Config) -> Result<GraphHandle, MmError>;
enum GraphCmd {
    Insert { graph: String, quad: Quad, ack: oneshot::Sender<Result<(), MmError>> },
    Sparql { graph: String, query: String, ack: oneshot::Sender<Result<Vec<serde_json::Value>, MmError>> },
    Validate { graph: String, ack: oneshot::Sender<Result<ShaclReport, MmError>> },
    Flush { ack: oneshot::Sender<Result<(), MmError>> },
}
```

### 5.4 `mm-eventlog`

```rust
pub struct NewEvent { pub kind: EventKind, pub payload: serde_json::Value, pub correlation: Option<Ulid> }
#[derive(Clone, Copy)] pub enum EventKind { KernelBoot, StoreMutation, OntologyLoad, Checkpoint, Custom }
pub trait EventApplier { fn apply(&mut self, r: &EventRecord) -> Result<(), MmError>; }
pub struct EventLog { /* SqlitePool + graph handle */ }
impl EventLog {
    pub async fn append(&self, e: NewEvent) -> Result<Ulid, MmError>;   // provisional
    pub async fn commit(&self, id: &Ulid) -> Result<(), MmError>;       // committed
    pub async fn replay(&self, until: Option<u64>, a: &mut dyn EventApplier) -> Result<StateSnapshot, MmError>;
}
```

### 5.5 `mm-cli`

| Command | Behaviour |
|---|---|
| `mm-cli doctor` | open config; open SQLite + Oxigraph; load ontology; print versions + triple count; append a `KernelBoot` event; exit non-zero on any failure |
| `mm-cli replay --from <seq> [--until <seq>]` | replay the event log through an `EventApplier`, verifying the terminal `state_hash` |
| `mm-cli graph validate --graph <name>` | SHACL-validate a named graph; exit non-zero on violations |
| `mm-cli logs verify` | validate log schema, audit completeness, gapless audit chain, redaction suite |
| `mm-cli logs tail -n <k>` / `mm-cli logs trace <ulid>` | inspect operational logs |

### 5.6 Bootstrap module `plugin.toml`

```toml
[plugin]
name = "mm-kernel-bootstrap"
uri = "https://metamind.dev/code/module/system/kernel-bootstrap"
version = "0.1.0"
description = "Kernel self-check module; proves the module contract loads."
[metadata]
category = "system"
owned_by_phase = 1
capability = "mm:KernelSelfCheck"
[tbox.functions]
"kernel.self_check" = { source = "handlers::self_check" }
[build]
rust_edition = "2021"
```

---

## 6. External references copied in and integration

Per parent plan §7, external code is **reference-only**; the needed parts are copied into `vendor/` and
integrated as Metamind-owned source. Phase 1 copies exactly three things:

| Reference | Copied from | What is copied | Integration |
|---|---|---|---|
| `rdf-codec` | `rust_symbolic/crates/rdf-codec` | `RdfContext`, `Namespace`, `ToRdf`/`FromRdf`, `new_ulid`, `ulid_from_content`, canonical serializer | `vendor/rust_symbolic/rdf-codec/`, re-exported through `mm-core::iri` and `mm-store-graph`; `mm-*` types implement `ToRdf`/`FromRdf`, never a reference's types |
| `temporal-store` semantics | `rust_symbolic/crates/temporal-store` | Immutable record-version + monotonic-sequence + replay contract (semantics, not the Fjall/redb physical layer) | Reimplemented on SQLite in `mm-eventlog`; the reference is the contract test oracle |
| `nexus-core::logging` conventions | `nexus/nexus-core/src/logging.rs` | Structured logger shape (level, target, integer timestamps, no floats) | Reimplemented in `mm-log` on `tracing`; reference used as the behavior spec |

Each copied tree gets `vendor/<origin>/COPYING.md` recording origin repo, revision, and license
(`grep -n` provenance is emitted into the code graph in Phase 2 as `mmc:copiedFrom`). No git submodules,
no external path dependencies, no `[patch]` on unowned code.

---

## 7. Step-by-step implementation tasks

1. **Workspace scaffold.** Create `Cargo.toml` (`resolver = "2"`, members `crates/*`, `modules/*/*`),
   `rust-toolchain.toml` (`1.96.1`), `rustfmt.toml`, `clippy.toml`, and `.github/workflows/ci.yml`
   (`build`, `clippy --all-targets -- -D warnings`, `test`, plus the §10 gate commands). Add
   `[workspace.lints.rust] unsafe_code = "forbid"` and apply `lints.workspace = true` in every crate.
2. **`mm-core`.** Implement `id.rs` (`UlidFactory` with a persisted high-water mark file for monotonicity
   across restarts), `time.rs` (`Timestamp`), `error.rs` (`MmError`), `config.rs` (`Config` from
   `MM_*` env + `metamind.toml`), `hash.rs` (`content_hash`), `store.rs` (the three async traits).
   Deliberately no I/O beyond the ULID watermark file.
3. **`mm-log`.** Implement `record.rs` (the `LogRecord` schema), `codes.rs` (`pub const` event codes),
   `sinks.rs` (console layer, rotating `data/logs/YYYY-MM-DD.jsonl` layer, audit layer that writes to
   `audit_log` on the same SQLite transaction as its event), `redact.rs` (pattern-based `Redactor`).
   `init()` returns a `LogGuard` that flushes on drop.
4. **`mm-store-sqlite`.** Implement `pool.rs` (`SqlitePoolOptions`, `PRAGMA journal_mode=WAL;
   foreign_keys=ON`, `busy_timeout`), `0001_kernel.sql`, `tabular.rs` (`Tabular` impl), and a
   `migrate()` called on open (`sqlx::migrate!(".../migrations")`). Pin the SQLite file at
   `data/metamind.db`.
5. **`mm-store-graph`.** Implement `actor.rs` (owned `Store` on one blocking thread with an `mpsc`
   mailbox and a bounded read handle), `graphs.rs` (named-graph registry + `rdf-codec` `RdfContext`),
   `shacl.rs` (call `rdf-shacl::validate`). Store path `data/graph/`. Writes are serialized; every
   write is a `graph_tx` span.
6. **`mm-eventlog`.** Implement `record.rs` (event row + canonical JSON), `log.rs` (append→apply→commit
   with the same-transaction audit write), `replay.rs` (ordered replay, checkpoint write, `state_hash`
   comparison). Crash-injection path: a provisional event with no commit is aborted on replay start.
7. **Ontology.** Author `ontology/mm.ttl` (T-Box v0), `ontology/code.ttl` (namespace stub), and
   `ontology/shapes/mm-shapes.ttl`; generate SHACL from the ontology with `rdf-shacl`; `mm-cli doctor`
   loads `mm.ttl`, and `graph validate` applies shapes.
8. **`mm-cli`.** Implement `doctor.rs`, `graph_cmd.rs`, `replay_cmd.rs`, `logs_cmd.rs`, and the
   subcommand wiring in `main.rs`.
9. **Bootstrap module.** Add `modules/system/kernel-bootstrap/` with `plugin.toml`, a trivial
   `kernel.self_check` handler, `manual/module.md`, and a smoke test; generate
   `modules/registry.json` (full scanner lands in Phase 2).
10. **CI + docs.** Wire the §10 gates into CI and add `crates/mm-core/README.md` describing the kernel
    contracts.

---

## 8. Detailed logging requirements

All records carry the global fields from parent plan §10. Phase 1 required event codes:

| Event code | Level | Required fields |
|---|---|---|
| `log.init` | INFO | sinks, level, log_dir, redaction=on |
| `log.redact` | WARN | pattern_id, target, field (never the secret value) |
| `store.sql.open` | INFO | path, journal_mode, pool_size |
| `store.sql.migrate` | INFO | migration, direction, duration_ms |
| `store.graph.open` | INFO | path, backend=`oxigraph-rocksdb`, triple_count |
| `store.graph.tx` | DEBUG | graph, quads, duration_ms |
| `store.graph.sparql` | DEBUG | graph, query_hash, rows, duration_ms |
| `eventlog.append` | DEBUG | id, kind, correlation, seq |
| `eventlog.apply` | DEBUG | id, seq, applier |
| `eventlog.commit` | INFO | id, seq, state_hash |
| `eventlog.abort` | WARN | id, reason |
| `eventlog.replay.start` | INFO | from_seq, until_seq, source |
| `eventlog.replay.end` | INFO | events_applied, state_hash, expected_hash, match |
| `ontology.load` | INFO | file, triples, classes |
| `shacl.validate` | INFO | graph, shapes, conforms, violations |
| `audit.sequence.check` | ERROR | first_gap_seq, expected, found |
| `kernel.boot` | INFO | version, stores_ready |

Audit records (immutable) are emitted for `eventlog.commit`, `eventlog.abort`, `ontology.load`,
`kernel.boot`, and every migration. A committed state change without its audit record must be
impossible by construction (same transaction).

---

## 9. Testing plan

| Type | Test | Assertion |
|---|---|---|
| Unit | `mm-core` id/time/error/config/hash | round-trips; `Timestamp::to_rfc3339` fixed vectors |
| Property (`proptest`) | ULID uniqueness + monotonicity | 100,000 IDs all unique and strictly increasing |
| Property | RDF canonical round-trip | 10,000 triples encode→serialize→re-parse → identical `content_hash` |
| Property | event replay | random mutation sequence replayed ⇒ identical `state_hash` |
| Golden (`insta`) | `LogRecord` JSON schema | snapshots for each event code |
| Integration | crash injection | provisional event without commit ⇒ abort on replay; no double-apply |
| Integration | migrations | up then down leaves the schema empty and re-up is identical |
| Conformance | SHACL | clean instance graph ⇒ 0 violations; deliberately broken ⇒ ≥1 |
| E2E | `mm-cli doctor` | exit 0; appends `KernelBoot`; prints store versions |
| E2E | `mm-cli logs verify` | schema valid, audit chain gapless, redaction suite clean |
| Redaction | secret fixtures | known key/token patterns never appear in JSONL, console, audit, or error paths |

---

## 10. Pass gate

Run as plain commands; earlier phases don't exist yet, so this gate stands alone.

```bash
cd ~/Build/metamind
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p mm-cli -- doctor
cargo run -p mm-cli -- graph validate --graph being
cargo run -p mm-cli -- logs verify
cargo run -p mm-cli -- replay --from 0
```

Objective criteria — **all** must hold:
1. Build, clippy, and tests exit 0 with **zero warnings**; `unsafe_code` forbidden workspace-wide.
2. `mm-cli doctor` exits 0, reports SQLite and Oxigraph versions, the ontology triple count, and marks
   both stores writable; it appends exactly one `KernelBoot` event.
3. Property tests: 100k ULIDs unique/monotonic; 10k-triple graph export→reimport yields an identical
   content hash; random-mutation replay yields an identical `state_hash`.
4. Crash-injection test: a killed writer replays with **no double-apply**.
5. `mm-cli graph validate --graph being` reports 0 SHACL violations for a clean graph and ≥1 for a
   deliberately broken one.
6. `mm-cli logs verify` passes: every record schema-valid, every committed event has exactly one audit
   record, the audit chain is gapless, and the redaction suite finds no secret.
7. `mm-cli replay --from 0` reproduces the state hash recorded at commit.

---

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Oxigraph `Store` is synchronous and would block the async runtime | Single-writer actor on a blocking thread with an `mpsc` mailbox and bounded read handles; no `Store` handle leaves the actor |
| RocksDB `pkg-config` build failure | Enforce the `rocksdb-pkg-config` feature workspace-wide; `mm-cli doctor` fails fast with a clear message; CI installs `librocksdb-dev` |
| SQLite WAL on a network filesystem risks corruption | Keep `data/` on local disk; document it; `doctor` warns if the path is not local |
| ULID monotonicity lost across process restarts | Persist a high-water mark in `mm-core`; a test restarts the factory and asserts ordering |
| Event log and store state diverge on crash | Append→apply→commit with same-transaction audit; replay aborts provisional events |
| Schema/ontology drift between code and RDF | SHACL generated from the ontology in-tree; `graph validate` is a gate; Phase 2 adds drift detection |
| Audit log tampering | `BEFORE UPDATE`/`BEFORE DELETE` triggers raise `ABORT`; chained `hash` detects rewrites |
| Over-building the kernel | Enforce design §108: no cognition, no LLM, no personality in Phase 1 |

---

## 12. Design traceability

| Source | Section |
|---|---|
| `digital_mind_design1.md` | §92 (Bootstrap Phase 0: deterministic kernel — identity, state storage, event log, versioning, permissions, tool registry, resource accounting, execution engine, test harness); §87 (RDF/OWL data model); §88 (Rust runtime model); §51 (world model storage); §108 (what not to overbuild); §109–110 (invariants, final system) |
| `bootstrap_procses1.md` | §3 (start with a tiny kernel), §11 (bootstrapping guardrails: immutable kernel / stable core / adaptive policies) |
| `bootstrap_procses2.md` | Unified self-improvement framing; "Behavior = Code × Data × Configuration × Model × Environment" (defines what later phases version) |
| `state_structure1.md` | Minimal persistent state; the principle that only behaviorally material state is stored; verified-state vs generated-state boundary |
| Parent plan | §3 workspace layout, §4 identifiers, §5 storage topology, §6 ontology, §7 copy-in integration, §8 module contract, §9 testing, §10 logging, §13 Phase 1 |

**Phase 1 exit condition:** the kernel is deterministic, replayable, logged, ontology-validated, and
green on its gate — the substrate Phases 2–12 build on.
