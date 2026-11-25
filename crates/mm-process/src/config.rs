//! Reviewed process-pipeline configuration.
//!
//! The file is `config/process.toml`. Unknown keys and invalid bounds are
//! rejected at load, so a typo cannot silently change a limit. The built-in
//! defaults exist for library tests and for a data root that has no file; the
//! server loads the reviewed file before it enables `MM_PROCESS_V1`.

use std::path::Path;

use mm_data::ProcessError;
use serde::{Deserialize, Serialize};

/// Engine limits and cadence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineConfig {
    /// Supervisor tick in milliseconds.
    pub tick_ms: u64,
    /// Queue lease duration in milliseconds.
    pub lease_ms: i64,
    /// Steps executed concurrently per batch.
    pub batch_size: usize,
    /// Live instances across all sessions.
    pub max_live_instances: usize,
    /// Default step timeout.
    pub step_timeout_ms: u64,
    /// Priority aging window: one point per window waited.
    pub priority_aging_ms: i64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            tick_ms: 200,
            // A step may hold a long model call and the lease is not renewed
            // while it runs, so it must outlast `step_timeout_ms`.
            lease_ms: 300_000,
            batch_size: 4,
            max_live_instances: 64,
            step_timeout_ms: 180_000,
            priority_aging_ms: 60_000,
        }
    }
}

/// Size caps applied at validation time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitConfig {
    /// Template steps.
    pub max_steps: usize,
    /// Sub-process nesting depth.
    pub max_depth: u32,
    /// PROCESS.md characters.
    pub max_doc_chars: usize,
    /// One step's instructions.
    pub max_instruction_chars: usize,
    /// One step's stored output.
    pub max_step_output_chars: usize,
    /// Steps in one parallel group.
    pub max_parallel: usize,
}

impl Default for LimitConfig {
    fn default() -> Self {
        Self {
            max_steps: 50,
            max_depth: 3,
            max_doc_chars: 65_536,
            max_instruction_chars: 4_000,
            max_step_output_chars: 8_000,
            max_parallel: 8,
        }
    }
}

/// Plan-builder bounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanConfig {
    /// Hours before a non-persisted plan expires.
    pub default_expiry_hours: u32,
    /// Steps the builder may generate.
    pub max_steps: usize,
    /// Extra attempts after a malformed decode.
    pub decode_retries: u32,
    /// Regenerations after a failed review.
    pub review_retries: u32,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            default_expiry_hours: 24,
            max_steps: 24,
            decode_retries: 2,
            review_retries: 1,
        }
    }
}

/// The whole pipeline configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    /// Engine.
    pub engine: EngineConfig,
    /// Limits.
    pub limits: LimitConfig,
    /// Plan builder.
    pub plan: PlanConfig,
}

impl ProcessConfig {
    /// Load and validate the reviewed file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProcessError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|error| {
            ProcessError::Invalid(format!(
                "process config {} is unreadable: {error}",
                path.display()
            ))
        })?;
        let config: Self = toml::from_str(&text).map_err(|error| {
            ProcessError::Invalid(format!(
                "process config {} is invalid: {error}",
                path.display()
            ))
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Check every bound.
    pub fn validate(&self) -> Result<(), ProcessError> {
        let engine = &self.engine;
        if engine.tick_ms == 0 || engine.tick_ms > 60_000 {
            return Err(ProcessError::Invalid(
                "engine.tick_ms must be 1–60000".into(),
            ));
        }
        if engine.lease_ms < 1_000 || engine.lease_ms > 3_600_000 {
            return Err(ProcessError::Invalid(
                "engine.lease_ms must be 1000–3600000".into(),
            ));
        }
        if engine.batch_size == 0 || engine.batch_size > 32 {
            return Err(ProcessError::Invalid(
                "engine.batch_size must be 1–32".into(),
            ));
        }
        if engine.max_live_instances == 0 || engine.max_live_instances > 1_024 {
            return Err(ProcessError::Invalid(
                "engine.max_live_instances must be 1–1024".into(),
            ));
        }
        if engine.step_timeout_ms < 1_000 || engine.step_timeout_ms > 3_600_000 {
            return Err(ProcessError::Invalid(
                "engine.step_timeout_ms must be 1000–3600000".into(),
            ));
        }
        if engine.priority_aging_ms < 1_000 {
            return Err(ProcessError::Invalid(
                "engine.priority_aging_ms must be at least 1000".into(),
            ));
        }
        let limits = &self.limits;
        if limits.max_steps == 0 || limits.max_steps > mm_data::process::MAX_TEMPLATE_STEPS {
            return Err(ProcessError::Invalid(format!(
                "limits.max_steps must be 1–{}",
                mm_data::process::MAX_TEMPLATE_STEPS
            )));
        }
        if limits.max_depth == 0 || limits.max_depth > 8 {
            return Err(ProcessError::Invalid("limits.max_depth must be 1–8".into()));
        }
        if limits.max_doc_chars == 0 || limits.max_doc_chars > mm_data::process::MAX_DOC_CHARS {
            return Err(ProcessError::Invalid(format!(
                "limits.max_doc_chars must be 1–{}",
                mm_data::process::MAX_DOC_CHARS
            )));
        }
        if limits.max_instruction_chars == 0
            || limits.max_instruction_chars > mm_data::process::MAX_INSTRUCTION_CHARS
        {
            return Err(ProcessError::Invalid(format!(
                "limits.max_instruction_chars must be 1–{}",
                mm_data::process::MAX_INSTRUCTION_CHARS
            )));
        }
        if limits.max_step_output_chars == 0
            || limits.max_step_output_chars > mm_data::process::MAX_STEP_OUTPUT_CHARS
        {
            return Err(ProcessError::Invalid(format!(
                "limits.max_step_output_chars must be 1–{}",
                mm_data::process::MAX_STEP_OUTPUT_CHARS
            )));
        }
        if limits.max_parallel < 2 || limits.max_parallel > 16 {
            return Err(ProcessError::Invalid(
                "limits.max_parallel must be 2–16".into(),
            ));
        }
        if self.plan.default_expiry_hours == 0 || self.plan.default_expiry_hours > 24 * 365 {
            return Err(ProcessError::Invalid(
                "plan.default_expiry_hours must be 1–8760".into(),
            ));
        }
        if self.plan.max_steps == 0 || self.plan.max_steps > 50 {
            return Err(ProcessError::Invalid("plan.max_steps must be 1–50".into()));
        }
        if self.plan.decode_retries > 4 || self.plan.review_retries > 2 {
            return Err(ProcessError::Invalid(
                "plan retries exceed the reviewed bounds".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_unknown_keys_fail() {
        ProcessConfig::default().validate().unwrap();
        let text = r#"
[engine]
tick_ms = 100
lease_ms = 1000
batch_size = 2
max_live_instances = 4
step_timeout_ms = 1000
priority_aging_ms = 1000
surprise = 1

[limits]
max_steps = 1
max_depth = 1
max_doc_chars = 1
max_instruction_chars = 1
max_step_output_chars = 1
max_parallel = 2

[plan]
default_expiry_hours = 1
max_steps = 1
decode_retries = 0
review_retries = 0
"#;
        assert!(toml::from_str::<ProcessConfig>(text).is_err());
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        let mut config = ProcessConfig::default();
        config.engine.batch_size = 0;
        assert!(config.validate().is_err());
    }
}
