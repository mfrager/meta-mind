//! The one interface every decision core implements, and the id that names it.
//!
//! Three cores answer the same bounded questions (`rules`, `local`, `hosted`) and
//! one conformance suite decides whether each may be selected. The trait is
//! deliberately narrow: an answer, or a typed refusal. There is no third option,
//! because a core that guesses is the failure this whole organ exists to prevent —
//! `Unavailable` is how a core says "not here", and the caller escalates rather
//! than receiving a plausible-looking answer.
//!
//! Two contracts sit on the trait and are checked by `tests/conformance.rs`:
//!
//! * **Determinism.** Identical `(question, state)` yields an identical
//!   `(answer, confidence, features)`. A core whose answer depends on the clock,
//!   on iteration order, or on a random seed cannot be selected.
//! * **Typing.** The answer is one the question admitted — the option was offered,
//!   the score is inside the scale. `DecisionAnswer::new` enforces this, so a core
//!   that gets it wrong is refused at construction rather than trusted.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{DecisionError, Result};
use crate::question::{DecisionAnswer, DecisionQuestion, DecisionState};

/// Which backing answered a question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreId {
    /// The table-driven rules core: always available, cheap, deterministic.
    Rules,
    /// The distilled local head: available only when a head is loaded.
    Local,
    /// The hosted core: available only when a provider is reachable.
    Hosted,
}

/// Every core, in wire order. The conformance suite runs over all three.
pub const CORE_IDS: [CoreId; 3] = [CoreId::Rules, CoreId::Local, CoreId::Hosted];

impl CoreId {
    /// The stable wire name, which is also the `decisions.core_impl` value.
    pub fn as_str(self) -> &'static str {
        match self {
            CoreId::Rules => "rules",
            CoreId::Local => "local",
            CoreId::Hosted => "hosted",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        CORE_IDS.into_iter().find(|core| core.as_str() == text)
    }
}

impl std::fmt::Display for CoreId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A source of bounded answers.
///
/// `&self`, not `&mut self`: a core is shared across episodes and answers
/// concurrently, so all mutable state a core carries must be internally
/// synchronized and must not change an answer it already gave.
#[async_trait]
pub trait DecisionCore: Send + Sync {
    /// Which core this is.
    fn id(&self) -> CoreId;

    /// Answer one bounded question about one state.
    ///
    /// Deterministic: the same `(question, state)` always produces the same
    /// answer, confidence and features. Refuses with
    /// [`DecisionError::Unavailable`] when it cannot answer at all — never with a
    /// fabricated answer.
    async fn answer(
        &self,
        question: &DecisionQuestion,
        state: &DecisionState,
    ) -> Result<DecisionAnswer>;
}

/// A shared, type-erased decision core.
pub type SharedCore = Arc<dyn DecisionCore>;

/// Answer a question through an optional core, degrading honestly.
///
/// A caller that configures no core — or a core that refuses — gets `None` and
/// must escalate. This is the single place "no core" is turned into a decision a
/// firewall can see, so no call site accidentally treats a missing answer as a
/// negative one.
pub async fn answer_or_none(
    core: Option<&SharedCore>,
    question: &DecisionQuestion,
    state: &DecisionState,
) -> Option<Result<DecisionAnswer>> {
    let core = core?;
    Some(core.answer(question, state).await)
}

/// Whether a refusal means "this core cannot answer here" rather than "the answer
/// was bad". Only the former is a legal outcome for a conformance run.
pub fn is_legal_refusal(error: &DecisionError) -> bool {
    error.is_unavailable()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_ids_round_trip_and_reject_unknown_names() {
        for core in CORE_IDS {
            assert_eq!(CoreId::parse(core.as_str()), Some(core));
            assert_eq!(CoreId::parse(&core.as_str().to_uppercase()), Some(core));
        }
        assert_eq!(CoreId::parse("oracle"), None);
        assert_eq!(CoreId::parse(""), None);
    }
}
