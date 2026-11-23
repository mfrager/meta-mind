//! The typed RDF mapper.
//!
//! A [`DataEntity`] is a subject plus `rdf:type`, its typed properties, and a
//! small fixed set of system predicates. Nothing here invents a predicate: the
//! broker owns every system field and the model supplies only the property set.

use std::collections::BTreeMap;

use mm_search::SearchDocument;
use oxrdf::{Literal, NamedNode, Quad, Term};

use crate::engine::RdfEngine;
use crate::rdf::{iri, lit, typed_lit, MMD_NS, MM_NS, RDF_TYPE, XSD_NS};

use super::{DataEntity, DataError, DataValue, EntityStatus};

/// `mm:revision`.
pub const MM_REVISION: &str = "https://metamind.dev/ontology#revision";
/// `mm:status`.
pub const MM_STATUS: &str = "https://metamind.dev/ontology#status";
/// `mm:createdAt`.
pub const MM_CREATED_AT: &str = "https://metamind.dev/ontology#createdAt";
/// `mm:updatedAt`.
pub const MM_UPDATED_AT: &str = "https://metamind.dev/ontology#updatedAt";
/// `mm:deletedAt`.
pub const MM_DELETED_AT: &str = "https://metamind.dev/ontology#deletedAt";
/// `mm:deletedBy`.
pub const MM_DELETED_BY: &str = "https://metamind.dev/ontology#deletedBy";
/// `mm:recordedBy`.
pub const MM_RECORDED_BY: &str = "https://metamind.dev/ontology#recordedBy";
/// `mm:sourceGraph`.
pub const MM_SOURCE_GRAPH: &str = "https://metamind.dev/ontology#sourceGraph";
/// `mm:searchable`.
pub const MM_SEARCHABLE: &str = "https://metamind.dev/ontology#searchable";
/// `mm:confidence`.
pub const MM_CONFIDENCE: &str = "https://metamind.dev/ontology#confidence";
/// `mm:supersedes`.
pub const MM_SUPERSEDES: &str = "https://metamind.dev/ontology#supersedes";
/// `mm:name`.
pub const MM_NAME: &str = "https://metamind.dev/ontology#name";
/// `rdfs:label`.
pub const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
/// `foaf:name`.
pub const FOAF_NAME: &str = "http://xmlns.com/foaf/0.1/name";
/// `schema:name`.
pub const SCHEMA_NAME: &str = "https://schema.org/name";

/// The predicates the broker owns and no model write may set.
pub const SYSTEM_PREDICATES: &[&str] = &[
    RDF_TYPE,
    MM_REVISION,
    MM_STATUS,
    MM_CREATED_AT,
    MM_UPDATED_AT,
    MM_DELETED_AT,
    MM_DELETED_BY,
    MM_RECORDED_BY,
    MM_SOURCE_GRAPH,
    MM_SEARCHABLE,
];

/// Render an entity as quads in its authoritative graph.
pub fn entity_quads(entity: &DataEntity) -> Vec<Quad> {
    let mut quads = Vec::new();
    for class in &entity.types {
        quads.push(RdfEngine::quad(
            &entity.graph,
            &entity.iri,
            RDF_TYPE,
            iri(class.clone()),
        ));
    }
    if let Some(label) = &entity.label {
        quads.push(RdfEngine::quad(
            &entity.graph,
            &entity.iri,
            RDFS_LABEL,
            lit(label.clone()),
        ));
    }
    for (predicate, values) in &entity.properties {
        for value in values {
            let object = match value {
                DataValue::Iri { iri } => crate::rdf::iri(iri.clone()),
                DataValue::Literal {
                    value,
                    datatype,
                    language,
                } => match (datatype, language) {
                    (_, Some(language)) => Term::Literal(
                        Literal::new_language_tagged_literal(value.clone(), language.clone())
                            .unwrap_or_else(|_| Literal::new_simple_literal(value.clone())),
                    ),
                    (Some(datatype), None) => typed_lit(value.clone(), datatype),
                    (None, None) => lit(value.clone()),
                },
            };
            quads.push(RdfEngine::quad(
                &entity.graph,
                &entity.iri,
                predicate,
                object,
            ));
        }
    }
    quads.push(RdfEngine::quad(
        &entity.graph,
        &entity.iri,
        MM_REVISION,
        typed_lit(
            entity.revision.to_string(),
            &format!("{XSD_NS}unsignedLong"),
        ),
    ));
    quads.push(RdfEngine::quad(
        &entity.graph,
        &entity.iri,
        MM_STATUS,
        lit(status_name(entity.status)),
    ));
    quads.push(RdfEngine::quad(
        &entity.graph,
        &entity.iri,
        MM_CREATED_AT,
        typed_lit(entity.created_at.to_string(), &format!("{XSD_NS}dateTime")),
    ));
    quads.push(RdfEngine::quad(
        &entity.graph,
        &entity.iri,
        MM_UPDATED_AT,
        typed_lit(entity.updated_at.to_string(), &format!("{XSD_NS}dateTime")),
    ));
    quads.push(RdfEngine::quad(
        &entity.graph,
        &entity.iri,
        MM_RECORDED_BY,
        lit(entity.recorded_by.clone()),
    ));
    if entity.status == EntityStatus::Deleted {
        quads.push(RdfEngine::quad(
            &entity.graph,
            &entity.iri,
            MM_DELETED_AT,
            typed_lit(entity.updated_at.to_string(), &format!("{XSD_NS}dateTime")),
        ));
        quads.push(RdfEngine::quad(
            &entity.graph,
            &entity.iri,
            MM_DELETED_BY,
            lit(entity.recorded_by.clone()),
        ));
    }
    quads.push(RdfEngine::quad(
        &entity.graph,
        &entity.iri,
        MM_SOURCE_GRAPH,
        lit(entity.graph.clone()),
    ));
    quads
}

/// Rebuild an entity from its quads.
pub fn entity_from_quads(subject: &str, graph: &str, quads: &[Quad]) -> Option<DataEntity> {
    let mut types = Vec::new();
    let mut label = None;
    let mut properties: BTreeMap<String, Vec<DataValue>> = BTreeMap::new();
    let mut revision = None;
    let mut status = None;
    let mut created_at = None;
    let mut updated_at = None;
    let mut recorded_by = None;

    for quad in quads {
        let predicate = quad.predicate.as_str().to_string();
        let object = &quad.object;
        match predicate.as_str() {
            RDF_TYPE => {
                if let Term::NamedNode(node) = object {
                    types.push(node.as_str().to_string());
                }
            }
            MM_REVISION => revision = lexical(object).and_then(|value| parse_u64(&value)),
            MM_STATUS => status = lexical(object),
            MM_CREATED_AT => created_at = lexical(object).and_then(|value| parse_i64(&value)),
            MM_UPDATED_AT => updated_at = lexical(object).and_then(|value| parse_i64(&value)),
            MM_RECORDED_BY => recorded_by = lexical(object),
            MM_SOURCE_GRAPH | MM_SEARCHABLE | MM_DELETED_AT | MM_DELETED_BY => {}
            RDFS_LABEL | FOAF_NAME | SCHEMA_NAME | MM_NAME => {
                if label.is_none() {
                    label = lexical(object);
                }
                properties
                    .entry(predicate)
                    .or_default()
                    .push(value_of(object));
            }
            _ => properties
                .entry(predicate)
                .or_default()
                .push(value_of(object)),
        }
    }

    let revision = revision?;
    if types.is_empty() {
        return None;
    }
    types.sort();
    types.dedup();
    Some(DataEntity {
        iri: subject.to_string(),
        types,
        label,
        properties,
        graph: graph.to_string(),
        revision,
        status: status_from_name(status.as_deref().unwrap_or("active")),
        created_at: created_at.unwrap_or(0),
        updated_at: updated_at.unwrap_or(0),
        recorded_by: recorded_by.unwrap_or_else(|| "system".to_string()),
    })
}

/// The label priority from the design.
pub fn label_of(entity: &DataEntity) -> String {
    entity.label.clone().unwrap_or_default()
}

/// Class and property labels resolved through the ontology graph.
pub async fn vocabulary_labels(engine: &RdfEngine) -> Result<BTreeMap<String, String>, DataError> {
    let mut out = BTreeMap::new();
    for row in engine.triples("ontology").await? {
        let Some(subject) = row.get("s").and_then(crate::rdf::term_value) else {
            continue;
        };
        let Some(predicate) = row.get("p").and_then(crate::rdf::term_value) else {
            continue;
        };
        if predicate != RDFS_LABEL {
            continue;
        }
        if let Some(text) = row.get("o").and_then(crate::rdf::term_value) {
            out.insert(subject.to_string(), text.to_string());
        }
    }
    Ok(out)
}

/// Build the index document for one entity.
///
/// The body is every literal plus class and property labels, truncated at
/// 32 KiB on a character boundary. Fields marked `mm:searchable false` and
/// secret-looking predicates never enter the index.
pub fn search_document(
    entity: &DataEntity,
    vocabulary: &BTreeMap<String, String>,
    namespace: &str,
) -> SearchDocument {
    let mut body = String::new();
    for class in &entity.types {
        if let Some(label) = vocabulary.get(class) {
            push_limited(&mut body, label);
        }
    }
    for (predicate, values) in &entity.properties {
        if predicate_matches(predicate, "secret")
            || predicate_matches(predicate, "password")
            || predicate_matches(predicate, "token")
        {
            continue;
        }
        if let Some(label) = vocabulary.get(predicate) {
            push_limited(&mut body, label);
        }
        for value in values {
            if let DataValue::Literal { value, .. } = value {
                push_limited(&mut body, value);
            }
        }
    }
    SearchDocument {
        subject: entity.iri.clone(),
        graph: entity.graph.clone(),
        types: entity.types.clone(),
        label: label_of(entity),
        body,
        namespace: namespace.to_string(),
        revision: entity.revision,
        updated_at: entity.updated_at,
        status: status_name(entity.status).to_string(),
    }
}

/// A subject as a plain IRI string (blank nodes keep the `_:` prefix).
pub fn subject_str(subject: &oxrdf::NamedOrBlankNode) -> String {
    match subject {
        oxrdf::NamedOrBlankNode::NamedNode(node) => node.as_str().to_string(),
        oxrdf::NamedOrBlankNode::BlankNode(node) => format!("_:{node}"),
    }
}

/// An object IRI, when the term is one.
pub fn term_iri(term: &Term) -> Option<&str> {
    match term {
        Term::NamedNode(node) => Some(node.as_str()),
        _ => None,
    }
}

/// The local name of an IRI.
pub fn local_name(iri: &str) -> &str {
    iri.rsplit(['#', '/']).next().unwrap_or(iri)
}

/// The canonical graph alias for a class.
pub fn graph_for_class(class: &str) -> &'static str {
    let local = if class.starts_with("http") {
        local_name(class)
    } else {
        class.rsplit(':').next().unwrap_or(class)
    };
    match local {
        "Being" | "Identity" => "being",
        "Person" | "User" => "users/index",
        "Plan" | "PlanVersion" | "PlanStep" | "PlanRegistry" | "Thought" => "plan",
        "ProcessTemplate" | "ProcessTemplateVersion" | "ProcessInstance" | "StepExecution" => {
            "process"
        }
        "ScheduledAction" | "CronJob" | "ActionReceipt" => "action",
        "Session" | "SessionTrace" | "WorkItem" | "ConversationFrame" | "FrameUtterance" => {
            "session"
        }
        "LogEntry" => "log",
        "Schema" | "Namespace" => "schema",
        "Claim" | "Evidence" | "Observation" => "epistemic",
        _ if class.starts_with(&format!("{MM_NS}Memory")) => "memory",
        _ => "world",
    }
}

/// Whether a class is declared by a known namespace.
pub fn known_class(class: &str) -> bool {
    const NAMESPACES: &[&str] = &[
        MM_NS,
        MMD_NS,
        "http://xmlns.com/foaf/0.1/",
        "https://schema.org/",
        "http://purl.org/dc/terms/",
        "http://usefulinc.com/ns/doap#",
        "http://www.w3.org/ns/prov#",
        "http://www.w3.org/2004/02/skos/core#",
        "http://www.w3.org/2002/07/owl#",
        "http://www.w3.org/2000/01/rdf-schema#",
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
    ];
    NAMESPACES
        .iter()
        .any(|namespace| class.starts_with(namespace))
}

/// Validate a literal against its declared datatype, refusing lossy coercion.
pub fn validate_literal(datatype: Option<&str>, value: &str) -> Result<(), DataError> {
    let Some(datatype) = datatype else {
        return Ok(());
    };
    let local = local_name(datatype);
    let ok = match local {
        "integer" | "int" | "long" | "unsignedLong" | "unsignedInt" => {
            value.parse::<i128>().is_ok()
        }
        "double" | "float" | "decimal" => value.parse::<f64>().is_ok(),
        "boolean" => matches!(value, "true" | "false" | "1" | "0"),
        "dateTime" | "date" | "time" => value.contains('-') || value.contains(':'),
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(DataError::SchemaViolation(format!(
            "{value:?} is not a valid {local}"
        )))
    }
}

fn value_of(term: &Term) -> DataValue {
    match term {
        Term::NamedNode(node) => DataValue::Iri {
            iri: node.as_str().to_string(),
        },
        Term::Literal(literal) => {
            let datatype = literal.datatype().as_str();
            let is_plain = datatype == format!("{XSD_NS}string");
            DataValue::Literal {
                value: literal.value().to_string(),
                datatype: (!is_plain).then(|| datatype.to_string()),
                language: literal.language().map(ToString::to_string),
            }
        }
        Term::BlankNode(node) => DataValue::Literal {
            value: format!("_:{node}"),
            datatype: None,
            language: None,
        },
    }
}

fn lexical(term: &Term) -> Option<String> {
    match term {
        Term::Literal(literal) => Some(literal.value().to_string()),
        Term::NamedNode(node) => Some(node.as_str().to_string()),
        _ => None,
    }
}

fn parse_u64(value: &str) -> Option<u64> {
    value.parse().ok()
}

fn parse_i64(value: &str) -> Option<i64> {
    value.parse().ok()
}

fn status_name(status: EntityStatus) -> &'static str {
    match status {
        EntityStatus::Active => "active",
        EntityStatus::Deleted => "deleted",
        EntityStatus::Superseded => "superseded",
    }
}

fn status_from_name(name: &str) -> EntityStatus {
    match name {
        "deleted" => EntityStatus::Deleted,
        "superseded" => EntityStatus::Superseded,
        _ => EntityStatus::Active,
    }
}

fn predicate_matches(predicate: &str, needle: &str) -> bool {
    local_name(predicate).to_ascii_lowercase().contains(needle)
}

fn push_limited(body: &mut String, value: &str) {
    const MAX_BYTES: usize = 32 * 1024;
    if body.len() >= MAX_BYTES {
        return;
    }
    for ch in value.chars() {
        if body.len() + ch.len_utf8() > MAX_BYTES {
            break;
        }
        body.push(ch);
    }
    body.push(' ');
}

/// The canonical `mmd:` instance namespace.
pub const fn data_namespace() -> &'static str {
    MMD_NS
}

/// A named node helper for the service's IRI checks.
pub fn named_node(iri: &str) -> Option<NamedNode> {
    NamedNode::new(iri).ok()
}
