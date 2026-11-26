//! The verification backend contract.

use async_trait::async_trait;
use serde_json::Value;

use super::types::{ProcessResult, Result, VerificationResult, VerifyCheck, VerifyContext};

/// One check executor.
#[async_trait]
pub trait VerificationBackend: Send + Sync {
    /// The backend identifier written into checks and results.
    fn id(&self) -> &'static str;

    /// Validate one check's backend-specific configuration.
    fn validate_config(&self, params: &Value) -> Result<()>;

    /// Run one check.
    async fn verify(
        &self,
        check: &VerifyCheck,
        process_output: &ProcessResult,
        context: &VerifyContext,
    ) -> Result<VerificationResult>;
}
