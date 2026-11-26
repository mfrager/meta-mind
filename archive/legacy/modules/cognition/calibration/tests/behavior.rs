//! Module behavior test: the module reproduces the recorded calibration reference, its
//! drift list is what the gap asked for, and its two handlers agree with the typed API.
//!
//! These assertions are what make `mm:CalibrationSummary` a *tested* capability rather
//! than a declared one: the code graph records them as `mmc:hasTest` for the capability,
//! and `codex verify` refuses a capability that has none.
//!
//! The corpus is `bench/calibration/predictions.jsonl` and the reference is
//! `bench/calibration/reference.json`, both committed. The reference is produced by
//! `bench/calibration/compute_reference.py` — a second, independent implementation of
//! the documented metrics — so this test is a cross-check rather than a snapshot of
//! whatever this module happens to compute.

use calibration::{
    drifting_classes, handlers, parse_jsonl, score_at, summarize, CalibrationSummary, LabeledPoint,
    DEFAULT_DRIFT_TOLERANCE,
};

/// The committed corpus and its reference, read from the repository root.
fn corpus() -> (Vec<LabeledPoint>, serde_json::Value) {
    let root = mm_core::Config::repo_root();
    let text = std::fs::read_to_string(root.join("bench/calibration/predictions.jsonl"))
        .expect("bench/calibration/predictions.jsonl must exist");
    let reference = std::fs::read_to_string(root.join("bench/calibration/reference.json"))
        .expect("bench/calibration/reference.json must exist");
    (
        parse_jsonl(&text).expect("the corpus must parse"),
        serde_json::from_str(&reference).expect("the reference must parse"),
    )
}

/// The stored per-class temperatures, as `(class, temperature)` pairs.
fn reference_temperatures(reference: &serde_json::Value) -> Vec<(String, f64)> {
    reference["classes"]
        .as_object()
        .expect("a class map")
        .iter()
        .map(|(class, value)| {
            (
                class.clone(),
                value["temperature"].as_f64().expect("a temperature"),
            )
        })
        .collect()
}

/// The ECE reconstructed from the reported bins.
///
/// This is the module's own definition of the error, evaluated from the numbers an
/// operator actually reads, so a curve and a number that disagreed would be caught here
/// rather than in a disagreement about who was right.
fn ece_from_bins(summary: &CalibrationSummary) -> f64 {
    if summary.n == 0 {
        return 0.0;
    }
    summary
        .bins
        .iter()
        .map(|bin| {
            (f64::from(bin.n) / summary.n as f64)
                * (bin.observed_frequency - bin.mean_predicted).abs()
        })
        .sum()
}

#[test]
fn the_corpus_is_the_shape_the_reference_describes() {
    let (points, reference) = corpus();
    assert_eq!(points.len(), 100);
    assert_eq!(reference["n"].as_u64(), Some(100));
    assert_eq!(reference["bins"].as_u64(), Some(10));

    // Two classes, fifty points each, and every point a probability.
    let mut classes: Vec<&str> = points.iter().map(|p| p.class.as_str()).collect();
    classes.sort_unstable();
    classes.dedup();
    assert_eq!(classes, vec!["retrieval", "safety"]);
    for class in &classes {
        let count = points.iter().filter(|p| p.class == *class).count();
        assert_eq!(count, 50, "{class}");
    }
    for point in &points {
        assert!(
            (0.0..=1.0).contains(&point.predicted),
            "{point:?} is not a probability"
        );
    }
    // The reference's per-class point counts are the corpus's.
    for class in &classes {
        let value = &reference["classes"][*class];
        assert_eq!(value["n"].as_u64(), Some(50), "{class}");
    }
}

#[test]
fn the_module_reproduces_the_recorded_reference_metrics() {
    let (points, reference) = corpus();

    // 1. The arithmetic, given the reference's own temperatures. This is the check that
    //    grades the metrics.
    //
    //    The tolerance is 1e-7 rather than the 1e-9 a reference usually wants, and the
    //    reason is a real difference rather than slop: the corpus stores its
    //    probabilities as the `f32` the kernel's `Prediction` type holds, while
    //    `compute_reference.py` computes in `f64` from the shortest decimal form. The
    //    f32 representation of 0.98 is 0.9800000190734863, so the two implementations
    //    are scoring inputs that differ by 1.9e-8; the metrics inherit that, and a
    //    1e-9 assertion would be asserting something false. Exactness is asserted
    //    where it is achievable — against hand-computed vectors at `T = 1`, in the
    //    module's own unit tests.
    let given = score_at(&points, &reference_temperatures(&reference));
    let expected_brier = reference["brier"].as_f64().unwrap();
    let expected_log_loss = reference["log_loss"].as_f64().unwrap();
    let expected_ece = reference["ece"].as_f64().unwrap();
    assert!(
        (given.brier - expected_brier).abs() < 1e-7,
        "brier {} vs reference {expected_brier}",
        given.brier
    );
    assert!(
        (given.log_loss - expected_log_loss).abs() < 1e-7,
        "log loss {} vs reference {expected_log_loss}",
        given.log_loss
    );
    assert!(
        (given.ece - expected_ece).abs() < 1e-7,
        "ece {} vs reference {expected_ece}",
        given.ece
    );
    assert_eq!(given.n, 100);

    // 2. The fit. Each class's temperature is within the grid's own resolution of the
    //    reference's, which for a log-spaced grid of 2000 points over [0.05, 20] means
    //    the two implementations took the same grid point (adjacent points differ by
    //    0.3%, ~0.0085 at T ≈ 2.8, an order of magnitude above 1e-3).
    let fitted = calibration::fitted_temperatures(&points);
    for (class, expected) in reference_temperatures(&reference) {
        let actual = fitted.get(&class).copied().expect("a fitted temperature");
        assert!(
            (actual - expected).abs() < 1e-3,
            "{class}: fitted {actual} vs reference {expected}"
        );
        assert!(
            actual > 1.0,
            "{class} is overconfident and must be softened"
        );
    }

    // 3. The whole summary. Its metrics are within 1e-4 of the reference: the residual
    //    is the metric's sensitivity to a temperature difference of up to 1e-3, which
    //    is ~3.2e-5 in the Brier score.
    let summary = summarize(&points, reference["baseline_brier"].as_f64().unwrap());
    assert!(
        (summary.brier - expected_brier).abs() < 1e-4,
        "{} vs {expected_brier}",
        summary.brier
    );
    assert!((summary.log_loss - expected_log_loss).abs() < 1e-4);
    // The ECE is flatter near the fit than the Brier score, so a temperature difference
    // of up to 1e-3 moves it by less than the scaled probability moves: ~8e-5.
    assert!(
        (summary.ece - expected_ece).abs() < 1e-3,
        "ece {} vs reference {expected_ece}",
        summary.ece
    );
    assert_eq!(summary.n, 100);
    assert_eq!(
        summary.baseline_brier,
        reference["baseline_brier"].as_f64().unwrap()
    );

    // The gate's own thresholds: the calibrated corpus passes them, and the recorded
    // baseline is beaten. Both halves matter — a summary that passed the first by
    // chance would still have to explain the second.
    assert!(summary.brier <= 0.20, "{}", summary.brier);
    assert!(summary.ece <= 0.10, "{}", summary.ece);
    assert!(
        summary.brier < summary.baseline_brier,
        "{} must beat the baseline {}",
        summary.brier,
        summary.baseline_brier
    );
    let before = reference["brier_before"].as_f64().unwrap();
    assert!(
        summary.brier < before,
        "the fit must improve on the raw score: {} vs {before}",
        summary.brier
    );
}

#[test]
fn the_drift_list_is_empty_after_fitting_and_was_not_before() {
    let (points, _) = corpus();

    // This is the gap's question — "is a class overconfident in a way an operator can
    // see" — asked twice: once of the raw corpus, once of the fitted one.
    let raw = score_at(&points, &[]);
    assert!(
        raw.ece > DEFAULT_DRIFT_TOLERANCE,
        "the seeded corpus is overconfident before fitting: {}",
        raw.ece
    );
    assert_eq!(
        drifting_classes(&points, DEFAULT_DRIFT_TOLERANCE),
        Vec::<String>::new(),
        "one temperature per class repairs both"
    );

    // A class no temperature can repair is named, and the list is sorted by class.
    let mut hopeless = Vec::new();
    for (index, class) in ["zulu", "alpha"].into_iter().enumerate() {
        for step in 0..50 {
            hopeless.push(LabeledPoint {
                class: class.to_string(),
                predicted: 0.999,
                label: (step + index) % 2 == 0,
            });
        }
    }
    assert_eq!(
        drifting_classes(&hopeless, DEFAULT_DRIFT_TOLERANCE),
        vec!["alpha".to_string(), "zulu".to_string()]
    );
    assert!(drifting_classes(&hopeless, 1.0).is_empty());
}

#[test]
fn the_reported_curve_and_the_reported_error_agree() {
    let (points, _) = corpus();
    let summary = summarize(&points, 0.30);
    assert!(!summary.bins.is_empty());
    // Bins ascend and do not overlap.
    for pair in summary.bins.windows(2) {
        assert!(pair[0].upper <= pair[1].lower, "{:?}", summary.bins);
    }
    // Every bin's edges are in [0,1] and its observed frequency is a frequency.
    for bin in &summary.bins {
        assert!(bin.lower >= 0.0 && bin.upper <= 1.0);
        assert!(bin.n > 0);
        assert!((0.0..=1.0).contains(&bin.observed_frequency));
        assert!((0.0..=1.0).contains(&bin.mean_predicted));
    }
    // The curve covers every point.
    let covered: u32 = summary.bins.iter().map(|bin| bin.n).sum();
    assert_eq!(covered as usize, summary.n);

    // The number and the picture come from the same values, so rebuilding the error from
    // the bins reproduces the metric — to `f32` precision, not to 1e-9. The metric is
    // `mm_decision`'s, which sums in `f32` and widens afterwards, while this rebuild sums
    // in `f64`; demanding more would be demanding that two different roundings agree bit
    // for bit, which is not a property of the arithmetic. 1e-6 is ~70 `f32` ulps here.
    let rebuilt = ece_from_bins(&summary);
    assert!(
        (rebuilt - summary.ece).abs() < 1e-6,
        "bins rebuild to {rebuilt}, the metric says {}",
        summary.ece
    );
}

#[test]
fn the_handlers_answer_the_corpus_with_the_same_numbers() {
    let (points, _) = corpus();
    let as_json = serde_json::to_value(&points).expect("the corpus serializes");

    let out = handlers::calibration_summary(&as_json, 0.30).expect("a summary");
    let summary = summarize(&points, 0.30);
    assert_eq!(out["n"].as_u64(), Some(summary.n as u64));
    assert!((out["brier"].as_f64().unwrap() - summary.brier).abs() < 1e-12);
    assert!((out["log_loss"].as_f64().unwrap() - summary.log_loss).abs() < 1e-12);
    assert!((out["ece"].as_f64().unwrap() - summary.ece).abs() < 1e-12);
    assert_eq!(out["bins"].as_array().unwrap().len(), summary.bins.len());
    assert_eq!(out["drifting"].as_array().unwrap().len(), 0);
    assert_eq!(out["baseline_brier"].as_f64(), Some(0.30));

    // The whole-set metrics the gate asserts, read back from the handler's own output.
    assert!(out["brier"].as_f64().unwrap() <= 0.20);
    assert!(out["ece"].as_f64().unwrap() <= 0.10);

    let drift =
        handlers::calibration_drift(&as_json, DEFAULT_DRIFT_TOLERANCE).expect("a drift list");
    assert_eq!(drift["n"].as_u64(), Some(100));
    assert_eq!(drift["drifting_count"].as_u64(), Some(0));
    assert_eq!(drift["drifting"], serde_json::json!([]));
}

#[test]
fn the_manifest_declares_a_module_the_scanner_can_register() {
    let root = mm_core::Config::repo_root();
    let manifest = calibration::manifest_path();
    assert!(manifest.starts_with(root.join("modules/cognition/calibration")));
    // The path the gap names as its target is this module's own directory, which is what
    // makes the gap closed rather than merely described.
    let gap = std::fs::read_to_string(root.join("bench/gaps/gap_01.json")).expect("the gap");
    let gap: serde_json::Value = serde_json::from_str(&gap).expect("the gap parses");
    assert_eq!(
        gap["target_uri"].as_str(),
        Some(calibration::module_iri().as_str())
    );
    // Two spellings of the same place, and both are asserted: the gap names the module's
    // *repository* path, the module declares its IRI-relative path (the one an IRI is
    // derived from), and they agree when the first is the second under `modules/`.
    assert_eq!(
        gap["target_path"].as_str(),
        Some(format!("modules/{}", calibration::MODULE_PATH).as_str())
    );
}
