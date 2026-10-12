//! The controller: the being speaks, calls read tools, and the console shows
//! state or one document (plan phase 3).

pub(crate) mod admit;
pub(crate) mod catalog;
pub(crate) mod exec;
pub(crate) mod guide;
pub(crate) mod intro;
pub(crate) mod limits;
pub(crate) mod persona;
pub(crate) mod prompt;
pub(crate) mod tools;
pub(crate) mod turn;

use std::sync::Arc;

use mm_llm::{LlmClient, LlmConfig, OpenAiClient};

use crate::state::AppState;

pub use intro::{compose, frames, SELF_DESCRIPTION};
pub use turn::reply;

/// A MockClient script for Playwright. Each element is one model round.
pub fn load_controller_script(path: &std::path::Path) -> Result<Arc<dyn LlmClient>, String> {
    let raw = std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let lines: Vec<ScriptLine> =
        serde_json::from_str(&raw).map_err(|err| format!("{}: {err}", path.display()))?;
    let script = lines
        .into_iter()
        .map(|line| {
            let calls = line
                .tool_calls
                .into_iter()
                .map(|call| mm_llm::ProviderToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: call.arguments,
                })
                .collect();
            mm_llm::ScriptedResponse::text(line.text).with_tool_calls(calls)
        })
        .collect();
    let client: Arc<dyn LlmClient> = Arc::new(mm_llm::MockClient::script(script));
    Ok(client)
}

#[derive(serde::Deserialize)]
struct ScriptLine {
    text: String,
    #[serde(default)]
    tool_calls: Vec<ScriptCall>,
}

#[derive(serde::Deserialize)]
struct ScriptCall {
    id: String,
    name: String,
    arguments: String,
}

/// How a controller turn finds a model.
pub enum ControllerModel {
    /// `OpenAiClient` when both provider variables are set, otherwise no model.
    FromEnv,
    /// Do not call a model, even when the variables are set.
    Off,
    /// A scripted client. Tests use this.
    Scripted(Arc<dyn LlmClient>),
}

pub(crate) fn resolve_client(app: &AppState, cfg: &LlmConfig) -> Option<Arc<dyn LlmClient>> {
    let mode = app
        .controller_model
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    match &*mode {
        ControllerModel::Off => None,
        ControllerModel::Scripted(client) => Some(Arc::clone(client)),
        ControllerModel::FromEnv => OpenAiClient::new(cfg).ok().map(|client| {
            let client: Arc<dyn LlmClient> = Arc::new(client);
            client
        }),
    }
}
