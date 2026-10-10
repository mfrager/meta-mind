//! `mm-cli library` — the cognitive library's operator surface, plus the shared
//! accessors every other library-facing command module uses.
//!
//! Two rules hold here:
//!
//! * **Every write goes through [`LibraryManager`]**, so a CLI write takes exactly
//!   the validated path a runtime write does: shapes, duplicate check, orphan check,
//!   then the graph and the index. There is no second door, and `library import`
//!   cannot write an entry the gate would refuse.
//! * **Every read reports what it actually checked.** `library validate` prints the
//!   entry count, the shape violations, the duplicates and the orphans, and exits
//!   non-zero if any of the four is a problem — a "0 violations" that only counted
//!   one of the three checks would be worse than no report at all.
//!
//! The applicability surface is a *proof* rather than a listing: with `--gold` it
//! compares its ranking against a committed table and exits non-zero on any
//! difference, because "the ranker is deterministic" is a claim that has to be
//! checked rather than asserted.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Subcommand;
use mm_core::{Config, MmError};
use mm_library::{
    declaratives_from_turtle, rank, seed, selectable, state_from_json, EntryKind, LibraryManager,
    LibraryStore,
};

use crate::kernel::Kernel;

/// `mm-cli library`.
#[derive(Subcommand, Debug)]
pub enum LibraryCommand {
    /// Import a Turtle document: validate it, then index and graph-write every entry.
    Import {
        /// The document to import, e.g. `ontology/seed/library_seed.ttl`.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Print the accepted and rejected entries as one JSON object.
        #[arg(long)]
        json: bool,
    },
    /// SHACL-validate `/library` and report duplicates and orphans.
    Validate {
        /// The named graph to validate; the gate uses `library`.
        #[arg(long, default_value = "library", value_name = "NAME")]
        graph: String,
        /// Print the report as one JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Report dangling references; optionally fail when there are any.
    Orphans {
        /// Exit non-zero when a reference does not resolve.
        #[arg(long)]
        fail_if_any: bool,
    },
    /// List entries, optionally of one kind.
    List {
        /// One of the entry kinds, e.g. `Technique`.
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        /// Print the rows as one JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Rank the applicable entries for a state; optionally check a gold table.
    Applicable {
        /// The state fixture, e.g. `bench/library/state_uncertain_strategy.json`.
        #[arg(long, value_name = "FILE")]
        state: PathBuf,
        /// How many ranked entries to print.
        #[arg(long, default_value_t = 5)]
        top: usize,
        /// A committed ranking to match exactly.
        #[arg(long, value_name = "FILE")]
        gold: Option<PathBuf>,
    },
    /// Extract candidate entries from a JSONL fixture, refusing the malformed ones.
    Extract {
        /// The candidate fixture, e.g. `bench/library/extraction/raw.jsonl`.
        #[arg(long, value_name = "FILE")]
        source: PathBuf,
        /// Where the accepted candidates are written.
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
}

/// `mm-cli library`.
pub async fn run(cfg: Config, command: LibraryCommand) -> Result<ExitCode, MmError> {
    match command {
        LibraryCommand::Import { file, json } => import(cfg, file, json).await,
        LibraryCommand::Validate { graph, json } => validate(cfg, graph, json).await,
        LibraryCommand::Orphans { fail_if_any } => orphans(cfg, fail_if_any).await,
        LibraryCommand::List { kind, json } => list(cfg, kind, json).await,
        LibraryCommand::Applicable { state, top, gold } => applicable(cfg, state, top, gold).await,
        LibraryCommand::Extract { source, out } => extract(cfg, source, out).await,
    }
}

// ------------------------------------------------------------------- context ----

/// Open the kernel and a manager wired to `/library`.
///
/// This is the only place a library command opens the stores, so `import`,
/// `validate` and the skill/case/policy/frame surfaces cannot disagree about which
/// graphs were loaded or which ULID watermark they share.
pub async fn open_manager(cfg: Config) -> Result<(Kernel, LibraryManager), MmError> {
    let kernel = Kernel::open(cfg, true).await?;
    kernel.load_ontology().await?;
    let store = LibraryStore::new(
        kernel.sqlite.clone(),
        Arc::clone(&kernel.logger),
        Arc::clone(&kernel.ids),
    );
    let handle = kernel.graph()?.handle().clone();
    Ok((kernel, LibraryManager::new(store, Some(handle))))
}

/// Close the stores a command opened.
pub async fn shutdown(kernel: Kernel) -> Result<(), MmError> {
    if let Some(graph) = &kernel.graph {
        graph.shutdown().await?;
    }
    kernel.sqlite.close().await;
    Ok(())
}

/// Every indexed entry, decoded back into a value.
///
/// The index holds the canonical body of each entry; the ranker and the selection
/// surfaces need the *fields* (`applicable_when`, `domain_fitness`, the evidence),
/// so the codec's decode half is what turns a row back into an entry. Entries whose
/// body does not decode are skipped rather than guessed at, and the skip is printed,
/// so a corrupt row is visible instead of silently missing from a rank.
pub async fn load_declaratives(
    manager: &LibraryManager,
) -> Result<Vec<mm_library::Declarative>, MmError> {
    let mut out = Vec::new();
    for row in manager.store().list_entries(None).await? {
        match declaratives_from_turtle(&row.body_ttl) {
            Ok(mut entries) => out.append(&mut entries),
            Err(e) => eprintln!("mm-cli: skipping {}: {e}", row.version_iri),
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ commands ----

async fn import(cfg: Config, file: PathBuf, json: bool) -> Result<ExitCode, MmError> {
    let (kernel, manager) = open_manager(cfg).await?;
    let turtle = std::fs::read_to_string(&file)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", file.display())))?;
    let report = manager.import_document(&turtle).await?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "file": file.display().to_string(),
                "accepted": report.accepted,
                "rejected": report.rejected,
            })
        );
    } else {
        for iri in &report.accepted {
            println!("accepted {iri}");
        }
        for (subject, reason) in &report.rejected {
            println!("rejected {subject}: {reason}");
        }
        println!(
            "{} accepted, {} rejected",
            report.accepted.len(),
            report.rejected.len()
        );
    }
    shutdown(kernel).await?;
    Ok(exit(!report.rejected.is_empty()))
}

async fn validate(cfg: Config, graph: String, json: bool) -> Result<ExitCode, MmError> {
    let (kernel, manager) = open_manager(cfg).await?;

    // The graph itself, against the library's own shapes.
    let shapes = std::fs::read_to_string(kernel.cfg.library_shapes_file())
        .map_err(|e| MmError::Config(format!("cannot read library shapes: {e}")))?;
    let dump = kernel.graph()?.dump_turtle(&graph).await?;
    let report = mm_store_graph::validate_turtle_text(&shapes, &dump, &graph)
        .map_err(|e| MmError::Codec(e.to_string()))?;

    // And the index: duplicates and orphans are not SHACL's business.
    let index = manager.validate_all().await?;
    let violations = report.violations.len() + index.violations.len();
    let clean = report.conforms && index.is_ok();

    if json {
        println!(
            "{}",
            serde_json::json!({
                "graph": graph,
                "conforms": report.conforms,
                "graph_violations": report.violations.iter().map(|v| serde_json::json!({
                    "path": v.path,
                    "message": v.message,
                    "severity": v.severity,
                })).collect::<Vec<_>>(),
                "entries": index.entries,
                "index_violations": index.violations.iter().map(|(iri, v)| serde_json::json!({
                    "entry": iri,
                    "shape": v.shape,
                    "message": v.message,
                    "path": v.path,
                })).collect::<Vec<_>>(),
                "duplicates": index.duplicates.iter().map(|d| serde_json::json!({
                    "iri": d.iri,
                    "existing": d.existing,
                    "content_hash": d.content_hash,
                })).collect::<Vec<_>>(),
                "orphans": index.orphans.iter().map(|o| serde_json::json!({
                    "entry": o.entry,
                    "missing": o.missing,
                })).collect::<Vec<_>>(),
            })
        );
    } else {
        println!("graph {graph}: {} triples", dump.lines().count());
        println!("{violations} shape violation(s)");
        for violation in &report.violations {
            println!("  {} {}", violation.path, violation.message);
        }
        for (entry, violation) in &index.violations {
            println!("  {entry} [{}] {}", violation.shape, violation.message);
        }
        println!("{}", index.summary());
        println!("library {}", if clean { "clean" } else { "not clean" });
    }

    shutdown(kernel).await?;
    Ok(exit(!clean))
}

async fn orphans(cfg: Config, fail_if_any: bool) -> Result<ExitCode, MmError> {
    let (kernel, manager) = open_manager(cfg).await?;
    let found = manager.orphans().await?;
    for report in &found {
        for missing in &report.missing {
            println!("{} -> {missing}", report.entry);
        }
    }
    println!("{} orphaned entry(ies)", found.len());
    shutdown(kernel).await?;
    Ok(exit(fail_if_any && !found.is_empty()))
}

async fn list(cfg: Config, kind: Option<String>, json: bool) -> Result<ExitCode, MmError> {
    let (kernel, manager) = open_manager(cfg).await?;
    let kind = match &kind {
        Some(text) => Some(
            EntryKind::parse(text)
                .ok_or_else(|| MmError::Config(format!("{text:?} is not an entry kind")))?,
        ),
        None => None,
    };
    let rows = manager
        .store()
        .list_entries(kind.map(EntryKind::as_str))
        .await?;

    if json {
        println!(
            "{}",
            serde_json::json!(rows
                .iter()
                .map(|row| serde_json::json!({
                    "version_iri": row.version_iri,
                    "kind": row.kind,
                    "version": row.version,
                    "title": row.title,
                    "activation_score": row.activation_score,
                }))
                .collect::<Vec<_>>())
        );
    } else {
        for row in &rows {
            println!(
                "{} {} v{} {}",
                row.version_iri, row.kind, row.version, row.title
            );
        }
    }
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

async fn applicable(
    cfg: Config,
    state_path: PathBuf,
    top: usize,
    gold: Option<PathBuf>,
) -> Result<ExitCode, MmError> {
    let (kernel, manager) = open_manager(cfg).await?;
    let raw = std::fs::read_to_string(&state_path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", state_path.display())))?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| MmError::Codec(format!("state: {e}")))?;
    let state = state_from_json(&value)?;

    let entries = load_declaratives(&manager).await?;
    let candidates: Vec<mm_library::Declarative> =
        selectable(&entries).into_iter().cloned().collect();
    let ranked = rank(&candidates, &state, top);

    for entry in &ranked {
        println!(
            "{:.6} {} {} [activation={:.6} fitness={:.6} penalty={:.6}]",
            entry.score.total,
            entry.iri,
            entry.title,
            entry.score.activation,
            entry.score.fitness,
            entry.score.contraindication_penalty
        );
    }

    manager
        .logger()
        .emit(
            mm_log::LogRecord::new(
                mm_log::Level::Info,
                mm_log::codes::LIBRARY_APPLICABLE_RANK,
                mm_library::manager::TARGET,
            )
            .with_field(
                "episode_ulid",
                mm_core::ulid_string(&manager.store().next_id()),
            )
            .with_field("state_hash", state.hash())
            .with_field(
                "ranked",
                ranked
                    .iter()
                    .map(|entry| entry.iri.clone())
                    .collect::<Vec<_>>(),
            )
            .with_field(
                "score_components",
                ranked.first().map(|e| e.score.components()),
            ),
        )
        .await?;

    let mismatches = match &gold {
        Some(path) => compare_gold(path, &ranked)?,
        None => Vec::new(),
    };
    for mismatch in &mismatches {
        println!("mismatch {mismatch}");
    }
    shutdown(kernel).await?;
    Ok(exit(!mismatches.is_empty()))
}

async fn extract(cfg: Config, source: PathBuf, out: PathBuf) -> Result<ExitCode, MmError> {
    let (kernel, manager) = open_manager(cfg).await?;
    let jsonl = std::fs::read_to_string(&source)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", source.display())))?;
    let report = seed::extract(&manager, &source.display().to_string(), &jsonl).await?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &report.accepted_jsonl)?;

    for rejection in &report.rejected {
        println!(
            "rejected line {} ({}): {}",
            rejection.line, rejection.iri_or_slug, rejection.reason
        );
    }
    println!(
        "{} accepted, {} rejected (one candidate per rejected line)",
        report.accepted.len(),
        report.rejected.len()
    );
    shutdown(kernel).await?;
    Ok(exit(report.has_rejections()))
}

// -------------------------------------------------------------------- helpers ---

/// One mismatch between the computed rank and the gold table.
fn compare_gold(path: &PathBuf, ranked: &[mm_library::Ranked]) -> Result<Vec<String>, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    let mut mismatches = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let expected: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| MmError::Codec(format!("{}:{}: {e}", path.display(), index + 1)))?;
        let Some(actual) = ranked.get(index) else {
            mismatches.push(format!(
                "line {}: no ranked entry at position {index}",
                index + 1
            ));
            continue;
        };
        if expected.get("iri").and_then(|v| v.as_str()) != Some(actual.iri.as_str()) {
            mismatches.push(format!(
                "line {}: expected {:?}, ranked {}",
                index + 1,
                expected
                    .get("iri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<none>"),
                actual.iri
            ));
        }
        for (key, actual_value) in [
            ("total", actual.score.total),
            ("activation", actual.score.activation),
            ("fitness", actual.score.fitness),
            (
                "contraindication_penalty",
                actual.score.contraindication_penalty,
            ),
        ] {
            if let Some(expected_value) = expected.get(key).and_then(|v| v.as_f64()) {
                if (expected_value as f32 - actual_value).abs() > 1e-6 {
                    mismatches.push(format!(
                        "line {}: {key} expected {expected_value}, computed {actual_value}",
                        index + 1
                    ));
                }
            }
        }
    }
    if ranked.len()
        != raw
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .count()
    {
        mismatches.push(format!(
            "the gold table has {} rows and the rank has {}",
            raw.lines()
                .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
                .count(),
            ranked.len()
        ));
    }
    Ok(mismatches)
}

/// A command's exit status: success or a plain failure.
fn exit(failed: bool) -> ExitCode {
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
