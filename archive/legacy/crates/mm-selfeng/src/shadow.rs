//! The shadow: what the candidate does that the system did not.
//!
//! A benchmark says whether the candidate is *better*; a shadow says whether it is
//! *different*. The plan asks for both because a change can pass a benchmark and still
//! change behaviour on cases nobody thought to measure — and the promotion gate is the
//! only place that difference is visible before production sees it.
//!
//! Three decisions are worth stating:
//!
//! * **Comparison is exact, not scored.** Two outputs are the same or they are not.
//!   A similarity threshold would turn "did the candidate change its answer" into a
//!   question about the threshold, and the gate's job is to notice a difference rather
//!   than to quantify it.
//! * **A different number of cases is itself a divergence.** If the baseline answered
//!   five cases and the candidate answered four, the missing answer is the finding, so
//!   the count mismatch is reported as one divergence with the two counts in the
//!   detail rather than silently comparing the common prefix.
//! * **The report is deterministic.** Cases are run in the order given, and the detail
//!   names the first divergence, so the same tree produces the same report.

use std::path::Path;

use mm_log::{codes, Level, Logger};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{Result, SelfEngError};

/// What a shadow run found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowReport {
    /// How many cases were compared.
    pub runs: u32,
    /// How many behaved differently.
    pub divergences: u32,
    /// A readable summary naming the first difference.
    pub detail: String,
}

impl ShadowReport {
    /// True when the candidate behaved the same everywhere.
    pub fn is_identical(&self) -> bool {
        self.divergences == 0
    }

    /// The stable rendering the evidence column stores.
    pub fn canonical(&self) -> String {
        format!(
            "runs={},divergences={},detail={}",
            self.runs, self.divergences, self.detail
        )
    }
}

/// Compare two answer sequences.
///
/// The pairs are compared index by index over the *common prefix*, and a length
/// mismatch adds one divergence — the plan's requirement is to notice that the
/// candidate changed, and a candidate that answers fewer cases did change.
pub fn compare(baseline_outputs: &[String], candidate_outputs: &[String]) -> ShadowReport {
    let common = baseline_outputs.len().min(candidate_outputs.len());
    let mut divergences = 0_u32;
    let mut first: Option<String> = None;
    for index in 0..common {
        if baseline_outputs[index] != candidate_outputs[index] {
            divergences += 1;
            if first.is_none() {
                first = Some(format!(
                    "case {index}: baseline {:?} vs candidate {:?}",
                    baseline_outputs[index], candidate_outputs[index]
                ));
            }
        }
    }
    if baseline_outputs.len() != candidate_outputs.len() {
        divergences += 1;
        if first.is_none() {
            first = Some(format!(
                "case count: baseline {} vs candidate {}",
                baseline_outputs.len(),
                candidate_outputs.len()
            ));
        }
    }
    let detail = match first {
        Some(text) => text,
        None => format!("{common} case(s) identical"),
    };
    ShadowReport {
        runs: candidate_outputs.len() as u32,
        divergences,
        detail,
    }
}

/// Run the same cases against two trees and compare what they answered.
///
/// Each case is a shell command run with `/bin/sh -c` in both directories, exactly as
/// a benchmark is, so the shadow measures the tree rather than a harness this module
/// invented.
pub async fn shadow(
    candidate: &Path,
    baseline: &Path,
    cases: &[String],
    logger: &Logger,
) -> Result<ShadowReport> {
    let mut baseline_outputs = Vec::with_capacity(cases.len());
    let mut candidate_outputs = Vec::with_capacity(cases.len());
    for case in cases {
        baseline_outputs.push(run_case(baseline, case).await?);
        candidate_outputs.push(run_case(candidate, case).await?);
    }
    let report = compare(&baseline_outputs, &candidate_outputs);
    logger
        .audit(
            Level::Info,
            codes::SHADOW_START,
            crate::TARGET,
            None,
            json!({
                "candidate": candidate.display().to_string(),
                "baseline": baseline.display().to_string(),
                "runs": report.runs,
                "divergences": report.divergences,
                "detail": report.detail,
            }),
        )
        .await?;
    Ok(report)
}

async fn run_case(dir: &Path, case: &str) -> Result<String> {
    let output = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(case)
        .current_dir(dir)
        .output()
        .await
        .map_err(|e| SelfEngError::Sandbox(format!("cannot run a shadow case: {e}")))?;
    let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        text = format!("{} (exit {})", text, output.status.code().unwrap_or(-1));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Config;

    /// A logger whose audit writer is a migrated store in `dir`.
    ///
    /// `shadow.start` is audited, so a logger without a writer would fail the record
    /// rather than the behaviour under test. Async because every caller is inside a tokio
    /// test, where building a second runtime panics.
    async fn logger(dir: &Path) -> Logger {
        let store = mm_store_sqlite::SqliteStore::open(&dir.join("mm.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let cfg = Config::for_data_dir(dir);
        Logger::from_config(&cfg.log, Some(std::sync::Arc::new(store))).unwrap()
    }

    #[test]
    fn identical_outputs_diverge_nowhere() {
        let a = vec!["1".to_string(), "2".to_string()];
        let b = vec!["1".to_string(), "2".to_string()];
        let report = compare(&a, &b);
        assert_eq!(report.runs, 2);
        assert_eq!(report.divergences, 0);
        assert!(report.is_identical());
        assert!(report.detail.contains("identical"), "{}", report.detail);
    }

    #[test]
    fn one_different_answer_is_one_divergence_naming_the_case() {
        let a = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let b = vec!["1".to_string(), "different".to_string(), "3".to_string()];
        let report = compare(&a, &b);
        assert_eq!(report.divergences, 1);
        assert!(report.detail.contains("case 1"), "{}", report.detail);
        assert!(!report.is_identical());
    }

    #[test]
    fn a_changed_case_count_is_itself_a_divergence() {
        let a = vec!["1".to_string(), "2".to_string()];
        let b = vec!["1".to_string()];
        let report = compare(&a, &b);
        assert_eq!(report.divergences, 1);
        assert!(report.detail.contains("count"), "{}", report.detail);
        assert_eq!(report.runs, 1);
        // And it does not hide a real difference in the common prefix.
        let c = vec!["other".to_string()];
        assert_eq!(compare(&a, &c).divergences, 2);
    }

    #[test]
    fn an_empty_comparison_is_identical_with_a_zero_run_count() {
        let report = compare(&[], &[]);
        assert_eq!(report.runs, 0);
        assert!(report.is_identical());
    }

    #[tokio::test]
    async fn a_shadow_run_compares_two_directories() {
        let dir = tempfile::tempdir().unwrap();
        let baseline = dir.path().join("baseline");
        let candidate = dir.path().join("candidate");
        std::fs::create_dir_all(&baseline).unwrap();
        std::fs::create_dir_all(&candidate).unwrap();
        std::fs::write(baseline.join("answer.txt"), "yes").unwrap();
        std::fs::write(candidate.join("answer.txt"), "no").unwrap();
        let cases = vec!["cat answer.txt".to_string()];
        let report = shadow(&candidate, &baseline, &cases, &logger(dir.path()).await)
            .await
            .unwrap();
        assert_eq!(report.runs, 1);
        assert_eq!(report.divergences, 1);
        assert!(report.detail.contains("yes"), "{}", report.detail);
    }

    #[tokio::test]
    async fn a_failing_case_is_reported_with_its_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let baseline = dir.path().join("baseline");
        let candidate = dir.path().join("candidate");
        std::fs::create_dir_all(&baseline).unwrap();
        std::fs::create_dir_all(&candidate).unwrap();
        let cases = vec!["exit 4".to_string()];
        let report = shadow(&candidate, &baseline, &cases, &logger(dir.path()).await)
            .await
            .unwrap();
        assert_eq!(report.divergences, 0, "both failed the same way");
        let cases = vec!["printf hi".to_string()];
        let report = shadow(&candidate, &baseline, &cases, &logger(dir.path()).await)
            .await
            .unwrap();
        assert!(report.is_identical(), "{report:?}");
    }
}
