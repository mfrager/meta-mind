//! Admission. Pure over the catalog and the turn. No store I/O.

use serde_json::Value;

use super::catalog::{self, GroupId, ToolSpec};
use super::limits::MAX_EXTRA_GROUPS;

pub struct TurnView<'a> {
    pub groups: &'a [GroupId],
    pub extra_opens: u32,
}

pub struct Admitted<'a> {
    /// Kept so a later phase can read `confirm` without a second catalog lookup.
    #[allow(dead_code)]
    pub spec: &'a ToolSpec,
}

/// Reject with a reason, or accept the call.
pub fn admit(turn: &TurnView<'_>, name: &str, input: &Value) -> Result<Admitted<'static>, String> {
    let Some(spec) = catalog::get(name) else {
        return Err(format!("unknown tool {name}"));
    };
    if !spec.available {
        return Err(spec.summary.clone());
    }
    if name == "catalog.open" {
        let group = input.get("group").and_then(Value::as_str).unwrap_or("");
        let Some(id) = GroupId::parse(group) else {
            return Err(format!("unknown group {group}"));
        };
        if !turn.groups.contains(&id) && turn.extra_opens >= MAX_EXTRA_GROUPS {
            return Err("a turn can open at most three extra groups".into());
        }
    } else if !turn.groups.contains(&spec.group) {
        return Err(format!(
            "group {} is not open; call catalog.open",
            spec.group.as_str()
        ));
    }
    if let Err(err) = mm_llm::schema::validate_value(&spec.input_schema, input, name) {
        return Err(err.to_string());
    }
    if name == "graph.query" {
        let query = input.get("query").and_then(Value::as_str).unwrap_or("");
        if let Err(err) = mm_store_graph::ensure_read_query(query) {
            return Err(err.to_string());
        }
    }
    Ok(Admitted { spec })
}
