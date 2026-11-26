//! The verification backend registry and check validation.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::backend::VerificationBackend;
use super::types::{Result, VerificationError, VerifyCheck};

/// Maximum checks one turn may register.
pub const MAX_CHECKS: usize = 8;
/// Maximum label length.
pub const MAX_LABEL_CHARS: usize = 64;
/// Maximum description length.
pub const MAX_DESCRIPTION_CHARS: usize = 2_000;
/// Maximum details length.
pub const MAX_DETAILS_CHARS: usize = 2_000;
/// Maximum serialized backend parameter size.
pub const MAX_BACKEND_PARAMS_BYTES: usize = 8 * 1024;

/// The backends available to a turn.
#[derive(Default)]
pub struct BackendRegistry {
    backends: BTreeMap<String, Arc<dyn VerificationBackend>>,
}

impl std::fmt::Debug for BackendRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BackendRegistry")
            .field("ids", &self.ids())
            .finish()
    }
}

impl BackendRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a backend.
    pub fn register(&mut self, backend: Arc<dyn VerificationBackend>) -> Result<()> {
        let id = backend.id().to_owned();
        if self.backends.contains_key(&id) {
            return Err(VerificationError::DuplicateBackend(id));
        }
        self.backends.insert(id, backend);
        Ok(())
    }

    /// Look up a backend.
    pub fn get(&self, id: &str) -> Option<Arc<dyn VerificationBackend>> {
        self.backends.get(id).cloned()
    }

    /// Registered backend ids.
    pub fn ids(&self) -> Vec<&str> {
        self.backends.keys().map(String::as_str).collect()
    }

    /// Whether a backend is registered.
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// Validate one check against its backend.
    pub fn validate_check(&self, check: &VerifyCheck) -> Result<()> {
        let label = check.label.trim();
        if label.is_empty() {
            return Err(VerificationError::InvalidConfig(
                "a check label must not be empty".into(),
            ));
        }
        if label.chars().count() > MAX_LABEL_CHARS {
            return Err(VerificationError::InvalidConfig(format!(
                "check label `{label}` exceeds {MAX_LABEL_CHARS} characters"
            )));
        }
        if check
            .description
            .as_deref()
            .is_some_and(|text| text.chars().count() > MAX_DESCRIPTION_CHARS)
        {
            return Err(VerificationError::InvalidConfig(format!(
                "check `{label}` description is too long"
            )));
        }
        if check
            .details
            .as_deref()
            .is_some_and(|text| text.chars().count() > MAX_DETAILS_CHARS)
        {
            return Err(VerificationError::InvalidConfig(format!(
                "check `{label}` details are too long"
            )));
        }
        if !check.confidence_threshold.is_finite()
            || !(0.0..=1.0).contains(&check.confidence_threshold)
        {
            return Err(VerificationError::InvalidConfig(format!(
                "check `{label}` confidence_threshold must be in [0, 1]"
            )));
        }
        let serialized = serde_json::to_string(&check.backend_params)
            .map_err(|error| VerificationError::InvalidConfig(error.to_string()))?;
        if serialized.len() > MAX_BACKEND_PARAMS_BYTES {
            return Err(VerificationError::InvalidConfig(format!(
                "check `{label}` backend_params exceed {MAX_BACKEND_PARAMS_BYTES} bytes"
            )));
        }
        let backend = self
            .get(&check.backend)
            .ok_or_else(|| VerificationError::UnknownBackend(check.backend.clone()))?;
        backend.validate_config(&check.backend_params)
    }

    /// Validate a set of checks.
    pub fn validate_checks(&self, checks: &[VerifyCheck]) -> Result<()> {
        if checks.len() > MAX_CHECKS {
            return Err(VerificationError::InvalidConfig(format!(
                "a turn may register at most {MAX_CHECKS} checks"
            )));
        }
        let mut labels = BTreeSet::new();
        for check in checks {
            self.validate_check(check)?;
            if !labels.insert(check.label.trim().to_owned()) {
                return Err(VerificationError::InvalidConfig(format!(
                    "duplicate check label `{}`",
                    check.label
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::llm_judge::LlmJudgeBackend;
    use serde_json::json;

    fn registry() -> BackendRegistry {
        let mut registry = BackendRegistry::new();
        registry
            .register(Arc::new(LlmJudgeBackend::new(None, None)))
            .unwrap();
        registry
    }

    fn check(label: &str) -> VerifyCheck {
        VerifyCheck {
            label: label.into(),
            backend: "llm_judge".into(),
            backend_params: json!({
                "evaluation_prompt": "check {{process_reply}}",
                "expected_output_schema": {
                    "type": "object",
                    "properties": {
                        "pass": {"type": "boolean"},
                        "reasoning": {"type": "string"},
                        "confidence": {"type": "number"}
                    },
                    "required": ["pass", "reasoning", "confidence"]
                }
            }),
            ..VerifyCheck::default()
        }
    }

    #[test]
    fn duplicate_backends_are_refused() {
        let mut registry = registry();
        assert!(registry
            .register(Arc::new(LlmJudgeBackend::new(None, None)))
            .is_err());
    }

    #[test]
    fn checks_validate_their_backend_config() {
        let registry = registry();
        registry.validate_check(&check("a")).unwrap();
        let mut bad = check("a");
        bad.backend = "missing".into();
        assert!(registry.validate_check(&bad).is_err());
    }

    #[test]
    fn duplicate_labels_and_too_many_checks_are_refused() {
        let registry = registry();
        assert!(registry.validate_checks(&[check("a"), check("a")]).is_err());
        let many = (0..MAX_CHECKS + 1)
            .map(|index| check(&format!("c{index}")))
            .collect::<Vec<_>>();
        assert!(registry.validate_checks(&many).is_err());
    }

    #[test]
    fn thresholds_must_be_finite_and_bounded() {
        let registry = registry();
        let mut bad = check("a");
        bad.confidence_threshold = 1.5;
        assert!(registry.validate_check(&bad).is_err());
        bad.confidence_threshold = f64::NAN;
        assert!(registry.validate_check(&bad).is_err());
    }

    #[test]
    fn empty_labels_are_refused() {
        let registry = registry();
        assert!(registry.validate_check(&check("  ")).is_err());
    }
}
