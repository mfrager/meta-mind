//! Strict JSON Schema: registration, hashing, and validation.
//!
//! "Strict" is not a style preference here. A schema that lets an extra field
//! through, or that leaves `required` implicit, is a schema whose output the
//! substrate cannot trust, so registration *rejects* it rather than accepting a
//! loose one. Only a deliberate subset of JSON Schema is supported, and any
//! construct outside that subset is a hard error — never a silently weakened
//! check, which is the failure mode that would make a green gate meaningless.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::client::SchemaId;
use crate::error::LlmError;

/// Why a value did not satisfy a schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaErrorKind {
    /// The text is not JSON at all.
    NotJson,
    /// The text is empty or whitespace.
    Empty,
    /// The value's type is not the one the schema requires.
    TypeMismatch,
    /// A required property is absent.
    MissingField,
    /// A property the schema does not allow is present.
    ExtraField,
    /// A value is outside its declared range or length.
    OutOfRange,
    /// A value is not one of the allowed ones.
    EnumMismatch,
    /// The schema itself uses an unsupported construct.
    UnsupportedSchema,
}

impl SchemaErrorKind {
    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            SchemaErrorKind::NotJson => "not_json",
            SchemaErrorKind::Empty => "empty",
            SchemaErrorKind::TypeMismatch => "type_mismatch",
            SchemaErrorKind::MissingField => "missing_field",
            SchemaErrorKind::ExtraField => "extra_field",
            SchemaErrorKind::OutOfRange => "out_of_range",
            SchemaErrorKind::EnumMismatch => "enum_mismatch",
            SchemaErrorKind::UnsupportedSchema => "unsupported_schema",
        }
    }
}

/// A validation failure, with the path it occurred at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaError {
    /// What kind of failure this is.
    pub kind: SchemaErrorKind,
    /// A JSON-pointer-ish path to the offending value.
    pub path: String,
    /// A human-readable explanation.
    pub detail: String,
}

impl SchemaError {
    fn new(kind: SchemaErrorKind, path: impl Into<String>, detail: impl Into<String>) -> Self {
        SchemaError {
            kind,
            path: path.into(),
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at {}: {}",
            self.kind.as_str(),
            self.path,
            self.detail
        )
    }
}

/// The keywords the validator implements. Anything else is unsupported.
const KEYWORDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "const",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "minProperties",
    "maxProperties",
    "description",
    "title",
    "$schema",
    "$id",
];

/// Keywords that would change acceptance but are deliberately unsupported.
const FORBIDDEN_KEYWORDS: &[&str] = &[
    "oneOf",
    "anyOf",
    "allOf",
    "not",
    "$ref",
    "$defs",
    "definitions",
    "patternProperties",
    "propertyNames",
    "dependentRequired",
    "dependentSchemas",
    "if",
    "then",
    "else",
    "pattern",
    "format",
    "default",
    "examples",
    "multipleOf",
    "uniqueItems",
    "contains",
    "prefixItems",
];

/// Render JSON with object keys sorted, so two equal schemas hash the same.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = String::from("{");
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                out.push_str(&canonical_json(&map[*key]));
            }
            out.push('}');
            out
        }
        Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", rendered.join(","))
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// The sha256 of a schema's canonical rendering.
pub fn schema_sha(schema: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_json(schema).as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// A fingerprint used to key compiled grammars by schema *and* decoder.
///
/// sha256 rather than `DefaultHasher`: the fingerprint is a *persistent* cache key
/// (it is written to `llm_grammars`), and `DefaultHasher` is explicitly documented
/// as unstable across Rust releases, so a compiler upgrade would silently
/// invalidate or, worse, collide every recorded grammar.
pub fn grammar_fingerprint(schema_sha: &str, decoder: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(schema_sha.as_bytes());
    hasher.update(b"|");
    hasher.update(decoder.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Reject a schema the substrate cannot enforce strictly.
pub fn check_strict(schema: &Value, path: &str) -> Result<(), SchemaError> {
    let Value::Object(map) = schema else {
        return Err(SchemaError::new(
            SchemaErrorKind::UnsupportedSchema,
            path,
            "a schema must be a JSON object",
        ));
    };
    for key in map.keys() {
        if FORBIDDEN_KEYWORDS.contains(&key.as_str()) {
            return Err(SchemaError::new(
                SchemaErrorKind::UnsupportedSchema,
                path,
                format!("`{key}` is not supported; the checker never weakens a schema"),
            ));
        }
        if !KEYWORDS.contains(&key.as_str()) {
            return Err(SchemaError::new(
                SchemaErrorKind::UnsupportedSchema,
                path,
                format!("unknown schema keyword `{key}`"),
            ));
        }
    }

    let types = declared_types(map, path)?;
    let is_object = types.iter().any(|t| t == "object");
    if is_object {
        match map.get("additionalProperties") {
            Some(Value::Bool(false)) => {}
            Some(_) => {
                return Err(SchemaError::new(
                    SchemaErrorKind::UnsupportedSchema,
                    path,
                    "an object schema must set additionalProperties = false",
                ))
            }
            None => {
                return Err(SchemaError::new(
                    SchemaErrorKind::UnsupportedSchema,
                    path,
                    "an object schema must set additionalProperties = false",
                ))
            }
        }
    }

    if is_object {
        let properties: BTreeSet<&str> = map
            .get("properties")
            .and_then(Value::as_object)
            .map(|p| p.keys().map(String::as_str).collect())
            .unwrap_or_default();
        let required: BTreeSet<&str> = match map.get("required") {
            Some(Value::Array(items)) => {
                let mut out = BTreeSet::new();
                for item in items {
                    match item.as_str() {
                        Some(name) => {
                            out.insert(name);
                        }
                        None => {
                            return Err(SchemaError::new(
                                SchemaErrorKind::UnsupportedSchema,
                                path,
                                "`required` must be an array of strings",
                            ))
                        }
                    }
                }
                out
            }
            Some(_) => {
                return Err(SchemaError::new(
                    SchemaErrorKind::UnsupportedSchema,
                    path,
                    "`required` must be an array of strings",
                ))
            }
            None => {
                return Err(SchemaError::new(
                    SchemaErrorKind::UnsupportedSchema,
                    path,
                    "an object schema must declare `required`",
                ))
            }
        };
        for name in &properties {
            if !required.contains(name) {
                return Err(SchemaError::new(
                    SchemaErrorKind::UnsupportedSchema,
                    path,
                    format!(
                        "`{name}` must be listed in `required` (a strict schema has no optional fields)"
                    ),
                ));
            }
        }
        for name in &required {
            if !properties.contains(name) {
                return Err(SchemaError::new(
                    SchemaErrorKind::UnsupportedSchema,
                    path,
                    format!("`required` names `{name}`, which is not in `properties`"),
                ));
            }
        }
    }

    if let Some(Value::Object(props)) = map.get("properties") {
        for (name, sub) in props {
            check_strict(sub, &format!("{path}/properties/{name}"))?;
        }
    }
    if let Some(items) = map.get("items") {
        match items {
            Value::Object(_) => check_strict(items, &format!("{path}/items"))?,
            _ => {
                return Err(SchemaError::new(
                    SchemaErrorKind::UnsupportedSchema,
                    path,
                    "`items` must be a single schema object",
                ))
            }
        }
    }
    Ok(())
}

fn declared_types(map: &Map<String, Value>, path: &str) -> Result<Vec<String>, SchemaError> {
    match map.get("type") {
        Some(Value::String(s)) => Ok(vec![s.clone()]),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Err(SchemaError::new(
                            SchemaErrorKind::UnsupportedSchema,
                            path,
                            "`type` entries must be strings",
                        ))
                    }
                }
            }
            Ok(out)
        }
        Some(_) => Err(SchemaError::new(
            SchemaErrorKind::UnsupportedSchema,
            path,
            "`type` must be a string or an array of strings",
        )),
        None => Ok(vec![]),
    }
}

/// Validate a parsed value against a schema.
pub fn validate_value(schema: &Value, value: &Value, path: &str) -> Result<(), SchemaError> {
    let Value::Object(map) = schema else {
        return Err(SchemaError::new(
            SchemaErrorKind::UnsupportedSchema,
            path,
            "a schema must be a JSON object",
        ));
    };
    let types = declared_types(map, path)?;
    if !types.is_empty() && !types.iter().any(|t| type_matches(t, value)) {
        return Err(SchemaError::new(
            SchemaErrorKind::TypeMismatch,
            path,
            format!(
                "expected {}, got {}",
                types.join("|"),
                json_type_name(value)
            ),
        ));
    }

    if let Some(Value::Array(allowed)) = map.get("enum") {
        if !allowed.contains(value) {
            return Err(SchemaError::new(
                SchemaErrorKind::EnumMismatch,
                path,
                "value is not one of the allowed ones",
            ));
        }
    }
    if let Some(expected) = map.get("const") {
        if expected != value {
            return Err(SchemaError::new(
                SchemaErrorKind::EnumMismatch,
                path,
                "value is not the declared constant",
            ));
        }
    }

    if let Some(obj) = value.as_object() {
        let properties = map.get("properties").and_then(Value::as_object);
        let required: BTreeSet<&str> = map
            .get("required")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        for name in &required {
            if !obj.contains_key(*name) {
                return Err(SchemaError::new(
                    SchemaErrorKind::MissingField,
                    format!("{path}/{name}"),
                    "required field is missing",
                ));
            }
        }
        let additional_allowed = matches!(map.get("additionalProperties"), Some(Value::Bool(true)));
        if !additional_allowed {
            for key in obj.keys() {
                let known = properties.is_some_and(|p| p.contains_key(key));
                if !known {
                    return Err(SchemaError::new(
                        SchemaErrorKind::ExtraField,
                        format!("{path}/{key}"),
                        "property is not declared by the schema",
                    ));
                }
            }
        }
        if let Some(properties) = properties {
            for (name, sub) in properties {
                if let Some(inner) = obj.get(name) {
                    validate_value(sub, inner, &format!("{path}/{name}"))?;
                }
            }
        }
        if let Some(min) = map.get("minProperties").and_then(Value::as_u64) {
            if (obj.len() as u64) < min {
                return Err(SchemaError::new(
                    SchemaErrorKind::OutOfRange,
                    path,
                    format!("needs at least {min} properties"),
                ));
            }
        }
        if let Some(max) = map.get("maxProperties").and_then(Value::as_u64) {
            if (obj.len() as u64) > max {
                return Err(SchemaError::new(
                    SchemaErrorKind::OutOfRange,
                    path,
                    format!("allows at most {max} properties"),
                ));
            }
        }
    }

    if let Some(items) = map.get("items") {
        if let Some(array) = value.as_array() {
            for (i, item) in array.iter().enumerate() {
                validate_value(items, item, &format!("{path}/{i}"))?;
            }
        }
    }
    if let Some(array) = value.as_array() {
        if let Some(min) = map.get("minItems").and_then(Value::as_u64) {
            if (array.len() as u64) < min {
                return Err(SchemaError::new(
                    SchemaErrorKind::OutOfRange,
                    path,
                    format!("needs at least {min} items"),
                ));
            }
        }
        if let Some(max) = map.get("maxItems").and_then(Value::as_u64) {
            if (array.len() as u64) > max {
                return Err(SchemaError::new(
                    SchemaErrorKind::OutOfRange,
                    path,
                    format!("allows at most {max} items"),
                ));
            }
        }
    }

    if let Some(text) = value.as_str() {
        let len = text.chars().count() as u64;
        if let Some(min) = map.get("minLength").and_then(Value::as_u64) {
            if len < min {
                return Err(SchemaError::new(
                    SchemaErrorKind::OutOfRange,
                    path,
                    format!("needs at least {min} characters"),
                ));
            }
        }
        if let Some(max) = map.get("maxLength").and_then(Value::as_u64) {
            if len > max {
                return Err(SchemaError::new(
                    SchemaErrorKind::OutOfRange,
                    path,
                    format!("allows at most {max} characters"),
                ));
            }
        }
    }

    if let Some(number) = value.as_f64() {
        if let Some(min) = map.get("minimum").and_then(Value::as_f64) {
            if number < min {
                return Err(SchemaError::new(
                    SchemaErrorKind::OutOfRange,
                    path,
                    format!("must be at least {min}"),
                ));
            }
        }
        if let Some(max) = map.get("maximum").and_then(Value::as_f64) {
            if number > max {
                return Err(SchemaError::new(
                    SchemaErrorKind::OutOfRange,
                    path,
                    format!("must be at most {max}"),
                ));
            }
        }
        if let Some(min) = map.get("exclusiveMinimum").and_then(Value::as_f64) {
            if number <= min {
                return Err(SchemaError::new(
                    SchemaErrorKind::OutOfRange,
                    path,
                    format!("must be greater than {min}"),
                ));
            }
        }
        if let Some(max) = map.get("exclusiveMaximum").and_then(Value::as_f64) {
            if number >= max {
                return Err(SchemaError::new(
                    SchemaErrorKind::OutOfRange,
                    path,
                    format!("must be less than {max}"),
                ));
            }
        }
    }

    Ok(())
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A registry of strict schemas, keyed by [`SchemaId`].
#[derive(Debug, Clone, Default)]
pub struct SchemaRegistry {
    schemas: BTreeMap<SchemaId, Value>,
}

impl SchemaRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        SchemaRegistry::default()
    }

    /// Register a schema, rejecting anything that is not strict.
    pub fn register(&mut self, id: SchemaId, schema: Value) -> Result<(), LlmError> {
        check_strict(&schema, &id.to_string())
            .map_err(|e| LlmError::Config(format!("schema `{id}` is not strict: {e}")))?;
        if self.schemas.insert(id.clone(), schema).is_some() {
            return Err(LlmError::Config(format!(
                "schema `{id}` is already registered"
            )));
        }
        Ok(())
    }

    /// The schema registered under `id`.
    pub fn get(&self, id: &SchemaId) -> Option<&Value> {
        self.schemas.get(id)
    }

    /// Every registered id, sorted.
    pub fn ids(&self) -> Vec<&SchemaId> {
        self.schemas.keys().collect()
    }

    /// The sha256 of a registered schema.
    pub fn schema_sha(&self, id: &SchemaId) -> Result<String, LlmError> {
        self.get(id)
            .map(schema_sha)
            .ok_or_else(|| LlmError::Config(format!("schema `{id}` is not registered")))
    }

    /// Parse and validate `raw` against a registered schema.
    ///
    /// Returns the parsed value on success. It never repairs, coerces, or
    /// truncates: a caller that wants a repaired text asks for a repair pass, and
    /// a caller that cannot proceed gets a rejection, not a plausible-looking
    /// value.
    pub fn validate(&self, id: &SchemaId, raw: &str) -> Result<Value, SchemaError> {
        let schema = self.get(id).ok_or_else(|| {
            SchemaError::new(
                SchemaErrorKind::UnsupportedSchema,
                "",
                format!("schema `{id}` is not registered"),
            )
        })?;
        if raw.trim().is_empty() {
            return Err(SchemaError::new(
                SchemaErrorKind::Empty,
                "",
                "the payload is empty",
            ));
        }
        let value: Value = serde_json::from_str(raw).map_err(|e| {
            SchemaError::new(SchemaErrorKind::NotJson, "", format!("not JSON: {e}"))
        })?;
        validate_value(schema, &value, "")?;
        Ok(value)
    }
}

/// A type that fixes its own strict schema.
///
/// Implementing this on a type and registering it means the type *is* the schema:
/// the schema cannot drift from the struct it describes without the round trip
/// failing in a test.
pub trait StructuredOut: serde::de::DeserializeOwned + Send + 'static {
    /// The schema's name.
    fn schema_id() -> SchemaId;
    /// The strict JSON Schema.
    fn schema() -> Value;
}

/// Register `T`'s schema in a registry.
pub fn register_structured<T: StructuredOut>(
    registry: &mut SchemaRegistry,
) -> Result<(), LlmError> {
    registry.register(T::schema_id(), T::schema())
}

/// Decode a validated value into `T`.
pub fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, SchemaError> {
    serde_json::from_value(value)
        .map_err(|e| SchemaError::new(SchemaErrorKind::TypeMismatch, "", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn strict_object() -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["name", "score"],
            "properties": {
                "name": { "type": "string", "minLength": 1 },
                "score": { "type": "integer", "minimum": 0, "maximum": 10 }
            }
        })
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Rating {
        name: String,
        score: i64,
    }

    impl StructuredOut for Rating {
        fn schema_id() -> SchemaId {
            SchemaId::new("rating.v1")
        }
        fn schema() -> Value {
            strict_object()
        }
    }

    #[test]
    fn a_strict_schema_registers_and_validates_a_good_value() {
        let mut registry = SchemaRegistry::new();
        register_structured::<Rating>(&mut registry).unwrap();
        let value = registry
            .validate(&SchemaId::new("rating.v1"), r#"{"name":"a","score":3}"#)
            .unwrap();
        let typed: Rating = decode(value).unwrap();
        assert_eq!(
            typed,
            Rating {
                name: "a".into(),
                score: 3
            }
        );
    }

    #[test]
    fn registration_rejects_a_non_strict_schema() {
        let mut registry = SchemaRegistry::new();
        let mut loose = strict_object();
        loose["additionalProperties"] = Value::Bool(true);
        let err = registry
            .register(SchemaId::new("loose"), loose)
            .unwrap_err();
        assert!(err.to_string().contains("additionalProperties"), "{err}");

        let mut missing_required = strict_object();
        missing_required.as_object_mut().unwrap().remove("required");
        assert!(registry
            .register(SchemaId::new("noreq"), missing_required)
            .is_err());

        // A property that is not required is not strict either.
        let mut optional = strict_object();
        optional["required"] = serde_json::json!(["name"]);
        assert!(registry.register(SchemaId::new("opt"), optional).is_err());
    }

    #[test]
    fn an_unsupported_construct_is_a_hard_error_not_a_looser_schema() {
        let mut registry = SchemaRegistry::new();

        // A combinator would change acceptance, and the checker cannot enforce it.
        let mut forbidden = strict_object();
        forbidden["oneOf"] = serde_json::json!([]);
        let err = registry
            .register(SchemaId::new("u"), forbidden)
            .unwrap_err();
        assert!(err.to_string().contains("oneOf"), "{err}");

        // A keyword outside the supported subset is refused by name.
        let err = check_strict(
            &serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": [],
                "properties": {},
                "pattern": "^a$"
            }),
            "root",
        )
        .unwrap_err();
        assert!(err.detail.contains("pattern"), "{err}");

        // A union of scalar types *is* supported and must stay registrable.
        let mut union = strict_object();
        union["properties"]["score"] = serde_json::json!({ "type": ["integer", "string"] });
        assert!(registry.register(SchemaId::new("union"), union).is_ok());

        // A `required` naming a field that does not exist can never be satisfied.
        let mut unsatisfiable = strict_object();
        unsatisfiable["required"] = serde_json::json!(["name", "score", "ghost"]);
        assert!(check_strict(&unsatisfiable, "root").is_err());
    }

    #[test]
    fn validation_reports_the_specific_failure() {
        let mut registry = SchemaRegistry::new();
        register_structured::<Rating>(&mut registry).unwrap();
        let id = SchemaId::new("rating.v1");

        let cases = [
            ("", SchemaErrorKind::Empty),
            ("not json", SchemaErrorKind::NotJson),
            ("[1,2]", SchemaErrorKind::TypeMismatch),
            (r#"{"name":"a"}"#, SchemaErrorKind::MissingField),
            (
                r#"{"name":"a","score":1,"extra":true}"#,
                SchemaErrorKind::ExtraField,
            ),
            (r#"{"name":"a","score":99}"#, SchemaErrorKind::OutOfRange),
            (r#"{"name":5,"score":1}"#, SchemaErrorKind::TypeMismatch),
            (r#"{"name":"","score":1}"#, SchemaErrorKind::OutOfRange),
            (r#"{"name":"a","score":1.5}"#, SchemaErrorKind::TypeMismatch),
        ];
        for (raw, expected) in cases {
            let err = registry.validate(&id, raw).unwrap_err();
            assert_eq!(err.kind, expected, "for {raw:?} got {err}");
        }
    }

    #[test]
    fn an_unknown_schema_id_is_a_rejection_not_a_pass() {
        let registry = SchemaRegistry::new();
        let err = registry
            .validate(&SchemaId::new("missing"), "{}")
            .unwrap_err();
        assert_eq!(err.kind, SchemaErrorKind::UnsupportedSchema);
    }

    #[test]
    fn duplicate_registration_is_refused() {
        let mut registry = SchemaRegistry::new();
        registry
            .register(SchemaId::new("x"), strict_object())
            .unwrap();
        assert!(registry
            .register(SchemaId::new("x"), strict_object())
            .is_err());
        assert_eq!(registry.ids().len(), 1);
    }

    #[test]
    fn schema_hashing_ignores_key_order() {
        let a = serde_json::json!({"a": 1, "b": {"c": 2, "d": 3}});
        let b = serde_json::json!({"b": {"d": 3, "c": 2}, "a": 1});
        assert_eq!(schema_sha(&a), schema_sha(&b));
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(schema_sha(&a).len(), 64);
        assert_ne!(
            schema_sha(&a),
            schema_sha(&serde_json::json!({"a": 2, "b": {}}))
        );
    }

    #[test]
    fn a_grammar_fingerprint_separates_decoders() {
        let a = grammar_fingerprint("abc", "llguidance");
        let b = grammar_fingerprint("abc", "xgrammar");
        assert_ne!(a, b);
        assert_eq!(a, grammar_fingerprint("abc", "llguidance"));
    }
}
