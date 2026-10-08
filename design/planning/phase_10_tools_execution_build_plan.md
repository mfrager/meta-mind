# Phase 10 Build Plan — Tool execution, verification, and the external world

> Parent plan: `design/planning/implementation_plan1.md` §13 Phase 10 · Codename Metamind (`mm`)

## 1. Objective and scope

Turn proposals into **authorized, observable action**. This phase builds the layer that lets the cognitive
runtime (Phases 8–9) actually touch the world and learn whether it succeeded, under deterministic
permission and sandbox control.

In scope:
- `mm-tools`: typed `CognitiveOp` → tool mapping, a `ToolRegistry`, a deterministic `PermissionEngine`,
  a deterministic `Executor`, `ActionResult`, an append-only hash-chained **action ledger**, and
  **rollback** for reversible actions.
- Sandbox capability enforcement (filesystem / network / process / resource), copied in from
  `logic-planner::sandbox`.
- Verification tools: cargo build/test, static analysis, symbolic/SMT checks, and residual math verify.
- Authoritative `Observation` records sourced only from execution evidence.
- `mm-cli tool` and `mm-cli action` subcommands.

Out of scope (later phases): the Pi editor tool implementation (Phase 10 ships only its registry stub and
permission profile; Phase 11 wires the RPC client), self-engineering/promotion (Phase 11), the
metacognitive program compiler (Phase 8), and the bounded-decision core / firewall (Phase 9).

Architectural invariants enforced here:
1. **The environment decides what is true** — a plan never produces an observation; only execution does.
2. **Permissions are deterministic** — the LLM proposes a tool call; the `PermissionEngine` (code) allows,
   denies, or escalates to a human. Model confidence never overrides a denial.
3. **Every action is auditable and reversible or explicitly marked irreversible.**

## 2. Prerequisites and dependencies

| Requires | Artifact | Why |
|---|---|---|
| Phase 1 | `mm-core`, `mm-log`, `mm-store-sqlite`, `mm-store-graph`, `mm-eventlog` | IDs, logging, stores, append-only event log, deterministic replay |
| Phase 2 | `mm-codex` module registry | Each tool ships as a registered module with a stable `uri` |
| Phase 6 | `mm-epistemic` `Observation`, `Evidence`, `EpistemicStatus` | Observations must be typed, evidenced, and `OBSERVED` (never inferred) |
| Phase 8/9 | `mm-metacog` `CognitiveOp::Act`, `mm-decision`/`mm-firewall` | Proposals arrive here only after the firewall returns `PROCEED`/`PROCEED_WITH_CAUTION` |
| Phase 11 (forward) | `mm-pi` | Consumes the Pi editor tool stub registered here |

Configuration (`config/tools.toml`, overridable by env): `workspace_root`, `sandbox_root`
(`data/sandbox`), `network_allowlist`, `max_action_bytes`, `default_action_timeout_ms`,
`verification_toolchain` paths.

## 3. Deliverables (exact paths)

```
~/Build/metamind/
├── crates/mm-tools/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs              # re-exports; #![forbid(unsafe_code)]
│       ├── spec.rs             # ToolSpec, ToolName, SchemaRef, Reversibility, SideEffectClass
│       ├── registry.rs         # ToolRegistry (register/resolve/list/describe)
│       ├── permissions.rs      # PermissionEngine, Principal, PermissionReq, Decision
│       ├── action.rs           # ActionSpec, ActionStatus, ActionResult
│       ├── observation.rs      # Observation, ObservationPayload, SourceRef -> Phase 6 Evidence
│       ├── executor.rs         # deterministic Executor (resolve -> authorize -> sandbox -> invoke -> observe)
│       ├── sandbox.rs          # SandboxCapabilities, CapabilityGuard (copied from logic-planner::sandbox)
│       ├── ledger.rs           # append-only hash-chained action ledger
│       ├── rollback.rs         # Snapshot, SnapshotKind, rollback()
│       ├── verify.rs           # verification tools: cargo/static/symbolic/math-residual
│       └── tools/
│           ├── fs.rs           # read, write, move, list
│           ├── process.rs      # exec (build/test), bounded
│           ├── graph.rs        # SPARQL query/update via mm-store-graph
│           ├── tabular.rs      # SQL query/execute via mm-store-sqlite
│           ├── http.rs         # fetch (allowlisted)
│           └── pi_editor.rs    # STUB: declares the Pi editor tool + permissions (Phase 11 implements)
├── crates/mm-store-sqlite/migrations/0010_tools.sql
├── ontology/mm.ttl             # + classes/relations (see §4)
├── ontology/shapes/tools.ttl   # SHACL shapes for Action/Observation/Tool/Permission
├── modules/tools/fs-read/      # nexus-style module wrapping fs.read (plugin.toml + src + tests + manual)
├── modules/tools/process-exec/
├── modules/tools/verify-build/
├── bench/tools/                # permission, sandbox, rollback, and adversarial fixtures
└── tests/e2e/phase10_tools.rs
```

## 4. Data model and ontology deltas

**SQLite (`0010_tools.sql`)** — every table follows the workspace conventions (`id TEXT(26) PRIMARY KEY`,
`*_ulid`, bitemporal columns where applicable):

```sql
CREATE TABLE tool_registry (
  id TEXT(26) PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  version TEXT NOT NULL,
  module_uri TEXT NOT NULL,          -- https://metamind.dev/code/module/tools/...
  spec_json TEXT NOT NULL,
  reversibility TEXT NOT NULL,       -- reversible | compensatable | irreversible
  registered_ulid TEXT NOT NULL,
  active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE permission_grants (
  id TEXT(26) PRIMARY KEY,
  principal TEXT NOT NULL,           -- being ULID | user ULID | system
  scope TEXT NOT NULL,               -- e.g. fs:write:/home/mfrager/Build/metamind/data/sandbox/**
  tool_pattern TEXT NOT NULL,
  granted_ulid TEXT NOT NULL,
  granted_at TEXT NOT NULL,
  expires_at TEXT,
  revoked_at TEXT,
  granted_by TEXT NOT NULL
);

CREATE TABLE tool_calls (
  id TEXT(26) PRIMARY KEY,
  tool_name TEXT NOT NULL,
  args_hash TEXT NOT NULL,           -- sha256 of canonical args
  args_json TEXT NOT NULL,           -- redacted by mm-log Redactor
  permission_decision TEXT NOT NULL, -- allow | deny | escalate
  status TEXT NOT NULL,              -- ok | error | denied | timeout
  trace_id TEXT NOT NULL,
  started_at TEXT NOT NULL,
  ended_at TEXT,
  latency_ms INTEGER
);

CREATE TABLE action_ledger (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT(26) NOT NULL UNIQUE,
  action_id TEXT NOT NULL,
  event TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  prev_hash TEXT NOT NULL,           -- hash chain; genesis = 64 zeros
  entry_hash TEXT NOT NULL,          -- sha256(prev_hash || id || event || payload_json)
  at TEXT NOT NULL
);

CREATE TABLE observations (
  id TEXT(26) PRIMARY KEY,
  action_id TEXT NOT NULL,
  source TEXT NOT NULL,              -- tool name + endpoint
  payload_json TEXT NOT NULL,
  evidence_id TEXT NOT NULL,         -- Phase 6 Evidence ULID
  observed_at TEXT NOT NULL,
  graph_iri TEXT NOT NULL            -- https://metamind.dev/data/{ulid}
);

CREATE TABLE rollback_snapshots (
  id TEXT(26) PRIMARY KEY,
  action_id TEXT NOT NULL,
  kind TEXT NOT NULL,                -- fs_path | sql_row | graph_quad | none
  ref TEXT NOT NULL,
  content_hash TEXT NOT NULL,        -- for byte-identical restore checks
  created_at TEXT NOT NULL
);
```

**Ontology (`mm:` = `https://metamind.dev/ontology#`)** — add classes `mm:Tool`, `mm:Action`,
`mm:Observation`, `mm:Permission`, `mm:ExecutionEvidence`, `mm:VerificationObligation`; relations
`mm:performedBy`, `mm:authorizedBy`, `mm:observedIn`, `mm:producedEvidence`, `mm:resultedIn`,
`mm:reversedBy`, `mm:verifiedBy`, `mm:hasCapability`; plus `mm:reversibility` and `mm:sideEffectClass`
datatype properties. Named graphs: `/world` (observations only, `OBSERVED` status) and `/provenance`
(action → observation → evidence). SHACL (`ontology/shapes/tools.ttl`) requires every `mm:Observation` to
carry exactly one `mm:producedEvidence` and one `mm:observedIn` source, and forbids `mm:Observation`
nodes in any graph other than `/world`.

## 5. Public interfaces (Rust traits/types, CLI, plugin.toml)

```rust
// spec.rs
pub struct ToolName(pub String);
pub struct SchemaRef(pub String);            // e.g. "mm:FilePath"
pub enum Reversibility { Reversible, Compensatable, Irreversible }
pub struct ToolSpec {
    pub name: ToolName,
    pub version: String,
    pub inputs: Vec<SchemaRef>,
    pub outputs: Vec<SchemaRef>,
    pub permissions: Vec<PermissionReq>,
    pub reversibility: Reversibility,
    pub side_effects: SideEffectClass,
    pub module_uri: String,
}

// registry.rs
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;
    fn invoke<'a>(&'a self, ctx: &'a ExecCtx, args: ToolArgs)
        -> BoxFuture<'a, Result<ToolOutcome, ToolError>>;
}
pub struct ToolRegistry;
impl ToolRegistry {
    pub fn register(&self, tool: Box<dyn Tool>) -> Result<(), RegistryError>;
    pub fn resolve(&self, name: &ToolName) -> Result<Arc<dyn Tool>, RegistryError>;
    pub fn list(&self) -> Vec<ToolSpec>;
}

// permissions.rs
pub struct PermissionReq { pub resource: String, pub action: PermissionAction }
pub struct Principal(pub String);
pub enum Decision { Allow, Deny { reason: DenyReason }, Escalate { to: EscalationTarget } }
pub trait PermissionEngine {
    fn authorize(&self, principal: &Principal, req: &PermissionReq) -> Decision; // pure & deterministic
}

// executor.rs
pub struct Executor { /* registry, permissions, sandbox, ledger, eventlog, graph, sql */ }
impl Executor {
    pub async fn execute(&self, action: ActionSpec) -> Result<ActionResult, ExecError>;
}
pub struct ActionResult {
    pub action_id: Ulid,
    pub status: ActionStatus,        // Ok | Error | Denied | Timeout
    pub observation: Option<Observation>,
    pub evidence: Vec<EvidenceId>,
}

// sandbox.rs
#[derive(Clone, Copy, Default)]
pub struct SandboxCapabilities {
    pub filesystem: FilesystemAccess,  // None | ReadOnly(PathBuf) | ReadWrite(PathBuf)
    pub network: NetworkAccess,        // Denied | Allowlist(Vec<String>)
    pub subprocess: bool,
    pub max_bytes: u64,
}

// ledger.rs / rollback.rs
pub fn append(entry: LedgerEntry) -> Result<LedgerRef, LedgerError>; // verifies prev_hash chain
pub fn verify_chain() -> Result<(), LedgerError>;
pub fn snapshot(action: &ActionSpec) -> Result<Option<SnapshotId>, RollbackError>;
pub async fn rollback(snapshot: SnapshotId) -> Result<RestoredHash, RollbackError>;
```

CLI (`mm-cli`):

```text
mm-cli tool list [--json]
mm-cli tool describe <name>
mm-cli tool run <name> [--arg k=v]... [--principal <ulid>] [--dry-run]
mm-cli action ledger [--since <ulid>] [--json]
mm-cli action show <action-ulid>
mm-cli action rollback <action-ulid>
mm-cli verify run <obligation>          # cargo | static | symbolic | math-residual
```

Tool module manifest (nexus-style, registered in Phase 2):

```toml
[plugin]
name = "mm-fs-read"
uri = "https://metamind.dev/code/module/tools/fs-read"   # stable identifier
version = "0.1.0"
[metadata]
category = "tools"
owned_by_phase = 10
capability = "mm:FilesystemRead"
[tbox.functions]
"fs.read"  = { source = "handlers::fs_read" }
"fs.list"  = { source = "handlers::fs_list" }
[monad.operations]
name = "fs"
arity = 1
[build]
rust_edition = "2021"
```

## 6. External references copied in and integration

Per parent-plan §7, external code is **reference only**: the needed source is copied into
`vendor/<origin>/…` as workspace members and integrated, with provenance recorded.

| Reference | Copied from | Integrated as |
|---|---|---|
| `logic-planner::sandbox` (`Sandbox`, `Capabilities`, `FilesystemAccess`, `NetworkAccess`, `admit`) | `rust_symbolic` | `mm-tools/src/sandbox.rs` (adapted to `ExecCtx`; capability checks stay a hard error, never a silent skip) |
| `backend-registry` / `solver-ir` (`SolverBackend`, `CompiledProblem`) | `rust_symbolic` | `mm-tools/src/verify.rs` symbolic/SMT verification adapter |
| `math-runtime` residual verifier | `rust_symbolic` | `mm-tools/src/verify.rs` `math-residual` obligation |
| `llm-interface` (`tool_catalog`, `orchestrate`, `repair`) | `rust_symbolic` | pattern/skeleton for `ToolRegistry` + repair loop; not the runtime |
| nexus `monad-process` / `monad-file` / `monad-sparql` / `monad-sql` / `monad-resource` | `nexus` | concrete tool implementations behind the `Tool` trait |
| `kg-validate` `EvidenceValidator` / `ClaimValidator` | `rust_extract` | observation→evidence validation before commit |

Every copied tree gets `vendor/<origin>/COPYING.md` (origin repo, path, revision, license). The Phase 2
scanner emits `mmc:copiedFrom` for each copied file. Copied code is Metamind-owned from that point and may
be modified in Phase 11.

## 7. Step-by-step implementation tasks

1. **Scaffold the crate.** Create `crates/mm-tools` with `#![forbid(unsafe_code)]`, add to the workspace,
   wire `mm-core`/`mm-log`/`mm-epistemic`. Check: `cargo build -p mm-tools` clean.
2. **Define specs and status.** Implement `spec.rs`, `action.rs`, `observation.rs`; every type serializes
   canonically (content-hash stable) and derives `ToRdf`/`FromRdf` via `rdf-codec`. Check: round-trip test.
3. **Registry.** Implement `ToolRegistry` + migration-backed persistence into `tool_registry`; loading a
   module registers its T-Box functions. Check: register/resolve/list unit tests; duplicate name rejected.
4. **Permission engine.** Implement `PermissionEngine` over `permission_grants` (pure `authorize`,
   deterministic). Include grant/revoke/expire and a `mm-cli tool` grant path. Check: allow/deny/escalate
   table tests; expired and revoked grants deny.
5. **Sandbox.** Copy in `logic-planner::sandbox`, adapt to `ExecCtx`, and enforce `FilesystemAccess` /
   `NetworkAccess` / `subprocess` / `max_bytes`. Check: undeclared path/network fails with a typed error.
6. **Executor.** Implement `resolve → authorize → sandbox → snapshot → invoke → observe → ledger append`.
   Denials short-circuit before any side effect. Check: order-of-operations test.
7. **Tools v1.** Implement `fs`, `process`, `graph`, `tabular`, `http`, and the `pi_editor` **stub**
   (spec + permissions only). Every tool declares reversibility and side effects. Check: per-tool unit tests.
8. **Observations.** Convert every `ToolOutcome` into an `Observation` with a Phase 6 `EvidenceId` and a
   `/world` graph node; reject any observation lacking execution evidence. Check: fabricated-observation
   rejection test.
9. **Action ledger.** Implement hash-chained append + `verify_chain`; write each action's lifecycle entries.
   Check: tamper/insert/delete tests all fail verification.
10. **Rollback.** Implement snapshots for `fs_path`, `sql_row`, `graph_quad`; restore to the recorded
    `content_hash`. Irreversible tools refuse and require firewall escalation. Check: byte-identical restore.
11. **Verification tools.** Implement `cargo build/test`, static analysis, symbolic (`backend-registry`),
    and `math-residual` obligations; each returns a proof/evidence object, never a prose verdict.
    Check: seeded inconsistency caught.
12. **Ontology + SHACL.** Add the classes/relations and `ontology/shapes/tools.ttl`. Check:
    `mm-cli graph validate --graph world` and `--graph provenance` report 0 violations.
13. **CLI.** Add `tool list|describe|run`, `action ledger|show|rollback`, `verify run`. Check: e2e in
    `tests/e2e/phase10_tools.rs`.
14. **Bench corpus + docs.** Add `bench/tools/*` fixtures and a `manual/module.md` per tool module.
    Check: fixtures load; each capability names its tests (Phase 2 `codex verify` stays green).

## 8. Detailed logging requirements

All records go through `mm-log` with the global fields (§10 of the parent plan) and a `trace_id` ULID
shared with the event log and RDF provenance. **Audit records** (immutable, transactionally written with
the state change) unless marked operational:

| Event code | Level | Fields |
|---|---|---|
| `tool.registry.register` | INFO | `tool`, `version`, `module_uri`, `reversibility`, `result` |
| `tool.invoke.start` | INFO | `tool`, `args_hash`, `principal`, `trace_id`, `action_id` (audit) |
| `tool.permission.check` | INFO | `tool`, `resource`, `decision`, `reason`, `principal` (audit) |
| `tool.sandbox.deny` | WARN | `tool`, `capability`, `requested`, `allowed` (audit) |
| `tool.invoke.end` | INFO | `tool`, `status`, `latency_ms`, `result_hash`, `error` (audit) |
| `action.ledger.append` | DEBUG | `seq`, `entry_hash`, `prev_hash`, `action_id` |
| `action.rollback.start` / `.end` | INFO | `action_id`, `snapshot_id`, `restored_hash` (audit) |
| `observation.record` | INFO | `observation_id`, `action_id`, `source`, `evidence_id`, `graph_iri` (audit) |
| `verify.run.start` / `.end` | INFO | `obligation`, `subject`, `status`, `proof_ref` (audit) |
| `verify.obligation.failure` | WARN | `obligation`, `subject`, `counterexample_ref` (audit) |

Every action emits start + end (success and failure); errors carry the full cause chain. Argument payloads
are passed through `mm-log`'s `Redactor` (secrets, tokens, PII never emitted; free text length-bounded).
`mm-cli logs trace <trace_id>` must reconstruct the full action timeline.

## 9. Testing plan

- **Unit** (`crates/mm-tools/src/**`): spec/hash stability, registry duplicate rejection, permission table
  (allow/deny/escalate/expired/revoked), sandbox capability failures, per-tool behavior.
- **Property** (`proptest`): ledger hash-chain verifies for arbitrary append sequences and fails for any
  mutation; canonical `ActionSpec` serialization is byte-stable across re-parses.
- **Golden / replay** (`insta`): recorded tool-call replay reproduces identical observations and ledger
  hashes; `mm-cli replay` yields the same state with logging on and off.
- **Adversarial** (`bench/tools/`): missing grant → deny; grant scope too narrow → deny; sandbox escape
  attempts (path traversal, undeclared network, oversize payload) → deny; fabricated observation → reject.
- **End-to-end** (`tests/e2e/phase10_tools.rs`): propose → authorize → execute → observe → ledger → rollback.
- **Ontology**: SHACL valid/garbage fixtures for `/world` and `/provenance`.

## 10. Pass gate

Run as plain commands; earlier phases' gates must still pass.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mm-tools

# Registry + tools present
mm-cli tool list --json | jq -e 'length >= 6 and (map(.name) | index("fs.read")) != null'

# Unauthorized call is denied by deterministic code (never by model judgment)
mm-cli tool run process.exec --arg cmd='cargo test' --principal "$BEING_ULID" ; test $? -ne 0

# Sandbox enforcement
mm-cli tool run fs.read --arg path=/etc/shadow            ; test $? -ne 0   # undeclared path
mm-cli tool run http.fetch --arg url=https://example.com  ; test $? -ne 0   # not allowlisted

# Authorized read succeeds and yields an observation + ledger entry
mm-cli tool run fs.read --arg path=data/sandbox/README.md ; test $? -eq 0
mm-cli action ledger --json | jq -e '.[-1].event == "observation.record"'

# Ledger is append-only and hash-chained
mm-cli action ledger --verify | grep -q "chain ok"

# Reversible action rolls back byte-identically
before=$(sha256sum data/sandbox/target.txt | cut -d' ' -f1)
mm-cli tool run fs.write --arg path=data/sandbox/target.txt --arg text='hello'
after=$(sha256sum data/sandbox/target.txt | cut -d' ' -f1)
test "$before" != "$after"
mm-cli action rollback "$(mm-cli action ledger --json | jq -r '.[-1].action_id')"
test "$(sha256sum data/sandbox/target.txt | cut -d' ' -f1)" = "$before"

# Symbolic verifier catches a seeded inconsistency
mm-cli verify run symbolic --subject bench/tools/seeded_inconsistency.logic ; test $? -ne 0

# Logging gate
mm-cli logs verify
```

Objective criteria: unauthorized → non-zero + `tool.permission.check` deny record; sandbox violations →
non-zero + `tool.sandbox.deny`; every successful action has exactly one `observation.record` with an
`evidence_id`; `action ledger --verify` reports an intact chain; rollback restores the recorded hash;
`codex verify` and `logs verify` pass.

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| LLM-produced action treated as performed | Observations are only created by the executor from `ToolOutcome`; the epistemic layer rejects evidence-free observations |
| Permission bypass via tool composition | Authorization is re-checked per primitive call, not only at the action root |
| Sandbox escape | Capability guard is a hard error (no silent skip), mirroring `logic-planner::sandbox`; adversarial escape fixtures in the gate |
| Ledger tampering | Hash-chained, append-only; `verify_chain` runs in the gate and on open |
| Irreversible action without rollback | Tools declare `Reversibility`; `Irreversible` requires firewall escalation and explicit confirmation |
| Copied sink/tool code drifts | `mmc:copiedFrom` + `COPYING.md`; copied tools are re-tested under `mm-tools` CI |

## 12. Design traceability

| Design section | This phase |
|---|---|
| §25 Constraints and feasibility | `PermissionReq`, sandbox capability checks, `Irreversible` escalation |
| §52 External state and tool execution | `ActionResult`, executor separation (LLM proposes → firewall → policy → executor → external confirm) |
| §70 Code improvement by the LLM | verification tools the LLM's patches are judged by (compile/tests/static/symbolic) |
| §79 Verification strategy | `verify.rs` obligations costed against error cost × probability × decision dependence |
| §80 Formal invariants | permission checks, resource limits, rollback correctness enforced deterministically |
| §87–90 ontology, runtime model, typed ops, backend map | `mm:Action/Observation/Tool/Permission`, `CognitiveOp::Act`, backend mapping for exec/verify |
| §108–110 guardrails / invariants | environment decides truth; deterministic enforcement; no untested self-modification |
| `congitive_elements4.md` (§4, §5, §16) | epistemic status on observations; "LLM for plausibility, world/tools for truth"; scarce-resource action value |
| `bootstrap_procses1.md` / `bootstrap_procses2.md` | capability gaps detected via failed execution feed Phase 11; tools are learned skills |
