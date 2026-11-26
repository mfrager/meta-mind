//! The error taxonomy and the triggers.
//!
//! Two closed vocabularies, and the priority an analysis is ranked by.
//!
//! * [`ErrorClass`] is *what went wrong*, in sixteen kinds that are different enough
//!   to act on differently. The set is closed on purpose: an open vocabulary would
//!   let two analyses of the same failure disagree about its class, and the point of
//!   classifying at all is that the class decides which corrective change is worth
//!   making.
//! * [`Trigger`] is *why anyone looked*. The eight triggers are events, not a
//!   schedule: the phase plan is explicit that meta-analysis is event-triggered and
//!   never periodic, so a trigger that has not happened is not a reason to analyse.
//!
//! [`MetaAnalysisPriority`] is the sum of five bounded reasons to look. A sum rather
//! than a product or a maximum, and the choice matters: each component is on its own
//! a sufficient reason to spend attention (a failure that never recurs is still worth
//! one look), so a component of zero must not zero the total. The components are
//! bounded to `[0,1]` and the sum is therefore bounded by five, which is what lets
//! the queue be ordered deterministically without normalizing anything.

use serde::{Deserialize, Serialize};

use crate::error::{MetaError, Result};

/// What kind of mistake was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    /// A fact was not known and could have been.
    Knowledge,
    /// A fact was known but not found in time.
    Retrieval,
    /// The available information was read wrongly.
    Interpretation,
    /// Two things were compared that are not comparable.
    Comparison,
    /// An assumption that did not hold was relied on.
    Assumption,
    /// A cause was attributed that was not the cause.
    Causal,
    /// The plan was wrong, not its steps.
    Planning,
    /// The decision rule was wrong, given the plan.
    Decision,
    /// The plan was right and the execution was not.
    Execution,
    /// The result was not checked, or checked badly.
    Verification,
    /// The interaction with a user went wrong.
    Social,
    /// Time, money or compute ran out.
    Resource,
    /// A rule was broken that should have constrained the action.
    Policy,
    /// The code was wrong.
    Code,
    /// The data was wrong, missing or stale.
    Data,
    /// The model's own output was wrong.
    Model,
}

/// Every class, in the order the taxonomy lists them.
pub const ERROR_CLASSES: [ErrorClass; 16] = [
    ErrorClass::Knowledge,
    ErrorClass::Retrieval,
    ErrorClass::Interpretation,
    ErrorClass::Comparison,
    ErrorClass::Assumption,
    ErrorClass::Causal,
    ErrorClass::Planning,
    ErrorClass::Decision,
    ErrorClass::Execution,
    ErrorClass::Verification,
    ErrorClass::Social,
    ErrorClass::Resource,
    ErrorClass::Policy,
    ErrorClass::Code,
    ErrorClass::Data,
    ErrorClass::Model,
];

impl ErrorClass {
    /// Every class.
    pub const ALL: [ErrorClass; 16] = ERROR_CLASSES;

    /// The stable wire name, matching `meta_analyses.error_classes_json`.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorClass::Knowledge => "knowledge",
            ErrorClass::Retrieval => "retrieval",
            ErrorClass::Interpretation => "interpretation",
            ErrorClass::Comparison => "comparison",
            ErrorClass::Assumption => "assumption",
            ErrorClass::Causal => "causal",
            ErrorClass::Planning => "planning",
            ErrorClass::Decision => "decision",
            ErrorClass::Execution => "execution",
            ErrorClass::Verification => "verification",
            ErrorClass::Social => "social",
            ErrorClass::Resource => "resource",
            ErrorClass::Policy => "policy",
            ErrorClass::Code => "code",
            ErrorClass::Data => "data",
            ErrorClass::Model => "model",
        }
    }

    /// Parse a wire name. Unknown names are `None`, never a default: a stored class
    /// this build does not know is evidence of drift, not of `Execution`.
    pub fn parse(text: &str) -> Option<Self> {
        ERROR_CLASSES.into_iter().find(|c| c.as_str() == text)
    }
}

impl std::fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an analysis ran at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// A prediction came due and was wrong.
    PredictionError,
    /// A person said the answer was wrong.
    UserCorrection,
    /// The same failure happened again.
    RepeatedFailure,
    /// Two things the system believes cannot both be true.
    Contradiction,
    /// The outcome was not the one the system expected.
    UnexpectedOutcome,
    /// A goal was not reached.
    GoalFailure,
    /// Nothing failed, and it nearly did.
    NearMiss,
    /// Something worked that had never worked before.
    NovelSuccess,
}

/// Every trigger, in the order the phase plan lists them.
pub const TRIGGERS: [Trigger; 8] = [
    Trigger::PredictionError,
    Trigger::UserCorrection,
    Trigger::RepeatedFailure,
    Trigger::Contradiction,
    Trigger::UnexpectedOutcome,
    Trigger::GoalFailure,
    Trigger::NearMiss,
    Trigger::NovelSuccess,
];

impl Trigger {
    /// Every trigger.
    pub const ALL: [Trigger; 8] = TRIGGERS;

    /// The stable wire name, matching `meta_analyses.trigger`.
    pub fn as_str(self) -> &'static str {
        match self {
            Trigger::PredictionError => "prediction_error",
            Trigger::UserCorrection => "user_correction",
            Trigger::RepeatedFailure => "repeated_failure",
            Trigger::Contradiction => "contradiction",
            Trigger::UnexpectedOutcome => "unexpected_outcome",
            Trigger::GoalFailure => "goal_failure",
            Trigger::NearMiss => "near_miss",
            Trigger::NovelSuccess => "novel_success",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        TRIGGERS.into_iter().find(|t| t.as_str() == text)
    }
}

impl std::fmt::Display for Trigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The five reasons to look, each bounded to `[0,1]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MetaAnalysisPriority {
    /// How new the situation is.
    pub novelty: f64,
    /// How badly it failed.
    pub failure: f64,
    /// How much the failure cost.
    pub impact: f64,
    /// How often it has happened.
    pub recurrence: f64,
    /// How uncertain the diagnosis is expected to be.
    pub uncertainty: f64,
}

impl MetaAnalysisPriority {
    /// The five components, in the order the plan names them.
    pub const COMPONENTS: [&'static str; 5] =
        ["novelty", "failure", "impact", "recurrence", "uncertainty"];

    /// Build from the five components, refusing anything out of range.
    ///
    /// Refused rather than clamped: a component of 1.4 means the caller believes
    /// something different about the scale than this type does, and silently
    /// clamping would let a mis-scaled component look like a strong reason.
    pub fn new(
        novelty: f64,
        failure: f64,
        impact: f64,
        recurrence: f64,
        uncertainty: f64,
    ) -> Result<Self> {
        let priority = MetaAnalysisPriority {
            novelty,
            failure,
            impact,
            recurrence,
            uncertainty,
        };
        priority.validate()?;
        Ok(priority)
    }

    /// All-zero: nothing argues for spending attention.
    pub fn none() -> Self {
        MetaAnalysisPriority {
            novelty: 0.0,
            failure: 0.0,
            impact: 0.0,
            recurrence: 0.0,
            uncertainty: 0.0,
        }
    }

    /// Refuse a component outside `[0,1]`.
    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("novelty", self.novelty),
            ("failure", self.failure),
            ("impact", self.impact),
            ("recurrence", self.recurrence),
            ("uncertainty", self.uncertainty),
        ] {
            if !(0.0..=1.0).contains(&value) || !value.is_finite() {
                return Err(MetaError::validation(
                    "priority",
                    format!("{name} must be in [0,1], got {value}"),
                ));
            }
        }
        Ok(())
    }

    /// The total: the sum, bounded by five.
    pub fn total(&self) -> f64 {
        self.novelty + self.failure + self.impact + self.recurrence + self.uncertainty
    }

    /// The components as `(name, value)` pairs, for logging a record's fields.
    pub fn fields(&self) -> [(&'static str, f64); 5] {
        [
            ("novelty", self.novelty),
            ("failure", self.failure),
            ("impact", self.impact),
            ("recurrence", self.recurrence),
            ("uncertainty", self.uncertainty),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_and_trigger_round_trips_through_its_wire_name() {
        for class in ErrorClass::ALL {
            assert_eq!(ErrorClass::parse(class.as_str()), Some(class));
            assert_eq!(class.to_string(), class.as_str());
        }
        for trigger in Trigger::ALL {
            assert_eq!(Trigger::parse(trigger.as_str()), Some(trigger));
        }
        assert_eq!(ErrorClass::parse("nonsense"), None);
        assert_eq!(Trigger::parse("periodic_check"), None);
    }

    #[test]
    fn the_vocabularies_are_closed_and_distinct() {
        let classes: std::collections::BTreeSet<&str> =
            ErrorClass::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(classes.len(), ERROR_CLASSES.len());
        let triggers: std::collections::BTreeSet<&str> =
            Trigger::ALL.iter().map(|t| t.as_str()).collect();
        assert_eq!(triggers.len(), TRIGGERS.len());
    }

    #[test]
    fn the_priority_is_the_sum_and_refuses_an_out_of_range_component() {
        let p = MetaAnalysisPriority::new(0.2, 0.7, 0.6, 0.8, 0.4).unwrap();
        assert!((p.total() - 2.7).abs() < 1e-12);
        assert_eq!(MetaAnalysisPriority::none().total(), 0.0);
        assert!(MetaAnalysisPriority::new(1.4, 0.0, 0.0, 0.0, 0.0).is_err());
        assert!(MetaAnalysisPriority::new(0.0, 0.0, 0.0, 0.0, -0.1).is_err());
        assert!(MetaAnalysisPriority::new(f64::NAN, 0.0, 0.0, 0.0, 0.0).is_err());
    }

    #[test]
    fn a_zero_component_does_not_zero_the_total() {
        // The plan's intent: any one sufficient reason is enough to look.
        let only_failure = MetaAnalysisPriority::new(0.0, 0.9, 0.0, 0.0, 0.0).unwrap();
        assert!(only_failure.total() > 0.0);
    }
}
