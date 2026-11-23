//! `mm-search` — the derived lexical read model behind `data.search`.
//!
//! Tantivy is the only backend. The index is never authority: every document is
//! built from RDF, and `reindex` can rebuild the whole thing from the graph. The
//! model supplies a bounded text query and exact filters; it never supplies
//! query syntax.

#![forbid(unsafe_code)]

pub mod index;
pub mod query;
pub mod schema;

pub use index::SearchIndex;
pub use query::{build_query, sanitize_tokens};
pub use schema::{build_schema, SearchFields, INDEX_DIR, SCHEMA_VERSION};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum hits one search may request.
pub const MAX_SEARCH_LIMIT: usize = 50;

/// One entity as the index sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchDocument {
    /// Entity IRI, the unique key.
    pub subject: String,
    /// Authoritative graph IRI.
    pub graph: String,
    /// `rdf:type` IRIs.
    pub types: Vec<String>,
    /// Display label.
    pub label: String,
    /// Searchable literals plus class/property labels, capped at 32 KiB.
    pub body: String,
    /// Namespace prefix.
    pub namespace: String,
    /// Entity revision.
    pub revision: u64,
    /// Last update, Unix ms.
    pub updated_at: i64,
    /// `active`, `deleted`, or `superseded`.
    pub status: String,
}

/// A bounded search request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Free text; sanitized into plain tokens.
    pub text: String,
    /// Exact class filter.
    pub class: Option<String>,
    /// Exact graph filter.
    pub graph: Option<String>,
    /// Exact namespace filter.
    pub namespace: Option<String>,
    /// Page size, 1–[`MAX_SEARCH_LIMIT`].
    pub limit: usize,
    /// Include snippets.
    pub snippet: bool,
    /// Include soft-deleted entities.
    pub include_deleted: bool,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            class: None,
            graph: None,
            namespace: None,
            limit: 10,
            snippet: true,
            include_deleted: false,
        }
    }
}

/// One hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    /// Entity IRI.
    pub subject: String,
    /// Graph IRI.
    pub graph: String,
    /// Namespace prefix.
    pub namespace: String,
    /// Label, when the entity has one.
    pub label: Option<String>,
    /// `rdf:type` IRIs.
    pub types: Vec<String>,
    /// Entity revision.
    pub revision: u64,
    /// Last update, Unix ms.
    pub updated_at: i64,
    /// Lifecycle status.
    pub status: String,
    /// Score in integer micros, for deterministic JSON.
    pub score_micros: i64,
    /// Optional snippet.
    pub snippet: Option<String>,
}

/// One page of hits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPage {
    /// Hits, best first.
    pub hits: Vec<SearchHit>,
    /// Whether another page may exist.
    pub has_more: bool,
}

/// The result of a rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexReport {
    /// Documents written.
    pub documents: usize,
    /// Wall time.
    pub took_ms: u64,
}

/// Search failures.
#[derive(Debug, Error)]
pub enum SearchError {
    /// Invalid arguments.
    #[error("invalid search: {0}")]
    Invalid(String),
    /// The index is unavailable.
    #[error("search unavailable: {0}")]
    Unavailable(String),
    /// The index rejected an operation.
    #[error("search index error: {0}")]
    Index(String),
}

/// Search result alias.
pub type Result<T, E = SearchError> = std::result::Result<T, E>;
