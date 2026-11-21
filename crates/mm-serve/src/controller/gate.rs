//! Shared post-admission checks for console tools.
//!
//! Both the legacy controller and CME run a tool only after admission has
//! checked its group, schema and read-only graph contract. This module adds the
//! irreversible-path checks so neither transport can bypass them.

use serde_json::Value;

use crate::state::AppState;

use super::catalog::ToolSpec;
use super::{firewall, tools::rest};

/// Run the side-effect checks that are not part of pure admission.
pub(crate) async fn check(
    app: &AppState,
    spec: &ToolSpec,
    name: &str,
    input: &Value,
) -> Result<(), String> {
    if firewall::guards(spec.effect) {
        firewall::allow(app, spec)?;
    }
    if name == "loop.run" {
        rest::loop_goal_allowed(app, input).await?;
    }
    if name == "module.load"
        && input
            .get("change_set")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
    {
        return Err(crate::chat::PROMOTE_REASON.into());
    }
    Ok(())
}
