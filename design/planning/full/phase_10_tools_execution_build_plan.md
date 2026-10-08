# Phase 10 — Tool Execution, Verification, and the External World (Extended Build Plan)

> Extended from: `design/planning/phase_10_tools_execution_build_plan.md` · Parent plan: `design/planning/implementation_plan1.md` §13 Phase 10 · Codename Metamind (`mm`)

This is the research-grounded, streamlined revision. It keeps Phase 10's scope and gate but locks in the
mechanisms that make action safe and truthful: a typed tool protocol, default-deny authorization with
declarative policy, tiered sandbox isolation, durable idempotent execution, and an append-only
hash-chained action ledger whose only outputs are execution-sourced observations.

---

## 0. Research foundation (code-available)

Every idea below is borrowed from a project whose source is available. Only ideas with code are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this phase |
|---|---|---|---|
| Typed tool protocol + discovery/invocation | Model Context Protocol — https://github.com/modelcontextprotocol/modelcontextprotocol (org: https://github.com/modelcontextprotocol) | Tool schema (name, JSON-schema input/output), `tools/list` + `tools/call`, JSON-RPC transport | Every tool is described by an MCP-shaped `ToolSpec`; a `McpBridge` exposes Metamind tools over MCP and ingests external MCP servers behind the same `Tool` trait |
| Capability-based in-process isolation | Wasmtime / WASI — https://github.com/bytecodealliance/wasmtime | WASI capability model: no ambient authority; explicit fs/net/preopen grants | Tier-1 sandbox for pure tools: run in-process under an explicit capability set; undeclared access is a hard error |
| Syscall-interception sandbox | gVisor — https://github.com/google/gvisor | User-space application kernel; syscall filtering | Tier-2 sandbox for untrusted-but-native tools when a microVM is too heavy |
| MicroVM isolation | Firecracker — https://github.com/firecracker-microvm/firecracker | Lightweight KVM microVMs with a minimal device model | Tier-3 sandbox for arbitrary code (Pi-generated builds, untrusted binaries) |
| Agent code-execution sandbox lifecycle | E2B — https://github.com/e2b-dev/E2B · microsandbox — https://github.com/team-microsandbox/microsandbox | Sandbox create/exec/kill API, per-session ephemeral FS | `Sandbox` lifecycle API: create → exec → collect artifacts → destroy; no state leaks between actions |
| Declarative policy-as-code authorization | Open Policy Agent / Rego — https://github.com/open-policy-agent/opa | Policy separated from code; query `allow` per request | `PermissionEngine` evaluates capability grants **and** a Rego-style policy set; policy is data, reviewed and versioned |
| Permit/forbid authorization language | Cedar — https://github.com/cedar-policy/cedar | `permit`/`forbid` with explicit-deny precedence over allow | Policy evaluation order: explicit `forbid` beats any `permit`; absence of a permit is deny (default-deny) |
| Typed function calling / tool selection | Gorilla — https://github.com/ShishirPatil/gorilla · ToolBench/ToolLLM — https://github.com/OpenBMB/ToolBench | API/tool schema grounding; tool-selection evaluation | Tool selection consumes `ToolSpec` schemas; `bench/tools/` mirrors ToolBench's selection/eval split |
| Durable, retryable, idempotent execution | Temporal — https://github.com/temporalio/temporal · Restate — https://github.com/restatedev/restate | Deterministic replay of workflows; exactly-once effects via idempotency keys | Action execution is journaled with idempotency keys; retries never duplicate an effect; the ledger is the journal |
| Sandbox isolation taxonomy | https://github.com/restyler/awesome-sandbox · https://github.com/bureado/awesome-agent-runtime-security | Comparison of isolation tiers | The three-tier `SandboxTier` choice above |

---

## 1. Objective and scope

Turn proposals into **authorized, observable action**. Phase 10 is the boundary where the being can touch
the world and learn whether it succeeded — under deterministic permission and sandbox control.

**In scope**
- `mm-tools`: MCP-shaped `ToolSpec`, `ToolRegistry`, `McpBridge`, deterministic `PermissionEngine`
  (capability + policy), tiered `Sandbox`, deterministic `Executor`, `ActionResult`, authoritative
  `Observation`, append-only hash-chained action ledger, and rollback for reversible actions.
- Verification tools: cargo build/test, static analysis, symbolic/SMT checks, residual math verify.
- Tool modules (`fs`, `process`, `graph`, `tabular`, `http`, and the `pi_editor` stub).
- `mm-cli tool | action | policy | verify | mcp` subcommands.

**Out of scope (later phases)** the Pi editor implementation (Phase 10 ships only its spec + permission
profile; Phase 11 wires the RPC client), self-engineering/promotion (Phase 11), the program compiler
(Phase 8), and the decision core / firewall (Phase 9).

**Invariants enforced here**
1. **The environment decides what is true** — a plan never produces an observation; only execution does.
2. **Authorization is deterministic** — the LLM proposes; `PermissionEngine` (code) allows, denies, or
   escalates. Model confidence never overrides a denial, and explicit `forbid` always wins.
3. **Every action is auditable, and either reversible or explicitly marked irreversible.**
4. **Effects are exactly-once** — retries are guarded by idempotency keys; the ledger is the journal.

---

## 2. Architecture

Pipeline: **propose → authorize → sandbox → snapshot → invoke → observe → ledger** (any denial
short-circuits before a side effect).

```
CognitiveOp::Act (Phase 8/9, firewall must return PROCEED*)
        │
        ▼
  ToolRegistry ──resolve──▶ ToolSpec ──▶ PermissionEngine ──deny/escalate──▶ ActionResult{Denied}
        │                                     │ allow
        │                                     ▼
        │                              PolicySet (forbid > permit > default-deny)
        │                                     │
        ▼                                     ▼
  Sandbox ──tier select──▶ Tier1 wasm-caps │ Tier2 gVisor │ Tier3 microVM
        │                                     │
        │       snapshot(reversible?) ────────┤
        ▼                                     ▼
   Executor.invoke(idempotency_key) ──▶ ToolOutcome ──▶ Observation ──▶ /world + Evidence
        │                                                       │
        └──────────────▶ ActionLedger.append(hash-chained) ◀────┘
```

**Isolation tiers** (selected per tool/trust level):

| Tier | Backend | Use |
|---|---|---|
| 1 — capability-limited in-process | Wasmtime/WASI semantics | pure tools; explicit fs/net/preopen grants, no ambient authority |
| 2 — syscall interception | gVisor | untrusted native tools where a microVM is too heavy |
| 3 — microVM | Firecracker / E2B-style | arbitrary code: Pi-generated builds, untrusted binaries; ephemeral FS |

**Permission model.** Default-deny. A request must be covered by (a) a capability grant in
`permission_grants` **and** (b) a policy set that does not `forbid` it; the effective decision is
`forbid > escalate > permit`. Grants carry a scope, an expiry, and the grantor; policy sets are versioned
data evaluated in a pure function so the same inputs always yield the same decision.

---

## 3. Deliverables and workspace layout

```
~/Build/metamind/
├── crates/mm-tools/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # re-exports; #![forbid(unsafe_code)]
│       ├── spec.rs           # ToolSpec, ToolName, SchemaRef, Reversibility, SideEffectClass, Annotations
│       ├── registry.rs       # ToolRegistry (register/resolve/list/describe)
│       ├── mcp.rs            # McpBridge: tools/list + tools/call over JSON-RPC (serve + ingest)
│       ├── policy.rs         # PolicySet, PolicyEngine (forbid > permit > default-deny)
│       ├── permissions.rs    # PermissionEngine, Principal, PermissionReq, Decision
│       ├── action.rs         # ActionSpec, ActionStatus, ActionResult
│       ├── observation.rs    # Observation, ObservationPayload, SourceRef -> Phase 6 Evidence
│       ├── sandbox/
│       │   ├── mod.rs        # Sandbox trait, SandboxTier, SandboxCapabilities
│       │   ├── wasi.rs       # Tier 1 (capability-limited, Wasmtime/WASI model)
│       │   ├── gvisor.rs     # Tier 2 (syscall interception) — feature-gated
│       │   └── microvm.rs    # Tier 3 (Firecracker/E2B-style lifecycle) — feature-gated
│       ├── executor.rs       # deterministic Executor (resolve->authorize->sandbox->invoke->observe)
│       ├── idempotency.rs    # idempotency keys; exactly-once effect guard
│       ├── ledger.rs         # append-only hash-chained action ledger
│       ├── rollback.rs       # Snapshot, SnapshotKind, rollback()
│       ├── verify.rs         # cargo/static/symbolic/math-residual obligations
│       └── tools/{fs,process,graph,tabular,http,pi_editor}.rs
├── crates/mm-store-sqlite/migrations/0010_tools.sql
├── ontology/mm.ttl            # + classes/relations (see §4.3)
├── ontology/shapes/tools.ttl  # SHACL for Action/Observation/Tool/Permission
├── modules/tools/{fs-read,process-exec,verify-build}/
├── bench/tools/               # permission, sandbox, rollback, tool-selection, adversarial fixtures
└── tests/e2e/phase10_tools.rs
```

## 4. Detailed specifications

### 4.1 Tool protocol and registry

```rust
// spec.rs — MCP-shaped, extended with reversibility/side-effects
pub struct ToolName(pub String);
pub struct SchemaRef(pub String);                 // mm: schema IRI (e.g. mm:FilePath)
pub struct JsonSchema(pub serde_json::Value);     // MCP inputSchema/outputSchema
pub enum Reversibility { Reversible, Compensatable, Irreversible }
pub enum SideEffectClass { None, Local, External, Irreversible }

pub struct ToolAnnotations {           // MCP hints, authoritative only when a tool declares them
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

pub struct ToolSpec {
    pub name: ToolName,
    pub version: String,
    pub description: String,
    pub input_schema: JsonSchema,
    pub output_schema: JsonSchema,
    pub permissions: Vec<PermissionReq>,
    pub reversibility: Reversibility,
    pub side_effects: SideEffectClass,
    pub annotations: ToolAnnotations,
    pub module_uri: String,            // https://metamind.dev/code/module/tools/...
    pub sandbox_tier: SandboxTier,
}

// registry.rs
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;
    fn invoke<'a>(&'a self, ctx: &'a ExecCtx, args: ToolArgs)
        -> BoxFuture<'a, Result<ToolOutcome, ToolError>>;
}
impl ToolRegistry {
    pub fn register(&self, tool: Box<dyn Tool>) -> Result<(), RegistryError>; // duplicate name -> Err
    pub fn resolve(&self, name: &ToolName) -> Result<Arc<dyn Tool>, RegistryError>;
    pub fn list(&self) -> Vec<ToolSpec>;
}

// mcp.rs — serve Metamind tools over MCP and ingest external MCP servers
pub struct McpBridge {
    registry: Arc<ToolRegistry>,
    policy: Arc<PolicyEngine>,
}
impl McpBridge {
    pub async fn serve_stdio(&self) -> Result<(), McpError>;         // tools/list, tools/call
    pub async fn call_tool(&self, name: &ToolName, args: ToolArgs,
                           principal: &Principal) -> Result<ToolOutcome, McpError>;
    pub fn ingest_remote(&self, spec: ToolSpec, client: McpClient) -> Result<(), McpError>;
}
```

### 4.2 Authorization, sandbox, execution, ledger

```rust
// policy.rs — declarative policy-as-code; pure, deterministic
pub struct PolicySet { pub id: String, pub version: u32, pub rules: Vec<PolicyRule> }
pub enum Effect { Permit, Forbid }
pub struct PolicyRule { pub effect: Effect, pub principal: String, pub action: String, pub resource: String }
pub struct PolicyEngine;
impl PolicyEngine {
    /// forbid > permit > (default) deny — same inputs always yield the same decision.
    pub fn evaluate(&self, sets: &[PolicySet], p: &Principal, req: &PermissionReq) -> Effect;
}

// permissions.rs
pub struct PermissionReq { pub resource: String, pub action: PermissionAction }
pub struct Principal(pub String);       // being ULID | user ULID | system
pub enum Decision { Allow, Deny { reason: DenyReason }, Escalate { to: EscalationTarget } }
pub trait PermissionEngine: Send + Sync {
    fn authorize(&self, principal: &Principal, req: &PermissionReq) -> Decision; // pure
}

// sandbox/mod.rs
#[derive(Clone, Copy)] pub enum SandboxTier { WasmCaps, Gvisor, MicroVm }
#[derive(Clone, Default)]
pub struct SandboxCapabilities {
    pub filesystem: FilesystemAccess,   // None | ReadOnly(PathBuf) | ReadWrite(PathBuf)
    pub network: NetworkAccess,         // Denied | Allowlist(Vec<String>)
    pub subprocess: bool,
    pub max_bytes: u64,
}
pub trait Sandbox: Send + Sync {
    fn tier(&self) -> SandboxTier;
    async fn run(&self, caps: &SandboxCapabilities, action: &ActionSpec) -> Result<ToolOutcome, SandboxError>;
}

// executor.rs
pub struct Executor { /* registry, permissions, policy, sandbox, ledger, idempotency, eventlog, graph, sql */ }
impl Executor {
    pub async fn execute(&self, action: ActionSpec, principal: Principal) -> Result<ActionResult, ExecError>;
}
pub struct ActionResult {
    pub action_id: Ulid,
    pub status: ActionStatus,           // Ok | Error | Denied | Timeout
    pub observation: Option<Observation>,
    pub evidence: Vec<EvidenceId>,
}

// idempotency.rs — exactly-once effects across retries (durable-execution pattern)
pub struct IdempotencyKey(pub String);
impl IdempotencyStore {
    /// First call records in-flight; a repeat with the same key returns the recorded outcome.
    pub async fn begin(&self, k: &IdempotencyKey) -> Result<Begin, DupError>;
    pub async fn finish(&self, k: &IdempotencyKey, r: &ActionResult) -> Result<(), MmError>;
}

// ledger.rs / rollback.rs
pub fn append(entry: LedgerEntry) -> Result<LedgerRef, LedgerError>;  // verifies prev_hash chain
pub fn verify_chain() -> Result<(), LedgerError>;
pub fn snapshot(action: &ActionSpec) -> Result<Option<SnapshotId>, RollbackError>;
pub async fn rollback(snapshot: SnapshotId) -> Result<RestoredHash, RollbackError>;
```

### 4.3 SQLite (`crates/mm-store-sqlite/migrations/0010_tools.sql`)

All tables follow workspace conventions (`id TEXT(26) PRIMARY KEY`, ULID, bitemporal columns where
applicable).

```sql
CREATE TABLE tool_registry (
  id TEXT(26) PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  version TEXT NOT NULL,
  module_uri TEXT NOT NULL,
  spec_json TEXT NOT NULL,
  sandbox_tier TEXT NOT NULL CHECK (sandbox_tier IN ('WasmCaps','Gvisor','MicroVm')),
  reversibility TEXT NOT NULL,       -- reversible | compensatable | irreversible
  active INTEGER NOT NULL DEFAULT 1,
  registered_ulid TEXT NOT NULL,
  system_from TEXT NOT NULL, system_to TEXT,
  valid_from TEXT NOT NULL, valid_to TEXT
);

CREATE TABLE policy_sets (
  id TEXT(26) PRIMARY KEY,
  name TEXT NOT NULL, version INTEGER NOT NULL,
  rules_json TEXT NOT NULL,
  created_ulid TEXT NOT NULL, system_from TEXT NOT NULL,
  UNIQUE(name, version)
);

CREATE TABLE permission_grants (
  id TEXT(26) PRIMARY KEY,
  principal TEXT NOT NULL,           -- being ULID | user ULID | system
  scope TEXT NOT NULL,               -- e.g. fs:write:data/sandbox/**
  tool_pattern TEXT NOT NULL,
  policy_set_id TEXT REFERENCES policy_sets(id),
  granted_ulid TEXT NOT NULL, granted_by TEXT NOT NULL,
  granted_at TEXT NOT NULL, expires_at TEXT, revoked_at TEXT
);

CREATE TABLE tool_calls (
  id TEXT(26) PRIMARY KEY,
  tool_name TEXT NOT NULL,
  args_hash TEXT NOT NULL,           -- sha256 of canonical args
  args_json TEXT NOT NULL,           -- redacted by mm-log Redactor
  permission_decision TEXT NOT NULL, -- allow | deny | escalate
  sandbox_tier TEXT NOT NULL,
  status TEXT NOT NULL,              -- ok | error | denied | timeout
  idempotency_key TEXT,
  trace_id TEXT(26) NOT NULL,
  started_at TEXT NOT NULL, ended_at TEXT, latency_ms INTEGER
);

CREATE TABLE idempotency_keys (
  key TEXT PRIMARY KEY,
  action_id TEXT(26) NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('in_flight','done')),
  result_json TEXT,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);

CREATE TABLE action_ledger (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT(26) NOT NULL UNIQUE,
  action_id TEXT NOT NULL,
  event TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  prev_hash TEXT NOT NULL,           -- genesis = 64 zeros
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

### 4.4 Ontology and SHACL

Namespaces: `mm:` = `https://metamind.dev/ontology#`. Named graphs: `/world` (observations only) and
`/provenance` (action → observation → evidence).

- Classes: `mm:Tool`, `mm:Action`, `mm:Observation`, `mm:Permission`, `mm:Policy`, `mm:ExecutionEvidence`,
  `mm:VerificationObligation`.
- Relations: `mm:performedBy`, `mm:authorizedBy`, `mm:observedIn`, `mm:producedEvidence`, `mm:resultedIn`,
  `mm:reversedBy`, `mm:verifiedBy`, `mm:hasCapability`.
- Datatype properties: `mm:reversibility`, `mm:sideEffectClass`, `mm:sandboxTier`.
- SHACL (`ontology/shapes/tools.ttl`): every `mm:Observation` has exactly one `mm:producedEvidence` and
  one `mm:observedIn`; `mm:Observation` nodes are forbidden outside `/world`; every `mm:Action` with
  `mm:reversibility "irreversible"` must carry `mm:authorizedBy` an escalated permission.

---

## 5. Build sequence

1. **Scaffold.** `crates/mm-tools` with `#![forbid(unsafe_code)]`, add to workspace, wire
   `mm-core`/`mm-log`/`mm-epistemic`. *Check:* `cargo build -p mm-tools` clean.
2. **Spec + action + observation.** `spec.rs`, `action.rs`, `observation.rs` with canonical
   serialization and `rdf-codec` `ToRdf`/`FromRdf`. *Check:* round-trip test.
3. **Registry + MCP schema.** `ToolRegistry` persisted into `tool_registry`; MCP-shaped `ToolSpec`.
   *Check:* register/resolve/list tests; duplicate name rejected.
4. **Policy engine + permission engine.** `PolicyEngine::evaluate` (`forbid > permit > deny`) and
   `PermissionEngine::authorize` over `permission_grants`/`policy_sets`. *Check:* allow/deny/escalate
   table tests; expired and revoked grants deny; `forbid` beats `permit`.
5. **Sandbox tiers.** `Sandbox` trait + Tier 1 (`wasi.rs`) first; `gvisor.rs`/`microvm.rs` feature-gated.
   Enforce `FilesystemAccess`/`NetworkAccess`/`subprocess`/`max_bytes` as hard errors. *Check:* undeclared
   path/network fails with a typed error; sandbox is destroyed after each action.
6. **Idempotency.** `IdempotencyStore` in/out; retries return the recorded outcome. *Check:* double-invoke
   of a non-idempotent tool executes once.
7. **Executor.** `resolve → authorize → tier-select → snapshot → invoke → observe → ledger`. Denials
   short-circuit before any side effect. *Check:* order-of-operations test.
8. **Tools v1.** `fs`, `process`, `graph`, `tabular`, `http`, and the `pi_editor` **stub** (spec +
   permissions only, `sandbox_tier = MicroVm`). Each declares reversibility + side effects.
9. **Observations.** Convert `ToolOutcome` → `Observation` with a Phase 6 `EvidenceId` and `/world` node;
   reject any observation lacking execution evidence. *Check:* fabricated-observation rejection.
10. **Ledger.** Hash-chained append + `verify_chain`; write each action's lifecycle entries.
    *Check:* tamper/insert/delete all fail verification.
11. **Rollback.** Snapshots for `fs_path`, `sql_row`, `graph_quad`; restore to `content_hash`;
    `Irreversible` refuses and requires firewall escalation. *Check:* byte-identical restore.
12. **Verification tools.** `cargo build/test`, static analysis, symbolic (`backend-registry`), and
    `math-residual`; each returns a proof/evidence object, never prose. *Check:* seeded inconsistency caught.
13. **Ontology + SHACL.** Add classes/relations + `ontology/shapes/tools.ttl`. *Check:*
    `graph validate --graph world` and `--graph provenance` report 0 violations.
14. **MCP bridge + CLI.** `mcp serve`, `tool list|describe|run`, `policy check`, `action
    ledger|show|rollback`, `verify run`. *Check:* e2e in `tests/e2e/phase10_tools.rs`.
15. **Bench + docs.** `bench/tools/*` fixtures and `manual/module.md` per tool module. *Check:* fixtures
    load; `codex verify` stays green.

---

## 6. Logging and observability

All records go through `mm-log` with the global fields (§10 of the parent plan) and a `trace_id` ULID
shared with the event log and RDF provenance. **Audit records** (immutable, written transactionally with
the state change) unless marked operational.

| Event code | Level | Required fields |
|---|---|---|
| `tool.registry.register` | INFO | `tool`, `version`, `module_uri`, `sandbox_tier`, `reversibility`, `result` |
| `tool.invoke.start` | INFO | `tool`, `args_hash`, `principal`, `trace_id`, `action_id` (audit) |
| `tool.permission.check` | INFO | `tool`, `resource`, `decision`, `policy_effect`, `reason`, `principal` (audit) |
| `tool.policy.forbid` | WARN | `tool`, `policy_set`, `rule_id`, `principal` (audit) |
| `tool.sandbox.start` / `tool.sandbox.deny` | INFO / WARN | `tool`, `tier`, `caps_hash` / `capability`, `requested`, `allowed` (audit) |
| `tool.idempotency.hit` | INFO | `key`, `action_id`, `original_status` |
| `tool.invoke.end` | INFO | `tool`, `status`, `latency_ms`, `result_hash`, `error` (audit) |
| `action.ledger.append` | DEBUG | `seq`, `entry_hash`, `prev_hash`, `action_id` |
| `action.rollback.start` / `.end` | INFO | `action_id`, `snapshot_id`, `restored_hash` (audit) |
| `observation.record` | INFO | `observation_id`, `action_id`, `source`, `evidence_id`, `graph_iri` (audit) |
| `verify.run.start` / `.end` | INFO | `obligation`, `subject`, `status`, `proof_ref` (audit) |
| `verify.obligation.failure` | WARN | `obligation`, `subject`, `counterexample_ref` (audit) |
| `mcp.tools.list` / `mcp.tools.call` | INFO | `server`, `count` / `server`, `tool`, `principal`, `result` |

Every action emits start + end (success and failure); errors carry the full cause chain. Argument payloads
pass through `mm-log`'s `Redactor` (secrets/tokens/PII never emitted; free text length-bounded).
`mm-cli logs trace <trace_id>` reconstructs the full action timeline; `mm-cli logs verify` checks schema,
audit completeness, gapless chain, and redaction.

---

## 7. Testing

- **Unit.** Spec/hash stability; registry duplicate rejection; policy precedence (`forbid` > `permit` >
  deny); permission table (allow/deny/escalate/expired/revoked); sandbox capability failures per tier;
  per-tool behavior; idempotency hit returns the original outcome.
- **Property (`proptest`).** Ledger hash-chain verifies for arbitrary append sequences and fails for any
  mutation; canonical `ActionSpec` serialization is byte-stable across re-parses; policy evaluation is a
  pure function of its inputs.
- **Golden / replay (`insta`).** Recorded tool-call replay reproduces identical observations and ledger
  hashes; `mm-cli replay` yields the same state with logging on and off.
- **Adversarial (`bench/tools/`).** Missing grant → deny; grant scope too narrow → deny; explicit `forbid`
  overrides a permit; sandbox escapes (path traversal, undeclared network, oversize payload) → deny;
  fabricated observation → reject; tool-selection fixtures mirror ToolBench's selection/eval split.
- **End-to-end (`tests/e2e/phase10_tools.rs`).** propose → authorize → sandbox → execute → observe →
  ledger → rollback; plus an MCP `tools/call` round trip through `mcp.rs`.
- **Ontology.** SHACL valid/garbage fixtures for `/world` and `/provenance`.

---

## 8. Pass gate

Run as plain commands; earlier phases' gates must still pass.

```bash
cd ~/Build/metamind
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mm-tools

# Registry + tools present
mm-cli tool list --json | jq -e 'length >= 6 and (map(.name) | index("fs.read")) != null'

# Unauthorized call is denied by deterministic code (never by model judgment)
mm-cli tool run process.exec --arg cmd='cargo test' --principal "$BEING_ULID" ; test $? -ne 0

# Explicit forbid beats a permit
mm-cli policy check --principal "$BEING_ULID" --action fs.write --resource data/sandbox/x | grep -q deny

# Sandbox enforcement
mm-cli tool run fs.read --arg path=/etc/shadow            ; test $? -ne 0   # undeclared path
mm-cli tool run http.fetch --arg url=https://example.com  ; test $? -ne 0   # not allowlisted

# Authorized read succeeds and yields an observation + ledger entry
mm-cli tool run fs.read --arg path=data/sandbox/README.md ; test $? -eq 0
mm-cli action ledger --json | jq -e '.[-1].event == "observation.record"'

# Idempotency: repeating an executed action does not re-apply its effect
mm-cli tool run fs.write --arg path=data/sandbox/target.txt --arg text='hello' --idempotency-key k1 --json | jq -e '.status=="ok"'
mm-cli tool run fs.write --arg path=data/sandbox/target.txt --arg text='hello' --idempotency-key k1 --json | jq -e '.status=="ok" and .deduplicated==true'

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

**Objective criteria.** Unauthorized → non-zero + `tool.permission.check` deny record; explicit `forbid`
wins over a permit; sandbox violations → non-zero + `tool.sandbox.deny`; a repeated idempotency key
returns `deduplicated=true` with exactly one ledger effect; every successful action has exactly one
`observation.record` with an `evidence_id`; `action ledger --verify` reports an intact chain; rollback
restores the recorded hash; `codex verify` and `logs verify` pass.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| LLM-produced action treated as performed | Observations are created only by the executor from `ToolOutcome`; the epistemic layer rejects evidence-free observations |
| Permission bypass via tool composition | Authorization is re-checked per primitive call, not only at the action root; policy is data, versioned and audited |
| Sandbox escape | Capability guard is a hard error (no silent skip); tier 3 (microVM) for untrusted code; adversarial escape fixtures in the gate |
| Confused deputy (tool acts with wider rights than the caller) | Every call carries a `Principal`; effective permission is the intersection of caller scope, tool scope, and policy |
| Duplicate effect on retry | `IdempotencyStore` + `idempotency_keys`; exactly-once verified in the gate |
| Ledger tampering | Hash-chained, append-only; `verify_chain` runs in the gate and on open |
| Irreversible action without rollback | Tools declare `Reversibility`; `Irreversible` requires firewall escalation and explicit confirmation |
| MCP server supply-chain risk | External servers are ingested as `ToolSpec` behind the same `Tool` trait and the same default-deny policy; no ambient authority |
| Copied sink/tool code drifts | `mmc:copiedFrom` + `COPYING.md`; copied tools are re-tested under `mm-tools` CI |

---

## 10. References (code-available)

- Model Context Protocol — https://github.com/modelcontextprotocol/modelcontextprotocol (org: https://github.com/modelcontextprotocol)
- Wasmtime / WASI — https://github.com/bytecodealliance/wasmtime
- gVisor — https://github.com/google/gvisor
- Firecracker — https://github.com/firecracker-microvm/firecracker
- E2B — https://github.com/e2b-dev/E2B · microsandbox — https://github.com/team-microsandbox/microsandbox
- Open Policy Agent — https://github.com/open-policy-agent/opa · Cedar — https://github.com/cedar-policy/cedar
- Gorilla — https://github.com/ShishirPatil/gorilla · ToolBench/ToolLLM — https://github.com/OpenBMB/ToolBench
- Temporal — https://github.com/temporalio/temporal · Restate — https://github.com/restatedev/restate
- Sandbox surveys — https://github.com/restyler/awesome-sandbox · https://github.com/bureado/awesome-agent-runtime-security

Copied-in references (parent plan §7): `logic-planner::sandbox` → `mm-tools/src/sandbox/`;
`backend-registry`/`solver-ir` and `math-runtime` residual verifier → `verify.rs`; `llm-interface` tool
catalog pattern → `registry.rs`; nexus `monad-process`/`monad-file`/`monad-sparql`/`monad-sql`/`monad-resource`
→ concrete tools; `kg-validate` `EvidenceValidator`/`ClaimValidator` → observation→evidence validation.
Each copied tree gets `vendor/<origin>/COPYING.md` and `mmc:copiedFrom` provenance from the Phase 2 scan.
