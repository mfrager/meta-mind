//! `mm-cli codex` — the code-metadata operator surface.
//!
//! `scan` is the only command that writes: it finds the modules, rewrites the
//! `/code` graph, mirrors the SQLite index, and (re)generates `registry.json`,
//! `codex.lock`, and the per-module `metadata.ttl`. Every other command is a read:
//! `verify` checks the rules, `meta` answers graph questions, `graph` renders a
//! derived view, `diff` reports drift, and `lock --check` compares the committed
//! lock against a fresh scan.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mm_codex::model::CodexReport;
use mm_codex::{CodexLock, MetaContext, MetaQuery, PreviousState, ViewOptions};
use mm_core::{Config, MmError};
use mm_log::{codes, Level, LogRecord};

use crate::kernel::Kernel;

/// The repository root a scan runs against.
///
/// A configuration loaded from `config/metamind.toml` by a relative path resolves
/// its root to the empty path (the paths inside it stay relative to the working
/// directory), so an empty root falls back to the compiled-in repository root.
/// That keeps `codex` commands working from any directory.
fn repo_root_of(cfg: &Config) -> PathBuf {
    if cfg.root.as_os_str().is_empty() {
        Config::repo_root()
    } else {
        cfg.root.clone()
    }
}

/// Resolve `--root`, relative paths resolving against the repository root.
fn root_of(cfg: &Config, root: Option<PathBuf>) -> PathBuf {
    let base = repo_root_of(cfg);
    match root {
        Some(path) if path.is_absolute() => path,
        Some(path) => base.join(path),
        None => base,
    }
}

/// A deterministic full scan of a tree.
fn scan_root(root: &Path) -> Result<mm_codex::ScanOutput, MmError> {
    mm_codex::scan(root, &PreviousState::default(), true)
}

/// Write the per-module `metadata.ttl` files, returning how many were written.
///
/// Only nexus modules under `modules/` get a file next to them; a crate's metadata
/// lives in the `/code` graph and `registry.json`. The files are git-ignored and
/// excluded from the walk, so regenerating them cannot feed back into the scan.
fn emit_module_metadata(root: &Path, report: &CodexReport) -> Result<usize, MmError> {
    let mut written = 0usize;
    for module in &report.modules {
        if !module.rel_path.starts_with("modules/") {
            continue;
        }
        let document = mm_codex::emit::module_metadata(report, &module.module_uri)?;
        let path = root.join(&module.rel_path).join("metadata.ttl");
        let unchanged = std::fs::read_to_string(&path).is_ok_and(|existing| existing == document);
        if !unchanged {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, document)?;
        }
        written += 1;
    }
    Ok(written)
}

/// `mm-cli codex scan`.
pub async fn scan(
    cfg: Config,
    root: Option<PathBuf>,
    no_emit: bool,
    update_lock: bool,
    json: bool,
) -> Result<ExitCode, MmError> {
    let root = root_of(&cfg, root);
    let lock_path = root.join("codex.lock");
    let registry_path = root.join("modules").join("registry.json");
    let started_at = mm_core::Timestamp::now().to_rfc3339();
    let kernel = Kernel::open(cfg.clone(), true).await?;

    kernel
        .logger
        .emit(
            LogRecord::new(Level::Info, codes::CODEX_SCAN_START, "mm.codex")
                .with_field("root", root.display().to_string())
                .with_field("mode", "full"),
        )
        .await?;

    let output = scan_root(&root)?;
    let report = output.report;
    let lock_previous = mm_codex::lock::read_lock(&lock_path)?;
    let failures = mm_codex::verify(&report, lock_previous.as_ref());

    // The `/code` projection is rewritten in full: re-inserting an unchanged
    // document is idempotent, but a node that disappeared from the source would
    // otherwise survive forever.
    let triples = kernel
        .graph()?
        .replace_turtle("code", &mm_codex::emit::turtle(&report))
        .await?;

    mm_codex::registry::mirror(
        &kernel.sqlite,
        &report,
        &kernel.ids,
        &started_at,
        failures.len(),
    )
    .await?;
    let registry_changed = mm_codex::registry::write_registry(&registry_path, &report)?;

    let mut lock_written = false;
    if update_lock {
        lock_written = mm_codex::lock::write_lock(&lock_path, &CodexLock::from_report(&report))?;
    }

    let emitted = if no_emit {
        0
    } else {
        emit_module_metadata(&root, &report)?
    };

    kernel
        .logger
        .emit(
            LogRecord::new(Level::Info, codes::CODEX_REGISTRY_WRITE, "mm.codex")
                .with_field("path", registry_path.display().to_string())
                .with_field("modules", report.modules.len())
                .with_field("files", report.files.len())
                .with_field("symbols", report.symbols.len())
                .with_field("graph_hash", report.graph_hash.clone())
                .with_field("changed", registry_changed),
        )
        .await?;

    kernel
        .logger
        .emit(
            LogRecord::new(Level::Info, codes::CODEX_SCAN_END, "mm.codex")
                .with_field("root", root.display().to_string())
                .with_field("modules", report.modules.len())
                .with_field("files", report.files.len())
                .with_field("symbols", report.symbols.len())
                .with_field("changed_files", report.changed_files)
                .with_field("violations", failures.len())
                .with_field("graph_hash", report.graph_hash.clone()),
        )
        .await?;

    for failure in &failures {
        kernel
            .logger
            .emit(
                LogRecord::new(Level::Warn, codes::CODEX_VERIFY_FAIL, "mm.codex")
                    .with_field("rule", failure.rule())
                    .with_field("detail", failure.to_string()),
            )
            .await?;
    }

    if json {
        let value = serde_json::json!({
            "root": root.display().to_string(),
            "graph_hash": report.graph_hash,
            "modules": report.modules.len(),
            "files": report.files.len(),
            "symbols": report.symbols.len(),
            "references": report.references.len(),
            "capabilities": report.capabilities.len(),
            "orphan_files": report.orphan_files.len(),
            "changed_files": report.changed_files,
            "violations": failures.len(),
            "triples": triples,
            "registry_changed": registry_changed,
            "lock_written": lock_written,
            "metadata_files": emitted,
        });
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("scanned {}", root.display());
        println!("  modules      {}", report.modules.len());
        println!("  files        {}", report.files.len());
        println!("  symbols      {}", report.symbols.len());
        println!("  capabilities {}", report.capabilities.len());
        println!("  triples      {} (in /code)", triples);
        println!("  graph_hash   {}", report.graph_hash);
        println!("  registry     {}", registry_path.display());
        if emitted > 0 {
            println!("  metadata.ttl {emitted} module(s)");
        }
        if !failures.is_empty() {
            println!("  violations   {}", failures.len());
        }
    }
    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}

/// `mm-cli codex verify`.
pub async fn verify(cfg: Config, root: Option<PathBuf>, json: bool) -> Result<ExitCode, MmError> {
    let root = root_of(&cfg, root);
    let output = scan_root(&root)?;
    // The lock describes the repository's own modules; a `--root` override points at
    // a different tree, whose modules the lock says nothing about.
    let lock = if root == repo_root_of(&cfg) {
        mm_codex::lock::read_lock(&root.join("codex.lock"))?
    } else {
        None
    };
    let failures = mm_codex::verify(&output.report, lock.as_ref());

    let kernel = Kernel::open(cfg, false).await?;
    for failure in &failures {
        kernel
            .logger
            .emit(
                LogRecord::new(Level::Warn, codes::CODEX_VERIFY_FAIL, "mm.codex")
                    .with_field("rule", failure.rule())
                    .with_field("detail", failure.to_string()),
            )
            .await?;
    }

    if json {
        let value = serde_json::json!({
            "root": root.display().to_string(),
            "graph_hash": output.report.graph_hash,
            "conforms": failures.is_empty(),
            "failures": failures
                .iter()
                .map(|f| serde_json::json!({ "rule": f.rule(), "detail": f.to_string() }))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string(&value)?);
    } else if failures.is_empty() {
        println!(
            "codex verify: {} modules conform",
            output.report.modules.len()
        );
    } else {
        println!("codex verify: {} violation(s)", failures.len());
        for failure in &failures {
            println!("  [{}] {}", failure.rule(), failure);
        }
    }
    kernel.sqlite.close().await;
    Ok(if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// `mm-cli codex meta`.
pub async fn meta(
    cfg: Config,
    root: Option<PathBuf>,
    query: String,
    json: bool,
) -> Result<ExitCode, MmError> {
    let query = MetaQuery::parse(&query)?;
    let root = root_of(&cfg, root);
    let kernel = Kernel::open(cfg.clone(), true).await?;
    let output = scan_root(&root)?;
    kernel
        .graph()?
        .replace_turtle("code", &output.turtle())
        .await?;
    let lock = mm_codex::lock::read_lock(&root.join("codex.lock"))?;
    let graph: &dyn mm_core::store::Graph = kernel.graph()?;
    let ctx = MetaContext {
        report: &output.report,
        graph: Some(graph),
        sqlite: Some(&kernel.sqlite),
        lock: lock.as_ref(),
    };
    let answer = mm_codex::answer_meta(&ctx, query).await?;

    if json {
        let value = serde_json::json!({
            "query": answer.query.name(),
            "columns": answer.columns,
            "rows": answer.rows,
        });
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("{}:", answer.query.name());
        if answer.rows.is_empty() {
            println!("  0 rows");
        } else {
            println!("  {}", answer.columns.join("  "));
            for row in &answer.rows {
                println!("  {}", row.join("  "));
            }
        }
    }
    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}

/// `mm-cli codex graph`.
pub async fn graph(
    cfg: Config,
    root: Option<PathBuf>,
    format: String,
    out: Option<PathBuf>,
    focus: Option<String>,
) -> Result<ExitCode, MmError> {
    let root = root_of(&cfg, root);
    let output = scan_root(&root)?;
    let view = mm_codex::mermaid(&output.report, &ViewOptions { focus });
    let document = match format.as_str() {
        "mermaid" => view.mermaid.clone(),
        "html" => format!(
            "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>Metamind code graph</title></head>\n\
             <body><pre class=\"mermaid\">\n{}</pre></body></html>\n",
            view.mermaid
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        ),
        other => {
            return Err(MmError::Config(format!(
                "unknown format {other:?}; use mermaid or html"
            )))
        }
    };
    match out {
        Some(path) => std::fs::write(&path, &document)?,
        None => println!("{document}"),
    }
    eprintln!(
        "codex graph: {} node(s), {} edge(s), {} hidden node(s), {} hidden edge(s)",
        view.manifest.nodes,
        view.manifest.edges,
        view.manifest.hidden_nodes,
        view.manifest.hidden_edges
    );
    Ok(ExitCode::SUCCESS)
}

/// `mm-cli codex diff` — structural interface drift, and optionally a lock diff.
pub async fn diff(cfg: Config, lock_path: Option<PathBuf>) -> Result<ExitCode, MmError> {
    let repo = repo_root_of(&cfg);
    let output = scan_root(&repo)?;

    if let Some(path) = lock_path {
        let committed = mm_codex::lock::read_lock(&path)?
            .ok_or_else(|| MmError::Config(format!("{} does not exist", path.display())))?;
        let fresh = CodexLock::from_report(&output.report);
        let delta = mm_codex::diff(&committed, &fresh);
        if delta.is_empty() {
            println!("codex diff: {} is current", path.display());
            return Ok(ExitCode::SUCCESS);
        }
        for uri in &delta.added {
            println!("+ {uri}");
        }
        for uri in &delta.removed {
            println!("- {uri}");
        }
        for uri in &delta.changed {
            println!("~ {uri}");
        }
        return Ok(ExitCode::from(1));
    }

    let kernel = Kernel::open(cfg, false).await?;
    let previous = mm_codex::drift::previous_interfaces(&kernel.sqlite).await?;
    let drift = mm_codex::detect_drift(&previous, &output.report);
    if drift.is_empty() {
        println!("codex diff: no interface drift since the last scan");
    } else {
        for change in &drift {
            println!("{}", change.module_uri);
            for added in &change.added {
                println!("  + {added}");
            }
            for removed in &change.removed {
                println!("  - {removed}");
            }
        }
    }
    kernel.sqlite.close().await;
    Ok(if drift.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// `mm-cli codex lock`.
pub async fn lock(cfg: Config, root: Option<PathBuf>, check: bool) -> Result<ExitCode, MmError> {
    let root = root_of(&cfg, root);
    let output = scan_root(&root)?;
    let fresh = CodexLock::from_report(&output.report);
    let path = root.join("codex.lock");
    let kernel = Kernel::open(cfg, false).await?;

    if check {
        let committed = mm_codex::lock::read_lock(&path)?.ok_or_else(|| {
            MmError::Config(format!(
                "{} does not exist; run `mm-cli codex lock` to create it",
                path.display()
            ))
        })?;
        let delta = mm_codex::diff(&committed, &fresh);
        kernel
            .logger
            .emit(
                LogRecord::new(Level::Info, codes::CODEX_LOCK_DIFF, "mm.codex")
                    .with_field("path", path.display().to_string())
                    .with_field("added", delta.added.len())
                    .with_field("removed", delta.removed.len())
                    .with_field("changed", delta.changed.len()),
            )
            .await?;
        if delta.is_empty() {
            println!("codex.lock is current: {} module(s)", fresh.module.len());
            kernel.sqlite.close().await;
            return Ok(ExitCode::SUCCESS);
        }
        println!("codex.lock is stale:");
        for uri in &delta.added {
            println!("  + {uri}");
        }
        for uri in &delta.removed {
            println!("  - {uri}");
        }
        for uri in &delta.changed {
            println!("  ~ {uri}");
        }
        kernel.sqlite.close().await;
        return Ok(ExitCode::from(1));
    }

    let changed = mm_codex::lock::write_lock(&path, &fresh)?;
    kernel
        .logger
        .emit(
            LogRecord::new(Level::Info, codes::CODEX_LOCK_WRITE, "mm.codex")
                .with_field("path", path.display().to_string())
                .with_field("modules", fresh.module.len())
                .with_field("changed", changed),
        )
        .await?;
    if changed {
        println!(
            "wrote {} ({} module(s))",
            path.display(),
            fresh.module.len()
        );
    } else {
        println!(
            "{} is already current ({} module(s))",
            path.display(),
            fresh.module.len()
        );
    }
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}
