//! `math-core` — the RDF semantic layer: vocabulary, `MathModel` (Oxigraph
//! store wrapper), and canonical operand normalization.

pub mod model;
pub mod operands;
pub mod versioning;
pub mod vocabulary;

// Re-export oxigraph so downstream crates can name its `Term`/`NamedNode`
// types without adding the dependency (and its feature flags) again.
pub use oxigraph;

pub use model::{bare_iri, local_name, named_node_iri, MathModel};
pub use versioning::{diff_versions, snapshot_default_graph, triples_in_graph, VersionDiff};
pub use vocabulary::*;
