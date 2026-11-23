//! Which tools each pass may use.
//!
//! This is the single source of truth for pass visibility. The dispatcher
//! enforces it before parsing arguments; the schema projection uses it too.
//! Prompt text is never the security boundary.

use serde::{Deserialize, Serialize};

/// The seventeen work tools, in design order.
pub const WORK_TOOLS: [&str; 17] = [
    "agent.run",
    "conversation.status",
    "conversation.frame",
    "conversation.topic",
    "conversation.depth",
    "data.create",
    "data.update",
    "data.query",
    "data.delete",
    "data.search",
    "data.schema",
    "data.namespace",
    "metamind.plan",
    "metamind.think",
    "metamind.memory",
    "metamind.log",
    "metamind.act",
];

/// The two control tools.
pub const CONTROL_TOOLS: [&str; 2] = ["metamind.verify", "metamind.approve"];

/// One turn pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassScope {
    /// Interpret: conversation state plus the two control tools.
    Interpret,
    /// Process: every work tool.
    Process,
    /// Verify: work tools except delegation/thinking, plus approve.
    Verify,
    /// Think: read-only data, activity, delegation, and control tools.
    Think,
}

impl PassScope {
    /// Stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interpret => "interpret",
            Self::Process => "process",
            Self::Verify => "verify",
            Self::Think => "think",
        }
    }

    /// Whether `name` may be called in this pass.
    ///
    /// When `action` is supplied, action-level restrictions apply (the thinker
    /// may only recall memory). Passing `None` asks the tool-level question used
    /// for schema projection.
    pub fn allows(self, name: &str, action: Option<&str>) -> bool {
        match self {
            Self::Interpret => matches!(
                name,
                "conversation.topic"
                    | "conversation.depth"
                    | "conversation.status"
                    | "conversation.frame"
                    | "metamind.verify"
                    | "metamind.approve"
            ),
            Self::Process => WORK_TOOLS.contains(&name),
            Self::Verify => {
                WORK_TOOLS.contains(&name) && name != "agent.run" && name != "metamind.think"
                    || name == "metamind.approve"
            }
            Self::Think => match name {
                "data.query" | "data.search" | "data.schema" | "metamind.log" | "agent.run"
                | "metamind.verify" | "metamind.approve" => true,
                "metamind.memory" => action.is_none_or(|action| action == "recall"),
                _ => false,
            },
        }
    }

    /// Tool names visible to this pass, for tests and schema projection.
    pub fn visible_tools(self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = WORK_TOOLS
            .into_iter()
            .filter(|name| self.allows(name, None))
            .collect();
        names.extend(
            CONTROL_TOOLS
                .into_iter()
                .filter(|name| self.allows(name, None)),
        );
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_counts_match_the_design() {
        assert_eq!(PassScope::Interpret.visible_tools().len(), 6);
        assert_eq!(PassScope::Process.visible_tools().len(), 17);
        assert_eq!(PassScope::Verify.visible_tools().len(), 16);
        assert_eq!(PassScope::Think.visible_tools().len(), 8);
    }

    #[test]
    fn delegation_and_thinking_are_confined() {
        assert!(!PassScope::Interpret.allows("agent.run", None));
        assert!(!PassScope::Interpret.allows("data.query", None));
        assert!(PassScope::Process.allows("agent.run", None));
        assert!(PassScope::Process.allows("metamind.think", None));
        assert!(!PassScope::Verify.allows("agent.run", None));
        assert!(!PassScope::Verify.allows("metamind.think", None));
        assert!(PassScope::Think.allows("agent.run", None));
    }

    #[test]
    fn control_tools_are_limited_to_the_right_passes() {
        assert!(PassScope::Interpret.allows("metamind.verify", None));
        assert!(PassScope::Interpret.allows("metamind.approve", None));
        assert!(!PassScope::Process.allows("metamind.verify", None));
        assert!(!PassScope::Process.allows("metamind.approve", None));
        assert!(!PassScope::Verify.allows("metamind.verify", None));
        assert!(PassScope::Verify.allows("metamind.approve", None));
        assert!(PassScope::Think.allows("metamind.verify", None));
    }

    #[test]
    fn the_thinker_may_only_recall_memory() {
        assert!(PassScope::Think.allows("metamind.memory", None));
        assert!(PassScope::Think.allows("metamind.memory", Some("recall")));
        assert!(!PassScope::Think.allows("metamind.memory", Some("remember")));
        assert!(!PassScope::Think.allows("metamind.memory", Some("forget")));
        assert!(!PassScope::Think.allows("data.create", None));
        assert!(!PassScope::Think.allows("conversation.status", None));
    }

    #[test]
    fn unknown_tools_are_never_allowed() {
        for scope in [
            PassScope::Interpret,
            PassScope::Process,
            PassScope::Verify,
            PassScope::Think,
        ] {
            assert!(!scope.allows("shell.exec", None));
        }
    }
}
