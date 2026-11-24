# Unified Five-Phase Plan: Three-Pass Turns and the Process Pipeline

> **Status:** proposed implementation plan · **Date:** 2026-11-23 · **Baseline:** `integration/cme-rdf-backend` @ `b9e8a54` (2026-11-23)
> **Supersedes for execution:** the migration sections of [`three_pass_turn_sequence.md`](./three_pass_turn_sequence.md) (§15) and [`process_pipeline_system.md`](./process_pipeline_system.md) (§10). Those documents remain the normative descriptions of the two architectures; this plan is the single ordered path that delivers both.
> **Scope:** `mm-cme`, `mm-serve`, `mm-data`, `mm-prompt`, `mm-llm`, `mm-agent`, `mm-sm`, a new `mm-process` crate, `config/prompts/`, `frontend/`, `infodocs/`
> **Flags:** `MM_THREE_PASS_V2` (default `true` as of 2026-11-23; the rollout below was written while it was `false`), `MM_PROCESS_V1` (default `false`)

---

## 1. Executive summary

The system today executes one user turn as one model/tool loop inside `TurnExecutor::execute` and persists plans as revisioned RDF `mm:Plan` documents. This plan turns that into two composable layers:

1. **A turn is three passes** — Interpret (plan the checks), Process (do the work, produce the reply), Verify (run checks, update durable state, approve or retry). Pure conversation exits after Interpret.
2. **A process is a state machine over the same three-pass runner** — reusable templates, instanced queues, step execution, wait conditions, and plan→process compilation.

Both layers share one set of seams, built once in Phase 1: the **verification engine**, the **pass-scoped tool dispatcher**, the **prompt/strategy set**, the **budget envelope**, the **three-pass runner**, and the **agentstream pass events**. That shared substrate is why five phases are enough; neither source document requires a bespoke stack.

| Phase | Ships | User-visible change | Default flag state at exit |
|---|---|---|---|
| **1. Substrate** | Verification trait + LLM judge, 19-tool catalog with pass scopes, four new prompt strategies, turn budget envelope, clean-log projection, pass events, config | None | both `false` |
| **2. Interpret** | Interpret pass, fast path for pure conversation, default check injection, interpret streaming/telemetry | Pure chat exits after Interpret; action turns get explicit checks | `MM_THREE_PASS_V2=false` (tests on) |
| **3. Process** | Extracted Process pass with full 17-tool scope, `agent.run` confinement, retry error context, reply identity | None (behavior-preserving extraction) | `MM_THREE_PASS_V2=false` |
| **4. Verify** | Check execution, review call with staged writes, retry loop (max 5), three-pass orchestration, inspector timeline, dual-run record | Verified answers, honest failure when checks fail | `MM_THREE_PASS_V2=false` (default flip is a separate change after gates) |
| **5. Process pipeline** | `mm-process` crate, templates/instances/queues, 4-pass plan builder, plan→process tools, PROCESS.md, sub-processes/versioning/analytics | Durable multi-step work driven by chat or plans | `MM_PROCESS_V1=false` (default flip is a separate change after gates) |

**What "complete" means here:** every normative item in the two source documents is either delivered by a numbered workstream in this plan or explicitly listed as a non-goal in §15. The traceability matrix in §18 maps every source section to a phase and an acceptance test.

---

## 2. Baseline: what actually exists

This section is verified against the tree, not assumed. It is the contract the plan extends.

### 2.1 Turn path

- `crates/mm-cme/src/executor.rs:262` — `TurnExecutor::execute` runs planning → routing → speculative pre-warm → streaming → tool loop → finalize. It is one prompt (`cm.turn.v1`) and one model/tool loop.
- `crates/mm-cme/src/turn.rs` — `Turn` + `TurnEvent` state machine; `set_prompt`, `set_plan`, `route_selected`, chunk spawning.
- `crates/mm-cme/src/budget.rs` — ten `BudgetClass` values already match the design table (`quick`, `standard`, `deep`, `resume`, `plan`, `think`, `memory`, `frame`, `action`, `summarize`), with `BudgetLedger` reserve/settle/refund. Child ledgers are **isolated**, not reserved from a parent.
- `crates/mm-cme/src/subagent.rs` — `PlanService`, `ThinkService`, `IntrospectionService`, `read_only_schemas()`; `metamind.think` already returns `ThinkResult`/`ThinkFinding`/`ThinkEvidence`.
- `crates/mm-cme/src/prompt.rs` + `mm-prompt` — manifest-selected strategies, strict variables, hash-addressed `PromptInstance`, fail-closed `PROMPT_UNAVAILABLE`.
- `crates/mm-serve/src/cme.rs` — `CmeRuntime::run_turn`; `state.rs` `cme_executor()` builds the client, wraps it in `LlmService` for `llm_calls` accounting, and threads the routed model into the request (fixed 2026-11-23).

### 2.2 Tools

- `crates/mm-serve/src/tool_schemas.rs` — `TOOL_NAMES: [&str; 17]`, count-checked against `crates/mm-serve/schemas/tools.json`, strict parsing, shared envelope.
- `crates/mm-serve/src/cm_tools.rs` — the dispatcher (`call`), timeouts, action validation. Tool names: conversation (4), `agent.run`, data (7), metamind (5: plan/think/memory/log/act).
- `crates/mm-serve/src/metamind_tools.rs` — `metamind.plan` is `action`-based (`create|update|get|list`); `metamind.act` is action-based (`internal|introspection|plan_*|cancel`). These are **not** the `plan=`/`run=` signatures proposed in `process_pipeline_system.md` §7; see decision D10.

### 2.3 Data

- `crates/mm-data/src/agentstream.rs` — `EventType` (26 variants), append-only session events, `AgentstreamStore`.
- `crates/mm-data/src/plan.rs` — revisioned `Plan` (`id`, `session_id`, `goal`, `status`, `steps`, `current_step`, `revision`, `format`, `body`, `current_version`), `PlanStep`, `PlanRegistry` (`PLAN_REGISTRY_IRI`), `PlanControlRequest { expected_revision }`, `PlanFormat::{Markdown, Structured}`. Plan statuses: Draft/Active/Paused/Done/Cancelled/Failed/Stopped/Completed/Abandoned.
- Named graphs, Tantivy sync outbox, soft delete/supersede, canonical `mmd:` IRIs, and `mm-cli data verify` are established invariants.
- `mm-sm` — `StateMachine` (states/transitions/guards/actions/regions/history), `SmInstance`, `snapshot` module. Process execution can be built directly on it; no new state-machine engine is needed.
- `mm-metacog` **does not depend on `mm-cme`**; it is the cognitive controller (scan/tier/program), not a workflow runtime. See decision D9.

### 2.4 Constraints the plan must respect

| Constraint | Why it matters |
|---|---|
| `TOOL_NAMES` is a 17-entry, count-checked invariant | Adding two control tools must not silently break the catalog or its tests |
| One agentstream session per conversation, mandatory `main` trace | All passes must append to the same session, tagged by pass |
| Prompt text never appears in executor code | New passes need strategy/fragment files, not `format!` prompts |
| Budget exhaustion is a hard, honest `BUDGET` outcome | Retries and sub-agents must not borrow silently |
| RDF is authoritative; Tantivy is repaired from it | Process/plan writes go through `mm-data`, not a side store |
| Revision checks on every state write | Staged Verify writes must carry preconditions |
| `MM_CM_V3` and `MM_CME_ENABLED` already gate the CM path | New flags must compose with, not replace, them |
| Live provider quota is shared and finite | Verification and process steps multiply calls; budgets and fast path are required, not polish |

---

## 3. Target architecture

### 3.1 Turn: three passes

```
user turn
  │
  ├─ Interpret (quick budget, 6 tools)
  │     ├─ fast path: approve(success=true, reply) ──────────────► finalize
  │     └─ normal: VerifyCheck[] + ProcessContext
  │
  ├─ Process (standard budget, 17 tools incl. agent.run/think)
  │     └─ reply_text + tool emissions
  │
  └─ Verify (quick budget; Phase A checks, Phase B review)
        ├─ approve(success=true)  → commit staged writes → finalize
        ├─ approve(success=false) → discard staged writes → retry Process
        └─ retries exhausted      → VERIFY_EXHAUSTED (honest failure)
```

Invariants:

1. Exactly one **user-visible reply** per turn: the Process pass text (or Interpret fast-path text). Verify never rewrites it; it approves or rejects.
2. All passes append to the same agentstream session with `turn_id` + `pass`; only final approved replies enter the clean log.
3. Tool availability is enforced by the dispatcher, not only by the schema list. A model that hallucinates `metamind.verify` in Process gets a typed `tool_not_available` result.
4. Verification checks are turn-scoped. They are logged as events, not persisted as RDF facts.
5. Any pass can end the turn only through `metamind.approve` (Interpret `true`, Verify `true|false`); text alone is a protocol failure, repaired once, then failed.

### 3.2 Process: a state machine over the same runner

```
ProcessTemplate (RDF, versioned) ──instantiate──► ProcessInstance (URI, queue, data, session)
                                                   │
                          ┌────────────────────────┴──────────────────────┐
                          │ mm-sm states: Created → Running(step_n)       │
                          │   → Waiting(condition) → Running → … → Done   │
                          │   any → Paused / Failed                       │
                          └────────────────────────┬──────────────────────┘
                                                   │ each llm_tool_loop step
                                          ThreePassRunner (scoped tools,
                                          synthetic turn + process session)
```

A process step is not a second implementation of the turn loop. It calls the same `ThreePassRunner` with:

- a synthetic `Turn` whose `turn_id` is the step execution id;
- a `SessionContext` bound to the process instance session, not the conversation;
- a tool scope = `ProcessStep.tools_allowed ∩ ProcessScope`;
- a `ProcessContext` built from the step instructions, PROCESS.md, and bound data;
- no user-facing streaming except `user_input` steps, which pause the instance and surface a prompt.

### 3.3 Shared seams

| Seam | Owner | Used by |
|---|---|---|
| `VerificationBackend` registry + `LlmJudgeBackend` | `mm-cme::verification` | Turn Verify, step Verify, plan-builder Pass 4 |
| `PassScope` + dispatcher guard | `mm-cme` types, enforced in `mm-serve::cm_tools` | Every pass and step |
| Strategies `cm.interpret.v1` / `cm.process.v1` / `cm.verify.v1` / `cm.think.v1` | `config/prompts/` + `mm-prompt` | Turn passes, think sub-agent |
| `TurnEnvelope` + per-pass ledgers | `mm-cme::budget` | Turn, retries, think, steps, plan builder |
| Pass/process agentstream events + clean-log projection | `mm-data::agentstream` | UI, inspector, resume |
| `ThreePassRunner` | `mm-cme` | Conversation turns and process steps |
| `ProcessRegistry` + queue leases | new `mm-process` | `mm-serve`, scheduled sweeps |

### 3.4 Crate layering

```
mm-core ─► mm-data ─► mm-process ─► mm-serve
   ▲          ▲           ▲
   └─ mm-sm ──┴─ mm-cme ──┘
                 ▲
             mm-prompt/mm-llm/mm-agent
```

`mm-process` depends on `mm-cme` (runner), `mm-data` (RDF), `mm-sm` (states), `mm-prompt`, `mm-llm`, `mm-core`. Nothing depends on `mm-process` except `mm-serve` and tests. `mm-metacog` is untouched.

### 3.5 Resolved decisions and deviations from the source documents

The two source documents contradict themselves or each other in seven places, and leave several contracts implicit. This plan fixes them once, here. Any implementation that disagrees with this table is a defect.

| ID | Question | Decision | Rationale |
|---|---|---|---|
| **D1** | Which tools may Interpret call? (§3.2 says two; §12's matrix says nearly all) | `conversation.topic/depth/status/frame`, `metamind.verify`, `metamind.approve` = **6** | Fast path must still update conversation state; no data/action tools before the plan exists. `data.search` deferred (Q1) |
| **D2** | Which tools may Verify call? | 16: all work tools except `agent.run` and `metamind.think`, plus `metamind.approve` | Review must not launch new external/deep work; `think`-in-Verify deferred (Q2) |
| **D3** | How do the two control tools coexist with the 17-tool invariant? | Split `WORK_TOOL_NAMES` (17) + `CONTROL_TOOL_NAMES` (2) = 19; per-pass projection + dispatcher guard | Preserves the count-checked work catalog and makes scope violations observable instead of trusting prompts |
| **D4** | Verify updates state before it may reject the turn | **Staged Verify writes**: validate + journal in Verify, commit only on `approve(true)`, discard on reject. Process effects are real work: durable and journaled for reconciliation (see §7.3) | Bookkeeping must not land for a rejected turn; work already performed cannot be conjured away, so retries reconcile instead of pretending to roll back |
| **D5** | What does a retry re-run? | Process + Verify only; Interpret's checks are fixed; 5 retries (6 attempts), each reserving a budget pair up front | Re-planning checks mid-turn hides failures and multiplies cost |
| **D6** | Can the reviewer approve a failed check? | No: `VERIFICATION_OVERRIDE_DENIED` unless `verify.allow_override=true` (default false) | Prevents rubber-stamping; override is an explicit operator choice |
| **D7** | Does the think sub-agent get `agent.run`? (§7.3 yes; §12 no) | Yes, with read-only user data and no user-state writes; own session | Deep research needs the external door; writing user state from a thought is unsafe |
| **D8** | Where is structured output allowed? (§5.3 says judge only; process builder uses it too) | Judge + plan-builder classify/decode only; main passes use normal calls | One structured-output boundary per decision point, never in the reply path |
| **D9** | Which crate owns the process engine? (source says `mm-metacog`) | New `mm-process`, depending on `mm-cme` | `mm-metacog` has no `mm-cme` dependency; placing it there would invert the graph or create a cycle |
| **D10** | Adopt the proposed `plan=`/`run=`/`list=` tool signatures? | No: extend the existing action-based `metamind.plan`/`metamind.act` schemas additively | Back-compat with the 17-tool contract, existing clients, and phase tests |
| **D11** | Where do verification checks live? | Turn/step-scoped, logged as agentstream events; not RDF facts | They describe one attempt, not durable knowledge |
| **D12** | Does the fast path infer topic/depth? | No; only explicit `conversation.*` calls mutate state | Avoid silent state drift from one cheap call |
| **D13** | How does a process step execute? | `ThreePassRunner` with synthetic turn/session and `step.tools_allowed ∩ ProcessScope` | One verified execution path; no second loop implementation |
| **D14** | How is process durability guaranteed? | RDF authoritative; one supervisor per instance; leases; step execution ids; at-most-once completion | Crash recovery must not double-write or double-charge |
| **D15** | Are plan-derived templates persistent? | Ephemeral, retained 7 days after completion; `persist=true` makes them durable; marketplace is a non-goal | Keeps the process graph from filling with one-off plans |
| **D16** | Is a backend-service runtime in scope? | No: trait + empty registry + honest `backend_unavailable` | The source marks it future; a fake service is worse than an unavailable one |
| **D17** | When is the default check added? | Only when Interpret classifies the turn as an action and registers zero checks | Pure chat uses the fast path; explicit checks are always honored |
| **D18** | Default flag state? | Both new flags off at every phase exit; defaults flip only after the Phase 4/5 evidence | One-flag rollback per layer is the release contract |

---

## 4. Phase 1 — Substrate: verification, scopes, prompts, budgets, observability

**Status:** implemented 2026-11-23; both flags remain off. See [`infodocs/three-pass-substrate.md`](../infodocs/three-pass-substrate.md) for the shipped contract.

**Goal:** Land every shared contract with **zero behavior change** while both flags are off. Phase 1 is the only phase that may be merged without a route change.

**Entry criteria:** baseline `cargo test` green; a scripted `LlmClient` test double exists or is added here; provider quota is not required (all tests mocked).

### 4.1 Workstreams

| ID | Deliverable | Files | Size |
|---|---|---|---|
| P1.1 | Verification types + trait + registry | `crates/mm-cme/src/verification/{mod,types,backend,registry}.rs`, `lib.rs` | M |
| P1.2 | `LlmJudgeBackend` | `crates/mm-cme/src/verification/llm_judge.rs` | M |
| P1.3 | 19-tool catalog with pass scopes | `crates/mm-serve/src/tool_schemas.rs`, `crates/mm-serve/schemas/tools.json`, `cm_tools.rs` | M |
| P1.4 | Four prompt strategies + fragments + manifest | `config/prompts/applications/*.toml`, `config/prompts/fragments/*.toml`, `manifest.toml` | M |
| P1.5 | Turn budget envelope + per-pass ledgers | `crates/mm-cme/src/budget.rs` | S |
| P1.6 | Pass events + clean-log projection | `crates/mm-data/src/agentstream.rs` | M |
| P1.7 | Config loading + validation + flags | `config/three_pass.toml`, `mm-cme` config module, `mm-serve/src/feature_flags.rs` | S |
| P1.8 | Docs: contracts, operator note, index | `infodocs/`, `design/DESIGN_INDEX.md` | S |

### 4.2 P1.1 — Verification contract

```rust
pub struct VerifyCheck {
    pub label: String,
    pub description: Option<String>,
    pub details: Option<String>,
    pub backend: String,
    pub backend_params: serde_json::Value,
    pub confidence_threshold: f64, // default 0.5, clamped to [0,1]
}

pub struct VerificationResult {
    pub check_label: String,
    pub pass: bool,
    pub reasoning: String,
    pub confidence: f64,           // clamped to [0,1]
    pub backend: String,
    pub elapsed_ms: u64,
    pub error: Option<String>,     // backend/protocol failure, distinct from pass=false
}

#[async_trait]
pub trait VerificationBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn validate_config(&self, params: &serde_json::Value) -> Result<(), VerificationError>;
    async fn verify(
        &self,
        check: &VerifyCheck,
        process_output: &ProcessResult,
        context: &VerifyContext,
    ) -> Result<VerificationResult, VerificationError>;
}
```

Rules:

- `validate_config` runs when a check is registered, so a bad check fails the Interpret pass immediately (`VERIFY_CHECK_INVALID`) rather than at the end of a costly turn.
- `VerificationResult.pass` is `judge.pass && confidence >= threshold`; confidence is clamped, never rounded up. A backend error is a failed check with `error` set, not a panic and not a silent pass.
- Registry rejects duplicate ids; unknown backend at registration is a typed error. Backends are injected, not constructed inside the executor.
- Checks are capped: `max_checks = 8` per turn/step, `label` ≤ 64 chars, `details` ≤ 2000 chars, `backend_params` ≤ 8 KB. Exceeding is a protocol error.

### 4.3 P1.2 — LLM judge

- Template variables: `{{user_request}}`, `{{process_reply}}`, `{{check_details}}`, `{{state_summary}}`. Unknown variables fail rendering (same strictness as the prompt store).
- The judge call uses the **default structured-output path** in `mm-llm` with the schema supplied by the check; it runs through `LlmService` so `llm_calls` accounting and routing apply (the CM accounting regression from the dual-run must not be reintroduced).
- Judge model: `verification.backends.llm_judge.model` if set, else the routed turn model. Temperature 0.0. Output token cap 512.
- Prompt-injection guard: `check_details`, `process_reply`, and `state_summary` are fenced as data with an instruction to ignore embedded instructions; the judge schema has no free-form command surface.
- Timeout: 20 s per check (configurable); on timeout the check fails with `error = "timeout"`.
- Redaction: `details` may be quoted by the model; the judge prompt is subject to the existing redaction pass, and verification events store label/backend/threshold but **not** `backend_params` verbatim (secrets can leak into an Interpret-authored prompt).

### 4.4 P1.3 — Tool catalog and pass scopes

Split the catalog without breaking the 17-tool invariant:

```rust
pub const WORK_TOOL_NAMES: [&str; 17] = [ /* existing TOOL_NAMES verbatim */ ];
pub const CONTROL_TOOL_NAMES: [&str; 2] = ["metamind.verify", "metamind.approve"];
pub const ALL_TOOL_NAMES: [&str; 19] = /* concatenation, design order */;
```

`TOOL_NAMES` remains as an alias for `WORK_TOOL_NAMES` so existing callers and tests keep their meaning. `ToolCatalog::new` validates the embedded 19 against `ALL_TOOL_NAMES`, while `work_schemas()` filters to the 17 for legacy callers. New tests assert `WORK.len()==17 && CONTROL.len()==2 && ALL.len()==19`, that the embedded JSON contains exactly those names, and that `work_schemas()` still returns 17.

Scope matrix (the corrected version of `three_pass_turn_sequence.md` §12, which contradicts its own §3.2):

| Tool | Interpret | Process | Verify | Think |
|---|---|---|---|---|
| `conversation.topic` / `.depth` / `.status` / `.frame` | ✓ | ✓ | ✓ | ✗ |
| `agent.run` | ✗ | ✓ | ✗ | ✓ |
| `data.create` / `.update` / `.delete` | ✗ | ✓ | ✓ (staged) | ✗ |
| `data.query` / `.search` / `.schema` | ✗ | ✓ | ✓ | ✓ |
| `data.namespace` | ✗ | ✓ | ✓ (staged) | ✗ |
| `metamind.plan` | ✗ | ✓ | ✓ (staged) | ✗ |
| `metamind.think` | ✗ | ✓ | ✗ | ✗ |
| `metamind.memory` | ✗ | ✓ | ✓ (staged) | recall only |
| `metamind.log` | ✗ | ✓ | ✓ | ✓ |
| `metamind.act` | ✗ | ✓ | ✓ (staged) | ✗ |
| `metamind.verify` | ✓ | ✗ | ✗ | ✓ |
| `metamind.approve` | ✓ (true) | ✗ | ✓ (false/true) | ✓ |

Scope counts: Interpret 6, Process 17, Verify 16, Think 8.

Implementation:

- `PassScope { Interpret, Process, Verify, Think }` on the request/turn, carried into the dispatcher.
- The dispatcher checks `scope.allows(name, action)` **before** parsing arguments and returns `{ok:false, code:"tool_not_available", tool, pass}`. This is deliberately observable: it catches prompt drift and is asserted in tests.
- Action-level filtering: Think may call `metamind.memory` only with `action="recall"`; Verify staged writes are recognized by tool+action.
- Schema projection per pass means the model never sees a tool it cannot use; the dispatcher is the second line of defense.
- Control-tool arguments: `metamind.verify {checks, process?}` (`process` = `{task?, constraints?, suggested_tools?}`) and `metamind.approve {success, reply?, error?}`. The handlers live in `cm_tools.rs` but write to a per-turn `InterpretSession` / `VerifySession` object, not to conversation state directly.

### 4.5 P1.4 — Prompt strategies and fragments

| Strategy | Application file | Fragments (new) |
|---|---|---|
| `cm.interpret.v1` | `cm.interpret.v1.toml` | `interpret_contract`, `verification_guide`, `fast_path_criteria` |
| `cm.process.v1` | `cm.process.v1.toml` | `process_execution`, `tool_selection` |
| `cm.verify.v1` | `cm.verify.v1.toml` | `verify_contract`, `verification_review`, `state_update_guidance`, `retry_guidance` |
| `cm.think.v1` | `cm.think.v1.toml` | `think_contract`, `deep_analysis` |

Reuse existing `identity/*`, `state_ownership`, `budget_contract`, `honesty_contract`. No prompt literal may appear in Rust; a test greps the executor for the pass instructions' first sentence and fails on a match. Manifest selection stays explicit; unknown strategy is `PROMPT_UNAVAILABLE` at boot, not at turn time.

### 4.6 P1.5 — Budget envelope

```rust
pub struct TurnEnvelope {
    envelope: BudgetLimits,        // sum of the passes' allowances
    spent: BudgetSpent,            // rounds/steps/tokens/wall
    verified_retries: u32,         // counts against the retry cap, not the envelope
}
impl TurnEnvelope {
    pub fn ledger(&self, class: BudgetClass) -> Result<BudgetLedger, BudgetError>;
    pub fn absorb(&mut self, ledger: &BudgetLedger) -> Result<(), BudgetError>;
}
```

- Per-pass ledgers keep their class limits (Interpret/Verify `Quick`, Process `Standard`), but `absorb` debits the envelope and the envelope fails the turn with `BUDGET` when exhausted. Retries reserve another Process+Verify pair up front; if the envelope cannot cover a retry, the turn ends with `BUDGET`, not an unverified success.
- Default envelope: Interpret 32k + Verify 16k + Process 120k = 168k tokens, 6 retries max. Configurable in `three_pass.toml`; `MM_TURN_MAX_TOKENS` can only lower it.
- Think/agent.run child budgets are reserved from the **Process** ledger before the call, matching the source design's inheritance rule; unused reservation is refunded by `settle_tokens`.

### 4.7 P1.6 — Agentstream and clean log

New `EventType` variants (additive):

`PassStarted`, `PassCompleted`, `VerificationRegistered`, `VerificationResult`, `RetryScheduled`, `WriteStaged`, `WriteCommitted`, `WriteDiscarded` (Phase 1); `ProcessStarted`, `ProcessStepStarted`, `ProcessStepCompleted`, `ProcessWaiting`, `ProcessCompleted`, `ProcessFailed`, `QueueItemEnqueued`, `QueueItemLeased`, `QueueItemCompleted` (Phase 5, declared then but reserved now).

Every pass event carries `{turn_id, pass, attempt}`. Final replies are `AgentMessage` with provenance `pass` and metadata `{"final": true}`. Each model call still emits the existing `PromptBuilt` event; pass events are additive wrappers, so current trace readers keep working.

Clean log:

```rust
pub struct CleanTurn { pub role: Role, pub text: String, pub turn_id: String }
pub struct CleanLog { pub turns: Vec<CleanTurn>, pub truncated: bool }
```

Built by filtering the main trace for `UserMessage` + final `AgentMessage`, bounded to 4,000 tokens (most recent first, then reversed). Persisted nowhere new; rebuilt on resume from the agentstream, cached in `SessionContext`. Tool results, Interpret text, verification reasoning, and retry attempts never enter it.

### 4.8 P1.7 — Config and flags

`config/three_pass.toml`:

```toml
[interpret]
budget_class = "quick"
strategy = "cm.interpret.v1"
max_checks = 8

[process]
budget_class = "standard"
strategy = "cm.process.v1"

[verify]
budget_class = "quick"
strategy = "cm.verify.v1"
max_retries = 5
check_timeout_ms = 20000
max_parallel_checks = 4
allow_think = false

[envelope]
max_tokens = 168000

[verification.backends.llm_judge]
enabled = true
temperature = 0.0
max_output_tokens = 512

[think]
budget_class = "think"
strategy = "cm.think.v1"
```

Validation at boot: unknown strategy, unknown backend, `max_retries > 10`, `max_parallel_checks == 0`, or `max_checks > 16` fail startup with a typed error. Missing file = defaults; a present-but-invalid file never falls back silently. `MM_VERIFY_MAX_RETRIES` may only lower `max_retries`; it cannot raise it above the file's cap.

Flags added to `feature_flags.rs`: `MM_THREE_PASS_V2` (false), `MM_PROCESS_V1` (false), plus per-session override for tests only.

### 4.9 Tests and exit gate

- Unit: registry duplicates, unknown backend, threshold/clamp table, judge schema rejection, timeout mapping, template-variable strictness, prompt-injection fencing.
- Catalog: 17+2 counts, per-pass projections, dispatcher denial codes, action-level memory filtering.
- Budget: envelope absorb/shortfall, retry reservation, child reservation/refund, no negative balances.
- Agentstream: serde round-trip for new events, clean-log projection excludes internals, resume rebuild equals in-memory log.
- Regression: all existing suites unchanged; `MM_THREE_PASS_V2=false` path byte-identical behavior.

**Exit gate:** `cargo test -p mm-cme -p mm-serve -p mm-data -p mm-prompt --tests`, `cargo clippy ... -D warnings`, `cargo fmt --check`; no route/flag behavior change; docs updated.

---

## 5. Phase 2 — Interpret and the fast path

**Status:** implemented 2026-11-23; `MM_THREE_PASS_V2` remains off by default. See [`infodocs/three-pass-interpret.md`](../infodocs/three-pass-interpret.md).

**Goal:** Make Interpret a real pass, including the one-call fast path, default check injection, and pass-aware streaming. Process still runs the existing loop; Verify is stubbed to "approve" so the flag can be exercised end-to-end without changing outcomes.

**Entry criteria:** Phase 1 merged; `MM_THREE_PASS_V2` exists but is off.

### 5.1 Workstreams

| ID | Deliverable | Files | Size |
|---|---|---|---|
| P2.1 | `ThreePassRunner` skeleton + `PassOutcome` types | `crates/mm-cme/src/runner.rs` (new), `lib.rs` | M |
| P2.2 | `pass_interpret` + protocol repair | `runner.rs`, `crates/mm-cme/src/executor.rs` | L |
| P2.3 | Fast-path finalize | `runner.rs`, `turn.rs`, `cme.rs` | M |
| P2.4 | Pass context assembly + clean-log injection | `runner.rs`, `prompt.rs`, `state.rs` | M |
| P2.5 | Turn pass state/telemetry | `turn.rs`, `observability/` | S |
| P2.6 | SSE pass events (interpret) | `mm-serve` chat route, `frontend/src/lib/transcript/` | M |

### 5.2 P2.1 — Runner contract

```rust
pub enum PassKind { Interpret, Process, Verify, Think }

pub struct PassContext {
    pub kind: PassKind,
    pub scope: PassScope,
    pub clean_log: CleanLog,
    pub state_block: String,          // existing canonical [STATE]
    pub user_request: String,
    pub interpret: Option<InterpretResult>,
    pub process: Option<ProcessResult>,
    pub verification: Vec<VerificationResult>,
    pub retry: Option<RetryContext>,
    pub envelope: TurnEnvelope,
}

pub struct PassOutcome {
    pub text: String,                       // pass text (interpret thoughts, process reply, verify status)
    pub prompt_instance_ids: Vec<String>,
    pub tool_emissions: Vec<ToolCallEmission>,
    pub budget: BudgetSnapshot,
    pub finished: PassFinish,               // Approved/FastPath/Rejected/Exhausted etc.
}
```

`ThreePassRunner` is deterministic around the model: it owns prompt rendering, scope, budget, transcript accumulation, and event emission. The model only chooses text and tool calls. This is the unit process steps will call in Phase 5, so it must not depend on `CmeRuntime` or HTTP types.

**Context isolation is enforced by construction:** the runner builds each pass's `PassContext` from a whitelist. Interpret receives no Process/Verify fields; Process receives Interpret's plan but no verification results; Verify receives all three plus retry state. Tests assert that a leaked field is absent from the rendered prompt, not just unused by convention.

### 5.3 P2.2 — Interpret pass

Flow:

1. Render `cm.interpret.v1` with clean log + `[STATE]` + request + allowed tool schemas.
2. Loop up to `Quick` rounds:
   - `metamind.verify({checks, process?})` → validate every check through the backend registry, accumulate, and store the optional `process` context; return ack. At most `max_checks`; duplicate labels rejected. `process` carries `{task, constraints, suggested_tools}`; if omitted, the server derives `{task: user_request, constraints: [], suggested_tools: none}`. This is how `ProcessContext` travels without a fourth pass or a structured-output call — the source document specifies the output but no transport, so the control tool carries it.
   - `metamind.approve(success=true, reply)` → fast-path result; no further tool calls.
   - `conversation.*` → execute immediately (low-risk state writes); these are the only direct writes Interpret may make.
   - Any other tool → `tool_not_available`.
3. If the model emits text without verify/approve → one **protocol repair** turn: inject "You must call metamind.verify or metamind.approve"; a second failure ends the turn with `INTERPRET_PROTOCOL`.
4. If Interpret returned a normal turn (not fast path), zero checks are legal only as "action, no explicit checks": the server adds the default LLM-judge check (D17) and emits `VerificationRegistered` with `implicit:true`. A turn with neither verify nor approve is never legal.
5. Build `ProcessContext { task, constraints, suggested_tools, budget_hint }` from the verify payload or the derived fallback; `suggested_tools` is a hint only and is intersected with the Process scope.

Fast path criteria live in the prompt fragment, but the server enforces the shape: `approve(true)` must include a non-empty reply ≤ 8,000 chars; no other tool calls in the same round.

### 5.4 P2.3 — Fast path finalize

- Turn outcome: `Completed`, one model call, `quick` ledger only.
- Append `UserMessage`, Interpret `AgentMessage` (final), `PassStarted/Completed`, clean-log entry.
- Stream reply tokens as the assistant message; no "Interpret:" prefix in the user bubble.
- Update conversation state only through `conversation.*` calls the model made; no implicit topic guessing.
- Budget/ledger snapshot recorded; no verification events.

### 5.5 P2.4 — Context assembly

- `PromptContextProvider` gains `pass_context()` returning the clean log, scope, interpret/process/verification/retry data. The existing `PromptVariables::for_turn` stays for the legacy path.
- `[STATE]` is rebuilt before every model call in every pass (the current executor already does this every tool round; keep it).
- Clean log is injected as a distinct message section, not concatenated into the system prompt, so prompt-hash changes are attributable.

### 5.6 P2.5 — Turn telemetry

- `Turn` records `pass`, `attempt`, `pass_history` (kind, prompt hash, budget snapshot, outcome).
- `TurnEvent` gains `InterpretStart/InterpretDone/ProcessStart/ProcessDone/VerifyStart/VerifyDone/RetryRequested`; the V2 path dispatches them, the legacy path does not. The SM validation test asserts the V2 table is reachable and the legacy table is unchanged.
- Inspector `Status` projection gains pass fields; no frontend work beyond rendering in P2.6.

### 5.7 P2.6 — Streaming

SSE events: `pass_start {pass,turn_id,attempt}`, `pass_token {pass,token}`, `pass_complete {pass,finished,budget}`, `verification {label,pass,confidence}` (Phase 4). Fast-path tokens carry `pass:"interpret"` and `final:true`; normal Interpret tokens carry `final:false` and are rendered as a subdued status line, not appended to the reply bubble.

Frontend transcript reducer treats only `final:true` tokens as assistant content; unknown pass events are ignored (forward compatibility).

### 5.8 Tests and exit gate

- Greeting/thanks → exactly one model call, `FastPath`, no Process/Verify events.
- Action request → `verify` + `ProcessContext`, then existing Process loop runs once, then stub approve; reply unchanged in contract.
- Malformed interpret (text only, then text again) → `INTERPRET_PROTOCOL`; no partial reply committed.
- Interpret cannot call `data.*`, `agent.run`, `metamind.plan/act`; dispatcher denial asserted.
- `conversation.topic` write during fast path is visible in `[STATE]` next turn.
- Clean log has exactly one user + one assistant entry per turn, even after retries (Phase 4 covers retries).
- Legacy path (`MM_THREE_PASS_V2=false`) emits no new events and no extra calls.

**Exit gate:** all above plus existing suites; flag on only in tests; a shadow run (flag off) shows no latency/call-count change.

---

## 6. Phase 3 — Process pass and delegation

**Status:** implemented 2026-11-23; `MM_THREE_PASS_V2` remains off by default. See [`infodocs/three-pass-process.md`](../infodocs/three-pass-process.md).

**Goal:** Extract the existing execution loop into a reusable Process pass, confine `agent.run` to it, add retry error context, and make the reply identity explicit. No new model behavior when Interpret is stubbed.

**Entry criteria:** Phase 2 merged; scripted Process fixtures exist.

### 6.1 Workstreams

| ID | Deliverable | Files | Size |
|---|---|---|---|
| P3.1 | Extract `pass_process` from `execute` (mechanical, behavior-preserving) | `executor.rs`, `runner.rs` | L |
| P3.2 | `ProcessContext`/`RetryContext` injection | `runner.rs`, `prompt.rs` | M |
| P3.3 | Tool scope + `agent.run` confinement | `cm_tools.rs`, `runner.rs` | S |
| P3.4 | Reply identity + streaming | `runner.rs`, `executor.rs`, `chunk.rs` | M |
| P3.5 | Think delegation parity | `runner.rs`, `cm_tools.rs` | S |
| P3.6 | Legacy parity harness | `crates/mm-cme/tests/parity.rs` (new) | M |

### 6.2 P3.1 — Extraction rules

- Move the loop body (render → reserve → call → tools → re-render) into `ProcessPass::run(&mut self, ctx: &PassContext) -> Result<PassOutcome>`.
- `TurnExecutor::execute` keeps its legacy orchestration and calls `ProcessPass` with a `PassContext` whose `interpret = None`, `scope = Process`, `envelope = Standard` equivalent. The resulting prompt hash, tool order, and outcome for fixtures must be identical to the pre-extraction baseline; P3.6 snapshots this.
- `Turn` mutation (prompt instance, chunks, tool events) stays in the executor adapter, not the runner, so the runner remains usable by process steps that have no `Turn`.

### 6.3 P3.2 — Context injection

`ProcessContext` is rendered as a dedicated section:

```
[TASK]        one-sentence restatement from Interpret
[CONSTRAINTS] bullet list, each ≤ 300 chars, max 8
[TOOLS]       suggested subset (advisory)
[BUDGET]      class + remaining (advisory; enforcement is server-side)
[RETRY]       only on retry: attempt n/5, prior reply digest,
              failed checks with reasoning/confidence, and the exact error string
```

`RetryContext` never contains internal verification prompts or secrets; it contains check labels, reasoning, confidence, and the model-authored error. It is inserted as a user-role message prefixed `RETRY_CONTEXT` so the model can distinguish it from the user's request.

### 6.4 P3.3 — Delegation

- `agent.run` is available only in the Process scope; the existing handler remains the only external door (Hermes/Pi, one downstream turn, summary).
- `metamind.think` continues to call the existing `ThinkService`; Phase 4 adds the recursive three-pass mode behind `verify.allow_think`/`think.three_pass`.
- Sub-agent child budgets are reserved from the Process ledger; a child cannot exceed the parent's remaining allowance (P1.5).
- Tool emissions are returned to the Process transcript exactly as today; the runner records `ToolCall`/`ToolResult` with `pass:"process"`.

### 6.5 P3.4 — Reply identity

- The final assistant text from Process is the reply. No Verify rewrite, no post-hoc summarizer.
- If the Process loop exhausts budget or rounds without final text, the turn fails with `PROCESS_INCOMPLETE` (existing honest-failure contract); it must not synthesize a reply.
- Chunk spawning (`spawn_chunks`) happens once per turn, not per retry. Retry text replaces the pending bubble content; the frontend uses `attempt` to reset the streaming buffer.

### 6.6 P3.5 — Think parity

- `metamind.think` stays read-only against user data (existing `read_only_schemas()`), with its own agentstream session.
- Phase 3 keeps current single-service behavior and adds scope-aware argument validation; recursive three-pass is Phase 4 P4.7.

### 6.7 Tests and exit gate

- Golden transcript/hash fixtures for the legacy path before/after extraction (prompt hash, tool sequence, outcome).
- Process with 17 tools: `agent.run` succeeds, `metamind.verify/approve` denied, `data.create` allowed.
- Retry context appears only on attempt ≥ 2 and contains exactly the failed check labels.
- Budget exhaustion mid-loop → `BUDGET`, no reply, no verification.
- Child think budget cannot exceed remaining Process tokens.
- Legacy path unchanged; V2 path with stub Interpret/Verify produces the same reply for 3 fixtures.

**Exit gate:** parity fixtures green, all suites green, clippy/fmt clean; flag still off by default.

---

## 7. Phase 4 — Verify, staged writes, retry loop, and orchestration

**Status:** implemented 2026-11-23; `MM_THREE_PASS_V2` remains off by default and the live dual-run gate is open. See [`infodocs/three-pass-verify.md`](../infodocs/three-pass-verify.md) and [`infodocs/phase12-dual-run.md`](../infodocs/phase12-dual-run.md) §"Three-pass turn run".

**Goal:** Complete the turn architecture: run checks, review with staged writes, approve/retry, expose the timeline, and produce a real dual-run record.

**Entry criteria:** Phases 2–3 merged; provider quota available for the live gate (mocked tests do not need it).

### 7.1 Workstreams

| ID | Deliverable | Files | Size |
|---|---|---|---|
| P4.1 | Phase A check runner (parallel, timeouts, deterministic order) | `runner.rs`, `verification/` | M |
| P4.2 | Staged write journal | `runner.rs`, `cm_tools.rs`, `mm-data` write adapter | L |
| P4.3 | Phase B review + approve semantics | `runner.rs`, `cm_tools.rs` | L |
| P4.4 | Retry loop + exhaustion outcome | `runner.rs`, `executor.rs` | M |
| P4.5 | Three-pass orchestration + turn outcome mapping | `runner.rs`, `executor.rs`, `cme.rs` | M |
| P4.6 | SSE pass timeline + inspector | `mm-serve`, `frontend/src/components/inspector/`, `chat/` | M |
| P4.7 | Think recursive mode | `runner.rs`, `subagent.rs`, `cm_tools.rs` | M |
| P4.8 | Dual-run record + operator doc | `infodocs/phase12-dual-run.md`, `infodocs/operator-guide.md` | S |

### 7.2 P4.1 — Phase A

- Run checks concurrently with `max_parallel_checks` (default 4), preserving declaration order in the result vector.
- Per-check timeout (`check_timeout_ms`); a timeout/backend error becomes `pass:false, error:Some(...)` and is included in the review context. A failed check does **not** abort other checks.
- No checks → Phase A skipped, `verification = []`; the review call still happens (Verify is the state-update pass).
- Checks run against the final Process reply, the clean log, `[STATE]`, and the user request. They do not see the full Process transcript or tool arguments (avoids leaking secrets and reduces tokens).
- Verification events record label, backend, threshold, pass, confidence, elapsed; not raw judge prompts.

### 7.3 P4.2 — Staged writes

The core integrity decision (D4): **Verify may not mutate durable state before approval.**

- The dispatcher gains `WriteMode::{Immediate, Staged}`. Verify runs `Staged`.
- Read-only tools execute immediately. Mutating tools (`data.create/update/delete`, `data.namespace`, `metamind.memory` remember/forget, `metamind.plan` create/update, `metamind.act` non-control) validate arguments, execute **nothing**, and append an intent `{seq, tool, args, preconditions, staged_result_preview}` to `MutationJournal`.
- The tool result returned to the model says `staged:true` and echoes the intent, so the model does not believe the write landed.
- On `approve(true)`: replay intents in order through the real dispatcher. Each data write carries the revision observed at staging time; on `RevisionConflict`, retry the intent once against fresh state, then fail the turn with `VERIFY_COMMIT_FAILED` and retain the journal. No partial-commit claim: the turn outcome names the failed intent.
- On `approve(false)`: discard the journal, emit `WriteDiscarded` with count, and retry Process.
- Fast path never stages (no Verify); its `conversation.*` writes are immediate by design.

**Process effects are durable, not staged.** The Process pass does real work (`data.create`, `agent.run`, plan actions) and those effects remain if Verify rejects the turn. Pretending otherwise would either make Process unable to read its own writes or would hide side effects behind a fake transaction boundary. The rules are:

- The dispatcher records every mutating Process call in a `ProcessMutationLog` (tool, args digest, resulting URI/id, revision) attached to the turn attempt; this is observational, not a rollback log.
- `RetryContext` includes the prior attempt's effects so the model reconciles (`data.update`/supersede) instead of duplicating. `data.create` is expected to use the request's idempotency key where the schema provides one.
- On `VERIFY_EXHAUSTED`, the effects stay, the user-facing failure names what was changed, and the agentstream retains the full log. The system never claims nothing happened.
- Tests assert **no duplicate creation** across a retry for the reconciliation fixture, and assert that no Verify-journaled write was committed — not that the Process made no writes.

This resolves the source document's ambiguity where Verify updated state before possibly rejecting the turn, without pretending the Process pass is transactional.

### 7.4 P4.3 — Phase B

- Prompt `cm.verify.v1` sees clean log, `[STATE]`, user request, Process reply, verification results, retry count, and the staged-intent previews.
- Tools: 16 allowed (Verify scope), `metamind.approve`. `agent.run`, `metamind.think`, `metamind.verify` are denied.
- The model must finish with exactly one `metamind.approve`. Text without approve triggers one protocol repair, then `VERIFY_PROTOCOL`.
- `approve(true)`: commit journal, mark turn completed, append clean log.
- `approve(false, error)`: require non-empty error ≤ 2,000 chars; discard journal; schedule retry.
- If Phase A failed but the model approves anyway: server-side guard rejects the approval with `VERIFICATION_OVERRIDE_DENIED` unless `verify.allow_override=true` (default false). This prevents the reviewer from rubber-stamping hard failures.

### 7.5 P4.4 — Retry loop

- `max_retries` default 5 → at most 6 Process attempts per turn.
- Retry reserves a full Process+Verify budget pair up front. If the envelope cannot cover it, stop immediately with `BUDGET` (no half-retry).
- Re-run Process with `RetryContext`; re-run Phase A (the reply changed) and Phase B.
- Retry does **not** re-run Interpret; checks are fixed for the turn. Interpret is re-invoked only on a new user turn.
- Exhaustion outcome `VERIFY_EXHAUSTED`: the user receives an honest message ("I couldn't verify this answer after N attempts: <label> — <reason>"), the last unverified reply is **not** presented as the answer, and the agentstream retains every attempt. This follows the honesty contract: no fabricated success.
- Cancellation/interruption between attempts ends the turn `Interrupted`; staged writes are discarded.

### 7.6 P4.5 — Orchestration

```
execute_three_pass(turn, session):
  envelope = TurnEnvelope::for_turn(plan)
  interpret = pass_interpret(...)
  if fast_path: finalize(reply)
  for attempt in 0..=max_retries:
    process = pass_process(interpret.context, retry_context?)
    checks  = run_checks(interpret.checks)
    verify  = pass_verify(process, checks, staged)
    if verify.approved: commit; finalize(process.reply)
    else: discard; retry_context = ...
  fail(VERIFY_EXHAUSTED)
```

- `MM_THREE_PASS_V2=false` keeps the legacy `execute` byte-for-byte; the flag is read once per turn.
- Turn outcome mapping: fast path/approved → `Completed`; protocol/budget/commit → `Failed` with a typed code; retries exhausted → `Failed(VERIFY_EXHAUSTED)`; interrupted → `Interrupted`.
- Turn-level telemetry: attempts, per-pass tokens, judge calls, check pass rate, staged/committed/discarded counts, wall time.
- `llm_calls` accounting: every pass and judge call goes through `LlmService`; each row carries `pass`/`attempt` metadata (extend the accounting metadata, no schema break).

### 7.7 P4.6 — UI

Inspector Trace tab gains a pass timeline: Interpret (checks registered), Process (tool rounds), Verify (checks + review + retry), with prompt hashes, budgets, and confidence. Chat shows a single bubble with a subtle "verified" indicator; retries do not create extra bubbles, but the timeline shows them. No mock data; all fields come from events.

### 7.8 P4.7 — Think recursive mode

- Behind `think.three_pass=true`: `metamind.think` runs `ThreePassRunner` with Think scope, its own session `think/<parent_turn>/<ulid>`, read-only data, `agent.run` allowed, writes denied.
- Its Interpret uses a step-style default check ("does the finding answer the question with evidence?"); its Verify reviews and updates only the sub-agent's session, never user state.
- Falls back to the existing `ThinkService` when the flag is off; both return the same `ThinkResult` shape.

### 7.9 P4.8 — Validation record

- Extend `infodocs/phase12-dual-run.md` with a three-pass section: the same prompt set, legacy vs V2, per-pass call counts, tokens, cost, latency, verification outcomes, retry counts.
- Gate: on a quota-available key, 6/6 prompts complete in both modes; V2 fast path for pure chat is 1 call; action prompts run 3+ calls and at least one verified turn; no unaccounted calls in `llm_calls`.
- Quota failure is recorded as a limitation, never replaced with scripted latency.

### 7.10 Tests and exit gate

- Default check: action request with no Interpret checks → exactly one implicit check → Process → Verify → approve.
- Custom check failure → retry → success; assert Process attempts = 2, checks re-run, state written once.
- Five failures → `VERIFY_EXHAUSTED`; assert no Verify-journaled writes committed, no clean-log entry, and the Process effects are reported rather than erased.
- Staged rollback: rejected Verify review creates no bookkeeping rows; accepted review creates exactly the intended rows. Reconciliation fixture: a retried Process reuses/updates the prior attempt's entity, producing no duplicate.
- Revision conflict at commit → one retry, then `VERIFY_COMMIT_FAILED`; journal retained.
- Parallel checks preserve order; timeout is a failed check; backend error never passes.
- Override denied when a check fails and `allow_override=false`.
- Fast path makes exactly one call; no Verify events.
- Clean log contains only approved replies; retries never leak.
- Legacy path parity fixtures still green.

**Exit gate:** all above + live dual-run record + clippy/fmt; then `MM_THREE_PASS_V2=true` may become the default in a follow-up change (not in this phase's code).

---

## 8. Phase 5 — Process pipeline: templates, queues, plans, sub-processes

**Goal:** Deliver the entire process pipeline on the Phase 4 runner: RDF templates, mm-sm-driven instances, durable queues with parallel/batch consumption, the four-pass plan builder, plan→process integration, PROCESS.md, and the advanced lifecycle features. One phase, five workstreams, ordered by dependency.

**Status (2026-11-23):** implemented behind `MM_PROCESS_V1=false`. The `mm-process` crate, RDF store, queue/supervisor, builder, tools, expiry, sub-processes, versioning, analytics, inspector routes and Plan-tab process section are in place. Deliberate deviations and limitations are recorded in [`infodocs/process-pipeline.md`](../infodocs/process-pipeline.md). The live plan→process soak (and the Phase 4 dual-run) remains open before the flag can be enabled by default.

**Entry criteria:** Phase 4 merged and V2 enabled in staging; `MM_PROCESS_V1` off.

### 8.1 Workstreams

| ID | Deliverable | Files | Size |
|---|---|---|---|
| P5A | `mm-process` crate: templates, types, RDF storage, PROCESS.md | new crate, `mm-data/src/process.rs` | L |
| P5B | Instance state machines, queues, leases, parallel/batch, recovery | `mm-process/src/{instance,queue,engine}.rs` | L |
| P5C | Plan builder (4 passes) | `mm-process/src/plan_builder.rs`, prompts | L |
| P5D | Plan→process, tool surface, expiry, control, inspector | `mm-process`, `mm-serve`, `mm-data::plan`, frontend | L |
| P5E | Process types, sub-processes, versioning, analytics | `mm-process`, `mm-serve`, frontend | L |

### 8.2 P5A — Templates and storage

Types:

```rust
pub enum ProcessType { Conversational, LlmOperations, BackendService }
pub enum StepType { LlmToolLoop, WaitCondition, BackendCall, UserInput }

pub struct ProcessTemplate {
    pub uri: String,              // mmd:process/<ULID>
    pub label: String,
    pub description: Option<String>,
    pub process_type: ProcessType,
    pub steps: Vec<ProcessStep>,
    pub parallel_enabled: bool,
    pub batch: Option<BatchConfig>,
    pub process_doc: Option<String>, // relative path, confined to workspace
    pub version: u32,
    pub derived_from_plan: Option<String>,
    pub created_at: i64,
    pub created_by: String,
}

pub struct ProcessStep {
    pub uri: String,
    pub order: u32,
    pub label: String,
    pub instructions_template: String, // {{var}} whitelist only
    pub step_type: StepType,
    pub tools_allowed: Vec<String>,    // intersected with Process scope
    pub wait: Option<WaitCondition>,
    pub parallel_group: Option<String>,
    pub output_bindings: Vec<String>,
    pub timeout_ms: Option<u64>,
}
```

Validation (`validate_template`): 1–50 steps; unique order/URIs; `tools_allowed` ⊆ Process scope (control tools rejected); `llm_tool_loop` must have instructions; parallel group members must not depend on each other; bindings must be produced by an earlier or same-group step; template variables ⊆ `{target_path, instance_id, data.*}` resolved at instantiation; PROCESS.md ≤ 64 KB and confined to the workspace (no `..`, no absolute paths).

RDF: named graph `mm:process`; classes `mm:ProcessTemplate`, `mm:ProcessStep`, `mm:ProcessInstance`, `mm:ProcessQueue`, `mm:QueueItem`, `mm:StepExecution`; properties mirror the structs; `mm:revision` and `mm:contentHash` for optimistic concurrency. CRUD via `data.*` types (`process_template`, `process_instance`) so the existing schema/namespace/search machinery applies; soft delete only. New numbered migration, idempotent, plus a `data verify` extension for process graphs.

PROCESS.md: loaded at instantiation, injected as a bounded section into each step's ProcessContext; headings are informational, not parsed into steps (markdown plans are the parse path). A missing doc is a warning, not a failure.

### 8.3 P5B — Instance engine

- Build one `StateMachine` per template at instantiation: `Created`, `Step(n)` for each step, `Waiting(condition)`, `Paused`, `Completed`, `Failed`; transitions guarded by payload (step index, condition satisfied). Use `mm-sm` snapshots (`snapshot.rs`) for persistence; no custom state engine.
- `ProcessInstance` stores `uri`, `template_uri`, `template_version`, `state`, `current_step`, `data`, `history`, `agentstream_session` (`process/<template>/<instance>`), `revision`, `lease`, and an `InstanceConfig { parallel_enabled, batch, max_parallel_items }` copied from the template and overridable at instantiation (so one template can run a serial instance and a parallel instance).
- **Single supervisor:** `ProcessRegistry` ensures at most one executor per instance, like `SessionAutomation` does per session. Leases carry owner + deadline and are renewed by heartbeats; an expired lease is reclaimable.
- Crash recovery: on boot, scan `Running` instances. If a step completed but the transition was not persisted, the step is replay-safe because writes are journaled by step execution id (at-most-once completion). `Waiting` instances are re-registered with their condition. `Paused` stays paused.
- Step budget: each `llm_tool_loop` step has its own `TurnEnvelope` (default `standard`, overridable per step/template) independent of the conversation turn, recorded in `StepExecution`. A step cannot consume another step's or the conversation's allowance.
- Step execution: for `llm_tool_loop`, create a synthetic `Turn`/`SessionContext`, call `ThreePassRunner` with the step scope (the same `cm.*` strategies plus a `[STEP]` context block; separate `process.step.*` strategies from the source document are intentionally not duplicated, to keep one prompt path per pass), then:
  - extract `output_bindings` from staged/committed data writes and the step reply; missing bindings fail the step with `PROCESS_BINDING_MISSING`;
  - persist `StepExecution {step, attempt, reply_digest, bindings, verification, tokens, duration}`;
  - advance only on Verify approval; exhaustion marks the instance `Failed` with the failed step and attempt history.
- `wait_condition`: `ToolResult` (required keys present), `UserInput` (instance pauses; a chat message resumes it; one outstanding prompt per instance; the reply is bound as data without running Process/Verify unless the template sets `verify_user_input=true`), `ExternalSignal` (internal action/API callback), `Timeout` (deadline persisted; the scheduler resumes/fails it). Timeouts are evaluated by the existing scheduler tick, not a per-instance timer.
- `backend_call`: trait `BackendService { id, call(input) -> Result<Value> }` with an empty registry in Phase 5; calls return typed `backend_unavailable`, never a fake success. This is the only intentionally stubbed piece (see §15).

Queue:

- Persisted in RDF per instance; in-memory index for dequeue. `enqueue` writes RDF first, then memory; `dequeue` leases an item (owner, deadline), sets `processing`; completion removes it; lease expiry requeues.
- Modes: sequential FIFO; parallel up to `max_parallel_items` (default 4); batched up to `max_batch_size` (default 10) or `batch_timeout_ms` (default 5,000), optionally grouped by `batch_key`.
- Dedupe by `(instance, idempotency_key)`; duplicate enqueue returns the existing item. Priority is a stable secondary sort; starvation guard promotes an item older than `priority_age_ms`.
- Backpressure: queue depth cap per instance (default 1,000) → `QUEUE_FULL`, no silent drop.

### 8.4 P5C — Plan builder

Two to four passes, matching the source pipeline's "2-4 pass" shape, with the structured-output boundary corrected (D8):

| Pass | Output | Model class | Structured output | Runs for |
|---|---|---|---|---|
| 1 Classify | `plan_type` (flat/tree/markdown), complexity, estimated_steps, reasoning | quick | yes | all |
| 2 Generate | Markdown plan | standard | no | all |
| 3 Decode | flat/tree JSON | quick | yes | flat/tree only |
| 4 Verify-decode | faithful, reasoning, confidence | judge model | yes (judge schema) | flat/tree only |

The markdown path is a **two-pass build** (classify → generate) plus deterministic validation; it has no decoded structure to judge, so Passes 3–4 are skipped. Flat/tree run all four. The builder's Pass 4 reuses the judge **prompt/schema conventions and `StructuredOut` path**, not the turn-scoped `VerificationBackend` trait (whose inputs are a `ProcessResult` and `VerifyContext`). It is a plan-quality check, not a turn verification.

- Pass 2 uses Pass 1's `plan_type`/`complexity` as structural hints (simple sequence for flat, depth-bounded branches for tree, unconstrained for markdown).
- Validation after decode: flat deps acyclic and refer to earlier steps; tree depth ≤ 3; 1–50 steps; non-empty titles; unique order; markdown ≤ 64 KB. Invalid → retry the failing pass with the validator error appended (max 5), then fail with `PLAN_DECODE_INVALID`; no silent downgrade to markdown.
- Pass 1 invalid enum → one repair with the schema error, then fail.
- Builder calls go through `LlmService` for accounting and routing; prompts are `plan.builder.*.v1` strategies with strict variables.
- Output is a `PlanDraft { plan_type, markdown, steps, template }`, persisted via the existing `Plan` store (P5D) — the builder never writes RDF directly.
- Idempotency: `(session_id, request_hash)` returns the existing plan unless `force=true`.

### 8.5 P5D — Plans, tools, expiry, control

Extend the existing `mm:Plan` rather than introducing a parallel plan graph (D10):

- `Plan` additions: `structure: PlanStructure {Flat,Tree,Markdown}`, `description: Option<String>` (`goal` remains the label), `process_uri: Option<String>`, `expires_at: Option<i64>`, `never_expires: bool`, `doc_path: Option<String>`, `derived_template_uri: Option<String>`. Statuses add `Expired`, `Archived` (additive; `is_terminal`, exhaustive matches, and `serde` round-trip tests updated).
- `PlanFormat::{Markdown,Structured}` stays for content; `structure` is the execution shape. A markdown plan can compile to a process; a structured plan is already steps.
- Migration maps existing plans to `structure = markdown|flat` based on format and keeps `expires_at = None` (grandfathered) to avoid mass expiry.

Tool changes (additive, schema version 1.1, backward compatible):

- `metamind.plan`: keep `action=create|update|get|list`. Add `structure` (`flat|tree|markdown|auto`), `run` (bool), `never_expires` (bool), `limit` (list), and `last` (bool). `create` with no `content` invokes the builder (four passes for flat/tree, two for markdown); `run=true` compiles and starts a process. `list` returns `{plans, last_plan}` and excludes expired unless `include_expired=true`; `last=true` returns only the most recent non-expired plan.
- `metamind.act`: keep existing actions and controls. Add `process_start|process_pause|process_resume|process_cancel|process_retry_step` plus fields `template`, `process_id`, `data`, `run`. `plan_start` returns a process URI when `MM_PROCESS_V1=true`; with the flag off it keeps the current plan-monitor behavior.
- `metamind.verify`/`approve` remain control tools, not available outside their scopes.
- All new fields are optional and unknown-field rejection stays on; the 19-tool count test is unchanged.

Plan→process compilation:

- flat → sequential steps; tree → DFS with sibling parallel groups when `parallel_enabled`; markdown → heading sections become steps in order, with a synthesized final "report" step.
- Dependencies become guard order; conflicting dependencies fail compilation with `PLAN_STRUCTURE_INVALID`.
- Derived templates are ephemeral by default, retained 7 days after completion, and linked `derivedFromPlan`; `persist=true` on `metamind.plan` makes them durable.

Expiry:

- Plans: 24 h default (configurable), `never_expires` bypass; expired plans are hidden from `list` and become `Expired`; a daily sweep archives them (retains data, emits an event). No deletion.
- Instances: conversational 1 h inactivity default; LLM operations no expiry; backend per template. Sweep uses the existing `ScheduledActionService`/cron path, not a new scheduler.
- Expiry is idempotent and revision-checked; a sweep racing an execution loses and retries next tick.

Inspector: Process tab (templates, instances, queue depth, current step, leases), Plan tab extension (structure, expiry, linked process, control buttons), Trace links to step sessions.

### 8.6 P5E — Types, sub-processes, versioning, analytics

- Conversational process: `user_input` steps pause and emit a frame/message; resume binds the user reply into `data`. One outstanding prompt per instance; expiry uses the inactivity timeout.
- Backend service process: `backend_call` steps use the empty registry/stub; the plan documents the contract and the honest `backend_unavailable` behavior.
- Sub-processes: a step may reference a template URI; depth cap 5, cycle detection at validation, child budget reserved from the parent step, child session linked by `parent_step`. A child failure follows the template's failure policy (`fail_fast` default, `collect` optional).
- Versioning: template edits create a new `mm:revision`; instances pin `template_version`; in-flight instances finish on their pinned version; rollback creates a new version equal to the old content (never rewrites history). Content hash is checked on load.
- Analytics: deterministic SPARQL aggregations (instances by state, step durations, retry rates, verification failure labels) surfaced in the inspector; no ML, no fabricated scores.
- PROCESS.md editing/import: template export/import as JSON+RDF; marketplace/sharing is a non-goal.

### 8.7 Tests and exit gate

Unit: template validation (each rejection reason), state-machine transitions and guards, lease reclaim, dedupe/priority, batch timeout, binding extraction, plan decode validation (cycles/depth), expiry idempotency, version pinning.

Integration:

- Chat → plan (builder) → process → 3 steps (one parallel group) → report; assert RDF template/instance/queue and step sessions.
- Queue: 25 items, parallel 4 → no double-processing, order preserved per item; batch 10/5 s groups correctly.
- Wait: tool-result condition, user-input resume, timeout failure; pause/resume/cancel.
- Crash recovery: kill between step completion and advance → replay-safe, no duplicate writes.
- Sub-process: depth 2 completes; depth 6 rejected; cycle rejected.
- Expiry: plan hidden after 24 h, `never_expires` survives, sweep archives once.
- Backend stub: `backend_unavailable`, instance fails honestly.
- Inspector: process timeline renders from events, no mock data.

**Exit gate:** `cargo test -p mm-process -p mm-serve -p mm-data -p mm-cme --tests`, e2e suites, `frontend` typecheck/test/build, clippy/fmt; `mm-cli data verify` clean on a fresh root and on a migrated root; operator runbook reviewed.

---

## 9. Cross-phase contracts

### 9.1 Agentstream events (wire)

```json
{"type":"pass_started","turn_id":"01T","pass":"interpret","attempt":0}
{"type":"pass_completed","turn_id":"01T","pass":"interpret","finished":"normal","rounds":1,"tokens":1840}
{"type":"verification_registered","turn_id":"01T","check":"default_action_verification","backend":"llm_judge","threshold":0.5}
{"type":"verification_result","turn_id":"01T","check":"default_action_verification","pass":false,"confidence":0.42,"elapsed_ms":830,"error":null}
{"type":"retry_scheduled","turn_id":"01T","attempt":1,"max_retries":5,"reason":"..."}
{"type":"write_staged","turn_id":"01T","seq":1,"tool":"data.create","revision":7}
{"type":"write_committed","turn_id":"01T","seq":1,"tool":"data.create","uri":"mmd:note/01N"}
{"type":"write_discarded","turn_id":"01T","count":2}
```

Process events use the same envelope with `process_id`/`step_id` instead of `turn_id`; step sessions carry both.

### 9.2 Error taxonomy

| Code | Meaning | Turn outcome |
|---|---|---|
| `PROMPT_UNAVAILABLE` | strategy/fragment missing or variable unknown | Failed |
| `INTERPRET_PROTOCOL` | no verify/approve after one repair | Failed |
| `VERIFY_CHECK_INVALID` | unknown backend/invalid params/too many checks | Failed |
| `VERIFY_PROTOCOL` | no approve after one repair | Failed |
| `VERIFY_EXHAUSTED` | retries exhausted | Failed (honest message) |
| `VERIFICATION_OVERRIDE_DENIED` | approve(true) with a failed check and override off | Failed/retry |
| `VERIFY_COMMIT_FAILED` | staged write failed after retry | Failed (journal retained) |
| `PROCESS_INCOMPLETE` | no final Process text | Failed |
| `TOOL_NOT_AVAILABLE` | scope violation | Tool result, not turn failure |
| `BUDGET` | envelope/ledger exhausted | Failed |
| `PROCESS_BINDING_MISSING`, `PROCESS_STEP_TIMEOUT`, `PROCESS_FAILED` | step/instance failures | Instance Failed |
| `QUEUE_FULL` | backpressure | Tool result |
| `backend_unavailable` | stub backend service | Step Failed |
| `PLAN_DECODE_INVALID`, `PLAN_STRUCTURE_INVALID` | builder/compile failure | Tool result |

### 9.3 Observability

Metrics per pass: calls, rounds, tokens in/out, cost micros, wall time, failures by code; verification: checks run, pass rate, judge confidence histogram, retries, override attempts; process: instances by state, queue depth/age, step duration, lease reclamations, sub-process depth. These are counters/histograms emitted alongside existing telemetry; no new external service is required.

### 9.4 Configuration summary

`three_pass.toml` (§4.8), `process.toml`:

```toml
[process]
default_parallel_enabled = false
default_max_parallel_items = 4
default_batch_max_size = 10
default_batch_timeout_ms = 5000
default_step_timeout_ms = 300000
conversational_inactivity_timeout_ms = 3600000
queue_depth_cap = 1000
max_subprocess_depth = 5

[plan]
default_structure = "auto"
default_expires_hours = 24
markdown_plans_dir = "data/plans/"
max_decode_retries = 5
max_steps = 50
max_tree_depth = 3

[plan.builder]
classify_model = ""      # empty = routed model
generate_model = ""
decode_model = ""
judge_model = ""
```

Empty model means "use the routed turn model"; the judge falls back to `verification.backends.llm_judge.model`.

---

## 10. Testing and quality strategy

| Layer | What | When |
|---|---|---|
| Unit | Types, validation, clamp/threshold, scope matrix, budget envelope, template validation, plan decode validation | Every phase |
| Contract | Tool schemas (19 count, strict parsing), event serde, error codes, prompt rendering | Every phase |
| Integration (scripted LLM) | Pass flows, retries, staged writes, process transitions, queue modes, plan builder with fixture responses | P2–P5 |
| Property | Budget monotonicity; clean-log coherence; retry termination; journal commit/discard symmetry; template/instance revision invariants | P1, P4, P5 |
| E2E (real server, mock model) | Chat → fast path; chat → verified action; plan → process; recovery; inspector | P4, P5 |
| Live | Dual-run legacy vs V2; one plan→process run | P4/P5, quota permitting |
| Frontend | Reducer tests for pass events; inspector render tests; typecheck/build | P2, P4, P5 |

Determinism rules: scripted clients keyed by prompt hash + call index; clocks injected for lease/expiry tests; no `sleep`-based assertions except the existing timing tests; all new timing tests go through the clock abstraction.

Regression gates: `cargo fmt --all -- --check`; `cargo clippy -p <touched crates> --all-targets -- -D warnings`; crate test suites; e2e suites. Phase 4 adds the dual-run record; Phase 5 adds `mm-cli data verify` on fresh and migrated roots.

---

## 11. Rollout, compatibility, rollback

### 11.1 Flags

| Flag | Default | Evidence gate (then a separate default flip) | Rollback |
|---|---|---|---|
| `MM_THREE_PASS_V2` | false | Phase 4 exit + dual-run + soak | Per-turn legacy path; no data migration involved |
| `MM_PROCESS_V1` | false | Phase 5 exit + plan→process e2e + soak | `metamind.plan/act` keep legacy semantics; process rows remain readable |
| `MM_CM_V3` | true (existing) | already on | Existing one-flag rollback |
| `MM_CME_ENABLED` | false (existing) | unchanged | — |

Flags are read once per turn/instance, logged in the agentstream `Lifecycle`/`ProcessStarted` event, and exported in the status projection so a dual-run is attributable.

### 11.2 Compatibility

- Legacy `execute` remains compiled and tested until a separate removal decision. Phase 3's parity fixtures are the guard.
- Tool schemas are additive; `TOOL_NAMES` semantics preserved; clients that ignore new fields keep working.
- Plan statuses are additive; exhaustive matches are updated in the same change; old plans get `structure` inferred and no expiry.
- Agentstream readers ignore unknown event types (already the case via `Custom`/serde defaults); new variants are additive.
- Process RDF is a new named graph; `data verify` learns it; no rewrite of existing graphs.
- No migration deletes or rewrites legacy `urn:metamind:*` triples.

### 11.3 Rollback

- Phase 1–2: code revert only; no persistent artifacts.
- Phase 4: disabling the flag restores one-pass turns; verification/write events remain as history, staged writes were never committed on failure.
- Phase 5: disabling `MM_PROCESS_V1` stops new process starts; running instances are paused (not deleted) by the supervisor, resumable when re-enabled. Plans remain valid RDF. A template/instance revision rollback is data-level (new version), not a code rollback.

---

## 12. Performance and cost targets

These are **targets to validate**, not measured results. The dual-run must confirm or reject them.

| Path | Calls | Target p50 | Target tokens/turn | Notes |
|---|---|---|---|---|
| Fast path (greeting/thanks) | 1 | ≤ 1.5 s | ≤ 2.5k | Interpret only; no tools |
| Simple action (one lookup + reply) | 3 | ≤ 5 s | ≤ 12k | Interpret + Process + Verify review; judge call optional |
| Verified action (default check) | 4 | ≤ 7 s | ≤ 16k | Adds one judge call |
| Retry | +2 per attempt | +4 s/attempt | +10k/attempt | Bounded by envelope |
| Process step (LLM ops, no checks) | 3 | ≤ 20 s | ≤ 14k | Step context is smaller than a conversation turn |
| Plan builder | 4 | ≤ 15 s | ≤ 10k | Classification/decode are small |

Cost control levers, in order: fast path; skip Phase A when no checks; judge only when a check exists; `suggested_tools` to shrink tool schemas; per-pass model overrides; envelope caps. The shared provider quota observed on 2026-11-23 makes this non-negotiable.

---

## 13. Security, privacy, and integrity

- Verification `details`/`evaluation_prompt` are model-authored and untrusted: size-capped, fenced, redacted, and never stored verbatim in events.
- Staged Verify writes prevent a rejected turn from adding bookkeeping/memory records; Process effects are journaled and reconciled, never duplicated on retry. Commit requires `approve(true)` and revision preconditions.
- Think scope is read-only against user data; no `data.create/update/delete`, no `conversation.*`.
- Tool scope enforcement is server-side; prompt text is not a security boundary.
- PROCESS.md and plan markdown paths are workspace-confined; template instructions cannot reference absolute paths.
- Process instances run under the same user/workspace confinement as `agent.run`; backend service calls are unavailable until a real registry with authorization exists.
- Plan/template export excludes secrets; redaction applies to stored step transcripts.
- No fabricated success anywhere: budget, verification, commit, backend, and queue failures all surface typed outcomes.

---

## 14. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| 3–5× calls make turns slow/expensive | High | High | Fast path; envelope; skip empty checks; per-pass models; measure in P4 dual-run |
| Verification is lenient or rubber-stamps | Medium | High | Thresholds, override denial, judge temperature 0, prompt-injection fencing, eval set of known-bad replies |
| Reviewer blocks good answers | Medium | Medium | Default check is narrow; retry with concrete error; measure false-reject rate before default-on |
| Staged write replay conflicts | Medium | Medium | Preconditions, one retry, explicit `VERIFY_COMMIT_FAILED`, journal audit |
| Process engine duplicates work after crash | Medium | High | Leases, step execution ids, at-most-once completion, recovery tests |
| Queue starvation/backpressure surprises | Low | Medium | Priority aging, depth cap, metrics, batch timeouts |
| Prompt drift between passes | Medium | Medium | Strategies in TOML, hash in events, golden render tests, no literals in Rust |
| Tool schema growth degrades selection | Medium | Medium | Scope projection, suggested tools, catalog count test, eval on tool-selection accuracy |
| Plan builder silently degrades structure | Low | Medium | Validation + explicit failure; no downgrade |
| Quota exhaustion blocks validation | High | Medium | Mock-first gates; live gate recorded honestly with rerun procedure |
| Process scope creep (marketplace, backend runtime) | Medium | Low | Non-goals §15; stub contract only |

---

## 15. Non-goals and deferred items

1. **Backend service runtime** (real registry, auth, callbacks): Phase 5 delivers the trait and honest `backend_unavailable` only.
2. **Marketplace/sharing** of templates: export/import JSON+RDF only.
3. **Multi-agent orchestration** beyond `agent.run` and think sub-agents.
4. **Voice changes**: the three passes work for voice turns, but prosody/barge-in behavior is unchanged.
5. **Learned verification policies / policy genome**: verification backend selection stays configured, not learned.
6. **Automatic promotion** of `MM_THREE_PASS_V2`/`MM_PROCESS_V1` defaults: separate change after the dual-run and a soak period.
7. **Removal of the legacy `execute` path**: after one release of dual-run evidence.
8. **Verification result caching** (source open question 5): deferred; idempotency keys on checks are a prerequisite.
9. **Interpret access to `data.search`** (source open question 1): deferred to an eval; if check quality suffers, add read-only data tools to Interpret behind config.
10. **Think inside Verify** (source open question 2): deferred; `verify.allow_think=false` stays until re-evaluated.

---

## 16. Open questions with decision points

| # | Question | Decision point | Default if unresolved |
|---|---|---|---|
| Q1 | Does Interpret need read-only data lookup to design good checks? | End of Phase 4 eval | No (6-tool scope) |
| Q2 | Should Verify be allowed `metamind.think`? | Phase 5 planning | No |
| Q3 | Cache idempotent check results? | Post-Phase 5 | No |
| Q4 | Retry budget: fixed 5 or stakes-based? | Phase 4 telemetry | Fixed 5, envelope-bound |
| Q5 | Per-pass model overrides (cheap Interpret/judge)? | Phase 4 cost review | Routed model everywhere; judge configurable |
| Q6 | Process step failure policy default | Phase 5 template review | `fail_fast` |
| Q7 | Do approved fast-path turns update topic/depth automatically? | Phase 2 eval | Only via explicit `conversation.*` calls |

---

## 17. Phase dependency and sequencing

```
P1 ──► P2 ──► P3 ──► P4 ──► P5
 │                    │
 └──► P5A/P5C (types and plan builder can start after P1; integration waits for P4)
```

- P2 and P3 can overlap if the runner seam (P2.1) lands first; P3's parity harness is independent.
- P5A/P5C are mostly additive and can be developed in parallel with P3/P4, but P5B/P5D/P5E require the P4 orchestration.
- Each phase is a separate merge train with its own flag state; no phase flips a default.

---

## 18. Traceability matrix (completeness check)

### 18.1 `three_pass_turn_sequence.md`

| Source section | Delivered by | Acceptance |
|---|---|---|
| §1 pass flow/principles | P2.1–P2.4, P3.1, P4.5 | Orchestration e2e |
| §2 clean log + per-pass contexts | P1.6, P2.4 | Clean-log property test; context fixtures |
| §3 Interpret (tools, checks, default, output) | P2.2, P4.1 | Interpret tests; default-check test |
| §4 Process (17 tools, loop, outputs) | P3.1–P3.5 | Process tests; parity |
| §5 Verify (backend trait, judge, Phase A/B) | P1.1–P1.2, P4.1–P4.3 | Judge tests; verify tests |
| §6 fast path | P2.3 | One-call test |
| §7 think sub-agent | P3.5, P4.7 | Recursive think test |
| §8 budgets | P1.5, P3.3 | Envelope property tests |
| §9 retry logic | P4.4 | Retry/exhaustion tests |
| §10 streaming/aggregation | P2.6, P4.6 | Reducer tests; inspector |
| §11 prompt strategies | P1.4 | Render/hash tests |
| §12 tool visibility | P1.3 | Scope matrix tests |
| §13 agentstream logging | P1.6, P4.5 | Event/serde tests |
| §14 configuration | P1.7 | Boot validation tests |
| §15 migration phases | This plan (P1–P4) | Phase gates |
| §16 testing strategy | §10 of this plan | Suite green |
| §17 affected files | §19 file map | Review |
| §18 rollback | §11 | Flag-off tests |
| §19 open questions | §16 | Decisions recorded |
| §20 references | Unchanged | — |

### 18.2 `process_pipeline_system.md`

| Source section | Delivered by | Acceptance |
|---|---|---|
| §1 concepts/lifecycle | P5A, P5B | Template/instance e2e |
| §2 templates, steps, PROCESS.md | P5A | Validation + PROCESS.md tests |
| §3 instances, queue, mm-sm engine | P5B | State/queue/recovery tests |
| §4 three process types | P5E (backend stubbed) | Conversational + LLM ops e2e; backend honest failure |
| §5 plan→process, builder, storage | P5C, P5D | Builder tests; plan compile tests |
| §6 expiration/cleanup | P5D | Expiry sweep tests |
| §7 tool integration | P5D | Schema/back-compat tests |
| §8 RDF graph | P5A | Graph/query tests; `data verify` |
| §9 configuration | P5 config | Boot validation |
| §10 migration phases | This plan (P5A–P5E) | Phase gates |
| §11 testing | §10 of this plan + §8.7 | Suite green |
| §12 affected files | §19 | Review |
| §13 references | Unchanged | — |

### 18.3 Source contradictions resolved

| Contradiction | Resolution |
|---|---|
| 3-pass §3.2 "Interpret has only 2 tools" vs §12 matrix "all 19 except…" | D1: 6-tool Interpret scope; matrix is authoritative only after correction |
| 3-pass §12 "Think has no `agent.run`" vs §7.3 "Think searches web via Hermes" | D7: Think may use `agent.run`, no user-data writes |
| 3-pass §5.3 "only judge uses structured output" vs process builder passes 1/3/4 | D8: structured output is confined to judge + builder classify/decode |
| Process §10 "mm-metacog" vs dependency direction | D9: new `mm-process` crate |
| Process §7 tool signatures (`plan=`, `run=`, `list=`) vs existing action-based schemas | D10: additive fields/actions, back-compatible |
| Verify updates state before approve can reject | D4: staged writes with commit/discard |
| Process §5.3 "ephemeral or persistent" templates | P5D: ephemeral by default, 7-day retention, `persist=true` |

---

## 19. Affected file map (consolidated)

| Area | Files |
|---|---|
| Verification | `crates/mm-cme/src/verification/*` (new), `crates/mm-cme/src/lib.rs` |
| Runner/passes | `crates/mm-cme/src/runner.rs` (new), `executor.rs`, `turn.rs`, `prompt.rs`, `subagent.rs`, `budget.rs` |
| Tools | `crates/mm-serve/src/tool_schemas.rs`, `crates/mm-serve/schemas/tools.json`, `cm_tools.rs`, `metamind_tools.rs` |
| Events/data | `crates/mm-data/src/agentstream.rs`, `plan.rs`, `process.rs` (new), migrations |
| Prompt | `config/prompts/manifest.toml`, `applications/*.v1.toml`, `fragments/*.toml` |
| Config/flags | `config/three_pass.toml` (new), `config/process.toml` (new), `mm-cme` config, `mm-serve/src/feature_flags.rs`, `state.rs`, `cme.rs` |
| Process crate | `crates/mm-process/Cargo.toml`, `src/{lib,template,instance,queue,engine,plan_builder,backend}.rs` |
| Frontend | `frontend/src/lib/transcript/*`, `components/chat/*`, `components/inspector/*` |
| Docs | `design/DESIGN_INDEX.md`, `infodocs/phase12-dual-run.md`, `infodocs/operator-guide.md`, `infodocs/components-and-status.md`, new `infodocs/three-pass.md`, `infodocs/process-pipeline.md` |

---

## 20. Definition of done

A phase is done only when all of the following hold:

1. Every workstream in the phase has a merged change behind the phase flag.
2. The phase's tests (unit/contract/integration/property/e2e as applicable) pass, and the existing suites still pass.
3. `cargo fmt --all -- --check` and clippy with `-D warnings` are clean for touched crates.
4. New events, errors, flags, and config are documented in `infodocs/` and indexed.
5. No prompt literals in Rust; no mock frontend data; no silent fallbacks.
6. The flag-off path is unchanged and covered by at least one test.
7. A rollback has been exercised (flag off) or explicitly shown to be code-only.
8. The phase's acceptance evidence (test names, dual-run numbers or an explicit quota limitation) is recorded.

Overall done (all five phases): a user turn is verified by default, pure chat costs one call, durable work runs as a resumable process, plans compile to processes, and every stage accounts for its tokens and failures. The system can be returned to the current behavior with one flag per layer.

---

## 21. Appendix — representative schemas

### A. `ProcessContext` (model-facing section, not JSON)

```
[TASK] Fix the flaky auth test.
[CONSTRAINTS]
- Do not modify unrelated tests.
- Report the root cause.
[TOOLS] data.search, data.update, agent.run
[BUDGET] standard; 31 rounds, 95 steps, 118k tokens remain
[RETRY] none
```

### B. Interpret fast-path tool call

```json
{"tool":"metamind.approve","arguments":{"success":true,"reply":"Hey — good to see you. What are you working on?"}}
```

### C. Check registration

```json
{"tool":"metamind.verify","arguments":{"checks":[{
  "label":"tests_pass",
  "description":"The reported fix makes the auth test pass",
  "details":"Run the auth test; the reply must cite a passing run.",
  "backend":"llm_judge",
  "confidence_threshold":0.6,
  "backend_params":{
    "evaluation_prompt":"User: {{user_request}}\nReply: {{process_reply}}\nDoes the reply show the auth test passing? {{check_details}}",
    "expected_output_schema":{"type":"object","properties":{"pass":{"type":"boolean"},"reasoning":{"type":"string"},"confidence":{"type":"number","minimum":0,"maximum":1}},"required":["pass","reasoning","confidence"]}
  }
}]}}
```

### D. Process template (Turtle, abbreviated)

```turtle
mmd:process/01ABCD a mm:ProcessTemplate ;
  mm:label "Code Review" ;
  mm:processType "llm_operations" ;
  mm:version 1 ;
  mm:parallelEnabled true ;
  mm:hasStep mmd:process/01ABCD/step1, mmd:process/01ABCD/step2 .
mmd:process/01ABCD/step1 a mm:ProcessStep ;
  mm:stepOrder 1 ; mm:stepType "llm_tool_loop" ;
  mm:instructionsTemplate "Run the linter on {{target_path}} and report findings." ;
  mm:toolsAllowed "data.query", "agent.run" ;
  mm:outputBindings "lint_results" .
```

### E. Instance snapshot (abbreviated)

```turtle
mmd:process/01ABCD/instance/01INST a mm:ProcessInstance ;
  mm:template mmd:process/01ABCD ;
  mm:templateVersion 1 ;
  mm:state "waiting" ;
  mm:currentStep mmd:process/01ABCD/step2 ;
  mm:revision 7 ;
  mm:agentstreamSession "process/01ABCD/01INST" ;
  mm:leaseOwner "supervisor-3" ; mm:leaseDeadline "2026-11-23T14:05:00Z"^^xsd:dateTime .
```

---

*This plan is the single execution path for both design documents. If a source document and this plan disagree, the decision table in §18.3 and the non-goals in §15 govern, and the disagreement must be fixed in the source document rather than silently implemented twice.*
