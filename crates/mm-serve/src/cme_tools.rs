//! The conversation engine's bridge to the console tool catalog.
//!
//! The controller's catalog is the single inventory of kernel capabilities. CME
//! gets the same names, schemas and dispatcher: a tool called here runs through
//! `controller::exec::run`, not a second implementation.

use std::sync::Weak;

use async_trait::async_trait;
use mm_cme::{CmeError, Result, ToolHandler};
use mm_llm::ToolSchema;
use serde_json::{json, Value};

use crate::controller::{admit, catalog, exec, gate};
use crate::state::AppState;

/// Keep a single tool payload from crowding out the rest of the transcript.
const MAX_TOOL_RESULT_BYTES: usize = 16 * 1024;

/// The full controller catalog, bound to a live [`AppState`].
pub struct ConsoleTools {
    app: Weak<AppState>,
}

impl ConsoleTools {
    /// Build a bridge for the application state that owns the stores.
    pub fn new(app: Weak<AppState>) -> Self {
        Self { app }
    }
}

#[async_trait]
impl ToolHandler for ConsoleTools {
    fn schemas(&self) -> Vec<ToolSchema> {
        catalog::table()
            .iter()
            .map(|spec| ToolSchema {
                name: spec.name.to_string(),
                description: spec.summary.clone(),
                parameters: spec.input_schema.clone(),
            })
            .collect()
    }

    async fn call(&self, name: &str, arguments: &str) -> Result<String> {
        let app = self
            .app
            .upgrade()
            .ok_or_else(|| CmeError::Tool("the console state is no longer available".to_owned()))?;
        let input = parse_arguments(arguments);
        let groups = open_groups();
        let view = admit::TurnView {
            groups: &groups,
            extra_opens: 0,
        };
        let admitted = admit::admit(&view, name, &input).map_err(CmeError::Tool)?;
        gate::check(&app, admitted.spec, name, &input)
            .await
            .map_err(CmeError::Tool)?;
        let result = exec::run(&app, name, &input).await;
        let mut text = serde_json::to_string(&result.to_value())
            .map_err(|error| CmeError::Tool(error.to_string()))?;
        if text.len() > MAX_TOOL_RESULT_BYTES {
            let mut end = MAX_TOOL_RESULT_BYTES;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str("...[truncated]");
        }
        Ok(text)
    }
}

fn open_groups() -> Vec<catalog::GroupId> {
    let mut groups = Vec::new();
    for spec in catalog::table() {
        if !groups.contains(&spec.group) {
            groups.push(spec.group);
        }
    }
    groups
}

fn parse_arguments(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return json!({});
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| json!({}))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn the_bridge_exposes_every_catalog_tool() {
        let tools = ConsoleTools::new(Weak::new());
        let schemas = tools.schemas();
        let advertised: BTreeSet<&str> = schemas.iter().map(|tool| tool.name.as_str()).collect();
        let catalog_names: BTreeSet<&str> = catalog::table().iter().map(|spec| spec.name).collect();
        assert_eq!(advertised, catalog_names);
        assert!(schemas.iter().all(|tool| tool.parameters.is_object()));
    }
}
