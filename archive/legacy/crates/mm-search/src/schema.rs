//! The Tantivy schema: one document per managed entity.
//!
//! Search is a derived read model. Every field here is either copied from RDF
//! (`subject`, `graph`, `types`, `namespace`, `revision`, `updated_at`,
//! `status`) or rendered from it (`label`, `body`). Losing the index loses no
//! authority; rebuilding it from RDF restores every document.

use tantivy::schema::{
    Field, IndexRecordOption, NumericOptions, Schema, TextFieldIndexing, TextOptions, STORED,
    STRING,
};

/// The index schema version. A mismatch makes the index rebuildable.
pub const SCHEMA_VERSION: u32 = 1;

/// Bumped whenever the folder layout changes incompatibly.
pub const INDEX_DIR: &str = "tantivy";

/// Field handles for one index.
#[derive(Debug, Clone)]
pub struct SearchFields {
    /// Entity IRI, the unique key.
    pub subject: Field,
    /// Authoritative graph.
    pub graph: Field,
    /// `rdf:type` IRIs (multi-valued).
    pub types: Field,
    /// Display label, boosted.
    pub label: Field,
    /// Every literal plus class/property labels, capped.
    pub body: Field,
    /// Namespace prefix.
    pub namespace: Field,
    /// Entity revision.
    pub revision: Field,
    /// Last update, Unix ms.
    pub updated_at: Field,
    /// `active`, `deleted`, or `superseded`.
    pub status: Field,
}

/// Build the schema and return it with its field handles.
pub fn build_schema() -> (Schema, SearchFields) {
    let mut builder = Schema::builder();
    let subject = builder.add_text_field("subject", STRING | STORED);
    let graph = builder.add_text_field("graph", STRING | STORED);
    let types = builder.add_text_field("types", STRING | STORED);
    let label = builder.add_text_field("label", text_field());
    let body = builder.add_text_field("body", text_field());
    let namespace = builder.add_text_field("namespace", STRING | STORED);
    let revision = builder.add_u64_field(
        "revision",
        NumericOptions::default().set_stored().set_fast(),
    );
    let updated_at = builder.add_i64_field(
        "updated_at",
        NumericOptions::default().set_stored().set_fast(),
    );
    let status = builder.add_text_field("status", STRING | STORED);
    let schema = builder.build();
    (
        schema,
        SearchFields {
            subject,
            graph,
            types,
            label,
            body,
            namespace,
            revision,
            updated_at,
            status,
        },
    )
}

/// English stemming with positions, so phrase and boost queries behave.
fn text_field() -> TextOptions {
    TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("en_stem")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    )
}
