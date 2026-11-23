# Three-Pass Turn Sequence: Interpret → Process → Verify

> **Status:** proposed design · **Date:** 2026-11-23 · **Scope:** `mm-cme`, `mm-serve`, `mm-data`, `mm-prompt`, `mm-agent`
>
> This document defines a three-pass LLM-driven turn architecture where each pass is a separate model call with its own tailored prompt, tool set, and agentstream context. The passes are: **Interpret** (analyze request, plan verification), **Process** (execute with full internal tools + `agent.run`), **Verify** (run checks, update state, approve/retry).

---

## 1. Architecture Overview

### 1.1 Pass Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        USER TURN (one request → one reply)                   │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ PASS 1: INTERPRET                                                            │
│ ┌─────────────────────────────────────────────────────────────────────────┐ │
│ │ LLM Call 1: Analyze user request + conversation state                   │ │
│ │ Tools: metamind.verify() [add checks], metamind.approve(success=true)   │ │
│ │ Output: VerifyCheck[] + ProcessContext + (optional fast-path reply)     │ │
│ └─────────────────────────────────────────────────────────────────────────┘ │
│        │                    │                                                │
│        │ fast-path         │ normal                                         │
│        ▼                    ▼                                                │
│   Return reply         PASS 2: PROCESS                                     │
│   (turn done)       ┌─────────────────────────────────────────────────┐   │
│                     │ LLM Call 2: Execute with full internal tools    │   │
│                     │ Tools: 16 internal + agent.run() [external]     │   │
│                     │ Output: User-facing reply text + tool emissions │   │
│                     └─────────────────────────────────────────────────┘   │
│                                      │                                    │
│                                      ▼                                    │
│                          PASS 3: VERIFY                                   │
│                     ┌─────────────────────────────────────────────────┐   │
│                     │ For each VerifyCheck: run verification backend  │   │
│                     │ LLM Call 3a: Update state/memory/db from result │   │
│                     │ LLM Call 3b: Review → metamind.approve()        │   │
│                     │ On failure: retry Process (max 5) with error    │   │
│                     └─────────────────────────────────────────────────┘   │
│                                      │                                    │
│                                      ▼                                    │
│                          Return final reply to user                        │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 1.2 Key Principles

| Principle | Implementation |
|-----------|----------------|
| **One agentstream session** | All three passes append to the same session; contexts are carefully constructed per pass |
| **Clean conversation log** | A separate "clean" log tracks only user messages + final replies for coherent context |
| **Pass-specific tool sets** | Interpret: `metamind.verify`, `metamind.approve(true)` | Process: 16 internal + `agent.run` | Verify: internal tools + `metamind.approve(false, error)` |
| **Structured verification** | Abstract `VerificationBackend` trait; LLM-as-judge is first implementation |
| **Retry with error feedback** | Verify failure → prepend error to Process context → retry (max 5) |
| **Streaming aggregation** | UI streams all passes concatenated; appears as one message |
| **Sub-agent thinking** | `metamind.think()` spawns a sub-agent with its own full 3-pass cycle |

---

## 2. Agentstream Context Construction

### 2.1 Clean Conversation Log

Maintained separately from the full agentstream. Contains only:
- User messages (original requests)
- Final assistant replies (from Process pass, or Interpret fast-path)

This log is included in **every pass's context** so each LLM sees a coherent conversation history.

### 2.2 Per-Pass Context Assembly

Each pass receives a `PromptInstance` rendered from a pass-specific strategy with a tailored context:

| Pass | Strategy | Key Context Components |
|------|----------|------------------------|
| **Interpret** | `cm.interpret.v1` | Clean log, current `[STATE]`, user request, available verification backends, `metamind.verify` tool schema, `metamind.approve(true)` tool schema |
| **Process** | `cm.process.v1` | Clean log, current `[STATE]`, user request, **Interpret's output** (VerifyCheck[], ProcessContext), full 17-tool schemas (16 internal + `agent.run`) |
| **Verify** | `cm.verify.v1` | Clean log, current `[STATE]`, user request, **Process output** (reply text, tool emissions), **Interpret's VerifyCheck[]**, verification results, internal tool schemas, `metamind.approve(false, error)` tool schema |

### 2.3 Context Isolation

- **Interpret** sees: user request + conversation state. Does NOT see Process/Verify internals.
- **Process** sees: user request + Interpret's plan. Does NOT see verification logic.
- **Verify** sees: user request + Process output + Interpret's checks + verification results. Does NOT re-plan.

---

## 3. Pass 1: INTERPRET

### 3.1 Purpose
Analyze the user's request to determine:
1. Is this pure conversation (fast-path) or an action request?
2. What verification checks are needed?
3. What context does the Process pass need?

### 3.2 Tools Available (Only These Two)

| Tool | Schema | Purpose |
|------|--------|---------|
| `metamind.verify` | `{ checks: VerifyCheckInput[] }` | Register verification checks for Verify pass |
| `metamind.approve` | `{ success: true, reply: string }` | Fast-path: pure conversation, return reply immediately |

### 3.3 VerifyCheckInput Schema

```rust
#[derive(Serialize, Deserialize)]
pub struct VerifyCheckInput {
    /// Unique label for this check
    pub label: String,
    /// Optional human-readable description of what is being verified
    pub description: Option<String>,
    /// Optional additional details for the verification backend
    pub details: Option<String>,
    /// Verification backend to use (currently only "llm_judge")
    pub backend: String, // "llm_judge" | "jev_decision" | ...
    /// Backend-specific parameters
    #[serde(flatten)]
    pub backend_params: serde_json::Value,
    /// Confidence threshold to accept (0.0000-1.0000, default 0.5)
    pub confidence_threshold: Option<f64>,
}
```

### 3.4 LLM Judge Backend Params

```json
{
  "backend": "llm_judge",
  "backend_params": {
    "evaluation_prompt": "string - the prompt template for the judge LLM",
    "expected_output_schema": "json_schema - structured output schema for judge result"
  }
}
```

### 3.5 Default Verification Check

If Interpret determines an action is requested and adds no explicit checks, a **default LLM judge check** is implicitly added:

```json
{
  "label": "default_action_verification",
  "description": "Verify the Process pass reply correctly fulfills the user's request",
  "backend": "llm_judge",
  "backend_params": {
    "evaluation_prompt": "The user asked: {{user_request}}\n\nThe assistant replied: {{process_reply}}\n\nEvaluate whether the reply correctly and completely fulfills the request. Return pass/fail, reasoning, and confidence (0.0000-1.0000).",
    "expected_output_schema": {
      "type": "object",
      "properties": {
        "pass": { "type": "boolean" },
        "reasoning": { "type": "string" },
        "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
      },
      "required": ["pass", "reasoning", "confidence"]
    }
  },
  "confidence_threshold": 0.5
}
```

### 3.6 Interpret Output

```rust
pub enum InterpretResult {
    FastPath {
        reply: String,           // User-facing reply from Interpret
        prompt_instance_id: String,
    },
    Normal {
        verify_checks: Vec<VerifyCheck>,
        process_context: ProcessContext,  // Instructions/context for Process pass
        prompt_instance_id: String,
    },
}

pub struct ProcessContext {
    /// High-level task description for Process pass
    pub task: String,
    /// Any specific constraints or requirements
    pub constraints: Vec<String>,
    /// Suggested tools to use (subset of 17)
    pub suggested_tools: Option<Vec<String>>,
    /// Budget guidance
    pub budget_hint: BudgetClass,
}
```

### 3.7 Prompt Strategy: `cm.interpret.v1`

Key fragments:
- `identity/character` — Metamind voice
- `state/ownership` — Read `[STATE]`, understand conversation
- `tools/interpret_contract` — Only `metamind.verify` and `metamind.approve(true)` available
- `verification/guide` — How to design good verification checks
- `fast_path/criteria` — When to use fast-path (pure conversation, no action needed)

---

## 4. Pass 2: PROCESS

### 4.1 Purpose
Execute the task using all internal tools plus `agent.run` for external work. Generate the **user-facing reply**.

### 4.2 Tools Available (17 Total)

| Family | Tools | Available in Process? |
|--------|-------|----------------------|
| Conversation | `conversation.topic`, `conversation.depth`, `conversation.status`, `conversation.frame`, `agent.run` | **YES** (only pass with `agent.run`) |
| Data | `data.create`, `data.update`, `data.query`, `data.delete`, `data.search`, `data.schema`, `data.namespace` | YES |
| Metamind | `metamind.plan`, `metamind.think`, `metamind.memory`, `metamind.log`, `metamind.act` | YES |
| **Verify-only** | `metamind.verify`, `metamind.approve` | **NO** |

### 4.3 Process Loop

The Process pass runs a **model/tool loop** (like current `TurnExecutor`):

1. Render prompt with `cm.process.v1` strategy + Interpret's `ProcessContext`
2. Call model with 17 tool schemas
3. While model requests tools:
   - Execute tool (strict JSON validation)
   - Append result to transcript
   - Re-render prompt with updated state
   - Check budget caps
4. When model returns final text → **this is the user-facing reply**

### 4.4 Special Tools

| Tool | Behavior |
|------|----------|
| `agent.run` | Starts/continues Hermes/Pi sub-agent in user workspace; returns compact summary; max 1 downstream turn per call |
| `metamind.think` | Spawns a **sub-agent with its own full 3-pass cycle**; useful for deep research, multi-step analysis; returns structured findings |
| `metamind.verify` | **NOT available** — only in Interpret |
| `metamind.approve` | **NOT available** — only in Interpret (true) and Verify (false) |

### 4.5 Process Output

```rust
pub struct ProcessResult {
    pub reply_text: String,           // Final user-facing text
    pub tool_emissions: Vec<ToolCallEmission>,
    pub prompt_instance_ids: Vec<String>,  // One per model round
    pub budget_ledger: BudgetLedger,
}
```

### 4.6 Prompt Strategy: `cm.process.v1`

Key fragments:
- `identity/character` — Metamind voice
- `state/ownership` — Maintain `[STATE]` automatically
- `tools/full_contract` — All 17 tools with exact schemas
- `tools/selection` — Choose by job family
- `process/execution` — Continue until task complete; use `agent.run` for external work; use `metamind.think` for deep analysis
- `budget/contract` — Spend budget wisely

---

## 5. Pass 3: VERIFY

### 5.1 Purpose
1. Run all verification checks registered by Interpret
2. Update conversation state, memory, database from Process output
3. Review and approve/reject the turn
4. On rejection: retry Process with error context (max 5 attempts)

### 5.2 Verification Backend Interface

```rust
#[async_trait]
pub trait VerificationBackend: Send + Sync {
    /// Unique backend identifier (e.g., "llm_judge", "jev_decision")
    fn id(&self) -> &'static str;

    /// Run a single verification check
    async fn verify(
        &self,
        check: &VerifyCheck,
        process_output: &ProcessResult,
        context: &VerifyContext,
    ) -> Result<VerificationResult>;

    /// Validate check configuration at registration time
    fn validate_config(&self, params: &serde_json::Value) -> Result<()>;
}

pub struct VerifyCheck {
    pub label: String,
    pub description: Option<String>,
    pub details: Option<String>,
    pub backend: String,
    pub backend_params: serde_json::Value,
    pub confidence_threshold: f64,  // Default 0.5
}

pub struct VerificationResult {
    pub check_label: String,
    pub pass: bool,
    pub reasoning: String,
    pub confidence: f64,  // 0.0000-1.0000, clamped to 1.0 if > 1
    pub backend: String,
}

pub struct VerifyContext {
    pub clean_log: Vec<ConversationTurn>,  // User + final replies only
    pub current_state: StateView,
    pub user_request: String,
    pub interpret_checks: Vec<VerifyCheck>,
}
```

### 5.3 LLM Judge Backend Implementation

First (and initially only) backend: `LlmJudgeBackend`

**Important:** The LLM Judge is the **only** component that uses structured output. All three main passes (Interpret, Process, Verify) use normal LLM calls that return text/streaming/tool calls. The structured output is confined to the verification check execution inside the Verify pass.

```rust
pub struct LlmJudgeBackend {
    client: Arc<dyn LlmClient>,
    prompt_store: Arc<PromptStore>,
}

#[async_trait]
impl VerificationBackend for LlmJudgeBackend {
    fn id(&self) -> &'static str { "llm_judge" }

    async fn verify(&self, check, process_output, context) -> Result<VerificationResult> {
        // 1. Build judge prompt from check.backend_params.evaluation_prompt
        //    Template variables: {{user_request}}, {{process_reply}}, {{check_details}}, {{state_summary}}
        // 2. Call model WITH STRUCTURED OUTPUT schema from check.backend_params.expected_output_schema
        // 3. Parse result → VerificationResult
        // 4. Clamp confidence: if > 1.0 → 1.0
        // 5. Compare confidence against check.confidence_threshold
    }

    fn validate_config(&self, params) -> Result<()> {
        // Require evaluation_prompt and expected_output_schema
    }
}
```

**Structured Output Schema (required for LLM Judge only):**
```json
{
  "type": "object",
  "properties": {
    "pass": { "type": "boolean" },
    "reasoning": { "type": "string" },
    "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
  },
  "required": ["pass", "reasoning", "confidence"]
}
```

### 5.4 Verify Pass Steps

The Verify pass has **two distinct phases**:

```
VERIFY PASS:
┌─────────────────────────────────────────────────────────────────────────────┐
│ PHASE A: Run Verification Checks (0-N items, parallel where possible)      │
│                                                                             │
│ For each check in InterpretResult.verify_checks:                           │
│   - Look up backend by check.backend (e.g., "llm_judge")                   │
│   - Call backend.verify(check, process_result, verify_context)             │
│     → This is the ONLY place structured output is used                     │
│   - Collect VerificationResult into a results collection                   │
│                                                                             │
│ If Interpret added 0 checks: Phase A is skipped, results = []              │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ PHASE B: Verify LLM Call (normal text/streaming/tool calls)                │
│                                                                             │
│ - Prompt: cm.verify.v1                                                     │
│ - Context: clean_log + [STATE] + user_request + process_result             │
│              + verification_results (collection from Phase A)              │
│ - Tools: All 16 internal tools (NO agent.run, NO metamind.verify)          │
│            PLUS metamind.approve(success=false, error=...)                 │
│ - LLM reviews Process output + verification results                        │
│ - LLM calls internal tools to update state/memory/db:                      │
│     * Conversation state (topic, depth, status)                            │
│     * Memory (metamind.memory remember/forget)                             │
│     * Database (data.* if new facts learned)                               │
│     * Plans/actions (metamind.plan/act if relevant)                        │
│ - LLM then calls ONE of:                                                   │
│     * metamind.approve(success=true) → turn complete                       │
│     * metamind.approve(success=false, error="...") → retry Process         │
│                                                                             │
│ This LLM call uses NORMAL streaming/tool calls — NO structured output      │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 5.5 Verify Tools

| Tool | Schema | Purpose |
|------|--------|---------|
| `metamind.approve` | `{ success: boolean, error?: string }` | Only tool that can end the turn. `success=true` → done. `success=false` → retry Process with error. |

**Note:** `metamind.approve(success=true)` is **also** available in Interpret for fast-path. In Verify, only `success=false` triggers retry.

### 5.6 Prompt Strategy: `cm.verify.v1`

Key fragments:
- `identity/character` — Metamind voice
- `state/ownership` — Update `[STATE]` from Process output
- `verification/review` — How to interpret verification results
- `tools/verify_contract` — 16 internal tools + `metamind.approve(false, error)`
- `verification/state_update` — Update memory, db, plans from Process output
- `retry/guidance` — When to retry vs. give up; how to write helpful error messages

---

## 6. Fast-Path (Pure Conversation)

### 6.1 Trigger
Interpret determines the user's message is **purely conversational** — no action, no fact-seeking, no task. Examples:
- Greetings, pleasantries
- "How are you?"
- "Thanks"
- Casual chat

### 6.2 Fast-Path Flow

```
Interpret LLM call:
  - Sees user message + clean log + [STATE]
  - Determines: "This is pure conversation"
  - Calls: metamind.approve(success=true, reply="Hey! I'm doing great...")
  - Returns InterpretResult::FastPath { reply, prompt_instance_id }

Turn complete. No Process pass. No Verify pass.
```

### 6.3 Fast-Path Criteria (in `cm.interpret.v1` fragment)

> Use `metamind.approve(success=true)` **only** when:
> - The user is greeting, thanking, saying goodbye, or chatting casually
> - No tool use is needed (no data lookup, no memory recall, no planning, no external work)
> - No verification is needed (nothing to verify)
> - The reply can be generated directly from conversation context
>
> Do NOT fast-path if:
> - User asks a question requiring data search / memory recall
> - User requests an action (create, update, plan, delegate)
> - User asks for analysis or reasoning
> - Any fact could be wrong and needs verification

---

## 7. Sub-Agent Thinking: `metamind.think()`

### 7.1 Purpose
Enable deep, multi-step reasoning that doesn't clutter the main conversation. The sub-agent runs its **own full 3-pass cycle**.

### 7.2 Interface

```rust
// Tool available in Process pass only
pub struct ThinkInput {
    pub question: String,
    pub focus_entities: Option<Vec<String>>,  // Optional: entities to focus on
    pub budget_class: Option<BudgetClass>,    // Default: "think" (4 rounds, 8 steps, 8k tokens)
}

pub struct ThinkOutput {
    pub findings: String,        // Structured findings for main agent
    pub evidence: Vec<String>,   // Citations/evidence
    pub confidence: f64,         // 0.0000-1.0000
    pub agentstream_session_id: String,  // Separate session for traceability
}
```

### 7.3 Execution

1. Process LLM calls `metamind.think(question, ...)`
2. System spawns a **sub-agent** (via `mm-agent`) with:
   - Its own agentstream session (`think/<parent_turn_id>/<ulid>`)
   - Read-only access to data tools (`data.query`, `data.search`, `data.schema`)
   - Access to `agent.run` for web search via Hermes
   - **Full 3-pass cycle**: Interpret → Process → Verify
3. Sub-agent returns `ThinkOutput`
4. Main Process LLM receives findings as tool result
5. Main Process continues

### 7.4 Use Cases
- "Research the latest Rust async patterns" → sub-agent searches web via Hermes, synthesizes
- "Analyze why the auth test is flaky" → sub-agent reads logs, code, memory, reasons deeply
- "Compare these three design options" → sub-agent gathers data, structures comparison

---

## 8. Budget Management

Each pass has its own budget class. Sub-agents (`metamind.think`, `agent.run`) have independent budgets.

| Pass | Default Budget Class | Rounds | Steps | Tokens | Wall Time |
|------|---------------------|--------|-------|--------|-----------|
| Interpret | `quick` | 8 | 24 | 32k | 120s |
| Process | `standard` | 32 | 96 | 120k | 600s |
| Verify | `quick` | 8 | 16 | 16k | 90s |
| `metamind.think` sub-agent | `think` | 4 | 8 | 8k | 90s |
| `agent.run` downstream turn | `summarize` | 1 | 1 | 8k | 30s |

**Budget inheritance:** Parent Process budget reserves child budget upfront. Child cannot exceed parent's remaining budget.

---

## 9. Retry Logic

### 9.1 Verify → Process Retry

```
Max retries: 5 (configurable via `MM_VERIFY_MAX_RETRIES`)

On metamind.approve(success=false, error):
  retry_count++
  if retry_count <= max_retries:
    augment ProcessContext:
      "Previous attempt failed verification: {error}
       Verification results: {verification_results}
       Please address the issues and try again."
    Re-run Process pass (new LLM call, new prompt instance)
    Re-run Verify pass
  else:
    Return failure to user
```

### 9.2 Error Context Construction

The error passed to retry includes:
- The `error` string from `metamind.approve(success=false, error=...)`
- All `VerificationResult` objects (pass/fail, reasoning, confidence)
- Which checks failed and their thresholds

This gives the Process LLM concrete feedback to fix.

---

## 10. UI Streaming & Aggregation

### 10.1 Streaming Model

Each pass streams its text output to the UI via SSE:

```
Event: pass_start    { pass: "interpret", turn_id: "..." }
Event: token         { pass: "interpret", token: "I" }
Event: token         { pass: "interpret", token: " understand" }
...
Event: pass_complete { pass: "interpret", result: "fast_path" | "normal" }

Event: pass_start    { pass: "process", turn_id: "..." }
Event: token         { pass: "process", token: "Here's" }
Event: token         { pass: "process", token: " what" }
...
Event: tool_call     { pass: "process", tool: "data.search", ... }
Event: tool_result   { pass: "process", tool: "data.search", ... }
Event: pass_complete { pass: "process", ... }

Event: pass_start    { pass: "verify", turn_id: "..." }
Event: token         { pass: "verify", token: "Updating" }
...
Event: pass_complete { pass: "verify", approved: true, retries: 0 }
```

### 10.2 UI Aggregation

The frontend **concatenates all pass outputs** into a single message bubble:

- Show "Interpret: ..." prefix (collapsible) for debug/transparency
- Show "Process: ..." as main reply
- Show "Verify: ✓ approved" or "Verify: ↻ retry 1/5" indicator
- On retry: append "Retry 1: ..." to same bubble (or show as threaded)

**User sees one coherent reply.** Internally, three+ LLM calls occurred.

---

## 11. Prompt Strategies Summary

| Strategy | Application | Pass | Key Fragments |
|----------|-------------|------|---------------|
| `cm.interpret.v1` | Interpret | 1 | `identity/*`, `state/ownership`, `tools/interpret_contract`, `verification/guide`, `fast_path/criteria` |
| `cm.process.v1` | Process | 2 | `identity/*`, `state/ownership`, `tools/full_contract`, `tools/selection`, `process/execution`, `budget/contract` |
| `cm.verify.v1` | Verify | 3 | `identity/*`, `state/ownership`, `verification/review`, `tools/verify_contract`, `verification/state_update`, `retry/guidance` |
| `cm.think.v1` | Think sub-agent | All | `identity/*`, `state/ownership`, `tools/think_contract` (read-only data + agent.run), `think/deep_analysis` |
| `agent.system.v1` | Agent (Hermes/Pi) | — | Downstream agent system prompt |
| `agent.summarize.v1` | Agent summarizer | — | Summarize agent turn for CM |

---

## 12. Tool Visibility Matrix

**Rule:** All 19 tools are available in all passes **except** these three explicit exceptions:
- `metamind.approve` — only in **Interpret** (success=true) and **Verify** (success=false)
- `metamind.verify` — only in **Interpret**
- `agent.run` — only in **Process**

| Tool | Interpret | Process | Verify | Think Sub-Agent |
|------|-----------|---------|--------|-----------------|
| `conversation.topic` | ✓ | ✓ | ✓ | ✓ |
| `conversation.depth` | ✓ | ✓ | ✓ | ✓ |
| `conversation.status` | ✓ | ✓ | ✓ | ✓ |
| `conversation.frame` | ✓ | ✓ | ✓ | ✓ |
| `agent.run` | ✗ | ✓ | ✗ | ✗ |
| `data.create` | ✓ | ✓ | ✓ | ✓ |
| `data.update` | ✓ | ✓ | ✓ | ✓ |
| `data.query` | ✓ | ✓ | ✓ | ✓ |
| `data.delete` | ✓ | ✓ | ✓ | ✓ |
| `data.search` | ✓ | ✓ | ✓ | ✓ |
| `data.schema` | ✓ | ✓ | ✓ | ✓ |
| `data.namespace` | ✓ | ✓ | ✓ | ✓ |
| `metamind.plan` | ✓ | ✓ | ✓ | ✓ |
| `metamind.think` | ✓ | ✓ | ✓ | ✓ |
| `metamind.memory` | ✓ | ✓ | ✓ | ✓ |
| `metamind.log` | ✓ | ✓ | ✓ | ✓ |
| `metamind.act` | ✓ | ✓ | ✓ | ✓ |
| `metamind.verify` | ✓ | ✗ | ✗ | ✗ |
| `metamind.approve` | ✓ | ✗ | ✓ | ✗ |

---

## 13. Agentstream Logging

All passes log to the **same agentstream session** (the conversation session). Each model call creates a `PromptBuilt` event. Tool calls create `ToolCall`/`ToolResult` events.

**Clean log** (user messages + final replies) is maintained as a derived projection for context injection.

Sub-agents (`metamind.think`, `agent.run`) create **separate agentstream sessions** linked via `parent_turn_id`.

---

## 14. Configuration

```toml
# config/three_pass.toml
[interpret]
budget_class = "quick"
strategy = "cm.interpret.v1"

[process]
budget_class = "standard"
strategy = "cm.process.v1"
max_tool_rounds = 32
max_tool_steps = 96

[verify]
budget_class = "quick"
strategy = "cm.verify.v1"
max_retries = 5

[verification.backends.llm_judge]
enabled = true
model = "gpt-4o-mini"  # or configured default
temperature = 0.0

[think]
budget_class = "think"
strategy = "cm.think.v1"
```

---

## 15. Migration from Current Architecture

### 15.1 Phase 1: Verification Backend Infrastructure
1. Add `VerificationBackend` trait in `mm-cme` or `mm-data`
2. Implement `LlmJudgeBackend` in `mm-cme`
3. Add `metamind.verify` and `metamind.approve` tools to `mm-serve/cm_tools.rs`
4. Unit tests for backend interface

### 15.2 Phase 2: Interpret Pass
1. Create `cm.interpret.v1` strategy and fragments
2. Implement `pass_interpret()` in `TurnExecutor` (new method)
3. Tool handler for `metamind.verify` / `metamind.approve(true)`
4. Fast-path logic

### 15.3 Phase 3: Process Pass (Refactor Existing)
1. Restructure current `execute()` → `pass_process()`
2. Ensure only Process gets `agent.run` tool
3. Add `metamind.think` tool handler (spawns sub-agent)

### 15.4 Phase 4: Verify Pass
1. Create `cm.verify.v1` strategy and fragments
2. Implement `pass_verify()` with verification runner + retry loop
3. Tool handler for `metamind.approve(false, error)`
4. State update logic from Process output

### 15.5 Phase 5: Orchestration & Streaming
1. `execute_three_pass()` orchestrates Interpret → Process → Verify
2. SSE streaming for each pass
3. Clean log maintenance
4. Inspector pass timeline

### 15.6 Phase 6: Sub-Agents
1. `metamind.think` full implementation
2. `agent.run` integration with Hermes/Pi
3. Agentstream session linking

---

## 16. Testing Strategy

### 16.1 Unit Tests

| Component | Tests |
|-----------|-------|
| `VerificationBackend` trait | Mock backend, verify interface compliance |
| `LlmJudgeBackend` | Structured output parsing, confidence clamping, threshold comparison |
| `metamind.verify` tool | Registers checks correctly, validates backend params |
| `metamind.approve` tool | Fast-path in Interpret, retry in Verify |
| Interpret pass | Pure conversation → fast-path; action request → checks + context |
| Process pass | Tool loop, budget enforcement, `agent.run` delegation |
| Verify pass | Runs checks, updates state, approves, retries on failure |
| Retry logic | Max 5 retries, error context passed correctly |

### 16.2 Integration Tests

- **Fast-path conversation** — greetings, thanks → single Interpret call
- **Action with default verification** — "create a note" → Interpret adds default check → Process creates → Verify approves
- **Action with custom verification** — "write correct Rust code" → Interpret adds LLM judge check for compilation → Process writes → Verify runs check → retry on failure
- **Multi-retry** — Process fails verification 3 times → succeeds on 4th
- **Sub-agent think** — Process calls `metamind.think` → sub-agent runs 3-pass → returns findings
- **Agent delegation** — Process calls `agent.run` → Hermes runs → summary returned

### 16.3 Property Tests

- **Budget monotonicity** — each pass spends from reserved budget; child budgets ≤ parent
- **Clean log coherence** — user messages + final replies only; no internal passes
- **Agentstream completeness** — all PromptBuilt, ToolCall, ToolResult events present
- **Retry termination** — max 5 retries always respected

---

## 17. Affected Files

| File | Change |
|------|--------|
| `crates/mm-cme/src/executor.rs` | New `pass_interpret`, `pass_process`, `pass_verify`, `execute_three_pass` |
| `crates/mm-cme/src/verification.rs` (new) | `VerificationBackend` trait, `LlmJudgeBackend`, `VerifyCheck`, `VerificationResult` |
| `crates/mm-cme/src/budget.rs` | Per-pass budget classes, child budget reservation |
| `crates/mm-cme/src/lib.rs` | Re-export verification types |
| `crates/mm-serve/src/cm_tools.rs` | `metamind.verify`, `metamind.approve`, `metamind.think` tool handlers |
| `crates/mm-serve/src/cme.rs` | `CmeRuntime::run_turn` → calls `execute_three_pass` |
| `crates/mm-serve/src/state.rs` | Clean log maintenance, per-pass context assembly |
| `crates/mm-prompt/src/lib.rs` | Strategies `cm.interpret.v1`, `cm.process.v1`, `cm.verify.v1`, `cm.think.v1` |
| `config/prompts/fragments/` | New fragments for each pass |
| `config/prompts/applications/` | New strategy files |
| `crates/mm-agent/src/lib.rs` | Sub-agent spawning for `metamind.think` and `agent.run` |
| `crates/mm-data/src/agentstream.rs` | Clean log projection |
| `frontend/src/components/chat/` | SSE pass streaming, aggregated message display |
| `frontend/src/components/inspector/` | Pass timeline, verification results display |

---

## 18. Rollback Plan

Feature flag: `MM_THREE_PASS_V2` (default `false`)

```rust
pub async fn execute(...) -> Result<TurnOutcome> {
    if cfg!(feature = "three_pass_v2") || std::env::var("MM_THREE_PASS_V2").is_ok() {
        self.execute_three_pass_v2(...).await
    } else {
        self.execute_legacy(...).await  // Current monolithic execute
    }
}
```

---

## 19. Open Questions

1. **Should Interpret have access to `data.search`?** Currently no — but it might need to check facts before designing checks.
2. **Can Verify pass call `metamind.think`?** Currently no — but deep verification might need it.
3. **Should `metamind.approve(true)` in Interpret log the reply to clean log immediately?** Yes, for context in subsequent turns.
4. **How to handle streaming tokens from Verify pass?** Verify mostly calls tools; final approve is instant. Stream minimal "Updating..." tokens.
5. **Should verification results be cached?** For idempotent checks (e.g., "code compiles"), yes.

---

## 20. References

- `conversation_manager_agent_inspector_plan.md` — Overall CM v3 architecture
- `crates/mm-cme/src/executor.rs` — Current `TurnExecutor::execute`
- `crates/mm-cme/src/turn.rs` — Turn state machine
- `crates/mm-prompt/src/lib.rs` — `PromptManager`, `PromptInstance`
- `crates/mm-agent/src/lib.rs` — `AgentRuntime`, `TurnSummarizer`
- `crates/mm-serve/src/cm_tools.rs` — Current 17-tool handler