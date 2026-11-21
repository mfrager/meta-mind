//! ConfirmMw. The body is built from the tool input, not from the model.

use serde_json::{json, Value};

use super::catalog::{Confirm, ToolSpec};

/// Whether this call waits for Approve.
pub fn needs_confirm(spec: &ToolSpec, input: &Value) -> bool {
    if spec.name == "decide.ask" {
        let core = input.get("core").and_then(Value::as_str).unwrap_or("rules");
        return matches!(core, "hosted" | "local");
    }
    match spec.confirm {
        Confirm::Never => false,
        Confirm::Always | Confirm::IfMutating => true,
    }
}

/// The snapshot the tests lock. The frame the operator sees is [`confirm_request`].
#[allow(dead_code)]
pub fn confirm_body(name: &str, input: &Value) -> Value {
    json!({
        "kind": "approve",
        "title": name,
        "body": body_line(name, input),
        "options": ["Approve", "Reject", "Defer"],
    })
}

/// The input-request the existing button grid can render.
pub fn confirm_request(name: &str, input: &Value) -> Value {
    let body = body_line(name, input);
    json!({
        "kind": "buttons",
        "purpose": "controller",
        "title": name,
        "prompt": body,
        "columns": 3,
        "variant": "confirm",
        "options": [
            { "label": "Approve", "value": "Approve" },
            { "label": "Reject", "value": "Reject" },
            { "label": "Defer", "value": "Defer" }
        ]
    })
}

fn body_line(name: &str, input: &Value) -> String {
    match name {
        "memory.add" => {
            let kind = input.get("kind").and_then(Value::as_str).unwrap_or("semantic");
            let content = input.get("content").and_then(Value::as_str).unwrap_or("");
            format!("Store a {kind} memory: \"{content}\"")
        }
        "memory.forget" => {
            let apply = input.get("apply").and_then(Value::as_bool).unwrap_or(false);
            if apply {
                "Forget memories with apply: true".into()
            } else {
                "Forget memories as a dry run".into()
            }
        }
        "promote.run" => {
            let id = input.get("id").and_then(Value::as_str).unwrap_or("");
            format!("Promote change set {id}")
        }
        "sandbox_tool.run" => {
            let tool = input.get("name").and_then(Value::as_str).unwrap_or("");
            format!("Run sandbox tool {tool}")
        }
        "loop.run" => {
            let goal = input
                .get("goal")
                .or_else(|| input.get("path"))
                .and_then(Value::as_str)
                .unwrap_or("");
            format!("Run the loop for {goal}")
        }
        "being.goal_add" => {
            let description = input
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            format!("Add a goal: \"{description}\"")
        }
        "focus.patch" => {
            let id = input.get("id").and_then(Value::as_str).unwrap_or("");
            format!("Patch the focused document {id}")
        }
        "changeset.new" => "Open a change set".into(),
        "sandbox.run" => "Run the change set in the sandbox".into(),
        "reject.run" => {
            let id = input.get("id").and_then(Value::as_str).unwrap_or("");
            format!("Reject change set {id}")
        }
        "action.rollback" => "Roll the action back to its snapshot".into(),
        other => format!("Run {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn confirm_bodies_match_the_operator_copy() {
        assert_eq!(
            confirm_body(
                "memory.add",
                &json!({"kind": "semantic", "content": "the migration order is 0001 through 0012"})
            ),
            json!({
                "kind": "approve",
                "title": "memory.add",
                "body": "Store a semantic memory: \"the migration order is 0001 through 0012\"",
                "options": ["Approve", "Reject", "Defer"]
            })
        );
        assert_eq!(
            confirm_body("memory.forget", &json!({"apply": true}))["body"],
            json!("Forget memories with apply: true")
        );
        assert_eq!(
            confirm_body("promote.run", &json!({"id": "01abc"}))["body"],
            json!("Promote change set 01abc")
        );
        assert_eq!(
            confirm_body("sandbox_tool.run", &json!({"name": "fs.read"}))["body"],
            json!("Run sandbox tool fs.read")
        );
        assert_eq!(
            confirm_body("loop.run", &json!({"goal": "bench/goals/one.md"}))["body"],
            json!("Run the loop for bench/goals/one.md")
        );
    }
}
