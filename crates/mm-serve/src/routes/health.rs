//! `GET /api/health`.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;

use crate::dto::HealthEnvelope;
use crate::state::AppState;

/// Health, with the counters the operator needs to see the server is idle.
pub async fn get(State(state): State<Arc<AppState>>) -> Json<HealthEnvelope> {
    let sessions = state.store.sessions().map(|list| list.len()).unwrap_or(0);
    Json(HealthEnvelope {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        data_dir: state.cfg.data_dir.display().to_string(),
        sessions,
        turns_started: state.metrics.started(),
        turns_completed: state.metrics.completed(),
        turns_failed: state.metrics.failed(),
    })
}
