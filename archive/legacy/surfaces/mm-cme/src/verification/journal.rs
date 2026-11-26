//! Staged writes for the Verify pass.
//!
//! Verify may inspect and propose durable changes, but it may not perform them.
//! The dispatcher validates a mutating call and records a [`StagedMutation`];
//! only an `approve(true)` replays the journal through the real handlers.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Maximum staged mutations in one Verify pass.
pub const MAX_STAGED_MUTATIONS: usize = 32;

/// How a tool call interacts with durable state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    /// Execute the call now.
    Immediate,
    /// Validate the call and append it to the mutation journal.
    Staged,
}

/// One validated write intent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StagedMutation {
    /// One-based order within the journal.
    pub seq: u32,
    /// Tool name.
    pub tool: String,
    /// Action, when the tool is action-shaped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Validated arguments, exactly as the commit will replay them.
    pub arguments: Value,
    /// Revision and identity guards observed at staging time.
    #[serde(default)]
    pub preconditions: Value,
    /// Compact description shown to the reviewer.
    pub preview: Value,
}

impl StagedMutation {
    /// A tool result telling the model the write did not land.
    pub fn staged_result(&self) -> Value {
        json!({
            "staged": true,
            "seq": self.seq,
            "tool": self.tool,
            "action": self.action,
            "message": "validated but not committed; approval commits the journal",
            "intent": self.preview,
        })
    }
}

/// The ordered intents of one Verify pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MutationJournal {
    /// Session the writes belong to.
    pub session_id: String,
    /// Intents in call order.
    pub intents: Vec<StagedMutation>,
}

impl MutationJournal {
    /// An empty journal for a session.
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            intents: Vec::new(),
        }
    }

    /// Whether no write was staged.
    pub fn is_empty(&self) -> bool {
        self.intents.is_empty()
    }

    /// Number of staged writes.
    pub fn len(&self) -> usize {
        self.intents.len()
    }

    /// Append a validated intent. The caller is responsible for the bound.
    pub fn push(
        &mut self,
        tool: impl Into<String>,
        action: Option<String>,
        arguments: Value,
        preconditions: Value,
        preview: Value,
    ) -> Result<StagedMutation, String> {
        if self.intents.len() >= MAX_STAGED_MUTATIONS {
            return Err(format!(
                "the Verify journal holds at most {MAX_STAGED_MUTATIONS} writes"
            ));
        }
        let intent = StagedMutation {
            seq: self.intents.len() as u32 + 1,
            tool: tool.into(),
            action,
            arguments,
            preconditions,
            preview,
        };
        self.intents.push(intent.clone());
        Ok(intent)
    }

    /// The compact intents shown in the review prompt.
    pub fn prompt_view(&self) -> Value {
        Value::Array(
            self.intents
                .iter()
                .map(|intent| {
                    json!({
                        "seq": intent.seq,
                        "tool": intent.tool,
                        "action": intent.action,
                        "preview": intent.preview,
                    })
                })
                .collect(),
        )
    }
}

/// A committed intent and the handler's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommittedMutation {
    /// Journal sequence.
    pub seq: u32,
    /// Tool name.
    pub tool: String,
    /// Action, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Handler result.
    pub result: Value,
    /// Whether the commit retried after a revision conflict.
    #[serde(default)]
    pub retried: bool,
}

/// An observational record of a durable Process effect.
///
/// Process is not transactional. This log lets a retry reconcile with what the
/// previous attempt already changed instead of duplicating it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessMutation {
    /// Tool name.
    pub tool: String,
    /// Action, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Stable digest of the arguments.
    pub arguments_digest: String,
    /// The affected entity/plan/memory id when the result names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Resulting revision when the result names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// Short result summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Whether a call mutates durable state and therefore must be staged in Verify.
pub fn is_mutating_call(name: &str, action: Option<&str>) -> bool {
    match name {
        "data.create" | "data.update" | "data.delete" => true,
        "data.namespace" => !matches!(action, Some("list" | "expand" | "resolve")),
        "metamind.memory" => matches!(action, Some("remember" | "forget")),
        "metamind.plan" => matches!(action, Some("create" | "update")),
        "metamind.act" => !matches!(
            action,
            Some(
                "plan_start"
                    | "plan_stop"
                    | "plan_pause"
                    | "plan_resume"
                    | "plan_retry_step"
                    | "plan_advance_step"
                    | "plan_complete"
                    | "plan_abandon"
                    | "cancel"
            )
        ),
        "conversation.topic"
        | "conversation.depth"
        | "conversation.status"
        | "conversation.frame" => !matches!(action, Some("get")),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_sequences_and_bounds_intents() {
        let mut journal = MutationJournal::new("s1");
        let first = journal
            .push(
                "data.create",
                None,
                json!({"type": "mm:Project"}),
                json!({"dataRevision": 4}),
                json!({"type": "mm:Project"}),
            )
            .unwrap();
        assert_eq!(first.seq, 1);
        assert!(journal.intents[0].staged_result().get("staged") == Some(&json!(true)));
        for _ in 0..MAX_STAGED_MUTATIONS {
            if journal.len() == MAX_STAGED_MUTATIONS {
                break;
            }
            journal
                .push("data.update", None, json!({}), json!({}), json!({}))
                .unwrap();
        }
        assert!(journal
            .push("data.delete", None, json!({}), json!({}), json!({}))
            .is_err());
    }

    #[test]
    fn mutating_actions_are_classified() {
        assert!(is_mutating_call("data.create", None));
        assert!(!is_mutating_call("data.query", None));
        assert!(!is_mutating_call("data.namespace", Some("list")));
        assert!(is_mutating_call("data.namespace", Some("bind")));
        assert!(is_mutating_call("metamind.memory", Some("remember")));
        assert!(!is_mutating_call("metamind.memory", Some("recall")));
        assert!(is_mutating_call("metamind.plan", Some("create")));
        assert!(!is_mutating_call("conversation.topic", Some("get")));
        assert!(is_mutating_call("conversation.topic", Some("set")));
    }
}
