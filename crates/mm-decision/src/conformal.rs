//! Split-conformal abstention: turning a calibrated confidence into a threshold.
//!
//! Phase 9 §4.2 gives the decision primitive a way to *decline*. A calibrated
//! confidence is still just a number; what makes it actionable is a threshold with
//! a stated coverage — "admit this answer only when answers at least this
//! unexpected were right `target` of the time". That is the split-conformal
//! quantity, borrowed from MAPIE, and it is what the firewall's `VERIFY_FIRST` and
//! `ASK_USER` cutoffs are stored as.
//!
//! Three things are deliberate here:
//!
//! * **The rank, not the percentile.** The threshold is the
//!   `ceil((n + 1) · coverage)`-th smallest score, which is what gives *marginal*
//!   coverage of at least `target` under exchangeability. The naive
//!   `floor(n · coverage)` is one rank too low and quietly under-covers on small
//!   sets — precisely the case a firewall meets.
//! * **The nonconformity score is `1 − p`.** One direction throughout, so
//!   [`ConformalSet::admit`] is a single comparison and nothing has to remember
//!   which way round it is.
//! * **An empty set admits nothing.** `q_hat = 0` would admit any confidence of 1
//!   and any score of 0, which is the "default yes" a firewall must never have. No
//!   threshold means no admission, and the caller escalates.
//!
//! The empirical coverage a fit reports *in sample* is true by construction and
//! therefore worth very little; [`fit_held_out`] is the honest number, and it is
//! what the calibration gate measures.

use serde::{Deserialize, Serialize};

/// The split-conformal quantile of a set of nonconformity scores.
///
/// With `coverage` clamped to `[0,1]` and `n = scores.len()`, this is the
/// `ceil((n + 1) · coverage)`-th smallest score, 1-indexed, saturating at `n`.
/// `None` when there are no scores: there is no threshold to be had from an empty
/// calibration set.
pub fn split_conformal_quantile(scores: &[f32], coverage: f32) -> Option<f32> {
    let n = scores.len();
    if n == 0 {
        return None;
    }
    let coverage = f64::from(coverage).clamp(0.0, 1.0);
    let rank = ((n + 1) as f64 * coverage).ceil() as usize;
    let rank = rank.clamp(1, n);
    let mut ordered: Vec<f32> = scores.to_vec();
    ordered.sort_by(|a, b| a.total_cmp(b));
    ordered.get(rank - 1).copied()
}

/// The fraction of `scores` at or below `q_hat`.
pub fn empirical_coverage(scores: &[f32], q_hat: f32) -> f32 {
    if scores.is_empty() {
        return 0.0;
    }
    let covered = scores
        .iter()
        .filter(|score| score.total_cmp(&q_hat) != std::cmp::Ordering::Greater)
        .count();
    (covered as f64 / scores.len() as f64) as f32
}

/// A per-class admission threshold and the coverage it was fitted for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConformalSet {
    /// The decision class this threshold belongs to.
    pub decision_class: String,
    /// The fitted nonconformity threshold. A prediction is admitted when its
    /// nonconformity is at or below this.
    pub q_hat: f32,
    /// The coverage the threshold was fitted for.
    pub coverage: f32,
    /// How many scores stood behind the threshold.
    pub n: usize,
    /// The coverage `q_hat` achieved on the scores it was fitted from.
    pub empirical_coverage: f32,
}

impl ConformalSet {
    /// Fit a threshold for a decision class.
    ///
    /// `empirical_coverage` is measured on the same scores, which makes it true by
    /// construction — it is reported because a caller storing a threshold wants to
    /// see the set size next to it, not because it is evidence. Use
    /// [`fit_held_out`] for evidence.
    pub fn fit(decision_class: impl Into<String>, scores: &[f32], target: f32) -> Self {
        let coverage = f64::from(target).clamp(0.0, 1.0) as f32;
        let decision_class = decision_class.into();
        match split_conformal_quantile(scores, coverage) {
            None => ConformalSet {
                decision_class,
                // No threshold: no confidence is ever admitted, and the caller
                // escalates rather than receiving a default yes.
                q_hat: 0.0,
                coverage,
                n: 0,
                empirical_coverage: 0.0,
            },
            Some(q_hat) => ConformalSet {
                decision_class,
                q_hat,
                coverage,
                n: scores.len(),
                empirical_coverage: empirical_coverage(scores, q_hat),
            },
        }
    }

    /// The nonconformity score of a calibrated confidence: `1 − p`, clamped to
    /// `[0,1]`.
    pub fn nonconformity(p: f32) -> f32 {
        (1.0 - p).clamp(0.0, 1.0)
    }

    /// Whether a calibrated confidence clears the threshold.
    ///
    /// Equivalently `p >= 1 - q_hat`. An unfitted set (`n == 0`) admits nothing.
    pub fn admit(&self, calibrated_p: f32) -> bool {
        if self.n == 0 {
            return false;
        }
        ConformalSet::nonconformity(calibrated_p) <= self.q_hat
    }

    /// A fixed rendering, so a stored threshold compares as a string.
    pub fn canonical(&self) -> String {
        format!(
            "class={},q_hat={:.9},coverage={:.9},n={},empirical_coverage={:.9}",
            self.decision_class, self.q_hat, self.coverage, self.n, self.empirical_coverage
        )
    }
}

/// Fit on half the scores and report the coverage the threshold achieves on the
/// other half.
///
/// This is the number the phase plan's calibration gate checks ("empirical coverage
/// within ±0.03 of target"), because in-sample coverage is guaranteed rather than
/// measured. The split is a fixed even/odd interleave so the result is a pure
/// function of the input — a random split would make the gate flaky, which is
/// exactly the property a gate must not have.
///
/// With fewer than four scores, either half would be too small to say anything, so
/// the fit and the measurement both use the whole set and the returned coverage is
/// the in-sample one. Four is the smallest size at which both halves have at least
/// two points.
pub fn fit_held_out(
    decision_class: impl Into<String>,
    scores: &[f32],
    target: f32,
) -> (ConformalSet, f32) {
    if scores.len() < 4 {
        let set = ConformalSet::fit(decision_class, scores, target);
        let in_sample = set.empirical_coverage;
        return (set, in_sample);
    }
    let fit_at: Vec<f32> = scores.iter().step_by(2).copied().collect();
    let held_out: Vec<f32> = scores.iter().skip(1).step_by(2).copied().collect();
    let set = ConformalSet::fit(decision_class, &fit_at, target);
    let coverage = empirical_coverage(&held_out, set.q_hat);
    (set, coverage)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic linear congruential generator, so the determinism
    /// test is reproducible rather than merely likely.
    struct Lcg(u64);

    impl Lcg {
        fn next_unit(&mut self) -> f32 {
            // Numerical Recipes' constants; the low bits are ignored because an
            // LCG's low bits are the least random.
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) as f64 / (1u64 << 31) as f64) as f32
        }
    }

    fn lcg_scores(seed: u64, n: usize) -> Vec<f32> {
        let mut rng = Lcg(seed);
        (0..n).map(|_| rng.next_unit()).collect()
    }

    /// A low-discrepancy schedule: the fractional parts of `i · φ`.
    ///
    /// The held-out coverage assertion is about the *estimator*, not about a
    /// generator's luck, and a low-discrepancy schedule is equidistributed in both
    /// halves, so the measured coverage differs from the target by the sampling
    /// grid rather than by chance. A pseudo-random schedule would make a ±0.03
    /// assertion a coin flip dressed as a test.
    fn equidistributed_scores(n: usize) -> Vec<f32> {
        const GOLDEN: f64 = 0.618_033_988_749_894_9;
        (1..=n)
            .map(|i| ((i as f64 * GOLDEN).fract()) as f32)
            .collect()
    }

    #[test]
    fn the_quantile_is_the_conformal_order_statistic() {
        let scores = [0.1_f32, 0.2, 0.3, 0.4];
        // n = 4, coverage 0.75 -> ceil(5 * 0.75) = ceil(3.75) = 4 -> the 4th
        // smallest. floor(n * coverage) = 3 would under-cover, which is the whole
        // reason the rank is written this way.
        assert_eq!(split_conformal_quantile(&scores, 0.75), Some(0.4));

        // The rank saturates at n instead of running past the end.
        assert_eq!(split_conformal_quantile(&scores, 1.0), Some(0.4));
        // Coverage 0 reads the smallest score.
        assert_eq!(split_conformal_quantile(&scores, 0.0), Some(0.1));
        assert_eq!(split_conformal_quantile(&[], 0.9), None);
    }

    #[test]
    fn held_out_coverage_lands_within_tolerance_of_the_target() {
        let scores = equidistributed_scores(1000);
        for target in [0.8_f32, 0.9, 0.95] {
            let (set, coverage) = fit_held_out("safety", &scores, target);
            assert_eq!(set.n, 500);
            assert!(
                (coverage - target).abs() < 0.03,
                "target {target} measured {coverage} (q_hat {})",
                set.q_hat
            );
        }
    }

    #[test]
    fn held_out_coverage_is_deterministic() {
        let scores = lcg_scores(7, 400);
        let a = fit_held_out("safety", &scores, 0.9);
        let b = fit_held_out("safety", &scores, 0.9);
        assert_eq!(a, b);
        // A pseudo-random set from a fixed seed is also a fixed set, so the
        // estimator cannot drift between runs.
        assert_eq!(lcg_scores(7, 400), scores);
    }

    #[test]
    fn admission_is_monotone_in_confidence() {
        let set = ConformalSet::fit("safety", &[0.1_f32, 0.2, 0.3, 0.4], 0.75);
        // q_hat = 0.4, so a confidence of 0.6 (nonconformity 0.4) is admitted and
        // anything lower is not.
        assert!(set.admit(1.0));
        assert!(set.admit(0.6));
        assert!(!set.admit(0.5));
        assert!(!set.admit(0.0));
        for p in [0.0_f32, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0] {
            if set.admit(p) {
                assert!(
                    set.admit((p + 0.1).min(1.0)),
                    "admitting {p} must admit a higher confidence"
                );
            }
        }
    }

    #[test]
    fn an_unfitted_set_admits_nothing() {
        let set = ConformalSet::fit("safety", &[], 0.9);
        assert_eq!(set.n, 0);
        assert!(!set.admit(1.0));
        assert!(!set.admit(0.5));
        assert_eq!(set.empirical_coverage, 0.0);
        assert!((set.coverage - 0.9).abs() < 1e-6);
    }

    #[test]
    fn a_small_set_falls_back_to_in_sample_coverage() {
        let scores = [0.1_f32, 0.5, 0.9];
        let (set, coverage) = fit_held_out("safety", &scores, 0.9);
        assert_eq!(set.n, 3);
        assert_eq!(coverage, set.empirical_coverage);
        assert_eq!(split_conformal_quantile(&scores, 0.9), Some(0.9));
    }

    #[test]
    fn empirical_coverage_counts_the_boundary() {
        let scores = [0.1_f32, 0.2, 0.3, 0.4];
        assert!((empirical_coverage(&scores, 0.4) - 1.0).abs() < 1e-9);
        assert!((empirical_coverage(&scores, 0.25) - 0.5).abs() < 1e-9);
        assert_eq!(empirical_coverage(&[], 0.5), 0.0);
    }

    #[test]
    fn the_nonconformity_score_is_the_complement() {
        assert!((ConformalSet::nonconformity(0.9) - 0.1).abs() < 1e-6);
        assert!((ConformalSet::nonconformity(0.5) - 0.5).abs() < 1e-6);
        // Out-of-range confidences clamp rather than inverting.
        assert_eq!(ConformalSet::nonconformity(1.5), 0.0);
        assert_eq!(ConformalSet::nonconformity(-0.5), 1.0);
    }

    #[test]
    fn a_set_renders_deterministically() {
        let set = ConformalSet::fit("safety", &[0.2_f32, 0.4, 0.6, 0.8], 0.75);
        assert_eq!(set.canonical(), set.canonical());
        assert!(set.canonical().contains("class=safety"));
        assert!((set.q_hat - 0.8).abs() < 1e-6);
    }
}
