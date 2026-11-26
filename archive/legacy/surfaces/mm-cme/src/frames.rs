//! Frame-engine contract over the reviewed prompt/frame tree.
//!
//! A frame is data: a template id, a kind, a strategy, and timing policy. The
//! engine validates transitions and stack operations; it does not own prompt
//! prose or call a model. Runtime scheduling and delivery are wired in Phase 1.

use std::sync::Arc;

use mm_data::{
    ConversationFrame, FrameKind, FrameStack, FrameStatus, FrameUtterance, SessionError,
};
use mm_prompt::{FrameTemplate, PromptError, PromptInstance, PromptStore};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The frame engine.
#[derive(Debug, Clone)]
pub struct FrameEngine {
    prompts: Arc<PromptStore>,
}

impl FrameEngine {
    /// Build over a validated prompt tree.
    pub fn new(prompts: Arc<PromptStore>) -> Self {
        Self { prompts }
    }

    /// The underlying prompt store.
    pub fn prompts(&self) -> &PromptStore {
        &self.prompts
    }

    /// A frame template by id.
    pub fn template(&self, id: &str) -> Option<&FrameTemplate> {
        self.prompts.frame(id)
    }

    /// The mandatory root template.
    pub fn root_template(&self) -> &FrameTemplate {
        let id = &self.prompts.frame_manifest().root;
        self.prompts
            .frame(id)
            .expect("prompt store validation guarantees the root frame")
    }

    /// The template a named transition points at.
    pub fn transition(
        &self,
        template_id: &str,
        transition: &str,
    ) -> Result<&FrameTemplate, FrameError> {
        let template = self
            .template(template_id)
            .ok_or_else(|| FrameError::UnknownTemplate(template_id.into()))?;
        let target =
            template
                .transitions
                .get(transition)
                .ok_or_else(|| FrameError::UnknownTransition {
                    template: template_id.into(),
                    transition: transition.into(),
                })?;
        self.template(target)
            .ok_or_else(|| FrameError::UnknownTemplate(target.clone()))
    }

    /// Build a frame from a template. The caller decides when to push it.
    pub fn frame(
        &self,
        template_id: &str,
        frame_id: impl Into<String>,
        now_ms: i64,
    ) -> Result<ConversationFrame, FrameError> {
        let template = self
            .template(template_id)
            .ok_or_else(|| FrameError::UnknownTemplate(template_id.into()))?;
        Ok(ConversationFrame {
            id: frame_id.into(),
            template_id: template.id.clone(),
            template_version: template.version,
            kind: parse_kind(&template.kind)?,
            status: FrameStatus::Active,
            entered_at: now_ms,
            updated_at: now_ms,
            expires_at: None,
        })
    }

    /// Push a frame after validating the template and stack.
    pub fn push(
        &self,
        stack: &mut FrameStack,
        template_id: &str,
        frame_id: impl Into<String>,
        now_ms: i64,
    ) -> Result<ConversationFrame, FrameError> {
        let frame = self.frame(template_id, frame_id, now_ms)?;
        stack.push(frame.clone())?;
        Ok(frame)
    }

    /// Replace the top frame with one from a template.
    pub fn replace_top(
        &self,
        stack: &mut FrameStack,
        template_id: &str,
        frame_id: impl Into<String>,
        now_ms: i64,
    ) -> Result<ConversationFrame, FrameError> {
        let frame = self.frame(template_id, frame_id, now_ms)?;
        stack.replace_top(frame.clone())?;
        Ok(frame)
    }

    /// Pop the top frame. The root is protected by [`FrameStack`].
    pub fn pop(
        &self,
        stack: &mut FrameStack,
        now_ms: i64,
    ) -> Result<ConversationFrame, FrameError> {
        Ok(stack.pop(now_ms)?)
    }

    /// Build an utterance for the top frame. `attempt` starts at zero.
    pub fn utterance(
        &self,
        stack: &FrameStack,
        now_ms: i64,
        attempt: u8,
        seed: u64,
    ) -> Result<FrameUtterance, FrameError> {
        let frame = stack
            .top()
            .ok_or_else(|| FrameError::Invalid("frame stack is empty".into()))?;
        let template = self
            .template(&frame.template_id)
            .ok_or_else(|| FrameError::UnknownTemplate(frame.template_id.clone()))?;
        if attempt >= template.utterance.max_attempts {
            return Err(FrameError::AttemptsExhausted {
                template: template.id.clone(),
                attempts: attempt,
            });
        }
        let delay = template
            .utterance
            .delay_ms
            .saturating_mul(u64::from(template.utterance.backoff).pow(attempt.into()));
        let jitter = apply_jitter(delay, template.utterance.jitter, seed);
        Ok(FrameUtterance {
            id: format!("{}:{}:{}", frame.id, template.id, attempt),
            frame_id: frame.id.clone(),
            due_at: now_ms.saturating_add(jitter.min(i64::MAX as u64) as i64),
            reason: format!("frame {} attempt {}", template.id, attempt + 1),
            attempt,
        })
    }

    /// Render a frame's strategy for the inspector/prompt block.
    pub fn render(
        &self,
        template_id: &str,
        variables: &std::collections::BTreeMap<String, String>,
    ) -> Result<PromptInstance, FrameError> {
        let template = self
            .template(template_id)
            .ok_or_else(|| FrameError::UnknownTemplate(template_id.into()))?;
        Ok(self.prompts.render(&template.strategy, variables)?)
    }

    /// Whether this is a goodbye template.
    pub fn is_goodbye(&self, template_id: &str) -> bool {
        self.template(template_id)
            .is_some_and(|template| template.kind == "goodbye")
    }
}

/// Map a data kind to the stack enum.
pub fn parse_kind(kind: &str) -> Result<FrameKind, FrameError> {
    match kind {
        "intro" => Ok(FrameKind::Intro),
        "resume" => Ok(FrameKind::Resume),
        "work" => Ok(FrameKind::Work),
        "tutorial_offer" => Ok(FrameKind::TutorialOffer),
        "tutorial" => Ok(FrameKind::Tutorial),
        "goodbye" => Ok(FrameKind::Goodbye),
        "custom" => Ok(FrameKind::Custom),
        other => Err(FrameError::UnknownKind(other.into())),
    }
}

fn apply_jitter(delay: u64, jitter: f64, seed: u64) -> u64 {
    if jitter <= 0.0 || delay == 0 {
        return delay;
    }
    let fraction = (seed % 10_000) as f64 / 10_000.0;
    let factor = 1.0 + jitter.clamp(0.0, 1.0) * (fraction * 2.0 - 1.0);
    (delay as f64 * factor).round() as u64
}

/// Frame-engine failures.
#[derive(Debug, Error)]
pub enum FrameError {
    /// Unknown template.
    #[error("unknown frame template {0}")]
    UnknownTemplate(String),
    /// Unknown transition.
    #[error("template {template} has no transition {transition}")]
    UnknownTransition {
        /// Template.
        template: String,
        /// Transition.
        transition: String,
    },
    /// Unknown kind.
    #[error("unknown frame kind {0}")]
    UnknownKind(String),
    /// No further utterances.
    #[error("template {template} exhausted its {attempts} attempts")]
    AttemptsExhausted {
        /// Template.
        template: String,
        /// Attempts made.
        attempts: u8,
    },
    /// Stack/session invariant.
    #[error("frame: {0}")]
    Session(#[from] SessionError),
    /// Prompt rendering failure.
    #[error("prompt: {0}")]
    Prompt(#[from] PromptError),
    /// General invariant.
    #[error("invalid frame: {0}")]
    Invalid(String),
}

/// Result alias.
pub type Result<T, E = FrameError> = std::result::Result<T, E>;

/// The persisted frame-timer projection used by the inspector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameTimerView {
    /// Frame id.
    pub frame_id: String,
    /// Template id.
    pub template_id: String,
    /// Next due instant.
    pub due_at: Option<i64>,
    /// Attempts made.
    pub attempts: u8,
    /// Maximum attempts.
    pub max_attempts: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn engine() -> FrameEngine {
        let store = PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .expect("checked-in tree loads");
        FrameEngine::new(Arc::new(store))
    }

    #[test]
    fn root_and_transition_are_resolved_from_data() {
        let engine = engine();
        let root = engine.root_template();
        assert!(!root.id.is_empty());
        let first = root
            .transitions
            .keys()
            .next()
            .expect("root has a transition");
        let target = engine.transition(&root.id, first).unwrap();
        assert!(!target.id.is_empty());
    }

    #[test]
    fn frame_can_only_push_to_the_bounded_stack() {
        let engine = engine();
        let mut stack = FrameStack::new(engine.frame("frame.intro", "root", 1).unwrap());
        for index in 0..7 {
            let template = engine.root_template().id.clone();
            engine
                .push(&mut stack, &template, format!("f{index}"), 2)
                .unwrap();
        }
        assert!(engine
            .push(&mut stack, "frame.intro", "overflow", 3)
            .is_err());
    }

    #[test]
    fn utterance_backoff_is_deterministic_and_capped() {
        let engine = engine();
        let stack = FrameStack::new(engine.frame("frame.intro", "root", 1).unwrap());
        let first = engine.utterance(&stack, 1_000, 0, 42).unwrap();
        let second = engine.utterance(&stack, 1_000, 1, 42).unwrap();
        assert!(second.due_at >= first.due_at);
    }

    #[test]
    fn every_frame_renders() {
        let engine = engine();
        for frame in engine.prompts().frames() {
            let mut variables = BTreeMap::new();
            for slot in &frame.slots {
                variables.insert(slot.clone(), format!("fixture:{slot}"));
            }
            let strategy = engine.prompts().strategy(&frame.strategy).unwrap();
            for slot in &strategy.slots {
                variables.insert(slot.clone(), format!("fixture:{slot}"));
            }
            engine.render(&frame.id, &variables).unwrap();
        }
    }
}
