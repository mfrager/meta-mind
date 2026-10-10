//! What an operation is worth, and which one to run next.
//!
//! The phase's invariant 1 is "spend the least cognition that can change the
//! decision", and invariant 3 is "selection is deterministic". Both live here.
//! [`OperationValue::score`] is one fixed arithmetic expression over four
//! caller-supplied numbers — expected error reduction, decision importance, the
//! probability the decision changes, and cost — and [`select_next`] is a maximum
//! with an explicit tie-break, so no ordering depends on a container's iteration
//! order.
//!
//! Nothing here computes a probability. Every input is a parameter, which is what
//! makes the golden `operation_value.csv` a legitimate gate rather than a
//! snapshot of whatever a model happened to say.

use serde::{Deserialize, Serialize};

use crate::episode::check_unit;
use crate::error::{MetacogError, Result};
use crate::graph::{NodeId, OpGraph};

/// The four components an operation is scored by, and the score they produce.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationValue {
    /// How much running it is expected to reduce the error, in `[0,1]`.
    pub expected_error_reduction: f64,
    /// How much the decision depends on being right, in `[0,1]`.
    pub decision_importance: f64,
    /// How likely the answer is to change the decision, in `[0,1]`.
    pub probability_change: f64,
    /// What it costs, in the same units as the budget. Non-negative.
    pub cost: f64,
}

impl OperationValue {
    /// The smallest divisor the score will use, so a free operation is never a
    /// division by zero.
    pub const EPS: f64 = 1e-9;

    /// Score an operation.
    ///
    /// The expression is `eer * importance * p_change / max(cost, EPS)`: value
    /// per unit of cost, so a cheap operation that could change the decision
    /// outranks an expensive one that could not. `cost` is a *weight*, not a
    /// probability, so clamping it is not needed — but a non-finite or negative
    /// one is refused rather than clamped, because a negative cost would make a
    /// bad operation look free.
    pub fn new(
        expected_error_reduction: f64,
        decision_importance: f64,
        probability_change: f64,
        cost: f64,
    ) -> Result<Self> {
        let value = OperationValue {
            expected_error_reduction,
            decision_importance,
            probability_change,
            cost,
        };
        value.validate()?;
        Ok(value)
    }

    /// A value that could never justify running an operation.
    pub fn worthless(cost: f64) -> Self {
        OperationValue {
            expected_error_reduction: 0.0,
            decision_importance: 0.0,
            probability_change: 0.0,
            cost,
        }
    }

    /// The value per unit of cost.
    pub fn score(&self) -> f64 {
        self.expected_error_reduction * self.decision_importance * self.probability_change
            / self.cost.max(Self::EPS)
    }

    /// True when running the operation could change the decision.
    pub fn is_worth_running(&self) -> bool {
        self.score() > 0.0
    }

    /// Refuse a value that cannot be ranked.
    pub fn validate(&self) -> Result<()> {
        check_unit(
            "value.expected_error_reduction",
            self.expected_error_reduction,
        )?;
        check_unit("value.decision_importance", self.decision_importance)?;
        check_unit("value.probability_change", self.probability_change)?;
        if !self.cost.is_finite() || self.cost < 0.0 {
            return Err(MetacogError::validation(
                "value.cost",
                format!("must be finite and non-negative, got {}", self.cost),
            ));
        }
        Ok(())
    }

    /// A fixed rendering, so a golden file can pin the score exactly.
    ///
    /// Nine decimal places: enough to distinguish every reachable score at the
    /// scales the corpus uses, and a fixed shape so a printed score and a stored
    /// score compare equal.
    pub fn canonical(&self) -> String {
        format!(
            "eer={:.9},importance={:.9},p_change={:.9},cost={:.9},score={:.9}",
            self.expected_error_reduction,
            self.decision_importance,
            self.probability_change,
            self.cost,
            self.score()
        )
    }

    /// The score as the CSV column renders it.
    pub fn score_column(&self) -> String {
        format!("{:.9}", self.score())
    }
}

/// The next operation to run: the highest score among `ready`, ties broken by the
/// smallest node id.
///
/// A node whose value could never justify running is never selected — there is no
/// "run it anyway to be safe", because that is exactly the overthinking the phase
/// exists to bound.
pub fn select_next(graph: &OpGraph, ready: &[NodeId]) -> Option<NodeId> {
    let mut candidates: Vec<(NodeId, f64)> = ready
        .iter()
        .filter_map(|id| graph.node(*id).map(|node| (*id, node.value.score())))
        .filter(|(_, score)| *score > 0.0)
        .collect();
    candidates.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    candidates.first().map(|(id, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::CognitiveOp;
    use proptest::prelude::*;

    fn value(eer: f64, importance: f64, p_change: f64, cost: f64) -> OperationValue {
        OperationValue::new(eer, importance, p_change, cost).unwrap()
    }

    #[test]
    fn the_score_is_the_value_per_unit_of_cost() {
        // 0.5 * 0.5 * 0.5 / 0.25 = 0.5
        let v = value(0.5, 0.5, 0.5, 0.25);
        assert!((v.score() - 0.5).abs() < 1e-12, "{}", v.score());
        assert_eq!(v.canonical(), v.canonical());
        assert!(v.canonical().contains("score=0.500000000"));
    }

    #[test]
    fn a_free_operation_is_ranked_by_its_numerator_alone() {
        let free = value(0.5, 0.5, 0.5, 0.0);
        assert!(free.score().is_finite());
        assert!((free.score() - 0.125 / OperationValue::EPS).abs() < 1.0);
    }

    #[test]
    fn a_bad_value_is_refused_rather_than_clamped() {
        assert!(OperationValue::new(1.5, 0.5, 0.5, 1.0).is_err());
        assert!(OperationValue::new(0.5, 0.5, 0.5, -1.0).is_err());
        assert!(OperationValue::new(f64::NAN, 0.5, 0.5, 1.0).is_err());
        assert!(OperationValue::new(0.5, 0.5, 0.5, f64::INFINITY).is_err());
        assert!(OperationValue::new(0.0, 0.0, 0.0, 0.0).is_ok());
    }

    #[test]
    fn a_worthless_value_never_justifies_running_anything() {
        let v = OperationValue::worthless(0.05);
        assert!(!v.is_worth_running());
        assert_eq!(v.score(), 0.0);
    }

    #[test]
    fn select_next_takes_the_best_score_and_breaks_ties_by_id() {
        let mut graph = OpGraph::new();
        let cheap = value(0.9, 0.9, 0.9, 0.01);
        let dear = value(0.9, 0.9, 0.9, 0.5);
        let a = graph
            .add(CognitiveOp::Recall { query: "a".into() }, cheap, &[])
            .unwrap();
        let b = graph
            .add(CognitiveOp::Recall { query: "b".into() }, dear, &[])
            .unwrap();
        assert_eq!(select_next(&graph, &[a, b]), Some(a));

        // Two identical scores: the smaller id wins, whichever order they arrive.
        let mut tied = OpGraph::new();
        let v = value(0.5, 0.5, 0.5, 0.05);
        let first = tied
            .add(CognitiveOp::Recall { query: "a".into() }, v, &[])
            .unwrap();
        let second = tied
            .add(CognitiveOp::Recall { query: "b".into() }, v, &[])
            .unwrap();
        assert_eq!(select_next(&tied, &[second, first]), Some(first));
        assert_eq!(select_next(&tied, &[first, second]), Some(first));
    }

    #[test]
    fn a_ready_set_of_worthless_operations_selects_nothing() {
        let mut graph = OpGraph::new();
        let id = graph
            .add(
                CognitiveOp::Recall { query: "a".into() },
                OperationValue::worthless(0.05),
                &[],
            )
            .unwrap();
        assert_eq!(select_next(&graph, &[id]), None);
        assert_eq!(select_next(&graph, &[999]), None);
    }

    proptest! {
        #[test]
        fn the_score_rises_with_the_numerator_and_falls_with_cost(
            eer in 1u32..500,
            importance in 1u32..500,
            p_change in 1u32..500,
            cost in 1u32..500,
        ) {
            let unit = |n: u32| n as f64 / 1000.0;
            let base = value(unit(eer), unit(importance), unit(p_change), unit(cost));
            prop_assert!(base.score() > 0.0);

            // Raising any numerator in [0,1], or raising the cost, moves the score
            // strictly in the documented direction.
            let delta = |n: u32| unit(n + 500);
            prop_assert!(value(delta(eer), unit(importance), unit(p_change), unit(cost)).score() > base.score());
            prop_assert!(value(unit(eer), delta(importance), unit(p_change), unit(cost)).score() > base.score());
            prop_assert!(value(unit(eer), unit(importance), delta(p_change), unit(cost)).score() > base.score());
            prop_assert!(value(unit(eer), unit(importance), unit(p_change), delta(cost)).score() < base.score());
        }
    }
}
