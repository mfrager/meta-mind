//! The `calibration` module.
//!
//! This module is the remediation of the capability gap recorded in
//! `bench/gaps/gap_01.json`: *no module can summarize a drifting calibration bin, so a
//! class that has become overconfident is only visible as a number an operator has to
//! read*. It answers that with two surfaces and one field that did not exist before —
//! `drifting`, the classes whose post-calibration error is still too large to trust.
//!
//! The arithmetic lives in [`mm_decision::calibration`], where the Brier score, the log
//! loss, the expected calibration error and the temperature fit already have tests and
//! a labeled bench; duplicating any of it here would create a second place for the
//! definition of "calibrated" to drift, and the kernel's own decision path would be
//! graded by a different formula than this summary. What this module owns is the
//! *summary*: the per-class fit, the reliability curve as bins an operator can read,
//! the drift list, and the two T-Box functions a caller can invoke without linking the
//! decision crate.
//!
//! Three decisions are worth stating because they are not visible in the shape of the
//! code:
//!
//! * **The summary is computed at the fitted temperature, not at the raw
//!   probability.** A calibration summary of uncalibrated numbers reports the problem
//!   rather than the answer; the point of the module is to say what the confidence
//!   *would* mean after fitting, and then whether that is still drifting.
//! * **`drifting` is a list, not a boolean.** "The system is miscalibrated" is not
//!   actionable; "class `retrieval` is overconfident beyond the tolerance" is, and it
//!   is what a later change set is assembled from.
//! * **A prediction outside `[0,1]`, an empty class, or a malformed line is refused,
//!   never clamped.** A clamped probability would let a caller's bug read as a
//!   well-calibrated answer.
//!
//! The manifest is embedded at compile time and parsed by a test, so a module whose
//! `plugin.toml` is malformed — or whose declared `uri` has drifted from its directory —
//! fails a test rather than a load.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;

use mm_core::MmError;
// The metrics and the scaling are the kernel's, imported by name so a reader can see
// that this module defines no formula of its own.
use mm_decision::calibration::{
    brier_score, expected_calibration_error, log_loss, logit, sigmoid, Calibrator,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The module's embedded manifest.
pub const MANIFEST_TOML: &str = include_str!("../plugin.toml");

/// `cognition.calibration_summary` — score a labeled set and report the reliability
/// curve, the fitted temperature and the classes that still drift.
pub const COGNITION_CALIBRATION_SUMMARY: &str = "cognition.calibration_summary";
/// `cognition.calibration_drift` — name the classes whose post-calibration error
/// exceeds a caller's tolerance.
pub const COGNITION_CALIBRATION_DRIFT: &str = "cognition.calibration_drift";

/// Every T-Box function this module exposes, in a stable order.
pub const SURFACE_FUNCTIONS: [&str; 2] =
    [COGNITION_CALIBRATION_SUMMARY, COGNITION_CALIBRATION_DRIFT];

/// The capability this module implements.
pub const CAPABILITY: &str = "mm:CalibrationSummary";

/// This module's path inside `modules/`, which is what its IRI is derived from.
pub const MODULE_PATH: &str = "cognition/calibration";

/// How many bins the reliability curve and the ECE use.
///
/// The same ten as [`mm_decision::calibration::DEFAULT_BINS`], restated as a constant
/// here because the two numbers have to agree: a curve binned at ten and an error
/// binned at five would show an operator a picture that does not match the number
/// beside it.
pub const DEFAULT_BINS: usize = 10;

/// The default drift tolerance: a class whose post-calibration ECE exceeds five points
/// of probability is drifting.
///
/// It is a *policy* number, not a fact about the data, so it lives here as a named
/// constant a caller can read and override rather than as a literal inside the fit.
pub const DEFAULT_DRIFT_TOLERANCE: f64 = 0.05;

/// The tables this module reads.
///
/// Empty, and deliberately so: the module is pure arithmetic over a labeled set handed
/// to it, and a caller can therefore score points that were never stored — a replayed
/// episode, a handful of hand-written cases — without the module reaching for a store.
pub const READS: [&str; 0] = [];

/// The tables this module writes.
///
/// Empty for the same reason as [`READS`]: what is persisted about a calibration run is
/// `mm-metaanalysis`'s `calibration_runs` row, written by the crate that owns the
/// ledger, not by a cognition module.
pub const WRITES: [&str; 0] = [];

/// One labeled point: what was predicted, in what class, and what happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabeledPoint {
    /// The calibration subject, e.g. `safety` or `retrieval`.
    pub class: String,
    /// The staked probability, in `[0,1]`.
    pub predicted: f32,
    /// Whether the proposition came true.
    pub label: bool,
}

/// One non-empty bin of the reliability curve.
///
/// Empty bins are omitted rather than reported as zero, because a bin nobody landed in
/// says nothing about calibration and a zeroed row would look like a measurement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReliabilityBin {
    /// Inclusive lower edge, `index / bins`.
    pub lower: f64,
    /// Exclusive upper edge, `(index + 1) / bins`, except for the last bin, which is
    /// right-inclusive so a probability of exactly 1.0 has a home.
    pub upper: f64,
    /// How many points landed here.
    pub n: u32,
    /// The mean of the calibrated probabilities in the bin.
    pub mean_predicted: f64,
    /// The observed frequency of the label in the bin.
    pub observed_frequency: f64,
}

/// The three metrics of a scored set, at whatever temperature produced them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// How many points were scored.
    pub n: usize,
    /// Mean squared error of the probabilities against their labels. Lower is better.
    pub brier: f64,
    /// The negative log likelihood, poles clamped. Lower is better.
    pub log_loss: f64,
    /// The count-weighted mean gap between confidence and observed frequency.
    pub ece: f64,
}

/// What a caller learns about a labeled set: the metrics, the curve, and what still
/// drifts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationSummary {
    /// How many points were scored.
    pub n: usize,
    /// The Brier score at the fitted temperatures.
    pub brier: f64,
    /// The log loss at the fitted temperatures.
    pub log_loss: f64,
    /// The expected calibration error at the fitted temperatures.
    pub ece: f64,
    /// The reliability curve of the calibrated probabilities, non-empty bins only.
    pub bins: Vec<ReliabilityBin>,
    /// The classes whose post-calibration ECE still exceeds
    /// [`DEFAULT_DRIFT_TOLERANCE`], sorted by class name.
    pub drifting: Vec<String>,
    /// The baseline Brier the caller said to compare against, carried through so a
    /// report shows both sides of the comparison without a second lookup.
    pub baseline_brier: f64,
}

/// Everything this module can refuse.
///
/// Two variants, because the two refusals mean different things to a caller: a
/// [`CalibrationModuleError::Json`] refusal is the caller's payload not being the shape
/// the function takes, and a [`CalibrationModuleError::Refused`] one is the module
/// refusing a payload it understood — an out-of-range probability, an empty class, a
/// tolerance that is negative or not a number.
#[derive(Debug, Clone, PartialEq)]
pub enum CalibrationModuleError {
    /// The payload was not the JSON shape the function takes.
    Json {
        /// The T-Box function that refused.
        function: &'static str,
        /// What was wrong with the payload.
        detail: String,
    },
    /// The module refused a payload it understood.
    Refused {
        /// The T-Box function that refused.
        function: &'static str,
        /// A stable code for the refusal.
        code: &'static str,
        /// What was refused and why.
        detail: String,
    },
}

impl CalibrationModuleError {
    /// A payload-shape refusal.
    pub fn json(function: &'static str, detail: impl Into<String>) -> Self {
        CalibrationModuleError::Json {
            function,
            detail: detail.into(),
        }
    }

    /// A refusal of a payload the module understood.
    pub fn refused(function: &'static str, code: &'static str, detail: impl Into<String>) -> Self {
        CalibrationModuleError::Refused {
            function,
            code,
            detail: detail.into(),
        }
    }

    /// The T-Box function that refused.
    pub fn function(&self) -> &'static str {
        match self {
            CalibrationModuleError::Json { function, .. }
            | CalibrationModuleError::Refused { function, .. } => function,
        }
    }

    /// The stable code, so a caller can branch on the refusal rather than parse it.
    pub fn code(&self) -> &'static str {
        match self {
            CalibrationModuleError::Json { .. } => "json",
            CalibrationModuleError::Refused { code, .. } => code,
        }
    }
}

impl fmt::Display for CalibrationModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CalibrationModuleError::Json { function, detail } => {
                write!(f, "{function}: malformed payload: {detail}")
            }
            CalibrationModuleError::Refused {
                function,
                code,
                detail,
            } => write!(f, "{function}: {code}: {detail}"),
        }
    }
}

impl std::error::Error for CalibrationModuleError {}

impl From<CalibrationModuleError> for MmError {
    fn from(e: CalibrationModuleError) -> Self {
        MmError::Internal(format!("{} ({})", e, e.code()))
    }
}

/// Every T-Box function this module exposes.
pub fn functions() -> Vec<&'static str> {
    SURFACE_FUNCTIONS.to_vec()
}

/// This module's stable, path-derived IRI.
pub fn module_iri() -> mm_core::NamedNode {
    mm_core::iri::module(MODULE_PATH)
}

/// The handler behind `cognition.calibration_summary`, named so the manifest and the
/// binary cannot drift silently.
pub fn summary_handler() -> &'static str {
    "handlers::calibration_summary"
}

/// The handler behind `cognition.calibration_drift`.
pub fn drift_handler() -> &'static str {
    "handlers::calibration_drift"
}

/// This module's directory, as compiled.
pub fn module_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// This module's manifest path, as compiled.
pub fn manifest_path() -> std::path::PathBuf {
    module_dir().join("plugin.toml")
}

/// Read a JSONL labeled set.
///
/// One JSON object per line, blank lines skipped, each object a [`LabeledPoint`]. A
/// line that is not a labeled point, a prediction outside `[0,1]`, and an empty class
/// are all errors that name the line: a corpus is read once and read literally, and a
/// silently skipped point would change the score it is supposed to explain.
pub fn parse_jsonl(text: &str) -> Result<Vec<LabeledPoint>, String> {
    let mut points = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let point: LabeledPoint = serde_json::from_str(trimmed)
            .map_err(|e| format!("line {} is not a labeled point: {e}", index + 1))?;
        validate(&point).map_err(|detail| format!("line {}: {detail}", index + 1))?;
        points.push(point);
    }
    Ok(points)
}

/// The fit, the curve and the drift list for a labeled set.
///
/// The temperatures are fitted per class with
/// [`mm_decision::calibration::Calibrator::fit`], so a class with too little data to
/// fit keeps `T = 1.0` — the kernel's own rule, and the reason a small corpus reads as
/// uncalibrated rather than as confidently wrong.
pub fn summarize(points: &[LabeledPoint], baseline_brier: f64) -> CalibrationSummary {
    let temperatures = fitted_temperatures(points);
    let metrics = score_with(points, &temperatures);
    let labels: Vec<bool> = points.iter().map(|point| point.label).collect();
    let scaled = scaled_predictions(points, &temperatures);
    CalibrationSummary {
        n: metrics.n,
        brier: metrics.brier,
        log_loss: metrics.log_loss,
        ece: metrics.ece,
        bins: reliability_bins(&scaled, &labels, DEFAULT_BINS),
        drifting: drifting_classes(points, DEFAULT_DRIFT_TOLERANCE),
        baseline_brier,
    }
}

/// The classes whose post-calibration expected calibration error exceeds `tolerance`,
/// sorted by class name.
///
/// The ECE is computed after the per-class fit, which is what makes the answer
/// meaningful: every raw overconfident class would drift by definition, and the
/// question an operator has is which classes are *still* wrong once the fit has done
/// what it can.
pub fn drifting_classes(points: &[LabeledPoint], tolerance: f64) -> Vec<String> {
    let temperatures = fitted_temperatures(points);
    class_ece(points, &temperatures)
        .into_iter()
        .filter(|(_, ece)| *ece > tolerance)
        .map(|(class, _)| class)
        .collect()
}

/// Score a labeled set with explicit per-class temperatures.
///
/// A class that is not in `temperatures` is scored at `T = 1.0`, which is the
/// identity. This is the entry point a caller uses to reproduce a stored run: the
/// temperatures a `calibration_runs` row recorded are applied to the same points, so
/// the numbers and the fit can be checked apart from each other.
pub fn score_at(points: &[LabeledPoint], temperatures: &[(String, f64)]) -> Metrics {
    let map: BTreeMap<String, f64> = temperatures.iter().cloned().collect();
    score_with(points, &map)
}

/// The reliability curve of a set of probabilities against their labels.
///
/// The binning is the kernel's: `floor(p * bins)` with the last bin right-inclusive,
/// and empty bins skipped. It is written here rather than taken from
/// [`mm_decision::calibration::reliability_curve`] because that function returns the
/// mean and the count without the bin's *index*, and an operator reading a curve needs
/// the edges; the ECE itself still comes from `mm-decision`, so the number and the
/// picture cannot disagree about the boundaries.
pub fn reliability_bins(predicted: &[f32], labels: &[bool], bins: usize) -> Vec<ReliabilityBin> {
    let bins = bins.max(1);
    let n = predicted.len().min(labels.len());
    let mut counts = vec![0u32; bins];
    let mut sums = vec![0.0f64; bins];
    let mut positives = vec![0u32; bins];
    for index in 0..n {
        let p = f64::from(predicted[index]).clamp(0.0, 1.0);
        let bin = (p * bins as f64).floor().min(bins as f64 - 1.0) as usize;
        counts[bin] += 1;
        sums[bin] += p;
        if labels[index] {
            positives[bin] += 1;
        }
    }
    let mut curve = Vec::new();
    for bin in 0..bins {
        if counts[bin] == 0 {
            continue;
        }
        curve.push(ReliabilityBin {
            lower: bin as f64 / bins as f64,
            upper: (bin + 1) as f64 / bins as f64,
            n: counts[bin],
            mean_predicted: sums[bin] / f64::from(counts[bin]),
            observed_frequency: f64::from(positives[bin]) / f64::from(counts[bin]),
        });
    }
    curve
}

/// One temperature per class, fitted the way the kernel fits them.
///
/// A `BTreeMap`, so iteration — and therefore every report built from it — is
/// deterministic. Fitting is a pure function of the points, so two runs over the same
/// corpus produce the same temperatures.
///
/// Public because the fitted temperature is itself a result: a caller recording a
/// calibration run stores what was fitted alongside the metrics, and this test needs to
/// compare the fit against the recorded reference separately from the metrics it
/// produces.
pub fn fitted_temperatures(points: &[LabeledPoint]) -> BTreeMap<String, f64> {
    grouped(points)
        .into_iter()
        .map(|(class, (predicted, labels))| {
            let fitted = Calibrator::fit(class.clone(), &predicted, &labels);
            (class, f64::from(fitted.temperature))
        })
        .collect()
}

/// The points, grouped by class into `(probabilities, labels)` pairs.
fn grouped(points: &[LabeledPoint]) -> BTreeMap<String, (Vec<f32>, Vec<bool>)> {
    let mut grouped: BTreeMap<String, (Vec<f32>, Vec<bool>)> = BTreeMap::new();
    for point in points {
        let entry = grouped.entry(point.class.clone()).or_default();
        entry.0.push(point.predicted);
        entry.1.push(point.label);
    }
    grouped
}

/// The metrics after applying each point's class temperature.
fn score_with(points: &[LabeledPoint], temperatures: &BTreeMap<String, f64>) -> Metrics {
    let scaled = scaled_predictions(points, temperatures);
    let labels: Vec<bool> = points.iter().map(|point| point.label).collect();
    Metrics {
        n: points.len(),
        brier: f64::from(brier_score(&scaled, &labels)),
        log_loss: f64::from(log_loss(&scaled, &labels)),
        ece: f64::from(expected_calibration_error(&scaled, &labels, DEFAULT_BINS)),
    }
}

/// Every point's probability after its class temperature is applied.
fn scaled_predictions(points: &[LabeledPoint], temperatures: &BTreeMap<String, f64>) -> Vec<f32> {
    points
        .iter()
        .map(|point| {
            let temperature = temperatures.get(&point.class).copied().unwrap_or(1.0);
            scale(point.predicted, temperature)
        })
        .collect()
}

/// `sigmoid(logit(p) / T)`, with `T = 1.0` and any non-finite or non-positive
/// temperature as the identity.
///
/// The scaling is `mm-decision`'s formula, evaluated in `f64` against the temperature
/// the fit produced, so a stored f64 temperature is applied as it was recorded rather
/// than after a round trip through `f32`.
fn scale(p: f32, temperature: f64) -> f32 {
    if !temperature.is_finite() || temperature <= 0.0 || (temperature - 1.0).abs() < f64::EPSILON {
        return p;
    }
    sigmoid(logit(f64::from(p)) / temperature) as f32
}

/// The post-calibration ECE of each class.
fn class_ece(
    points: &[LabeledPoint],
    temperatures: &BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    grouped(points)
        .into_iter()
        .map(|(class, (predicted, labels))| {
            let temperature = temperatures.get(&class).copied().unwrap_or(1.0);
            let scaled: Vec<f32> = predicted.iter().map(|p| scale(*p, temperature)).collect();
            let ece = expected_calibration_error(&scaled, &labels, DEFAULT_BINS);
            (class, f64::from(ece))
        })
        .collect()
}

/// Refuse a point that is not a usable labeled point.
fn validate(point: &LabeledPoint) -> Result<(), String> {
    if !point.predicted.is_finite() || !(0.0..=1.0).contains(&point.predicted) {
        return Err(format!(
            "predicted {} is not a probability in [0,1]",
            point.predicted
        ));
    }
    if point.class.trim().is_empty() {
        return Err("class is empty".to_string());
    }
    Ok(())
}

/// Turn a JSON array of labeled points into typed ones, or refuse.
pub fn points_from_json(
    value: &Value,
    function: &'static str,
) -> Result<Vec<LabeledPoint>, CalibrationModuleError> {
    let array = match value.as_array() {
        Some(array) => array,
        None => {
            return Err(CalibrationModuleError::json(
                function,
                "expected an array of points",
            ))
        }
    };
    let mut points = Vec::with_capacity(array.len());
    for (index, item) in array.iter().enumerate() {
        let point: LabeledPoint = serde_json::from_value(item.clone()).map_err(|e| {
            CalibrationModuleError::json(
                function,
                format!("point {index} is not a labeled point: {e}"),
            )
        })?;
        validate(&point)
            .map_err(|detail| CalibrationModuleError::refused(function, "validation", detail))?;
        points.push(point);
    }
    Ok(points)
}

/// The two T-Box handlers. `plugin.toml` names them by their path
/// (`handlers::calibration_summary`), which is the name the code graph's symbol table
/// knows them by, so a rename here is a rename there or `codex verify` refuses it.
pub mod handlers {
    use serde_json::{json, Value};

    use crate::{
        drifting_classes, points_from_json, summarize, CalibrationModuleError,
        COGNITION_CALIBRATION_DRIFT, COGNITION_CALIBRATION_SUMMARY,
    };

    /// `cognition.calibration_summary` — score a labeled set.
    ///
    /// `points` is a JSON array of `{class, predicted, label}`. The returned value is
    /// the `CalibrationSummary`, including the reliability curve and the drift list at
    /// the module's default tolerance.
    pub fn calibration_summary(
        points: &Value,
        baseline_brier: f64,
    ) -> Result<Value, CalibrationModuleError> {
        let points = points_from_json(points, COGNITION_CALIBRATION_SUMMARY)?;
        let summary = summarize(&points, baseline_brier);
        serde_json::to_value(&summary)
            .map_err(|e| CalibrationModuleError::json(COGNITION_CALIBRATION_SUMMARY, e.to_string()))
    }

    /// `cognition.calibration_drift` — name the classes that still drift.
    ///
    /// A negative or non-finite tolerance is refused rather than treated as zero: a
    /// caller that asked for a tolerance of `NaN` asked a question with no answer, and
    /// answering "nothing drifts" would hide that.
    pub fn calibration_drift(
        points: &Value,
        tolerance: f64,
    ) -> Result<Value, CalibrationModuleError> {
        if !tolerance.is_finite() || tolerance < 0.0 {
            return Err(CalibrationModuleError::refused(
                COGNITION_CALIBRATION_DRIFT,
                "validation",
                format!("tolerance {tolerance} must be finite and not negative"),
            ));
        }
        let points = points_from_json(points, COGNITION_CALIBRATION_DRIFT)?;
        let drifting = drifting_classes(&points, tolerance);
        Ok(json!({
            "n": points.len(),
            "tolerance": tolerance,
            "drifting_count": drifting.len(),
            "drifting": drifting,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn point(class: &str, predicted: f32, label: bool) -> LabeledPoint {
        LabeledPoint {
            class: class.to_string(),
            predicted,
            label,
        }
    }

    /// A class that is permanently overconfident: it claims 0.999 and delivers half,
    /// so no temperature can repair it — the fit can only flatten it toward 0.5, and
    /// the grid stops at `T = 20`.
    fn permanently_overconfident() -> Vec<LabeledPoint> {
        let mut points = Vec::new();
        for index in 0..50 {
            points.push(point("safety", 0.999, index % 2 == 0));
        }
        points
    }

    /// A class that is overconfident but honest underneath: it says 0.98 and delivers
    /// 0.8, which one temperature fixes.
    fn fixable() -> Vec<LabeledPoint> {
        let mut points = Vec::new();
        for index in 0..50 {
            points.push(point("safety", 0.98, index < 40));
        }
        points
    }

    #[test]
    fn the_metrics_at_a_unit_temperature_are_the_hand_computed_ones() {
        let points = vec![
            point("safety", 0.5, true),
            point("safety", 0.5, false),
            point("safety", 0.0, false),
            point("safety", 1.0, true),
        ];
        let metrics = score_at(&points, &[]);
        assert_eq!(metrics.n, 4);
        // (0.25 + 0.25 + 0 + 0) / 4, exact in binary.
        assert_eq!(metrics.brier, 0.125);
        // Every bin's mean equals its observed frequency, so the error is zero.
        assert_eq!(metrics.ece, 0.0);
        // (-ln 0.5 - ln 0.5) / 4. Asserted to 1e-7 rather than exactly, because the
        // metric is `mm-decision`'s and comes back as an `f32`.
        let expected = 1.386_294_361_119_890_6_f64 / 4.0;
        assert!(
            (metrics.log_loss - expected).abs() < 1e-7,
            "{} vs {expected}",
            metrics.log_loss
        );
    }

    #[test]
    fn a_fixable_class_improves_and_does_not_drift() {
        let points = fixable();
        let raw = score_at(&points, &[]);
        let summary = summarize(&points, 0.30);
        assert!(
            summary.brier < raw.brier,
            "fitting must improve the Brier score: {} vs {}",
            summary.brier,
            raw.brier
        );
        assert!(
            summary.ece < raw.ece,
            "fitting must improve the ECE: {} vs {}",
            summary.ece,
            raw.ece
        );
        assert!(
            summary.drifting.is_empty(),
            "a repaired class must not drift: {:?}",
            summary.drifting
        );
        assert_eq!(summary.n, 50);
        assert!(!summary.bins.is_empty());
    }

    #[test]
    fn a_permanently_overconfident_class_is_named_as_drifting() {
        let points = permanently_overconfident();
        let summary = summarize(&points, 0.30);
        assert_eq!(summary.drifting, vec!["safety".to_string()]);
        // Fitting cannot rescue it, so the ECE stays far above the tolerance.
        assert!(summary.ece > DEFAULT_DRIFT_TOLERANCE, "{}", summary.ece);
        // The drift list is sorted and de-duplicated by construction: one entry per
        // class, because the map behind it is keyed by class.
        let both = vec![
            point("zulu", 0.999, true),
            point("alpha", 0.999, false),
            point("zulu", 0.999, false),
            point("alpha", 0.999, true),
        ];
        assert_eq!(
            drifting_classes(&both, 0.0),
            vec!["alpha".to_string(), "zulu".to_string()]
        );
        // A tolerance nothing can exceed names nobody.
        assert!(drifting_classes(&both, 1.0).is_empty());
    }

    #[test]
    fn an_empty_set_summarizes_to_zeros_rather_than_failing() {
        let summary = summarize(&[], 0.25);
        assert_eq!(summary.n, 0);
        assert_eq!(summary.brier, 0.0);
        assert_eq!(summary.log_loss, 0.0);
        assert_eq!(summary.ece, 0.0);
        assert!(summary.bins.is_empty());
        assert!(summary.drifting.is_empty());
        assert_eq!(summary.baseline_brier, 0.25);
    }

    #[test]
    fn the_reliability_curve_reports_edges_and_skips_empty_bins() {
        let bins = reliability_bins(&[0.05, 0.05, 0.95], &[false, true, true], 10);
        assert_eq!(bins.len(), 2, "{bins:?}");
        assert_eq!(bins[0].lower, 0.0);
        assert_eq!(bins[0].upper, 0.1);
        assert_eq!(bins[0].n, 2);
        assert!((bins[0].observed_frequency - 0.5).abs() < 1e-6);
        assert_eq!(bins[1].lower, 0.9);
        assert_eq!(bins[1].n, 1);
        assert!((bins[1].observed_frequency - 1.0).abs() < 1e-6);
        // A probability of exactly 1.0 lands in the last bin rather than off the end.
        let edge = reliability_bins(&[1.0], &[true], 10);
        assert_eq!(edge.len(), 1);
        assert_eq!(edge[0].upper, 1.0);
    }

    #[test]
    fn parse_jsonl_reads_the_corpus_shape_and_refuses_a_bad_line() {
        let text = "{\"class\": \"safety\", \"predicted\": 0.5, \"label\": true}\n\n\
                    {\"class\": \"safety\", \"predicted\": 0.25, \"label\": false}\n";
        let points = parse_jsonl(text).expect("two points");
        assert_eq!(points.len(), 2);
        assert_eq!(points[1].predicted, 0.25);

        let error =
            parse_jsonl("{\"class\": \"a\", \"predicted\": 0.5, \"label\": true}\nnot json\n")
                .expect_err("a refusal");
        assert!(error.contains("line 2"), "{error}");

        let error = parse_jsonl("{\"class\": \"a\", \"predicted\": 1.5, \"label\": true}\n")
            .expect_err("a refusal");
        assert!(error.contains("line 1"), "{error}");
        assert!(error.contains("not a probability"), "{error}");

        let error = parse_jsonl("{\"class\": \"\", \"predicted\": 0.5, \"label\": true}\n")
            .expect_err("a refusal");
        assert!(error.contains("class is empty"), "{error}");

        assert!(parse_jsonl("")
            .expect("an empty corpus is zero points")
            .is_empty());
    }

    #[test]
    fn the_handlers_answer_the_same_numbers_as_the_typed_api() {
        let points = fixable();
        let as_json = serde_json::to_value(&points).expect("the points serialize");
        let out = handlers::calibration_summary(&as_json, 0.30).expect("a summary");
        let summary = summarize(&points, 0.30);
        assert_eq!(out["n"].as_u64(), Some(summary.n as u64));
        assert!((out["brier"].as_f64().unwrap() - summary.brier).abs() < 1e-12);
        assert!((out["ece"].as_f64().unwrap() - summary.ece).abs() < 1e-12);
        assert_eq!(out["drifting"].as_array().unwrap().len(), 0);
        assert!(!out["bins"].as_array().unwrap().is_empty());
        assert_eq!(out["baseline_brier"].as_f64(), Some(0.30));

        let drift = handlers::calibration_drift(
            &serde_json::to_value(permanently_overconfident()).unwrap(),
            DEFAULT_DRIFT_TOLERANCE,
        )
        .expect("a drift list");
        assert_eq!(drift["drifting_count"].as_u64(), Some(1));
        assert_eq!(drift["drifting"][0], json!("safety"));
    }

    #[test]
    fn the_handlers_refuse_a_malformed_payload_and_a_bad_tolerance() {
        for payload in [json!({}), json!(null), json!(7), json!("points")] {
            let error = handlers::calibration_summary(&payload, 0.3).expect_err("a refusal");
            assert_eq!(error.code(), "json");
            assert_eq!(error.function(), COGNITION_CALIBRATION_SUMMARY);
        }
        // A well-shaped array holding an impossible point is the module's refusal, not
        // the payload's shape.
        let bad = json!([{ "class": "safety", "predicted": 2.0, "label": true }]);
        let error = handlers::calibration_summary(&bad, 0.3).expect_err("a refusal");
        assert_eq!(error.code(), "validation");
        assert_eq!(error.function(), COGNITION_CALIBRATION_SUMMARY);

        let good = json!([{ "class": "safety", "predicted": 0.5, "label": true }]);
        for tolerance in [-0.1, f64::NAN, f64::INFINITY] {
            let error = handlers::calibration_drift(&good, tolerance).expect_err("a refusal");
            assert_eq!(error.code(), "validation");
            assert_eq!(error.function(), COGNITION_CALIBRATION_DRIFT);
        }
        // The boundary itself is allowed.
        assert!(handlers::calibration_drift(&good, 0.0).is_ok());

        let refusal = CalibrationModuleError::refused(
            COGNITION_CALIBRATION_DRIFT,
            "validation",
            "tolerance was not a number",
        );
        let mm: MmError = refusal.into();
        assert!(mm.to_string().contains("validation"), "{mm}");
    }

    #[test]
    fn the_embedded_manifest_declares_this_module_and_its_functions() {
        let manifest: toml::Value = toml::from_str(MANIFEST_TOML).expect("plugin.toml parses");
        assert_eq!(manifest["plugin"]["name"].as_str(), Some("calibration"));
        assert_eq!(manifest["plugin"]["version"].as_str(), Some("0.1.0"));
        assert_eq!(
            manifest["plugin"]["uri"].as_str(),
            Some(module_iri().as_str()),
            "plugin.toml's uri has drifted from {MODULE_PATH}"
        );
        assert_eq!(manifest["metadata"]["category"].as_str(), Some("cognition"));
        assert_eq!(
            manifest["metadata"]["owned_by_phase"].as_integer(),
            Some(11)
        );
        assert_eq!(
            manifest["metadata"]["capability"].as_str(),
            Some(CAPABILITY)
        );

        let functions = manifest["tbox"]["functions"]
            .as_table()
            .expect("a tbox function table");
        assert_eq!(functions.len(), SURFACE_FUNCTIONS.len());
        for function in SURFACE_FUNCTIONS {
            assert!(functions.contains_key(function), "{function} is undeclared");
        }
        assert_eq!(
            manifest["tbox"]["functions"][COGNITION_CALIBRATION_SUMMARY]["source"].as_str(),
            Some(summary_handler())
        );
        assert_eq!(
            manifest["tbox"]["functions"][COGNITION_CALIBRATION_DRIFT]["source"].as_str(),
            Some(drift_handler())
        );
        assert_eq!(
            manifest["monad"]["operations"]["name"].as_str(),
            Some("cognition")
        );
    }

    #[test]
    fn the_module_touches_no_table_and_names_itself() {
        // The module is pure arithmetic, which is a contract a caller can rely on:
        // scoring a replayed episode needs no store.
        assert!(READS.is_empty());
        assert!(WRITES.is_empty());
        assert_eq!(functions(), SURFACE_FUNCTIONS.to_vec());
        assert_eq!(CAPABILITY, "mm:CalibrationSummary");
        assert_eq!(
            module_iri().as_str(),
            "https://metamind.dev/code/module/cognition/calibration"
        );
        assert_eq!(manifest_path().file_name().unwrap(), "plugin.toml");
        assert_eq!(
            module_dir().file_name().unwrap(),
            std::ffi::OsStr::new("calibration")
        );
    }
}
