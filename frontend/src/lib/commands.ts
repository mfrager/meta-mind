/**
 * The 42 top-level `mm-cli` subcommands (`Command` in crates/mm-cli/src/main.rs).
 * The palette lists them. Execution arrives with the chat orchestrator.
 */

export type CommandGroup =
  | "Kernel"
  | "Knowledge"
  | "Decision"
  | "Tools"
  | "Self-engineering"
  | "Runtime";

export type ConsoleCommand = {
  id: string;
  title: string;
  group: CommandGroup;
  /** Canonical invocation, without the `mm` binary prefix. */
  argv: string;
  summary: string;
  /** Sidebar document to open beside the passthrough notice. */
  doc?: string;
};

export const COMMAND_GROUPS: CommandGroup[] = [
  "Kernel",
  "Knowledge",
  "Decision",
  "Tools",
  "Self-engineering",
  "Runtime",
];

export const COMMANDS: ConsoleCommand[] = [
  {
    id: "doctor",
    title: "Doctor",
    group: "Kernel",
    argv: "doctor",
    summary: "Open every store and prove the kernel is writable.",
    doc: "doctor",
  },
  {
    id: "replay",
    title: "Replay",
    group: "Kernel",
    argv: "replay",
    summary: "Rebuild state from the event log and report the hash it reached.",
  },
  {
    id: "logs",
    title: "Logs",
    group: "Kernel",
    argv: "logs verify",
    summary: "Schema, audit completeness, chain integrity, redaction, replay.",
    doc: "logs",
  },
  {
    id: "gc",
    title: "Garbage collection",
    group: "Kernel",
    argv: "gc",
    summary: "Reversible cognitive garbage collection, cheapest intervention first.",
  },
  {
    id: "being",
    title: "Being",
    group: "Knowledge",
    argv: "being show",
    summary: "The persistent being: identity, blocks, beliefs, goals, relationships.",
    doc: "being",
  },
  {
    id: "memory",
    title: "Memory",
    group: "Knowledge",
    argv: "memory recall",
    summary: "The long-term memory organ: add, recall, consolidate, forget, eval, verify.",
    doc: "memory",
  },
  {
    id: "epistemic",
    title: "Epistemics",
    group: "Knowledge",
    argv: "epistemic contradictions",
    summary: "Claims, promotion, contradictions, justification.",
    doc: "epistemic",
  },
  {
    id: "library",
    title: "Library",
    group: "Knowledge",
    argv: "library list",
    summary: "The cognitive library: entries, validation, applicability, extraction.",
    doc: "library",
  },
  {
    id: "skill",
    title: "Skills",
    group: "Knowledge",
    argv: "skill retrieve",
    summary: "The verified skill library: register, verify, retrieve.",
  },
  {
    id: "case",
    title: "Cases",
    group: "Knowledge",
    argv: "case retrieve",
    summary: "Structure-mapped case retrieval.",
  },
  {
    id: "experience",
    title: "Experience",
    group: "Knowledge",
    argv: "experience compile",
    summary: "Trajectories and insights into draft candidates.",
  },
  {
    id: "policy",
    title: "Policy",
    group: "Knowledge",
    argv: "policy check",
    summary: "The policy genome: immutable versions, fitness, evolution.",
  },
  {
    id: "frame",
    title: "Frames",
    group: "Knowledge",
    argv: "frame compose",
    summary: "Conceptual frames: composition and missing slots.",
  },
  {
    id: "decide",
    title: "Decide",
    group: "Decision",
    argv: "decide",
    summary: "Answer one bounded question and record the answer.",
  },
  {
    id: "firewall",
    title: "Firewall",
    group: "Decision",
    argv: "firewall eval",
    summary: "Evaluate one episode, or a corpus of inputs.",
  },
  {
    id: "compare",
    title: "Compare",
    group: "Decision",
    argv: "compare check",
    summary: "Check a pair of comparison contracts, or a fixtures corpus.",
  },
  {
    id: "risk",
    title: "Risk",
    group: "Decision",
    argv: "risk analyze",
    summary: "Risk measures over an outcome distribution, graded against references.",
  },
  {
    id: "calibration",
    title: "Calibration fit",
    group: "Decision",
    argv: "calibration fit",
    summary: "Fit one temperature per decision class and report ECE before and after.",
  },
  {
    id: "conformance",
    title: "Conformance",
    group: "Decision",
    argv: "conformance",
    summary: "Run the one conformance suite against the named cores.",
  },
  {
    id: "calibrate",
    title: "Calibrate",
    group: "Decision",
    argv: "calibrate",
    summary: "Score a staked probability against what happened, and record the run.",
  },
  {
    id: "codex",
    title: "Codex",
    group: "Tools",
    argv: "codex scan",
    summary: "Scan, verify, query, and render the code graph.",
  },
  {
    id: "llm",
    title: "LLM",
    group: "Tools",
    argv: "llm stats",
    summary: "The LLM substrate: ledger, replay, cache, schemas, routing, grammars.",
    doc: "llm",
  },
  {
    id: "tool",
    title: "Tools",
    group: "Tools",
    argv: "tool list",
    summary: "What is registered, what it declares, and what happens when it runs.",
    doc: "tools",
  },
  {
    id: "action",
    title: "Actions",
    group: "Tools",
    argv: "action ledger",
    summary: "List the action ledger, verify its chain, and roll an action back.",
  },
  {
    id: "verify",
    title: "Verify",
    group: "Tools",
    argv: "verify run",
    summary: "A build, a test run, static analysis, a refutation, residuals.",
  },
  {
    id: "mcp",
    title: "MCP",
    group: "Tools",
    argv: "mcp list",
    summary: "List tools, call one, or serve over stdio.",
  },
  {
    id: "graph",
    title: "Graph",
    group: "Tools",
    argv: "graph validate",
    summary: "SHACL-validate a named graph.",
  },
  {
    id: "meta",
    title: "Meta",
    group: "Self-engineering",
    argv: "meta analyze",
    summary: "The trigger queue, the diagnosis, and the lessons.",
  },
  {
    id: "regression",
    title: "Regressions",
    group: "Self-engineering",
    argv: "regression run",
    summary: "The regression suite the mistake compiler writes.",
    doc: "regressions",
  },
  {
    id: "changeset",
    title: "Change set",
    group: "Self-engineering",
    argv: "changeset show",
    summary: "Assemble a change set from a gap, and read it back.",
    doc: "changesets",
  },
  {
    id: "sandbox",
    title: "Sandbox",
    group: "Self-engineering",
    argv: "sandbox run",
    summary: "The sandbox a candidate is built and tested in.",
  },
  {
    id: "promote",
    title: "Promote",
    group: "Self-engineering",
    argv: "promote",
    summary: "Judge a change set: evaluate the gate, record the decision, version the lineage.",
  },
  {
    id: "reject",
    title: "Reject",
    group: "Self-engineering",
    argv: "reject",
    summary: "Reject a change set by hand, with a written reason.",
  },
  {
    id: "budget",
    title: "Budgets",
    group: "Self-engineering",
    argv: "budget show",
    summary: "The four self-engineering budgets.",
    doc: "budgets",
  },
  {
    id: "pi",
    title: "Pi",
    group: "Self-engineering",
    argv: "pi run",
    summary: "Run a Pi task, or ingest a recorded session.",
    doc: "pi",
  },
  {
    id: "audit",
    title: "Audit",
    group: "Self-engineering",
    argv: "audit production-tree",
    summary: "Prove the production tree was not written outside a promotion.",
  },
  {
    id: "debt",
    title: "Debt",
    group: "Self-engineering",
    argv: "debt scan",
    summary: "Architectural-debt scanning over the code graph, the memory, and the policies.",
  },
  {
    id: "design",
    title: "Design",
    group: "Self-engineering",
    argv: "design history",
    summary: "Design revisions, which reach a document only through a promoted change set.",
  },
  {
    id: "episode",
    title: "Episode",
    group: "Runtime",
    argv: "episode run",
    summary: "The metacognitive controller: run, replay, verify, value, audit, program.",
    doc: "runs",
  },
  {
    id: "loop",
    title: "Loop",
    group: "Runtime",
    argv: "loop status",
    summary: "The closed developmental loop: run it, read what it produced, reproduce it.",
    doc: "runs",
  },
  {
    id: "self-model",
    title: "Self-model",
    group: "Runtime",
    argv: "self-model report",
    summary: "Actual vs Model vs Ideal, per run.",
  },
  {
    id: "module",
    title: "Modules",
    group: "Runtime",
    argv: "module load",
    summary: "Hot-load a promoted module version, without restarting the process.",
    doc: "modules",
  },
];

const BESPOKE = new Set([
  "doctor",
  "replay",
  "graph",
  "logs",
  "being",
  "decide",
  "firewall",
  "risk",
  "verify",
  "meta",
  "changeset",
  "sandbox",
  "promote",
  "reject",
  "budget",
  "audit",
  "module",
  "design",
]);

/** Matches `crates/mm-serve/src/parity.rs`. */
export function commandTier(id: string): "bespoke" | "passthrough" {
  return BESPOKE.has(id) ? "bespoke" : "passthrough";
}

export function passthroughNotice(argv: string): string {
  return `Passthrough — ${argv} is listed and was not run. The kernel call arrives with the chat orchestrator.`;
}
