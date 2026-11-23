//! `mm-cli data` — compare and repair the managed-data lexical index.
//!
//! RDF is authority and the search index is derived. This command opens the
//! same data root the console uses, applies any pending outbox rows, then
//! reports the two disagreement classes an operator can act on:
//!
//! * **missing** — a live RDF entity the index cannot find;
//! * **orphan** — an indexed subject whose RDF entity is gone or deleted.
//!
//! `reindex` is the repair: it rebuilds the index from RDF, so it is safe to run
//! whenever `verify` is not clean.

use std::process::ExitCode;

use mm_core::{Config, MmError};
use mm_data::{initialize_data_layer, InitConfig};

/// Verify the index against RDF. Exits non-zero when either side disagrees.
pub async fn verify(cfg: Config, json: bool) -> Result<ExitCode, MmError> {
    let data = open(&cfg).await?;
    let pending = data
        .data_service
        .process_outbox()
        .await
        .map_err(data_error)?;
    let report = data.data_service.verify_index().await.map_err(data_error)?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "entities": report.entities,
                "documents": report.documents,
                "missing": report.missing,
                "orphaned": report.orphaned,
                "pendingApplied": pending,
                "clean": report.is_clean(),
            })
        );
    } else {
        println!(
            "data index: {} RDF entities, {} indexed documents, {} pending row(s) applied",
            report.entities, report.documents, pending
        );
        println!("  missing   {}", report.missing.len());
        println!("  orphaned  {}", report.orphaned.len());
        for iri in report.missing.iter().take(20) {
            println!("  missing   {iri}");
        }
        for iri in report.orphaned.iter().take(20) {
            println!("  orphan    {iri}");
        }
        if report.missing.len() > 20 || report.orphaned.len() > 20 {
            println!("  (lists truncated; run with --json for every IRI)");
        }
        println!(
            "{}",
            if report.is_clean() {
                "index clean"
            } else {
                "index dirty: run `mm-cli data reindex`"
            }
        );
    }
    shutdown(data).await?;
    Ok(if report.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Rebuild the index from RDF and report the resulting document count.
pub async fn reindex(cfg: Config, json: bool) -> Result<ExitCode, MmError> {
    let data = open(&cfg).await?;
    let documents = data.data_service.reindex().await.map_err(data_error)?;
    if json {
        println!("{}", serde_json::json!({ "reindexed": documents }));
    } else {
        println!("reindexed {documents} document(s) from RDF");
    }
    shutdown(data).await?;
    Ok(ExitCode::SUCCESS)
}

async fn open(cfg: &Config) -> Result<mm_data::InitResult, MmError> {
    initialize_data_layer(InitConfig {
        base_dir: Some(cfg.store.data_dir.clone()),
        ..InitConfig::default()
    })
    .await
    .map_err(init_error)
}

async fn shutdown(data: mm_data::InitResult) -> Result<(), MmError> {
    data.engine
        .store()
        .shutdown()
        .await
        .map_err(|error| MmError::Graph(error.to_string()))
}

fn data_error(error: mm_data::data::DataError) -> MmError {
    MmError::Internal(format!("data: {error}"))
}

fn init_error(error: mm_data::DataError) -> MmError {
    MmError::Internal(format!("data: {error}"))
}
