//! `/api/commands` and `/api/exec` (plan `09` §5).
//!
//! Exec is an allowlist, not a shell. Unknown names and shell metacharacters
//! are rejected. A registered command runs the same orchestrator the chat pane uses.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::chat;
use crate::error::AppError;
use crate::parity::{self, Tier};
use crate::state::AppState;

const SHELL_META: &[char] = &[';', '|', '&', '`', '$', '\n', '<', '>'];

#[derive(Deserialize)]
pub struct ExecBody {
    pub argv: Vec<String>,
}

pub async fn commands() -> Json<Value> {
    let rows: Vec<Value> = parity::COMMANDS
        .iter()
        .map(|command| {
            json!({
                "name": command.name,
                "op": command.op,
                "tier": match command.tier {
                    Tier::Bespoke => "bespoke",
                    Tier::Passthrough => "passthrough",
                },
            })
        })
        .collect();
    Json(json!({ "commands": rows }))
}

pub async fn exec(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ExecBody>,
) -> Result<Json<Value>, AppError> {
    let name = body
        .argv
        .first()
        .map(|arg| arg.trim().trim_start_matches('/').to_string())
        .filter(|arg| !arg.is_empty())
        .ok_or_else(|| AppError::BadRequest("an argv is required".into()))?;
    if body
        .argv
        .iter()
        .any(|arg| arg.chars().any(|ch| SHELL_META.contains(&ch)))
    {
        return Err(AppError::BadRequest(
            "shell metacharacters are not accepted".into(),
        ));
    }
    let Some(command) = parity::command(&name) else {
        return Err(AppError::BadRequest(format!(
            "no operation is registered for {name}"
        )));
    };
    let query = body.argv.join(" ");
    let (success, output) = chat::execute(&state, command.op, &query).await;
    Ok(Json(json!({
        "name": command.name,
        "op": command.op,
        "tier": match command.tier {
            Tier::Bespoke => "bespoke",
            Tier::Passthrough => "passthrough",
        },
        "success": success,
        "output": output,
    })))
}
