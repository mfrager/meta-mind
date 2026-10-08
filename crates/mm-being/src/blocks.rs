//! Core blocks: the small, always-in-context state (Letta's lesson).
//!
//! A being that keeps its whole self in the prompt has no self at all — it has a
//! transcript. So the identity core and the user model are a handful of *labeled,
//! size-limited* blocks: `self`, `human`, `task`. The limit is the mechanism, not
//! a nicety: an unbounded block is how a prompt silently grows past the window it
//! was designed for, and a block that no longer fits is a block that has quietly
//! stopped being read.
//!
//! Editing a block goes through the façade's guard, which is why `set_content`
//! returns a `Result` rather than panicking: a refusal is a refusal, not a crash.

use mm_core::Ulid;
use serde::{Deserialize, Serialize};

use crate::error::{BeingError, Result};

/// Which part of the always-in-context state a block holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    /// Who the being is: its stable self-description.
    Self_,
    /// Who it is working with: the user model's prose half.
    Human,
    /// What is being worked on right now.
    Task,
}

impl BlockKind {
    /// The wire name, matching the migration's `CHECK (kind IN …)`.
    pub fn as_str(&self) -> &'static str {
        match self {
            BlockKind::Self_ => "self",
            BlockKind::Human => "human",
            BlockKind::Task => "task",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "self" => Some(BlockKind::Self_),
            "human" => Some(BlockKind::Human),
            "task" => Some(BlockKind::Task),
            _ => None,
        }
    }
}

impl std::fmt::Display for BlockKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The default limit for a block with no explicit one.
pub const DEFAULT_LIMIT_CHARS: u32 = 2000;

/// The default character limit for one kind of block.
///
/// The self-description gets the most room because it is the one that must stay
/// stable across model swaps; the task block gets the least because it is the one
/// most likely to be regenerated per episode.
pub fn default_limit(kind: BlockKind) -> u32 {
    match kind {
        BlockKind::Self_ => DEFAULT_LIMIT_CHARS,
        BlockKind::Human => 1500,
        BlockKind::Task => 1200,
    }
}

/// A labeled, size-limited piece of always-in-context state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreBlock {
    /// Which part of the state this is.
    pub kind: BlockKind,
    /// The block's label, e.g. `self` or `human:primary`.
    pub label: String,
    /// The text itself.
    pub content: String,
    /// The hard character limit.
    pub limit_chars: u32,
    /// The ULID of the edit that last wrote this block.
    pub updated_ulid: Ulid,
}

impl CoreBlock {
    /// A new block, refused if the content already exceeds its limit.
    pub fn new(
        kind: BlockKind,
        label: impl Into<String>,
        content: impl Into<String>,
        limit_chars: u32,
        updated_ulid: Ulid,
    ) -> Result<Self> {
        let content = content.into();
        let actual = content.chars().count();
        if actual > limit_chars as usize {
            return Err(BeingError::BlockTooLarge {
                kind: kind.as_str().to_string(),
                limit: limit_chars as usize,
                actual,
            });
        }
        Ok(CoreBlock {
            kind,
            label: label.into(),
            content,
            limit_chars,
            updated_ulid,
        })
    }

    /// Replace the content, refusing anything over the limit and leaving the old
    /// content in place when it is refused.
    pub fn set_content(&mut self, content: impl Into<String>, updated_ulid: Ulid) -> Result<()> {
        let content = content.into();
        let actual = content.chars().count();
        if actual > self.limit_chars as usize {
            return Err(BeingError::BlockTooLarge {
                kind: self.kind.as_str().to_string(),
                limit: self.limit_chars as usize,
                actual,
            });
        }
        self.content = content;
        self.updated_ulid = updated_ulid;
        Ok(())
    }

    /// The content length, in characters rather than bytes: a limit that counted
    /// bytes would punish every non-Latin script.
    pub fn len(&self) -> usize {
        self.content.chars().count()
    }

    /// True when the block is empty.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ulid(seed: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, seed)
    }

    #[test]
    fn an_over_limit_block_is_refused_and_names_its_kind_limit() {
        let err = CoreBlock::new(BlockKind::Task, "task", "x".repeat(11), 10, ulid(1)).unwrap_err();
        assert_eq!(err.kind(), "block_too_large");
        assert!(err.to_string().contains("task"), "{err}");
        assert!(err.to_string().contains("10"), "{err}");
        match err {
            BeingError::BlockTooLarge {
                kind,
                limit,
                actual,
            } => {
                assert_eq!(kind, "task");
                assert_eq!(limit, 10);
                assert_eq!(actual, 11);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn a_refused_set_leaves_the_old_content_intact() {
        let mut block = CoreBlock::new(BlockKind::Self_, "self", "steady", 16, ulid(1)).unwrap();
        let before = block.clone();
        let err = block.set_content("x".repeat(17), ulid(2)).unwrap_err();
        assert_eq!(err.kind(), "block_too_large");
        assert_eq!(block, before, "a refused edit must not mutate the block");
        assert_eq!(block.updated_ulid, before.updated_ulid);
    }

    #[test]
    fn a_boundary_block_is_accepted_and_the_limit_measures_characters() {
        let block =
            CoreBlock::new(BlockKind::Human, "human", "x".repeat(1500), 1500, ulid(1)).unwrap();
        assert_eq!(block.len(), 1500);
        assert!(!block.is_empty());

        // Five multi-byte characters are five characters, not fifteen bytes.
        let wide = CoreBlock::new(BlockKind::Human, "human", "日本語です", 5, ulid(1)).unwrap();
        assert_eq!(wide.len(), 5);
    }

    #[test]
    fn kinds_round_trip_and_have_distinct_limits() {
        for kind in [BlockKind::Self_, BlockKind::Human, BlockKind::Task] {
            assert_eq!(BlockKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(BlockKind::parse("self_"), None);
        assert!(default_limit(BlockKind::Self_) > default_limit(BlockKind::Human));
        assert!(default_limit(BlockKind::Human) > default_limit(BlockKind::Task));
    }
}
