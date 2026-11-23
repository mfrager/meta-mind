//! The namespace registry and canonical identity minting.
//!
//! Bindings live in the `schema` graph as `mm:Namespace` entries. Built-in
//! prefixes are immutable. New writes use the canonical `mmd:` forms; legacy
//! `urn:metamind:*` IRIs resolve to the canonical entity and record an
//! `owl:sameAs` alias once, so a person is never duplicated because two
//! spellings existed.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxrdf::NamedNode;

use crate::engine::RdfEngine;
use crate::rdf::{iri, lit, MMD_NS, MM_NS, OWL_NS, RDF_TYPE};

use super::{DataError, NamespaceBinding};

/// The graph that holds the catalog and namespace registry.
pub const SCHEMA_GRAPH: &str = "https://metamind.dev/graph/schema";

/// `mm:Namespace`.
pub const MM_NAMESPACE: &str = "https://metamind.dev/ontology#Namespace";
/// `mm:namespacePrefix`.
pub const MM_NAMESPACE_PREFIX: &str = "https://metamind.dev/ontology#namespacePrefix";
/// `mm:namespaceIri`.
pub const MM_NAMESPACE_IRI: &str = "https://metamind.dev/ontology#namespaceIri";
/// `mm:builtin`.
pub const MM_BUILTIN: &str = "https://metamind.dev/ontology#builtin";
/// `owl:sameAs`.
pub const OWL_SAME_AS: &str = "http://www.w3.org/2002/07/owl#sameAs";

/// The immutable built-in bindings from the design.
pub const BUILTINS: &[(&str, &str)] = &[
    ("mm", "https://metamind.dev/ontology#"),
    ("mmd", "https://metamind.dev/data/"),
    ("mmg", "https://metamind.dev/graph/"),
    ("foaf", "http://xmlns.com/foaf/0.1/"),
    ("schema", "https://schema.org/"),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("doap", "http://usefulinc.com/ns/doap#"),
    ("prov", "http://www.w3.org/ns/prov#"),
    ("skos", "http://www.w3.org/2004/02/skos/core#"),
    ("owl", "http://www.w3.org/2002/07/owl#"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
];

/// The kinds whose IRIs are minted as `mmd:<kind>/<ULID>`.
pub const MINTED_KINDS: &[&str] = &[
    "user",
    "project",
    "plan",
    "thought",
    "action",
    "log",
    "session",
    "agent",
    "schema",
    "namespace",
];

/// Namespace bindings and identity minting over the schema graph.
#[derive(Debug, Clone)]
pub struct NamespaceRegistry {
    engine: Arc<RdfEngine>,
}

impl NamespaceRegistry {
    /// Open a registry over `engine`.
    pub fn new(engine: Arc<RdfEngine>) -> Self {
        Self { engine }
    }

    /// Insert the built-in bindings when they are absent. Idempotent.
    pub async fn bootstrap(&self) -> Result<(), DataError> {
        let existing = self.list_raw().await?;
        for (prefix, namespace) in BUILTINS {
            if existing.contains_key(*prefix) {
                continue;
            }
            let subject = format!("{MMD_NS}namespace/{prefix}");
            let quads = vec![
                RdfEngine::quad(SCHEMA_GRAPH, &subject, RDF_TYPE, iri(MM_NAMESPACE)),
                RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_NAMESPACE_PREFIX, lit(*prefix)),
                RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_NAMESPACE_IRI, lit(*namespace)),
                RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_BUILTIN, lit("true")),
            ];
            self.engine.insert_all(quads).await?;
        }
        Ok(())
    }

    /// Every binding, built-ins first, sorted by prefix.
    pub async fn list(&self) -> Result<Vec<NamespaceBinding>, DataError> {
        let raw = self.list_raw().await?;
        Ok(raw
            .into_iter()
            .map(|(prefix, (namespace, builtin))| NamespaceBinding {
                prefix,
                namespace,
                builtin,
            })
            .collect())
    }

    /// Register a non-builtin binding.
    pub async fn register(&self, binding: NamespaceBinding) -> Result<NamespaceBinding, DataError> {
        binding.validate()?;
        if binding.builtin {
            return Err(DataError::NamespaceConflict(
                "built-in namespaces are immutable".into(),
            ));
        }
        let raw = self.list_raw().await?;
        if let Some((namespace, _)) = raw.get(&binding.prefix) {
            if namespace == &binding.namespace {
                return Ok(binding);
            }
            return Err(DataError::NamespaceConflict(format!(
                "prefix {:?} is already bound to {namespace}",
                binding.prefix
            )));
        }
        if raw
            .values()
            .any(|(namespace, _)| namespace == &binding.namespace)
        {
            return Err(DataError::NamespaceConflict(format!(
                "namespace {} already has a prefix",
                binding.namespace
            )));
        }
        let subject = format!("{MMD_NS}namespace/{}", binding.prefix);
        self.engine
            .insert_all(vec![
                RdfEngine::quad(SCHEMA_GRAPH, &subject, RDF_TYPE, iri(MM_NAMESPACE)),
                RdfEngine::quad(
                    SCHEMA_GRAPH,
                    &subject,
                    MM_NAMESPACE_PREFIX,
                    lit(binding.prefix.clone()),
                ),
                RdfEngine::quad(
                    SCHEMA_GRAPH,
                    &subject,
                    MM_NAMESPACE_IRI,
                    lit(binding.namespace.clone()),
                ),
                RdfEngine::quad(SCHEMA_GRAPH, &subject, MM_BUILTIN, lit("false")),
            ])
            .await?;
        Ok(binding)
    }

    /// Expand `prefix:local` to a full IRI.
    pub async fn expand(&self, compact: &str) -> Result<String, DataError> {
        let Some((prefix, local)) = compact.split_once(':') else {
            return if compact.starts_with("http") {
                Ok(compact.to_string())
            } else {
                Err(DataError::Invalid(format!(
                    "{compact:?} is not a compact IRI"
                )))
            };
        };
        if compact.starts_with("http") {
            return Ok(compact.to_string());
        }
        let raw = self.list_raw().await?;
        let Some((namespace, _)) = raw.get(prefix) else {
            return Err(DataError::NamespaceConflict(format!(
                "unknown namespace prefix {prefix:?}"
            )));
        };
        Ok(format!("{namespace}{local}"))
    }

    /// Compact a full IRI, or return it unchanged when no binding matches.
    pub async fn resolve(&self, iri: &str) -> Result<String, DataError> {
        let raw = self.list_raw().await?;
        for (prefix, (namespace, _)) in raw {
            if let Some(local) = iri.strip_prefix(&namespace) {
                if !local.is_empty() {
                    return Ok(format!("{prefix}:{local}"));
                }
            }
        }
        Ok(iri.to_string())
    }

    /// Mint a canonical entity IRI.
    ///
    /// An explicit full IRI is normalized (legacy aliases resolve); an explicit
    /// short id becomes `mmd:<kind>/<id>`; no id mints a fresh lowercase ULID.
    pub async fn mint(&self, kind: &str, id: Option<&str>) -> Result<String, DataError> {
        if !MINTED_KINDS.contains(&kind) {
            return Err(DataError::Invalid(format!(
                "{kind:?} is not a mintage kind"
            )));
        }
        match id.map(str::trim).filter(|id| !id.is_empty()) {
            None => Ok(format!("{MMD_NS}{kind}/{}", crate::rdf::new_id())),
            Some(id) if id.starts_with("urn:metamind:") => {
                let canonical = legacy_to_canonical(id)
                    .ok_or_else(|| DataError::Invalid(format!("unrecognized legacy IRI {id:?}")))?;
                self.ensure_alias(&canonical, id).await?;
                Ok(canonical)
            }
            Some(id) if id.starts_with("http") => {
                let _ = NamedNode::new(id)
                    .map_err(|e| DataError::Invalid(format!("invalid IRI {id:?}: {e}")))?;
                Ok(id.to_string())
            }
            Some(id) => {
                if !id
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
                {
                    return Err(DataError::Invalid(format!(
                        "id {id:?} must be alphanumeric, '-' or '_'"
                    )));
                }
                Ok(format!("{MMD_NS}{kind}/{id}"))
            }
        }
    }

    /// Resolve a legacy `urn:metamind:*` IRI to its canonical form, recording
    /// the alias when the two differ.
    pub async fn normalize(&self, value: &str) -> Result<String, DataError> {
        let Some(canonical) = legacy_to_canonical(value) else {
            return Ok(value.to_string());
        };
        self.ensure_alias(&canonical, value).await?;
        Ok(canonical)
    }

    /// Record `owl:sameAs` once, in the schema graph.
    pub async fn ensure_alias(&self, canonical: &str, alias: &str) -> Result<(), DataError> {
        if canonical == alias {
            return Ok(());
        }
        let predicate = format!("{OWL_NS}sameAs");
        let existing = self.engine.triples(SCHEMA_GRAPH).await?;
        let present = existing.iter().any(|row| {
            row.get("s").and_then(crate::rdf::term_value) == Some(canonical)
                && row.get("p").and_then(crate::rdf::term_value) == Some(predicate.as_str())
                && row.get("o").and_then(crate::rdf::term_value) == Some(alias)
        });
        if present {
            return Ok(());
        }
        self.engine
            .insert(RdfEngine::quad(
                SCHEMA_GRAPH,
                canonical,
                &predicate,
                iri(alias),
            ))
            .await?;
        Ok(())
    }

    async fn list_raw(&self) -> Result<BTreeMap<String, (String, bool)>, DataError> {
        let triples = self.engine.triples(SCHEMA_GRAPH).await?;
        let mut by_subject: BTreeMap<String, (Option<String>, Option<String>, bool)> =
            BTreeMap::new();
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
            let entry = by_subject.entry(subject.to_string()).or_default();
            match predicate {
                MM_NAMESPACE_PREFIX => entry.0 = Some(object.to_string()),
                MM_NAMESPACE_IRI => entry.1 = Some(object.to_string()),
                MM_BUILTIN => entry.2 = object == "true",
                _ => {}
            }
        }
        let mut out = BTreeMap::new();
        for (_, (prefix, namespace, builtin)) in by_subject {
            if let (Some(prefix), Some(namespace)) = (prefix, namespace) {
                out.insert(prefix, (namespace, builtin));
            }
        }
        Ok(out)
    }
}

/// Map a legacy `urn:metamind:<kind>:<id>` IRI to `mmd:<kind>/<id>`.
pub fn legacy_to_canonical(value: &str) -> Option<String> {
    let rest = value.strip_prefix("urn:metamind:")?;
    let (kind, id) = rest.split_once(':')?;
    if kind.is_empty() || id.is_empty() || !MINTED_KINDS.contains(&kind) {
        return None;
    }
    Some(format!("{MMD_NS}{kind}/{id}"))
}

/// A `mm:Namespace` subject IRI.
pub fn namespace_subject(prefix: &str) -> String {
    format!("{MMD_NS}namespace/{prefix}")
}

/// The `mm:` namespace, for callers that need it without importing `rdf`.
pub const fn mm_namespace() -> &'static str {
    MM_NS
}
