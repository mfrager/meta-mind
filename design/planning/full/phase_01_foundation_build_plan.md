# Phase 1 — Foundation (Extended Build Plan)

> Extended from: `design/planning/phase_01_foundation_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 1 · Codename Metamind (`mm`)

This is the research-grounded, streamlined revision. It keeps the same scope and gate as the Phase 1
plan but locks in the specific mechanisms — bitemporal event log, canonical RDF, actor-based graph store,
and an immutable audit chain — that the rest of the system depends on.

---

## 0. Research foundation (code-available)

Every idea below is borrowed from a project whose source is available. Only ideas with code are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| Embedded RDF/SPARQL store | Oxigraph — https://github.com/oxigraph/oxigraph | RDF 1.2 + SPARQL 1.1, named graphs, RocksDB backend, `Store::bulk_loader`, synchronous `Store` API | `mm-store-graph` wraps Oxigraph (RocksDB) behind an async actor; data lives in `data/graph/` |
| Async SQLite + migrations | sqlx — https://github.com/launchbadge/sqlx | Async pool, `PRAGMA journal_mode=WAL`, `sqlx::migrate!`, optional compile-time checked queries | `mm-store-sqlite` owns `data/metamind.db`; migrations are phase-numbered under `crates/mm-store-sqlite/migrations/` |
| Bitemporal, immutable facts | XTDB — https://github.com/xtdb/xtdb | Two time axes (system time vs valid time), immutable facts, as-of / time-travel reads | Event log and every tabular row carry `system_from/system_to` + `valid_from/valid_to`; reads are `as_of(system, valid)` |
| Structured logs + spans | tracing — https://github.com/tokio-rs/tracing | Span/field model, JSON layer, per-target filtering | `mm-log` emits one JSON object per record; spans carry the correlation ULID |
| Trace correlation | tracing-opentelemetry — https://github.com/tokio-rs/tracing-opentelemetry | OTel-compatible context propagation | `trace_id` is a ULID shared by logs, event log, and RDF provenance |
| Event sourcing core | `event-sourcing` — https://crates.io/crates/event-sourcing · `esrc` — https://docs.rs/esrc · `eventually` — https://github.com/eventually-rs/eventually-rs · `cqrs-es` example — https://github.com/j5ik2o/cqrs-es-example-rs | Aggregate/event-store split, append-then-apply, snapshotting, projections, idempotent replay | `mm-eventlog` uses append→apply→commit, stores checkpoints (`state_hash`), and dedupes by event hash |
| Monotonic ULIDs | ulid-rs — https://github.com/dylanhart/ulid-rs | Monotonic generator within a millisecond; lexicographically sortable IDs | `mm-core::UlidFactory` persists a high-water mark so ordering survives restarts |
| LSM + atomic batches | RocksDB — https://github.com/facebook/rocksdb | WriteBatch atomicity, point-in-time snapshots, prefix iteration | Oxigraph's RocksDB backend gives atomic `bulk_loader`/batch writes; `mm-store-graph` flushes per transaction |

---

## 1. Objective and scope

Build the deterministic, non-cognitive kernel: a compiling Rust workspace with ULID identity, a bitemporal
append-only event log with deterministic replay, **SQLite** (tabular) and **Oxigraph** (RDF) behind one
async abstraction, `ontology v0`, structured logging, and the test harness every later phase is gated on.
This is design §92 ("Bootstrap Phase 0"), with PostgreSQL replaced by SQLite.

The one invariant everything must uphold from here on:

> **Deterministic systems enforce what must be enforced; the environment decides what is true.**
> Nothing in Phase 1 may be probabilistic, and no committed state may exist without its audit record.

**In scope:** workspace + toolchain + CI; `mm-core` (IDs, time, errors, config, store traits, hashing);
`mm-log`; `mm-store-sqlite`; `mm-store-graph`; `mm-eventlog`; `ontology/mm.ttl` v0 + SHACL;
`mm-cli {doctor,replay,graph validate,logs verify}`; one bootstrap module.

**Out of scope (later phases):** any LLM call, memory, being state, cognition, tool execution, or
self-modification; content in forward tables (only their migration scaffolding exists).

**Definition of done:** the §8 pass gate runs green as plain commands.

---

## 2. Architecture and invariants

Three stores with disjoint authority; no fact is authoritative in two places. The **event log is the
source of truth**; SQLite and Oxigraph are projections that are replayable from it.

```
                      mm-core  (ULID · Timestamp · MmError · Config · traits · hashing)
                                        │
        ┌───────────────────────────────┼────────────────────────────────┐
        ▼                               ▼                                ▼
  mm-store-sqlite                 mm-eventlog                     mm-store-graph
  (sqlx, WAL)                     (append→apply→commit,            (Oxigraph actor)
  tabular projections             bitemporal, replay)              RDF projections
        │                               │                                │
        └──────────── owner: mm-store-* ─┴────── shared: /provenance ─────┘
                                        │
                                    mm-log  →  console · JSONL · audit · /provenance · SQLite
                                        │
                                     mm-cli  (doctor · replay · graph validate · logs verify)
```

**Invariants**

1. **Single-writer graph.** Oxigraph's `Store` is synchronous; all writes funnel through one actor task.
   Reads use a separate `Arc<Store>` handle on `spawn_blocking` (Oxigraph reads are thread-safe).
2. **Append→apply→commit.** An event is appended `provisional`, the projection is mutated, then the event
   is marked `committed` — all in one SQLite transaction, with its audit record in the same transaction.
   On replay start, any `provisional` event is `aborted` (exactly-once, no double-apply).
3. **Bitemporal facts (XTDB model).** Domain facts carry *valid time* (`valid_from`, `valid_to`); the log
   carries *system time* (`seq` + `system_from`). Every read is expressed as `as_of(system_time,
   valid_time)`; "now" is the default for both.
4. **Canonical RDF.** All RDF goes through `rdf-codec`: deterministic serialization means re-parsing
   yields an identical content hash — this is what makes replay and diffing possible.
5. **Audit is append-only.** `audit_log` has `BEFORE UPDATE/DELETE` triggers that `RAISE(ABORT)`, and a
   chained hash so a rewrite is detectable.
6. **Zero warnings, no `unsafe`.** `#![forbid(unsafe_code)]` in every `mm-*` crate.

---

## 3. Deliverables and workspace layout

```
~/Build/metamind/
├── Cargo.toml                      # [workspace] resolver=2, members = crates/* + modules/*/*
├── rust-toolchain.toml             # channel = "1.96.1"
├── rustfmt.toml · clippy.toml · .cargo/config.toml
├── .github/workflows/ci.yml        # build · clippy -D warnings · test · §8 gates
├── crates/
│   ├── mm-core/src/{lib,id,time,error,config,hash,store}.rs
│   ├── mm-log/src/{lib,record,codes,sinks,redact}.rs
│   ├── mm-store-sqlite/{migrations/0001_kernel.sql, src/{lib,pool,tabular,bitemporal}.rs}
│   ├── mm-store-graph/src/{lib,actor,graphs,shacl}.rs
│   ├── mm-eventlog/src/{lib,record,log,replay}.rs
│   └── mm-cli/src/{main,doctor,graph_cmd,replay_cmd,logs_cmd}.rs
├── modules/system/kernel-bootstrap/{plugin.toml, src/lib.rs, manual/module.md, tests/smoke.rs}
├── modules/registry.json
├── ontology/{mm.ttl, code.ttl, shapes/mm-shapes.ttl}
├── config/metamind.toml
├── data/{metamind.db, graph/, logs/}     # gitignored
└── tests/e2e/{kernel_boot.rs, replay_determinism.rs, cli_doctor.rs}
```

---

## 4. Detailed specifications

### 4.1 `mm-core`

```rust
pub struct UlidFactory { /* last: Mutex<Ulid>, watermark: PathBuf */ }
impl UlidFactory {
    pub fn new() -> Self;
    pub fn open(watermark: &Path) -> Result<Self, MmError>; // restores across restarts
    pub fn next(&self) -> Ulid;                             // monotonic within a ms
}

pub mod iri {
    pub const MM:   &str = "https://metamind.dev/ontology#";
    pub const MMC:  &str = "https://metamind.dev/code#";
    pub const DATA: &str = "https://metamind.dev/data/";
    pub fn data(u: &Ulid) -> NamedNode;                     // https://metamind.dev/data/{ulid}
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp { pub seconds: u64, pub nanos: u32 }
impl Timestamp { pub fn now() -> Self; pub fn to_rfc3339(&self) -> String; }

#[derive(Debug, thiserror::Error)]
pub enum MmError { Store(String), Graph(String), Event(String), Config(String),
                   Codec(String), Internal(String) }

pub fn content_hash(bytes: &[u8]) -> String;                // sha256 hex

#[async_trait::async_trait] pub trait Tabular: Send + Sync {
    async fn execute(&self, sql: &str, args: Params) -> Result<u64, MmError>;
    async fn query_json(&self, sql: &str, args: Params) -> Result<Vec<serde_json::Value>, MmError>;
}
#[async_trait::async_trait] pub trait Graph: Send + Sync {
    async fn insert(&self, graph: &str, quad: Quad) -> Result<(), MmError>;
    async fn sparql(&self, graph: &str, q: &str) -> Result<Vec<serde_json::Value>, MmError>;
    async fn validate(&self, graph: &str) -> Result<ShaclReport, MmError>;
}
#[async_trait::async_trait] pub trait EventSink: Send + Sync {
    async fn append(&self, e: NewEvent) -> Result<Ulid, MmError>;
    async fn commit(&self, id: &Ulid) -> Result<(), MmError>;
}
```

### 4.2 `mm-store-sqlite` and bitemporal model

`pool.rs` opens `data/metamind.db` with `SqlitePoolOptions`, then runs
`PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;` and `sqlx::migrate!`.
`bitemporal.rs` implements the XTDB-style read contract:

```rust
/// A row is visible iff it was committed at or before `sys` and its valid interval covers `valid`.
pub struct AsOf { pub system: Option<u64>, pub valid: Option<Timestamp> }
impl AsOf { pub fn now() -> Self; pub fn at(seq: u64, t: Timestamp) -> Self; }
// SQL shape: WHERE system_from <= :sys AND (system_to IS NULL OR system_to > :sys)
//                AND valid_from <= :valid AND (valid_to IS NULL OR valid_to > :valid)
```

Migration `0001_kernel.sql`:

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE events (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT(26) NOT NULL UNIQUE,               -- ULID
    kind         TEXT NOT NULL,
    payload      TEXT NOT NULL,                          -- canonical JSON
    status       TEXT NOT NULL CHECK (status IN ('provisional','committed','aborted')),
    correlation  TEXT(26),                               -- trace_id ULID
    system_from  TEXT NOT NULL,                          -- commit time (RFC3339 ns)
    created_at   TEXT NOT NULL,
    hash         TEXT NOT NULL                           -- sha256(kind|payload|created_at)
);
CREATE INDEX   events_status_seq ON events(status, seq);
CREATE UNIQUE INDEX events_hash  ON events(hash);        -- idempotent append

CREATE TABLE audit_log (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    record_id  TEXT(26) NOT NULL UNIQUE,
    event_code TEXT NOT NULL,
    level      TEXT NOT NULL,
    target     TEXT NOT NULL,
    trace_id   TEXT(26),
    payload    TEXT NOT NULL,
    prev_hash  TEXT,
    hash       TEXT NOT NULL                             -- chained
);
CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;

CREATE TABLE store_checkpoints (                          -- snapshotting (event-sourcing pattern)
    seq        INTEGER PRIMARY KEY,                       -- last applied event seq
    state_hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- bitemporal example table; later phases clone this shape for their domain tables
CREATE TABLE facts (
    id         TEXT(26) PRIMARY KEY,
    subject    TEXT NOT NULL, predicate TEXT NOT NULL, object TEXT NOT NULL,
    system_from TEXT NOT NULL, system_to TEXT,
    valid_from  TEXT NOT NULL, valid_to   TEXT,
    provenance  TEXT,
    trace_id    TEXT(26)
);
CREATE INDEX facts_asof ON facts(subject, predicate, system_from, valid_from);

-- forward scaffolding only
CREATE TABLE schema_versions (name TEXT PRIMARY KEY, version INTEGER NOT NULL, applied_at TEXT NOT NULL);
```

### 4.3 `mm-store-graph` actor

```rust
pub struct GraphHandle { tx: tokio::sync::mpsc::Sender<GraphCmd> }
pub fn open(cfg: &Config) -> Result<(GraphHandle, Arc<Store>), MmError>; // actor task + read handle

enum GraphCmd {
    Insert   { graph: String, quad: Quad,        ack: oneshot::Sender<Result<(), MmError>> },
    Sparql   { graph: String, query: String,     ack: oneshot::Sender<Result<Vec<serde_json::Value>, MmError>> },
    Validate { graph: String,                    ack: oneshot::Sender<Result<ShaclReport, MmError>> },
    Flush    {                                   ack: oneshot::Sender<Result<(), MmError>> },
}
```

**Actor contract.** One task owns the write path; the mailbox is bounded (`1024`) so producers await and
apply natural backpressure. Open the store with `Store::open(data/graph/)"` and load the ontology with
`bulk_loader` (single atomic batch). Writes are grouped per event transaction and end with `Flush`.
Reads take the `Arc<Store>` handle and run on `spawn_blocking`; they never take the mailbox. Named graphs:
`https://metamind.dev/graph/{being,memory,epistemic,library,world,provenance,code}`.

### 4.4 `mm-eventlog`

```rust
pub struct NewEvent { pub kind: EventKind, pub payload: serde_json::Value, pub correlation: Option<Ulid> }
#[derive(Clone, Copy)] pub enum EventKind { KernelBoot, StoreMutation, OntologyLoad, Checkpoint, Custom }
pub trait EventApplier { fn apply(&mut self, r: &EventRecord) -> Result<(), MmError>; }

impl EventLog {
    /// Insert `provisional` + its audit record in one transaction. Idempotent by `events.hash`.
    pub async fn append(&self, e: NewEvent) -> Result<Ulid, MmError>;
    /// Mark `committed`, write checkpoint when requested.
    pub async fn commit(&self, id: &Ulid) -> Result<(), MmError>;
    /// Abort any leftover `provisional` events from a crash, then replay in `seq` order.
    pub async fn replay(&self, as_of: AsOf, a: &mut dyn EventApplier)
        -> Result<StateSnapshot, MmError>;
}
```

### 4.5 Ontology v0

- `ontology/mm.ttl` — T-Box v0: the design §87 classes (`Being, Identity, User, Relationship, Goal,
  Commitment, Memory*, Belief, Claim, Evidence, Observation, Assumption, Inference, Prediction, Decision,
  Action, Outcome, Policy, Doctrine, Principle, Heuristic, Pattern, Case, AntiPattern, Capability,
  Experiment, Evaluation, Failure, NearMiss, EvolutionEvent, Frame, Technique, Skill`) and relations
  (`dependsOn, supports, contradicts, derivedFrom, observedIn, caused, predicts, verifiedBy, similarTo,
  analogousTo, applicableWhen, inapplicableWhen, implementedBy, requires, improves, replaces, testedBy,
  triggeredBy`). Only `mm:Being`/`mm:Identity` receive data before Phase 4.
- Namespaces: `mm:` `https://metamind.dev/ontology#`, `mmc:` `https://metamind.dev/code#`,
  `mmd:` `https://metamind.dev/data/`, `sh:` `http://www.w3.org/ns/shacl#`, `prov:` `http://www.w3.org/ns/prov#`.
- SHACL is generated with `rdf-shacl::check_ontology → generate_shacl → validate`; hand-written
  `ontology/shapes/mm-shapes.ttl` covers kernel constraints (an `mm:Identity` has exactly one stable IRI
  and one creation timestamp).
- `ontology/code.ttl` is a namespace stub filled in Phase 2.

### 4.6 `mm-cli`

| Command | Behaviour |
|---|---|
| `mm-cli doctor` | open config + both stores; load ontology; print versions/triple count; append one `KernelBoot`; exit non-zero on any failure |
| `mm-cli replay --from <seq> [--until <seq>] [--as-of <ts>]` | replay through an `EventApplier`, verifying the terminal `state_hash` |
| `mm-cli graph validate --graph <name>` | SHACL-validate a named graph; non-zero on violations |
| `mm-cli logs verify` | schema, audit completeness, gapless audit chain, redaction suite |
| `mm-cli logs tail -n <k>` / `logs trace <ulid>` | inspect operational logs |

---

## 5. Build sequence

1. **Scaffold.** Workspace (`resolver=2`), toolchain pin, lints (`unsafe_code = "forbid"`), CI skeleton.
   *Check:* `cargo build --workspace` on an empty workspace.
2. **`mm-core`.** `id`, `time`, `error`, `config`, `hash`, `store` traits. *Check:* unit tests + ULID property test.
3. **`mm-log`.** `record`, `codes`, `sinks` (console/JSONL/audit), `redact`. *Check:* golden `LogRecord` snapshots.
4. **`mm-store-sqlite`.** pool + `0001_kernel.sql` + `Tabular` + `bitemporal::AsOf`. *Check:* up/down migration test; `as_of` fixture.
5. **`mm-store-graph`.** actor, named-graph registry, `shacl`. *Check:* bulk-load ontology, triple count, atomic flush.
6. **`mm-eventlog`.** append/commit/replay + checkpoint + crash-injection abort. *Check:* replay determinism test.
7. **Ontology.** Author `mm.ttl`/`code.ttl`/shapes; generate SHACL. *Check:* `graph validate` clean vs broken.
8. **`mm-cli`.** Wire doctor/replay/graph/logs. *Check:* `doctor` exits 0 and appends one event.
9. **Bootstrap module.** `modules/system/kernel-bootstrap` + `registry.json`. *Check:* module smoke test.
10. **CI + docs.** Wire §8 gates into CI. *Check:* CI green with zero warnings.

---

## 6. Logging and observability

Every record carries the global fields (parent plan §10) and one JSON object per line.

| Event code | Level | Required fields |
|---|---|---|
| `log.init` / `log.redact` | INFO / WARN | sinks, level, log_dir / pattern_id, target, field |
| `store.sql.open` / `store.sql.migrate` | INFO | path, journal_mode, pool_size / migration, direction, duration_ms |
| `store.graph.open` / `store.graph.tx` / `store.graph.sparql` | INFO / DEBUG | path, backend, triple_count / graph, quads, duration_ms / graph, query_hash, rows |
| `eventlog.append` / `apply` / `commit` / `abort` | DEBUG / INFO / WARN | id, kind, correlation, seq / id, seq, applier / id, seq, state_hash / id, reason |
| `eventlog.replay.start` / `end` | INFO | from_seq, until_seq, source / events_applied, state_hash, expected_hash, match |
| `ontology.load` / `shacl.validate` | INFO | file, triples, classes / graph, shapes, conforms, violations |
| `audit.sequence.check` | ERROR | first_gap_seq, expected, found |
| `kernel.boot` | INFO | version, stores_ready |

**Audit records (immutable)** are written for `eventlog.commit`, `eventlog.abort`, `ontology.load`,
`kernel.boot`, and every migration — in the same transaction as the mutation they describe.
`mm-cli logs verify` checks: schema-valid records → exactly one audit record per committed event →
gapless chained audit sequence → the redaction suite finds no secret → replay is identical with logging
on and off.

---

## 7. Testing

| Type | Test | Assertion |
|---|---|---|
| Unit | `mm-core` id/time/error/config/hash | round-trips; fixed-vector RFC3339 |
| Property (`proptest`) | 100k ULIDs | all unique, strictly increasing across a simulated restart |
| Property | RDF canonical round-trip | 10k triples encode→serialize→re-parse ⇒ identical `content_hash` |
| Property | replay | random mutation sequence replays ⇒ identical `state_hash` |
| Property | bitemporal `as_of` | point-in-time reads return exactly the rows valid at that system+valid instant |
| Golden (`insta`) | `LogRecord` JSON per code | snapshots stable |
| Integration | crash injection | provisional without commit ⇒ aborted on replay; no double-apply |
| Integration | migrations | up→down leaves an empty schema; re-up is identical |
| Conformance | SHACL | clean graph ⇒ 0 violations; broken ⇒ ≥1 |
| E2E | `doctor`, `logs verify`, `replay` | exit 0; criteria per §8 |
| Redaction | secret fixtures | no key/token/PII appears in any sink, including ERROR paths |

Fixtures: `tests/fixtures/{ontology/clean.ttl, ontology/broken.ttl, events/mutations.jsonl, secrets/patterns.json}`.

---

## 8. Pass gate

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

All must hold:
1. Build/clippy/tests exit 0 with **zero warnings**; `unsafe_code` forbidden workspace-wide.
2. `doctor` exits 0, reports SQLite + Oxigraph versions and ontology triple count, both stores writable, and appends exactly one `KernelBoot` event.
3. Property tests: 100k ULIDs unique/monotonic across a restart; 10k-triple export→reimport identical content hash; random-mutation replay identical `state_hash`; `as_of` point-in-time reads correct.
4. Crash injection: replay aborts the provisional event and applies exactly once.
5. `graph validate --graph being`: 0 violations clean, ≥1 broken.
6. `logs verify`: schema-valid, one audit per commit, gapless chained sequence, no secret in any sink, replay identical with logging on/off.
7. `replay --from 0` reproduces the committed `state_hash`.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Oxigraph `Store` blocks the async runtime | Single-writer actor + bounded mailbox; reads via `Arc<Store>` on `spawn_blocking`; the `Store` never escapes the actor for writes |
| RocksDB `pkg-config` build failure | Enforce `rocksdb-pkg-config` workspace-wide; `doctor` fails fast; CI installs `librocksdb-dev` |
| SQLite WAL on a network FS corrupts | Keep `data/` local; `doctor` warns if not local |
| ULID monotonicity lost on restart | Persisted high-water mark; restart property test |
| Event log/store divergence on crash | append→apply→commit with same-transaction audit; provisional abort on replay |
| Audit tampering | `RAISE(ABORT)` triggers + chained hash |
| Ontology/code drift | SHACL generated in-tree; `graph validate` is a gate; Phase 2 adds drift detection |
| Over-building the kernel | Enforce design §108: no cognition, LLM, or personality in Phase 1 |

---

## 10. References (code-available)

- Oxigraph — https://github.com/oxigraph/oxigraph
- sqlx — https://github.com/launchbadge/sqlx
- XTDB (bitemporal) — https://github.com/xtdb/xtdb
- tracing — https://github.com/tokio-rs/tracing
- tracing-opentelemetry — https://github.com/tokio-rs/tracing-opentelemetry
- event-sourcing — https://crates.io/crates/event-sourcing · esrc — https://docs.rs/esrc · eventually — https://github.com/eventually-rs/eventually-rs · cqrs-es example — https://github.com/j5ik2o/cqrs-es-example-rs
- ulid-rs — https://github.com/dylanhart/ulid-rs
- RocksDB — https://github.com/facebook/rocksdb
