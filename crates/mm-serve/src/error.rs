//! The one error type `mm-serve` answers with (plan `09` §1).
//!
//! Every failure becomes an [`ApiError`] JSON body carrying a stable `code`,
//! the kernel's own `reason` string where one exists (the UI shows it
//! verbatim — plan `00` §4.3), and a `correlationId` that ties the response to
//! the UI log entry.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mm_core::MmError;

use crate::dto::{ApiError, ApiErrorCode};

/// A request- or boot-level failure.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The request is malformed or a field failed validation.
    #[error("{0}")]
    BadRequest(String),
    /// No such object.
    #[error("{0}")]
    NotFound(String),
    /// Another process holds the data-directory write lock (plan `01` §5.1).
    #[error("{0}")]
    Locked(String),
    /// A kernel operation failed; `MmError`'s Display is carried as `reason`.
    #[error(transparent)]
    Kernel(#[from] MmError),
    /// Anything unexpected.
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    /// Map to the wire error. `reason` holds the kernel's verbatim
    /// explanation whenever the error carries one.
    pub fn to_api_error(&self) -> ApiError {
        let (code, reason, retryable) = match self {
            AppError::BadRequest(_) => (ApiErrorCode::InvalidArgument, None, false),
            AppError::NotFound(_) => (ApiErrorCode::NotFound, None, false),
            AppError::Locked(_) => (ApiErrorCode::Locked, Some(self.to_string()), true),
            AppError::Kernel(e) => (ApiErrorCode::Internal, Some(e.to_string()), false),
            AppError::Internal(_) => (ApiErrorCode::Internal, None, true),
        };
        ApiError {
            code,
            message: self.to_string(),
            reason,
            correlation_id: UlidRef::new(),
            retryable,
        }
    }

    /// The HTTP status for this error.
    pub fn status(&self) -> StatusCode {
        match self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Locked(_) => StatusCode::LOCKED,
            AppError::Kernel(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Fresh lowercase ULIDs for correlation ids (never kernel event ids — those
/// come from the kernel's own `UlidFactory` watermark).
struct UlidRef;

impl UlidRef {
    fn new() -> String {
        ulid::Ulid::new().to_string().to_lowercase()
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        (status, Json(self.to_api_error())).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_errors_keep_the_reason_verbatim() {
        let err = AppError::Kernel(MmError::Config("ontology file missing: x.ttl".into()));
        let api = err.to_api_error();
        assert_eq!(api.code, ApiErrorCode::Internal);
        assert_eq!(api.reason.as_deref(), Some("ontology file missing: x.ttl"));
        assert_eq!(api.correlation_id.len(), 26, "ULID is 26 chars");
        assert_eq!(api.correlation_id, api.correlation_id.to_lowercase());
    }

    #[test]
    fn locked_is_retryable_and_carrys_reason() {
        let err = AppError::Locked("data/console.lock is held by pid 4242".into());
        let api = err.to_api_error();
        assert_eq!(api.code, ApiErrorCode::Locked);
        assert!(api.retryable);
        assert_eq!(
            api.reason.as_deref(),
            Some("data/console.lock is held by pid 4242")
        );
    }
}
