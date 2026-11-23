//! The backend registry and the check validator.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::backend::VerificationBackend;
use super::types::{Result, VerificationError, VerifyCheck};

/// Maximum checks one turn or step may register.
pub const MAX_CHECKS: usize = 8;
/// Maximum label length.
pub const MAX_LABEL_CHARS: usize = 64;
/// Maximum description length.
pub const MAX_DESCRIPTION_CHARS: usize = 2_000;
/// Maximum details length.
pub const MAX_DETAILS_CHARS: usize = 2_000;
/// Maximum serialized backend parameters.
pub const MAX_BACKEND_PARAMS_BYTES: usize = 8 * 1024;

/// The backends available to this process.
#[derive(Clone, Default)]
pub struct BackendRegistry {
    backends: BTreeMap<String, Arc<dyn VerificationBackend>>,
}

impl std::fmt::Debug for BackendRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendRegistry")
            .field("backends", &self.backends.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl BackendRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one backend. Duplicate ids are refused.
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

    /// Registered ids in stable order.
    pub fn ids(&self) -> Vec<&str> {
        self.backends.keys().map(String::as_str).collect()
    }

    /// Validate one check against the registry's limits and its backend.
    pub fn validate_check(&self, check: &VerifyCheck) -> Result<()> {
        let label = check.label.trim();
        if label.is_empty() {
            return Err(VerificationError::InvalidConfig(
                "a check label is required".into(),
            ));
        }
        if label.chars().count() > MAX_LABEL_CHARS {
            return Err(VerificationError::InvalidConfig(format!(
                "check label `{label}` is longer than {MAX_LABEL_CHARS} characters"
            )));
        }
        if check.backend.trim().is_empty() {
            return Err(VerificationError::InvalidConfig(format!(
                "check `{label}` has no backend"
            )));
        }
        if let Some(description) = &check.description {
            if description.chars().count() > MAX_DESCRIPTION_CHARS {
                return Err(VerificationError::InvalidConfig(format!(
                    "check `{label}` description is longer than {MAX_DESCRIPTION_CHARS} characters"
                )));
            }
        }
        if let Some(details) = &check.details {
            if details.chars().count() > MAX_DETAILS_CHARS {
                return Err(VerificationError::InvalidConfig(format!(
                    "check `{label}` details are longer than {MAX_DETAILS_CHARS} characters"
                )));
            }
        }
        if !check.confidence_threshold.is_finite()
            || !(0.0..=1.0).contains(&check.confidence_threshold)
        {
            return Err(VerificationError::InvalidConfig(format!(
                "check `{label}` threshold must be within [0, 1]"
            )));
        }
        if !check.backend_params.is_object() {
            return Err(VerificationError::InvalidConfig(format!(
                "check `{label}` backend_params must be an object"
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
        backend.validate_config(&check.backend_params)?;
        Ok(())
    }

    /// Validate a whole set: count, duplicate labels, and every check.
    pub fn validate_checks(&self, checks: &[VerifyCheck]) -> Result<()> {
        if checks.len() > MAX_CHECKS {
            return Err(VerificationError::InvalidConfig(format!(
                "{} checks registered; the maximum is {MAX_CHECKS}",
                checks.len()
            )));
        }
        let mut labels = BTreeSet::new();
        for check in checks {
            let label = check.label.trim().to_owned();
            if !labels.insert(label.clone()) {
                return Err(VerificationError::InvalidConfig(format!(
                    "duplicate check label `{label}`"
                )));
            }
            self.validate_check(check)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::types::{ProcessResult, VerificationResult, VerifyContext};
    use async_trait::async_trait;

    struct FixedBackend;

    #[async_trait]
    impl VerificationBackend for FixedBackend {
        fn id(&self) -> &'static str {
            "fixed"
        }

        fn validate_config(&self, params: &serde_json::Value) -> Result<()> {
            if params.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
                Ok(())
            } else {
                Err(VerificationError::InvalidConfig("ok=true required".into()))
            }
        }

        async fn verify(
            &self,
            check: &VerifyCheck,
            _process_output: &ProcessResult,
            _context: &VerifyContext,
        ) -> Result<VerificationResult> {
            Ok(VerificationResult {
                check_label: check.label.clone(),
                pass: true,
                reasoning: "fixed".into(),
                confidence: 1.0,
                backend: "fixed".into(),
                elapsed_ms: 0,
                error: None,
            })
        }
    }

    fn registry() -> BackendRegistry {
        let mut registry = BackendRegistry::new();
        registry.register(Arc::new(FixedBackend)).unwrap();
        registry
    }

    fn check(label: &str) -> VerifyCheck {
        VerifyCheck {
            label: label.into(),
            backend: "fixed".into(),
            backend_params: serde_json::json!({"ok": true}),
            ..VerifyCheck::default()
        }
    }

    #[test]
    fn duplicate_backends_are_refused() {
        let mut registry = registry();
        let error = registry.register(Arc::new(FixedBackend)).unwrap_err();
        assert!(matches!(error, VerificationError::DuplicateBackend(_)));
        assert_eq!(registry.ids(), vec!["fixed"]);
    }

    #[test]
    fn checks_validate_backend_specific_config() {
        let registry = registry();
        let mut bad = check("x");
        bad.backend_params = serde_json::json!({"ok": false});
        assert!(matches!(
            registry.validate_check(&bad),
            Err(VerificationError::InvalidConfig(_))
        ));
        registry.validate_check(&check("x")).unwrap();
    }

    #[test]
    fn unknown_backends_fail_at_registration_time() {
        let registry = registry();
        let mut bad = check("x");
        bad.backend = "missing".into();
        assert!(matches!(
            registry.validate_check(&bad),
            Err(VerificationError::UnknownBackend(_))
        ));
    }

    #[test]
    fn duplicate_labels_and_too_many_checks_are_refused() {
        let registry = registry();
        let checks = vec![check("same"), check("same")];
        assert!(registry.validate_checks(&checks).is_err());
        let too_many: Vec<_> = (0..=MAX_CHECKS)
            .map(|index| check(&format!("c{index}")))
            .collect();
        assert!(registry.validate_checks(&too_many).is_err());
    }

    #[test]
    fn thresholds_must_be_finite_and_bounded() {
        let registry = registry();
        for threshold in [-0.1, 1.1, f64::NAN] {
            let mut bad = check("x");
            bad.confidence_threshold = threshold;
            assert!(registry.validate_check(&bad).is_err());
        }
        let mut good = check("x");
        good.confidence_threshold = 1.0;
        registry.validate_check(&good).unwrap();
    }

    #[test]
    fn empty_labels_and_non_object_params_are_refused() {
        let registry = registry();
        let empty = check("   ");
        assert!(registry.validate_check(&empty).is_err());
        let mut params = check("x");
        params.backend_params = serde_json::json!("nope");
        assert!(registry.validate_check(&params).is_err());
    }
}
