//! The benchmark: the frozen harness a candidate is measured against.
//!
//! The phase's risk table names "reward hacking / self-confirming evaluation" and the
//! mitigation is a harness that is *frozen within a cycle*: the candidate is measured
//! by a command the candidate did not write, in a directory the candidate cannot edit,
//! against a baseline recorded in advance. Three consequences shape this module:
//!
//! * **A spec carries its baseline and its noise margin.** The baseline is data, not a
//!   second run of the candidate, because "better than last time" and "better than the
//!   frozen baseline" are different claims and only the second survives a candidate
//!   that makes everything slower.
//! * **Every sample counts.** The command may print several numbers; the candidate's
//!   value is their mean and `n` is how many there were. A single sample and a reader
//!   that took the *best* sample are how a benchmark stops measuring.
//! * **`within_noise` is computed here, not by the gate.** The gate's rule 2 asks
//!   whether the candidate is worse *beyond* the margin; deciding that in one place
//!   keeps the boundary from being re-derived (and re-derived differently) later.
//!
//! The command runs through `/bin/sh -c`, so a spec can be a pipeline without this
//! module growing a shell. It runs with the candidate's own working directory, so what
//! it measures is the candidate's tree.

use std::path::{Path, PathBuf};

use mm_log::{codes, Level, Logger};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::changeset::BenchmarkId;
use crate::error::{Result, SelfEngError};

/// One benchmark: what to run, and what it is compared against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchSpec {
    /// Its identifier, which the change set names in `benchmarks`.
    pub id: BenchmarkId,
    /// A human name.
    pub name: String,
    /// The command, run through `/bin/sh -c`.
    pub command: String,
    /// The frozen baseline the candidate is compared with.
    pub baseline_value: f64,
    /// How much of a difference is noise rather than progress.
    pub noise_margin: f64,
}

impl BenchSpec {
    /// Read a JSON array of specs.
    ///
    /// Unknown keys are ignored so a fixture can carry a `_comment` explaining itself.
    pub fn from_json_array(text: &str) -> Result<Vec<BenchSpec>> {
        let specs: Vec<BenchSpec> = serde_json::from_str(text).map_err(|e| {
            SelfEngError::Bench(format!("the benchmark file is not an array of specs: {e}"))
        })?;
        if specs.is_empty() {
            return Err(SelfEngError::Bench(
                "the benchmark file lists no spec".into(),
            ));
        }
        for spec in &specs {
            spec.validate()?;
        }
        Ok(specs)
    }

    /// Refuse a spec that cannot measure anything.
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(SelfEngError::validation(
                "benchmark.id",
                "must not be empty",
            ));
        }
        if self.command.trim().is_empty() {
            return Err(SelfEngError::validation(
                "benchmark.command",
                "must not be empty",
            ));
        }
        if !self.baseline_value.is_finite() {
            return Err(SelfEngError::validation(
                "benchmark.baseline_value",
                format!("must be finite, got {}", self.baseline_value),
            ));
        }
        if !self.noise_margin.is_finite() || self.noise_margin < 0.0 {
            return Err(SelfEngError::validation(
                "benchmark.noise_margin",
                format!("must be finite and non-negative, got {}", self.noise_margin),
            ));
        }
        Ok(())
    }
}

/// What a benchmark run produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchResult {
    /// The candidate's mean over its samples.
    pub candidate: f64,
    /// The frozen baseline.
    pub baseline: f64,
    /// `candidate - baseline`. Negative is worse when lower is better, which is the
    /// convention the specs in `bench/promotion/` use.
    pub delta: f64,
    /// How many samples the command printed.
    pub n: u32,
    /// True when the difference is inside the noise margin.
    pub within_noise: bool,
}

impl BenchResult {
    /// True when the candidate is worse than the baseline by more than the margin.
    pub fn is_worse_beyond_noise(&self) -> bool {
        self.delta < 0.0 && !self.within_noise
    }

    /// The stable rendering the evidence column stores.
    pub fn canonical(&self) -> String {
        format!(
            "candidate={:.9},baseline={:.9},delta={:.9},n={},within_noise={}",
            self.candidate, self.baseline, self.delta, self.n, self.within_noise
        )
    }
}

/// Read the specs from a file.
pub fn load_specs(path: &Path) -> Result<Vec<BenchSpec>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| SelfEngError::Bench(format!("cannot read {}: {e}", path.display())))?;
    BenchSpec::from_json_array(&text)
}

/// Run one benchmark in `cwd` and compare it with its baseline.
///
/// `change_set_id` is not a parameter: the sandbox directory a benchmark runs in is
/// named by the change set's ULID, so the `bench.run` record's `dir` field is the
/// correlation, and inventing a second one would let the two disagree.
pub async fn run_benchmark(spec: &BenchSpec, cwd: &Path, logger: &Logger) -> Result<BenchResult> {
    spec.validate()?;
    let output = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&spec.command)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| SelfEngError::Bench(format!("cannot run the benchmark: {e}")))?;
    if !output.status.success() {
        return Err(SelfEngError::Build {
            dir: cwd.display().to_string(),
            command: format!("benchmark {}", spec.id),
            exit_code: output.status.code().unwrap_or(-1),
        });
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let samples = parse_samples(&text);
    if samples.is_empty() {
        return Err(SelfEngError::Bench(format!(
            "benchmark {} printed no number",
            spec.id
        )));
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    if !mean.is_finite() {
        return Err(SelfEngError::Bench(format!(
            "benchmark {} averaged to {}",
            spec.id, mean
        )));
    }
    let delta = mean - spec.baseline_value;
    let result = BenchResult {
        candidate: mean,
        baseline: spec.baseline_value,
        delta,
        n: samples.len() as u32,
        within_noise: delta.abs() <= spec.noise_margin,
    };
    logger
        .audit(
            Level::Info,
            codes::BENCH_RUN,
            crate::TARGET,
            None,
            json!({
                "benchmark": spec.id.as_str(),
                "name": spec.name,
                "dir": cwd.display().to_string(),
                "baseline": result.baseline,
                "candidate": result.candidate,
                "delta": result.delta,
                "n": result.n,
                "noise_margin": spec.noise_margin,
                "within_noise": result.within_noise,
            }),
        )
        .await?;
    Ok(result)
}

/// Every number the command printed, in order.
///
/// The whole line must be one number: a command that prints `candidate 0.5` is not
/// printing a sample, and accepting it would let a log line change the measurement.
pub fn parse_samples(text: &str) -> Vec<f64> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.parse::<f64>().ok())
        .collect()
}

/// The benchmark directory of the repository.
pub fn bench_dir() -> PathBuf {
    mm_core::Config::repo_root().join("bench").join("promotion")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Config;
    use mm_store_sqlite::SqliteStore;

    fn spec() -> BenchSpec {
        BenchSpec {
            id: BenchmarkId::new("promotion_bench"),
            name: "the frozen promotion bench".to_string(),
            command: "printf '0.5\\n'".to_string(),
            baseline_value: 0.5,
            noise_margin: 0.01,
        }
    }

    /// A logger whose audit writer is a migrated store in `dir`.
    ///
    /// Async, not a blocking helper: every caller is already inside a tokio test, and
    /// building a second runtime there panics rather than running anything.
    async fn logger(dir: &Path) -> std::sync::Arc<Logger> {
        let store = SqliteStore::open(&dir.join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        let cfg = Config::for_data_dir(dir);
        std::sync::Arc::new(
            Logger::from_config(&cfg.log, Some(std::sync::Arc::new(store))).unwrap(),
        )
    }

    #[test]
    fn samples_are_whole_lines_that_parse() {
        assert_eq!(parse_samples("0.5\n0.25\n"), vec![0.5, 0.25]);
        assert_eq!(parse_samples("0.5\ncandidate 0.9\n\n"), vec![0.5]);
        assert!(parse_samples("no numbers here").is_empty());
        assert_eq!(parse_samples(" -1.5 \n"), vec![-1.5]);
    }

    #[test]
    fn a_spec_that_cannot_measure_is_refused() {
        let mut bad = spec();
        bad.command = "  ".into();
        assert!(bad.validate().is_err());
        let mut bad = spec();
        bad.baseline_value = f64::NAN;
        assert!(bad.validate().is_err());
        let mut bad = spec();
        bad.noise_margin = -0.1;
        assert!(bad.validate().is_err());
        assert!(BenchSpec::from_json_array("[]").is_err());
    }

    #[test]
    fn the_noise_boundary_is_inclusive() {
        let mut result = BenchResult {
            candidate: 0.49,
            baseline: 0.5,
            delta: -0.01,
            n: 1,
            within_noise: true,
        };
        assert!(
            !result.is_worse_beyond_noise(),
            "exactly the margin is noise"
        );
        result.within_noise = false;
        assert!(result.is_worse_beyond_noise());
        result.delta = 0.5;
        result.within_noise = false;
        assert!(!result.is_worse_beyond_noise(), "better is never worse");
    }

    #[tokio::test]
    async fn a_run_averages_its_samples_and_compares_them_with_the_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let logger = logger(dir.path()).await;
        let mut spec = spec();
        spec.command = "printf '0.4\\n0.6\\n'".to_string();
        let result = run_benchmark(&spec, dir.path(), &logger).await.unwrap();
        assert!((result.candidate - 0.5).abs() < 1e-9);
        assert_eq!(result.n, 2);
        assert_eq!(result.baseline, 0.5);
        assert!(result.within_noise);
        assert!(result.canonical().contains("n=2"));
    }

    #[tokio::test]
    async fn a_command_that_prints_nothing_is_a_named_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let logger = logger(dir.path()).await;
        let mut spec = spec();
        spec.command = "true".to_string();
        let error = run_benchmark(&spec, dir.path(), &logger).await.unwrap_err();
        assert_eq!(error.code(), "bench");
        assert!(error.to_string().contains("no number"), "{error}");
    }

    #[tokio::test]
    async fn a_failing_command_is_a_build_refusal_not_a_zero() {
        let dir = tempfile::tempdir().unwrap();
        let logger = logger(dir.path()).await;
        let mut spec = spec();
        spec.command = "exit 3".to_string();
        let error = run_benchmark(&spec, dir.path(), &logger).await.unwrap_err();
        assert_eq!(error.code(), "sandbox.build");
    }

    #[test]
    fn the_committed_promotion_bench_loads_and_is_frozen() {
        let specs = load_specs(&bench_dir().join("promotion_bench.json")).unwrap();
        assert!(!specs.is_empty());
        for spec in &specs {
            spec.validate().unwrap();
            assert!(spec.noise_margin >= 0.0);
        }
        assert!(
            specs.iter().any(|s| s.id.as_str() == "promotion_bench"),
            "the gate's fixture must be the one the plans name"
        );
    }
}
