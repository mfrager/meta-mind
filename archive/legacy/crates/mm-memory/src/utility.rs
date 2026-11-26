//! What a memory is worth keeping, and how much of it survives.
//!
//! Two scores, both deterministic and both pure:
//!
//! * [`memory_utility`] — the plan's `impact × probability × reliability ÷ cost`.
//!   The division is why the function returns a `Result`: a zero cost is not a
//!   free memory, it is an undefined ratio, and a caller that got `0` or
//!   `inf` back would make a policy decision on a number that means nothing.
//! * [`retention_score`] — MemoryBank's Ebbinghaus curve, modulated by
//!   importance and rehearsal. It is what `forget_cycle` compares against a
//!   threshold, so it has to be monotone where the plan says it is: strictly
//!   falling in age, strictly rising in importance and access count.
//!
//! Neither function reads a clock. Both take the instant they are measured at, so
//! a replay computes the same numbers it computed the first time.

use crate::error::{MemoryError, Result, UtilityDenied};
use crate::model::UtilityInputs;

/// A week, in nanoseconds: the default half-life for a non-core memory.
pub const WEEK_NS: u64 = 7 * 24 * 60 * 60 * 1_000_000_000;

/// Thirty days, in nanoseconds.
pub const MONTH_NS: u64 = 30 * 24 * 60 * 60 * 1_000_000_000;

/// The retrieving channels' weights, in the plan's fixed order.
///
/// `hybrid = 0.35·lexical + 0.30·vector + 0.20·graph + 0.10·recency +
/// 0.05·importance`. They are constants here rather than literals at the call
/// site so the snapshot test and the `thresholds.toml` defaults cannot drift.
pub const WEIGHTS: [(crate::model::Channel, f64); 5] = [
    (crate::model::Channel::Lexical, 0.35),
    (crate::model::Channel::Vector, 0.30),
    (crate::model::Channel::Graph, 0.20),
    (crate::model::Channel::Recency, 0.10),
    (crate::model::Channel::Importance, 0.05),
];

/// The weight of one channel.
pub fn weight_of(channel: crate::model::Channel) -> f64 {
    WEIGHTS
        .iter()
        .find(|(c, _)| *c == channel)
        .map(|(_, w)| *w)
        .unwrap_or(0.0)
}

/// `impact × probability × reliability ÷ cost`.
///
/// Every input is validated. A probability above one, a negative reliability, and
/// a non-positive cost are all refused rather than clamped: a caller that passes
/// them has a bug, and a clamped score would hide it.
pub fn memory_utility(inputs: &UtilityInputs) -> Result<f64> {
    for (field, value) in [
        (
            "future_behavior_impact",
            f64::from(inputs.future_behavior_impact),
        ),
        (
            "retrieval_probability",
            f64::from(inputs.retrieval_probability),
        ),
        ("reliability", f64::from(inputs.reliability)),
    ] {
        if !(0.0..=1.0).contains(&value) {
            return Err(MemoryError::Utility(UtilityDenied {
                field: field.to_string(),
                reason: format!("must be in [0,1], got {value}"),
            }));
        }
    }
    if !inputs.storage_cost.is_finite() {
        return Err(MemoryError::Utility(UtilityDenied {
            field: "storage_cost".to_string(),
            reason: format!("must be finite, got {}", inputs.storage_cost),
        }));
    }
    if inputs.storage_cost <= 0.0 {
        return Err(MemoryError::Utility(UtilityDenied {
            field: "storage_cost".to_string(),
            reason: format!("must be > 0, got {}", inputs.storage_cost),
        }));
    }
    Ok(f64::from(inputs.future_behavior_impact)
        * f64::from(inputs.retrieval_probability)
        * f64::from(inputs.reliability)
        / inputs.storage_cost)
}

/// Ebbinghaus retention: `0.5^(age/half_life)`, lifted by importance and by
/// rehearsal.
///
/// The rehearsal term is `1 + 0.1·√access_count` rather than a linear count, so
/// rehearsal has diminishing returns and a memory cannot be kept alive forever by
/// being looked at often. The result is a *score*, not a probability: nothing
/// clamps it, because clamping is what would break strict monotonicity in the
/// access count.
pub fn retention_score(age_ns: u64, half_life_ns: u64, importance: f32, access_count: u32) -> f64 {
    let half_life = half_life_ns.max(1) as f64;
    let decay = 0.5f64.powf(age_ns as f64 / half_life);
    let importance_gain = 0.5 + 0.5 * f64::from(importance.clamp(0.0, 1.0));
    let rehearsal_gain = 1.0 + f64::from(access_count).sqrt() * 0.1;
    decay * importance_gain * rehearsal_gain
}

/// How prominently a memory should figure in the bounded core set.
///
/// Salience is retention, scaled by how much the memory is trusted and how much
/// it matters. The core tier is selected by this, so it uses the *current* clock
/// only through the caller's retention value.
pub fn salience(importance: f32, retention: f64, confidence: f32) -> f64 {
    f64::from(importance.clamp(0.0, 1.0))
        * retention.max(0.0)
        * (0.5 + 0.5 * f64::from(confidence.clamp(0.0, 1.0)))
}

/// Time decay for the recency channel: `0.5^(age/half_life)`, in `(0, 1]`.
pub fn recency_score(age_ns: u64, half_life_ns: u64) -> f64 {
    0.5f64.powf(age_ns as f64 / half_life_ns.max(1) as f64)
}

/// Map a raw FTS5 `bm25()` rank onto `[0,1]`.
///
/// SQLite's `bm25()` returns a *negative* number where more negative is a better
/// match, so the useful magnitude is `-rank`. The transform is `m/(1+m)`, which is
/// monotone and saturating: a better match always scores higher, and no single
/// candidate can dominate the sum by having an unbounded rank.
pub fn normalize_bm25(rank: f64) -> f64 {
    let magnitude = (-rank).max(0.0);
    magnitude / (1.0 + magnitude)
}

/// Map a cosine similarity in `[-1, 1]` onto `[0, 1]`.
pub fn normalize_cosine(cosine: f64) -> f64 {
    ((cosine.clamp(-1.0, 1.0)) + 1.0) / 2.0
}

/// Add the `(0.5 + 0.5·confidence)` gate to a hybrid score.
///
/// A memory the being does not trust should not rank as highly as one it does,
/// but it should never be erased either, so the multiplier floors at 0.5 rather
/// than at 0.
pub fn confidence_gate(confidence: f32) -> f64 {
    0.5 + 0.5 * f64::from(confidence.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Channel;

    fn inputs(impact: f32, probability: f32, reliability: f32, cost: f64) -> UtilityInputs {
        UtilityInputs {
            future_behavior_impact: impact,
            retrieval_probability: probability,
            reliability,
            storage_cost: cost,
        }
    }

    #[test]
    fn utility_matches_its_golden_values() {
        // The inputs are `f32`, so the comparison tolerance is a `f32` tolerance:
        // 0.8f32 is not 0.8 exactly, and pretending otherwise would make this test
        // pass only by accident.
        assert!((memory_utility(&inputs(0.8, 0.5, 0.9, 2.0)).unwrap() - 0.18).abs() < 1e-6);
        assert!((memory_utility(&inputs(1.0, 1.0, 1.0, 1.0)).unwrap() - 1.0).abs() < 1e-12);
        assert!((memory_utility(&inputs(0.6, 0.25, 0.5, 0.5)).unwrap() - 0.15).abs() < 1e-6);
        assert_eq!(memory_utility(&inputs(0.0, 1.0, 1.0, 3.0)).unwrap(), 0.0);
    }

    #[test]
    fn utility_rises_with_each_numerator_and_falls_with_cost() {
        let base = inputs(0.5, 0.5, 0.5, 2.0);
        let up = |i: UtilityInputs| memory_utility(&i).unwrap();
        assert!(up(inputs(0.9, 0.5, 0.5, 2.0)) > up(base));
        assert!(up(inputs(0.5, 0.9, 0.5, 2.0)) > up(base));
        assert!(up(inputs(0.5, 0.5, 0.9, 2.0)) > up(base));
        assert!(up(inputs(0.5, 0.5, 0.5, 1.0)) > up(base));
        assert!(up(inputs(0.5, 0.5, 0.5, 8.0)) < up(base));
    }

    #[test]
    fn a_zero_or_negative_cost_is_an_error_not_a_huge_number() {
        for cost in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let error = memory_utility(&inputs(1.0, 1.0, 1.0, cost)).unwrap_err();
            assert_eq!(error.code(), "memory.utility");
            match error {
                MemoryError::Utility(denied) => assert_eq!(denied.field, "storage_cost"),
                other => panic!("unexpected error {other:?}"),
            }
        }
    }

    #[test]
    fn an_out_of_range_probability_is_refused() {
        for (i, p, r) in [(1.5, 0.5, 0.5), (0.5, -0.1, 0.5), (0.5, 0.5, 2.0)] {
            assert!(memory_utility(&inputs(i, p, r, 1.0)).is_err());
        }
    }

    #[test]
    fn retention_decays_monotonically_with_age() {
        let mut previous = retention_score(0, WEEK_NS, 0.5, 0);
        for step in 1..=20u64 {
            let age = step * (WEEK_NS / 4);
            let current = retention_score(age, WEEK_NS, 0.5, 0);
            assert!(
                current < previous,
                "retention must fall: at {age} ns {current} !< {previous}"
            );
            previous = current;
        }
        // A full half-life is exactly half, before the importance modulation.
        assert!((retention_score(WEEK_NS, WEEK_NS, 0.0, 0) - 0.25).abs() < 1e-12);
    }

    #[test]
    fn retention_rises_with_importance_and_rehearsal() {
        let age = WEEK_NS / 2;
        assert!(retention_score(age, WEEK_NS, 0.9, 0) > retention_score(age, WEEK_NS, 0.1, 0));
        let mut previous = retention_score(age, WEEK_NS, 0.5, 0);
        for access in [1u32, 2, 3, 5, 8, 13] {
            let current = retention_score(age, WEEK_NS, 0.5, access);
            assert!(current > previous, "rehearsal {access} must lift retention");
            previous = current;
        }
    }

    #[test]
    fn a_zero_half_life_does_not_divide_by_zero() {
        // A half-life of zero means "forget immediately"; the guard turns that into
        // a one-nanosecond half-life rather than a division by zero.
        let score = retention_score(1, 0, 0.5, 0);
        assert!(score.is_finite());
        assert!(score > 0.0 && score < 1.0, "{score}");
        assert_eq!(retention_score(0, 0, 0.5, 0), 0.75);
    }

    #[test]
    fn salience_is_bounded_by_its_inputs() {
        assert_eq!(salience(0.0, 100.0, 1.0), 0.0);
        assert!(salience(1.0, 1.0, 1.0) > salience(1.0, 1.0, 0.0));
        // A negative retention cannot make a memory antisalient.
        assert_eq!(salience(1.0, -5.0, 1.0), 0.0);
    }

    #[test]
    fn the_channel_weights_are_the_ones_the_plan_pins() {
        assert_eq!(weight_of(Channel::Lexical), 0.35);
        assert_eq!(weight_of(Channel::Vector), 0.30);
        assert_eq!(weight_of(Channel::Graph), 0.20);
        assert_eq!(weight_of(Channel::Recency), 0.10);
        assert_eq!(weight_of(Channel::Importance), 0.05);
        let sum: f64 = crate::model::CHANNELS.into_iter().map(weight_of).sum();
        assert!(
            (sum - 1.0).abs() < 1e-12,
            "weights must sum to 1, got {sum}"
        );
    }

    #[test]
    fn the_normalizers_are_monotone_and_bounded() {
        assert!(normalize_bm25(-10.0) > normalize_bm25(-1.0));
        assert_eq!(normalize_bm25(5.0), 0.0, "a positive bm25 is not a match");
        assert!((normalize_cosine(1.0) - 1.0).abs() < 1e-12);
        assert!((normalize_cosine(0.0) - 0.5).abs() < 1e-12);
        assert!((normalize_cosine(-1.0) - 0.0).abs() < 1e-12);
        assert!((normalize_cosine(-9.0) - 0.0).abs() < 1e-12);
        assert_eq!(confidence_gate(0.0), 0.5);
        assert_eq!(confidence_gate(1.0), 1.0);
    }

    #[test]
    fn recency_is_one_at_zero_age() {
        assert_eq!(recency_score(0, WEEK_NS), 1.0);
        assert!((recency_score(WEEK_NS, WEEK_NS) - 0.5).abs() < 1e-12);
        assert!(recency_score(WEEK_NS * 2, WEEK_NS) < 0.5);
    }
}
