//! The verification backend seam.

use async_trait::async_trait;

use super::types::{ProcessResult, Result, VerificationResult, VerifyCheck, VerifyContext};

/// Executes one verification check.
///
/// A backend must be deterministic with respect to its inputs for tests to be
/// meaningful. `validate_config` runs when a check is registered, so malformed
/// configuration fails the cheap pass rather than a completed turn.
#[async_trait]
pub trait VerificationBackend: Send + Sync {
    /// Stable backend identifier (`llm_judge`).
    fn id(&self) -> &'static str;

    /// Validate backend-specific check parameters.
    fn validate_config(&self, params: &serde_json::Value) -> Result<()>;

    /// Run one check against the Process output and turn context.
    async fn verify(
        &self,
        check: &VerifyCheck,
        process_output: &ProcessResult,
        context: &VerifyContext,
    ) -> Result<VerificationResult>;
}
