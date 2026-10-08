//! `mm-store-graph` — the RDF half of kernel state.
//!
//! Oxigraph owns every graph projection. Writes go through a single writer
//! thread (Oxigraph's `Store` is synchronous); reads take a thread-safe handle.
//! Canonicalization and SHACL validation come from the vendored `rdf-codec` and
//! `rdf-shacl` crates, so "the same triples" and "conforms to the shapes" have
//! exactly one implementation in the system.
#![forbid(unsafe_code)]

pub mod actor;
pub mod graphs;
pub mod shacl;

pub use actor::{graph_iri, map_shacl_report, GraphHandle, GraphStore, MAILBOX_CAPACITY};

/// The Oxigraph release this binary actually links against.
///
/// Resolved from `Cargo.lock` at build time (`build.rs`), so `doctor` cannot
/// report a version that has drifted from the dependency it was built with.
pub const OXIGRAPH_VERSION: &str = env!("MM_OXIGRAPH_VERSION");
pub use graphs::{
    canonical_hash_of_turtle, canonical_ntriples_of_turtle, normalize_turtle, synthetic_turtle,
    term_ref_to_json, term_to_json,
};
pub use shacl::{load_shapes, validate_turtle_text};
