//! What `mm-mistakes` refuses, and why.
//!
//! Every refusal here is one an operator can act on, and the two that matter most are
//! named rather than collapsed into a boolean:
//!
//! * [`MistakeError::NotFailingBefore`] — the generated test passed on the buggy code.
//!   That is the failure mode of a *regression test* compiler: a test that never failed
//!   is not evidence of a fix, and reporting it as success would make the whole phase's
//!   gate meaningless.
//! * [`MistakeError::NotPassingAfter`] — the test still fails once the fix is applied,
//!   so the minimized incident was not actually the bug (or the fix is not the fix).
//!
//! The rest are the ordinary edges: a mistake that is not recorded, a suite directory
//! that cannot be read, a case that names no script, a script that cannot be spawned, a
//! script that does not finish. A single `Error(String)` would make `regression run`
//! print the same thing for "this case is broken" and "this case proves the bug is
//! still there".

use std::path::PathBuf;

/// Everything this crate can refuse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MistakeError {
    /// A field failed its own validation.
    #[error("{field}: {detail}")]
    Validation {
        /// The field that failed.
        field: &'static str,
        /// What was wrong with it.
        detail: String,
    },
    /// No mistake row has that ULID.
    #[error("no mistake {0} is recorded in `mistakes`")]
    NoSuchMistake(String),
    /// The suite directory could not be read.
    #[error("cannot read the regression suite {path}: {detail}")]
    SuiteUnreadable {
        /// The suite directory.
        path: PathBuf,
        /// What went wrong.
        detail: String,
    },
    /// The case directory could not be read, or its `case.json` is not a case.
    #[error("cannot read the regression case {path}: {detail}")]
    CaseUnreadable {
        /// The case directory (or the file that failed).
        path: PathBuf,
        /// What went wrong.
        detail: String,
    },
    /// A case names a script that is not there.
    #[error("case {case} names {script}, which is not in {path}")]
    MissingScript {
        /// The case's name.
        case: String,
        /// The script the case named.
        script: String,
        /// The case directory.
        path: PathBuf,
    },
    /// The test passed on the buggy code, so it is not a regression test.
    #[error("case {case} did not fail before the fix (check.sh exited {code})")]
    NotFailingBefore {
        /// The case's name.
        case: String,
        /// The exit code `check.sh` returned.
        code: i32,
    },
    /// The test still fails after the fix.
    #[error("case {case} did not pass after the fix (check.sh exited {code})")]
    NotPassingAfter {
        /// The case's name.
        case: String,
        /// The exit code `check.sh` returned.
        code: i32,
    },
    /// A command could not be spawned.
    #[error("cannot run {command}: {detail}")]
    Spawn {
        /// The command that was attempted.
        command: String,
        /// What went wrong.
        detail: String,
    },
    /// A script did not finish in its budget.
    #[error("case {case}: {script} did not finish within {seconds}s")]
    Timeout {
        /// The case's name.
        case: String,
        /// The script that hung.
        script: String,
        /// The budget it was given.
        seconds: u64,
    },
    /// A document could not be read or written.
    #[error("codec: {0}")]
    Codec(String),
    /// The store refused a write or a read.
    #[error("store: {0}")]
    Store(String),
    /// An internal invariant did not hold.
    #[error("internal: {0}")]
    Internal(String),
}

impl MistakeError {
    /// A validation refusal.
    pub fn validation(field: &'static str, detail: impl Into<String>) -> Self {
        MistakeError::Validation {
            field,
            detail: detail.into(),
        }
    }

    /// A case that could not be read, with the path and the cause.
    pub fn case_unreadable(path: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        MistakeError::CaseUnreadable {
            path: path.into(),
            detail: detail.into(),
        }
    }

    /// A suite that could not be read, with the path and the cause.
    pub fn suite_unreadable(path: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        MistakeError::SuiteUnreadable {
            path: path.into(),
            detail: detail.into(),
        }
    }

    /// The stable code `mm-cli` reports for this refusal.
    pub fn code(&self) -> &'static str {
        match self {
            MistakeError::Validation { .. } => "mistake.validation",
            MistakeError::NoSuchMistake(_) => "mistake.not_found",
            MistakeError::SuiteUnreadable { .. } => "regression.suite_unreadable",
            MistakeError::CaseUnreadable { .. } => "regression.case_unreadable",
            MistakeError::MissingScript { .. } => "regression.missing_script",
            MistakeError::NotFailingBefore { .. } => "regression.not_failing_before",
            MistakeError::NotPassingAfter { .. } => "regression.not_passing_after",
            MistakeError::Spawn { .. } => "regression.spawn",
            MistakeError::Timeout { .. } => "regression.timeout",
            MistakeError::Codec(_) => "codec",
            MistakeError::Store(_) => "store",
            MistakeError::Internal(_) => "internal",
        }
    }

    /// True when the refusal is a case that did not hold its half of the contract,
    /// which `regression run` reports per case rather than as a whole-suite failure.
    pub fn is_contract_failure(&self) -> bool {
        matches!(
            self,
            MistakeError::NotFailingBefore { .. } | MistakeError::NotPassingAfter { .. }
        )
    }
}

impl From<serde_json::Error> for MistakeError {
    fn from(e: serde_json::Error) -> Self {
        MistakeError::Codec(format!("serialization: {e}"))
    }
}

impl From<std::io::Error> for MistakeError {
    fn from(e: std::io::Error) -> Self {
        MistakeError::Internal(format!("io: {e}"))
    }
}

impl From<mm_core::MmError> for MistakeError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(m) => MistakeError::Store(m),
            other => MistakeError::Internal(other.to_string()),
        }
    }
}

impl From<mm_memory::MemoryError> for MistakeError {
    fn from(e: mm_memory::MemoryError) -> Self {
        MistakeError::Store(e.to_string())
    }
}

impl From<MistakeError> for mm_core::MmError {
    fn from(e: MistakeError) -> Self {
        match e {
            MistakeError::Store(m) => mm_core::MmError::Store(m),
            other => mm_core::MmError::Internal(format!("{} ({})", other, other.code())),
        }
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, MistakeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_has_a_distinct_code() {
        let cases = [
            MistakeError::validation("failure_mode", "empty"),
            MistakeError::NoSuchMistake("01h".into()),
            MistakeError::suite_unreadable("/s", "missing"),
            MistakeError::case_unreadable("/c", "missing"),
            MistakeError::MissingScript {
                case: "c".into(),
                script: "check.sh".into(),
                path: PathBuf::from("/c"),
            },
            MistakeError::NotFailingBefore {
                case: "c".into(),
                code: 0,
            },
            MistakeError::NotPassingAfter {
                case: "c".into(),
                code: 1,
            },
            MistakeError::Spawn {
                command: "sh".into(),
                detail: "no such file".into(),
            },
            MistakeError::Timeout {
                case: "c".into(),
                script: "check.sh".into(),
                seconds: 10,
            },
            MistakeError::Codec("bad json".into()),
            MistakeError::Store("closed".into()),
            MistakeError::Internal("bug".into()),
        ];
        let codes: std::collections::BTreeSet<&str> = cases.iter().map(|c| c.code()).collect();
        assert_eq!(codes.len(), cases.len(), "codes must distinguish refusals");
    }

    #[test]
    fn only_the_two_contract_failures_report_as_contract_failures() {
        assert!(MistakeError::NotFailingBefore {
            case: "c".into(),
            code: 0
        }
        .is_contract_failure());
        assert!(MistakeError::NotPassingAfter {
            case: "c".into(),
            code: 1
        }
        .is_contract_failure());
        assert!(!MistakeError::NoSuchMistake("x".into()).is_contract_failure());
    }

    #[test]
    fn a_contract_failure_names_the_case_and_the_exit_code() {
        let rendered = MistakeError::NotPassingAfter {
            case: "seeded_bug_01".into(),
            code: 3,
        }
        .to_string();
        assert!(rendered.contains("seeded_bug_01"), "{rendered}");
        assert!(rendered.contains('3'), "{rendered}");
    }
}
