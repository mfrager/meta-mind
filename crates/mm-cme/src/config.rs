//! The three-pass turn configuration.
//!
//! Missing values fall back to the defaults; a present-but-invalid file fails
//! startup rather than silently changing behavior.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::budget::BudgetClass;

/// Where the default configuration lives, relative to the process cwd.
pub const DEFAULT_CONFIG_PATH: &str = "config/three_pass.toml";
/// The largest retry count the runtime accepts.
pub const MAX_RETRIES_LIMIT: u32 = 10;
/// The largest check count the runtime accepts.
pub const MAX_CHECKS_LIMIT: usize = crate::verification::MAX_CHECKS;
/// The largest parallel-check fan-out.
pub const MAX_PARALLEL_CHECKS: usize = 8;

/// The whole three-pass configuration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThreePassConfig {
    /// Interpret pass.
    pub interpret: InterpretConfig,
    /// Process pass.
    pub process: PassConfig,
    /// Verify pass.
    pub verify: VerifyConfig,
    /// Turn envelope.
    pub envelope: EnvelopeConfig,
    /// Verification backends.
    pub verification: VerificationConfig,
    /// Turn resource budgeting.
    pub budget: BudgetConfig,
}

/// Whether token and wall limits are enforced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BudgetConfig {
    /// Enforce token and wall limits. Rounds are always capped.
    pub enabled: bool,
}

/// A regular pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PassConfig {
    /// Budget class.
    pub budget_class: BudgetClass,
    /// Prompt strategy.
    pub strategy: String,
    /// Completion ceiling.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
}

impl Default for PassConfig {
    fn default() -> Self {
        Self {
            budget_class: BudgetClass::Standard,
            strategy: "cm.process.v1".into(),
            max_tokens: 4096,
            temperature: 0.2,
        }
    }
}

/// The Interpret pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InterpretConfig {
    /// Budget class.
    pub budget_class: BudgetClass,
    /// Prompt strategy.
    pub strategy: String,
    /// Maximum registered checks.
    pub max_checks: usize,
    /// Completion ceiling.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
}

impl Default for InterpretConfig {
    fn default() -> Self {
        Self {
            budget_class: BudgetClass::Quick,
            strategy: "cm.interpret.v1".into(),
            max_checks: 8,
            max_tokens: 2048,
            temperature: 0.0,
        }
    }
}

/// The Verify pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VerifyConfig {
    /// Budget class.
    pub budget_class: BudgetClass,
    /// Prompt strategy.
    pub strategy: String,
    /// Retries after the first attempt.
    pub max_retries: u32,
    /// Per-check timeout.
    pub check_timeout_ms: u64,
    /// Parallel checks (the retained implementation runs them sequentially).
    pub max_parallel_checks: usize,
    /// Allow approving a turn whose checks failed.
    pub allow_override: bool,
    /// Completion ceiling.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self {
            budget_class: BudgetClass::Quick,
            strategy: "cm.verify.v1".into(),
            max_retries: 5,
            check_timeout_ms: 20_000,
            max_parallel_checks: 4,
            allow_override: false,
            max_tokens: 2048,
            temperature: 0.0,
        }
    }
}

/// The turn envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnvelopeConfig {
    /// Token ceiling; may only lower the built-in standard envelope.
    pub max_tokens: u64,
}

impl Default for EnvelopeConfig {
    fn default() -> Self {
        Self {
            max_tokens: 184_000,
        }
    }
}

/// Verification backend configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct VerificationConfig {
    /// Backends by id.
    pub backends: BTreeMap<String, JudgeConfig>,
}

impl VerificationConfig {
    /// The judge settings, or the defaults when the map omits it.
    pub fn llm_judge(&self) -> JudgeConfig {
        self.backends.get("llm_judge").cloned().unwrap_or_default()
    }
}

/// LLM judge settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JudgeConfig {
    /// Whether the backend is enabled.
    pub enabled: bool,
    /// Optional model pin.
    pub model: Option<String>,
    /// Sampling temperature.
    pub temperature: f32,
    /// Completion cap.
    pub max_output_tokens: u32,
}

impl Default for JudgeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
            temperature: 0.0,
            max_output_tokens: 512,
        }
    }
}

impl ThreePassConfig {
    /// Parse a configuration document.
    pub fn from_toml(text: &str) -> Result<Self, ThreePassConfigError> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// Load a file. A missing file is the defaults.
    pub fn load_or_default(path: impl AsRef<Path>) -> Result<Self, ThreePassConfigError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        Self::from_toml(&text)
    }

    /// Validate cross-field constraints and strategy names.
    pub fn validate(&self) -> Result<(), ThreePassConfigError> {
        if self.budget.enabled && self.envelope.max_tokens < 8_000 {
            return Err(ThreePassConfigError::Invalid(
                "envelope.max_tokens must be at least 8000 when budgeting is enabled".into(),
            ));
        }
        if self.interpret.max_checks == 0 || self.interpret.max_checks > MAX_CHECKS_LIMIT {
            return Err(ThreePassConfigError::Invalid(format!(
                "interpret.max_checks must be 1..={MAX_CHECKS_LIMIT}"
            )));
        }
        if self.verify.max_retries > MAX_RETRIES_LIMIT {
            return Err(ThreePassConfigError::Invalid(format!(
                "verify.max_retries must be <= {MAX_RETRIES_LIMIT}"
            )));
        }
        if self.verify.max_parallel_checks == 0
            || self.verify.max_parallel_checks > MAX_PARALLEL_CHECKS
        {
            return Err(ThreePassConfigError::Invalid(format!(
                "verify.max_parallel_checks must be 1..={MAX_PARALLEL_CHECKS}"
            )));
        }
        if self.verify.check_timeout_ms < 1_000 || self.verify.check_timeout_ms > 120_000 {
            return Err(ThreePassConfigError::Invalid(
                "verify.check_timeout_ms must be 1000..=120000".into(),
            ));
        }
        for (name, strategy) in [
            ("interpret.strategy", &self.interpret.strategy),
            ("process.strategy", &self.process.strategy),
            ("verify.strategy", &self.verify.strategy),
        ] {
            if strategy.trim().is_empty() {
                return Err(ThreePassConfigError::Invalid(format!(
                    "{name} must not be empty"
                )));
            }
        }
        if self.verification.llm_judge().max_output_tokens == 0 {
            return Err(ThreePassConfigError::Invalid(
                "verification.backends.llm_judge.max_output_tokens must be positive".into(),
            ));
        }
        for (name, max_tokens) in [
            ("interpret.max_tokens", self.interpret.max_tokens),
            ("process.max_tokens", self.process.max_tokens),
            ("verify.max_tokens", self.verify.max_tokens),
        ] {
            if max_tokens == 0 {
                return Err(ThreePassConfigError::Invalid(format!(
                    "{name} must be positive"
                )));
            }
        }
        Ok(())
    }

    /// Apply `MM_VERIFY_MAX_RETRIES`. The variable can only lower the file value.
    pub fn apply_retry_override(
        &mut self,
        value: Option<&str>,
    ) -> Result<(), ThreePassConfigError> {
        let Some(value) = value else {
            return Ok(());
        };
        let parsed: u32 = value.trim().parse().map_err(|_| {
            ThreePassConfigError::Invalid("MM_VERIFY_MAX_RETRIES is not a number".into())
        })?;
        if parsed > MAX_RETRIES_LIMIT {
            return Err(ThreePassConfigError::Invalid(format!(
                "MM_VERIFY_MAX_RETRIES must be <= {MAX_RETRIES_LIMIT}"
            )));
        }
        self.verify.max_retries = self.verify.max_retries.min(parsed);
        Ok(())
    }

    /// Read `MM_VERIFY_MAX_RETRIES` from the environment.
    pub fn apply_env(&mut self) -> Result<(), ThreePassConfigError> {
        let value = std::env::var("MM_VERIFY_MAX_RETRIES").ok();
        self.apply_retry_override(value.as_deref())
    }
}

/// Configuration failures.
#[derive(Debug, Error)]
pub enum ThreePassConfigError {
    /// The file could not be read.
    #[error("three-pass config read: {0}")]
    Io(#[from] std::io::Error),
    /// TOML parse failure.
    #[error("three-pass config parse: {0}")]
    Toml(#[from] toml::de::Error),
    /// A constraint failed.
    #[error("three-pass config invalid: {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_retained_core() {
        let config = ThreePassConfig::default();
        assert_eq!(config.interpret.strategy, "cm.interpret.v1");
        assert_eq!(config.process.strategy, "cm.process.v1");
        assert_eq!(config.verify.strategy, "cm.verify.v1");
        assert_eq!(config.verify.max_retries, 5);
        assert!(config.verification.llm_judge().enabled);
        config.validate().unwrap();
    }

    #[test]
    fn retired_sections_are_refused() {
        assert!(ThreePassConfig::from_toml("[think]\nthree_pass = false\n").is_err());
        assert!(ThreePassConfig::from_toml("[step]\nstrategy = \"cm.step.v1\"\n").is_err());
    }

    #[test]
    fn invalid_values_are_refused() {
        assert!(ThreePassConfig::from_toml("[verify]\nmax_retries = 11\n").is_err());
        assert!(ThreePassConfig::from_toml("[interpret]\nmax_checks = 0\n").is_err());
        assert!(ThreePassConfig::from_toml("[process]\nmax_tokens = 0\n").is_err());
    }

    #[test]
    fn a_partial_document_keeps_the_defaults() {
        let config = ThreePassConfig::from_toml("[verify]\nmax_retries = 2\n").unwrap();
        assert_eq!(config.verify.max_retries, 2);
        assert_eq!(config.interpret.max_checks, 8);
    }

    #[test]
    fn the_checked_in_file_parses() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/three_pass.toml");
        let config = ThreePassConfig::load_or_default(path).unwrap();
        assert_eq!(config.verify.max_retries, 5);
        assert!(config.verification.llm_judge().enabled);
    }
}
