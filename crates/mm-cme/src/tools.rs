//! The six turn tools.
//!
//! Four conversation-state tools stage a [`StatePatch`]; two control tools end
//! Interpret or Verify. Argument parsing is strict: an unknown field or a wrong
//! type is a tool error, never a coerced value.

use mm_agentstream::{Frame, FrameCommand, StatePatch};
use mm_llm::ToolSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{CmeError, Result};
use crate::pass_scope::PassScope;
use crate::verification::{ProcessContext, VerifyCheck};

/// The structured result of `metamind.verify`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InterpretResult {
    /// Checks the Verify pass must run.
    pub checks: Vec<VerifyCheck>,
    /// The Process brief.
    pub process: Option<ProcessContext>,
}

/// The parsed result of `metamind.approve`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Approval {
    /// Whether the turn ends successfully.
    pub success: bool,
    /// The final reply, on a fast path.
    pub reply: Option<String>,
    /// The rejection reason.
    pub error: Option<String>,
}

/// All six schemas, in catalog order.
pub fn all_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "conversation.topic".into(),
            description: "Set the conversation topic.".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "topic": { "type": "string", "minLength": 1, "maxLength": 200 }
                },
                "required": ["topic"]
            }),
        },
        ToolSchema {
            name: "conversation.depth".into(),
            description: "Set the conversation depth (0 = surface, 4 = deep).".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "depth": { "type": "integer", "minimum": 0, "maximum": 4 }
                },
                "required": ["depth"]
            }),
        },
        ToolSchema {
            name: "conversation.status".into(),
            description: "Replace the typed conversation status object.".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": { "type": "object", "additionalProperties": true }
                },
                "required": ["status"]
            }),
        },
        ToolSchema {
            name: "conversation.frame".into(),
            description: "Push, pop, or replace the top conversation frame.".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "action": { "type": "string", "enum": ["push", "pop", "replace"] },
                    "frame": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "id": { "type": "string" },
                            "kind": { "type": "string" },
                            "step": { "type": "string" }
                        },
                        "required": ["id", "kind", "step"]
                    }
                },
                "required": ["action"]
            }),
        },
        ToolSchema {
            name: "metamind.verify".into(),
            description: "Register verification checks and the Process brief.".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "checks": {
                        "type": "array",
                        "maxItems": 8,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "label": { "type": "string" },
                                "description": { "type": "string" },
                                "details": { "type": "string" },
                                "backend": { "type": "string", "enum": ["llm_judge"] },
                                "backend_params": { "type": "object" },
                                "confidence_threshold": { "type": "number", "minimum": 0, "maximum": 1 }
                            },
                            "required": ["label", "backend"]
                        }
                    },
                    "process": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "task": { "type": "string" },
                            "constraints": { "type": "array", "items": { "type": "string" } },
                            "suggested_tools": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["task"]
                    }
                },
                "required": ["checks"]
            }),
        },
        ToolSchema {
            name: "metamind.approve".into(),
            description: "End the turn, or reject the reply and retry Process.".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "success": { "type": "boolean" },
                    "reply": { "type": "string" },
                    "error": { "type": "string" }
                },
                "required": ["success"]
            }),
        },
    ]
}

/// The schemas visible to one pass.
pub fn schemas(scope: PassScope) -> Vec<ToolSchema> {
    all_schemas()
        .into_iter()
        .filter(|schema| scope.allows(&schema.name))
        .collect()
}

/// The tool names visible to one pass, for the prompt.
pub fn tools_summary(scope: PassScope) -> String {
    scope.visible_tools().join(", ")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TopicArgs {
    topic: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DepthArgs {
    depth: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusArgs {
    status: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameArgs {
    action: String,
    #[serde(default)]
    frame: Option<FrameArgsFrame>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameArgsFrame {
    id: String,
    kind: String,
    step: String,
}

impl From<FrameArgsFrame> for Frame {
    fn from(value: FrameArgsFrame) -> Self {
        Frame {
            id: value.id,
            kind: value.kind,
            step: value.step,
        }
    }
}

/// Parse one conversation-state tool into a patch.
pub fn parse_conversation(name: &str, arguments: &str) -> Result<StatePatch> {
    match name {
        "conversation.topic" => {
            let args: TopicArgs = parse(arguments, name)?;
            if args.topic.trim().is_empty() {
                return Err(CmeError::Tool("conversation.topic needs a topic".into()));
            }
            Ok(StatePatch {
                topic: Some(Some(args.topic)),
                ..StatePatch::default()
            })
        }
        "conversation.depth" => {
            let args: DepthArgs = parse(arguments, name)?;
            if args.depth > 4 {
                return Err(CmeError::Tool(
                    "conversation.depth must be between 0 and 4".into(),
                ));
            }
            Ok(StatePatch {
                depth: Some(args.depth),
                ..StatePatch::default()
            })
        }
        "conversation.status" => {
            let args: StatusArgs = parse(arguments, name)?;
            if !args.status.is_object() {
                return Err(CmeError::Tool("conversation.status needs an object".into()));
            }
            Ok(StatePatch {
                status: Some(args.status),
                ..StatePatch::default()
            })
        }
        "conversation.frame" => {
            let args: FrameArgs = parse(arguments, name)?;
            let command = match args.action.as_str() {
                "pop" => FrameCommand::Pop,
                "push" | "replace" => {
                    let frame = args.frame.ok_or_else(|| {
                        CmeError::Tool(format!("conversation.frame {} needs a frame", args.action))
                    })?;
                    if args.action == "push" {
                        FrameCommand::Push(frame.into())
                    } else {
                        FrameCommand::Replace(frame.into())
                    }
                }
                other => {
                    return Err(CmeError::Tool(format!(
                        "unknown conversation.frame action `{other}`"
                    )))
                }
            };
            Ok(StatePatch {
                frame: Some(command),
                ..StatePatch::default()
            })
        }
        other => Err(CmeError::Tool(format!(
            "unknown conversation tool `{other}`"
        ))),
    }
}

/// Render a staged tool result.
pub fn staged_output(name: &str, patch: &StatePatch) -> String {
    json!({ "ok": true, "staged": true, "tool": name, "patch": patch }).to_string()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyArgs {
    #[serde(default)]
    checks: Vec<VerifyCheck>,
    #[serde(default)]
    process: Option<ProcessContext>,
}

/// Parse `metamind.verify`.
pub fn parse_verify(arguments: &str) -> Result<InterpretResult> {
    let args: VerifyArgs = parse(arguments, "metamind.verify")?;
    Ok(InterpretResult {
        checks: args.checks,
        process: args.process,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApproveArgs {
    success: bool,
    #[serde(default)]
    reply: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Parse `metamind.approve`.
pub fn parse_approve(arguments: &str) -> Result<Approval> {
    let args: ApproveArgs = parse(arguments, "metamind.approve")?;
    if args.success {
        if args
            .reply
            .as_deref()
            .is_some_and(|reply| reply.chars().count() > 16_000)
        {
            return Err(CmeError::Tool(
                "metamind.approve reply is longer than 16000 characters".into(),
            ));
        }
    } else if args.error.as_deref().unwrap_or("").trim().is_empty() {
        return Err(CmeError::Tool(
            "metamind.approve(false) requires a non-empty error".into(),
        ));
    }
    Ok(Approval {
        success: args.success,
        reply: args.reply,
        error: args.error,
    })
}

/// The implicit action check used when Interpret registers none.
pub fn default_action_check(prompt: &str) -> VerifyCheck {
    VerifyCheck {
        label: "default_action".into(),
        description: Some("the reply addresses the request and delivers the claimed work".into()),
        details: None,
        backend: "llm_judge".into(),
        backend_params: json!({
            "evaluation_prompt": prompt,
            "expected_output_schema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "pass": { "type": "boolean" },
                    "reasoning": { "type": "string" },
                    "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
                },
                "required": ["pass", "reasoning", "confidence"]
            }
        }),
        confidence_threshold: 0.5,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: &str, name: &str) -> Result<T> {
    serde_json::from_str(arguments)
        .map_err(|error| CmeError::Tool(format!("{name}: invalid arguments: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pass_sees_exactly_its_tools() {
        assert_eq!(schemas(PassScope::Interpret).len(), 2);
        assert_eq!(schemas(PassScope::Process).len(), 4);
        assert_eq!(schemas(PassScope::Verify).len(), 5);
        assert!(all_schemas()
            .iter()
            .all(|schema| schema.parameters["additionalProperties"] == json!(false)));
    }

    #[test]
    fn conversation_tools_stage_the_expected_patch() {
        let patch = parse_conversation("conversation.topic", r#"{"topic":"incident"}"#).unwrap();
        assert_eq!(patch.topic, Some(Some("incident".into())));

        let patch = parse_conversation("conversation.depth", r#"{"depth":2}"#).unwrap();
        assert_eq!(patch.depth, Some(2));

        let patch =
            parse_conversation("conversation.status", r#"{"status":{"phase":"active"}}"#).unwrap();
        assert_eq!(patch.status.unwrap()["phase"], "active");

        let patch = parse_conversation(
            "conversation.frame",
            r#"{"action":"push","frame":{"id":"f","kind":"work","step":"task"}}"#,
        )
        .unwrap();
        assert!(matches!(patch.frame, Some(FrameCommand::Push(_))));
    }

    #[test]
    fn strict_arguments_reject_unknown_fields_and_bad_values() {
        assert!(parse_conversation("conversation.topic", r#"{"topic":"x","bogus":1}"#).is_err());
        assert!(parse_conversation("conversation.depth", r#"{"depth":9}"#).is_err());
        assert!(parse_conversation("conversation.frame", r#"{"action":"push"}"#).is_err());
        assert!(parse_conversation("agent.run", r#"{}"#).is_err());
    }

    #[test]
    fn verify_and_approve_parse_the_contract() {
        let result =
            parse_verify(r#"{"checks":[],"process":{"task":"answer","constraints":["short"]}}"#)
                .unwrap();
        assert!(result.checks.is_empty());
        assert_eq!(result.process.unwrap().task, "answer");

        let approval = parse_approve(r#"{"success":true,"reply":"Paris."}"#).unwrap();
        assert!(approval.success);
        assert_eq!(approval.reply.as_deref(), Some("Paris."));

        assert!(parse_approve(r#"{"success":false}"#).is_err());
        assert!(parse_verify(r#"{"checks":[],"bogus":1}"#).is_err());
    }
}
