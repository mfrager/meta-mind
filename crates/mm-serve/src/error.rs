//! Server errors and their HTTP mapping.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// A request failure.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The request was malformed.
    #[error("{0}")]
    BadRequest(String),
    /// The requested session does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The server could not fulfill a valid request.
    #[error("{0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let error = self.to_string();
        (status, Json(json!({ "error": error }))).into_response()
    }
}

impl From<mm_agentstream::StoreError> for AppError {
    fn from(error: mm_agentstream::StoreError) -> Self {
        AppError::Internal(format!("event store: {error}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        AppError::Internal(format!("io: {error}"))
    }
}

impl From<mm_core::MmError> for AppError {
    fn from(error: mm_core::MmError) -> Self {
        AppError::Internal(error.to_string())
    }
}

impl From<mm_cme::CmeError> for AppError {
    fn from(error: mm_cme::CmeError) -> Self {
        AppError::Internal(error.to_string())
    }
}

impl From<mm_prompt::PromptError> for AppError {
    fn from(error: mm_prompt::PromptError) -> Self {
        AppError::Internal(format!("prompt: {error}"))
    }
}

impl From<mm_llm::LlmError> for AppError {
    fn from(error: mm_llm::LlmError) -> Self {
        AppError::Internal(error.to_string())
    }
}
