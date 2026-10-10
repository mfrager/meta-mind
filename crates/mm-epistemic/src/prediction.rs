//! Predictions: propositions about the future, with the caller's probability.
//!
//! A prediction is the one kind of claim that is *meant* to be wrong, so the
//! interesting part is resolution. [`Prediction::resolve`] takes the [`Outcome`]
//! the world supplied and keeps it, rather than overwriting the prediction: a
//! prediction that quietly became "what happened" would destroy the only record
//! that it was a guess.
//!
//! The Brier contribution ([`Prediction::brier`]) is computed here and nowhere
//! else, so Phase 9 and Phase 11 score the same history the same way. It is
//! `(probability - outcome)^2`, the standard proper scoring rule, and it takes no
//! model input.

use mm_core::{Timestamp, Ulid};
use serde::{Deserialize, Serialize};

use crate::error::{EpistemicError, Result};
use crate::outcome::{Outcome, OutcomeStatus};
use crate::proposition::Proposition;

/// An expectation about the future.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prediction {
    /// The prediction's ULID.
    pub id: Ulid,
    /// What is expected.
    pub proposition: Proposition,
    /// The caller's probability, in `[0,1]`. Never computed here.
    pub probability: f64,
    /// How far ahead it looks, in seconds.
    pub horizon_secs: u64,
    /// The conditions it was made under.
    pub conditions: Vec<String>,
    /// What happened, once the world has answered.
    pub outcome: Option<PredictionOutcome>,
}

/// The world's answer to a prediction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredictionOutcome {
    /// How it turned out.
    pub status: OutcomeStatus,
    /// The observed value, when there is one.
    pub value: Option<String>,
    /// When it was observed.
    pub resolved_at: Timestamp,
    /// The numeric error, when the outcome was numeric.
    pub error: Option<f64>,
}

impl Prediction {
    /// Build a prediction.
    pub fn new(
        id: Ulid,
        proposition: Proposition,
        probability: f64,
        horizon_secs: u64,
    ) -> Result<Self> {
        let prediction = Prediction {
            id,
            proposition,
            probability,
            horizon_secs,
            conditions: Vec::new(),
            outcome: None,
        };
        prediction.validate()?;
        Ok(prediction)
    }

    /// The conditions it was made under.
    pub fn with_conditions(mut self, conditions: Vec<String>) -> Self {
        self.conditions = conditions;
        self
    }

    /// Refuse a prediction that cannot be scored.
    pub fn validate(&self) -> Result<()> {
        if self.id.is_nil() {
            return Err(EpistemicError::validation("id", "must not be the nil ULID"));
        }
        if !self.probability.is_finite() || !(0.0..=1.0).contains(&self.probability) {
            return Err(EpistemicError::validation(
                "probability",
                format!("must be in [0,1], got {}", self.probability),
            ));
        }
        if let Some(outcome) = &self.outcome {
            if let Some(error) = outcome.error {
                if !error.is_finite() {
                    return Err(EpistemicError::validation(
                        "error",
                        format!("must be finite, got {error}"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// True once the world has answered.
    pub fn is_resolved(&self) -> bool {
        self.outcome
            .as_ref()
            .is_some_and(|outcome| outcome.status.is_resolved())
    }

    /// Keep the world's answer. A prediction is resolved once, and a second
    /// attempt is refused rather than silently overwriting the first.
    pub fn resolve(&mut self, outcome: &Outcome, at: Timestamp) -> Result<()> {
        if self.outcome.is_some() {
            return Err(EpistemicError::validation(
                "outcome",
                "a prediction is resolved once; the first answer is the record",
            ));
        }
        outcome.validate()?;
        self.outcome = Some(PredictionOutcome {
            status: outcome.status,
            value: outcome.value.clone(),
            resolved_at: at,
            error: outcome.error,
        });
        Ok(())
    }

    /// The Brier contribution of a resolved prediction: `(p - outcome)^2`, where a
    /// success is 1 and every other resolution is 0.
    ///
    /// `None` for an unresolved prediction or one the world could not settle —
    /// scoring an unresolved guess would be inventing an outcome.
    pub fn brier(&self) -> Option<f64> {
        let outcome = self.outcome.as_ref()?;
        if !outcome.status.is_resolved() {
            return None;
        }
        let observed = if outcome.status == OutcomeStatus::Success {
            1.0
        } else {
            0.0
        };
        Some((self.probability - observed).powi(2))
    }

    /// The score this prediction contributes, in the same `[0,1]` shape the
    /// outcome uses: `1 - brier`, or the partial credit a near miss earns.
    pub fn score(&self) -> Option<f64> {
        let outcome = self.outcome.as_ref()?;
        if !outcome.status.is_resolved() {
            return None;
        }
        if outcome.status == OutcomeStatus::NearMiss {
            return Some(OutcomeStatus::NearMiss.score());
        }
        Some(1.0 - self.brier()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposition::Proposition;
    use ulid::Ulid as UlidType;

    fn id(n: u128) -> Ulid {
        UlidType::from_parts(1_700_000_000_000, n)
    }

    fn proposition() -> Proposition {
        Proposition::literal("https://x/s", "https://x/p", "v").unwrap()
    }

    fn at() -> Timestamp {
        Timestamp::from_epoch_seconds(1_800_000_000)
    }

    #[test]
    fn a_prediction_refuses_a_probability_outside_the_unit_interval() {
        assert!(Prediction::new(id(1), proposition(), 1.5, 60).is_err());
        assert!(Prediction::new(id(1), proposition(), f64::NAN, 60).is_err());
        assert!(Prediction::new(id(1), proposition(), 0.0, 0).is_ok());
    }

    #[test]
    fn resolving_keeps_the_answer_and_refuses_a_second_one() {
        let mut prediction = Prediction::new(id(1), proposition(), 0.8, 3_600)
            .unwrap()
            .with_conditions(vec!["the schedule is unchanged".to_string()]);
        assert!(!prediction.is_resolved());
        let outcome = Outcome::new(OutcomeStatus::Failure).with_value("stayed friday");
        prediction.resolve(&outcome, at()).unwrap();
        assert!(prediction.is_resolved());
        assert!((prediction.brier().unwrap() - 0.64).abs() < 1e-12);
        assert!((prediction.score().unwrap() - 0.36).abs() < 1e-12);
        // A second resolution is refused: the first answer is the record.
        assert!(prediction.resolve(&outcome, at()).is_err());
    }

    #[test]
    fn a_perfect_guess_scores_one_and_a_sure_miss_scores_zero() {
        let mut confident = Prediction::new(id(1), proposition(), 1.0, 60).unwrap();
        confident
            .resolve(&Outcome::new(OutcomeStatus::Success), at())
            .unwrap();
        assert_eq!(confident.brier(), Some(0.0));
        assert_eq!(confident.score(), Some(1.0));

        let mut wrong = Prediction::new(id(2), proposition(), 1.0, 60).unwrap();
        wrong
            .resolve(&Outcome::new(OutcomeStatus::Failure), at())
            .unwrap();
        assert_eq!(wrong.brier(), Some(1.0));
        assert_eq!(wrong.score(), Some(0.0));
    }

    #[test]
    fn an_unresolved_or_unsettled_prediction_has_no_score() {
        let prediction = Prediction::new(id(1), proposition(), 0.5, 60).unwrap();
        assert_eq!(prediction.brier(), None);
        assert_eq!(prediction.score(), None);

        let mut unsettled = Prediction::new(id(2), proposition(), 0.5, 60).unwrap();
        unsettled
            .resolve(&Outcome::new(OutcomeStatus::Unknown), at())
            .unwrap();
        // It is stored, but it is not scored: the world did not answer.
        assert!(!unsettled.is_resolved());
        assert_eq!(unsettled.brier(), None);
    }

    #[test]
    fn a_near_miss_earns_fixed_partial_credit() {
        let mut prediction = Prediction::new(id(1), proposition(), 0.9, 60).unwrap();
        prediction
            .resolve(
                &Outcome::new(OutcomeStatus::NearMiss).with_notes("it nearly shipped"),
                at(),
            )
            .unwrap();
        assert_eq!(prediction.score(), Some(0.25));
    }
}
