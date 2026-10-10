//! Calibration: is a core's confidence the frequency it claims to be?
//!
//! Phase 9 §4.2 makes calibration *mandatory*: a class with no calibration data
//! uses `temperature = 1.0` and must be reported as uncalibrated, and the firewall
//! escalates an uncalibrated high-confidence answer to `VERIFY_FIRST` rather than
//! `PROCEED`. That rule only has teeth if the fitted quantity is trustworthy and
//! the metrics are honest, which is what this module is for.
//!
//! Three decisions are worth stating because they are not visible in the shape of
//! the code:
//!
//! * **The fit optimizes the metric the gate checks.** Temperature scaling is
//!   conventionally fitted by minimizing log loss, but the plan's gate is "ECE
//!   improves on the labeled set after fitting". A log-loss optimum can *raise*
//!   ECE, which would make a green gate and a red operator report out of step. So
//!   the grid search minimizes ECE directly, with `T = 1.0` among the candidates —
//!   which is what makes [`Calibrator::fit`] unable to do worse than doing nothing.
//! * **Too little data means no temperature, not a small one.** Fewer than
//!   [`MIN_FIT_POINTS`] points yields `T = 1.0` and `n = 0`, i.e. *uncalibrated*.
//!   A temperature fitted on a handful of points is noise dressed as a correction,
//!   and `n = 0` is what lets the firewall see that and escalate.
//! * **The metrics are computed on the set they are reported for.** `ece_before`
//!   and `ece` are both on the input set, so the pair is directly comparable and a
//!   test can assert the direction.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// How many points a temperature needs before it is fitted at all.
pub const MIN_FIT_POINTS: usize = 8;

/// How many bins the reliability curve and ECE use.
pub const DEFAULT_BINS: usize = 10;

/// How many temperatures the grid search tries, plus `1.0` itself.
const GRID_POINTS: usize = 2000;
/// The lowest temperature on the grid. Below this a probability is driven to a
/// hard 0 or 1 and the calibration stops being a probability.
const MIN_TEMPERATURE: f64 = 0.05;
/// The highest temperature on the grid. Above this every answer is flattened to
/// 0.5, which is the same as answering nothing.
const MAX_TEMPERATURE: f64 = 20.0;

/// The logistic function.
pub fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        let z = (-x).exp();
        z.mul_add(1.0, 1.0).recip()
    } else {
        let z = x.exp();
        z / (1.0 + z)
    }
}

/// The inverse of [`sigmoid`], clamped away from the poles.
///
/// `logit(0)` and `logit(1)` are infinite, and an infinite logit survives the
/// division by temperature as an infinity — so a confidence of exactly 0 or 1
/// would come back as `NaN`. Clamping to `1e-6` keeps a confident answer confident
/// while keeping every value a number.
pub fn logit(p: f64) -> f64 {
    let p = p.clamp(1e-6, 1.0 - 1e-6);
    (p / (1.0 - p)).ln()
}

/// The Brier score: the mean squared error of a probability against its label.
/// Lower is better; 0 is perfect.
pub fn brier_score(predicted: &[f32], labels: &[bool]) -> f32 {
    let n = predicted.len().min(labels.len());
    if n == 0 {
        return 0.0;
    }
    let total: f64 = (0..n)
        .map(|i| {
            let target = if labels[i] { 1.0 } else { 0.0 };
            let error = f64::from(predicted[i]) - target;
            error * error
        })
        .sum();
    (total / n as f64) as f32
}

/// The log loss (negative log likelihood): the mean `-log p(observed)`.
pub fn log_loss(predicted: &[f32], labels: &[bool]) -> f32 {
    let n = predicted.len().min(labels.len());
    if n == 0 {
        return 0.0;
    }
    let total: f64 = (0..n)
        .map(|i| {
            let p = f64::from(predicted[i]).clamp(1e-9, 1.0 - 1e-9);
            if labels[i] {
                -p.ln()
            } else {
                -(1.0 - p).ln()
            }
        })
        .sum();
    (total / n as f64) as f32
}

/// The reliability curve: one `(mean confidence, observed frequency, count)` per
/// non-empty bin, in bin order.
///
/// Empty bins are omitted rather than reported as zero, because a bin nobody
/// landed in says nothing about calibration and a zeroed row would look like a
/// measurement.
pub fn reliability_curve(
    predicted: &[f32],
    labels: &[bool],
    bins: usize,
) -> Vec<(f32, f32, usize)> {
    let bins = bins.max(1);
    let n = predicted.len().min(labels.len());
    let mut counts = vec![0usize; bins];
    let mut sums = vec![0.0f64; bins];
    let mut positives = vec![0usize; bins];
    for i in 0..n {
        let p = f64::from(predicted[i]).clamp(0.0, 1.0);
        // The last bin is right-inclusive so p == 1.0 has a home.
        let mut index = (p * bins as f64).floor() as usize;
        if index >= bins {
            index = bins - 1;
        }
        counts[index] += 1;
        sums[index] += p;
        if labels[i] {
            positives[index] += 1;
        }
    }
    let mut curve = Vec::new();
    for index in 0..bins {
        if counts[index] == 0 {
            continue;
        }
        let mean = sums[index] / counts[index] as f64;
        let observed = positives[index] as f64 / counts[index] as f64;
        curve.push((mean as f32, observed as f32, counts[index]));
    }
    curve
}

/// Expected calibration error: the count-weighted mean gap between what the
/// model said and what happened, over the bins of the reliability curve.
pub fn expected_calibration_error(predicted: &[f32], labels: &[bool], bins: usize) -> f32 {
    let n = predicted.len().min(labels.len());
    if n == 0 {
        return 0.0;
    }
    let total: f64 = reliability_curve(predicted, labels, bins)
        .into_iter()
        .map(|(mean, observed, count)| {
            (count as f64 / n as f64) * (f64::from(observed) - f64::from(mean)).abs()
        })
        .sum();
    total as f32
}

/// One decision class's temperature, its metrics, and how much data stood behind
/// it.
///
/// `n == 0` is the *uncalibrated* state and is not a degenerate corner: it is the
/// signal the firewall reads to escalate, so it is spelled out in
/// [`Calibrator::is_calibrated`] rather than inferred from the temperature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibrator {
    /// The decision class this temperature belongs to.
    pub decision_class: String,
    /// The fitted temperature. `1.0` means "no scaling".
    pub temperature: f32,
    /// The ECE after calibration.
    pub ece: f32,
    /// The Brier score after calibration.
    pub brier: f32,
    /// How many points the temperature was fitted on. 0 = uncalibrated.
    pub n: usize,
    /// The ECE before calibration, so the improvement is visible.
    pub ece_before: f32,
    /// The Brier score before calibration.
    pub brier_before: f32,
}

impl Calibrator {
    /// A class with no calibration data: no scaling, and marked as such.
    pub fn uncalibrated(decision_class: impl Into<String>) -> Self {
        Calibrator {
            decision_class: decision_class.into(),
            temperature: 1.0,
            ece: 0.0,
            brier: 0.0,
            n: 0,
            ece_before: 0.0,
            brier_before: 0.0,
        }
    }

    /// True when a temperature was actually fitted.
    pub fn is_calibrated(&self) -> bool {
        self.n > 0
    }

    /// Fit one temperature for a decision class.
    ///
    /// The objective is the ECE *after* applying the candidate temperature, and
    /// `1.0` is one of the candidates. That is deliberate: minimizing log loss
    /// could leave the gate's ECE check worse than not fitting at all, and a
    /// calibration that can regress is worse than none, because it looks like
    /// work. With `1.0` in the running, `ece <= ece_before` holds by construction.
    ///
    /// Ties break toward `T = 1.0`, and the grid is scanned in increasing order,
    /// so the fit is a pure function of its input and never drifts between runs.
    pub fn fit(decision_class: impl Into<String>, predicted: &[f32], labels: &[bool]) -> Self {
        let decision_class = decision_class.into();
        let n = predicted.len().min(labels.len());
        let ece_before = expected_calibration_error(predicted, labels, DEFAULT_BINS);
        let brier_before = brier_score(predicted, labels);

        if n < MIN_FIT_POINTS {
            // Too little data for a temperature to mean anything. The raw metrics
            // are still reported so an operator can see how little there was.
            return Calibrator {
                decision_class,
                temperature: 1.0,
                ece: ece_before,
                brier: brier_before,
                n: 0,
                ece_before,
                brier_before,
            };
        }

        let mut best_temperature = 1.0_f64;
        let mut best_ece = f64::from(ece_before);
        for candidate in temperature_grid() {
            let scaled: Vec<f32> = predicted
                .iter()
                .map(|p| calibrate_at(f64::from(*p), candidate) as f32)
                .collect();
            let ece = f64::from(expected_calibration_error(&scaled, labels, DEFAULT_BINS));
            let better = ece < best_ece - 1e-12;
            let tied_closer_to_one = (ece - best_ece).abs() <= 1e-12
                && (candidate - 1.0).abs() < (best_temperature - 1.0).abs();
            if better || tied_closer_to_one {
                best_ece = ece;
                best_temperature = candidate;
            }
        }

        let calibrated: Vec<f32> = predicted
            .iter()
            .map(|p| calibrate_at(f64::from(*p), best_temperature) as f32)
            .collect();

        Calibrator {
            decision_class,
            temperature: best_temperature as f32,
            ece: expected_calibration_error(&calibrated, labels, DEFAULT_BINS),
            brier: brier_score(&calibrated, labels),
            n,
            ece_before,
            brier_before,
        }
    }

    /// Apply the fitted temperature: `sigmoid(logit(p) / T)`.
    pub fn calibrate(&self, p: f32) -> f32 {
        calibrate_at(f64::from(p), f64::from(self.temperature)) as f32
    }

    /// Apply the temperature to a whole batch.
    pub fn calibrate_all(&self, predicted: &[f32]) -> Vec<f32> {
        predicted.iter().map(|p| self.calibrate(*p)).collect()
    }

    /// A fixed rendering, so a stored calibrator compares as a string.
    pub fn canonical(&self) -> String {
        format!(
            "class={},temperature={:.9},ece={:.9},ece_before={:.9},\
             brier={:.9},brier_before={:.9},n={},calibrated={}",
            self.decision_class,
            self.temperature,
            self.ece,
            self.ece_before,
            self.brier,
            self.brier_before,
            self.n,
            self.is_calibrated()
        )
    }
}

/// The candidate temperatures: `1.0` plus `GRID_POINTS` log-spaced values.
fn temperature_grid() -> Vec<f64> {
    let mut grid: Vec<f64> = (0..GRID_POINTS)
        .map(|i| {
            let t = i as f64 / (GRID_POINTS - 1) as f64;
            MIN_TEMPERATURE * (MAX_TEMPERATURE / MIN_TEMPERATURE).powf(t)
        })
        .collect();
    grid.push(1.0);
    grid
}

/// The scaling itself, shared by [`Calibrator::fit`]'s search and
/// [`Calibrator::calibrate`] so the two cannot disagree about the formula.
fn calibrate_at(p: f64, temperature: f64) -> f64 {
    if !temperature.is_finite() || temperature <= 0.0 || (temperature - 1.0).abs() < f64::EPSILON {
        return p.clamp(0.0, 1.0);
    }
    sigmoid(logit(p) / temperature)
}

/// One calibrator per decision class.
///
/// A map keyed by class rather than a list, because a lookup is by class and an
/// ordered map makes iteration — and therefore every report built from it —
/// deterministic.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CalibratorSet(BTreeMap<String, Calibrator>);

impl CalibratorSet {
    /// An empty set.
    pub fn new() -> Self {
        CalibratorSet(BTreeMap::new())
    }

    /// Add or replace the calibrator for its class.
    pub fn insert(&mut self, calibrator: Calibrator) {
        self.0.insert(calibrator.decision_class.clone(), calibrator);
    }

    /// The calibrator for a class, if one was fitted.
    pub fn get(&self, class: &str) -> Option<&Calibrator> {
        self.0.get(class)
    }

    /// Every class in the set, sorted.
    pub fn classes(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }

    /// How many classes are in the set.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when the set is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An overconfident set: the model says 0.95 whenever it is right far less
    /// often than that.
    fn overconfident() -> (Vec<f32>, Vec<bool>) {
        let mut predicted = Vec::new();
        let mut labels = Vec::new();
        for i in 0..100 {
            predicted.push(0.95);
            // Right about 60% of the time.
            labels.push(i % 5 != 0 && i % 5 != 1);
        }
        (predicted, labels)
    }

    #[test]
    fn the_metrics_match_hand_computation() {
        let predicted = [1.0_f32, 0.0, 0.5, 0.5];
        let labels = [true, false, true, false];
        // (0^2 + 0^2 + 0.5^2 + 0.5^2) / 4
        assert!((brier_score(&predicted, &labels) - 0.125).abs() < 1e-6);
        // log loss with the p = 0 and p = 1 poles clamped.
        let loss = log_loss(&[0.9_f32, 0.1], &[true, false]);
        let expected = (-(0.9_f64.ln()) - (0.9_f64.ln())) / 2.0;
        assert!((f64::from(loss) - expected).abs() < 1e-6);

        // Half the mass at 0.1 (all wrong) and half at 0.9 (all right):
        // 0.45 from the low bin plus 0.05 from the high one.
        let ece = expected_calibration_error(&[0.1, 0.1, 0.9, 0.9], &[true, true, true, true], 10);
        assert!((ece - 0.5).abs() < 1e-6, "{ece}");
    }

    #[test]
    fn a_perfectly_calibrated_set_has_a_small_error() {
        // Bin 0.55 holds 55% positives, bin 0.85 holds 85%.
        let mut predicted = Vec::new();
        let mut labels = Vec::new();
        for _ in 0..55 {
            predicted.push(0.55);
            labels.push(true);
        }
        for _ in 0..45 {
            predicted.push(0.55);
            labels.push(false);
        }
        for _ in 0..85 {
            predicted.push(0.85);
            labels.push(true);
        }
        for _ in 0..15 {
            predicted.push(0.85);
            labels.push(false);
        }
        let ece = expected_calibration_error(&predicted, &labels, 10);
        assert!(ece < 0.02, "a calibrated set should read near zero: {ece}");
    }

    #[test]
    fn fitting_never_makes_calibration_worse() {
        let (predicted, labels) = overconfident();
        let calibrator = Calibrator::fit("safety", &predicted, &labels);
        assert!(calibrator.is_calibrated());
        assert_eq!(calibrator.n, 100);
        assert!(
            calibrator.ece <= calibrator.ece_before + 1e-9,
            "ece {} must not exceed ece_before {}",
            calibrator.ece,
            calibrator.ece_before
        );
        assert!(
            calibrator.ece < calibrator.ece_before,
            "an overconfident set must actually improve: {} vs {}",
            calibrator.ece,
            calibrator.ece_before
        );
        // Softening an overconfident 0.95 means moving it toward 0.5.
        let scaled = calibrator.calibrate(0.95);
        assert!(scaled < 0.95, "{scaled}");
    }

    #[test]
    fn fitting_is_a_pure_function_of_its_input() {
        let (predicted, labels) = overconfident();
        let a = Calibrator::fit("safety", &predicted, &labels);
        let b = Calibrator::fit("safety", &predicted, &labels);
        assert_eq!(a, b);
        assert_eq!(a.canonical(), b.canonical());
    }

    #[test]
    fn a_unit_temperature_is_the_identity() {
        let calibrator = Calibrator::uncalibrated("safety");
        assert!(!calibrator.is_calibrated());
        for p in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            assert!(
                (calibrator.calibrate(p) - p).abs() < 1e-6,
                "{p} -> {}",
                calibrator.calibrate(p)
            );
        }
    }

    #[test]
    fn too_little_data_is_reported_as_uncalibrated() {
        let predicted = [0.9_f32, 0.1, 0.8];
        let labels = [true, false, true];
        let calibrator = Calibrator::fit("safety", &predicted, &labels);
        assert!(!calibrator.is_calibrated());
        assert_eq!(calibrator.n, 0);
        assert!((calibrator.temperature - 1.0).abs() < 1e-9);
        // The raw metrics are still reported, so the shortfall is visible.
        assert!(calibrator.ece_before > 0.0);
        assert!((calibrator.ece - calibrator.ece_before).abs() < 1e-9);
    }

    #[test]
    fn an_empty_set_fits_to_the_uncalibrated_state() {
        let calibrator = Calibrator::fit("safety", &[], &[]);
        assert!(!calibrator.is_calibrated());
        assert_eq!(calibrator.n, 0);
        assert_eq!(calibrator.ece, 0.0);
        assert_eq!(calibrator.brier, 0.0);
    }

    #[test]
    fn the_reliability_curve_skips_bins_nobody_landed_in() {
        let curve = reliability_curve(&[0.05, 0.95], &[false, true], 10);
        assert_eq!(curve.len(), 2, "{curve:?}");
        assert_eq!(curve[0].2, 1);
        assert!((curve[1].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_calibrator_set_is_ordered_by_class() {
        let mut set = CalibratorSet::new();
        assert!(set.is_empty());
        set.insert(Calibrator::uncalibrated("risk"));
        set.insert(Calibrator::uncalibrated("safety"));
        assert_eq!(set.len(), 2);
        assert_eq!(
            set.classes(),
            vec!["risk".to_string(), "safety".to_string()]
        );
        assert!(set.get("safety").is_some());
        assert!(set.get("missing").is_none());
    }

    #[test]
    fn the_sigmoid_and_its_inverse_round_trip() {
        for p in [0.01_f64, 0.2, 0.5, 0.8, 0.99] {
            assert!((sigmoid(logit(p)) - p).abs() < 1e-9, "{p}");
        }
        // The poles are clamped rather than producing an infinity.
        assert!(logit(0.0).is_finite());
        assert!(logit(1.0).is_finite());
        assert!(logit(0.0) < 0.0);
        assert!(logit(1.0) > 0.0);
    }
}
