//! The seven `data.*` tool handlers.
//!
//! The CM names an entity, a class, typed properties, and a bounded search. It
//! never names quads, SPARQL, SQL, or index documents: this module maps the
//! high-level intent onto [`DataService`](mm_data::DataService) and projects the
//! result into compact, camel-cased JSON.
//!
//! Writes are quota-limited to twelve `create`/`update`/`delete` calls per turn.
//! A quota stop is a `BUDGET` outcome, never a silent success.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use mm_data::data::DataError;
use mm_data::{
    DataBroker, DataCreateRequest, DataDeleteMode, DataDeleteRequest, DataFilter,
    DataMutationResult, DataPatch, DataQuery, DataSearchRequest, DataService, DataUpdateMode,
    DataUpdateRequest, DataValue, IndexHealth, NamespaceBinding, SchemaDetail, SchemaInclude,
    SchemaSpec, SchemaSummary, ShaclViolation, ValidationTarget,
};
use serde_json::{json, Map, Value};

use crate::tool_schemas::ToolErrorBody;

/// The design's per-turn write quota for the data family.
pub const MAX_DATA_WRITES_PER_TURN: u32 = 12;

/// The names this module implements.
pub const DATA_TOOL_NAMES: &[&str] = &[
    "data.create",
    "data.update",
    "data.query",
    "data.delete",
    "data.search",
    "data.schema",
    "data.namespace",
];

/// Whether `name` belongs to the data family.
pub fn is_data_tool(name: &str) -> bool {
    DATA_TOOL_NAMES.contains(&name)
}

/// The data tool handlers over one service.
#[derive(Debug)]
pub struct DataTools {
    service: Arc<DataService>,
    writes: AtomicU32,
}

impl DataTools {
    /// Build handlers over `service`.
    pub fn new(service: Arc<DataService>) -> Self {
        Self {
            service,
            writes: AtomicU32::new(0),
        }
    }

    /// The service behind the handlers.
    pub fn service(&self) -> &Arc<DataService> {
        &self.service
    }

    /// Dispatch one data tool.
    pub async fn dispatch(&self, name: &str, arguments: &Value) -> Result<Value, DataToolError> {
        match name {
            "data.create" => self.create(arguments).await,
            "data.update" => self.update(arguments).await,
            "data.query" => self.query(arguments).await,
            "data.delete" => self.delete(arguments).await,
            "data.search" => self.search(arguments).await,
            "data.schema" => self.schema(arguments).await,
            "data.namespace" => self.namespace(arguments).await,
            other => Err(DataToolError::new(
                "NOT_AVAILABLE",
                format!("{other} is not a data tool"),
            )),
        }
    }

    fn consume_write(&self) -> Result<(), DataToolError> {
        let used = self.writes.load(Ordering::SeqCst);
        if used >= MAX_DATA_WRITES_PER_TURN {
            return Err(DataToolError {
                code: "BUDGET".into(),
                message: format!(
                    "the turn's {MAX_DATA_WRITES_PER_TURN}-write data quota is exhausted"
                ),
                field: None,
                retryable: false,
                next: Some("finish the turn; writes resume next turn".into()),
            });
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn create(&self, arguments: &Value) -> Result<Value, DataToolError> {
        self.consume_write()?;
        let class = string_arg(arguments, "type")?.to_owned();
        let properties = arguments
            .get("properties")
            .ok_or_else(|| DataToolError::missing("properties"))?;
        let patches = patches_from(properties)?;
        let schema = opt_string(arguments, "schema");
        if let Some(schema) = schema {
            self.service
                .catalog()
                .get(schema, SchemaInclude::Summary)
                .await
                .map_err(DataToolError::from)?;
        }
        let request = DataCreateRequest {
            r#type: class,
            id: opt_string(arguments, "id").map(str::to_owned),
            properties: patches,
            schema: schema.map(str::to_owned),
            graph: opt_string(arguments, "graph").map(str::to_owned),
            idempotency_key: opt_string(arguments, "idempotency_key").map(str::to_owned),
        };
        let result = self
            .service
            .create(request)
            .await
            .map_err(DataToolError::from)?;
        Ok(mutation_result(&self.service, &result))
    }

    async fn update(&self, arguments: &Value) -> Result<Value, DataToolError> {
        self.consume_write()?;
        let subject = string_arg(arguments, "subject")?.to_owned();
        let properties = arguments
            .get("properties")
            .ok_or_else(|| DataToolError::missing("properties"))?;
        let patches = patches_from(properties)?;
        let mode = match arguments.get("mode").and_then(Value::as_str) {
            None | Some("merge") => DataUpdateMode::Merge,
            Some("replace") => DataUpdateMode::Replace,
            Some(other) => {
                return Err(DataToolError::invalid(
                    Some("mode"),
                    format!("unsupported mode {other:?}"),
                ))
            }
        };
        let request = DataUpdateRequest {
            subject,
            properties: patches,
            mode,
            expected_revision: opt_u64(arguments, "expected_revision")?,
            idempotency_key: opt_string(arguments, "idempotency_key").map(str::to_owned),
        };
        let result = self
            .service
            .update(request)
            .await
            .map_err(DataToolError::from)?;
        Ok(mutation_result(&self.service, &result))
    }

    async fn query(&self, arguments: &Value) -> Result<Value, DataToolError> {
        let filters = match arguments.get("where") {
            None => Vec::new(),
            Some(where_clause) => self.filters_from(where_clause).await?,
        };
        let limit = opt_u64(arguments, "limit")?.unwrap_or(20) as usize;
        let offset = opt_u64(arguments, "offset")?.unwrap_or(0) as usize;
        let query = DataQuery {
            class: opt_string(arguments, "class").map(str::to_owned),
            subject: opt_string(arguments, "subject").map(str::to_owned),
            filters,
            graph: opt_string(arguments, "graph").map(str::to_owned),
            namespace: opt_string(arguments, "namespace").map(str::to_owned),
            include_deleted: arguments
                .get("include_deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            limit,
            offset,
        };
        query.validate().map_err(DataToolError::from)?;
        let include = match arguments.get("include") {
            None => None,
            Some(values) => Some(self.expand_include(values).await?),
        };
        let page = self
            .service
            .query(query)
            .await
            .map_err(DataToolError::from)?;
        let entities = page
            .entities
            .iter()
            .map(|entity| entity_json(entity, include.as_deref()))
            .collect::<Vec<_>>();
        Ok(json!({
            "entities": entities,
            "count": entities.len(),
            "hasMore": page.has_more,
            "nextOffset": page.next_offset,
            "dataRevision": self.service.data_revision(),
        }))
    }

    async fn delete(&self, arguments: &Value) -> Result<Value, DataToolError> {
        self.consume_write()?;
        let mode = match arguments.get("mode").and_then(Value::as_str) {
            None | Some("soft") => DataDeleteMode::Soft,
            Some("hard") => DataDeleteMode::Hard,
            Some(other) => {
                return Err(DataToolError::invalid(
                    Some("mode"),
                    format!("unsupported mode {other:?}"),
                ))
            }
        };
        let request = DataDeleteRequest {
            subject: string_arg(arguments, "subject")?.to_owned(),
            mode,
            expected_revision: opt_u64(arguments, "expected_revision")?,
            reason: opt_string(arguments, "reason").map(str::to_owned),
            idempotency_key: opt_string(arguments, "idempotency_key").map(str::to_owned),
            // The CM has no hard-delete authority; only an operator path does.
            authority: None,
        };
        let result = self
            .service
            .delete(request)
            .await
            .map_err(DataToolError::from)?;
        Ok(mutation_result(&self.service, &result))
    }

    async fn search(&self, arguments: &Value) -> Result<Value, DataToolError> {
        let request = DataSearchRequest {
            query: string_arg(arguments, "query")?.to_owned(),
            class: opt_string(arguments, "class").map(str::to_owned),
            graph: opt_string(arguments, "graph").map(str::to_owned),
            namespace: opt_string(arguments, "namespace").map(str::to_owned),
            limit: opt_u64(arguments, "limit")?.unwrap_or(10) as usize,
            snippet: arguments
                .get("snippet")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            include_deleted: arguments
                .get("include_deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        request.validate().map_err(DataToolError::from)?;
        let result = self
            .service
            .search(request)
            .await
            .map_err(DataToolError::from)?;
        let hits = result
            .hits
            .iter()
            .map(|hit| {
                json!({
                    "iri": hit.iri,
                    "type": hit.r#type,
                    "label": hit.label,
                    "scoreMicros": hit.score_micros,
                    "score": (hit.score_micros as f64) / 1_000_000.0,
                    "snippet": hit.snippet,
                    "graph": hit.graph,
                    "revision": hit.revision,
                    "updatedAt": hit.updated_at,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "hits": hits,
            "count": hits.len(),
            "index": index_name(result.index),
            "indexPending": result.index == IndexHealth::Pending,
            "dataRevision": self.service.data_revision(),
        }))
    }

    async fn schema(&self, arguments: &Value) -> Result<Value, DataToolError> {
        let action = string_arg(arguments, "action")?;
        match action {
            "list" => {
                let schemas = self
                    .service
                    .list_schemas()
                    .await
                    .map_err(DataToolError::from)?;
                Ok(json!({
                    "schemas": schemas.iter().map(schema_json).collect::<Vec<_>>(),
                    "count": schemas.len(),
                    "dataRevision": self.service.data_revision(),
                }))
            }
            "get" => {
                let id = string_arg(arguments, "schema")?;
                let include = schema_include(arguments);
                let detail = self
                    .service
                    .get_schema(id, include)
                    .await
                    .map_err(DataToolError::from)?;
                Ok(json!({
                    "schema": schema_detail_json(&detail),
                    "dataRevision": self.service.data_revision(),
                }))
            }
            "validate" => {
                let target = self.validation_target(arguments).await?;
                let violations = self
                    .service
                    .validate(target)
                    .await
                    .map_err(DataToolError::from)?;
                Ok(json!({
                    "conforms": violations.is_empty(),
                    "violations": violations.iter().map(violation_json).collect::<Vec<_>>(),
                    "count": violations.len(),
                    "dataRevision": self.service.data_revision(),
                }))
            }
            "register" => {
                self.consume_write()?;
                let prefix = string_arg(arguments, "prefix")?;
                let namespace = string_arg(arguments, "namespace")?;
                let source = opt_string(arguments, "source").unwrap_or("inline");
                let body = self.schema_body(arguments, source).await?;
                let spec = SchemaSpec {
                    id: prefix.to_owned(),
                    prefix: prefix.to_owned(),
                    namespace: namespace.to_owned(),
                    version: "1".to_owned(),
                    source: source.to_owned(),
                    body,
                    format: opt_string(arguments, "format").map(str::to_owned),
                };
                let summary = self
                    .service
                    .catalog()
                    .register(spec)
                    .await
                    .map_err(DataToolError::from)?;
                Ok(json!({
                    "schema": schema_json(&summary),
                    "dataRevision": self.service.data_revision(),
                }))
            }
            "activate" | "deprecate" => {
                self.consume_write()?;
                let id = string_arg(arguments, "schema")?;
                let summary = if action == "activate" {
                    self.service.catalog().activate(id).await
                } else {
                    self.service.catalog().deprecate(id).await
                }
                .map_err(DataToolError::from)?;
                Ok(json!({
                    "schema": schema_json(&summary),
                    "dataRevision": self.service.data_revision(),
                }))
            }
            other => Err(DataToolError::invalid(
                Some("action"),
                format!("unsupported action {other:?}"),
            )),
        }
    }

    async fn namespace(&self, arguments: &Value) -> Result<Value, DataToolError> {
        let action = string_arg(arguments, "action")?;
        match action {
            "list" => {
                let bindings = self
                    .service
                    .list_namespaces()
                    .await
                    .map_err(DataToolError::from)?;
                Ok(json!({
                    "namespaces": bindings.iter().map(namespace_json).collect::<Vec<_>>(),
                    "count": bindings.len(),
                    "dataRevision": self.service.data_revision(),
                }))
            }
            "bind" => {
                self.consume_write()?;
                let binding = NamespaceBinding {
                    prefix: string_arg(arguments, "prefix")?.to_owned(),
                    namespace: string_arg(arguments, "namespace")?.to_owned(),
                    builtin: false,
                };
                let binding = self
                    .service
                    .namespaces()
                    .register(binding)
                    .await
                    .map_err(DataToolError::from)?;
                Ok(json!({
                    "namespace": namespace_json(&binding),
                    "dataRevision": self.service.data_revision(),
                }))
            }
            "expand" => {
                let iri = string_arg(arguments, "iri")?;
                let expanded = self
                    .service
                    .expand(iri)
                    .await
                    .map_err(DataToolError::from)?;
                Ok(json!({ "compact": iri, "iri": expanded }))
            }
            "resolve" => {
                let iri = string_arg(arguments, "iri")?;
                let compact = self
                    .service
                    .resolve(iri)
                    .await
                    .map_err(DataToolError::from)?;
                Ok(json!({ "iri": iri, "compact": compact }))
            }
            other => Err(DataToolError::invalid(
                Some("action"),
                format!("unsupported action {other:?}"),
            )),
        }
    }

    async fn filters_from(&self, value: &Value) -> Result<Vec<DataFilter>, DataToolError> {
        let Some(object) = value.as_object() else {
            return Err(DataToolError::invalid(
                Some("where"),
                "where must be an object",
            ));
        };
        let mut filters = Vec::new();
        for (predicate, value) in object {
            let predicate = self
                .service
                .expand(predicate)
                .await
                .map_err(DataToolError::from)?;
            match value {
                Value::String(text) => filters.push(DataFilter::Equals {
                    predicate,
                    value: text.clone(),
                }),
                Value::Number(number) => filters.push(DataFilter::Equals {
                    predicate,
                    value: number.to_string(),
                }),
                Value::Bool(flag) => filters.push(DataFilter::Equals {
                    predicate,
                    value: flag.to_string(),
                }),
                Value::Object(map) => {
                    if let Some(iri) = map.get("iri").and_then(Value::as_str) {
                        if map.len() != 1 {
                            return Err(DataToolError::invalid(
                                Some(&format!("where.{predicate}")),
                                "an iri filter has exactly one key",
                            ));
                        }
                        filters.push(DataFilter::Iri {
                            predicate,
                            value: iri.to_owned(),
                        });
                    } else if let Some(needle) = map.get("$contains").and_then(Value::as_str) {
                        if map.len() != 1 {
                            return Err(DataToolError::invalid(
                                Some(&format!("where.{predicate}")),
                                "a $contains filter has exactly one key",
                            ));
                        }
                        filters.push(DataFilter::Contains {
                            predicate,
                            value: needle.to_owned(),
                        });
                    } else {
                        return Err(DataToolError::invalid(
                            Some(&format!("where.{predicate}")),
                            "unknown filter operator",
                        ));
                    }
                }
                _ => {
                    return Err(DataToolError::invalid(
                        Some(&format!("where.{predicate}")),
                        "filter must be a value, {iri}, or {$contains}",
                    ))
                }
            }
        }
        Ok(filters)
    }

    async fn expand_include(&self, value: &Value) -> Result<Vec<String>, DataToolError> {
        let Some(values) = value.as_array() else {
            return Err(DataToolError::invalid(
                Some("include"),
                "include must be an array of property names",
            ));
        };
        let mut out = Vec::with_capacity(values.len());
        for value in values {
            let Some(name) = value.as_str() else {
                return Err(DataToolError::invalid(
                    Some("include"),
                    "include must contain strings",
                ));
            };
            out.push(
                self.service
                    .expand(name)
                    .await
                    .map_err(DataToolError::from)?,
            );
        }
        Ok(out)
    }

    async fn validation_target(
        &self,
        arguments: &Value,
    ) -> Result<ValidationTarget, DataToolError> {
        let subject = opt_string(arguments, "subject");
        let class = opt_string(arguments, "class");
        let graph = opt_string(arguments, "graph");
        let given = [subject.is_some(), class.is_some(), graph.is_some()]
            .into_iter()
            .filter(|present| *present)
            .count();
        if given != 1 {
            return Err(DataToolError::invalid(
                Some("validate"),
                "validate needs exactly one of subject, class, or graph",
            ));
        }
        Ok(if let Some(subject) = subject {
            ValidationTarget::Subject(subject.to_owned())
        } else if let Some(class) = class {
            ValidationTarget::Class(class.to_owned())
        } else {
            ValidationTarget::Graph(graph.unwrap_or("world").to_owned())
        })
    }

    async fn schema_body(&self, arguments: &Value, source: &str) -> Result<String, DataToolError> {
        if let Some(body) = opt_string(arguments, "body") {
            return Ok(body.to_owned());
        }
        if source == "inline" || source == "builtin" {
            return Err(DataToolError::missing("body"));
        }
        if source.contains("..") || std::path::Path::new(source).is_absolute() {
            return Err(DataToolError::invalid(
                Some("source"),
                "a schema file must be relative to the ontology directory",
            ));
        }
        let path = mm_core::Config::repo_root().join("ontology").join(source);
        std::fs::read_to_string(&path).map_err(|error| DataToolError {
            code: "SCHEMA_VIOLATION".into(),
            message: format!("cannot read {}: {error}", path.display()),
            field: Some("source".into()),
            retryable: false,
            next: Some("use an ontology-relative file or inline body".into()),
        })
    }
}

/// A typed data-tool failure, mapped to the envelope's error codes.
#[derive(Debug, Clone)]
pub struct DataToolError {
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

impl DataToolError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            field: None,
            retryable: false,
            next: None,
        }
    }

    fn invalid(field: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_ARGUMENTS".into(),
            message: message.into(),
            field: field.map(str::to_owned),
            retryable: false,
            next: Some("fix the call and retry".into()),
        }
    }

    fn missing(field: &str) -> Self {
        Self::invalid(Some(field), "missing required field")
    }

    /// The envelope error body.
    pub fn body(&self) -> ToolErrorBody {
        ToolErrorBody {
            code: self.code.clone(),
            message: self.message.clone(),
            field: self.field.clone(),
            retryable: self.retryable,
            next: self.next.clone(),
        }
    }
}

impl From<DataError> for DataToolError {
    fn from(error: DataError) -> Self {
        match error {
            DataError::Invalid(message) => DataToolError::invalid(None, message),
            DataError::NotFound(message) => DataToolError {
                code: "NOT_FOUND".into(),
                message,
                field: None,
                retryable: false,
                next: Some("query the entity to confirm it exists".into()),
            },
            DataError::Conflict(message) => DataToolError {
                code: "DATA_CONFLICT".into(),
                message,
                field: Some("expected_revision".into()),
                retryable: true,
                next: Some("re-read the entity and retry once".into()),
            },
            DataError::SchemaViolation(message) => DataToolError {
                code: "SCHEMA_VIOLATION".into(),
                message,
                field: None,
                retryable: false,
                next: Some("fix the type or properties".into()),
            },
            DataError::NamespaceConflict(message) => DataToolError {
                code: "NAMESPACE_CONFLICT".into(),
                message,
                field: Some("prefix".into()),
                retryable: false,
                next: Some("choose another prefix".into()),
            },
            DataError::SearchUnavailable(message) => DataToolError {
                code: "SEARCH_UNAVAILABLE".into(),
                message,
                field: None,
                retryable: true,
                next: Some("use data.query for an exact read".into()),
            },
            other => DataToolError::new("INTERNAL", other.to_string()),
        }
    }
}

fn patches_from(value: &Value) -> Result<Vec<DataPatch>, DataToolError> {
    let Some(object) = value.as_object() else {
        return Err(DataToolError::invalid(
            Some("properties"),
            "properties must be an object",
        ));
    };
    let mut patches = Vec::with_capacity(object.len());
    for (predicate, value) in object {
        if predicate.trim().is_empty() {
            return Err(DataToolError::invalid(
                Some("properties"),
                "predicate names must not be empty",
            ));
        }
        let values = match value {
            Value::Null => Vec::new(),
            Value::Array(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(value_from(item, predicate)?);
                }
                values
            }
            other => vec![value_from(other, predicate)?],
        };
        patches.push(DataPatch {
            predicate: predicate.clone(),
            values,
        });
    }
    Ok(patches)
}

fn value_from(value: &Value, predicate: &str) -> Result<DataValue, DataToolError> {
    match value {
        Value::String(text) => Ok(DataValue::Literal {
            value: text.clone(),
            datatype: None,
            language: None,
        }),
        Value::Bool(flag) => Ok(DataValue::Literal {
            value: flag.to_string(),
            datatype: Some("http://www.w3.org/2001/XMLSchema#boolean".into()),
            language: None,
        }),
        Value::Number(number) => {
            let datatype = if number.is_i64() || number.is_u64() {
                "http://www.w3.org/2001/XMLSchema#integer"
            } else {
                "http://www.w3.org/2001/XMLSchema#double"
            };
            Ok(DataValue::Literal {
                value: number.to_string(),
                datatype: Some(datatype.into()),
                language: None,
            })
        }
        Value::Object(map) => match map.get("iri").and_then(Value::as_str) {
            Some(iri) if map.len() == 1 => Ok(DataValue::Iri {
                iri: iri.to_owned(),
            }),
            _ => Err(DataToolError::invalid(
                Some(&format!("properties.{predicate}")),
                "an object value must be exactly {iri: \"...\"}",
            )),
        },
        _ => Err(DataToolError::invalid(
            Some(&format!("properties.{predicate}")),
            "values must be strings, numbers, booleans, lists, or {iri}",
        )),
    }
}

fn mutation_result(service: &DataService, result: &DataMutationResult) -> Value {
    json!({
        "entity": entity_json(&result.entity, None),
        "unchanged": result.unchanged,
        "duplicate": result.duplicate,
        "index": index_name(result.index),
        "indexPending": result.index == IndexHealth::Pending,
        "dataRevision": service.data_revision(),
    })
}

fn entity_json(entity: &mm_data::DataEntity, include: Option<&[String]>) -> Value {
    let properties: Map<String, Value> = entity
        .properties
        .iter()
        .filter(|(predicate, _)| {
            include.is_none_or(|names| names.iter().any(|name| name == *predicate))
        })
        .map(|(predicate, values)| {
            (
                predicate.clone(),
                Value::Array(
                    values
                        .iter()
                        .filter_map(|value| serde_json::to_value(value).ok())
                        .collect(),
                ),
            )
        })
        .collect();
    json!({
        "iri": entity.iri,
        "types": entity.types,
        "label": entity.label,
        "properties": properties,
        "graph": entity.graph,
        "revision": entity.revision,
        "status": entity.status,
        "createdAt": entity.created_at,
        "updatedAt": entity.updated_at,
        "recordedBy": entity.recorded_by,
    })
}

fn schema_json(summary: &SchemaSummary) -> Value {
    json!({
        "id": summary.id,
        "prefix": summary.prefix,
        "namespace": summary.namespace,
        "version": summary.version,
        "source": summary.source,
        "active": summary.active,
        "classCount": summary.class_count,
        "propertyCount": summary.property_count,
        "loadedAt": summary.loaded_at,
    })
}

fn schema_detail_json(detail: &SchemaDetail) -> Value {
    let mut value = schema_json(&detail.summary);
    if let Some(object) = value.as_object_mut() {
        object.insert("classes".into(), json!(detail.classes));
        object.insert("properties".into(), json!(detail.properties));
        object.insert("shapes".into(), json!(detail.shapes));
        object.insert("imports".into(), json!(detail.imports));
        object.insert("body".into(), json!(detail.body));
    }
    value
}

fn violation_json(violation: &ShaclViolation) -> Value {
    json!({
        "focus": violation.focus,
        "path": violation.path,
        "severity": violation.severity,
        "message": violation.message,
    })
}

fn namespace_json(binding: &NamespaceBinding) -> Value {
    json!({
        "prefix": binding.prefix,
        "namespace": binding.namespace,
        "builtin": binding.builtin,
    })
}

fn schema_include(arguments: &Value) -> SchemaInclude {
    let Some(values) = arguments.get("include").and_then(Value::as_array) else {
        return SchemaInclude::Summary;
    };
    let names: Vec<&str> = values.iter().filter_map(Value::as_str).collect();
    if names.is_empty() {
        return SchemaInclude::Summary;
    }
    if names.len() > 1 || names.contains(&"all") || names.contains(&"body") {
        return SchemaInclude::All;
    }
    match names[0] {
        "classes" => SchemaInclude::Classes,
        "properties" => SchemaInclude::Properties,
        "shapes" => SchemaInclude::Shapes,
        "imports" => SchemaInclude::Imports,
        _ => SchemaInclude::Summary,
    }
}

fn index_name(index: IndexHealth) -> &'static str {
    match index {
        IndexHealth::Healthy => "healthy",
        IndexHealth::Pending => "pending",
        IndexHealth::Unavailable => "unavailable",
    }
}

fn string_arg<'a>(arguments: &'a Value, field: &str) -> Result<&'a str, DataToolError> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| DataToolError::missing(field))
}

fn opt_string<'a>(arguments: &'a Value, field: &str) -> Option<&'a str> {
    arguments.get(field).and_then(Value::as_str)
}

fn opt_u64(arguments: &Value, field: &str) -> Result<Option<u64>, DataToolError> {
    match arguments.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            DataToolError::invalid(
                Some(field),
                format!("{field} must be a non-negative integer"),
            )
        }),
    }
}
