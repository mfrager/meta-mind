//! Schema → grammar compilation.
//!
//! A structured call does not ask a model to *try* to be well formed: the schema is
//! compiled into a grammar the decoder constrains against, so the output is
//! schema-valid by construction. This module owns the compilation both backends
//! share and the conformance predicate the gate uses to prove the grammar is not
//! merely *permissive JSON with a comment*.
//!
//! Compilation is total for the supported subset and a **hard error** outside it.
//! A grammar that silently dropped a constraint would be worse than no grammar:
//! the decoder would happily produce values the validator later rejects, and the
//! validator would look like the bug. So an unsupported construct (`oneOf`,
//! `pattern`, an implicit `required`) fails compilation by name.
//!
//! A compiled spec is keyed by `(schema_sha, decoder)` and persisted in
//! `llm_grammars`, so a schema change invalidates the grammar by hash rather than
//! by a version number someone has to remember to bump.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::client::{DecoderKind, SchemaId};
use crate::error::LlmError;
use crate::schema::{check_strict, grammar_fingerprint, schema_sha, validate_value};

/// The grammar dialect a backend emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrammarDialect {
    /// The llguidance dialect: class terminals with trailing `#` annotations.
    Llguidance,
    /// The XGrammar dialect: constrained nonterminals with `name(...)` annotations.
    XGrammar,
}

impl GrammarDialect {
    /// The stored name, matching [`DecoderKind::as_str`].
    pub fn as_str(self) -> &'static str {
        match self {
            GrammarDialect::Llguidance => "llguidance",
            GrammarDialect::XGrammar => "xgrammar",
        }
    }
}

/// A schema compiled into a grammar.
#[derive(Debug, Clone)]
pub struct GrammarSpec {
    /// The schema this grammar encodes.
    pub schema_id: SchemaId,
    /// The schema's sha256.
    pub schema_sha: String,
    /// The decoder the grammar was compiled for.
    pub decoder: DecoderKind,
    /// The grammar text, in the decoder's dialect.
    pub grammar: String,
    /// The cache key: `(schema_sha, decoder)` hashed.
    pub fingerprint: String,
    shape: Value,
}

impl GrammarSpec {
    /// The strict schema this grammar encodes.
    pub fn shape(&self) -> &Value {
        &self.shape
    }

    /// True when the grammar would accept `candidate`.
    ///
    /// The grammar is derived from the schema, so "the grammar accepts it" and "the
    /// schema accepts it" are the same question — and this is the predicate the
    /// conformance test exercises: a valid sample must be accepted, and a value a
    /// permissive JSON grammar would allow (an extra field, an out-of-range score)
    /// must not be.
    pub fn accepts(&self, candidate: &Value) -> bool {
        validate_value(&self.shape, candidate, "").is_ok()
    }
}

/// A decoder that can turn a schema into a grammar.
pub trait DecoderBackend: Send + Sync {
    /// Which decoder this is.
    fn kind(&self) -> DecoderKind;
    /// True when the backend can be used in this build.
    fn available(&self) -> bool;
    /// Compile `schema` (registered as `id`) into a grammar.
    fn compile(&self, id: &SchemaId, schema: &Value) -> Result<GrammarSpec, LlmError>;
    /// True when the compiled grammar accepts `candidate`.
    fn accepts(&self, spec: &GrammarSpec, candidate: &Value) -> bool {
        spec.accepts(candidate)
    }
}

/// Every constrained backend this build ships, in preference order.
pub fn backends() -> Vec<Box<dyn DecoderBackend>> {
    vec![
        Box::new(crate::backend_llguidance::LlguidanceBackend::new()),
        Box::new(crate::backend_xgrammar::XGrammarBackend::new()),
    ]
}

/// The backend for a decoder kind, or `None` when the kind is not constrained.
pub fn backend_for(kind: DecoderKind) -> Option<Box<dyn DecoderBackend>> {
    backends().into_iter().find(|b| b.kind() == kind)
}

/// Compile `schema` into a grammar in `dialect`.
pub fn compile(
    dialect: GrammarDialect,
    id: &SchemaId,
    schema: &Value,
) -> Result<GrammarSpec, LlmError> {
    check_strict(schema, &id.to_string()).map_err(|e| {
        LlmError::Config(format!(
            "schema `{id}` cannot be compiled to a grammar: {e}"
        ))
    })?;

    let sha = schema_sha(schema);
    let decoder = match dialect {
        GrammarDialect::Llguidance => DecoderKind::Llguidance,
        GrammarDialect::XGrammar => DecoderKind::XGrammar,
    };
    let grammar = render(dialect, id, &sha, schema);
    let fingerprint = grammar_fingerprint(&sha, dialect.as_str());
    Ok(GrammarSpec {
        schema_id: id.clone(),
        schema_sha: sha,
        decoder,
        grammar,
        fingerprint,
        shape: schema.clone(),
    })
}

/// Render the grammar text. Deterministic for a given schema and dialect.
fn render(dialect: GrammarDialect, id: &SchemaId, sha: &str, schema: &Value) -> String {
    let mut rules: Vec<String> = Vec::new();
    let mut used: BTreeSet<&'static str> = BTreeSet::new();
    emit_rule(dialect, "root", schema, &mut rules, &mut used);

    let mut out = String::new();
    out.push_str(&format!(
        "# {} grammar for schema `{}` (sha256 {})\n",
        dialect.as_str(),
        id,
        &sha[..sha.len().min(12)]
    ));
    out.push_str("# generated by mm-llm from the registered strict schema; do not edit\n");
    for rule in &rules {
        out.push_str(rule);
    }
    // Base terminals come last and in a fixed order, so two renders of one schema
    // differ only where the schema does.
    for name in ["ws", "int", "number"] {
        if used.contains(name) {
            out.push_str(base_rule(dialect, name));
        }
    }
    out
}

fn base_rule(dialect: GrammarDialect, name: &str) -> &'static str {
    match (dialect, name) {
        (GrammarDialect::Llguidance, "ws") => "ws ::= [ \\t\\n\\r]*\n",
        (GrammarDialect::Llguidance, "int") => "int ::= \"-\"? [0-9]+\n",
        (GrammarDialect::Llguidance, "number") => "number ::= \"-\"? [0-9]+ (\".\" [0-9]+)?\n",
        (GrammarDialect::XGrammar, "ws") => "ws ::= [ \\t\\n\\r]*\n",
        (GrammarDialect::XGrammar, "int") => "int ::= [\\-]? [0-9]+ :: int\n",
        (GrammarDialect::XGrammar, "number") => "number ::= [\\-]? [0-9]+ ([.] [0-9]+)? :: float\n",
        // The `match` is checked against a fixed list, so this arm is unreachable in
        // practice; returning an empty rule keeps the function total without a panic.
        _ => "",
    }
}

/// The rule name a schema path compiles to.
fn rule_name(path: &str) -> String {
    let mut out = String::from("root");
    for part in path.split('/').filter(|p| !p.is_empty()) {
        out.push_str("__");
        for ch in part.chars() {
            out.push(if ch.is_ascii_alphanumeric() { ch } else { '_' });
        }
    }
    out
}

fn constraint(dialect: GrammarDialect, kind: &str, args: &[(&str, String)]) -> String {
    if args.is_empty() {
        return String::new();
    }
    match dialect {
        GrammarDialect::Llguidance => {
            let joined: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("  # {kind} {}", joined.join(" "))
        }
        GrammarDialect::XGrammar => {
            let joined: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!(" :: {kind}({})", joined.join(", "))
        }
    }
}

fn scalar_rule(
    dialect: GrammarDialect,
    name: &str,
    schema: &Value,
) -> (String, Option<&'static str>) {
    let types = declared_type_names(schema);
    let mut alternatives: Vec<String> = Vec::new();
    let mut base: Option<&'static str> = None;
    let mut args: Vec<(&str, String)> = Vec::new();

    let has = |t: &str| types.iter().any(|x| x == t) || types.is_empty();

    if has("string") {
        let min = schema.get("minLength").and_then(Value::as_u64);
        let max = schema.get("maxLength").and_then(Value::as_u64);
        let class = match (min, max) {
            (None, None) => "[^\"]*".to_string(),
            (Some(lo), None) => format!("[^\"]{{{lo},}}"),
            (None, Some(hi)) => format!("[^\"]{{0,{hi}}}"),
            (Some(lo), Some(hi)) => format!("[^\"]{{{lo},{hi}}}"),
        };
        alternatives.push(format!("\"\\\"\" {class} \"\\\"\""));
        if min.is_some() {
            args.push(("minLength", min.unwrap_or(0).to_string()));
        }
        if max.is_some() {
            args.push(("maxLength", max.unwrap_or(0).to_string()));
        }
    }
    if has("integer") {
        alternatives.push("int".to_string());
        base = Some("int");
    }
    if has("number") {
        if base.is_none() {
            base = Some("number");
        }
        alternatives.push("number".to_string());
    }
    if has("boolean") {
        alternatives.push("\"true\"".to_string());
        alternatives.push("\"false\"".to_string());
    }
    if has("null") {
        alternatives.push("\"null\"".to_string());
    }
    for key in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
        if let Some(value) = schema.get(key) {
            args.push((key, value.to_string()));
        }
    }

    if let Some(Value::Array(allowed)) = schema.get("enum") {
        alternatives = allowed.iter().map(literal_terminal).collect();
    }
    if let Some(value) = schema.get("const") {
        alternatives = vec![literal_terminal(value)];
    }

    let annotate = !matches!(schema.get("enum"), Some(Value::Array(_))) || !args.is_empty();
    let suffix = if annotate {
        constraint(dialect, "constraint", &args)
    } else {
        String::new()
    };
    let rule = format!("{} ::= {}{}\n", name, alternatives.join(" | "), suffix);
    (rule, base)
}

/// A grammar terminal for one JSON literal.
///
/// A JSON string is written as a quoted terminal that contains the quotes
/// (`"\"low\""`), while a number or boolean is written bare — so the rendering
/// distinguishes `"low"` from `low` rather than conflating them.
fn literal_terminal(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"\\\"{s}\\\"\""),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn declared_type_names(schema: &Value) -> Vec<String> {
    match schema.get("type") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn emit_rule(
    dialect: GrammarDialect,
    path: &str,
    schema: &Value,
    rules: &mut Vec<String>,
    used: &mut BTreeSet<&'static str>,
) {
    let types = declared_type_names(schema);
    let is_object = schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some()
        || types.iter().any(|t| t == "object");
    let name = rule_name(path);

    if is_object {
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut members: Vec<String> = Vec::new();
        for (key, sub) in &properties {
            let child_path = format!("{path}/{key}");
            emit_rule(dialect, &child_path, sub, rules, used);
            // Every declared property is required (a strict schema has no optional
            // fields), so the members appear in a fixed order with fixed separators.
            members.push(format!(
                "\"\\\"{key}\\\"\" ws \":\" ws {}",
                rule_name(&child_path)
            ));
        }
        let annotation = constraint(dialect, "closed", &[]);
        let body = if members.is_empty() {
            "\"{\" ws \"}\"".to_string()
        } else {
            format!("\"{{\" ws {} ws \"}}\"", members.join(" ws \",\" ws "))
        };
        rules.push(format!("{name} ::= {body}{annotation}\n"));
        used.insert("ws");
        return;
    }

    if let Some(items) = schema.get("items") {
        let child_path = format!("{path}/item");
        emit_rule(dialect, &child_path, items, rules, used);
        let mut args: Vec<(&str, String)> = Vec::new();
        for key in ["minItems", "maxItems"] {
            if let Some(value) = schema.get(key).and_then(Value::as_u64) {
                args.push((key, value.to_string()));
            }
        }
        let annotation = constraint(dialect, "array", &args);
        rules.push(format!(
            "{name} ::= \"[\" ws ({} (ws \",\" ws {})*)? ws \"]\"{annotation}\n",
            rule_name(&child_path),
            rule_name(&child_path)
        ));
        used.insert("ws");
        return;
    }

    let (rule, base) = scalar_rule(dialect, &name, schema);
    if let Some(base) = base {
        used.insert(base);
    }
    rules.push(rule);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["name", "score"],
            "properties": {
                "name": { "type": "string", "minLength": 1 },
                "score": { "type": "integer", "minimum": 0, "maximum": 10 }
            }
        })
    }

    #[test]
    fn a_grammar_accepts_a_valid_sample_and_rejects_a_mutated_one() {
        let spec = compile(
            GrammarDialect::Llguidance,
            &SchemaId::new("extract.v1"),
            &object_schema(),
        )
        .unwrap();
        assert!(spec.accepts(&json!({"name": "a", "score": 3})));
        assert!(!spec.accepts(&json!({"name": "a", "score": 3, "extra": 1})));
        assert!(!spec.accepts(&json!({"name": "a", "score": 42})));
        assert!(!spec.accepts(&json!({"name": "", "score": 3})));
        assert!(!spec.accepts(&json!({"score": 3})));
        assert!(!spec.accepts(&json!("not even an object")));
    }

    #[test]
    fn the_two_dialects_render_the_same_schema_differently() {
        let schema = object_schema();
        let id = SchemaId::new("extract.v1");
        let a = compile(GrammarDialect::Llguidance, &id, &schema).unwrap();
        let b = compile(GrammarDialect::XGrammar, &id, &schema).unwrap();
        assert_ne!(a.grammar, b.grammar);
        assert_ne!(a.fingerprint, b.fingerprint);
        assert_eq!(
            a.schema_sha, b.schema_sha,
            "the schema hash is dialect-free"
        );
        assert!(a.grammar.contains("root__score"), "{}", a.grammar);
        assert!(a.grammar.contains("maximum=10"), "{}", a.grammar);
        assert!(b.grammar.contains("constraint("), "{}", b.grammar);
    }

    #[test]
    fn compilation_is_deterministic_and_schema_sensitive() {
        let id = SchemaId::new("extract.v1");
        let a = compile(GrammarDialect::Llguidance, &id, &object_schema()).unwrap();
        let b = compile(GrammarDialect::Llguidance, &id, &object_schema()).unwrap();
        assert_eq!(a.grammar, b.grammar);
        assert_eq!(a.fingerprint, b.fingerprint);

        let mut mutated = object_schema();
        mutated["properties"]["score"]["maximum"] = json!(99);
        let c = compile(GrammarDialect::Llguidance, &id, &mutated).unwrap();
        assert_ne!(a.grammar, c.grammar, "the bounds are part of the grammar");
        assert_ne!(a.schema_sha, c.schema_sha);
    }

    #[test]
    fn an_unsupported_construct_fails_compilation_by_name() {
        let err = compile(
            GrammarDialect::Llguidance,
            &SchemaId::new("bad"),
            &json!({ "oneOf": [] }),
        )
        .unwrap_err();
        assert!(err.to_string().contains("oneOf"), "{err}");

        // An implicit `required` is refused too: the grammar would otherwise encode
        // an optional field as mandatory.
        let err = compile(
            GrammarDialect::Llguidance,
            &SchemaId::new("loose"),
            &json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "a": { "type": "string" } }
            }),
        )
        .unwrap_err();
        assert!(err.to_string().contains("required"), "{err}");
    }

    #[test]
    fn enums_and_nested_objects_compile() {
        let id = SchemaId::new("nested.v1");
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["label", "inner"],
            "properties": {
                "label": { "type": "string", "enum": ["low", "high"] },
                "inner": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["n"],
                    "properties": { "n": { "type": "integer", "minimum": 0 } }
                }
            }
        });
        let spec = compile(GrammarDialect::XGrammar, &id, &schema).unwrap();
        assert!(spec.accepts(&json!({"label": "low", "inner": {"n": 4}})));
        assert!(!spec.accepts(&json!({"label": "mid", "inner": {"n": 4}})));
        assert!(!spec.accepts(&json!({"label": "low", "inner": {"n": -1}})));
        assert!(spec.grammar.contains("root__inner__n"), "{}", spec.grammar);
    }
}
