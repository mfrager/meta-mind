//! The schema catalog, as RDF in the `schema` graph.
//!
//! Built-in prefixes are immutable. A registered schema is parsed and recorded
//! inactive; activation makes its shapes eligible for routing and validation;
//! deprecation is refused while instance data still uses its classes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use oxrdfio::{RdfFormat, RdfParser};
use sha2::{Digest, Sha256};

use crate::engine::RdfEngine;
use crate::rdf::{iri, lit, now_ms, MMD_NS, OWL_NS, RDF_TYPE};

use super::namespaces::{BUILTINS, SCHEMA_GRAPH};
use super::rdf_mapper::local_name;
use super::{
    DataError, SchemaDetail, SchemaInclude, SchemaSummary, ShaclViolation, ValidationTarget,
};

/// `mm:Schema`.
pub const MM_SCHEMA: &str = "https://metamind.dev/ontology#Schema";
/// `mm:schemaId`.
pub const MM_SCHEMA_ID: &str = "https://metamind.dev/ontology#schemaId";
/// `mm:schemaPrefix`.
pub const MM_SCHEMA_PREFIX: &str = "https://metamind.dev/ontology#schemaPrefix";
/// `mm:schemaNamespace`.
pub const MM_SCHEMA_NAMESPACE: &str = "https://metamind.dev/ontology#schemaNamespace";
/// `mm:schemaVersion`.
pub const MM_SCHEMA_VERSION: &str = "https://metamind.dev/ontology#schemaVersion";
/// `mm:schemaSource`.
pub const MM_SCHEMA_SOURCE: &str = "https://metamind.dev/ontology#schemaSource";
/// `mm:schemaActive`.
pub const MM_SCHEMA_ACTIVE: &str = "https://metamind.dev/ontology#schemaActive";
/// `mm:schemaEtag`.
pub const MM_SCHEMA_ETAG: &str = "https://metamind.dev/ontology#schemaEtag";
/// `mm:schemaLoadedAt`.
pub const MM_SCHEMA_LOADED_AT: &str = "https://metamind.dev/ontology#schemaLoadedAt";
/// `mm:schemaClass`.
pub const MM_SCHEMA_CLASS: &str = "https://metamind.dev/ontology#schemaClass";
/// `mm:schemaProperty`.
pub const MM_SCHEMA_PROPERTY: &str = "https://metamind.dev/ontology#schemaProperty";
/// `mm:schemaShape`.
pub const MM_SCHEMA_SHAPE: &str = "https://metamind.dev/ontology#schemaShape";
/// `mm:schemaBody`.
pub const MM_SCHEMA_BODY: &str = "https://metamind.dev/ontology#schemaBody";
/// `mm:owlImports`.
pub const MM_OWL_IMPORTS: &str = "https://metamind.dev/ontology#owlImports";

/// The graphs that hold managed instance data.
pub const MANAGED_GRAPHS: &[&str] = &[
    "being",
    "users/index",
    "plan",
    "action",
    "session",
    "log",
    "schema",
    "memory",
    "epistemic",
    "world",
];

/// A document to register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaSpec {
    /// Stable schema id, normally the prefix.
    pub id: String,
    /// Namespace prefix.
    pub prefix: String,
    /// Namespace IRI.
    pub namespace: String,
    /// Version string.
    pub version: String,
    /// Provenance.
    pub source: String,
    /// The document body; parsed for classes, properties, imports.
    pub body: String,
    /// The document's serialization; `None` tries Turtle, then RDF/XML.
    pub format: Option<String>,
}

/// What a registration extracted from its document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedSchema {
    /// Class IRIs.
    pub classes: Vec<String>,
    /// Property IRIs.
    pub properties: Vec<String>,
    /// Shape IRIs.
    pub shapes: Vec<String>,
    /// `owl:imports` IRIs.
    pub imports: Vec<String>,
}

/// The catalog.
#[derive(Debug, Clone)]
pub struct SchemaCatalog {
    engine: Arc<RdfEngine>,
}

impl SchemaCatalog {
    /// Open a catalog over `engine`.
    pub fn new(engine: Arc<RdfEngine>) -> Self {
        Self { engine }
    }

    /// Insert built-in entries when they are absent. Idempotent.
    pub async fn bootstrap(&self) -> Result<(), DataError> {
        let existing = self.list().await?;
        let known: BTreeSet<String> = existing.iter().map(|s| s.id.clone()).collect();
        for (prefix, namespace) in BUILTINS {
            if known.contains(*prefix) {
                continue;
            }
            let subject = schema_subject(prefix);
            let quads = vec![
                RdfEngine::quad(SCHEMA_GRAPH, &subject, RDF_TYPE, iri(MM_SCHEMA)),
                RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_SCHEMA_ID, lit(*prefix)),
                RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_SCHEMA_PREFIX, lit(*prefix)),
                RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_SCHEMA_NAMESPACE, lit(*namespace)),
                RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_SCHEMA_VERSION, lit("1")),
                RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_SCHEMA_SOURCE, lit("builtin")),
                RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_SCHEMA_ACTIVE, lit("true")),
            ];
            self.engine.insert_all(quads).await?;
        }
        Ok(())
    }

    /// Every catalog entry, sorted by id.
    pub async fn list(&self) -> Result<Vec<SchemaSummary>, DataError> {
        let raw = self.raw_entries().await?;
        Ok(raw.into_values().map(|entry| entry.summary()).collect())
    }

    /// One entry, with the requested detail.
    pub async fn get(&self, id: &str, include: SchemaInclude) -> Result<SchemaDetail, DataError> {
        let raw = self.raw_entries().await?;
        let entry = raw
            .get(id)
            .ok_or_else(|| DataError::NotFound(format!("schema {id:?} is not registered")))?;
        let all = include == SchemaInclude::All;
        Ok(SchemaDetail {
            summary: entry.summary(),
            classes: if matches!(include, SchemaInclude::Classes | SchemaInclude::All) {
                entry.classes.clone()
            } else {
                Vec::new()
            },
            properties: if matches!(include, SchemaInclude::Properties | SchemaInclude::All) {
                entry.properties.clone()
            } else {
                Vec::new()
            },
            shapes: if matches!(include, SchemaInclude::Shapes | SchemaInclude::All) {
                entry.shapes.clone()
            } else {
                Vec::new()
            },
            imports: if matches!(include, SchemaInclude::Imports | SchemaInclude::All) {
                entry.imports.clone()
            } else {
                Vec::new()
            },
            body: if all { entry.body.clone() } else { None },
        })
    }

    /// Register a schema. It is recorded inactive until [`Self::activate`].
    pub async fn register(&self, spec: SchemaSpec) -> Result<SchemaSummary, DataError> {
        let binding = super::NamespaceBinding {
            prefix: spec.prefix.clone(),
            namespace: spec.namespace.clone(),
            builtin: false,
        };
        binding.validate()?;
        if BUILTINS.iter().any(|(prefix, _)| *prefix == spec.prefix) {
            return Err(DataError::NamespaceConflict(format!(
                "prefix {:?} is built in and immutable",
                spec.prefix
            )));
        }
        let mut raw = self.raw_entries().await?;
        if let Some(existing) = raw.get(&spec.id) {
            if existing.namespace.as_deref() != Some(spec.namespace.as_str())
                || existing.prefix.as_deref() != Some(spec.prefix.as_str())
            {
                return Err(DataError::NamespaceConflict(format!(
                    "schema {:?} is already registered with a different namespace",
                    spec.id
                )));
            }
        }
        let parsed = parse_schema_document(&spec.body, spec.format.as_deref())?;
        let etag = format!("{:x}", Sha256::digest(spec.body.as_bytes()));
        let subject = schema_subject(&spec.prefix);
        // Replace the entry atomically: delete the previous quads, then write.
        let old = self
            .engine
            .store()
            .quads_for_subject(SCHEMA_GRAPH, &subject)
            .await?;
        let mut quads = vec![
            RdfEngine::quad(SCHEMA_GRAPH, &subject, RDF_TYPE, iri(MM_SCHEMA)),
            RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_SCHEMA_ID, lit(spec.id.clone())),
            RdfEngine::quad(
                SCHEMA_GRAPH,
                &subject,
                MM_SCHEMA_PREFIX,
                lit(spec.prefix.clone()),
            ),
            RdfEngine::quad(
                SCHEMA_GRAPH,
                &subject,
                MM_SCHEMA_NAMESPACE,
                lit(spec.namespace.clone()),
            ),
            RdfEngine::quad(
                SCHEMA_GRAPH,
                &subject,
                MM_SCHEMA_VERSION,
                lit(spec.version.clone()),
            ),
            RdfEngine::quad(
                SCHEMA_GRAPH,
                &subject,
                MM_SCHEMA_SOURCE,
                lit(spec.source.clone()),
            ),
            RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_SCHEMA_ACTIVE, lit("false")),
            RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_SCHEMA_ETAG, lit(etag)),
            RdfEngine::quad(
                SCHEMA_GRAPH,
                &subject,
                MM_SCHEMA_LOADED_AT,
                lit(now_ms().to_string()),
            ),
            RdfEngine::quad(
                SCHEMA_GRAPH,
                &subject,
                MM_SCHEMA_BODY,
                lit(cap_body(&spec.body)),
            ),
        ];
        for class in &parsed.classes {
            quads.push(RdfEngine::quad(
                SCHEMA_GRAPH,
                &subject,
                MM_SCHEMA_CLASS,
                iri(class.clone()),
            ));
        }
        for property in &parsed.properties {
            quads.push(RdfEngine::quad(
                SCHEMA_GRAPH,
                &subject,
                MM_SCHEMA_PROPERTY,
                iri(property.clone()),
            ));
        }
        for shape in &parsed.shapes {
            quads.push(RdfEngine::quad(
                SCHEMA_GRAPH,
                &subject,
                MM_SCHEMA_SHAPE,
                iri(shape.clone()),
            ));
        }
        for import in &parsed.imports {
            quads.push(RdfEngine::quad(
                SCHEMA_GRAPH,
                &subject,
                MM_OWL_IMPORTS,
                iri(import.clone()),
            ));
        }
        self.engine
            .store()
            .handle()
            .apply_batch(mm_store_graph::GraphBatch {
                deletes: old,
                inserts: quads,
            })
            .await?;
        raw.clear();
        let entries = self.raw_entries().await?;
        entries
            .get(&spec.id)
            .map(Entry::summary)
            .ok_or_else(|| DataError::Unavailable("schema entry did not persist".into()))
    }

    /// Activate a registered schema.
    pub async fn activate(&self, id: &str) -> Result<SchemaSummary, DataError> {
        self.set_active(id, true).await
    }

    /// Deprecate a schema, refusing while instance data uses its classes.
    pub async fn deprecate(&self, id: &str) -> Result<SchemaSummary, DataError> {
        let entries = self.raw_entries().await?;
        let entry = entries
            .get(id)
            .ok_or_else(|| DataError::NotFound(format!("schema {id:?} is not registered")))?;
        let in_use = self.count_instances(&entry.classes).await?;
        if in_use > 0 {
            return Err(DataError::SchemaViolation(format!(
                "schema {id:?} is used by {in_use} instance(s); migrate before deprecating"
            )));
        }
        self.set_active(id, false).await
    }

    /// Validate an entity, class, or graph against the data shapes.
    pub async fn validate(
        &self,
        target: ValidationTarget,
    ) -> Result<Vec<ShaclViolation>, DataError> {
        let graph = match &target {
            ValidationTarget::Graph(graph) => graph.clone(),
            ValidationTarget::Subject(subject) => self
                .graph_of(subject)
                .await?
                .ok_or_else(|| DataError::NotFound(format!("entity {subject:?}")))?,
            ValidationTarget::Class(class) => {
                let class = self.expand_class(class)?;
                if self.count_instances(std::slice::from_ref(&class)).await? == 0 {
                    return Ok(Vec::new());
                }
                self.graph_with_class(&class)
                    .await?
                    .unwrap_or_else(|| crate::graphs::store_graph("world"))
            }
        };
        let shapes = self
            .engine
            .store()
            .shapes_path()
            .parent()
            .map(|dir| dir.join("shapes").join("data.shacl.ttl"))
            .unwrap_or_else(|| std::path::PathBuf::from("data.shacl.ttl"));
        let report = self
            .engine
            .store()
            .validate_with(&graph, &shapes)
            .await
            .map_err(|e| DataError::Unavailable(e.to_string()))?;
        let focus = match &target {
            ValidationTarget::Graph(graph) => graph.clone(),
            ValidationTarget::Subject(subject) => subject.clone(),
            ValidationTarget::Class(class) => class.clone(),
        };
        Ok(report
            .violations
            .into_iter()
            .map(|violation| ShaclViolation {
                focus: focus.clone(),
                path: Some(violation.path),
                severity: violation.severity,
                message: violation.message,
            })
            .collect())
    }

    /// Expand a class through the registered namespaces.
    pub fn expand_class(&self, class: &str) -> Result<String, DataError> {
        if class.starts_with("http") {
            return Ok(class.to_string());
        }
        let Some((prefix, local)) = class.split_once(':') else {
            return Err(DataError::SchemaViolation(format!(
                "unknown class {class:?}"
            )));
        };
        let namespace = BUILTINS
            .iter()
            .find(|(name, _)| *name == prefix)
            .map(|(_, namespace)| (*namespace).to_string())
            .ok_or_else(|| {
                DataError::SchemaViolation(format!(
                    "unknown namespace prefix {prefix:?}; register it with data.namespace"
                ))
            })?;
        Ok(format!("{namespace}{local}"))
    }

    async fn set_active(&self, id: &str, active: bool) -> Result<SchemaSummary, DataError> {
        let entries = self.raw_entries().await?;
        let entry = entries
            .get(id)
            .ok_or_else(|| DataError::NotFound(format!("schema {id:?} is not registered")))?;
        if entry.source.as_deref() == Some("builtin") && !active {
            return Err(DataError::SchemaViolation(
                "built-in schemas cannot be deprecated".into(),
            ));
        }
        let subject = schema_subject(entry.prefix.as_deref().unwrap_or_default());
        let old = self
            .engine
            .store()
            .quads_for_subject(SCHEMA_GRAPH, &subject)
            .await?;
        let mut inserts = old
            .iter()
            .filter(|quad| quad.predicate.as_str() != MM_SCHEMA_ACTIVE)
            .cloned()
            .collect::<Vec<_>>();
        inserts.push(RdfEngine::quad(
            SCHEMA_GRAPH,
            &subject,
            MM_SCHEMA_ACTIVE,
            lit(if active { "true" } else { "false" }),
        ));
        self.engine
            .store()
            .handle()
            .apply_batch(mm_store_graph::GraphBatch {
                deletes: old,
                inserts,
            })
            .await?;
        Ok(SchemaSummary {
            active,
            ..entry.summary()
        })
    }

    async fn count_instances(&self, classes: &[String]) -> Result<usize, DataError> {
        if classes.is_empty() {
            return Ok(0);
        }
        let mut count = 0usize;
        for graph in MANAGED_GRAPHS {
            let triples = self.engine.triples(graph).await?;
            for row in triples {
                if row.get("p").and_then(crate::rdf::term_value) != Some(RDF_TYPE) {
                    continue;
                }
                if let Some(object) = row.get("o").and_then(crate::rdf::term_value) {
                    if classes.iter().any(|class| class == object) {
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }

    async fn graph_with_class(&self, class: &str) -> Result<Option<String>, DataError> {
        for graph in MANAGED_GRAPHS {
            let triples = self.engine.triples(graph).await?;
            let found = triples.iter().any(|row| {
                row.get("p").and_then(crate::rdf::term_value) == Some(RDF_TYPE)
                    && row.get("o").and_then(crate::rdf::term_value) == Some(class)
            });
            if found {
                return Ok(Some(crate::graphs::store_graph(graph)));
            }
        }
        Ok(None)
    }

    async fn graph_of(&self, subject: &str) -> Result<Option<String>, DataError> {
        for graph in MANAGED_GRAPHS {
            if !self
                .engine
                .store()
                .quads_for_subject(&crate::graphs::store_graph(graph), subject)
                .await?
                .is_empty()
            {
                return Ok(Some(crate::graphs::store_graph(graph)));
            }
        }
        Ok(None)
    }

    async fn raw_entries(&self) -> Result<BTreeMap<String, Entry>, DataError> {
        let triples = self.engine.triples(SCHEMA_GRAPH).await?;
        let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
        for row in triples {
            let Some(subject) = row.get("s").and_then(crate::rdf::term_value) else {
                continue;
            };
            let Some(predicate) = row.get("p").and_then(crate::rdf::term_value) else {
                continue;
            };
            let Some(object) = row.get("o").and_then(crate::rdf::term_value) else {
                continue;
            };
            if predicate == super::namespaces::MM_NAMESPACE_PREFIX {
                continue;
            }
            let entry = entries.entry(subject.to_string()).or_default();
            match predicate {
                MM_SCHEMA_ID => entry.id = Some(object.to_string()),
                MM_SCHEMA_PREFIX => entry.prefix = Some(object.to_string()),
                MM_SCHEMA_NAMESPACE => entry.namespace = Some(object.to_string()),
                MM_SCHEMA_VERSION => entry.version = Some(object.to_string()),
                MM_SCHEMA_SOURCE => entry.source = Some(object.to_string()),
                MM_SCHEMA_ACTIVE => entry.active = object == "true",
                MM_SCHEMA_LOADED_AT => entry.loaded_at = object.parse().ok(),
                MM_SCHEMA_CLASS => entry.classes.push(object.to_string()),
                MM_SCHEMA_PROPERTY => entry.properties.push(object.to_string()),
                MM_SCHEMA_SHAPE => entry.shapes.push(object.to_string()),
                MM_OWL_IMPORTS => entry.imports.push(object.to_string()),
                MM_SCHEMA_BODY => entry.body = Some(object.to_string()),
                _ => {}
            }
        }
        let mut out = BTreeMap::new();
        for (_, mut entry) in entries {
            let (Some(id), Some(prefix), Some(namespace)) = (
                entry.id.clone(),
                entry.prefix.clone(),
                entry.namespace.clone(),
            ) else {
                continue;
            };
            let _ = (id, prefix, namespace);
            entry.classes.sort();
            entry.classes.dedup();
            entry.properties.sort();
            entry.properties.dedup();
            entry.shapes.sort();
            entry.shapes.dedup();
            entry.imports.sort();
            entry.imports.dedup();
            out.insert(entry.summary().id, entry);
        }
        Ok(out)
    }
}

/// The subject IRI of one catalog entry.
pub fn schema_subject(prefix: &str) -> String {
    format!("{MMD_NS}schema/{prefix}")
}

/// Parse a schema body, extracting classes, properties, shapes, and imports.
pub fn parse_schema_document(body: &str, format: Option<&str>) -> Result<ParsedSchema, DataError> {
    let formats: Vec<RdfFormat> = match format.map(str::to_ascii_lowercase).as_deref() {
        Some("turtle") | Some("ttl") => vec![RdfFormat::Turtle],
        Some("rdfxml") | Some("rdf/xml") | Some("xml") => vec![RdfFormat::RdfXml],
        _ => vec![RdfFormat::Turtle, RdfFormat::RdfXml],
    };
    let mut last_error = None;
    for format in formats {
        let mut parsed = ParsedSchema::default();
        let mut subjects: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
        let mut ok = true;
        for quad in RdfParser::from_format(format).for_slice(body.as_bytes()) {
            let Ok(quad) = quad else {
                ok = false;
                last_error = Some("schema parse error".to_string());
                break;
            };
            let subject = super::rdf_mapper::subject_str(&quad.subject);
            if quad.predicate.as_str() == RDF_TYPE {
                let local = super::rdf_mapper::term_iri(&quad.object)
                    .map(local_name)
                    .unwrap_or_default();
                let kinds = subjects.entry(subject.clone()).or_default();
                match local {
                    "Class" => {
                        kinds.insert("class");
                        parsed.classes.push(subject);
                    }
                    "ObjectProperty" | "DatatypeProperty" | "AnnotationProperty" | "Property" => {
                        kinds.insert("property");
                        parsed.properties.push(subject);
                    }
                    "NodeShape" | "PropertyShape" => {
                        kinds.insert("shape");
                        parsed.shapes.push(subject);
                    }
                    _ => {}
                }
            }
            if quad.predicate.as_str() == format!("{OWL_NS}imports") {
                if let Some(import) = super::rdf_mapper::term_iri(&quad.object) {
                    parsed.imports.push(import.to_string());
                }
            }
        }
        if ok {
            parsed.classes.sort();
            parsed.classes.dedup();
            parsed.properties.sort();
            parsed.properties.dedup();
            parsed.shapes.sort();
            parsed.shapes.dedup();
            parsed.imports.sort();
            parsed.imports.dedup();
            return Ok(parsed);
        }
    }
    Err(DataError::SchemaViolation(last_error.unwrap_or_else(
        || "schema body is not Turtle or RDF/XML".into(),
    )))
}

fn cap_body(body: &str) -> String {
    const MAX: usize = 64 * 1024;
    body.chars().take(MAX).collect()
}

#[derive(Debug, Default, Clone)]
struct Entry {
    id: Option<String>,
    prefix: Option<String>,
    namespace: Option<String>,
    version: Option<String>,
    source: Option<String>,
    active: bool,
    loaded_at: Option<i64>,
    classes: Vec<String>,
    properties: Vec<String>,
    shapes: Vec<String>,
    imports: Vec<String>,
    body: Option<String>,
}

impl Entry {
    fn summary(&self) -> SchemaSummary {
        SchemaSummary {
            id: self.id.clone().unwrap_or_default(),
            prefix: self.prefix.clone().unwrap_or_default(),
            namespace: self.namespace.clone().unwrap_or_default(),
            version: self.version.clone().unwrap_or_else(|| "1".into()),
            source: self.source.clone().unwrap_or_else(|| "unknown".into()),
            active: self.active,
            class_count: self.classes.len(),
            property_count: self.properties.len(),
            loaded_at: self.loaded_at,
        }
    }
}
