//! The minimal session list.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;

use crate::dto::SessionList;
use crate::error::AppError;
use crate::state::AppState;

/// `GET /api/sessions`.
pub async fn list(State(state): State<Arc<AppState>>) -> Result<Json<SessionList>, AppError> {
    Ok(Json(SessionList {
        sessions: state.store.sessions()?,
    }))
}
