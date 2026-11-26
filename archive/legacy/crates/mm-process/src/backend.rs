//! Backend-service seam for process steps.
//!
//! The backend service runtime is the one deliberately stubbed piece of Phase 5.
//! The trait, request/response types and registry are real; the shipped registry
//! is empty, so a `backend` step fails with the typed
//! [`ProcessError::BackendUnavailable`] code instead of pretending to run.
//! Registering an implementation is a code change with a reviewed contract, not
//! a configuration value.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use mm_data::process::ProcessError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One backend execution request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendRequest {
    /// Tenant/session.
    pub session_id: String,
    /// Process instance.
    pub process_id: String,
    /// Step.
    pub step_id: String,
    /// Declared service kind.
    pub service: String,
    /// Structured input.
    #[serde(default)]
    pub input: Value,
}

/// One backend execution result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendResponse {
    /// Whether the step succeeded.
    pub ok: bool,
    /// Structured output stored in the step state.
    #[serde(default)]
    pub output: Value,
    /// A handle for an asynchronous backend, when the result is pending.
    #[serde(default)]
    pub handle: Option<String>,
    /// Explanation.
    #[serde(default)]
    pub detail: String,
}

/// A backend service implementation.
#[async_trait]
pub trait ProcessBackend: Send + Sync {
    /// The service kind this backend answers.
    fn kind(&self) -> &str;
    /// Execute (or start) the work.
    async fn execute(&self, request: BackendRequest) -> Result<BackendResponse, ProcessError>;
    /// Poll a pending handle.
    async fn status(&self, handle: &str) -> Result<BackendResponse, ProcessError>;
}

/// Every registered backend, keyed by kind.
#[derive(Default, Clone)]
pub struct BackendRegistry {
    backends: BTreeMap<String, Arc<dyn ProcessBackend>>,
}

impl std::fmt::Debug for BackendRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BackendRegistry")
            .field("kinds", &self.backends.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl BackendRegistry {
    /// An empty registry: the shipped stub.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Register one backend.
    pub fn register(&mut self, backend: Arc<dyn ProcessBackend>) -> &mut Self {
        self.backends.insert(backend.kind().to_owned(), backend);
        self
    }

    /// Whether a kind is registered.
    pub fn has(&self, kind: &str) -> bool {
        self.backends.contains_key(kind)
    }

    /// The registered kinds.
    pub fn kinds(&self) -> Vec<String> {
        self.backends.keys().cloned().collect()
    }

    /// Execute through the registered backend or refuse with a typed error.
    pub async fn execute(&self, request: BackendRequest) -> Result<BackendResponse, ProcessError> {
        let Some(backend) = self.backends.get(&request.service) else {
            return Err(ProcessError::BackendUnavailable(format!(
                "no backend service {:?} is registered",
                request.service
            )));
        };
        backend.execute(request).await
    }

    /// Poll a handle through the backend that owns it.
    pub async fn status(
        &self,
        service: &str,
        handle: &str,
    ) -> Result<BackendResponse, ProcessError> {
        let Some(backend) = self.backends.get(service) else {
            return Err(ProcessError::BackendUnavailable(format!(
                "no backend service {service:?} is registered"
            )));
        };
        backend.status(handle).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_shipped_registry_is_explicitly_unavailable() {
        let registry = BackendRegistry::empty();
        let error = registry
            .execute(BackendRequest {
                session_id: "s".into(),
                process_id: "p".into(),
                step_id: "a".into(),
                service: "deploy".into(),
                input: Value::Null,
            })
            .await
            .unwrap_err();
        assert!(matches!(error, ProcessError::BackendUnavailable(_)));
        assert!(error.to_string().contains("backend_unavailable"));
    }
}
