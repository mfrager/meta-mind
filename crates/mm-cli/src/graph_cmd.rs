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
    let shapes = kernel.cfg.shapes_file_for(graph_name);

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
