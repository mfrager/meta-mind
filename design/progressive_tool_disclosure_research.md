# Progressive Tool Disclosure: Research Report for Rich, Large Tool Sets

**Status:** Research Report  
**Location:** `design/progressive_tool_disclosure_research.md`  
**Date:** 2026-11-21

---

## Executive Summary

Progressive tool disclosure is the practice of presenting only a relevant subset of available tools to an LLM at each turn, rather than overwhelming the context window with all tool schemas. For systems with 50-200+ tools, effective disclosure is critical for:

- **Token efficiency**: Full schemas for 100 tools = 50K-100K tokens
- **Selection accuracy**: LLMs hallucinate or pick wrong tools when overloaded
- **Latency**: Smaller prompts = faster inference
- **Safety**: Limiting available actions reduces blast radius

This report surveys academic research (2023-2026), industry implementations, and the MetaMind/Hermes approaches, identifying what **actually works** for rich tool sets.

---

## 1. The Problem Space

### 1.1 Why Full Disclosure Fails

| Failure Mode | Evidence |
|--------------|----------|
| **Hallucinated calls** | GPT-4 picks wrong API 30%+ when given 50+ schemas (Gorilla paper) |
| **Argument errors** | Schema complexity correlates with malformed arguments |
| **Context exhaustion** | 100 tools × 500 tokens/schema = 50K tokens before user message |
| **Selection paralysis** | Model defaults to familiar tools, ignores specialized ones |
| **Cost** | 10x token cost per turn for marginal utility |

### 1.2 Desiderata for Disclosure Systems

1. **Recall** — Relevant tools are available when needed
2. **Precision** — Irrelevant tools are hidden
3. **Adaptivity** — Responds to conversation context, not just keywords
4. **User control** — Operator can override/guide disclosure
5. **Auditability** — Clear why a tool was shown/hidden
6. **Extensibility** — New tools integrate without redesign

---

## 2. Academic Research (2023-2026)

### 2.1 Gorilla (Patil et al., 2023) — Retrieval-Augmented Tool Selection

**Approach**: Fine-tuned LLaMA + document retriever over API docs

```python
# Two-stage: retrieve → generate
docs = retriever.get_relevant_docs(user_query, k=3)
prompt = f"API docs:\n{docs}\n\nUser: {query}\nAPI call:"
call = gorilla_model.generate(prompt)
```

**Key Results**:
- Surpasses GPT-4 on APIBench (HuggingFace, TorchHub, TensorHub)
- Retrieval adapts to doc changes at test time
- Hallucination substantially reduced

**Limitation**: Still presents all retrieved docs' schemas to model; doesn't do progressive *per-turn* disclosure.

### 2.2 ToolBench / ToolLLM (Qin et al., 2023) — Unified Tool Learning

**Approach**: 
- ToolBench: 16,000+ APIs, instruction tuning dataset
- ToolLLaMA: LLaMA fine-tuned on ToolBench
- DFSDT (Depth-First Search Decision Tree) for multi-step reasoning

**Selection Method**: Model sees **all** tool descriptions during training; at inference uses retriever (BM25/dense) to pick top-k relevant tools.

**Key Finding**: Retrieval + fine-tuning > prompt engineering alone for tool selection.

### 2.3 SBEMA (Wang et al., 2026) — **Explicit Progressive Disclosure in 3 Levels**

**Approach**: Three-level tool disclosure for battery energy management:

| Level | Tools | Trigger |
|-------|-------|---------|
| **L1: Core** | Basic solver, constraint templates | Every turn |
| **L2: Domain** | Error patterns, physics validators | On error / complex query |
| **L3: Specialized** | Advanced optimizers, repair loops | On repeated failure |

**Results**: 
- 66.8% success with knowledge base + progressive disclosure
- 0.6% without knowledge base
- Ablation: Each level adds distinct value

**Key Insight**: **Explicit leveling** (not heuristic) with clear escalation triggers works better than keyword matching.

### 2.4 CacheRouter (2026) — Dual-Path Routing with Cache Preservation

**Approach**: 
- **Fast path**: Cached tool selections for common queries (sub-ms)
- **Slow path**: Full retrieval + reasoning for novel queries
- Main-model isolation: Router doesn't pollute main model context

**Innovation**: Separates *selection* from *execution* models; caches routing decisions.

### 2.5 Convergent Detour Hijacking (Liu et al., 2026) — **Attack on Progressive Disclosure**

**Finding**: Progressive disclosure creates attack surface:
1. Skill *description* (selection) establishes relevance
2. Skill *body* (planning) fabricates dependencies
3. Attacker skills recruited into "detour" then re-enter main path

**Implication**: Disclosure mechanisms must validate **both** selection-time and execution-time behavior.

---

## 3. Industry Implementations

### 3.1 MetaMind (Legacy Controller) — Keyword-Activated Groups

```rust
// 12 static groups, keyword heuristics
let mut groups = vec![GroupId::Core];
add(GroupId::Memory, any_word(&lower, &["remember", "recall", "forget"]));
add(GroupId::Being, any_word(&lower, &["being", "goal"]));
// ...
// LLM sees only active groups' schemas
// Escape hatch: catalog.open(group_id) — max 3 extra groups/turn
```

**Strengths**: 
- Deterministic, auditable, low overhead
- Hard cap on context growth (max 4 groups)

**Weaknesses**:
- Keyword heuristics brittle (misses "what are my objectives?" → no `being` group)
- No learning/adaptation
- New tools require group assignment + keyword tuning

### 3.2 Hermes Agent — Explicit Toolsets + Platform Gating + MCP Auto-Discovery

```yaml
# User-controlled toolsets per platform
TOOLSETS:
  core: [terminal, file, search, memory, skills]
  web: [web_search, web_extract]
  coding: [lsp_*, git_*]
  safe: [read_file, list_directory, search_files]
# MCP servers auto-injected at startup: mcp_{server}_{tool}
```

**Strengths**:
- User intent explicit (enable/disable toolsets)
- Platform-specific (Discord ≠ CLI)
- MCP = infinite extensibility without code changes
- Safe toolset for locked-down sessions

**Weaknesses**:
- No per-turn adaptation (all enabled tools visible)
- LLM still sees 50+ schemas if many toolsets enabled
- Toolset changes need `/reset` (prompt caching constraint)

### 3.3 OpenAI Assistants API — File Search + Function Calling

- Tools declared at Assistant creation
- `file_search` tool auto-indexes uploaded files
- No progressive disclosure — all functions visible

### 3.4 Anthropic Claude — Tool Use with `tool_choice`

- `tool_choice: {"type": "auto"}` — model decides
- `tool_choice: {"type": "any"}` — force tool use
- `tool_choice: {"type": "tool", "name": "X"}` — force specific
- No built-in progressive disclosure

### 3.5 LangGraph / LangChain — ToolNode + Conditional Edges

```python
# Explicit routing in graph
def route_tools(state):
    if "search" in state["messages"][-1].content.lower():
        return "search_tools"
    return "default_tools"
```

- Disclosure = graph structure (deterministic)
- Requires manual graph design per use case

---

## 4. What Actually Works: Synthesis

### 4.1 Proven Patterns (Empirical Evidence)

| Pattern | Evidence | Best For |
|---------|----------|----------|
| **Retrieval + Reranking** | Gorilla, ToolBench, ToolLLM | Large static tool sets (1000+) |
| **Explicit Leveling (L1/L2/L3)** | SBEMA (2026) | Domain-specific agents with clear escalation |
| **User-Controlled Toolsets** | Hermes Agent | Multi-platform, operator-facing agents |
| **Dual-Path Routing (Fast/Slow)** | CacheRouter (2026) | High-throughput, long-tail tool sets |
| **Deterministic Group Activation** | MetaMind | Safety-critical, auditable systems |

### 4.2 Anti-Patterns (Don't Work)

| Anti-Pattern | Why It Fails |
|--------------|--------------|
| **Pure keyword heuristics** | Misses synonyms, context, intent; brittle |
| **All tools every turn** | Token bloat, hallucination, selection errors |
| **LLM self-selection without retrieval** | Fails beyond ~20 tools (context dilution) |
| **Single-level disclosure** | Can't handle "escalation" scenarios |
| **No user override** | Operator can't guide when heuristic fails |

### 4.3 What's Missing in Current Systems

| Gap | Needed |
|-----|--------|
| **Learned disclosure policy** | Policy that adapts from success/failure (meta_analysis1.md §6) |
| **Cross-turn tool memory** | "We used `graph.query` last turn, keep it available" |
| **Cost-aware disclosure** | Hide expensive tools unless high-value (OperationValue) |
| **Adversarial robustness** | CDH attack shows selection ≠ execution safety |
| **Unified selection + execution validation** | SBEMA's 4-stage validation is a model |

---

## 5. Recommended Architecture for MetaMind CME

Based on research, the CME should evolve from MetaMind's keyword groups to a **hybrid architecture**:

```
┌─────────────────────────────────────────────────────────────┐
│                    TOOL DISCLOSURE ENGINE                    │
├─────────────────────────────────────────────────────────────┤
│  1. STATIC TIER (L0) — Always available                     │
│     • Core: help, state, focus, session.note, catalog.open │
│     • Safety: confirmations, firewall.eval                 │
│                                                             │
│  2. SEMANTIC TIER (L1) — Retrieval-activated                │
│     • Embedding index over tool summaries + examples       │
│     • Top-k by cosine similarity to query + context        │
│     • Cache: query → toolset (TTL: session)                │
│                                                             │
│  3. ESCALATION TIER (L2) — Explicit triggers               │
│     • On error → add debug/tools (logs, replay, verify)    │
│     • On "write" intent → add mutating tools (confirm)     │
│     • On "plan" → add decision/loop/change tools           │
│     • On repeated failure → add meta-analysis/repair       │
│                                                             │
│  4. USER TIER (L3) — Explicit control                       │
│     • Toolset enable/disable (persisted per project/user)  │
│     • catalog.open(group) — escape hatch                   │
│     • Project-level toolset defaults                       │
│                                                             │
│  5. ADVERSARIAL GUARD                                       │
│     • Selection-time: validate tool description matches    │
│       implementation (SBEMA knowledge base)                │
│     • Execution-time: confirm effect matches declared      │
│       (MetaMind firewall + action ledger)                  │
└─────────────────────────────────────────────────────────────┘
```

### 5.1 Implementation Priorities

| Priority | Component | Effort | Impact |
|----------|-----------|--------|--------|
| **P0** | Embedding index over tool catalog | 1 week | Recall + precision |
| **P0** | Escalation triggers (error → debug tools) | 3 days | Handles failure modes |
| **P1** | Per-project toolset config | 1 week | User control |
| **P1** | Selection caching (query → toolset) | 2 days | Latency |
| **P2** | Learned disclosure policy (meta-analysis) | 3 weeks | Adaptivity |
| **P2** | Adversarial guard (CDH mitigation) | 2 weeks | Safety |

### 5.2 Schema for Disclosure Policy

```toml
# config/tool_disclosure.toml
[static_tier]
always = ["guide.*", "state.show", "focus.*", "session.note", "catalog.open", "firewall.eval"]

[semantic_tier]
embedding_model = "nomic-embed-text"
top_k = 8
similarity_threshold = 0.35
cache_ttl_turns = 10

[escalation_tier]
triggers = [
  { on = "tool_error", add = ["logs.*", "replay.*", "doctor.*"] },
  { on = "write_intent", add = ["*.goal_add", "memory.add", "changeset.*"] },
  { on = "plan_keyword", add = ["decide.*", "loop.*", "promote.*"] },
  { on = "repeated_failure", add = ["meta.*", "debt.*", "gc.*"] },
  { on = "confirmation_required", add = ["confirm.*"] }
]
max_escalation_groups = 3

[user_tier]
toolsets = { 
  core = ["being.*", "memory.*", "epistemic.*"],
  kernel = ["doctor.*", "logs.*", "graph.*", "replay.*"],
  decide = ["decide.*", "firewall.*", "risk.*", "calibration.*"],
  change = ["changeset.*", "sandbox.*", "promote.*", "reject.*"],
  loop = ["loop.*", "self_model.*", "debt.*", "gc.*", "module.*"],
  author = ["pi.*", "codex.*", "experience.*"]
}
default_enabled = ["core"]
per_project_overrides = true

[adversarial_guard]
validate_description_matches_impl = true
validate_effect_matches_declared = true
```

---

## 6. Evaluation Methodology

To validate any disclosure system, measure:

| Metric | Target | Method |
|--------|--------|--------|
| **Tool Recall@k** | > 0.95 | Held-out queries with known relevant tools |
| **Tool Precision@k** | > 0.8 | Human eval of disclosed set relevance |
| **Selection Accuracy** | > 0.9 | End-to-end: correct tool chosen for task |
| **Token Reduction** | > 70% | vs. full catalog baseline |
| **Latency Overhead** | < 50ms | Disclosure engine latency |
| **Escalation Success** | > 0.9 | Error → correct debug tools added |
| **User Override Rate** | < 5% | catalog.open / toolset enable frequency |
| **Adversarial Robustness** | 0 CDH | Red-team evaluation |

**Benchmark Suites**:
- ToolBench selection/eval split
- APIBench (Gorilla)
- MetaMind `bench/tools/` adversarial fixtures
- Custom: MetaMind-specific scenarios (self-engineering, epistemic, etc.)

---

## 7. Related Work References

| Paper / System | Year | Key Contribution |
|----------------|------|------------------|
| Gorilla (Patil et al.) | 2023 | Retrieval + fine-tuning for API calls |
| ToolBench (Qin et al.) | 2023 | 16K API benchmark, ToolLLaMA |
| ToolLLM (Qin et al.) | 2023 | DFSDT multi-step reasoning |
| SBEMA (Wang et al.) | 2026 | **3-level progressive disclosure + validation** |
| CacheRouter (Liu et al.) | 2026 | **Dual-path routing + cache preservation** |
| CDH Attack (Liu et al.) | 2026 | **Adversarial exploitation of progressive disclosure** |
| ReAct (Yao et al.) | 2022 | Reasoning + acting loop |
| Tree of Thoughts (Yao et al.) | 2023 | Explicit exploration + self-evaluation |
| MetaMind chat/02_tool_surface.md | 2024 | Keyword groups + catalog.open escape hatch |
| Hermes Agent toolsets | 2024 | User-controlled toolsets + MCP auto-discovery |

---

## 8. Conclusion

**For MetaMind's ~70-tool kernel-oriented set**, the research supports a **4-tier hybrid**:

1. **Static core** (always) — safety, navigation, help
2. **Semantic retrieval** (per-turn) — embedding-based relevance
3. **Explicit escalation** (on triggers) — error, write-intent, planning, failure
4. **User toolsets** (configured) — project/platform control

This combines:
- **SBEMA's** proven leveling + validation
- **CacheRouter's** fast/slow path + caching
- **Hermes's** user control + MCP extensibility
- **MetaMind's** firewall/ledger safety + deterministic admission

The keyword heuristic in `chat/02_tool_surface.md` should be **replaced with embedding retrieval** while keeping the **group abstraction** and **catalog.open escape hatch**. The escalation triggers should be **codified in config** (not hardcoded) to enable learning via meta-analysis.

---

## Appendix: Quick Reference — Disclosure Decision Tree

```
USER MESSAGE
     │
     ▼
┌──────────────────────────────────────┐
│ STATIC TIER (always)                 │
│ guide.*, state.show, focus.*,        │
│ session.note, catalog.open,          │
│ firewall.eval                        │
└──────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────┐
│ SEMANTIC TIER                        │
│ embed(query + recent_context)        │
│ → top-k tools by cosine similarity   │
│ → filter by threshold (0.35)         │
│ → cache result (10 turns)            │
└──────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────┐
│ ESCALATION CHECKS (in order)         │
│ 1. tool_error?       → + debug tools │
│ 2. write_intent?     → + mutating    │
│ 3. plan_keywords?    → + decide/loop │
│ 4. repeated_failure? → + meta/repair │
│ 5. confirmation?     → + confirm     │
│ (max 3 escalation groups)            │
└──────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────┐
│ USER TOOLSETS                        │
│ enabled toolsets for this project/   │
│ platform (persisted config)          │
└──────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────┐
│ ADVERSARIAL GUARD                    │
│ validate description ↔ impl          │
│ validate effect ↔ declared           │
└──────────────────────────────────────┘
     │
     ▼
  TOOL SCHEMAS TO LLM
```