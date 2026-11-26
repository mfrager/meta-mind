//! The tool catalog. Every CLI capability is a tool, including the ones a
//! particular call still refuses.

use std::sync::OnceLock;

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupId {
    Core,
    Kernel,
    Being,
    Memory,
    Epistemic,
    Library,
    Decide,
    Change,
    Loop,
    Sandbox,
    Author,
    Later,
}

impl GroupId {
    pub fn as_str(self) -> &'static str {
        match self {
            GroupId::Core => "core",
            GroupId::Kernel => "kernel",
            GroupId::Being => "being",
            GroupId::Memory => "memory",
            GroupId::Epistemic => "epistemic",
            GroupId::Library => "library",
            GroupId::Decide => "decide",
            GroupId::Change => "change",
            GroupId::Loop => "loop",
            GroupId::Sandbox => "sandbox",
            GroupId::Author => "author",
            GroupId::Later => "later",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "core" => GroupId::Core,
            "kernel" => GroupId::Kernel,
            "being" => GroupId::Being,
            "memory" => GroupId::Memory,
            "epistemic" => GroupId::Epistemic,
            "library" => GroupId::Library,
            "decide" => GroupId::Decide,
            "change" => GroupId::Change,
            "loop" => GroupId::Loop,
            "sandbox" => GroupId::Sandbox,
            "author" => GroupId::Author,
            "later" => GroupId::Later,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Read,
    Write,
    Gate,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    Never,
    IfMutating,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusKind {
    None,
    State,
    Document,
}

pub struct ToolSpec {
    pub name: &'static str,
    pub summary: String,
    pub group: GroupId,
    /// Read, write, gate, or external. Admission in a later phase uses this.
    #[allow(dead_code)]
    pub effect: Effect,
    /// When a later phase must ask before running.
    #[allow(dead_code)]
    pub confirm: Confirm,
    /// Where a successful call points the pane.
    #[allow(dead_code)]
    pub focus: FocusKind,
    pub input_schema: Value,
    pub available: bool,
}

pub fn table() -> &'static [ToolSpec] {
    SPECS.get_or_init(build).as_slice()
}

pub fn get(name: &str) -> Option<&'static ToolSpec> {
    table().iter().find(|spec| spec.name == name)
}

/// Groups attached at the start of a turn. Core is always first.
pub fn groups_for(text: &str) -> Vec<GroupId> {
    let lower = text.to_ascii_lowercase();
    let mut groups = vec![GroupId::Core];
    let mut add = |id: GroupId, hit: bool| {
        if hit && !groups.contains(&id) {
            groups.push(id);
        }
    };
    add(
        GroupId::Kernel,
        any_word(&lower, &["doctor", "health", "log", "replay", "graph"]),
    );
    add(
        GroupId::Being,
        any_word(&lower, &["being", "goal"]) || lower.contains("who are you"),
    );
    add(
        GroupId::Memory,
        any_word(&lower, &["remember", "recall", "forget"]),
    );
    add(
        GroupId::Epistemic,
        any_word(&lower, &["claim", "evidence", "assumption"]),
    );
    add(
        GroupId::Library,
        any_word(&lower, &["library", "policy", "skill", "case", "frame"]),
    );
    add(
        GroupId::Decide,
        any_word(
            &lower,
            &[
                "decide",
                "should",
                "firewall",
                "risk",
                "calibration",
                "conformance",
                "compare",
            ],
        ),
    );
    add(
        GroupId::Change,
        lower.contains("change set") || any_word(&lower, &["sandbox", "promote", "reject"]),
    );
    add(
        GroupId::Loop,
        any_word(&lower, &["loop", "debt", "gc", "module"]) || lower.contains("self-model"),
    );
    add(
        GroupId::Sandbox,
        any_word(&lower, &["tool", "file", "run", "rollback"]),
    );
    add(
        GroupId::Author,
        any_word(&lower, &["pi"]) || lower.contains("write the module"),
    );
    add(
        GroupId::Later,
        any_word(
            &lower,
            &[
                "codex",
                "experience",
                "llm",
                "meta",
                "regression",
                "verify",
                "calibrate",
                "mcp",
            ],
        ),
    );
    groups
}

fn any_word(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| has_word(text, word))
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .any(|token| token == word)
}

fn build() -> Vec<ToolSpec> {
    ROWS.iter()
        .map(|row| {
            let summary = if row.available {
                row.summary.to_string()
            } else {
                format!("{} is not wired on this build", row.name)
            };
            ToolSpec {
                name: row.name,
                summary,
                group: row.group,
                input_schema: schema_for(row.name),
                effect: row.effect,
                confirm: row.confirm,
                focus: row.focus,
                available: row.available,
            }
        })
        .collect()
}

struct Row {
    name: &'static str,
    group: GroupId,
    available: bool,
    effect: Effect,
    confirm: Confirm,
    focus: FocusKind,
    summary: &'static str,
}

const ROWS: &[Row] = &[
    row(
        "guide.list",
        GroupId::Core,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "List the guide chapters.",
    ),
    row(
        "guide.read",
        GroupId::Core,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Read one guide chapter by id.",
    ),
    row(
        "guide.search",
        GroupId::Core,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Search the guide by token overlap.",
    ),
    row(
        "catalog.open",
        GroupId::Core,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Open one tool group for the rest of this turn.",
    ),
    row(
        "state.show",
        GroupId::Core,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::State,
        "Read health, the being, budgets, and goals.",
    ),
    row(
        "focus.open",
        GroupId::Core,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::Document,
        "Open a document the console already serves.",
    ),
    row(
        "focus.read",
        GroupId::Core,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Read the body of a document the console serves.",
    ),
    row(
        "session.note",
        GroupId::Core,
        true,
        Effect::Write,
        Confirm::Never,
        FocusKind::None,
        "Keep a note on this chat only. It is not autobiographical memory.",
    ),
    row(
        "focus.patch",
        GroupId::Core,
        true,
        Effect::Write,
        Confirm::IfMutating,
        FocusKind::Document,
        "Patch a live goal description or a memory through the tool that owns it.",
    ),
    row(
        "doctor.run",
        GroupId::Kernel,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::State,
        "Read kernel health. The doctor line names this process when the graph is open.",
    ),
    row(
        "logs.head",
        GroupId::Kernel,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::Document,
        "Read the logs head sequence.",
    ),
    row(
        "logs.verify",
        GroupId::Kernel,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Check the audit chain.",
    ),
    row(
        "replay.run",
        GroupId::Kernel,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Replay committed events and return the state hash.",
    ),
    row(
        "graph.query",
        GroupId::Kernel,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Run a read-only SPARQL query on the graph.",
    ),
    row(
        "graph.validate",
        GroupId::Kernel,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Validate one named graph against its shapes.",
    ),
    row(
        "being.show",
        GroupId::Being,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::State,
        "Read the being: identity, invariants, affect, and goal counts.",
    ),
    row(
        "being.goal_add",
        GroupId::Being,
        true,
        Effect::Write,
        Confirm::IfMutating,
        FocusKind::Document,
        "Add a goal. Description and priority are required. Origin is the operator.",
    ),
    row(
        "being.goal_list",
        GroupId::Being,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::Document,
        "List goals.",
    ),
    row(
        "being.verify",
        GroupId::Being,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Run the invariant corpus.",
    ),
    row(
        "being.personality_show",
        GroupId::Being,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Read values, dispositions, and the projected chat voice.",
    ),
    row(
        "memory.add",
        GroupId::Memory,
        true,
        Effect::Write,
        Confirm::IfMutating,
        FocusKind::None,
        "Store a semantic or episodic memory in the operator's words.",
    ),
    row(
        "memory.recall",
        GroupId::Memory,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::Document,
        "Recall memory with the five score terms.",
    ),
    row(
        "memory.forget",
        GroupId::Memory,
        true,
        Effect::Write,
        Confirm::Always,
        FocusKind::None,
        "Forget memories. Dry-run unless apply is true.",
    ),
    row(
        "memory.verify",
        GroupId::Memory,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Reconcile SQLite with the memory graph.",
    ),
    row(
        "epistemic.ingest",
        GroupId::Epistemic,
        true,
        Effect::Write,
        Confirm::IfMutating,
        FocusKind::None,
        "Ingest one epistemic claim.",
    ),
    row(
        "epistemic.show",
        GroupId::Epistemic,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Show epistemic claim and assumption counts.",
    ),
    row(
        "library.applicable",
        GroupId::Library,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Rank library entries that apply to a goal.",
    ),
    row(
        "policy.history",
        GroupId::Library,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "List the versions of one policy.",
    ),
    row(
        "skill.retrieve",
        GroupId::Library,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Retrieve verified skills for a query.",
    ),
    row(
        "case.retrieve",
        GroupId::Library,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Retrieve cases whose title matches a query.",
    ),
    row(
        "frame.missing",
        GroupId::Library,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "List the slots a frame instance left unknown.",
    ),
    row(
        "decide.ask",
        GroupId::Decide,
        true,
        Effect::Write,
        Confirm::Never,
        FocusKind::Document,
        "Ask a decision core (decide). rules runs now. hosted and local wait for Approve.",
    ),
    row(
        "firewall.eval",
        GroupId::Decide,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Evaluate one episode or action (firewall).",
    ),
    row(
        "compare.check",
        GroupId::Decide,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Compare a baseline string with a candidate string.",
    ),
    row(
        "risk.analyze",
        GroupId::Decide,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Score a list of loss outcomes (risk).",
    ),
    row(
        "calibration.show",
        GroupId::Decide,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Read the calibration rows.",
    ),
    row(
        "conformance.run",
        GroupId::Decide,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Run the rules core conformance checks.",
    ),
    row(
        "changeset.new",
        GroupId::Change,
        true,
        Effect::Write,
        Confirm::IfMutating,
        FocusKind::Document,
        "Open a change set from a gap file or a title and hypothesis.",
    ),
    row(
        "changeset.show",
        GroupId::Change,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::Document,
        "List change sets (changeset).",
    ),
    row(
        "sandbox.run",
        GroupId::Change,
        true,
        Effect::Write,
        Confirm::Always,
        FocusKind::Document,
        "Prepare the change set sandbox worktree.",
    ),
    row(
        "promote.run",
        GroupId::Change,
        true,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "Promote a change set through the same gate as mm-cli promote.",
    ),
    row(
        "reject.run",
        GroupId::Change,
        true,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "Reject a change set. The reason must be at least 8 characters.",
    ),
    row(
        "audit.production_tree",
        GroupId::Change,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Audit promotions and list production writes outside a promotion.",
    ),
    row(
        "budget.show",
        GroupId::Change,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::Document,
        "Read the four budget rows.",
    ),
    row(
        "loop.run",
        GroupId::Loop,
        true,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "Run the improvement loop for a bench goal file or a stored goal.",
    ),
    row(
        "loop.status",
        GroupId::Loop,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Read loop run rows.",
    ),
    row(
        "self_model.report",
        GroupId::Loop,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Measure one loop run (self-model).",
    ),
    row(
        "debt.scan",
        GroupId::Loop,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Scan for architectural debt.",
    ),
    row(
        "gc.propose",
        GroupId::Loop,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Propose gc actions without applying them.",
    ),
    row(
        "gc.apply",
        GroupId::Loop,
        true,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "Apply the gc actions the collector proposed.",
    ),
    row(
        "module.load",
        GroupId::Loop,
        true,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "Load a materialised module through a change set. Without one, the answer is use the change-set flow.",
    ),
    row(
        "module.rollback",
        GroupId::Loop,
        true,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "Restore the module version that was active before the newest load.",
    ),
    row(
        "design.show",
        GroupId::Loop,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "List design revisions of a document.",
    ),
    row(
        "sandbox_tool.list",
        GroupId::Sandbox,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "List the in-process sandbox tools (tool).",
    ),
    row(
        "sandbox_tool.describe",
        GroupId::Sandbox,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Describe one sandbox tool.",
    ),
    row(
        "sandbox_tool.run",
        GroupId::Sandbox,
        true,
        Effect::External,
        Confirm::Always,
        FocusKind::None,
        "Call one sandbox tool as the operator.",
    ),
    row(
        "action.ledger",
        GroupId::Sandbox,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Read the action ledger.",
    ),
    row(
        "action.rollback",
        GroupId::Sandbox,
        true,
        Effect::Write,
        Confirm::Always,
        FocusKind::None,
        "Restore the snapshot an action took.",
    ),
    row(
        "pi.run",
        GroupId::Author,
        true,
        Effect::External,
        Confirm::Always,
        FocusKind::None,
        "Run Pi when the pi binary is on PATH. Absence is the result.",
    ),
    row(
        "pi.ingest",
        GroupId::Author,
        true,
        Effect::Write,
        Confirm::Always,
        FocusKind::None,
        "Ingest a recorded Pi session.",
    ),
    row(
        "codex.scan",
        GroupId::Later,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Scan the workspace into a codex report.",
    ),
    row(
        "codex.verify",
        GroupId::Later,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Verify the codex report.",
    ),
    row(
        "experience.compile",
        GroupId::Later,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Compile trajectories into draft experience candidates.",
    ),
    row(
        "llm.stats",
        GroupId::Later,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Count llm calls recorded by this process.",
    ),
    row(
        "meta.analyze",
        GroupId::Later,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Analyze one meta episode file.",
    ),
    row(
        "regression.run",
        GroupId::Later,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Run the regression suite.",
    ),
    row(
        "verify.run",
        GroupId::Later,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Name a verify obligation. This route does not spawn a compiler.",
    ),
    row(
        "calibrate.score",
        GroupId::Later,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "Read calibrate scores from the calibration rows.",
    ),
    row(
        "mcp.list",
        GroupId::Later,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "List in-process tools the way mcp tools/list does.",
    ),
];

const fn row(
    name: &'static str,
    group: GroupId,
    available: bool,
    effect: Effect,
    confirm: Confirm,
    focus: FocusKind,
    summary: &'static str,
) -> Row {
    Row {
        name,
        group,
        available,
        effect,
        confirm,
        focus,
        summary,
    }
}

fn schema_for(name: &str) -> Value {
    match name {
        "guide.read" | "focus.open" | "focus.read" => {
            object(&[("id", json!({"type": "string"}))], &["id"])
        }
        "guide.search" | "memory.recall" => {
            object(&[("query", json!({"type": "string"}))], &["query"])
        }
        "catalog.open" => object(&[("group", json!({"type": "string"}))], &["group"]),
        "session.note" => object(
            &[("text", json!({"type": "string", "minLength": 1}))],
            &["text"],
        ),
        "state.show" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "sections": { "type": "array", "items": { "type": "string" } } },
            "required": []
        }),
        "graph.query" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "query": { "type": "string", "minLength": 1 },
                "graph": { "type": "string" }
            },
            "required": ["query"]
        }),
        "graph.validate" => object(
            &[("graph", json!({"type": "string", "minLength": 1}))],
            &["graph"],
        ),
        "being.goal_add" => object(
            &[
                ("description", json!({"type": "string", "minLength": 1})),
                ("priority", json!({"type": "number"})),
            ],
            &["description", "priority"],
        ),
        "memory.add" => object(
            &[
                (
                    "kind",
                    json!({"type": "string", "enum": ["semantic", "episodic"]}),
                ),
                ("content", json!({"type": "string", "minLength": 1})),
            ],
            &["kind", "content"],
        ),
        "memory.forget" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "apply": { "type": "boolean" } },
            "required": []
        }),
        "decide.ask" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "kind": { "type": "string" },
                "prompt": { "type": "string", "minLength": 1 },
                "core": { "type": "string" },
                "options": { "type": "array", "items": { "type": "string" } },
                "no_record": { "type": "boolean" }
            },
            "required": ["prompt"]
        }),
        "firewall.eval" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "tool": { "type": "string" } },
            "required": []
        }),
        "compare.check" => object(
            &[
                ("baseline", json!({"type": "string"})),
                ("candidate", json!({"type": "string"})),
            ],
            &["baseline", "candidate"],
        ),
        "risk.analyze" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "outcomes": { "type": "array" } },
            "required": ["outcomes"]
        }),
        "changeset.new" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "gap": { "type": "string" },
                "title": { "type": "string" },
                "hypothesis": { "type": "string" }
            },
            "required": []
        }),
        "sandbox.run" | "promote.run" | "action.rollback" => object(
            &[("id", json!({"type": "string", "minLength": 1}))],
            &["id"],
        ),
        "reject.run" => object(
            &[
                ("id", json!({"type": "string", "minLength": 1})),
                ("reason", json!({"type": "string", "minLength": 8})),
            ],
            &["id", "reason"],
        ),
        "sandbox_tool.describe" => object(
            &[("name", json!({"type": "string", "minLength": 1}))],
            &["name"],
        ),
        "sandbox_tool.run" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string", "minLength": 1 },
                "arguments": { "type": "object" }
            },
            "required": ["name"]
        }),
        "action.ledger" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "id": { "type": "string" } },
            "required": []
        }),
        "focus.patch" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "id": { "type": "string", "minLength": 1 },
                "patch": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "op": { "type": "string" },
                        "field": { "type": "string" },
                        "value": { "type": "string" },
                        "priority": { "type": "number" },
                        "kind": { "type": "string" }
                    }
                }
            },
            "required": ["id", "patch"]
        }),
        "loop.run" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "goal": { "type": "string" },
                "path": { "type": "string" }
            },
            "required": []
        }),
        "epistemic.ingest" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "kind": { "type": "string" },
                "subject": { "type": "string" },
                "predicate": { "type": "string" },
                "object": { "type": "string" }
            },
            "required": ["subject", "predicate", "object"]
        }),
        "library.applicable" => object(&[("goal", json!({"type": "string"}))], &[]),
        "policy.history" => object(
            &[("policy", json!({"type": "string", "minLength": 1}))],
            &["policy"],
        ),
        "skill.retrieve" | "case.retrieve" => object(&[("query", json!({"type": "string"}))], &[]),
        "frame.missing" | "self_model.report" => object(
            &[("id", json!({"type": "string", "minLength": 1}))],
            &["id"],
        ),
        "loop.status" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "id": { "type": "string" } },
            "required": []
        }),
        "module.load" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "uri": { "type": "string" },
                "change_set": { "type": "string" }
            },
            "required": []
        }),
        "module.rollback" => object(
            &[("uri", json!({"type": "string", "minLength": 1}))],
            &["uri"],
        ),
        "design.show" => object(
            &[("doc", json!({"type": "string", "minLength": 1}))],
            &["doc"],
        ),
        "pi.ingest" | "experience.compile" => object(
            &[("file", json!({"type": "string", "minLength": 1}))],
            &["file"],
        ),
        "meta.analyze" => object(
            &[("episode", json!({"type": "string", "minLength": 1}))],
            &["episode"],
        ),
        "verify.run" => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "kind": { "type": "string" } },
            "required": []
        }),
        _ => empty_object(),
    }
}

fn empty_object() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {},
        "required": []
    })
}

fn object(properties: &[(&str, Value)], required: &[&str]) -> Value {
    let mut map = serde_json::Map::new();
    for (name, schema) in properties {
        map.insert((*name).to_string(), schema.clone());
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": Value::Object(map),
        "required": required,
    })
}

static SPECS: OnceLock<Vec<ToolSpec>> = OnceLock::new();

#[cfg(test)]
mod tests {
    #[test]
    fn every_cli_command_is_named_by_a_tool() {
        for command in crate::parity::COMMANDS {
            let needle = command.name.to_ascii_lowercase();
            let hit = super::table()
                .iter()
                .any(|spec| spec.summary.to_ascii_lowercase().contains(&needle));
            assert!(hit, "no tool summary mentions {}", command.name);
        }
    }
}
