//! Run documents and the promotion gate (plan `05` §5.6, `09` §2.4).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use crate::chat::{RUN_REASON, STAGE_REASON};
use crate::docs::RUN_ID;
use crate::error::AppError;
use crate::state::AppState;

pub async fn list(State(state): State<Arc<AppState>>) -> Json<Value> {
    let _ = state;
    Json(json!({
        "runs": [],
        "active": Value::Null,
        "reason": RUN_REASON,
        "docId": RUN_ID,
    }))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let _ = state;
    Err(AppError::NotFound(format!("no run {id} is recorded")))
}

/// A direct module load writes production. The change-set route is the only write.
pub async fn load_module(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    let _ = id;
    Err(AppError::Conflict(crate::chat::PROMOTE_REASON.into()))
}

pub fn stage_tick() -> Value {
    json!({
        "type": "stage_tick",
        "docId": RUN_ID,
        "stage": Value::Null,
        "reason": STAGE_REASON,
    })
}
