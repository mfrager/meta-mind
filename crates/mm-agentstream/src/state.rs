//! The conversation state projection.
//!
//! State is not stored in a database. It is folded from `session.started` and
//! `state.updated` events, and a turn stages a [`StatePatch`] that commits only
//! when Verify approves.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One frame in the conversation stack.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    /// Stable frame id.
    pub id: String,
    /// Frame kind (`work`, `intro`, …).
    pub kind: String,
    /// Current step in that frame.
    pub step: String,
}

/// The state the `[STATE]` block renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    /// Monotonic revision, starting at 0.
    pub revision: u64,
    /// Current topic.
    pub topic: Option<String>,
    /// Conversation depth.
    pub depth: u32,
    /// Free-form typed status owned by the conversation.
    pub status: Value,
    /// Frame stack, oldest first.
    pub frames: Vec<Frame>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            revision: 0,
            topic: None,
            depth: 0,
            status: json!({}),
            frames: Vec::new(),
        }
    }
}

impl SessionState {
    /// The state after a session starts.
    pub fn started() -> Self {
        Self::default()
    }
}

/// One frame-stack command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "action")]
pub enum FrameCommand {
    /// Push a new frame.
    Push(Frame),
    /// Pop the top frame.
    Pop,
    /// Replace the top frame.
    Replace(Frame),
}

/// A staged state change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatePatch {
    /// Set (or clear) the topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<Option<String>>,
    /// Set the depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    /// Replace the status object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Value>,
    /// Change the frame stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<FrameCommand>,
}

impl StatePatch {
    /// Whether the patch changes nothing.
    pub fn is_empty(&self) -> bool {
        self.topic.is_none()
            && self.depth.is_none()
            && self.status.is_none()
            && self.frame.is_none()
    }

    /// Merge a later patch over this one.
    pub fn merge(&mut self, later: StatePatch) {
        if later.topic.is_some() {
            self.topic = later.topic;
        }
        if later.depth.is_some() {
            self.depth = later.depth;
        }
        if later.status.is_some() {
            self.status = later.status;
        }
        if later.frame.is_some() {
            match (&mut self.frame, later.frame) {
                (None, command) => self.frame = command,
                (Some(existing), Some(command)) => {
                    // Only one frame command per turn is meaningful; the later
                    // command wins so a retry can correct an earlier attempt.
                    *existing = command;
                }
                (Some(_), None) => {}
            }
        }
    }

    /// Apply the patch. Returns true when the state changed.
    pub fn apply(&self, state: &mut SessionState) -> bool {
        let mut next = state.clone();
        if let Some(topic) = &self.topic {
            next.topic = topic.clone();
        }
        if let Some(depth) = self.depth {
            next.depth = depth;
        }
        if let Some(status) = &self.status {
            next.status = status.clone();
        }
        if let Some(command) = &self.frame {
            match command {
                FrameCommand::Push(frame) => next.frames.push(frame.clone()),
                FrameCommand::Pop => {
                    next.frames.pop();
                }
                FrameCommand::Replace(frame) => {
                    if let Some(top) = next.frames.last_mut() {
                        *top = frame.clone();
                    } else {
                        next.frames.push(frame.clone());
                    }
                }
            }
        }
        if next == *state {
            return false;
        }
        next.revision = state.revision.saturating_add(1);
        *state = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_commits_as_one_revision() {
        let mut state = SessionState::default();
        let patch = StatePatch {
            topic: Some(Some("incident".into())),
            depth: Some(2),
            ..StatePatch::default()
        };
        assert!(patch.apply(&mut state));
        assert_eq!(state.revision, 1);
        assert_eq!(state.topic.as_deref(), Some("incident"));
        assert_eq!(state.depth, 2);
        assert!(
            !patch.apply(&mut state),
            "a no-op must not bump the revision"
        );
    }

    #[test]
    fn frame_commands_change_the_top_frame() {
        let mut state = SessionState::default();
        StatePatch {
            frame: Some(FrameCommand::Push(Frame {
                id: "f1".into(),
                kind: "work".into(),
                step: "task".into(),
            })),
            ..StatePatch::default()
        }
        .apply(&mut state);
        assert_eq!(state.frames.len(), 1);
        StatePatch {
            frame: Some(FrameCommand::Replace(Frame {
                id: "f1".into(),
                kind: "work".into(),
                step: "verify".into(),
            })),
            ..StatePatch::default()
        }
        .apply(&mut state);
        assert_eq!(state.frames[0].step, "verify");
        StatePatch {
            frame: Some(FrameCommand::Pop),
            ..StatePatch::default()
        }
        .apply(&mut state);
        assert!(state.frames.is_empty());
    }
}
