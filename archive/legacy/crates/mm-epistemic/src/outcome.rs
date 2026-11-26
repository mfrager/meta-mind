//! Outcomes: what actually happened, as a first-class value.
//!
//! `INDEX.md` D2 settles a cross-phase seam: Phase 7's `record_fitness` takes the
//! Phase 6 outcome type, and Phase 7 must not define a parallel one. This is that
//! type, and it is deliberately small — a status, the observed value, an optional
//! numeric error, and when it was observed. Everything a fitness function needs,
//! and nothing a fitness function could invent.
//!
//! The status set is closed and the [`Outcome::score`] mapping is fixed, because
//! both feed a learning signal: a scoring function that a caller could override
//! would make two runs of the same history teach different lessons.

use mm_core::{Timestamp, Ulid};
use serde::{Deserialize, Serialize};

use crate::error::{EpistemicError, Result};

/// How an expectation turned out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    /// What was expected happened.
    Success,
    /// Partly: some of the expected result obtained.
    Partial,
    /// It did not happen.
    Failure,
    /// It nearly did, or nearly did something worse.
    NearMiss,
    /// The outcome could not be determined.
    Unknown,
}

/// Every outcome status, best to worst.
pub const OUTCOME_STATUSES: [OutcomeStatus; 5] = [
    OutcomeStatus::Success,
    OutcomeStatus::Partial,
    OutcomeStatus::NearMiss,
    OutcomeStatus::Failure,
    OutcomeStatus::Unknown,
];

impl OutcomeStatus {
    /// The stable wire name, matching `predictions.outcome_status`.
    pub fn as_str(self) -> &'static str {
        match self {
            OutcomeStatus::Success => "success",
            OutcomeStatus::Partial => "partial",
            OutcomeStatus::Failure => "failure",
            OutcomeStatus::NearMiss => "near_miss",
            OutcomeStatus::Unknown => "unknown",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        OUTCOME_STATUSES
            .into_iter()
            .find(|status| status.as_str() == text)
    }

    /// The fixed score this status contributes to a fitness signal, in `[0,1]`.
    pub fn score(self) -> f64 {
        match self {
            OutcomeStatus::Success => 1.0,
            OutcomeStatus::Partial => 0.5,
            OutcomeStatus::NearMiss => 0.25,
            OutcomeStatus::Failure => 0.0,
            OutcomeStatus::Unknown => 0.0,
        }
    }

    /// True when the outcome settles the question it was asked.
    pub fn is_resolved(self) -> bool {
        !matches!(self, OutcomeStatus::Unknown)
    }
}

impl std::fmt::Display for OutcomeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What happened, tied to the prediction it settles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    /// The prediction this resolves, when it resolves one.
    pub prediction_id: Option<Ulid>,
    /// How it turned out.
    pub status: OutcomeStatus,
    /// The observed value, when there is one worth recording.
    pub value: Option<String>,
    /// The numeric error, when the outcome was numeric.
    pub error: Option<f64>,
    /// When it was observed.
    pub observed_at: Option<Timestamp>,
    /// Anything a human needs to read later. Bounded by the caller.
    pub notes: String,
}

impl Outcome {
    /// An outcome with just a status.
    pub fn new(status: OutcomeStatus) -> Self {
        Outcome {
            prediction_id: None,
            status,
            value: None,
            error: None,
            observed_at: None,
            notes: String::new(),
        }
    }

    /// The prediction it resolves.
    pub fn for_prediction(mut self, prediction: Ulid) -> Self {
        self.prediction_id = Some(prediction);
        self
    }

    /// The observed value.
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// The numeric error.
    pub fn with_error(mut self, error: f64) -> Result<Self> {
        if !error.is_finite() {
            return Err(EpistemicError::validation(
                "error",
                format!("must be finite, got {error}"),
            ));
        }
        self.error = Some(error);
        Ok(self)
    }

    /// When it was observed.
    pub fn with_observed_at(mut self, at: Timestamp) -> Self {
        self.observed_at = Some(at);
        self
    }

    /// Free-text notes.
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = notes.into();
        self
    }

    /// True when the outcome is a success.
    pub fn succeeded(&self) -> bool {
        self.status == OutcomeStatus::Success
    }

    /// The fixed score, so a fitness function has one number to consume.
    pub fn score(&self) -> f64 {
        self.status.score()
    }

    /// Refuse an outcome that cannot be read back.
    pub fn validate(&self) -> Result<()> {
        if let Some(error) = self.error {
            if !error.is_finite() {
                return Err(EpistemicError::validation(
                    "error",
                    format!("must be finite, got {error}"),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulid::Ulid;

    #[test]
    fn statuses_round_trip_and_score_monotonically() {
        for status in OUTCOME_STATUSES {
            assert_eq!(OutcomeStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(OutcomeStatus::parse("nope"), None);
        assert!(
            OutcomeStatus::Success.score() > OutcomeStatus::Partial.score(),
            "a success must be worth more than a partial"
        );
        assert!(
            OutcomeStatus::Partial.score() > OutcomeStatus::NearMiss.score(),
            "a partial is worth more than a near miss"
        );
        assert!(
            OutcomeStatus::NearMiss.score() > OutcomeStatus::Failure.score(),
            "a near miss is worth more than a failure"
        );
        assert!(!OutcomeStatus::Unknown.is_resolved());
        assert!(OutcomeStatus::Failure.is_resolved());
    }

    #[test]
    fn an_outcome_carries_its_prediction_and_scores_itself() {
        let outcome = Outcome::new(OutcomeStatus::Success)
            .for_prediction(Ulid::from_parts(1_700_000_000_000, 7))
            .with_value("thursday")
            .with_observed_at(Timestamp::from_epoch_seconds(1))
            .with_notes("the window moved");
        assert!(outcome.succeeded());
        assert_eq!(outcome.score(), 1.0);
        assert_eq!(outcome.prediction_id.unwrap().to_string().len(), 26);
        outcome.validate().unwrap();
    }

    #[test]
    fn a_non_finite_error_is_refused() {
        assert!(Outcome::new(OutcomeStatus::Failure)
            .with_error(f64::NAN)
            .is_err());
        assert!(Outcome::new(OutcomeStatus::Failure)
            .with_error(f64::INFINITY)
            .is_err());
        assert!(Outcome::new(OutcomeStatus::Failure)
            .with_error(0.25)
            .is_ok());
    }
}
