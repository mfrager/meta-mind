//! Utility and retention arithmetic (plan §7, first bullet).
//!
//! These two functions decide two different things, and both decisions are
//! irreversible in the sense that matters: `memory_utility` decides whether a
//! record is worth keeping, and `retention_score` decides whether it survives a
//! forget cycle. Neither may read a clock — a replay has to recompute the same
//! numbers the first run computed — so both take the instant they are measured at,
//! or nothing at all.
//!
//! What is proved here is therefore shape, not taste: the golden ratios, the
//! monotonicity the policy depends on, and the refusals. A zero `storage_cost` is
//! the interesting one, because an implementation that returned `0.0` or
//! `f64::INFINITY` for it would let a record with no cost look maximally valuable
//! and win every ranking.

use mm_memory::utility::{confidence_gate, weight_of};
use mm_memory::{
    memory_utility, normalize_bm25, normalize_cosine, recency_score, retention_score, salience,
    Channel, MemoryError, UtilityInputs, CHANNELS, MONTH_NS, WEEK_NS,
};

/// The plan's utility inputs, spelled out so each test can vary one of them.
fn inputs(impact: f32, probability: f32, reliability: f32, cost: f64) -> UtilityInputs {
    UtilityInputs {
        future_behavior_impact: impact,
        retrieval_probability: probability,
        reliability,
        storage_cost: cost,
    }
}

/// `0.8 × 0.5 × 0.9 ÷ 2.0` is exactly the plan's worked example. The inputs are
/// `f32`, so the comparison tolerance is an `f32` tolerance: `0.8f32` is not `0.8`,
/// and a tighter bound would pass only by accident.
#[test]
fn the_golden_ratio_is_the_plans_worked_example() {
    assert!((memory_utility(&inputs(0.8, 0.5, 0.9, 2.0)).unwrap() - 0.18).abs() < 1e-6);
}

/// Three more golden points, each chosen so the arithmetic is checkable by hand:
/// a trivial `1.0`, a non-unit cost, and a zero numerator.
#[test]
fn more_golden_values_are_checkable_by_hand() {
    assert!((memory_utility(&inputs(1.0, 1.0, 1.0, 1.0)).unwrap() - 1.0).abs() < 1e-12);
    assert!((memory_utility(&inputs(0.6, 0.25, 0.5, 0.5)).unwrap() - 0.15).abs() < 1e-6);
    // A record with no impact is worth nothing however reliable it is, and that is
    // a plain zero rather than a refusal.
    assert_eq!(memory_utility(&inputs(0.0, 1.0, 1.0, 3.0)).unwrap(), 0.0);
}

/// Each numerator lifts the utility and the denominator lowers it. If any of these
/// five comparisons ever flips, the policy that keeps or drops a record has
/// silently changed meaning.
#[test]
fn utility_is_monotone_in_every_input() {
    let base = inputs(0.5, 0.5, 0.5, 2.0);
    let u = |i: UtilityInputs| memory_utility(&i).unwrap();

    assert!(u(inputs(0.9, 0.5, 0.5, 2.0)) > u(base), "impact must lift");
    assert!(
        u(inputs(0.5, 0.9, 0.5, 2.0)) > u(base),
        "probability must lift"
    );
    assert!(
        u(inputs(0.5, 0.5, 0.9, 2.0)) > u(base),
        "reliability must lift"
    );
    assert!(
        u(inputs(0.5, 0.5, 0.5, 1.0)) > u(base),
        "a lower cost must lift"
    );
    assert!(
        u(inputs(0.5, 0.5, 0.5, 8.0)) < u(base),
        "a higher cost must lower"
    );
}

/// The divisor is not optional. A zero, a negative, and a non-finite cost are all
/// refused, and the refusal names the field so a caller can fix it.
#[test]
fn a_non_positive_cost_is_refused_and_names_the_field() {
    for cost in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = memory_utility(&inputs(1.0, 1.0, 1.0, cost)).unwrap_err();
        assert_eq!(error.code(), "memory.utility", "cost {cost}");
        match error {
            MemoryError::Utility(denied) => {
                assert_eq!(denied.field, "storage_cost", "cost {cost}");
                assert!(!denied.reason.is_empty());
            }
            other => panic!("cost {cost}: unexpected error {other:?}"),
        }
    }
}

/// An out-of-range numerator is refused rather than clamped: a clamped score hides
/// the caller's bug behind a plausible number.
#[test]
fn an_out_of_range_numerator_is_refused_with_its_own_field() {
    for (impact, probability, reliability, field) in [
        (1.5f32, 0.5f32, 0.5f32, "future_behavior_impact"),
        (-0.1, 0.5, 0.5, "future_behavior_impact"),
        (0.5, 1.5, 0.5, "retrieval_probability"),
        (0.5, -0.1, 0.5, "retrieval_probability"),
        (0.5, 0.5, 2.0, "reliability"),
        (0.5, 0.5, -0.01, "reliability"),
    ] {
        match memory_utility(&inputs(impact, probability, reliability, 1.0)) {
            Err(MemoryError::Utility(denied)) => assert_eq!(denied.field, field),
            other => panic!("{field}: expected a utility refusal, got {other:?}"),
        }
    }
    // The boundary itself is allowed: `1.0` is in range.
    assert!(memory_utility(&inputs(1.0, 1.0, 1.0, 1.0)).is_ok());
    assert!(memory_utility(&inputs(0.0, 0.0, 0.0, 1.0)).is_ok());
}

/// Retention falls strictly with age. A record that decayed to the same score at
/// every age would never cross a threshold, and forgetting would never happen.
#[test]
fn retention_decays_strictly_with_age() {
    let mut previous = retention_score(0, WEEK_NS, 0.5, 0);
    for step in 1..=40u64 {
        let age = step * (WEEK_NS / 8);
        let current = retention_score(age, WEEK_NS, 0.5, 0);
        assert!(
            current < previous,
            "retention must fall: at {age} ns {current} !< {previous}"
        );
        previous = current;
    }
    // One half-life is one half, before the importance modulation.
    assert!((retention_score(WEEK_NS, WEEK_NS, 0.0, 0) - 0.25).abs() < 1e-12);
    assert!((retention_score(MONTH_NS, MONTH_NS, 0.0, 0) - 0.25).abs() < 1e-12);
}

/// Retention rises with both importance and rehearsal, and rises strictly in the
/// access count across a wide range — which is what makes reinforcement meaningful.
#[test]
fn retention_rises_with_importance_and_rehearsal() {
    let age = WEEK_NS / 2;
    assert!(
        retention_score(age, WEEK_NS, 0.9, 0) > retention_score(age, WEEK_NS, 0.1, 0),
        "importance must lift retention"
    );

    let mut previous = retention_score(age, WEEK_NS, 0.5, 0);
    for access in [1u32, 2, 3, 5, 8, 13, 21, 34] {
        let current = retention_score(age, WEEK_NS, 0.5, access);
        assert!(
            current > previous,
            "rehearsal {access} must lift retention: {current} !> {previous}"
        );
        previous = current;
    }
    // Rehearsal has diminishing returns: the second access is worth less than the
    // first, so a memory cannot be kept alive indefinitely by being poked.
    let first = retention_score(age, WEEK_NS, 0.5, 1) - retention_score(age, WEEK_NS, 0.5, 0);
    let second = retention_score(age, WEEK_NS, 0.5, 2) - retention_score(age, WEEK_NS, 0.5, 1);
    assert!(second < first, "rehearsal must have diminishing returns");
}

/// A zero half-life is a configuration mistake, not a division by zero. The guard
/// turns it into a one-nanosecond half-life.
#[test]
fn a_zero_half_life_does_not_divide_by_zero() {
    let score = retention_score(1, 0, 0.5, 0);
    assert!(score.is_finite(), "{score}");
    assert!(score > 0.0 && score < 1.0, "{score}");
    assert_eq!(retention_score(0, 0, 0.5, 0), 0.75);
}

/// Salience is zero for a memory worth nothing and never negative, so the bounded
/// core set can never be filled by a record that is antisalient.
#[test]
fn salience_is_zero_at_zero_and_never_negative() {
    assert_eq!(salience(0.0, 100.0, 1.0), 0.0, "no importance, no salience");
    assert_eq!(salience(1.0, -5.0, 1.0), 0.0, "negative retention clamps");
    assert!(
        salience(1.0, 1.0, 1.0) > salience(1.0, 1.0, 0.0),
        "confidence must lift salience"
    );
    for (importance, retention, confidence) in
        [(0.2f32, 0.1f64, 0.0f32), (0.5, 0.5, 0.5), (1.0, 2.0, 1.0)]
    {
        assert!(salience(importance, retention, confidence) >= 0.0);
    }
}

/// The channel weights are exactly the ones the plan pins, and they sum to one.
/// A retrieval whose weights do not sum to one is a retrieval whose scores are not
/// comparable across runs, which is what the snapshot test depends on.
#[test]
fn the_channel_weights_are_the_ones_the_plan_pins() {
    assert_eq!(weight_of(Channel::Lexical), 0.35);
    assert_eq!(weight_of(Channel::Vector), 0.30);
    assert_eq!(weight_of(Channel::Graph), 0.20);
    assert_eq!(weight_of(Channel::Recency), 0.10);
    assert_eq!(weight_of(Channel::Importance), 0.05);

    let sum: f64 = CHANNELS.into_iter().map(weight_of).sum();
    assert!(
        (sum - 1.0).abs() < 1e-12,
        "weights must sum to 1, got {sum}"
    );
    // Every channel the enum names has a weight; none is silently dropped.
    assert_eq!(CHANNELS.len(), 5);
}

/// The normalizers are monotone and bounded, which is what keeps one channel from
/// dominating the sum by having a larger raw range than the others.
#[test]
fn the_normalizers_are_monotone_and_bounded() {
    // More negative is a better BM25 match, and a positive rank is not a match.
    assert!(normalize_bm25(-10.0) > normalize_bm25(-1.0));
    assert_eq!(normalize_bm25(5.0), 0.0, "a positive bm25 is not a match");
    assert_eq!(normalize_bm25(0.0), 0.0);
    for rank in [-1e6, -100.0, -3.0, -0.5] {
        let value = normalize_bm25(rank);
        assert!((0.0..=1.0).contains(&value), "{rank} -> {value}");
    }

    assert!((normalize_cosine(1.0) - 1.0).abs() < 1e-12);
    assert!((normalize_cosine(0.0) - 0.5).abs() < 1e-12);
    assert!((normalize_cosine(-1.0) - 0.0).abs() < 1e-12);
    assert!((normalize_cosine(-9.0) - 0.0).abs() < 1e-12, "clamps to 0");
    assert!((normalize_cosine(9.0) - 1.0).abs() < 1e-12, "clamps to 1");
}

/// The confidence gate never erases a memory: an untrusted record ranks at half,
/// not at zero.
#[test]
fn the_confidence_gate_floors_at_one_half() {
    assert_eq!(confidence_gate(0.0), 0.5);
    assert_eq!(confidence_gate(1.0), 1.0);
    assert!((confidence_gate(0.5) - 0.75).abs() < 1e-12);
    assert_eq!(confidence_gate(-1.0), 0.5, "clamps");
    assert_eq!(confidence_gate(2.0), 1.0, "clamps");
}

/// Recency is one at zero age and half at one half-life, so the recency channel's
/// meaning is fixed by the same curve retention uses.
#[test]
fn recency_is_one_at_zero_age_and_half_a_half_life() {
    assert_eq!(recency_score(0, WEEK_NS), 1.0);
    assert!((recency_score(WEEK_NS, WEEK_NS) - 0.5).abs() < 1e-12);
    assert!(recency_score(WEEK_NS * 2, WEEK_NS) < 0.5);
    assert!(recency_score(WEEK_NS * 4, WEEK_NS) < recency_score(WEEK_NS * 2, WEEK_NS));
}
