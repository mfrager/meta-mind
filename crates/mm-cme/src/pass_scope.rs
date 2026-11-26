//! Which tools each pass may use.
//!
//! Six tools exist in the retained core. This table is the single source of
//! truth for visibility: prompt text is never the boundary.

use serde::{Deserialize, Serialize};

/// The four conversation-state tools.
pub const CONVERSATION_TOOLS: [&str; 4] = [
    "conversation.topic",
    "conversation.depth",
    "conversation.status",
    "conversation.frame",
];

/// The two control tools.
pub const CONTROL_TOOLS: [&str; 2] = ["metamind.verify", "metamind.approve"];

/// Every tool in the retained core.
pub const ALL_TOOLS: [&str; 6] = [
    "conversation.topic",
    "conversation.depth",
    "conversation.status",
    "conversation.frame",
    "metamind.verify",
    "metamind.approve",
];

/// One turn pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassScope {
    /// Interpret: register checks, approve a fast path, or hand off to Process.
    Interpret,
    /// Process: conversation-state updates only.
    Process,
    /// Verify: conversation-state updates plus approval.
    Verify,
}

impl PassScope {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            PassScope::Interpret => "interpret",
            PassScope::Process => "process",
            PassScope::Verify => "verify",
        }
    }

    /// Whether `name` may be called in this pass.
    pub fn allows(self, name: &str) -> bool {
        match self {
            PassScope::Interpret => CONTROL_TOOLS.contains(&name),
            PassScope::Process => CONVERSATION_TOOLS.contains(&name),
            PassScope::Verify => CONVERSATION_TOOLS.contains(&name) || name == "metamind.approve",
        }
    }

    /// Tool names visible to this pass, in catalog order.
    pub fn visible_tools(self) -> Vec<&'static str> {
        ALL_TOOLS
            .into_iter()
            .filter(|name| self.allows(name))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_is_exactly_the_design() {
        assert_eq!(
            PassScope::Interpret.visible_tools(),
            vec!["metamind.verify", "metamind.approve"]
        );
        assert_eq!(
            PassScope::Process.visible_tools(),
            vec![
                "conversation.topic",
                "conversation.depth",
                "conversation.status",
                "conversation.frame"
            ]
        );
        assert_eq!(
            PassScope::Verify.visible_tools(),
            vec![
                "conversation.topic",
                "conversation.depth",
                "conversation.status",
                "conversation.frame",
                "metamind.approve"
            ]
        );
    }

    #[test]
    fn removed_tools_are_never_visible() {
        for scope in [PassScope::Interpret, PassScope::Process, PassScope::Verify] {
            for removed in [
                "agent.run",
                "data.create",
                "data.query",
                "metamind.plan",
                "metamind.think",
                "metamind.memory",
                "metamind.act",
            ] {
                assert!(!scope.allows(removed), "{scope:?} allowed {removed}");
            }
        }
    }
}
