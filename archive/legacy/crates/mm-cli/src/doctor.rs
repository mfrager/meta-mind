//! `mm-cli doctor` — prove the kernel is installed, opened, writable, and
//! audited before anything else is trusted.

use std::process::ExitCode;

use mm_core::{EventKind, MmError, NewEvent};
use mm_log::{codes, Level};
use mm_store_graph::OXIGRAPH_VERSION;

use crate::kernel::Kernel;

/// Run the kernel self-check.
pub async fn run(cfg: mm_core::Config) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, true).await?;
    println!(
        "mm-cli {} starting kernel self-check",
        env!("CARGO_PKG_VERSION")
    );
    println!("  codename            {}", kernel.cfg.kernel.codename);
    println!(
        "  data dir            {}",
        kernel.cfg.store.data_dir.display()
    );

    // --- tabular store -----------------------------------------------------
    let sqlite_version = kernel.sqlite.sqlite_version().await?;
    let journal_mode = kernel.sqlite.journal_mode().await?;
    let migrations = kernel.sqlite.applied_migrations().await?;
    println!("  sqlite              {sqlite_version} (journal_mode={journal_mode})");
    println!("  database            {}", kernel.sqlite.path().display());
    println!("  migrations applied  {migrations:?}");

    // --- RDF store ---------------------------------------------------------
    let graph = kernel.graph()?;
    println!("  oxigraph            {OXIGRAPH_VERSION}");
    println!(
        "  graph dir           {}",
        graph
            .path()
            .unwrap_or(std::path::Path::new("(memory)"))
            .display()
    );
    let invariants_ok = graph.storage_invariants_ok().await?;
    println!(
        "  graph invariants    {}",
        if invariants_ok { "ok" } else { "VIOLATED" }
    );

    // --- ontology ----------------------------------------------------------
    let loaded = kernel.load_ontology().await?;
    let ontology_triples = graph.ontology_triple_count().await?;
    let total_quads = graph.quad_count().await?;
    let named_graphs = graph.named_graphs().await?;
    println!("  ontology triples    {ontology_triples} ({loaded} loaded this run)");
    println!("  total quads         {total_quads}");
    println!(
        "  named graphs        {}",
        if named_graphs.is_empty() {
            "(none yet)".to_string()
        } else {
            named_graphs.join(", ")
        }
    );
    if ontology_triples == 0 {
        return Err(MmError::Graph(
            "the ontology is empty: the T-Box did not load".into(),
        ));
    }

    // --- event log ---------------------------------------------------------
    if loaded > 0 {
        let event = NewEvent::new(
            EventKind::OntologyLoad,
            &serde_json::json!({
                "triples": loaded,
                "files": kernel
                    .cfg
                    .ontology_files()
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
            }),
            None,
        )?;
        let id = kernel.events.append(event).await?;
        kernel.events.commit(&id).await?;
    }

    let boot = NewEvent::new(
        EventKind::KernelBoot,
        &serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "stores_ready": true,
            "sqlite": sqlite_version,
            "oxigraph": OXIGRAPH_VERSION,
            "ontology_triples": ontology_triples,
        }),
        None,
    )?;
    let boot_id = kernel.events.append(boot).await?;
    kernel.events.commit(&boot_id).await?;

    let (provisional, committed, aborted) = kernel.events.status_counts().await?;
    let head = kernel.events.head_seq().await?;
    let audit = kernel.sqlite.audit_chain_report().await?;
    println!(
        "  events              head seq {head}, committed {committed}, provisional {provisional}, aborted {aborted}"
    );
    println!(
        "  audit chain         {} record(s): {}",
        audit.rows,
        audit.summary()
    );

    // Audited, not merely logged: the boot record is part of the chain.
    kernel
        .logger
        .audit(
            Level::Info,
            codes::KERNEL_BOOT,
            "mm.cli",
            None,
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "stores_ready": true,
                "ontology_triples": ontology_triples,
            }),
        )
        .await?;

    if !audit.is_ok() {
        return Err(MmError::Internal(format!(
            "audit chain failed verification: {}",
            audit.summary()
        )));
    }
    if kernel.logger.counts().sink_errors > 0 {
        return Err(MmError::Internal(format!(
            "{} log sink failure(s): {:?}",
            kernel.logger.counts().sink_errors,
            kernel.logger.sink_failures()
        )));
    }

    println!("kernel ready");
    if let Some(graph) = &kernel.graph {
        graph.shutdown().await?;
    }
    Ok(ExitCode::SUCCESS)
}
