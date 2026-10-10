//! `ActivationCondition` — the one condition type the whole system shares.
//!
//! The phase index (decision D1) settles this in `mm-core` because two phases need
//! it: a **technique** says when it applies, and a **policy** says when it becomes
//! relevant. Two definitions would drift, and a technique that applies under one
//! reading of a condition while a policy fires under another is exactly the kind
//! of inconsistency the library exists to prevent. `mm-library` re-exports this
//! type rather than declaring its own.
//!
//! A condition is a short piece of natural language with one machine-checkable
//! property: [`ActivationCondition::matches`] can say whether a state's text
//! mentions it. That is deliberately weak — matching is a *lexical* hint used for
//! ranking, never a decision. Nothing here is probabilistic.

use serde::{Deserialize, Serialize};

use crate::error::MmError;

/// The longest a condition may be, in characters.
pub const MAX_CONDITION_CHARS: usize = 300;

/// A condition under which an entry applies or becomes relevant.
///
/// Constructing one is the validation: empty or whitespace-only text is refused,
/// and the text is trimmed and length-bounded, so a stored condition is always
/// something a human could have written.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActivationCondition(String);

impl ActivationCondition {
    /// Build a condition, refusing empty or over-long text.
    pub fn new(text: impl Into<String>) -> Result<Self, MmError> {
        let text = text.into();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(MmError::Config(
                "an activation condition must not be empty".to_string(),
            ));
        }
        if trimmed.chars().count() > MAX_CONDITION_CHARS {
            return Err(MmError::Config(format!(
                "an activation condition must be at most {MAX_CONDITION_CHARS} characters"
            )));
        }
        Ok(ActivationCondition(trimmed.to_string()))
    }

    /// The condition's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The lowercase alphanumeric tokens of the condition, in order.
    ///
    /// Deterministic and allocation-light: digits and letters only, so punctuation
    /// and case cannot make two equivalent conditions look different.
    pub fn tokens(&self) -> Vec<String> {
        self.0
            .split(|c: char| !c.is_alphanumeric())
            .filter(|token| !token.is_empty())
            .map(|token| token.to_ascii_lowercase())
            .collect()
    }

    /// True when every token of the condition appears in `haystack`.
    ///
    /// All-of rather than any-of: a condition is a conjunction of its words, and a
    /// hint that fires on a single shared word would rank everything.
    pub fn matches(&self, haystack: &str) -> bool {
        let haystack = haystack.to_ascii_lowercase();
        let tokens = self.tokens();
        !tokens.is_empty() && tokens.iter().all(|token| haystack.contains(token.as_str()))
    }
}

impl std::fmt::Display for ActivationCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_condition_is_refused_and_text_is_trimmed() {
        assert!(ActivationCondition::new("   ").is_err());
        let condition = ActivationCondition::new("  two solutions are viable  ").unwrap();
        assert_eq!(condition.as_str(), "two solutions are viable");
    }

    #[test]
    fn an_over_long_condition_is_refused() {
        let long = "x".repeat(MAX_CONDITION_CHARS + 1);
        assert!(ActivationCondition::new(long).is_err());
    }

    #[test]
    fn matching_is_all_of_and_case_insensitive() {
        let condition = ActivationCondition::new("two solutions are viable").unwrap();
        assert!(condition.matches("TWO candidate SOLUTIONS are VIABLE now"));
        assert!(!condition.matches("two solutions exist"));
        // A single shared word must not fire it.
        assert!(!condition.matches("solutions"));
    }

    #[test]
    fn tokens_are_alphanumeric_and_lowercase() {
        let condition = ActivationCondition::new("High-stakes, DECISION (reversible?)").unwrap();
        assert_eq!(
            condition.tokens(),
            vec!["high", "stakes", "decision", "reversible"]
        );
    }

    #[test]
    fn a_condition_round_trips_through_json() {
        let condition = ActivationCondition::new("a simpler alternative exists").unwrap();
        let json = serde_json::to_string(&condition).unwrap();
        assert_eq!(json, "\"a simpler alternative exists\"");
        let back: ActivationCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(back, condition);
    }
}
