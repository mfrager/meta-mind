//! Typed ontology-data contracts behind the seven `data.*` tools.
//!
//! The CM names entities, classes, properties, and filters. It never writes
//! quads, SPARQL, SQL, or index documents. [`DataBroker`] is the single service
//! seam that maps these high-level intents to RDF and lexical search in later
//! phases.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod namespaces;
pub mod outbox;
pub mod rdf_mapper;
pub mod schema;
pub mod service;

pub use namespaces::{NamespaceRegistry, BUILTINS, MINTED_KINDS};
pub use outbox::{DataOutbox, OutboxEntry};
pub use schema::{ParsedSchema, SchemaCatalog, SchemaSpec};
pub use service::{DataService, DataStatus, IndexVerification};

/// Maximum property filters per query.
pub const MAX_DATA_FILTERS: usize = 16;
/// Maximum values for one predicate.
pub const MAX_PREDICATE_VALUES: usize = 32;
/// Maximum query page.
pub const MAX_QUERY_LIMIT: usize = 100;
/// Maximum search page.
pub const MAX_SEARCH_LIMIT: usize = 50;

/// A typed RDF entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataEntity {
    /// Entity IRI.
    pub iri: String,
    /// `rdf:type` IRIs.
    pub types: Vec<String>,
    /// Optional display label.
    pub label: Option<String>,
    /// Property values, sorted by predicate.
    pub properties: BTreeMap<String, Vec<DataValue>>,
    /// Authoritative graph IRI.
    pub graph: String,
    /// Monotonic entity revision.
    pub revision: u64,
    /// Entity status.
    pub status: EntityStatus,
    /// Creation instant, Unix ms.
    pub created_at: i64,
    /// Update instant, Unix ms.
    pub updated_at: i64,
    /// `user`, `cm`, `agent`, or `system`.
    pub recorded_by: String,
}

impl DataEntity {
    /// Validate the broker-owned fields.
    pub fn validate(&self) -> Result<(), DataError> {
        if self.iri.trim().is_empty() {
            return Err(DataError::Invalid("entity iri must not be empty".into()));
        }
        if self.types.is_empty() {
            return Err(DataError::Invalid("entity must have a type".into()));
        }
        if self.revision == 0 {
            return Err(DataError::Invalid(
                "entity revision must be positive".into(),
            ));
        }
        for (predicate, values) in &self.properties {
            if predicate.trim().is_empty() {
                return Err(DataError::Invalid("predicate must not be empty".into()));
            }
            if values.len() > MAX_PREDICATE_VALUES {
                return Err(DataError::Invalid(format!(
                    "predicate {predicate} exceeds the value limit"
                )));
            }
        }
        Ok(())
    }

    /// Whether the entity is active.
    pub fn is_active(&self) -> bool {
        self.status == EntityStatus::Active
    }
}

/// Entity lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityStatus {
    /// Queryable and searchable.
    Active,
    /// Retained for provenance, excluded by default.
    Deleted,
    /// Replaced by another entity/value.
    Superseded,
}

/// One RDF value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DataValue {
    /// Object property.
    Iri {
        /// Target IRI.
        iri: String,
    },
    /// Literal value.
    Literal {
        /// Lexical value.
        value: String,
        /// Optional XSD datatype IRI.
        datatype: Option<String>,
        /// Optional BCP-47 language tag.
        language: Option<String>,
    },
}

impl DataValue {
    /// A plain string literal.
    pub fn text(value: impl Into<String>) -> Self {
        Self::Literal {
            value: value.into(),
            datatype: Some("http://www.w3.org/2001/XMLSchema#string".into()),
            language: None,
        }
    }

    /// An IRI reference.
    pub fn iri(iri: impl Into<String>) -> Self {
        Self::Iri { iri: iri.into() }
    }
}

/// A predicate patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataPatch {
    /// Predicate IRI or compact IRI.
    pub predicate: String,
    /// Replacement values; an empty list removes the predicate.
    pub values: Vec<DataValue>,
}

impl DataPatch {
    /// Validate one patch entry.
    pub fn validate(&self) -> Result<(), DataError> {
        if self.predicate.trim().is_empty() {
            return Err(DataError::Invalid(
                "patch predicate must not be empty".into(),
            ));
        }
        if self.values.len() > MAX_PREDICATE_VALUES {
            return Err(DataError::Invalid(format!(
                "patch {} exceeds the value limit",
                self.predicate
            )));
        }
        Ok(())
    }
}

/// One query filter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DataFilter {
    /// Exact literal.
    Equals {
        /// Predicate.
        predicate: String,
        /// Expected lexical value.
        value: String,
    },
    /// Exact object IRI.
    Iri {
        /// Predicate.
        predicate: String,
        /// Object IRI.
        value: String,
    },
    /// Literal substring.
    Contains {
        /// Predicate.
        predicate: String,
        /// Substring.
        value: String,
    },
}

/// A bounded, typed query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataQuery {
    /// Optional class, including subclasses.
    pub class: Option<String>,
    /// Optional exact subject.
    pub subject: Option<String>,
    /// Property filters.
    #[serde(default)]
    pub filters: Vec<DataFilter>,
    /// Optional graph.
    pub graph: Option<String>,
    /// Optional namespace prefix.
    pub namespace: Option<String>,
    /// Include soft-deleted rows.
    #[serde(default)]
    pub include_deleted: bool,
    /// Page size.
    pub limit: usize,
    /// Offset.
    pub offset: usize,
}

impl Default for DataQuery {
    fn default() -> Self {
        Self {
            class: None,
            subject: None,
            filters: Vec::new(),
            graph: None,
            namespace: None,
            include_deleted: false,
            limit: 20,
            offset: 0,
        }
    }
}

impl DataQuery {
    /// Validate paging and filter bounds.
    pub fn validate(&self) -> Result<(), DataError> {
        if self.limit == 0 || self.limit > MAX_QUERY_LIMIT {
            return Err(DataError::Invalid(format!(
                "limit must be 1–{MAX_QUERY_LIMIT}"
            )));
        }
        if self.filters.len() > MAX_DATA_FILTERS {
            return Err(DataError::Invalid("query exceeds the filter limit".into()));
        }
        for filter in &self.filters {
            match filter {
                DataFilter::Equals { predicate, .. }
                | DataFilter::Iri { predicate, .. }
                | DataFilter::Contains { predicate, .. } => {
                    if predicate.trim().is_empty() {
                        return Err(DataError::Invalid(
                            "filter predicate must not be empty".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// A query page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataQueryResult {
    /// Entity summaries.
    pub entities: Vec<DataEntity>,
    /// Whether another page exists.
    pub has_more: bool,
    /// Next offset, when present.
    pub next_offset: Option<usize>,
}

/// Create request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataCreateRequest {
    /// Class IRI or compact IRI.
    pub r#type: String,
    /// Optional stable id or full IRI.
    pub id: Option<String>,
    /// Typed initial properties.
    pub properties: Vec<DataPatch>,
    /// Optional schema id.
    pub schema: Option<String>,
    /// Optional graph override.
    pub graph: Option<String>,
    /// Idempotency key.
    pub idempotency_key: Option<String>,
}

/// Update mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataUpdateMode {
    /// Merge named predicates.
    Merge,
    /// Replace the writable property set.
    Replace,
}

/// Update request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataUpdateRequest {
    /// Entity IRI or compact IRI.
    pub subject: String,
    /// Property patch.
    pub properties: Vec<DataPatch>,
    /// Merge or replace.
    pub mode: DataUpdateMode,
    /// Optional optimistic revision.
    pub expected_revision: Option<u64>,
    /// Idempotency key.
    pub idempotency_key: Option<String>,
}

impl DataUpdateRequest {
    /// Validate the patch.
    pub fn validate(&self) -> Result<(), DataError> {
        if self.subject.trim().is_empty() {
            return Err(DataError::Invalid("subject must not be empty".into()));
        }
        for patch in &self.properties {
            patch.validate()?;
        }
        Ok(())
    }
}

/// Delete mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataDeleteMode {
    /// Mark deleted, retain RDF provenance.
    Soft,
    /// Remove quads and the search document.
    Hard,
}

/// Delete request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataDeleteRequest {
    /// Entity IRI.
    pub subject: String,
    /// Delete mode.
    pub mode: DataDeleteMode,
    /// Optional optimistic revision.
    pub expected_revision: Option<u64>,
    /// Reason.
    pub reason: Option<String>,
    /// Idempotency key.
    pub idempotency_key: Option<String>,
    /// Hard-delete authority (`user`, `operator`, or `system`).
    ///
    /// A hard delete without one is refused; soft delete never needs it.
    #[serde(default)]
    pub authority: Option<String>,
}

/// Entity mutation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataMutationResult {
    /// Full entity.
    pub entity: DataEntity,
    /// Whether the mutation was a no-op.
    pub unchanged: bool,
    /// Whether the create matched an existing active entity with the same
    /// label and type in the same graph.
    #[serde(default)]
    pub duplicate: bool,
    /// Search index health after the write.
    pub index: IndexHealth,
}

/// Search request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSearchRequest {
    /// Bounded lexical query.
    pub query: String,
    /// Optional class.
    pub class: Option<String>,
    /// Optional graph.
    pub graph: Option<String>,
    /// Optional namespace.
    pub namespace: Option<String>,
    /// Page size.
    pub limit: usize,
    /// Include snippets.
    pub snippet: bool,
    /// Include deleted rows.
    pub include_deleted: bool,
}

impl DataSearchRequest {
    /// Validate query and page.
    pub fn validate(&self) -> Result<(), DataError> {
        if self.query.trim().is_empty() || self.query.chars().count() > 500 {
            return Err(DataError::Invalid(
                "search query must be 1–500 characters".into(),
            ));
        }
        if self.limit == 0 || self.limit > MAX_SEARCH_LIMIT {
            return Err(DataError::Invalid(format!(
                "search limit must be 1–{MAX_SEARCH_LIMIT}"
            )));
        }
        Ok(())
    }
}

/// One verified search hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    /// Entity IRI.
    pub iri: String,
    /// Primary type.
    pub r#type: String,
    /// Label.
    pub label: Option<String>,
    /// Lexical score scaled to integer micros for deterministic JSON.
    pub score_micros: i64,
    /// Optional snippet.
    pub snippet: Option<String>,
    /// Graph.
    pub graph: String,
    /// Entity revision.
    pub revision: u64,
    /// Update instant.
    pub updated_at: i64,
}

/// Search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    /// Verified hits.
    pub hits: Vec<SearchHit>,
    /// Index health.
    pub index: IndexHealth,
}

/// Index health, surfaced without hiding partial failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexHealth {
    /// Searchable.
    Healthy,
    /// RDF committed; indexing is still pending.
    Pending,
    /// Index unavailable.
    Unavailable,
}

/// Compact schema catalog entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaSummary {
    /// Schema id.
    pub id: String,
    /// Compact prefix.
    pub prefix: String,
    /// Namespace IRI.
    pub namespace: String,
    /// Version.
    pub version: String,
    /// Source description.
    pub source: String,
    /// Whether the schema is active.
    pub active: bool,
    /// Class count.
    pub class_count: usize,
    /// Property count.
    pub property_count: usize,
    /// Last load instant.
    pub loaded_at: Option<i64>,
}

/// Optional schema body/detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaDetail {
    /// Catalog entry.
    pub summary: SchemaSummary,
    /// Class IRIs.
    #[serde(default)]
    pub classes: Vec<String>,
    /// Property IRIs.
    #[serde(default)]
    pub properties: Vec<String>,
    /// Shape IRIs.
    #[serde(default)]
    pub shapes: Vec<String>,
    /// OWL imports.
    #[serde(default)]
    pub imports: Vec<String>,
    /// Capped source body.
    pub body: Option<String>,
}

/// Schema include selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SchemaInclude {
    /// Summary only.
    #[default]
    Summary,
    /// Classes.
    Classes,
    /// Properties.
    Properties,
    /// Shapes.
    Shapes,
    /// Imports.
    Imports,
    /// Everything.
    All,
}

/// One SHACL violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaclViolation {
    /// Focus node.
    pub focus: String,
    /// Property path.
    pub path: Option<String>,
    /// Severity.
    pub severity: String,
    /// Human-readable message.
    pub message: String,
}

/// Validation target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationTarget {
    /// One entity.
    Subject(String),
    /// One class's instances.
    Class(String),
    /// One graph.
    Graph(String),
}

/// Compact namespace binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceBinding {
    /// Prefix.
    pub prefix: String,
    /// Namespace IRI.
    pub namespace: String,
    /// Whether the binding is built in and immutable.
    pub builtin: bool,
}

impl NamespaceBinding {
    /// Validate prefix and namespace shape.
    pub fn validate(&self) -> Result<(), DataError> {
        let mut chars = self.prefix.chars();
        let Some(first) = chars.next() else {
            return Err(DataError::Invalid("namespace prefix is empty".into()));
        };
        if !first.is_ascii_lowercase()
            || !chars
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
            || self.prefix.len() > 32
        {
            return Err(DataError::Invalid(format!(
                "invalid namespace prefix {:?}",
                self.prefix
            )));
        }
        if !(self.namespace.ends_with('/')
            || self.namespace.ends_with('#')
            || self.namespace.starts_with("urn:"))
        {
            return Err(DataError::Invalid(
                "namespace must end in / or #, or be a urn:".into(),
            ));
        }
        Ok(())
    }
}

/// The single high-level data seam.
#[async_trait]
pub trait DataBroker: Send + Sync {
    /// Create one ontology instance.
    async fn create(&self, request: DataCreateRequest) -> Result<DataMutationResult, DataError>;

    /// Update one existing instance.
    async fn update(&self, request: DataUpdateRequest) -> Result<DataMutationResult, DataError>;

    /// Query instances with typed filters.
    async fn query(&self, query: DataQuery) -> Result<DataQueryResult, DataError>;

    /// Soft- or hard-delete one instance.
    async fn delete(&self, request: DataDeleteRequest) -> Result<DataMutationResult, DataError>;

    /// Lexical search.
    async fn search(&self, request: DataSearchRequest) -> Result<SearchResult, DataError>;

    /// List the schema catalog.
    async fn list_schemas(&self) -> Result<Vec<SchemaSummary>, DataError>;

    /// Fetch one schema.
    async fn get_schema(&self, id: &str, include: SchemaInclude)
        -> Result<SchemaDetail, DataError>;

    /// Validate an entity/class/graph.
    async fn validate(&self, target: ValidationTarget) -> Result<Vec<ShaclViolation>, DataError>;

    /// List namespace bindings.
    async fn list_namespaces(&self) -> Result<Vec<NamespaceBinding>, DataError>;

    /// Expand a compact IRI.
    async fn expand(&self, compact: &str) -> Result<String, DataError>;

    /// Resolve a full IRI to compact form.
    async fn resolve(&self, iri: &str) -> Result<String, DataError>;
}

/// Data-contract failures.
#[derive(Debug, Error)]
pub enum DataError {
    /// Invalid arguments.
    #[error("invalid arguments: {0}")]
    Invalid(String),
    /// Entity missing.
    #[error("not found: {0}")]
    NotFound(String),
    /// Revision/identity conflict.
    #[error("data conflict: {0}")]
    Conflict(String),
    /// Schema/type violation.
    #[error("schema violation: {0}")]
    SchemaViolation(String),
    /// Namespace conflict.
    #[error("namespace conflict: {0}")]
    NamespaceConflict(String),
    /// Search unavailable.
    #[error("search unavailable: {0}")]
    SearchUnavailable(String),
    /// JSON serialization failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// SQLite failure.
    #[error("sqlite: {0}")]
    Sqlite(#[from] sqlx::Error),
    /// A lower data-layer failure (graph, IO, configuration).
    #[error(transparent)]
    Store(#[from] crate::error::DataError),
    /// RDF store failure.
    #[error("graph: {0}")]
    Graph(#[from] mm_core::MmError),
    /// Filesystem failure.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Search-index failure.
    #[error("search: {0}")]
    Search(#[from] mm_search::SearchError),
    /// RDF store failure.
    #[error("data store unavailable: {0}")]
    Unavailable(String),
}

/// Result alias for data operations.
pub type Result<T, E = DataError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_caps_and_filters_are_validated() {
        let mut query = DataQuery {
            limit: 101,
            ..DataQuery::default()
        };
        assert!(query.validate().is_err());
        query.limit = 20;
        query.filters.push(DataFilter::Contains {
            predicate: "foaf:name".into(),
            value: "ada".into(),
        });
        assert!(query.validate().is_ok());
    }

    #[test]
    fn namespace_binding_requires_a_terminal_separator_or_urn() {
        assert!(NamespaceBinding {
            prefix: "mm".into(),
            namespace: "https://metamind.dev/ontology#".into(),
            builtin: true,
        }
        .validate()
        .is_ok());
        assert!(NamespaceBinding {
            prefix: "mm".into(),
            namespace: "https://metamind.dev/ontology".into(),
            builtin: true,
        }
        .validate()
        .is_err());
    }

    #[test]
    fn search_limits_are_enforced() {
        let mut request = DataSearchRequest {
            query: "refresh".into(),
            class: None,
            graph: None,
            namespace: None,
            limit: 10,
            snippet: true,
            include_deleted: false,
        };
        assert!(request.validate().is_ok());
        request.limit = 0;
        assert!(request.validate().is_err());
    }
}
