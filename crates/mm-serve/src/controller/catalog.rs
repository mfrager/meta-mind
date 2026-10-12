//! The tool catalog. Phase 2 wires reads. Every other name is listed so a call
//! is told it is not wired, rather than rejected as unknown.

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
        any_word(&lower, &["decide", "should", "firewall", "risk"]),
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
        false,
        Effect::Write,
        Confirm::IfMutating,
        FocusKind::Document,
        "",
    ),
    row(
        "doctor.run",
        GroupId::Kernel,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::State,
        "Read kernel health and the graph's quad count.",
    ),
    row(
        "logs.head",
        GroupId::Kernel,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::Document,
        "Read the event-log head sequence.",
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
        "Run a read-only SPARQL query.",
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
        "Read identity, invariants, affect, and goal counts.",
    ),
    row(
        "being.goal_add",
        GroupId::Being,
        false,
        Effect::Write,
        Confirm::IfMutating,
        FocusKind::None,
        "",
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
        false,
        Effect::Write,
        Confirm::IfMutating,
        FocusKind::None,
        "",
    ),
    row(
        "memory.recall",
        GroupId::Memory,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::Document,
        "Recall memories with the five score terms.",
    ),
    row(
        "memory.forget",
        GroupId::Memory,
        false,
        Effect::Write,
        Confirm::Always,
        FocusKind::None,
        "",
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
        false,
        Effect::Write,
        Confirm::IfMutating,
        FocusKind::None,
        "",
    ),
    row(
        "epistemic.show",
        GroupId::Epistemic,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "library.applicable",
        GroupId::Library,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "policy.history",
        GroupId::Library,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "skill.retrieve",
        GroupId::Library,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "case.retrieve",
        GroupId::Library,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "frame.missing",
        GroupId::Library,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "decide.ask",
        GroupId::Decide,
        false,
        Effect::Write,
        Confirm::Never,
        FocusKind::Document,
        "",
    ),
    row(
        "firewall.eval",
        GroupId::Decide,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "compare.check",
        GroupId::Decide,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "risk.analyze",
        GroupId::Decide,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "calibration.show",
        GroupId::Decide,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "conformance.run",
        GroupId::Decide,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "changeset.new",
        GroupId::Change,
        false,
        Effect::Write,
        Confirm::IfMutating,
        FocusKind::Document,
        "",
    ),
    row(
        "changeset.show",
        GroupId::Change,
        true,
        Effect::Read,
        Confirm::Never,
        FocusKind::Document,
        "List change sets.",
    ),
    row(
        "sandbox.run",
        GroupId::Change,
        false,
        Effect::Write,
        Confirm::Always,
        FocusKind::Document,
        "",
    ),
    row(
        "promote.run",
        GroupId::Change,
        false,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "",
    ),
    row(
        "reject.run",
        GroupId::Change,
        false,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "",
    ),
    row(
        "audit.production_tree",
        GroupId::Change,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
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
        false,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "",
    ),
    row(
        "loop.status",
        GroupId::Loop,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "self_model.report",
        GroupId::Loop,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "debt.scan",
        GroupId::Loop,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "gc.propose",
        GroupId::Loop,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "gc.apply",
        GroupId::Loop,
        false,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "",
    ),
    row(
        "module.load",
        GroupId::Loop,
        false,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "",
    ),
    row(
        "module.rollback",
        GroupId::Loop,
        false,
        Effect::Gate,
        Confirm::Always,
        FocusKind::None,
        "",
    ),
    row(
        "design.show",
        GroupId::Loop,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "sandbox_tool.list",
        GroupId::Sandbox,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "sandbox_tool.describe",
        GroupId::Sandbox,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "sandbox_tool.run",
        GroupId::Sandbox,
        false,
        Effect::External,
        Confirm::Always,
        FocusKind::None,
        "",
    ),
    row(
        "action.ledger",
        GroupId::Sandbox,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "action.rollback",
        GroupId::Sandbox,
        false,
        Effect::Write,
        Confirm::Always,
        FocusKind::None,
        "",
    ),
    row(
        "pi.run",
        GroupId::Author,
        false,
        Effect::External,
        Confirm::Always,
        FocusKind::None,
        "",
    ),
    row(
        "pi.ingest",
        GroupId::Author,
        false,
        Effect::Write,
        Confirm::Always,
        FocusKind::None,
        "",
    ),
    row(
        "codex.scan",
        GroupId::Later,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "codex.verify",
        GroupId::Later,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "experience.compile",
        GroupId::Later,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "llm.stats",
        GroupId::Later,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "meta.analyze",
        GroupId::Later,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "regression.run",
        GroupId::Later,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "verify.run",
        GroupId::Later,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "calibrate.score",
        GroupId::Later,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
    ),
    row(
        "mcp.list",
        GroupId::Later,
        false,
        Effect::Read,
        Confirm::Never,
        FocusKind::None,
        "",
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
