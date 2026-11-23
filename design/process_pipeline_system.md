# Process Pipeline System

> **Status:** proposed design · **Date:** 2026-11-23 · **Scope:** `mm-cme`, `mm-serve`, `mm-data`, `mm-prompt`, `mm-metacog`, `mm-agent`
>
> This document defines a reusable Process pipeline abstraction built on the internal state machine engine. Processes are templated, instantiated, queued, and executed — powering both conversational workflows and automated operations. Integrates with the three-pass turn system (Interpret → Process → Verify) and the planning subsystem.

---

## 1. Core Concepts

### 1.1 Process Definition

A **Process** is a reusable, executable workflow consisting of an ordered sequence of **Steps**, each backed by the `mm-sm` state machine engine. Processes can be:

| Type | Description | Execution Model |
|------|-------------|-----------------|
| **Conversational** | Step-by-step dialog with user; each step presents templated instructions to LLM, loops tool calls until done | LLM-driven, human-in-the-loop |
| **LLM Operations** | Automated LLM-driven steps (no user dialog); each step runs LLM + tools to completion | LLM-driven, autonomous |
| **Backend Service** | Steps delegate to external backend services (to be implemented); LLM orchestrates | Service-driven, LLM coordinates |

### 1.2 Process Template vs. Instance

| Concept | Description | Storage |
|---------|-------------|---------|
| **ProcessTemplate** | Reusable definition: steps, config, parallel/batch settings, wait conditions, PROCESS.md | RDF `process` graph + `PROCESS.md` file |
| **ProcessInstance** | Live execution: unique URI, queue inbox, current step, state, data payload, history | RDF `process` graph + agentstream session |
| **ProcessQueue** | Inbox for incoming items (data, user messages, triggers); supports parallel/batch consumption | In-memory + persisted to RDF |

### 1.3 Process Lifecycle

```
┌─────────────┐     instantiate      ┌──────────────────┐     enqueue      ┌─────────────┐
│  Template   │ ──────────────────► │   Instance       │ ◄─────────────── │  External   │
│  (persistent)│                     │  (has URI,       │                  │  data /     │
│             │                     │   queue, state)  │                  │  user msg   │
└─────────────┘                     └────────┬─────────┘                  └─────────────┘
                                              │
                                              ▼ execute (mm-sm)
                                     ┌──────────────────┐
                                     │  Step 1 → Step 2 │
                                     │  → ... → Step N  │
                                     │  (parallel/batch)│
                                     └────────┬─────────┘
                                              │
                                              ▼
                                     ┌──────────────────┐
                                     │   Complete /     │
                                     │   Failed / Paused│
                                     └──────────────────┘
```

---

## 2. Process Template Structure

### 2.1 RDF/OWL Representation

Each ProcessTemplate is an RDF resource with a unique URI (`mmd:process/<ULID>`):

```turtle
mmd:process/01ABCD a mm:ProcessTemplate ;
    mm:label "Code Review Workflow" ;
    mm:description "Automated code review with lint, test, and security checks" ;
    mm:processType "llm_operations" ;  # conversational | llm_operations | backend_service
    mm:hasStep mmd:process/01ABCD/step1, mmd:process/01ABCD/step2, ... ;
    mm:parallelEnabled false ;
    mm:batchConfig [ mm:maxBatchSize 10 ; mm:batchTimeoutMs 5000 ] ;
    mm:processDoc "PROCESS.md" ;  # path to documentation file
    mm:createdAt "2026-11-23T10:00:00Z"^^xsd:dateTime ;
    mm:createdBy mmd:user/01USER .
```

### 2.2 Step Definition

Each step is an RDF resource (`mmd:process/<ULID>/step<N>`):

```turtle
mmd:process/01ABCD/step1 a mm:ProcessStep ;
    mm:stepOrder 1 ;
    mm:label "Run Linter" ;
    mm:instructionsTemplate "Run the linter on {{target_path}}. Report all errors and warnings." ;
    mm:stepType "llm_tool_loop" ;  # llm_tool_loop | wait_condition | backend_call | user_input
    mm:toolsAllowed [ "data.query", "metamind.plan", "agent.run" ] ;  # subset of 17 tools
    mm:waitCondition {
        mm:conditionType "tool_result" ;  # tool_result | user_input | external_signal | time
        mm:requiredData "lint_results" ;
        mm:timeoutMs 120000
    } ;
    mm:parallelGroup "lint_group" ;  # optional: steps in same group run in parallel
    mm:outputBindings [ "lint_results" ] ;  # data produced for next steps
```

### 2.3 Step Types

| Step Type | Description | LLM Involvement |
|-----------|-------------|-----------------|
| `llm_tool_loop` | Present templated instructions → LLM calls tools until done | Full (Interpret → Process → Verify per turn) |
| `wait_condition` | Pause until condition met (tool result, user input, signal, timeout) | None (state machine waits) |
| `backend_call` | Delegate to registered backend service (future) | LLM orchestrates only |
| `user_input` | Conversational: prompt user, wait for response | Interpret only (fast-path or full) |

### 2.4 PROCESS.md File

Each template has an accompanying `PROCESS.md` (like `SKILL.md`):

```markdown
# Process: Code Review Workflow

## Overview
Automated code review pipeline for PRs. Runs lint, tests, security scan.

## Steps

### Step 1: Run Linter
**Instructions:** Run the linter on {{target_path}}. Report all errors and warnings.
**Tools:** data.query, agent.run
**Wait For:** lint_results (from linter tool)
**Output:** lint_results → passed to Step 2

### Step 2: Run Tests
**Instructions:** Execute test suite for {{target_path}}. Capture failures.
**Tools:** agent.run
**Wait For:** test_results

### Step 3: Security Scan
**Instructions:** Run security scanner on {{target_path}}.
**Tools:** agent.run
**Parallel Group:** scan_group (runs with Step 4)

### Step 4: Dependency Audit
**Instructions:** Check dependencies for vulnerabilities.
**Tools:** agent.run
**Parallel Group:** scan_group

### Step 5: Summarize
**Instructions:** Create summary report from {{lint_results}}, {{test_results}}, {{security_results}}, {{audit_results}}.
**Tools:** data.create, metamind.memory
**Output:** review_report

## Configuration
- parallelEnabled: true
- batchConfig: maxBatchSize=5, batchTimeoutMs=30000
```

---

## 3. Process Instance Execution

### 3.1 Instance Creation

```rust
pub struct ProcessInstance {
    pub uri: String,                    // mmd:process/01ABCD/instance/01INST
    pub template_uri: String,           // mmd:process/01ABCD
    pub state: ProcessState,
    pub current_step: Option<String>,   // step URI
    pub queue: ProcessQueue,
    pub data: HashMap<String, Value>,   // runtime data (bindings, results)
    pub history: Vec<StepExecution>,    // completed steps
    pub agentstream_session: String,    // e.g., "process/01ABCD/01INST"
    pub created_at: i64,
    pub config: InstanceConfig,
}

pub struct InstanceConfig {
    pub parallel_enabled: bool,
    pub batch_config: Option<BatchConfig>,
    pub max_parallel_items: usize,
}
```

### 3.2 Queue & Inbox

```rust
pub struct ProcessQueue {
    pub items: VecDeque<QueueItem>,
    pub processing: HashSet<String>,  // item IDs currently being processed
    pub parallel_enabled: bool,
    pub batch_config: Option<BatchConfig>,
}

pub struct QueueItem {
    pub id: String,
    pub payload: Value,        // arbitrary JSON data
    pub source: QueueSource,   // user_message | external_data | trigger | subprocess
    pub enqueued_at: i64,
    pub priority: u8,
}

pub struct BatchConfig {
    pub max_batch_size: usize,
    pub batch_timeout_ms: u64,
    pub batch_key_fn: Option<String>,  // optional: group by field
}
```

**Consumption modes:**
- **Sequential** (default): one item at a time, FIFO
- **Parallel**: up to `max_parallel_items` items concurrently
- **Batched**: collect up to `max_batch_size` items or wait `batch_timeout_ms`, then process as batch

### 3.3 Execution Engine (mm-sm backed)

The process execution is driven by an `mm-sm` state machine per instance:

```rust
pub enum ProcessState {
    Created,
    Running { current_step: String },
    Waiting { step: String, condition: WaitCondition },
    Paused { reason: String },
    Completed { output: Value },
    Failed { error: String, failed_step: String },
}

pub enum WaitCondition {
    ToolResult { required_keys: Vec<String> },
    UserInput { prompt: String },
    ExternalSignal { signal_name: String },
    Timeout { deadline_ms: i64 },
}
```

**Transitions:**
- `Created` → `Running` (on first `execute()`)
- `Running` → `Waiting` (step has wait condition)
- `Waiting` → `Running` (condition met)
- `Running` → `Running` (step complete, advance to next)
- `Running` → `Completed` (all steps done)
- Any → `Failed` (error)
- Any → `Paused` (manual pause, budget exhausted)

### 3.4 Step Execution: LLM Tool Loop

For `llm_tool_loop` steps, each step execution is a **mini three-pass turn**:

```
Step N Execution:
┌─────────────────────────────────────────────────────────────────┐
│ INTERPRET (step-level)                                          │
│ - Template instructions + current data → LLM                    │
│ - Tools: subset allowed by step + metamind.verify/approve      │
│ - Output: verify_checks[] + process_context for this step      │
├─────────────────────────────────────────────────────────────────┤
│ PROCESS (step-level)                                            │
│ - LLM + tools loop until done                                   │
│ - Tools: step's allowed subset                                  │
│ - Output: step_result (data bindings for next steps)           │
├─────────────────────────────────────────────────────────────────┤
│ VERIFY (step-level)                                             │
│ - Run verify_checks from Interpret                              │
│ - Update process instance data/state                            │
│ - Approve → advance to next step; Reject → retry step (max 5)  │
└─────────────────────────────────────────────────────────────────┘
```

**Key integration:** Reuses the **exact same verification engine** from the main Verify pass:
- `VerificationBackend` trait (LLM Judge, Jev Decision, etc.)
- `metamind.verify()` tool available in step Interpret
- `metamind.approve()` in step Verify
- Structured output only for LLM Judge checks

---

## 4. Process Types Detail

### 4.1 Conversational Process

- **Initiation:** User starts via `metamind.act(process="template_uri")` or chat
- **Step execution:** Each `user_input` step prompts user via chat
- **LLM role:** Interpret presents templated prompt → user responds → Process advances
- **Wait conditions:** Primarily `UserInput` type
- **Example:** Onboarding wizard, troubleshooting guide, approval workflow

### 4.2 LLM Operations Process

- **Initiation:** `metamind.act(process="template_uri", data={...})` or queue push
- **Step execution:** All `llm_tool_loop` steps, no user interaction
- **LLM role:** Full autonomous execution per step
- **Wait conditions:** `ToolResult` (wait for tool output), `Timeout`
- **Parallel/Batch:** Fully supported
- **Example:** Code review, data enrichment, report generation, research pipeline

### 4.3 Backend Service Process

- **Initiation:** Queue push or `metamind.act(process="template_uri", data={...})`
- **Step execution:** `backend_call` steps delegate to registered services
- **LLM role:** Orchestration only (decides which service, passes data)
- **Wait conditions:** `ExternalSignal` (service callback), `Timeout`
- **Future:** Service registry, async callback handling
- **Example:** CI/CD pipeline, data ETL, external API orchestration

---

## 5. Plan → Process Integration

### 5.1 Plan Evaluation Creates Process

When a Plan is executed (via `metamind.plan(run=true)` or `metamind.act(plan="...", run=true)`):

```
Plan (mm:Plan) ──evaluate──► ProcessTemplate (auto-generated) ──instantiate──► ProcessInstance
       │                           │                                       │
       │                           │ 1. Convert plan steps → ProcessStep   │
       │                           │ 2. Add PROCESS.md from plan markdown  │
       │                           │ 3. Set processType per plan type      │
       ▼                           ▼                                       ▼
  PlanStore                  TemplateStore (ephemeral or persistent)    InstanceStore + Queue
```

### 5.2 Plan Types & Process Mapping

| Plan Type | Structure | Process Mapping |
|-----------|-----------|-----------------|
| **flat** | Simple ordered list | Sequential process, one step per plan item |
| **tree** | Hierarchical (max depth 3) | Process with parallel groups per branch; DFS iteration |
| **markdown** | Free-form markdown file | Process steps parsed from markdown sections; flexible |

### 5.3 Plan Builder (2-4 Pass LLM Pipeline)

```
PLAN BUILDER PIPELINE:
┌─────────────────────────────────────────────────────────────────────────────┐
│ PASS 1: STRUCTURED CLASSIFICATION                                           │
│ - Input: user request + context                                             │
│ - Output: { plan_type: "flat"|"tree"|"markdown", complexity: "low|med|high",│
│             estimated_steps: N, reasoning: string }                         │
│ - Uses structured output (only Pass 1 uses structured output)              │
├─────────────────────────────────────────────────────────────────────────────┤
│ PASS 2: MARKDOWN GENERATION                                                 │
│ - Input: request + Pass 1 result (hints structure)                         │
│ - Output: Full plan as markdown text                                        │
│ - For "flat": hint → simple sequence                                        │
│ - For "tree": hint → multi-level (max depth 3)                             │
│ - For "markdown": no structural hints                                       │
├─────────────────────────────────────────────────────────────────────────────┤
│ PASS 3: DECODE TO TARGET FORMAT (if flat/tree)                             │
│ - Input: markdown from Pass 2                                               │
│ - Output: Structured flat list or tree (JSON)                              │
│ - Uses structured output                                                    │
├─────────────────────────────────────────────────────────────────────────────┤
│ PASS 4: VERIFY-DECODE (LLM Judge)                                          │
│ - Input: original markdown + decoded structure                             │
│ - Output: { faithful: bool, reasoning: string, confidence: 0.0000-1.0000 } │
│ - If !faithful AND retries < 5: repeat Pass 3 with error context           │
│ - If !faithful AND retries >= 5: use latest decoded output                 │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Pass 1 Classification Schema:**
```json
{
  "plan_type": "flat" | "tree" | "markdown",
  "complexity": "low" | "medium" | "high",
  "estimated_steps": 5,
  "reasoning": "Simple todo list with 5 independent tasks"
}
```

**Pass 3 Decode Schema (flat):**
```json
{
  "steps": [
    { "order": 1, "title": "Task 1", "description": "...", "dependencies": [] },
    { "order": 2, "title": "Task 2", "description": "...", "dependencies": [1] }
  ]
}
```

**Pass 3 Decode Schema (tree):**
```json
{
  "steps": [
    { "order": 1, "title": "Phase 1", "children: [
        { "order": 1.1, "title": "Subtask A", "children: [] },
        { "order": 1.2, "title": "Subtask B", "children: [] }
      ]}
  ]
}
```

### 5.4 Plan Storage

| Plan Type | Storage |
|-----------|---------|
| **flat / tree** | RDF `plan` graph: `mm:Plan` with `mm:hasStep` → `mm:PlanStep` (ordered) |
| **markdown** | File: `data/plans/<plan_id>.md` + RDF reference `mm:planDoc <file://...>` |

**Plan Metadata (all types):**
```turtle
mmd:plan/01PLAN a mm:Plan ;
    mm:planId "01PLAN" ;           # randomized short ID
    mm:label "Fix auth flake" ;
    mm:description "Investigate and fix flaky authentication test" ;
    mm:planType "tree" ;
    mm:status "active" ;           # active | completed | expired | archived
    mm:expiresAt "2026-11-24T10:00:00Z"^^xsd:dateTime ;  # 24h default
    mm:neverExpires false ;
    mm:planDoc <file://data/plans/01PLAN.md> ;  # if markdown
    mm:createdAt "2026-11-23T10:00:00Z"^^xsd:dateTime .
```

---

## 6. Expiration & Cleanup

### 6.1 Plan Expiration

- **Default:** 24 hours from creation (`mm:expiresAt`)
- **Never Expires:** `mm:neverExpires true` (set via `metamind.plan(never_expires=true)`)
- **Expired plans:** Status → `expired`; hidden from `metamind.plan(list)` unless `include_expired=true`
- **Cleanup job:** Daily sweep archives expired plans (moves to `archived` status, retains data)

### 6.2 Process Instance Expiration

- **Conversational:** Expires after configurable inactivity (default 1 hour)
- **LLM Operations:** No expiration (runs to completion)
- **Backend:** Configurable per template

---

## 7. Tool Integration

### 7.1 metamind.act() Extensions

```rust
pub struct ActInput {
    /// Execute a specific process template
    pub process: Option<String>,        // template URI: "mmd:process/01ABCD"
    /// Execute a plan (creates process from plan)
    pub plan: Option<String>,           // plan ID: "01PLAN"
    /// Run plan immediately (with plan=)
    pub run: Option<bool>,              // default false
    /// Input data for process/plan
    pub data: Option<Value>,
    /// Queue item priority
    pub priority: Option<u8>,
    /// Never expire the created plan
    pub never_expires: Option<bool>,
}
```

**Behavior:**
- `process=` → instantiate template, enqueue data, execute
- `plan=` + `run=true` → build plan → create process → execute immediately
- `plan=` + `run=false` → create plan only (returns plan ID for later `metamind.act(plan="01PLAN", run=true)`)

### 7.2 metamind.plan() Extensions

```rust
pub struct PlanInput {
    /// Natural language request to build plan for
    pub request: String,
    /// Plan structure type (default: "flat")
    pub structure: Option<String>,  // "flat" | "tree" | "markdown"
    /// Run immediately after creation
    pub run: Option<bool>,          // default false
    /// Never expire
    pub never_expires: Option<bool>,
    /// List non-expired plans
    pub list: Option<bool>,         // if true, ignore other fields
    /// Get last plan by ID
    pub last: Option<bool>,
}
```

**Outputs:**
- `list=true` → `{ plans: [PlanSummary], last_plan: Option<PlanSummary> }`
- `last=true` → `{ plan: PlanSummary }` (most recent non-expired)
- Normal → `{ plan_id: "01PLAN", plan_uri: "mmd:plan/01PLAN", process_uri: Option<String> }`

### 7.3 metamind.verify() in Process Steps

Available in step-level Interpret (same as main Verify pass):
- Registers verification checks for step's Verify phase
- Uses same `VerificationBackend` trait
- Checks run before step advances

---

## 8. RDF/OWL Graph Integration

### 8.1 Process Graph

All process data in named graph `mm:process`:

```
mm:process/
  ├── mmd:process/01ABCD                    # Template
  │     ├── mm:hasStep mmd:process/01ABCD/step1
  │     ├── mm:hasStep mmd:process/01ABCD/step2
  │     └── mm:processDoc "PROCESS.md"
  │
  ├── mmd:process/01ABCD/instance/01INST    # Instance
  │     ├── mm:template mmd:process/01ABCD
  │     ├── mm:state "running"
  │     ├── mm:currentStep mmd:process/01ABCD/step2
  │     ├── mm:hasData [ mm:key "lint_results"; mm:value "..." ]
  │     └── mm:agentstreamSession "process/01ABCD/01INST"
  │
  └── mmd:queue/01ABCD/01INST               # Queue (linked to instance)
        ├── mm:hasItem mmd:queueitem/01ITEM1
        └── mm:parallelEnabled true
```

### 8.2 Adding/Removing from Graph Store

- Templates: `data.create(type="process_template", ...)` → adds to `mm:process` graph
- Instances: Auto-created on instantiation; `data.delete(uri)` removes instance + queue
- Queries: `data.query` / `data.search` over `mm:process` graph
- SPARQL: Full query power for complex process analytics

---

## 9. Configuration

```toml
# config/process.toml
[process]
default_parallel_enabled = false
default_max_parallel_items = 4
default_batch_max_size = 10
default_batch_timeout_ms = 5000
default_step_timeout_ms = 300000
conversational_inactivity_timeout_ms = 3600000

[plan]
default_structure = "flat"
default_expires_hours = 24
markdown_plans_dir = "data/plans/"
max_decode_retries = 5

[plan.builder]
pass1_model = "gpt-4o-mini"
pass2_model = "gpt-4o"
pass3_model = "gpt-4o-mini"
pass4_judge_model = "gpt-4o-mini"
pass4_temperature = 0.0

[verification]
# Reuses main verification config
```

---

## 10. Migration & Implementation Phases

### Phase 1: Core Process Engine
1. `mm-metacog` crate: `ProcessTemplate`, `ProcessInstance`, `ProcessStep`, `ProcessQueue`
2. `mm-sm` integration: Process state machine per instance
3. RDF storage in `mm-data` (`mm:process` graph)
4. Basic `llm_tool_loop` step execution (reuses three-pass)

### Phase 2: Queue & Parallel/Batch
1. `ProcessQueue` with sequential/parallel/batch modes
2. Inbox API: `enqueue()`, `dequeue()`, `requeue()`
3. Persistence of queue to RDF

### Phase 3: Plan Builder (2-4 Pass)
1. `PlanBuilder` in `mm-metacog` or new `mm-planbuilder`
2. Pass 1: Classification (structured output)
3. Pass 2: Markdown generation
4. Pass 3: Decode to flat/tree (structured output)
5. Pass 4: Verify-decode (LLM Judge, retry up to 5)

### Phase 4: Plan ↔ Process Integration
1. `metamind.plan()` tool with builder pipeline
2. `metamind.act(process=, plan=, run=)` tool
3. Plan expiration sweep (daily cron)
4. Plan storage (RDF + markdown files)

### Phase 5: Process Types & PROCESS.md
1. Conversational process support (user_input steps)
2. Backend service step type (stub)
3. PROCESS.md parsing and LLM context injection
4. Template CRUD via `data.*` tools

### Phase 6: Advanced Features
1. Sub-processes (process calling process)
2. Process versioning and rollback
3. Process analytics dashboard (inspector)
4. Process marketplace / sharing

---

## 11. Testing Strategy

| Component | Tests |
|-----------|-------|
| Process state machine | All transitions, pause/resume, failure recovery |
| Queue modes | Sequential FIFO, parallel (max 4), batch (size 10, timeout) |
| Step execution | LLM tool loop completes, wait conditions met, timeout handling |
| Plan builder | Pass 1 classification accuracy, Pass 2 markdown quality, Pass 3 decode fidelity, Pass 4 judge catches errors |
| Plan expiration | 24h default, never_expires flag, list excludes expired |
| Process CRUD | Template create/read/update/delete via data tools |
| RDF integration | Graph queries return correct templates/instances |
| Tool integration | `metamind.act` starts process, `metamind.plan` builds/runs |

---

## 12. Affected Files

| File | Change |
|------|--------|
| `crates/mm-metacog/src/process.rs` (new) | Core process types, state machine, queue |
| `crates/mm-metacog/src/plan_builder.rs` (new) | 2-4 pass plan builder pipeline |
| `crates/mm-metacog/src/lib.rs` | Re-exports |
| `crates/mm-data/src/process.rs` (new) | RDF storage for process graph |
| `crates/mm-serve/src/cm_tools.rs` | `metamind.act(process=, plan=, run=)`, `metamind.plan(structure=, run=, list=, last=)` |
| `crates/mm-prompt/src/lib.rs` | Strategies: `process.step.llm_tool_loop.v1`, `process.step.user_input.v1`, `plan.builder.classify.v1`, `plan.builder.generate.v1`, `plan.builder.decode.v1`, `plan.builder.verify_decode.v1` |
| `config/prompts/applications/` | New strategy files |
| `config/process.toml` (new) | Process configuration |
| `frontend/src/components/inspector/` | Process timeline, queue view, plan list |

---

## 13. References

- `three_pass_turn_sequence.md` — Three-pass turn system (Interpret → Process → Verify)
- `conversation_manager_agent_inspector_plan.md` — Overall CM v3 architecture
- `mm-sm` — State machine engine (process execution backbone)
- `mm-prompt` — Deterministic prompt rendering (step instructions, PROCESS.md)
- `mm-agent` — `agent.run` for external work in process steps