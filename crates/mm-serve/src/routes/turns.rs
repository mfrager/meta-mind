//! `POST /api/turns`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use mm_core::{SessionId, TurnId};

use crate::dto::{StartTurnRequest, TurnAccepted};
use crate::error::AppError;
use crate::state::AppState;

/// Start one turn and return before its first event is written.
pub async fn start(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartTurnRequest>,
) -> Result<(StatusCode, Json<TurnAccepted>), AppError> {
    let text = body.text.trim().to_owned();
    if text.is_empty() {
        return Err(AppError::BadRequest("text must not be empty".into()));
    }
    if text.chars().count() > 32_000 {
        return Err(AppError::BadRequest(
            "text is longer than 32000 characters".into(),
        ));
    }
    let session_id = body
        .session_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| SessionId::generate().to_string());
    let user_id = body.user_id.unwrap_or_else(|| "operator".into());
    state.store.ensure_session(&session_id, &user_id, None)?;
    let turn_id = TurnId::generate().to_string();
    let seq = state.store.last_seq(&session_id)?.saturating_add(1);

    let state = Arc::clone(&state);
    let request = mm_cme::TurnRequest {
        session_id: SessionId::new(session_id.clone()),
        turn_id: TurnId::new(turn_id.clone()),
        user_input: text,
    };
    state.metrics.turn_started();
    tokio::spawn(async move {
        match state.pipeline.run(request).await {
            Ok(_) => state.metrics.turn_completed(),
            Err(error) => {
                state.metrics.turn_failed();
                tracing::error!(%error, "turn failed");
            }
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(TurnAccepted {
            session_id,
            turn_id,
            seq,
        }),
    ))
}
