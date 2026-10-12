//! The 42 `mm-cli` subcommands, each bespoke or passthrough (plan `09` §5).

use serde::Serialize;

/// How the console covers one CLI subcommand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// A route or chat op with a kernel read, a form, or a specific reason.
    Bespoke,
    /// Registered on `/api/exec` with the generic renderer. Not a shell.
    Passthrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CommandSpec {
    pub name: &'static str,
    /// Transcript op name. Specialized cards key off this string.
    pub op: &'static str,
    pub tier: Tier,
}

/// Same order as `Command` in `crates/mm-cli/src/main.rs`.
pub const COMMANDS: &[CommandSpec] = &[
    spec("doctor", "doctor", Tier::Bespoke),
    spec("replay", "replay", Tier::Bespoke),
    spec("graph", "graph", Tier::Bespoke),
    spec("logs", "logs", Tier::Bespoke),
    spec("codex", "codex", Tier::Passthrough),
    spec("llm", "llm", Tier::Passthrough),
    spec("being", "being", Tier::Bespoke),
    spec("memory", "memory", Tier::Passthrough),
    spec("epistemic", "epistemic", Tier::Passthrough),
    spec("library", "library", Tier::Passthrough),
    spec("skill", "skill", Tier::Passthrough),
    spec("case", "case", Tier::Passthrough),
    spec("experience", "experience", Tier::Passthrough),
    spec("policy", "policy", Tier::Passthrough),
    spec("frame", "frame", Tier::Passthrough),
    spec("episode", "episode", Tier::Passthrough),
    spec("decide", "decide", Tier::Bespoke),
    spec("firewall", "firewall eval", Tier::Bespoke),
    spec("compare", "compare", Tier::Passthrough),
    spec("risk", "risk analyze", Tier::Bespoke),
    spec("calibration", "calibration", Tier::Passthrough),
    spec("conformance", "conformance", Tier::Passthrough),
    spec("tool", "tool", Tier::Passthrough),
    spec("action", "action", Tier::Passthrough),
    spec("verify", "verify run", Tier::Bespoke),
    spec("mcp", "mcp", Tier::Passthrough),
    spec("calibrate", "calibrate", Tier::Passthrough),
    spec("meta", "meta analyze", Tier::Bespoke),
    spec("regression", "regression", Tier::Passthrough),
    spec("changeset", "changeset show", Tier::Bespoke),
    spec("sandbox", "sandbox run", Tier::Bespoke),
    spec("promote", "promote", Tier::Bespoke),
    spec("reject", "reject", Tier::Bespoke),
    spec("budget", "budget show", Tier::Bespoke),
    spec("pi", "pi", Tier::Passthrough),
    spec("audit", "audit", Tier::Bespoke),
    spec("loop", "loop", Tier::Passthrough),
    spec("self-model", "self-model", Tier::Passthrough),
    spec("debt", "debt", Tier::Passthrough),
    spec("gc", "gc", Tier::Passthrough),
    spec("module", "module", Tier::Bespoke),
    spec("design", "design", Tier::Bespoke),
];

const fn spec(name: &'static str, op: &'static str, tier: Tier) -> CommandSpec {
    CommandSpec { name, op, tier }
}

pub fn command(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|command| command.name == name)
}

/// First token, or a fixed phrase the operator flows use.
pub fn classify(query: &str) -> &'static str {
    let trimmed = query.trim().trim_start_matches('/');
    let lower = trimmed.to_lowercase();
    if lower.contains("stage") && (lower.contains("slow") || lower.contains("why")) {
        return "meta analyze";
    }
    if lower.contains("production-tree") {
        return "audit";
    }
    let token = lower.split_whitespace().next().unwrap_or("");
    command(token).map(|spec| spec.op).unwrap_or("passthrough")
}
