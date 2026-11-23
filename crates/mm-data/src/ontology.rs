//! Loading the MetaMind ontology into the RDF store.
//!
//! The source ontology lives with the repository (`ontology/`), not with the
//! data root, so a temporary data directory still resolves it. Files are loaded
//! into the dedicated ontology graph; the shapes are loaded beside the axioms
//! so SHACL validation can find them.

use std::path::{Path, PathBuf};

use crate::engine::RdfEngine;
use crate::error::Result;
use crate::rdf::ONTOLOGY_GRAPH;

/// Ontology documents, in dependency order.
pub const ONTOLOGY_FILES: &[&str] = &[
    "ontology/mm.ttl",
    "ontology/code.ttl",
    "ontology/llm.ttl",
    "ontology/library.ttl",
    "ontology/metacog.ttl",
    "ontology/tools.ttl",
    "ontology/selfeng.ttl",
    "ontology/self_model.ttl",
    "ontology/session.ttl",
    "ontology/log.ttl",
    "ontology/data.ttl",
];

/// Load every available ontology document and shape file.
///
/// Returns the number of triples inserted. Missing files are skipped: a
/// distribution may ship a subset, and the kernel must still boot.
pub async fn load_metamind_ontology(engine: &RdfEngine, ontology_dir: &Path) -> Result<usize> {
    let mut loaded = 0;
    for relative in ONTOLOGY_FILES {
        let name = Path::new(relative)
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(relative));
        loaded += engine
            .load_turtle_file(ONTOLOGY_GRAPH, &ontology_dir.join(name))
            .await?;
    }
    if let Ok(entries) = std::fs::read_dir(ontology_dir.join("shapes")) {
        let mut shapes: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "ttl"))
            .collect();
        shapes.sort();
        for path in shapes {
            loaded += engine.load_turtle_file(ONTOLOGY_GRAPH, &path).await?;
        }
    }
    Ok(loaded)
}

/// The ontology directory for a configuration, falling back to the repository.
pub fn default_ontology_dir(configured: Option<&Path>) -> PathBuf {
    configured
        .map(Path::to_path_buf)
        .unwrap_or_else(|| mm_core::Config::repo_root().join("ontology"))
}
