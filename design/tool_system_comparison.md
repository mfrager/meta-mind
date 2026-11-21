# Tool System Comparison: MetaMind vs Hermes Agent

**Status:** Reference Document  
**Location:** `design/tool_system_comparison.md`  
**Date:** 2026-11-21

---

## 1. Tool Philosophy & Architecture

| Aspect | MetaMind (mm-serve legacy) | Hermes Agent |
|--------|---------------------------|--------------|
| **Core model** | In-process Rust functions, dispatched via catalog | Python async functions, registered in tool registry |
| **Protocol** | Custom JSON over SSE + internal `ToolResult` | MCP-shaped (native client) + internal tool calling |
| **Extensibility** | Add Rust functions to `exec.rs::dispatch()` | Add Python functions to `toolsets.py` or MCP servers |
| **External integrations** | Via `mm-tools` crate (MCP bridge + sandbox) | Native MCP client (stdio/HTTP), plugins |
| **Authorization** | Firewall admission + confirmation gating (`IfMutating`/`Always`) | Approval modes (smart/manual/off), per-toolset enable/disable |
| **Safety** | Default-deny, action ledger, rollback, unbacked-claim detection | Secret redaction, website blocklist, approval gates, safe toolset |

---

## 2. Complete Tool Inventory

### 2.1 MetaMind Legacy Controller (70 tools, 12 groups)

| Group | Tool | Effect | Confirm | Focus | Description |
|-------|------|--------|---------|-------|-------------|
| **Core** | `guide.list` | Read | Never | None | List guide chapters |
|  | `guide.read` | Read | Never | None | Read guide chapter by id |
|  | `guide.search` | Read | Never | None | Search guide by token overlap |
|  | `catalog.open` | Read | Never | None | Open tool group for this turn |
|  | `state.show` | Read | Never | State | Read health, being, budgets, goals |
|  | `focus.open` | Read | Never | Document | Open document console serves |
|  | `focus.read` | Read | Never | None | Read body of served document |
|  | `session.note` | Write | Never | None | Keep note on this chat only |
|  | `focus.patch` | Write | IfMutating | Document | Patch live goal/memory via owning tool |
| **Kernel** | `doctor.run` | Read | Never | State | Kernel health (names graph holder) |
|  | `logs.head` | Read | Never | Document | Read event log head sequence |
|  | `logs.verify` | Read | Never | None | Check audit chain |
|  | `replay.run` | Read | Never | None | Replay events, return state hash |
|  | `graph.query` | Read | Never | None | Read-only SPARQL on graph |
|  | `graph.validate` | Read | Never | None | Validate named graph vs shapes |
| **Being** | `being.show` | Read | Never | State | Identity, invariants, affect, goal counts |
|  | `being.goal_add` | Write | IfMutating | Document | Add goal (desc + priority) |
|  | `being.goal_list` | Read | Never | Document | List goals |
|  | `being.verify` | Read | Never | None | Run invariant corpus |
|  | `being.personality_show` | Read | Never | None | Values, dispositions, projected voice |
| **Memory** | `memory.add` | Write | IfMutating | None | Store semantic/episodic memory |
|  | `memory.recall` | Read | Never | Document | Recall with 5 score terms |
|  | `memory.forget` | Write | Always | None | Forget memories (dry-run default) |
|  | `memory.verify` | Read | Never | None | Reconcile SQLite with memory graph |
| **Epistemic** | `epistemic.ingest` | Write | IfMutating | None | Ingest one epistemic claim |
|  | `epistemic.show` | Read | Never | None | Claim + assumption counts |
| **Library** | `library.applicable` | Read | Never | None | Rank entries applying to goal |
|  | `policy.history` | Read | Never | None | List versions of one policy |
|  | `skill.retrieve` | Read | Never | None | Retrieve verified skills for query |
|  | `case.retrieve` | Read | Never | None | Retrieve cases matching title |
|  | `frame.missing` | Read | Never | None | List unknown slots in frame instance |
| **Decide** | `decide.ask` | Write | Never | Document | Ask decision core (rules runs now) |
|  | `firewall.eval` | Read | Never | None | Evaluate episode/action (firewall) |
|  | `compare.check` | Read | Never | None | Compare baseline vs candidate |
|  | `risk.analyze` | Read | Never | None | Score loss outcomes (risk) |
|  | `calibration.show` | Read | Never | None | Read calibration rows |
|  | `conformance.run` | Read | Never | None | Run rules core conformance |
| **Change** | `changeset.new` | Write | IfMutating | Document | Open change set from gap/title |
|  | `changeset.show` | Read | Never | Document | List change sets |
|  | `sandbox.run` | Write | Always | Document | Prepare sandbox worktree |
|  | `promote.run` | Gate | Always | None | Promote change set through gate |
|  | `reject.run` | Gate | Always | None | Reject change set (reason ≥8 chars) |
|  | `audit.production_tree` | Read | Never | None | Audit promotions, list prod writes |
|  | `budget.show` | Read | Never | Document | Read four budget rows |
| **Loop** | `loop.run` | Gate | Always | None | Run improvement loop for goal |
|  | `loop.status` | Read | Never | None | Read loop run rows |
|  | `self_model.report` | Read | Never | None | Measure one loop run |
|  | `debt.scan` | Read | Never | None | Scan for architectural debt |
|  | `gc.propose` | Read | Never | None | Propose GC actions (no apply) |
|  | `gc.apply` | Gate | Always | None | Apply proposed GC actions |
|  | `module.load` | Gate | Always | None | Load materialised module via changeset |
|  | `module.rollback` | Gate | Always | None | Restore previous module version |
|  | `design.show` | Read | Never | None | List design revisions of document |
| **Sandbox** | `sandbox_tool.list` | Read | Never | None | List in-process sandbox tools |
|  | `sandbox_tool.describe` | Read | Never | None | Describe one sandbox tool |
|  | `sandbox_tool.run` | External | Always | None | Call sandbox tool as operator |
|  | `action.ledger` | Read | Never | None | Read action ledger |
|  | `action.rollback` | Write | Always | None | Restore snapshot action took |
| **Author** | `pi.run` | External | Always | None | Run Pi binary (absence = result) |
|  | `pi.ingest` | Write | Always | None | Ingest recorded Pi session |
| **Later** | `codex.scan` | Read | Never | None | Scan workspace → codex report |
|  | `codex.verify` | Read | Never | None | Verify codex report |
|  | `experience.compile` | Read | Never | None | Compile trajectories → candidates |
|  | `llm.stats` | Read | Never | None | Count LLM calls this process |
|  | `meta.analyze` | Read | Never | None | Analyze one meta episode file |
|  | `regression.run` | Read | Never | None | Run regression suite |
|  | `verify.run` | Read | Never | None | Name verify obligation |
|  | `calibrate.score` | Read | Never | None | Read calibrate scores |
|  | `mcp.list` | Read | Never | None | List in-process tools (MCP style) |

---

### 2.2 Hermes Agent (25+ toolsets, 100+ tools)

| Toolset | Tool | Description |
|---------|------|-------------|
| **file** | `read_file` | Read file contents |
|  | `write_file` | Write file (create/overwrite) |
|  | `patch_file` | Targeted find-replace edit |
|  | `list_directory` | List directory contents |
|  | `search_files` | Regex search in files |
|  | `glob_files` | Find files by pattern |
|  | `delete_file` | Delete file |
|  | `copy_file` | Copy file |
|  | `move_file` | Move/rename file |
| **terminal** | `run_command` | Execute shell command |
|  | `run_command_background` | Start background process |
|  | `process_list` | List background processes |
|  | `process_kill` | Kill background process |
|  | `process_wait` | Wait for process completion |
| **web** | `web_search` | Search web (Brave/DuckDuckGo/Google) |
|  | `web_extract` | Extract content from URL |
|  | `web_fetch` | Fetch raw HTML |
| **search** | `search` | Unified search (web + local) |
| **browser** | `browser_navigate` | Navigate to URL |
|  | `browser_click` | Click element |
|  | `browser_type` | Type in element |
|  | `browser_screenshot` | Capture screenshot |
|  | `browser_eval` | Execute JS in page |
|  | `browser_wait` | Wait for condition |
| **code_execution** | `python_exec` | Execute Python in sandbox |
|  | `python_repl` | Persistent REPL session |
| **coding** | `lsp_hover` | Hover symbol info |
|  | `lsp_goto_definition` | Go to definition |
|  | `lsp_references` | Find references |
|  | `lsp_diagnostics` | Get diagnostics |
|  | `lsp_rename` | Rename symbol |
|  | `lsp_format` | Format document |
|  | `lsp_code_action` | Apply code action |
|  | `git_status` | Git status |
|  | `git_diff` | Git diff |
|  | `git_log` | Git log |
|  | `git_commit` | Create commit |
|  | `git_push` | Push to remote |
| **computer_use** | `cua_screenshot` | Desktop screenshot |
|  | `cua_click` | Click at coordinates |
|  | `cua_type` | Type text |
|  | `cua_key` | Press key |
|  | `cua_scroll` | Scroll |
|  | `cua_drag` | Drag |
| **vision** | `analyze_image` | Analyze image (local/cloud) |
| **image_gen** | `generate_image` | Text-to-image |
|  | `edit_image` | Image-to-image editing |
| **video** | `analyze_video` | Video analysis |
| **video_gen** | `generate_video` | Text-to-video |
| **tts** | `speak` | Text-to-speech |
| **stt** | `transcribe` | Speech-to-text |
| **memory** | `memory_write` | Write to persistent memory |
|  | `memory_read` | Read from persistent memory |
|  | `memory_search` | Search memory |
|  | `memory_delete` | Delete memory |
| **session_search** | `search_sessions` | Search past conversations |
|  | `session_export` | Export session |
| **context_engine** | `context_inject` | Inject context hook |
|  | `context_extract` | Extract context |
| **project** | `project_add_folder` | Add folder to workspace |
|  | `project_remove_folder` | Remove folder |
|  | `project_list` | List project folders |
| **delegation** | `spawn_agent` | Spawn subagent task |
|  | `agent_status` | Check subagent status |
|  | `agent_result` | Get subagent result |
| **cronjob** | `cron_add` | Add scheduled task |
|  | `cron_remove` | Remove scheduled task |
|  | `cron_list` | List scheduled tasks |
| **clarify** | `ask_question` | Ask user clarifying question |
| **todo** | `todo_add` | Add todo item |
|  | `todo_update` | Update todo |
|  | `todo_list` | List todos |
| **kanban** | `kanban_show` | Show board (worker) |
|  | `kanban_complete` | Complete task (worker) |
|  | `kanban_assign` | Assign task (orchestrator) |
|  | `kanban_link` | Link tasks (orchestrator) |
| **safe** | `read_file` | Read only |
|  | `list_directory` | List only |
|  | `search_files` | Search only |
|  | `web_search` | Search only |
| **skills** | `skill_list` | List installed skills |
|  | `skill_install` | Install skill |
|  | `skill_remove` | Remove skill |
|  | `skill_update` | Update skill |
|  | `skill_view` | View skill content |
| **x_search** | `x_search` | X/Twitter search |
| **discord** | `discord_send` | Send Discord message |
|  | `discord_read` | Read Discord channel |
|  | `discord_admin_*` | Admin tools (gated) |
| **feishu_doc** | `feishu_doc_*` | Feishu document tools |
| **feishu_drive** | `feishu_drive_*` | Feishu drive tools |
| **yuanbao** | `yuanbao_*` | Yuanbao group tools |
| **MCP (auto-discovered)** | `mcp_filesystem_read_file` | From filesystem server |
|  | `mcp_filesystem_write_file` | From filesystem server |
|  | `mcp_filesystem_list_directory` | From filesystem server |
|  | `mcp_github_list_issues` | From GitHub server |
|  | `mcp_github_create_pr` | From GitHub server |
|  | `mcp_github_comment` | From GitHub server |
|  | `mcp_time_get_current_time` | From time server |
|  | `mcp_postgres_query` | From PostgreSQL server |
|  | `mcp_sqlite_query` | From SQLite server |
|  | `mcp_redis_get` | From Redis server |
|  | `mcp_redis_set` | From Redis server |
|  | `mcp_brave_search` | From Brave search server |
|  | `mcp_...` | Any MCP server tools |

---

## 3. Direct Capability Mapping

| Capability | MetaMind Tool(s) | Hermes Tool(s) |
|------------|------------------|----------------|
| **Read system health** | `doctor.run`, `state.show` | `hermes doctor` (CLI), `session_search` |
| **Read event log** | `logs.head`, `logs.verify`, `replay.run` | — (no event log) |
| **SPARQL query** | `graph.query` | `mcp_postgres_query`, `mcp_sqlite_query` |
| **Add goal** | `being.goal_add` | `todo_add` + `memory_write` |
| **List goals** | `being.goal_list` | `todo_list`, `memory_search` |
| **Store memory** | `memory.add` | `memory_write` |
| **Recall memory** | `memory.recall` | `memory_read`, `memory_search` |
| **Forget memory** | `memory.forget` | `memory_delete` |
| **Decision/firewall** | `decide.ask`, `firewall.eval` | — (LLM reasoning) |
| **Risk analysis** | `risk.analyze` | — |
| **Compare strings** | `compare.check` | — |
| **Calibration** | `calibration.show`, `calibrate.score` | — |
| **Self-engineering** | `changeset.*`, `promote.run`, `loop.run`, `debt.scan` | — |
| **Module hot-load** | `module.load`, `module.rollback` | — |
| **Run sandbox tool** | `sandbox_tool.run` | `run_command` (docker backend) |
| **Action ledger/rollback** | `action.ledger`, `action.rollback` | `cronjob` + checkpoints |
| **Pi integration** | `pi.run`, `pi.ingest` | — |
| **Codex scan/verify** | `codex.scan`, `codex.verify` | `skill_install` (different) |
| **File read/write** | `focus.open/read/patch` | `read_file`, `write_file`, `patch_file` |
| **Shell commands** | `sandbox_tool.run` | `run_command` |
| **Web search** | — | `web_search`, `web_extract` |
| **Code LSP** | — | `lsp_*` tools |
| **Git operations** | — | `git_*` tools |
| **Desktop automation** | — | `cua_*` tools |
| **Voice I/O** | — | `speak`, `transcribe` |
| **Multi-agent** | — | `spawn_agent`, `kanban_*` |
| **MCP external** | `mcp.list` (ingress only) | `mcp_*` (auto-discovered) |

---

## 4. Tools Unique to Each System

### Only in MetaMind (kernel-oriented)

- **Event log:** `logs.head`, `logs.verify`, `replay.run`
- **SPARQL:** `graph.query`, `graph.validate`
- **Being substrate:** `being.show`, `being.verify`, `being.personality_show`
- **Epistemic:** `epistemic.ingest`, `epistemic.show`
- **Cognitive library:** `library.applicable`, `policy.history`, `skill.retrieve`, `case.retrieve`, `frame.missing`
- **Decision cores:** `decide.ask`, `firewall.eval`, `compare.check`, `risk.analyze`
- **Calibration:** `calibration.show`, `conformance.run`, `calibrate.score`
- **Self-engineering:** `changeset.*`, `sandbox.run`, `promote.run`, `reject.run`, `audit.production_tree`, `budget.show`, `loop.*`, `self_model.report`, `debt.scan`, `gc.*`, `module.*`, `design.show`
- **Action ledger:** `action.ledger`, `action.rollback`
- **Pi:** `pi.run`, `pi.ingest`

### Only in Hermes (operator-oriented)

- **File CRUD:** `read_file`, `write_file`, `patch_file`, `delete_file`, `copy_file`, `move_file`
- **Shell:** `run_command`, `run_command_background`, `process_*`
- **Web:** `web_search`, `web_extract`, `web_fetch`, `x_search`
- **Browser:** `browser_*`
- **Code execution:** `python_exec`, `python_repl`
- **LSP:** `lsp_*`
- **Git:** `git_*`
- **Desktop:** `cua_*`
- **Vision/Media:** `analyze_image`, `generate_image`, `edit_image`, `analyze_video`, `generate_video`, `speak`, `transcribe`
- **Persistent memory:** `memory_write`, `memory_read`, `memory_search`, `memory_delete`
- **Session search:** `search_sessions`, `session_export`
- **Project workspace:** `project_*`
- **Delegation:** `spawn_agent`, `agent_status`, `agent_result`
- **Scheduling:** `cron_add`, `cron_remove`, `cron_list`
- **Clarification:** `ask_question`
- **Planning:** `todo_add`, `todo_update`, `todo_list`
- **Kanban:** `kanban_*`
- **Skills mgmt:** `skill_list`, `skill_install`, `skill_remove`, `skill_update`, `skill_view`
- **Platform integrations:** `discord_*`, `feishu_*`, `yuanbao_*`
- **MCP egress:** `mcp_*` (auto-discovered from external servers)

---

## 5. Overlap (Different Names, Similar Purpose)

| MetaMind | Hermes | Notes |
|----------|--------|-------|
| `focus.open/read` | `read_file` | Hermes more general |
| `focus.patch` | `patch_file` | Same JSON-patch style |
| `catalog.open` | `skill_list` / toolsets | Hermes explicit toolsets |
| `guide.*` | `skill_view` / docs | Hermes skills self-document |
| `session.note` | `memory_write` (ephemeral) | Hermes distinguishes persistent vs session |
| `memory.recall` | `memory_search` | Hermes uses embeddings |
| `sandbox_tool.run` | `run_command` (docker) | Hermes backend-agnostic |
| `mcp.list` (ingress) | `mcp_*` (egress) | Opposite directions |

---

## 6. Progressive Tool Disclosure Comparison

### MetaMind: Static Catalog + Dynamic Group Activation

```rust
// 1. All 70 tools defined at compile time in catalog.rs
const ROWS: &[Row] = &[...];

// 2. Core group always available
let mut groups = vec![GroupId::Core];

// 3. Additional groups activated by keyword matching on user query
add(GroupId::Memory, any_word(&lower, &["remember", "recall", "forget"]));
add(GroupId::Being, any_word(&lower, &["being", "goal"]));
add(GroupId::Loop, any_word(&lower, &["loop", "debt", "gc", "module"]));
// ... 12 groups total

// 4. LLM sees ONLY the tools in activated groups
fn tool_schemas(groups: &[GroupId]) -> Vec<ToolSchema> {
    catalog::table()
        .iter()
        .filter(|spec| groups.contains(&spec.group))
        .map(|spec| ToolSchema { ... })
        .collect()
}
```

**Mechanism:**
- **Compile-time**: Full catalog baked in
- **Turn-time**: Keyword heuristic selects relevant groups
- **LLM sees**: Only schemas for active groups (typically 15-30 tools)
- **User control**: `catalog.open` tool explicitly opens a group for the turn

**Limitations:** Heuristic-based, no user-facing "toolset" concept, cannot dynamically load without rebuild.

### Hermes Agent: Explicit Toolsets + Platform Gating + MCP Auto-Discovery

```python
# 1. Toolsets defined in toolsets.py (25+ named bundles)
_HERMES_CORE_TOOLS = ["terminal", "file", "search", "memory", "skills", ...]
TOOLSETS = {
    "web": ["web_search", "web_extract"],
    "coding": ["lsp_hover", "lsp_goto_definition", ...],
    "safe": ["read_file", "list_directory", ...],
    "kanban": ["kanban_show", "kanban_complete", ...],  # gated
    ...
}

# 2. Per-platform enablement (CLI, Discord, Telegram, etc.)
#    User runs: `hermes tools enable web` or `hermes tools` (curses UI)

# 3. MCP servers auto-discovered at startup → injected into ALL platform toolsets
#    mcp_filesystem_read_file, mcp_github_list_issues, etc.

# 4. Toolset changes take effect on NEXT session (/reset) — preserves prompt caching
```

**Mechanism:**
- **Config-time**: User explicitly enables/disables toolsets per platform
- **Startup**: MCP servers connected, tools auto-registered with `mcp_{server}_{tool}` prefix
- **Session**: LLM sees ALL enabled tools (no per-turn filtering)
- **User control**: Interactive `hermes tools` TUI, `/tools` slash command, per-invocation `-t` flag

**Advantages:** Explicit user control, MCP = infinite extensibility, platform-specific gating, safe toolset.

---

## 7. Tool Invocation Flow

| Step | MetaMind | Hermes |
|------|----------|--------|
| **1. Planning** | Deterministic `TurnPlanner` builds `TurnPlan` with `ToolPolicy` | LLM decides (no deterministic planner) |
| **2. Schema injection** | Only active-group schemas in prompt | All enabled tool schemas in system prompt |
| **3. LLM calls tool** | `TOOL_CALL` blocks intercepted by CME/executor | Native tool calling (OpenAI/Anthropic format) |
| **4. Admission** | `admit::admit()` → firewall → confirmation check | Approval mode check (smart/manual/off) |
| **5. Execution** | In-process Rust async (`exec.rs::dispatch`) | Python async (built-in or MCP client) |
| **6. Result** | `ToolResult` → injected as `Message::tool` → LLM continues | Tool result → LLM continues |
| **7. Safety** | Unbacked-claim detection, action ledger, rollback | Secret redaction, approval audit log |

---

## 8. Safety & Authorization Models

| Feature | MetaMind | Hermes |
|---------|----------|--------|
| **Default policy** | Default-deny (tools unavailable unless admitted) | Default-allow (enabled toolsets = available) |
| **Confirmation** | `Confirm::IfMutating` / `Always` → parks turn, asks user | `approvals.mode: smart` (auto-approve read, prompt write) |
| **Firewall** | `SanityFirewall` evaluates effect (Read/Write/Gate/External) | No firewall — relies on approval + toolset gating |
| **Rollback** | Action ledger + snapshot per `action.rollback` | Checkpoints (session snapshots, max 50) |
| **Audit** | Hash-chained action ledger, audit chain verification | Approval log, tool call transcripts |
| **Unbacked claims** | Detects "I stored X" without `memory.add` tool call | — |
| **MCP security** | Principal required, env filtering, credential stripping | Same (env filtering, credential redaction in errors) |

---

## 9. Extensibility Comparison

| Method | MetaMind | Hermes |
|--------|----------|--------|
| **Add internal tool** | Add Rust fn to `exec.rs`, Row to `catalog.rs`, rebuild | Add Python fn to `tools/`, register in `toolsets.py` |
| **Add external capability** | Write MCP server, `mm-tools` ingests via `McpBridge` | Write MCP server, add to `config.yaml` → auto-discovered |
| **Plugin system** | WASM plugins (designed, not wired) | `hermes plugins install <name>` (JS/TS desktop plugins) |
| **Sandbox** | `mm-tools` sandbox tiers (capability-bounded) | `terminal` backends (docker/ssh/modal/daytona/singularity) |
| **Hot reload** | No (Rust compile) | No (toolset changes need `/reset`) |

---

## 10. Summary: Which Approach Where?

| Scenario | Better Fit |
|----------|------------|
| **Kernel self-management** (event log, being, epistemic, self-engineering loops) | **MetaMind** — tools map 1:1 to kernel operations |
| **Operator productivity** (files, shell, web, code, media, external APIs) | **Hermes** — broader, more ergonomic, plugin ecosystem |
| **Progressive disclosure via keyword heuristic** | **MetaMind** — reduces LLM context, but brittle |
| **Progressive disclosure via explicit toolsets** | **Hermes** — user-controlled, predictable, auditable |
| **MCP as first-class extension** | **Both** — MetaMind via `mm-tools` bridge, Hermes via native client |
| **Multi-agent delegation** | **Hermes** — built-in `delegation` toolset, subagent spawning |
| **Safety-critical mutations** | **MetaMind** — firewall + ledger + rollback + unbacked detection |
| **Locked-down sessions** | **Hermes** — `safe` toolset, approval modes, per-platform gating |

---

## 11. Convergence Opportunity

MetaMind's CME *has the infrastructure* for Hermes-style tooling (`ToolHandler` trait, confirmation, policy) but lacks:
1. Tool registry wired to `mm-tools`/`mm-cme`
2. Tool schemas exposed to planner
3. Toolset concept (group → toolset, enable/disable per platform)

Hermes *has* the toolset/UIs/MCP but lacks:
1. Kernel-oriented tools (event log, being, epistemic, self-engineering)
2. Deterministic planner with tool policy
3. Unbacked-claim detection, action ledger, hash-chained audit

**Ideal hybrid**: Hermes' toolset/MCP/plugin UX + MetaMind's kernel tools + CME's deterministic planner + firewall/ledger safety.