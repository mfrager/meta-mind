//! Calibration: is a staked probability the frequency it claims to be?
//!
//! Phase 11 asks for four things of a confidence: that it is *scored* against what
//! happened (Brier, log loss), that the score says whether the *number* is honest
//! (ECE over reliability bins), that a threshold can be *learned* rather than
//! assumed (`threshold_for`, with coverage and selective risk to say what accepting
//! that threshold costs), and that the whole thing improves on a recorded baseline.
//!
//! Four decisions are worth stating because they are not visible in the shape of the
//! code:
//!
//! * **The metric arithmetic is Phase 9's.** `brier_score`, `log_loss`,
//!   `expected_calibration_error` and `Calibrator::fit` come from `mm-decision` and
//!   are not reimplemented. Two Brier scores that disagreed in the last place would
//!   make two gates contradict each other, and the one that is wrong would be
//!   whichever a reader happened to look at first.
//! * **The bin vector re-bins, and a test proves the two agree.** `reliability_curve`
//!   returns only the *non-empty* bins, so it cannot say which bin each row was in;
//!   the presentation vector here therefore bins again, under the same convention
//!   (ten bins, `floor(p * 10)`, `p == 1.0` in the last one), and
//!   `the_bin_vector_agrees_with_the_metric` asserts that the count-weighted gap over
//!   this vector equals `expected_calibration_error` over the same points. That is
//!   what makes it a presentation rather than a second definition.
//! * **Only the consequential predictions are scored, and the ledger cannot be
//!   edited.** The calibrator reads the ledger, which refuses UPDATE and DELETE in
//!   the database; there is no code path here that could score a prediction it had
//!   rewritten first.
//! * **A threshold that cannot be met is `1.0`, not the best available.** Returning
//!   the closest threshold would quietly answer at a precision the caller refused;
//!   `1.0` means "never answer on this class", which is a state the caller can see.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use mm_core::{Param, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, LogRecord, Logger};
use serde::{Deserialize, Serialize};

use crate::error::{MetaError, Result};

/// How many bins the reliability curve and ECE use. Phase 9's value, so the two
/// crates cannot disagree about what "ten bins" means.
pub const DEFAULT_BINS: usize = mm_decision::calibration::DEFAULT_BINS;

/// How many covered points a threshold needs before it is allowed to answer.
///
/// A threshold with a precision of 1.0 on two points is not a threshold; it is a
/// coincidence. Below this many points `threshold_for` answers `1.0` (never answer),
/// which is the honest reading of "you do not have enough data to target precision".
pub const MIN_COVERED: usize = 8;

/// One labeled prediction: what was claimed, and what happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Labeled {
    /// The calibration subject.
    pub class: String,
    /// The probability that was staked.
    pub predicted: f32,
    /// Whether the proposition came true.
    pub label: bool,
}

/// One non-empty bin of the reliability curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReliabilityBin {
    /// The bin's inclusive lower bound.
    pub lower: f64,
    /// The bin's upper bound.
    pub upper: f64,
    /// How many points landed in it.
    pub n: u32,
    /// The mean staked probability.
    pub mean_predicted: f64,
    /// The observed frequency.
    pub observed_frequency: f64,
}

impl ReliabilityBin {
    /// The gap between what was claimed and what happened.
    pub fn gap(&self) -> f64 {
        (self.observed_frequency - self.mean_predicted).abs()
    }
}

/// What one scoring pass measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationReport {
    /// How many points were scored.
    pub n: u32,
    /// Mean squared error of the calibrated probabilities. Lower is better.
    pub brier: f64,
    /// The negative log likelihood of the calibrated probabilities.
    pub log_loss: f64,
    /// The expected calibration error of the calibrated probabilities.
    pub ece: f64,
    /// The non-empty reliability bins, in bin order.
    pub bins: Vec<ReliabilityBin>,
    /// The fraction of points a target-precision threshold answers on.
    pub coverage: f64,
    /// The Brier score of the covered part: the error that is actually taken.
    pub selective_risk: f64,
    /// The recorded baseline the report is compared against.
    pub baseline_brier: f64,
}

impl CalibrationReport {
    /// True when the calibrated set beats the baseline on the gate's metric.
    pub fn improves_on_baseline(&self) -> bool {
        self.brier < self.baseline_brier
    }

    /// The one-line summary the CLI prints.
    pub fn summary(&self) -> String {
        format!(
            "n={} brier={:.6} log_loss={:.6} ece={:.6} coverage={:.3} selective_risk={:.6} \
             baseline_brier={:.6}",
            self.n,
            self.brier,
            self.log_loss,
            self.ece,
            self.coverage,
            self.selective_risk,
            self.baseline_brier
        )
    }
}

/// Score a labeled set, and answer thresholds from it.
pub trait Calibrator: Send + Sync {
    /// Score the set: the metrics *after* this calibrator's correction is applied.
    fn score(&self, labeled: &[Labeled]) -> CalibrationReport;

    /// The smallest staked probability of `class` whose precision on the labeled set
    /// is at least `target_precision`. `1.0` means "never answer on this class".
    fn threshold_for(&self, class: &str, target_precision: f64) -> f64;
}

/// The calibrator this phase ships: a per-class temperature (fitted by
/// [`mm_decision::calibration::Calibrator::fit`], which minimizes ECE and includes
/// `T = 1.0` among its candidates) and empirical thresholds read off the labeled set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricsCalibrator {
    /// The baseline the report is compared against.
    pub baseline_brier: f64,
    /// The precision a threshold is asked for.
    pub target_precision: f64,
    /// The labeled points the calibrator was fitted on, kept because the thresholds
    /// are read from them.
    pub points: Vec<Labeled>,
    /// One temperature per class, including the classes that fitted to `1.0`.
    pub temperatures: BTreeMap<String, f64>,
}

impl MetricsCalibrator {
    /// Fit a calibrator on a labeled set.
    pub fn fit(labeled: &[Labeled], baseline_brier: f64, target_precision: f64) -> Self {
        let mut temperatures = BTreeMap::new();
        for class in classes_of(labeled) {
            let predicted = predictions_of(labeled, &class);
            let labels = labels_of(labeled, &class);
            let fitted =
                mm_decision::calibration::Calibrator::fit(class.clone(), &predicted, &labels);
            temperatures.insert(class, f64::from(fitted.temperature));
        }
        MetricsCalibrator {
            baseline_brier,
            target_precision,
            points: labeled.to_vec(),
            temperatures,
        }
    }

    /// A calibrator with the temperatures a reference file recorded.
    ///
    /// Used by the golden test to check the *metric* implementations against numbers
    /// computed independently, without also asking that two languages' float grids
    /// pick the same argmin: the fit's own result is compared separately, as "no
    /// worse than the reference".
    pub fn with_temperatures(
        labeled: &[Labeled],
        baseline_brier: f64,
        target_precision: f64,
        temperatures: BTreeMap<String, f64>,
    ) -> Self {
        MetricsCalibrator {
            baseline_brier,
            target_precision,
            points: labeled.to_vec(),
            temperatures,
        }
    }

    /// The temperature fitted for a class. `1.0` when the class was not fitted.
    pub fn temperature(&self, class: &str) -> f64 {
        self.temperatures.get(class).copied().unwrap_or(1.0)
    }

    /// Apply the class's temperature to one staked probability.
    pub fn calibrate(&self, class: &str, predicted: f32) -> f32 {
        let fitted = mm_decision::calibration::Calibrator {
            decision_class: class.to_string(),
            temperature: self.temperature(class) as f32,
            ece: 0.0,
            brier: 0.0,
            n: 0,
            ece_before: 0.0,
            brier_before: 0.0,
        };
        fitted.calibrate(predicted)
    }

    /// The calibrated probabilities for a set, in order.
    pub fn calibrate_all(&self, labeled: &[Labeled]) -> Vec<f32> {
        labeled
            .iter()
            .map(|point| self.calibrate(&point.class, point.predicted))
            .collect()
    }

    /// The classes the calibrator knows about, sorted.
    pub fn classes(&self) -> Vec<String> {
        self.temperatures.keys().cloned().collect()
    }
}

impl Calibrator for MetricsCalibrator {
    fn score(&self, labeled: &[Labeled]) -> CalibrationReport {
        let scaled = self.calibrate_all(labeled);
        let labels: Vec<bool> = labeled.iter().map(|point| point.label).collect();
        let accepted: Vec<bool> = labeled
            .iter()
            .map(|point| {
                point.predicted as f64 >= self.threshold_for(&point.class, self.target_precision)
            })
            .collect();
        let (coverage, selective_risk) = coverage_and_risk(&scaled, &labels, &accepted);
        CalibrationReport {
            n: u32::try_from(labeled.len()).unwrap_or(u32::MAX),
            brier: f64::from(mm_decision::calibration::brier_score(&scaled, &labels)),
            log_loss: f64::from(mm_decision::calibration::log_loss(&scaled, &labels)),
            ece: f64::from(mm_decision::calibration::expected_calibration_error(
                &scaled,
                &labels,
                DEFAULT_BINS,
            )),
            bins: bins(labeled, &scaled, DEFAULT_BINS),
            coverage,
            selective_risk,
            baseline_brier: self.baseline_brier,
        }
    }

    fn threshold_for(&self, class: &str, target_precision: f64) -> f64 {
        if !(0.0..=1.0).contains(&target_precision) {
            return 1.0;
        }
        let candidates: BTreeSet<u32> = self
            .points
            .iter()
            .filter(|point| point.class == class)
            .map(|point| point.predicted.to_bits())
            .collect();
        let mut ordered: Vec<f32> = candidates
            .into_iter()
            .map(f32::from_bits)
            .filter(|p| p.is_finite())
            .collect();
        ordered.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for candidate in ordered {
            let covered: Vec<&Labeled> = self
                .points
                .iter()
                .filter(|point| point.class == class && point.predicted >= candidate)
                .collect();
            if covered.len() < MIN_COVERED {
                continue;
            }
            let positives = covered.iter().filter(|point| point.label).count();
            let precision = positives as f64 / covered.len() as f64;
            if precision >= target_precision {
                return f64::from(candidate);
            }
        }
        1.0
    }
}

/// The reliability bins of a calibrated set.
///
/// Ten bins by default, `floor(p * n)` with `p == 1.0` in the last bin, empty bins
/// omitted. Re-binning here rather than reading `reliability_curve` is deliberate:
/// that function omits the bin *indexes*, and a presentation that guessed them from
/// the mean would be wrong for every bin whose points are not clustered.
pub fn bins(labeled: &[Labeled], scaled: &[f32], count: usize) -> Vec<ReliabilityBin> {
    let count = count.max(1);
    let n = labeled.len().min(scaled.len());
    if n == 0 {
        return Vec::new();
    }
    let mut counts = vec![0u32; count];
    let mut sums = vec![0.0f64; count];
    let mut positives = vec![0u32; count];
    for index in 0..n {
        let p = f64::from(scaled[index]).clamp(0.0, 1.0);
        let mut bin = (p * count as f64).floor() as usize;
        if bin >= count {
            bin = count - 1;
        }
        counts[bin] += 1;
        sums[bin] += p;
        if labeled[index].label {
            positives[bin] += 1;
        }
    }
    let mut out = Vec::new();
    for index in 0..count {
        if counts[index] == 0 {
            continue;
        }
        out.push(ReliabilityBin {
            lower: index as f64 / count as f64,
            upper: (index + 1) as f64 / count as f64,
            n: counts[index],
            mean_predicted: sums[index] / f64::from(counts[index]),
            observed_frequency: f64::from(positives[index]) / f64::from(counts[index]),
        });
    }
    out
}

/// Coverage and selective risk over the accepted part of a scored set.
///
/// Coverage is the accepted fraction; selective risk is the Brier score of the
/// accepted points — a probability-aware error, so a confident wrong answer costs
/// more than a hesitant one, which is what "selective" means for a calibrated system.
pub fn coverage_and_risk(scaled: &[f32], labels: &[bool], accepted: &[bool]) -> (f64, f64) {
    let n = scaled.len().min(labels.len()).min(accepted.len());
    if n == 0 {
        return (0.0, 0.0);
    }
    let covered: Vec<usize> = (0..n).filter(|index| accepted[*index]).collect();
    let coverage = covered.len() as f64 / n as f64;
    if covered.is_empty() {
        return (coverage, 0.0);
    }
    let predicted: Vec<f32> = covered.iter().map(|index| scaled[*index]).collect();
    let truth: Vec<bool> = covered.iter().map(|index| labels[*index]).collect();
    (
        coverage,
        f64::from(mm_decision::calibration::brier_score(&predicted, &truth)),
    )
}

/// The reference numbers a labeled set was scored to, computed independently.
///
/// Produced by `bench/calibration/compute_reference.py`, which is a second
/// implementation of the documented conventions. It exists so the crate's metrics are
/// checked against numbers they did not produce.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationReference {
    /// How many points the reference covers.
    pub n: u32,
    /// The calibrated Brier score of the whole set.
    pub brier: f64,
    /// The calibrated log loss of the whole set.
    pub log_loss: f64,
    /// The calibrated ECE of the whole set.
    pub ece: f64,
    /// The uncalibrated Brier score.
    pub brier_before: f64,
    /// The uncalibrated ECE.
    pub ece_before: f64,
    /// The baseline the run must improve on.
    pub baseline_brier: f64,
    /// How close the implementation must come, in absolute terms.
    ///
    /// The reference computation in `bench/calibration/compute_reference.py` models the
    /// *f32* the ledger stores (Rust reads the column as `f32`, and the metric sums run
    /// in `f32` before being widened), so the two agree to f32 precision and no further.
    /// The file states the bound rather than leaving it to a constant in the test, so the
    /// reason is in the artifact and not in whoever last read the test.
    #[serde(default = "default_reference_precision")]
    pub precision: f64,
    /// One entry per class.
    pub classes: BTreeMap<String, CalibrationReferenceClass>,
}

/// The bound a reference file gets when it does not state one: an `f32` sum's own
/// precision at these magnitudes.
fn default_reference_precision() -> f64 {
    1e-7
}

/// One class's reference numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationReferenceClass {
    /// How many points the class holds.
    pub n: u32,
    /// The temperature the reference fit.
    pub temperature: f64,
    /// The calibrated Brier score.
    pub brier: f64,
    /// The uncalibrated Brier score.
    pub brier_before: f64,
    /// The calibrated log loss.
    pub log_loss: f64,
    /// The uncalibrated log loss.
    pub log_loss_before: f64,
    /// The calibrated ECE.
    pub ece: f64,
    /// The uncalibrated ECE.
    pub ece_before: f64,
}

impl CalibrationReference {
    /// The fitted temperatures, in the form [`MetricsCalibrator::with_temperatures`]
    /// takes.
    pub fn temperatures(&self) -> BTreeMap<String, f64> {
        self.classes
            .iter()
            .map(|(class, entry)| (class.clone(), entry.temperature))
            .collect()
    }
}

/// Read a labeled set: one JSON object per line, `{class, predicted, label}`.
///
/// Blank lines are skipped and a line that is not an object is a refusal, not a
/// silently dropped point: a corpus that half-loaded would produce a calibration
/// summary with no trace of what was missing.
pub async fn load_labeled(path: &Path) -> Result<Vec<Labeled>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
            MetaError::Record(format!("{} line {}: {e}", path.display(), index + 1))
        })?;
        let class = value
            .get("class")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                MetaError::Record(format!("{} line {}: no class", path.display(), index + 1))
            })?;
        let predicted = value
            .get("predicted")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| {
                MetaError::Record(format!(
                    "{} line {}: no predicted probability",
                    path.display(),
                    index + 1
                ))
            })?;
        let label = value
            .get("label")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                MetaError::Record(format!("{} line {}: no label", path.display(), index + 1))
            })?;
        if !(0.0..=1.0).contains(&predicted) {
            return Err(MetaError::Validation {
                field: "predicted",
                detail: format!(
                    "{} line {}: probability {predicted} is outside [0,1]",
                    path.display(),
                    index + 1
                ),
            });
        }
        out.push(Labeled {
            class: class.to_string(),
            predicted: predicted as f32,
            label,
        });
    }
    Ok(out)
}

/// Read a reference file.
pub async fn load_reference(path: &Path) -> Result<CalibrationReference> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|e| {
        MetaError::Record(format!(
            "{} is not a calibration reference: {e}",
            path.display()
        ))
    })
}

/// Record a scoring pass in `calibration_runs`, and emit `calib.report`.
///
/// The row is the durable half and the record is the observable half: the gate's
/// `logs verify` reads the second, and an operator asking "how well calibrated was it
/// in March" reads the first.
pub async fn record_run(
    store: &mm_store_sqlite::SqliteStore,
    logger: &Logger,
    ids: &UlidFactory,
    report: &CalibrationReport,
    subject: &str,
) -> Result<Ulid> {
    let id = ids.next();
    let at = Timestamp::now().to_rfc3339();
    Tabular::execute(
        store,
        "INSERT INTO calibration_runs (id, subject, n, brier, log_loss, ece, coverage, \
         selective_risk, baseline_brier, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        vec![
            Param::Text(mm_core::ulid_string(&id)),
            Param::Text(subject.to_string()),
            Param::Int(i64::from(report.n)),
            Param::Real(report.brier),
            Param::Real(report.log_loss),
            Param::Real(report.ece),
            Param::Real(report.coverage),
            Param::Real(report.selective_risk),
            Param::Real(report.baseline_brier),
            Param::Text(at),
        ],
    )
    .await
    .map_err(|e| MetaError::Store(e.to_string()))?;

    logger
        .emit(
            LogRecord::new(Level::Info, codes::CALIB_REPORT, crate::TARGET)
                .with_field("subject", subject)
                .with_field("n", i64::from(report.n))
                .with_field("brier", report.brier)
                .with_field("log_loss", report.log_loss)
                .with_field("ece", report.ece)
                .with_field("coverage", report.coverage)
                .with_field("baseline_brier", report.baseline_brier),
        )
        .await?;
    Ok(id)
}

/// Every class in the set, sorted.
pub fn classes_of(labeled: &[Labeled]) -> Vec<String> {
    let set: BTreeSet<String> = labeled.iter().map(|point| point.class.clone()).collect();
    set.into_iter().collect()
}

fn predictions_of(labeled: &[Labeled], class: &str) -> Vec<f32> {
    labeled
        .iter()
        .filter(|point| point.class == class)
        .map(|point| point.predicted)
        .collect()
}

fn labels_of(labeled: &[Labeled], class: &str) -> Vec<bool> {
    labeled
        .iter()
        .filter(|point| point.class == class)
        .map(|point| point.label)
        .collect()
}

/// The target precision the CLI's gate asks for when none is given.
pub const DEFAULT_TARGET_PRECISION: f64 = 0.8;

/// Build a calibrator for a labeled set with the default target precision.
pub fn fit_default(labeled: &[Labeled], baseline_brier: f64) -> MetricsCalibrator {
    MetricsCalibrator::fit(labeled, baseline_brier, DEFAULT_TARGET_PRECISION)
}

/// An `Arc` of a calibrator, for the diagnosis and CLI layers that hold traits.
pub type SharedCalibrator = Arc<dyn Calibrator>;

#[cfg(test)]
mod tests {
    use super::*;

    fn overconfident() -> Vec<Labeled> {
        let mut points = Vec::new();
        for index in 0..50 {
            points.push(Labeled {
                class: "safety".into(),
                predicted: 0.98,
                label: index < 40,
            });
        }
        points
    }

    #[test]
    fn the_bin_vector_agrees_with_the_metric() {
        let points = overconfident();
        let scaled: Vec<f32> = points.iter().map(|p| p.predicted).collect();
        let labels: Vec<bool> = points.iter().map(|p| p.label).collect();
        let bins = bins(&points, &scaled, DEFAULT_BINS);
        // The count-weighted mean gap over this presentation must equal Phase 9's ECE.
        let n = points.len() as f64;
        let weighted: f64 = bins
            .iter()
            .map(|bin| (f64::from(bin.n) / n) * bin.gap())
            .sum();
        let ece = f64::from(mm_decision::calibration::expected_calibration_error(
            &scaled,
            &labels,
            DEFAULT_BINS,
        ));
        // The tolerance is f32's, not f64's, and deliberately so: the probabilities are
        // `f32` and `mm-decision` sums its ECE in `f32` before widening, so an exact
        // comparison would be asserting that two different roundings agree to the last
        // bit. 1e-6 is ~70 f32 ulps at this magnitude, which is still a statement that
        // the bin vector and the metric measure the same thing.
        assert!(
            (weighted - ece).abs() < 1e-6,
            "bin vector {weighted} vs metric {ece}"
        );
    }

    #[test]
    fn fitting_improves_both_metrics_and_never_the_wrong_way() {
        let points = overconfident();
        let calibrator = MetricsCalibrator::fit(&points, 0.30, DEFAULT_TARGET_PRECISION);
        let report = calibrator.score(&points);
        assert_eq!(report.n, 50);
        assert!(
            calibrator.temperature("safety") > 1.0,
            "an overconfident set needs softening"
        );
        assert!(report.brier < 0.30, "{}", report.summary());
        assert!(report.improves_on_baseline());
        // ECE after fitting is far below the 0.18 the raw set shows.
        assert!(report.ece < 0.01, "{}", report.summary());
    }

    #[test]
    fn a_class_with_too_little_data_fits_to_the_identity() {
        let points: Vec<Labeled> = (0..3)
            .map(|index| Labeled {
                class: "safety".into(),
                predicted: 0.9,
                label: index == 0,
            })
            .collect();
        let calibrator = MetricsCalibrator::fit(&points, 0.25, DEFAULT_TARGET_PRECISION);
        assert_eq!(calibrator.temperature("safety"), 1.0);
        let report = calibrator.score(&points);
        assert_eq!(
            report.brier,
            f64::from(mm_decision::calibration::brier_score(
                &[0.9, 0.9, 0.9],
                &[true, false, false]
            ))
        );
    }

    #[test]
    fn a_threshold_is_the_smallest_one_that_meets_the_precision() {
        let mut points = Vec::new();
        for index in 0..20 {
            points.push(Labeled {
                class: "retrieval".into(),
                predicted: 0.9,
                label: index < 18,
            });
        }
        for index in 0..20 {
            points.push(Labeled {
                class: "retrieval".into(),
                predicted: 0.5,
                label: index < 5,
            });
        }
        let calibrator = MetricsCalibrator::fit(&points, 0.3, 0.8);
        let threshold = calibrator.threshold_for("retrieval", 0.8);
        assert!((threshold - 0.9).abs() < 1e-6, "{threshold}");
        // 0.9 covers 20 of 40 points with precision 0.9.
        let mut covered = points.clone();
        covered.retain(|point| point.predicted as f64 >= threshold);
        assert_eq!(covered.len(), 20);
        assert_eq!(covered.iter().filter(|point| point.label).count(), 18);
    }

    #[test]
    fn a_threshold_that_cannot_be_met_is_never_answer() {
        // Everything is wrong, so no threshold reaches precision 0.8.
        let points: Vec<Labeled> = (0..20)
            .map(|_| Labeled {
                class: "safety".into(),
                predicted: 0.9,
                label: false,
            })
            .collect();
        let calibrator = MetricsCalibrator::fit(&points, 0.3, 0.8);
        assert_eq!(calibrator.threshold_for("safety", 0.8), 1.0);
        // Too few points is also "never answer", rather than a confidence on nothing.
        let few: Vec<Labeled> = (0..4)
            .map(|_| Labeled {
                class: "safety".into(),
                predicted: 0.9,
                label: true,
            })
            .collect();
        let calibrator = MetricsCalibrator::fit(&few, 0.3, 0.8);
        assert_eq!(calibrator.threshold_for("safety", 0.8), 1.0);
        // An unknown class has no points at all.
        assert_eq!(calibrator.threshold_for("missing", 0.8), 1.0);
    }

    #[test]
    fn coverage_and_selective_risk_read_the_accepted_part() {
        let scaled = [0.9_f32, 0.9, 0.1, 0.1];
        let labels = [true, true, false, true];
        let accepted = [true, true, false, false];
        let (coverage, risk) = coverage_and_risk(&scaled, &labels, &accepted);
        assert!((coverage - 0.5).abs() < 1e-12);
        // The accepted part is perfectly right, so its Brier is 0.01 — to `f32`
        // precision, because the probabilities are `f32` and the squared errors are
        // summed in `f64` from widened values.
        assert!((risk - 0.01).abs() < 1e-6, "{risk}");
        let (none, zero) = coverage_and_risk(&scaled, &labels, &[false, false, false, false]);
        assert_eq!(none, 0.0);
        assert_eq!(zero, 0.0);
    }

    #[test]
    fn calibrating_a_known_class_is_the_identity_at_temperature_one() {
        let points = overconfident();
        let calibrator = MetricsCalibrator::with_temperatures(
            &points,
            0.3,
            DEFAULT_TARGET_PRECISION,
            BTreeMap::new(),
        );
        for p in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            assert!((calibrator.calibrate("safety", p) - p).abs() < 1e-6);
        }
    }
}
