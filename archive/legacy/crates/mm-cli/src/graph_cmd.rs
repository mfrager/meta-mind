//! `mm-cli graph validate` — enforce the shapes against a named graph.

use std::process::ExitCode;

use mm_core::{iri, MmError};
use mm_log::{codes, Level, LogRecord};

use crate::kernel::Kernel;

/// Validate one named graph against the kernel shapes.
pub async fn validate(cfg: mm_core::Config, graph_name: &str) -> Result<ExitCode, MmError> {
    if !iri::is_named_graph(graph_name) {
        return Err(MmError::Config(format!(
            "unknown graph {graph_name:?}; known graphs are {}",
            iri::NAMED_GRAPHS.join(", ")
        )));
    }
    let kernel = Kernel::open(cfg, true).await?;
    let graph = kernel.graph()?;
    // The `code` graph is described by the `mmc:` shapes; every other graph by
    // `mm:`. Validating one against the other's shapes would report nonsense.
    let shapes = governed_shapes(&kernel.cfg, graph_name)?;

    let report = graph.validate_with(graph_name, &shapes).await?;
    let quads = graph.graph_quad_count(graph_name).await?;

    kernel
        .logger
        .emit(
            LogRecord::new(Level::Info, codes::SHACL_VALIDATE, "mm.cli")
                .with_field("graph", graph_name)
                .with_field("shapes", shapes.display().to_string())
                .with_field("conforms", report.conforms)
                .with_field("violations", report.violations.len()),
        )
        .await?;

    println!(
        "graph {} : {} quad(s), shapes {}",
        graph_name,
        quads,
        shapes.display()
    );
    if report.conforms {
        println!("  conforms: 0 violations");
        graph.shutdown().await?;
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "  does not conform: {} violation(s)",
        report.violations.len()
    );
    for violation in &report.violations {
        println!(
            "    [{}] {} ({})",
            violation.severity, violation.message, violation.path
        );
    }
    graph.shutdown().await?;
    Ok(ExitCode::from(1))
}

/// The shapes file that governs a graph, merging the additional sets when it has
/// any.
///
/// `/epistemic` holds both the claims and the metacognitive controller's episodes,
/// and the two shape sets were written separately. The vendored validator takes
/// one shapes document, so when a graph is governed by more than one set the
/// documents are concatenated into a derived file. It is written under the log
/// directory — derived state, never a source — so an operator can read exactly
/// what was validated against.
fn governed_shapes(cfg: &mm_core::Config, graph_name: &str) -> Result<std::path::PathBuf, MmError> {
    let primary = cfg.shapes_file_for(graph_name);
    let extras = cfg.extra_shapes_files_for(graph_name);
    if extras.is_empty() {
        return Ok(primary);
    }
    let mut merged = String::new();
    for path in std::iter::once(&primary).chain(extras.iter()) {
        let text = std::fs::read_to_string(path)
            .map_err(|e| MmError::Config(format!("cannot read shapes {}: {e}", path.display())))?;
        merged.push_str(&text);
        if !merged.ends_with('\n') {
            merged.push('\n');
        }
    }
    let dir = cfg.log.dir.join("shapes");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}_merged.ttl", graph_name.replace('/', "_")));
    std::fs::write(&path, merged)?;
    Ok(path)
}
