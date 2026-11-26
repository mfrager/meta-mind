//! The `[STATE]` block.
//!
//! State is the conversation projection, not a store. This module is the only
//! renderer, so every pass sees the same canonical block.

use mm_agentstream::SessionState;
use serde_json::json;

/// The largest block the prompt tree accepts.
pub const MAX_STATE_BLOCK_BYTES: usize = 8 * 1024;

/// Render the authoritative state block.
pub fn render(user: &str, state: &SessionState) -> String {
    let as_of = mm_core::Timestamp::now().to_rfc3339();
    let body = json!({
        "revision": state.revision,
        "topic": state.topic,
        "depth": state.depth,
        "status": state.status,
        "frames": state.frames,
    });
    let block = format!(
        "[STATE user={user} rev={} as_of={as_of}]\n{}",
        state.revision,
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".into())
    );
    truncate_bytes(block)
}

/// An explicit unavailable block, used when state cannot be loaded.
pub fn unavailable(reason: &str) -> String {
    format!("[STATE unavailable: {reason}]")
}

fn truncate_bytes(text: String) -> String {
    if text.len() <= MAX_STATE_BLOCK_BYTES {
        return text;
    }
    let mut end = MAX_STATE_BLOCK_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_agentstream::{Frame, SessionState};

    #[test]
    fn the_block_names_the_revision_and_topic() {
        let state = SessionState {
            revision: 3,
            topic: Some("incident".into()),
            depth: 1,
            status: json!({"phase": "active"}),
            frames: vec![Frame {
                id: "f".into(),
                kind: "work".into(),
                step: "task".into(),
            }],
        };
        let block = render("operator", &state);
        assert!(block.starts_with("[STATE user=operator rev=3 as_of="));
        assert!(block.contains("\"topic\":\"incident\""));
        assert!(block.contains("\"step\":\"task\""));
    }

    #[test]
    fn the_block_is_bounded() {
        let state = SessionState {
            revision: 1,
            topic: Some("x".repeat(20_000)),
            ..SessionState::default()
        };
        assert!(render("u", &state).len() <= MAX_STATE_BLOCK_BYTES);
    }
}
