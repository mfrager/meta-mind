//! The data contracts shared by Interpret, Process, Verify, and the judge.

use mm_agentstream::CleanLog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::budget::BudgetClass;

/// One verification check registered by the Interpret pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyCheck {
    /// Unique label within the turn.
    pub label: String,
    /// Optional human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Additional detail for the backend.
    #[serde(default)]
    pub details: Option<String>,
    /// Backend identifier (`llm_judge`).
    pub backend: String,
    /// Backend-specific configuration.
    #[serde(default = "empty_object")]
    pub backend_params: Value,
    /// Acceptance threshold in `[0, 1]`.
    #[serde(default = "default_threshold")]
    pub confidence_threshold: f64,
}

fn default_threshold() -> f64 {
    0.5
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

impl Default for VerifyCheck {
    fn default() -> Self {
        Self {
            label: String::new(),
            description: None,
            details: None,
            backend: String::new(),
            backend_params: Value::Null,
            confidence_threshold: default_threshold(),
        }
    }
}

/// The Process pass output a check may inspect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResult {
    /// The user-facing reply.
    pub reply_text: String,
    /// Tool calls the Process pass made, as recorded events.
    #[serde(default)]
    pub tool_calls: Vec<Value>,
}

/// The per-turn context supplied to backends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VerifyContext {
    /// User messages and approved final replies only.
    pub clean_log: CleanLog,
    /// The canonical `[STATE]` rendering for this turn.
    pub state_summary: String,
    /// The operator's request.
    pub user_request: String,
    /// Checks registered by Interpret.
    #[serde(default)]
    pub interpret_checks: Vec<VerifyCheck>,
}

/// One backend's verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationResult {
    /// The check label this answers.
    pub check_label: String,
    /// Whether the check passed.
    pub pass: bool,
    /// The backend's reasoning.
    pub reasoning: String,
    /// Confidence in `[0, 1]`, clamped.
    pub confidence: f64,
    /// Backend identifier.
    pub backend: String,
    /// Wall time for this check.
    pub elapsed_ms: u64,
    /// A backend or protocol failure, distinct from `pass = false`.
    #[serde(default)]
    pub error: Option<String>,
}

impl VerificationResult {
    /// A failed check caused by a backend or protocol error.
    pub fn failed(check: &VerifyCheck, error: impl Into<String>, elapsed_ms: u64) -> Self {
        Self {
            check_label: check.label.clone(),
            pass: false,
            reasoning: String::new(),
            confidence: 0.0,
            backend: check.backend.clone(),
            elapsed_ms,
            error: Some(error.into()),
        }
    }
}

/// The judge's structured output before it is mapped to a check result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckVerdict {
    /// The model's pass/fail answer.
    pub pass: bool,
    /// Why.
    pub reasoning: String,
    /// Confidence in `[0, 1]`.
    pub confidence: f64,
}

/// High-level instructions passed from Interpret to Process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessContext {
    /// One-sentence task restatement.
    pub task: String,
    /// Bounded constraints.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Advisory tool subset.
    #[serde(default)]
    pub suggested_tools: Option<Vec<String>>,
    /// Advisory budget class.
    #[serde(default)]
    pub budget_hint: BudgetClass,
}

/// Verification failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum VerificationError {
    /// A check or backend configuration is invalid.
    #[error("invalid verification config: {0}")]
    InvalidConfig(String),
    /// The backend returned malformed or non-conforming output.
    #[error("invalid verification output: {0}")]
    InvalidOutput(String),
    /// The model could not be called.
    #[error("verification model unavailable: {0}")]
    Model(String),
    /// The backend exceeded its deadline.
    #[error("verification timed out after {0} ms")]
    Timeout(u64),
    /// The requested backend is not registered.
    #[error("unknown verification backend `{0}`")]
    UnknownBackend(String),
    /// The same backend id was registered twice.
    #[error("duplicate verification backend `{0}`")]
    DuplicateBackend(String),
}

/// Result alias.
pub type Result<T, E = VerificationError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_defaults_to_the_half_threshold() {
        assert_eq!(VerifyCheck::default().confidence_threshold, 0.5);
        let parsed: VerifyCheck = serde_json::from_value(serde_json::json!({
            "label": "x",
            "backend": "llm_judge",
            "backend_params": {}
        }))
        .unwrap();
        assert_eq!(parsed.confidence_threshold, 0.5);
    }

    #[test]
    fn a_failed_result_names_the_backend_and_error() {
        let check = VerifyCheck {
            label: "tests".into(),
            backend: "llm_judge".into(),
            ..VerifyCheck::default()
        };
        let result = VerificationResult::failed(&check, "timeout", 12);
        assert!(!result.pass);
        assert_eq!(result.check_label, "tests");
        assert_eq!(result.error.as_deref(), Some("timeout"));
        assert_eq!(result.elapsed_ms, 12);
    }

    #[test]
    fn a_check_rejects_unknown_fields() {
        assert!(serde_json::from_value::<VerifyCheck>(serde_json::json!({
            "label": "x",
            "backend": "llm_judge",
            "backend_params": {},
            "bogus": true
        }))
        .is_err());
    }
}
