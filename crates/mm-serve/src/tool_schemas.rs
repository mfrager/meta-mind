//! The exact seventeen CM tool schemas, strict argument parsing, and the shared
//! result envelope.
//!
//! The schema JSON is generated from the reviewed design artifact and embedded
//! so the runtime and tests cannot drift. Phase 0 adds no route; later phases
//! bind these definitions to the tool dispatcher.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

/// Embedded schemas, extracted verbatim from the design.
pub const TOOL_SCHEMA_JSON: &str = include_str!("../schemas/tools.json");

/// The exact tool names, in design order.
pub const TOOL_NAMES: [&str; 17] = [
    "agent.run",
    "conversation.status",
    "conversation.frame",
    "conversation.topic",
    "conversation.depth",
    "data.create",
    "data.update",
    "data.query",
    "data.delete",
    "data.search",
    "data.schema",
    "data.namespace",
    "metamind.plan",
    "metamind.think",
    "metamind.memory",
    "metamind.log",
    "metamind.act",
];

/// One tool schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    /// Tool name.
    pub name: String,
    /// Model-facing description.
    pub description: String,
    /// JSON Schema for arguments.
    pub parameters: Value,
}

/// The validated schema set.
#[derive(Debug, Clone)]
pub struct ToolCatalog {
    schemas: Vec<ToolSchema>,
}

impl ToolCatalog {
    /// Load and validate the embedded schemas.
    pub fn embedded() -> Result<Self, SchemaError> {
        Self::from_json(TOOL_SCHEMA_JSON)
    }

    /// Load and validate a schema array.
    pub fn from_json(json: &str) -> Result<Self, SchemaError> {
        let schemas: Vec<ToolSchema> =
            serde_json::from_str(json).map_err(|error| SchemaError::Json(error.to_string()))?;
        Self::new(schemas)
    }

    fn new(schemas: Vec<ToolSchema>) -> Result<Self, SchemaError> {
        if schemas.len() != TOOL_NAMES.len() {
            return Err(SchemaError::WrongCount {
                expected: TOOL_NAMES.len(),
                found: schemas.len(),
            });
        }
        let mut seen = BTreeSet::new();
        for schema in &schemas {
            if !seen.insert(schema.name.as_str()) {
                return Err(SchemaError::Duplicate(schema.name.clone()));
            }
            validate_schema(schema)?;
        }
        for name in TOOL_NAMES {
            if !seen.contains(name) {
                return Err(SchemaError::Missing(name.into()));
            }
        }
        Ok(Self { schemas })
    }

    /// All schemas in design order.
    pub fn schemas(&self) -> &[ToolSchema] {
        &self.schemas
    }

    /// Find by name.
    pub fn get(&self, name: &str) -> Option<&ToolSchema> {
        self.schemas.iter().find(|schema| schema.name == name)
    }

    /// Strictly parse a raw JSON argument object against one tool.
    pub fn parse(&self, name: &str, raw: &str) -> Result<Value, ToolArgumentError> {
        let schema = self
            .get(name)
            .ok_or_else(|| ToolArgumentError::UnknownTool(name.into()))?;
        parse_arguments(schema, raw)
    }
}

fn validate_schema(schema: &ToolSchema) -> Result<(), SchemaError> {
    if schema.name.trim().is_empty() || schema.description.trim().is_empty() {
        return Err(SchemaError::Invalid(schema.name.clone()));
    }
    let Value::Object(parameters) = &schema.parameters else {
        return Err(SchemaError::Invalid(schema.name.clone()));
    };
    if parameters.get("type").and_then(Value::as_str) != Some("object") {
        return Err(SchemaError::Invalid(schema.name.clone()));
    }
    if parameters
        .get("additionalProperties")
        .and_then(Value::as_bool)
        != Some(false)
    {
        return Err(SchemaError::Invalid(schema.name.clone()));
    }
    let Some(Value::Object(properties)) = parameters.get("properties") else {
        return Err(SchemaError::Invalid(schema.name.clone()));
    };
    if properties.is_empty() {
        return Err(SchemaError::Invalid(schema.name.clone()));
    }
    for (key, property) in properties {
        if key.trim().is_empty() || !property.is_object() {
            return Err(SchemaError::Invalid(schema.name.clone()));
        }
    }
    if let Some(required) = parameters.get("required") {
        let Some(required) = required.as_array() else {
            return Err(SchemaError::Invalid(schema.name.clone()));
        };
        for field in required {
            let Some(field) = field.as_str() else {
                return Err(SchemaError::Invalid(schema.name.clone()));
            };
            if !properties.contains_key(field) {
                return Err(SchemaError::Invalid(schema.name.clone()));
            }
        }
    }
    Ok(())
}

/// Parse raw JSON strictly. Invalid JSON is `INVALID_JSON`; every other problem
/// is `INVALID_ARGUMENTS` with the offending field.
pub fn parse_arguments(schema: &ToolSchema, raw: &str) -> Result<Value, ToolArgumentError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| ToolArgumentError::InvalidJson(format!("invalid JSON: {error}")))?;
    let Value::Object(object) = &value else {
        return Err(ToolArgumentError::InvalidArguments {
            field: None,
            message: "arguments must be a JSON object".into(),
        });
    };
    let parameters =
        schema
            .parameters
            .as_object()
            .ok_or_else(|| ToolArgumentError::InvalidArguments {
                field: None,
                message: "schema parameters are not an object".into(),
            })?;
    validate_object(object, parameters, None)?;
    Ok(value)
}

fn validate_object(
    object: &Map<String, Value>,
    schema: &Map<String, Value>,
    parent: Option<&str>,
) -> Result<(), ToolArgumentError> {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| ToolArgumentError::InvalidArguments {
            field: parent.map(str::to_owned),
            message: "schema has no properties".into(),
        })?;
    for (key, value) in object {
        let Some(property) = properties.get(key) else {
            return Err(ToolArgumentError::InvalidArguments {
                field: Some(path(parent, key)),
                message: "unknown field".into(),
            });
        };
        validate_value(value, property, Some(&path(parent, key)))?;
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required {
            let field = field.as_str().unwrap_or_default();
            if !object.contains_key(field) {
                return Err(ToolArgumentError::InvalidArguments {
                    field: Some(path(parent, field)),
                    message: "missing required field".into(),
                });
            }
        }
    }
    Ok(())
}

fn validate_value(
    value: &Value,
    schema: &Value,
    field: Option<&str>,
) -> Result<(), ToolArgumentError> {
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.iter().any(|option| option == value) {
            return Err(ToolArgumentError::InvalidArguments {
                field: field.map(str::to_owned),
                message: "value is not in the allowed enum".into(),
            });
        }
    }
    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        if !one_of
            .iter()
            .any(|candidate| validate_value(value, candidate, field).is_ok())
        {
            return Err(ToolArgumentError::InvalidArguments {
                field: field.map(str::to_owned),
                message: "value does not match any allowed shape".into(),
            });
        }
        return Ok(());
    }
    if let Some(types) = schema.get("type") {
        let matches = match types {
            Value::String(kind) => matches_type(value, kind),
            Value::Array(kinds) => kinds
                .iter()
                .filter_map(Value::as_str)
                .any(|kind| matches_type(value, kind)),
            _ => false,
        };
        if !matches {
            return Err(ToolArgumentError::InvalidArguments {
                field: field.map(str::to_owned),
                message: "wrong type".into(),
            });
        }
    }
    match value {
        Value::String(text) => {
            if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
                if text.chars().count() < min as usize {
                    return Err(ToolArgumentError::InvalidArguments {
                        field: field.map(str::to_owned),
                        message: format!("shorter than {min} characters"),
                    });
                }
            }
            if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
                if text.chars().count() > max as usize {
                    return Err(ToolArgumentError::InvalidArguments {
                        field: field.map(str::to_owned),
                        message: format!("longer than {max} characters"),
                    });
                }
            }
            if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
                // The reviewed schemas use simple anchored character classes.
                // Keep this explicit rather than adding a regex dependency.
                if pattern == "^[a-z][a-z0-9_-]{0,31}$" {
                    let valid = text
                        .chars()
                        .next()
                        .is_some_and(|ch| ch.is_ascii_lowercase())
                        && text.len() <= 32
                        && text.chars().all(|ch| {
                            ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-'
                        });
                    if !valid {
                        return Err(ToolArgumentError::InvalidArguments {
                            field: field.map(str::to_owned),
                            message: "does not match the required pattern".into(),
                        });
                    }
                }
            }
        }
        Value::Number(number) => {
            if let Some(minimum) = schema.get("minimum").and_then(Value::as_i64) {
                if number.as_i64().is_none_or(|value| value < minimum) {
                    return Err(ToolArgumentError::InvalidArguments {
                        field: field.map(str::to_owned),
                        message: format!("below minimum {minimum}"),
                    });
                }
            }
            if let Some(maximum) = schema.get("maximum").and_then(Value::as_i64) {
                if number.as_i64().is_none_or(|value| value > maximum) {
                    return Err(ToolArgumentError::InvalidArguments {
                        field: field.map(str::to_owned),
                        message: format!("above maximum {maximum}"),
                    });
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items") {
                for item in items {
                    validate_value(item, item_schema, field)?;
                }
            }
        }
        Value::Object(map) => {
            if let Some(additional) = schema.get("additionalProperties") {
                if additional == &Value::Bool(false) {
                    let Some(properties) = schema.get("properties").and_then(Value::as_object)
                    else {
                        return Err(ToolArgumentError::InvalidArguments {
                            field: field.map(str::to_owned),
                            message: "object schema has no properties".into(),
                        });
                    };
                    for key in map.keys() {
                        if !properties.contains_key(key) {
                            return Err(ToolArgumentError::InvalidArguments {
                                field: Some(path(field, key)),
                                message: "unknown field".into(),
                            });
                        }
                    }
                }
                if additional.is_object() {
                    for item in map.values() {
                        validate_value(item, additional, field)?;
                    }
                }
            }
            if schema.get("properties").is_some() {
                let object_schema = schema.as_object().expect("object schema");
                validate_object(map, object_schema, field)?;
            }
        }
        Value::Bool(_) | Value::Null => {}
    }
    Ok(())
}

fn matches_type(value: &Value, kind: &str) -> bool {
    match kind {
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn path(parent: Option<&str>, key: &str) -> String {
    match parent {
        Some(parent) if !parent.is_empty() => format!("{parent}.{key}"),
        _ => key.to_owned(),
    }
}

/// Tool-schema failures.
#[derive(Debug, Error)]
pub enum SchemaError {
    /// Embedded/loaded JSON is invalid.
    #[error("schema json: {0}")]
    Json(String),
    /// Wrong schema count.
    #[error("expected {expected} tool schemas, found {found}")]
    WrongCount {
        /// Expected count.
        expected: usize,
        /// Found count.
        found: usize,
    },
    /// Duplicate tool name.
    #[error("duplicate tool schema {0}")]
    Duplicate(String),
    /// Missing expected tool.
    #[error("missing tool schema {0}")]
    Missing(String),
    /// Invalid schema shape.
    #[error("invalid tool schema {0}")]
    Invalid(String),
}

/// Strict argument failure.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ToolArgumentError {
    /// Raw arguments were not valid JSON.
    #[error("INVALID_JSON: {0}")]
    InvalidJson(String),
    /// Missing/unknown/wrong-typed field.
    #[error("INVALID_ARGUMENTS: {message}")]
    InvalidArguments {
        /// Dotted field path.
        field: Option<String>,
        /// Human-readable reason.
        message: String,
    },
    /// Unknown tool.
    #[error("INVALID_ARGUMENTS: unknown tool {0}")]
    UnknownTool(String),
}

impl ToolArgumentError {
    /// Stable error code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidJson(_) => "INVALID_JSON",
            Self::InvalidArguments { .. } | Self::UnknownTool(_) => "INVALID_ARGUMENTS",
        }
    }

    /// Whether retrying the same call can help.
    pub fn retryable(&self) -> bool {
        false
    }
}

/// A prompt instance reference in the envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRef {
    /// Instance id.
    pub id: String,
    /// Strategy.
    pub strategy: String,
    /// Optional variant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// Instance hash.
    pub hash: String,
}

/// Search/index health in the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    /// Pending documents.
    pub pending: u64,
    /// Index lag.
    pub lag_ms: u64,
}

/// The shared tool envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolEnvelope {
    /// Success flag.
    pub ok: bool,
    /// Tool name.
    pub tool: String,
    /// Optional action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Conversation state revision.
    pub state_revision: u64,
    /// Full conversation state.
    pub state: Value,
    /// Prompt instance used for this turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<PromptRef>,
    /// Data revision, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_revision: Option<u64>,
    /// Index health, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<IndexStatus>,
    /// Tool-specific result.
    pub result: Option<Value>,
    /// Structured failure.
    pub error: Option<ToolErrorBody>,
}

/// The design's name for the shared tool result envelope.
pub type ToolResultEnvelope = ToolEnvelope;

impl ToolEnvelope {
    /// Successful result.
    pub fn success(
        tool: impl Into<String>,
        action: Option<String>,
        state_revision: u64,
        state: Value,
        result: Value,
    ) -> Self {
        Self {
            ok: true,
            tool: tool.into(),
            action,
            state_revision,
            state,
            prompt: None,
            data_revision: None,
            index: None,
            result: Some(result),
            error: None,
        }
    }

    /// Failed result with the current state.
    pub fn failure(
        tool: impl Into<String>,
        action: Option<String>,
        state_revision: u64,
        state: Value,
        error: ToolErrorBody,
    ) -> Self {
        Self {
            ok: false,
            tool: tool.into(),
            action,
            state_revision,
            state,
            prompt: None,
            data_revision: None,
            index: None,
            result: None,
            error: Some(error),
        }
    }
}

/// Structured envelope error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolErrorBody {
    /// Stable code.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Offending field.
    pub field: Option<String>,
    /// Whether a retry can help.
    pub retryable: bool,
    /// Suggested next action.
    pub next: Option<String>,
}

impl From<ToolArgumentError> for ToolErrorBody {
    fn from(error: ToolArgumentError) -> Self {
        Self {
            code: error.code().into(),
            message: error.to_string(),
            field: match &error {
                ToolArgumentError::InvalidArguments { field, .. } => field.clone(),
                _ => None,
            },
            retryable: error.retryable(),
            next: Some("fix the call and retry".into()),
        }
    }
}

/// Serialize a catalog for the CM prompt. The order is stable.
pub fn catalog_json(catalog: &ToolCatalog) -> Result<String, SchemaError> {
    serde_json::to_string(catalog.schemas()).map_err(|error| SchemaError::Json(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_seventeen_embedded_schemas() {
        let catalog = ToolCatalog::embedded().unwrap();
        assert_eq!(catalog.schemas().len(), 17);
        assert_eq!(catalog.schemas()[0].name, "agent.run");
        assert_eq!(catalog.schemas()[16].name, "metamind.act");
    }

    #[test]
    fn every_schema_is_valid_json_and_has_a_unique_name() {
        let raw: Value = serde_json::from_str(TOOL_SCHEMA_JSON).unwrap();
        let array = raw.as_array().unwrap();
        assert_eq!(array.len(), 17);
        let mut names = BTreeSet::new();
        for schema in array {
            let name = schema.get("name").unwrap().as_str().unwrap();
            assert!(names.insert(name));
            assert!(schema.get("parameters").unwrap().is_object());
        }
    }

    #[test]
    fn invalid_json_is_not_silently_empty() {
        let catalog = ToolCatalog::embedded().unwrap();
        let error = catalog
            .parse("conversation.status", "{not json")
            .unwrap_err();
        assert_eq!(error.code(), "INVALID_JSON");
    }

    #[test]
    fn unknown_fields_and_missing_required_fields_are_rejected() {
        let catalog = ToolCatalog::embedded().unwrap();
        let error = catalog
            .parse("conversation.status", r#"{"action":"set","bogus":true}"#)
            .unwrap_err();
        assert_eq!(error.code(), "INVALID_ARGUMENTS");
        let error = catalog.parse("conversation.status", "{}").unwrap_err();
        assert_eq!(error.code(), "INVALID_ARGUMENTS");
    }

    #[test]
    fn types_are_not_coerced() {
        let catalog = ToolCatalog::embedded().unwrap();
        let error = catalog
            .parse(
                "conversation.depth",
                r#"{"action":"set","depth":"3","expected_revision":1}"#,
            )
            .unwrap_err();
        assert_eq!(error.code(), "INVALID_ARGUMENTS");
    }

    #[test]
    fn valid_arguments_round_trip() {
        let catalog = ToolCatalog::embedded().unwrap();
        let value = catalog
            .parse(
                "conversation.topic",
                r#"{"action":"set","topic":"auth refresh","expected_revision":1}"#,
            )
            .unwrap();
        assert_eq!(value["topic"], "auth refresh");
    }

    #[test]
    fn envelope_serializes_with_camel_case_and_nulls() {
        let envelope = ToolEnvelope::success(
            "conversation.topic",
            Some("set".into()),
            2,
            serde_json::json!({"topic":"auth"}),
            serde_json::json!({"set":["topic"],"unchanged":false}),
        );
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["stateRevision"], 2);
        assert_eq!(json["error"], Value::Null);
        assert!(json.get("dataRevision").is_none());
        let back: ToolEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(back, envelope);
    }
}
