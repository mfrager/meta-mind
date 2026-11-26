//! The retained router: turns, agentstream, sessions, and health.

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::error::AppError;
use crate::state::AppState;

pub mod agentstream;
pub mod health;
pub mod sessions;
pub mod turns;

/// Build the server router.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(health::get))
        .route("/api/turns", post(turns::start))
        .route("/api/sessions", get(sessions::list))
        .route("/api/agentstream/sessions", get(agentstream::sessions))
        .route(
            "/api/agentstream/sessions/{session_id}/events",
            get(agentstream::events),
        )
        .route(
            "/api/agentstream/sessions/{session_id}/stream",
            get(agentstream::stream),
        )
        .fallback(not_found)
        .with_state(state)
}

async fn not_found() -> AppError {
    AppError::NotFound("route not found".into())
}
