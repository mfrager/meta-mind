//! `mm-cli loop | self-model | debt | gc | module | design` — Phase 12's operator surface.
//!
//! Every command here drives the *same* objects the `mm-loop` background process drives:
//! [`mm_runtime::LoopController`] for a run, its replay and its artifacts,
//! [`mm_runtime::CompositeDebtScanner`] for a scan, [`mm_runtime::LadderCollector`] for the
//! ladder, [`mm_runtime::ChangeSetDesignWriter`] for a revision and
//! [`mm_runtime::ManifestModuleLoader`] for a load. Nothing re-implements a stage, because
//! a command that decided something the loop would have decided differently would make the
//! phase's replay claim false for the run it inspected.
//!
//! # Two deviations from the phase plan's §3, recorded rather than taken silently
//!
//! * The plan sketches `crates/mm-cli/src/cmd/{loop,self_model,debt,gc}.rs`. This
//!   repository's CLI has one flat module per command group (`tools_cmd.rs`,
//!   `selfeng_cmd.rs`, …) and no `cmd/` directory; the six groups live in this one module
//!   instead, so `main.rs` keeps one `mod` line per phase rather than a tree for one.
//! * The plan's §8 gate pipes `loop run`'s stdout straight into `jq`. Every command here
//!   prints a human summary by default and a single JSON object with `--json`, which is
//!   the convention the other nineteen command groups follow; the gate's `jq` reads the
//!   `--json` form, which the phase's e2e test asserts.
//!
//! # Exit status
//!
//! `0` when the command did what it was asked, `1` when it did and the answer was *no*
//! (a run that did not complete, a replay that did not reproduce, a protected subject the
//! collector touched, a design revision that was refused), and `2` when the invocation
//! itself was malformed (an unknown ULID, a missing fixture). The distinction is the same
//! one the loop's own errors draw between "the run stopped where it was told to" and "the
//! run is broken".

use std::process::ExitCode;

use clap::Subcommand;
use mm_core::{Config, MmError, Param, Tabular, Ulid};
use mm_runtime::{
    doc_ref, BudgetEnvelope, ChangeSetDesignWriter, LadderCollector, LoopController, LoopGoal,
    LoopServices, ManifestModuleLoader, ModuleLoader, ModuleVersion, ReplayOutcome,
};

use crate::kernel::Kernel;

/// `mm-cli loop`.
#[derive(Subcommand, Debug)]
pub enum LoopCommand {
    /// Run the ten-stage closed loop for a goal.
    Run {
        /// The goal fixture, e.g. `bench/qualification/goal_novel.json`.
        #[arg(long, value_name = "PATH")]
        goal_file: std::path::PathBuf,
        /// The budget envelope, e.g. `bench/qualification/budget.toml`.
        #[arg(long, value_name = "PATH")]
        budget_file: Option<std::path::PathBuf>,
        /// Assert the goal is novel: refuse a fixture that does not declare `novel`.
        #[arg(long)]
        novel: bool,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// What a run's row says, and which stages it committed.
    Status {
        /// The run's ULID.
        #[arg(long, value_name = "ULID")]
        run: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Every artifact a run registered, and its provenance chain.
    Artifacts {
        /// The run's ULID.
        #[arg(long, value_name = "ULID")]
        run: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Re-fold a run's events and compare them with the digest it recorded.
    Replay {
        /// The run's ULID.
        #[arg(long, value_name = "ULID")]
        run: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Continue a run at the first stage it has no committed row for.
    Resume {
        /// The run's ULID.
        #[arg(long, value_name = "ULID")]
        run: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli self-model`.
#[derive(Subcommand, Debug)]
pub enum SelfModelCommand {
    /// Measure a run: Actual vs Model vs Ideal, numerically.
    Report {
        /// The run's ULID.
        #[arg(long, value_name = "ULID")]
        run: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli debt`.
#[derive(Subcommand, Debug)]
pub enum DebtCommand {
    /// Scan the code graph, the memory and the policies for architectural debt.
    Scan {
        /// Attribute the findings to a run.
        #[arg(long, value_name = "ULID")]
        run: Option<String>,
        /// Exit non-zero unless at least this many findings were found.
        #[arg(long, value_name = "N")]
        at_least: Option<usize>,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli module`.
#[derive(Subcommand, Debug)]
pub enum ModuleCommand {
    /// Hot-load a materialised version, without restarting the process.
    Load {
        /// `URI@VERSION`, or just `URI` for the version on record.
        #[arg(long, value_name = "URI@VERSION")]
        uri: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Restore the version that was active before the newest load.
    Rollback {
        /// The module's IRI.
        #[arg(long, value_name = "URI")]
        uri: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli design`.
#[derive(Subcommand, Debug)]
pub enum DesignCommand {
    /// Revise a design document through a promoted change set, and only through one.
    Revise {
        /// The document's repository-relative path.
        #[arg(long, value_name = "PATH")]
        doc: std::path::PathBuf,
        /// The change set that carries the revision.
        #[arg(long, value_name = "ULID")]
        change_set: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Every recorded revision of a document, oldest first.
    History {
        /// The document's repository-relative path.
        #[arg(long, value_name = "PATH")]
        doc: std::path::PathBuf,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli loop`.
pub async fn run_loop(cfg: Config, command: LoopCommand) -> Result<ExitCode, MmError> {
    match command {
        LoopCommand::Run {
            goal_file,
            budget_file,
            novel,
            json,
        } => run(cfg, &goal_file, budget_file.as_deref(), novel, json).await,
        LoopCommand::Status { run, json } => status(cfg, &run, json).await,
        LoopCommand::Artifacts { run, json } => artifacts(cfg, &run, json).await,
        LoopCommand::Replay { run, json } => replay(cfg, &run, json).await,
        LoopCommand::Resume { run, json } => resume(cfg, &run, json).await,
    }
}

/// `mm-cli self-model`.
pub async fn self_model(cfg: Config, command: SelfModelCommand) -> Result<ExitCode, MmError> {
    match command {
        SelfModelCommand::Report { run, json } => report(cfg, &run, json).await,
    }
}

/// `mm-cli debt`.
pub async fn debt(cfg: Config, command: DebtCommand) -> Result<ExitCode, MmError> {
    match command {
        DebtCommand::Scan {
            run,
            at_least,
            json,
        } => scan(cfg, run.as_deref(), at_least, json).await,
    }
}

/// `mm-cli gc --apply`.
pub async fn gc(cfg: Config, apply: bool, json: bool) -> Result<ExitCode, MmError> {
    let (kernel, controller) = open(cfg).await?;
    let scan = controller.scan_debt(None).await.map_err(MmError::from)?;
    let collector = {
        let collector = LadderCollector::new(
            kernel.sqlite.clone(),
            kernel.logger.clone(),
            kernel.ids.clone(),
        );
        match &kernel.graph {
            Some(graph) => collector.with_graph(graph.clone()),
            None => collector,
        }
    };
    let report = collector.run(&scan.findings).await.map_err(MmError::from)?;
    let applied = if apply {
        for action in &report.actions {
            collector.apply(action).await.map_err(MmError::from)?;
        }
        report.actions.len()
    } else {
        0
    };

    if json {
        println!(
            "{}",
            serde_json::json!({
                "findings": scan.findings.len(),
                "protected": scan.protected().len(),
                "proposed": report.actions.len(),
                "refused": report.refused.len(),
                "applied": applied,
                "dry_run": !apply,
                "protected_untouched": report.protected_untouched(),
                "refusals": report.refused.iter().map(|refusal| serde_json::json!({
                    "action": refusal.action.kind.as_str(),
                    "subject_uri": refusal.action.subject,
                    "reason": refusal.reason,
                })).collect::<Vec<_>>(),
            })
        );
    } else {
        println!("findings          {}", scan.findings.len());
        println!("protected         {}", scan.protected().len());
        println!(
            "proposed          {} ({})",
            report.actions.len(),
            if apply { "applied" } else { "dry run" }
        );
        println!("refused           {}", report.refused.len());
        for refusal in &report.refused {
            println!(
                "  {:<14} {} — {}",
                refusal.action.kind.as_str(),
                refusal.action.subject,
                refusal.reason
            );
        }
        println!("protected_untouched {}", report.protected_untouched());
    }
    shutdown(&kernel).await?;
    // A collector that applied a protected subject has broken the phase's central guard,
    // so the command says so in its exit status rather than in a line of output.
    Ok(if report.protected_untouched() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// `mm-cli module`.
pub async fn module(cfg: Config, command: ModuleCommand) -> Result<ExitCode, MmError> {
    match command {
        ModuleCommand::Load { uri, json } => load(cfg, &uri, json).await,
        ModuleCommand::Rollback { uri, json } => rollback(cfg, &uri, json).await,
    }
}

/// `mm-cli design`.
pub async fn design(cfg: Config, command: DesignCommand) -> Result<ExitCode, MmError> {
    match command {
        DesignCommand::Revise {
            doc,
            change_set,
            json,
        } => revise(cfg, &doc, &change_set, json).await,
        DesignCommand::History { doc, json } => history(cfg, &doc, json).await,
    }
}

// ------------------------------------------------------------------- the loop ----

/// Open the kernel with the RDF store and build the controller over it.
async fn open(cfg: Config) -> Result<(Kernel, LoopController), MmError> {
    let kernel = Kernel::open(cfg, true).await?;
    let controller = controller_for(&kernel);
    Ok((kernel, controller))
}

/// The controller the CLI drives.
///
/// Built from the kernel's own stores, logger, identifier factory and event log, so a run
/// started from this command and a run started by `mm-loop` are the same run: the same
/// `config_hash` (the kernel's configuration), the same `code_version` (this build) and
/// the same append-only log.
fn controller_for(kernel: &Kernel) -> LoopController {
    let services = LoopServices::new(
        &kernel.cfg,
        kernel.sqlite.clone(),
        kernel.logger.clone(),
        kernel.ids.clone(),
        // The event log is not cloneable — it is a writer over the store — so the loop
        // gets its own handle to the *same* log: same store, same sequence, same ids.
        mm_eventlog::EventLog::new(
            kernel.sqlite.clone(),
            kernel.logger.clone(),
            kernel.ids.clone(),
        ),
        kernel.graph.clone(),
    );
    LoopController::new(services)
}

/// Run the loop for a goal.
async fn run(
    cfg: Config,
    goal_file: &std::path::Path,
    budget_file: Option<&std::path::Path>,
    novel: bool,
    json: bool,
) -> Result<ExitCode, MmError> {
    // A fixture path is written relative to the *repository* (it is a committed file),
    // while the process may be started from anywhere; resolving it here is what makes
    // `loop run --goal-file bench/qualification/goal_novel.json` mean the same thing from
    // the repository root and from a test harness.
    let goal_file = repo_path(goal_file);
    let goal = LoopGoal::from_file(&goal_file).map_err(MmError::from)?;
    if novel && !goal.novel {
        return Err(MmError::Config(format!(
            "--novel asserts a goal {} does not declare novel; the fixture, not the flag, \
             is what says whether a capability already answers it",
            goal_file.display()
        )));
    }
    let envelope = match budget_file {
        Some(path) => BudgetEnvelope::from_file(&repo_path(path)).map_err(MmError::from)?,
        None => BudgetEnvelope::default(),
    };
    let (kernel, controller) = open(cfg).await?;
    let report = controller
        .run_loop(goal.clone(), envelope)
        .await
        .map_err(MmError::from)?;
    if json {
        println!("{}", serde_json::to_string(&report).map_err(internal)?);
    } else {
        println!("run               {}", mm_core::ulid_string(&report.run_id));
        println!(
            "status            {}",
            if report.completed() {
                "completed"
            } else {
                "aborted"
            }
        );
        println!("novel             {}", goal.novel);
        for stage in &report.stages {
            println!(
                "  {:<2} {:<16} {:<8} {} ms  {}",
                stage.idx,
                stage.stage.as_str(),
                stage.outcome,
                stage.latency_ms,
                stage.detail
            );
        }
        println!(
            "promoted          {}",
            report
                .promoted
                .map(|id| mm_core::ulid_string(&id.0))
                .unwrap_or_else(|| "none".to_string())
        );
        println!("digest            {}", report.digest);
        println!("remaining         {}", report.budget_used.canonical());
    }
    shutdown(&kernel).await?;
    Ok(if report.completed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// The run's row and its stages.
async fn status(cfg: Config, run: &str, json: bool) -> Result<ExitCode, MmError> {
    let run = parse_ulid(run, "run")?;
    let (kernel, controller) = open(cfg).await?;
    let row = kernel
        .sqlite
        .query_json(
            "SELECT * FROM loop_runs WHERE id = ?",
            vec![Param::Text(mm_core::ulid_string(&run))],
        )
        .await?;
    let Some(row) = row.first() else {
        return Err(MmError::Config(format!(
            "no run {}",
            mm_core::ulid_string(&run)
        )));
    };
    let stages = kernel
        .sqlite
        .query_json(
            "SELECT idx, stage, outcome, detail, started_at, ended_at FROM loop_iterations \
             WHERE run_id = ? ORDER BY idx",
            vec![Param::Text(mm_core::ulid_string(&run))],
        )
        .await?;
    // The digest the row carries is the recorded fold; recomputing it is what makes
    // `status` a check rather than a rehearsal of the row.
    let outcome = controller.replay(&run).await.map_err(MmError::from)?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "run_id": mm_core::ulid_string(&run),
                "row": row,
                "stages": stages,
                "digest_identical": outcome.identical,
                "recorded_digest": outcome.recorded,
                "recomputed_digest": outcome.recomputed,
            })
        );
    } else {
        println!("run_id            {}", mm_core::ulid_string(&run));
        println!(
            "status            {}",
            row["status"].as_str().unwrap_or_default()
        );
        println!(
            "novel             {}",
            row["novel"].as_i64().unwrap_or(0) == 1
        );
        println!(
            "goal              {}",
            row["goal_text"].as_str().unwrap_or_default()
        );
        println!(
            "config_hash       {}",
            row["config_hash"].as_str().unwrap_or_default()
        );
        println!(
            "code_version      {}",
            row["code_version"].as_str().unwrap_or_default()
        );
        println!(
            "started_at        {}",
            row["started_at"].as_str().unwrap_or_default()
        );
        println!(
            "ended_at          {}",
            row["ended_at"].as_str().unwrap_or("running")
        );
        for stage in &stages {
            println!(
                "  {:<2} {:<16} {:<8} {}",
                stage["idx"].as_i64().unwrap_or_default(),
                stage["stage"].as_str().unwrap_or_default(),
                stage["outcome"].as_str().unwrap_or_default(),
                stage["detail"].as_str().unwrap_or_default()
            );
        }
        println!("digest_identical  {}", outcome.identical);
    }
    shutdown(&kernel).await?;
    Ok(if outcome.identical {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// The run's artifacts, and the provenance chain they are part of.
async fn artifacts(cfg: Config, run: &str, json: bool) -> Result<ExitCode, MmError> {
    let run = parse_ulid(run, "run")?;
    let (kernel, controller) = open(cfg).await?;
    let artifacts = controller.artifacts_of(&run).await.map_err(MmError::from)?;
    let chain = controller.chain(&run).await.map_err(MmError::from)?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "run_id": mm_core::ulid_string(&run),
                "artifacts": artifacts,
                "chain": chain,
                "chain_rendered": chain.rendered(),
                "chain_complete": chain.complete(),
            })
        );
    } else {
        println!("run_id            {}", mm_core::ulid_string(&run));
        for artifact in &artifacts {
            println!(
                "  {:<12} {:<48} {}",
                artifact.kind, artifact.uri, artifact.detail
            );
        }
        println!("chain             {}", chain.rendered());
        println!(
            "pi_session        {}",
            chain
                .pi_session
                .map(|id| mm_core::ulid_string(&id))
                .unwrap_or_else(|| "none".to_string())
        );
        println!("edits             {}", chain.edits.len());
        println!(
            "module            {}",
            chain.module.clone().unwrap_or_default()
        );
        println!(
            "change_set        {}",
            chain
                .change_set
                .map(|id| mm_core::ulid_string(&id))
                .unwrap_or_else(|| "none".to_string())
        );
        println!(
            "promotion         {}",
            chain
                .promotion
                .map(|id| mm_core::ulid_string(&id))
                .unwrap_or_else(|| "none".to_string())
        );
        println!("complete          {}", chain.complete());
    }
    shutdown(&kernel).await?;
    Ok(if chain.complete() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Re-fold the run's events and compare.
async fn replay(cfg: Config, run: &str, json: bool) -> Result<ExitCode, MmError> {
    let run = parse_ulid(run, "run")?;
    let (kernel, controller) = open(cfg).await?;
    let outcome = controller.replay(&run).await.map_err(MmError::from)?;
    render_replay(&outcome, json);
    shutdown(&kernel).await?;
    Ok(if outcome.identical {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Continue a run at the first stage it has no committed row for.
async fn resume(cfg: Config, run: &str, json: bool) -> Result<ExitCode, MmError> {
    let run = parse_ulid(run, "run")?;
    let (kernel, controller) = open(cfg).await?;
    let report = controller.resume(&run).await.map_err(MmError::from)?;
    if json {
        println!("{}", serde_json::to_string(&report).map_err(internal)?);
    } else {
        println!("run               {}", mm_core::ulid_string(&report.run_id));
        println!("stages            {}", report.stages.len());
        println!(
            "status            {}",
            if report.completed() {
                "completed"
            } else {
                "aborted"
            }
        );
        println!("digest            {}", report.digest);
    }
    shutdown(&kernel).await?;
    Ok(if report.completed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn render_replay(outcome: &ReplayOutcome, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "run_id": mm_core::ulid_string(&outcome.run_id),
                "recorded": outcome.recorded,
                "recomputed": outcome.recomputed,
                "identical": outcome.identical,
                "stages": outcome.stages,
            })
        );
        return;
    }
    println!(
        "run_id            {}",
        mm_core::ulid_string(&outcome.run_id)
    );
    println!("recorded          {}", outcome.recorded);
    println!("recomputed        {}", outcome.recomputed);
    println!("identical         {}", outcome.identical);
    println!("stages            {}", outcome.stages.join(", "));
}

// --------------------------------------------------------------- the self-model ----

/// Measure a run.
async fn report(cfg: Config, run: &str, json: bool) -> Result<ExitCode, MmError> {
    let run = parse_ulid(run, "run")?;
    let (kernel, controller) = open(cfg).await?;
    let report = controller.measure(&run).await.map_err(MmError::from)?;
    if json {
        println!("{}", serde_json::to_string(&report).map_err(internal)?);
    } else {
        println!("report_id         {}", mm_core::ulid_string(&report.id));
        println!("run_id            {}", mm_core::ulid_string(&report.run_id));
        println!("actual_model      {:.6}", report.actual_model);
        println!("actual_ideal      {:.6}", report.actual_ideal);
        println!("model_ideal       {:.6}", report.model_ideal);
        for divergence in &report.dims {
            println!("  {:<24} {:.6}", divergence.dimension, divergence.value);
        }
    }
    shutdown(&kernel).await?;
    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------------- debt ----

/// Scan for architectural debt.
async fn scan(
    cfg: Config,
    run: Option<&str>,
    at_least: Option<usize>,
    json: bool,
) -> Result<ExitCode, MmError> {
    let run = match run {
        Some(run) => Some(parse_ulid(run, "run")?),
        None => None,
    };
    let (kernel, controller) = open(cfg).await?;
    let report = controller.scan_debt(run).await.map_err(MmError::from)?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "findings": report.findings,
                "persisted": report.persisted,
                "graph_triples": report.graph_triples,
                "protected": report.protected().len(),
            })
        );
    } else {
        for finding in &report.findings {
            println!(
                "  {:<20} {:.2} {}{} {}",
                finding.kind.as_str(),
                finding.severity,
                finding.subject,
                if finding.protects_ledger {
                    "  [protected]"
                } else {
                    ""
                },
                if finding.evidence.is_empty() {
                    String::new()
                } else {
                    format!("— {}", finding.evidence[0])
                }
            );
        }
        println!("findings          {}", report.findings.len());
        println!("persisted         {}", report.persisted);
        println!("graph_triples     {}", report.graph_triples);
        println!("protected         {}", report.protected().len());
    }
    shutdown(&kernel).await?;
    Ok(match at_least {
        Some(minimum) if report.findings.len() < minimum => ExitCode::from(1),
        _ => ExitCode::SUCCESS,
    })
}

// --------------------------------------------------------------------- module ----

/// Hot-load a materialised module version.
async fn load(cfg: Config, spec: &str, json: bool) -> Result<ExitCode, MmError> {
    let (uri, version) = ModuleVersion::parse_spec(spec).map_err(MmError::from)?;
    let kernel = Kernel::open(cfg, true).await?;
    let loader = loader_for(&kernel);
    // A version that was never materialised cannot be loaded, and the artifact root is
    // the only place a promoted version lives — so a `load` of an unknown version is a
    // refusal here rather than a row in `module_loads` that claims otherwise.
    let dir = loader
        .find(&uri, &version)
        .ok_or_else(|| MmError::Config(format!("no materialised version {uri}@{version}")))?;
    let receipt = loader
        .hot_load(&ModuleVersion::new(uri, version, dir))
        .await
        .map_err(MmError::from)?;
    render_receipt(&receipt, json);
    shutdown(&kernel).await?;
    Ok(ExitCode::SUCCESS)
}

/// Restore the version that was active before the newest load.
async fn rollback(cfg: Config, uri: &str, json: bool) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, true).await?;
    let loader = loader_for(&kernel);
    let receipt = loader.rollback(uri).await.map_err(MmError::from)?;
    render_receipt(&receipt, json);
    shutdown(&kernel).await?;
    Ok(ExitCode::SUCCESS)
}

fn render_receipt(receipt: &mm_runtime::LoadReceipt, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": mm_core::ulid_string(&receipt.id),
                "uri": receipt.uri,
                "version": receipt.version,
                "status": receipt.status.as_str(),
                "functions": receipt.functions,
                "previous": receipt.previous,
                "latency_ms": receipt.latency_ms,
            })
        );
        return;
    }
    println!("load_id           {}", mm_core::ulid_string(&receipt.id));
    println!("uri               {}", receipt.uri);
    println!("version           {}", receipt.version);
    println!("status            {}", receipt.status.as_str());
    println!("functions         {}", receipt.functions.join(", "));
    println!(
        "previous          {}",
        receipt
            .previous
            .clone()
            .unwrap_or_else(|| "none".to_string())
    );
    println!("latency_ms        {}", receipt.latency_ms);
}

fn loader_for(kernel: &Kernel) -> ManifestModuleLoader {
    let loader = ManifestModuleLoader::new(
        kernel.sqlite.clone(),
        kernel.logger.clone(),
        kernel.ids.clone(),
        kernel.cfg.store.data_dir.join("artifacts"),
    );
    match &kernel.graph {
        Some(graph) => loader.with_graph(graph.clone()),
        None => loader,
    }
}

// --------------------------------------------------------------------- design ----

/// Revise a document through a promoted change set.
async fn revise(
    cfg: Config,
    doc: &std::path::Path,
    change_set: &str,
    json: bool,
) -> Result<ExitCode, MmError> {
    let change_set = parse_ulid(change_set, "change_set")?;
    let doc = doc_ref(doc);
    let kernel = Kernel::open(cfg, true).await?;
    let writer = writer_for(&kernel);
    let revision = writer
        .revise_from_change_set(&doc, change_set)
        .await
        .map_err(MmError::from)?;
    render_revision(&revision, json);
    shutdown(&kernel).await?;
    Ok(ExitCode::SUCCESS)
}

/// Every recorded revision of a document.
async fn history(cfg: Config, doc: &std::path::Path, json: bool) -> Result<ExitCode, MmError> {
    let doc = doc_ref(doc);
    let kernel = Kernel::open(cfg, true).await?;
    let writer = writer_for(&kernel);
    let revisions = writer.history(&doc.uri).await.map_err(MmError::from)?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "doc_uri": doc.uri,
                "revisions": revisions,
            })
        );
    } else {
        println!("doc_uri           {}", doc.uri);
        for revision in &revisions {
            println!(
                "  {:<4} {:<8} {}",
                revision.revision,
                if revision.applied {
                    "applied"
                } else {
                    "refused"
                },
                mm_core::ulid_string(&revision.change_set)
            );
        }
        println!("revisions         {}", revisions.len());
    }
    shutdown(&kernel).await?;
    Ok(ExitCode::SUCCESS)
}

fn render_revision(revision: &mm_runtime::Revision, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": mm_core::ulid_string(&revision.id),
                "doc_uri": revision.doc_uri,
                "path": revision.path.display().to_string(),
                "revision": revision.revision,
                "change_set": mm_core::ulid_string(&revision.change_set),
                "applied": revision.applied,
            })
        );
        return;
    }
    println!("revision_id       {}", mm_core::ulid_string(&revision.id));
    println!("doc_uri           {}", revision.doc_uri);
    println!("path              {}", revision.path.display());
    println!("revision          {}", revision.revision);
    println!(
        "change_set        {}",
        mm_core::ulid_string(&revision.change_set)
    );
    println!("applied           {}", revision.applied);
}

fn writer_for(kernel: &Kernel) -> ChangeSetDesignWriter {
    let writer = ChangeSetDesignWriter::new(
        kernel.sqlite.clone(),
        kernel.logger.clone(),
        kernel.ids.clone(),
        kernel.cfg.store.data_dir.join("artifacts"),
    );
    match &kernel.graph {
        Some(graph) => writer.with_graph(graph.clone()),
        None => writer,
    }
}

// ------------------------------------------------------------------------ misc ----

/// Stop the graph writer before the process ends, the way every other command does.
async fn shutdown(kernel: &Kernel) -> Result<(), MmError> {
    if let Some(graph) = &kernel.graph {
        graph.shutdown().await?;
    }
    kernel.sqlite.close().await;
    Ok(())
}

/// A repository-relative path, resolved against the repository the build knows about.
fn repo_path(path: &std::path::Path) -> std::path::PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Config::repo_root().join(path)
    }
}

fn parse_ulid(text: &str, field: &str) -> Result<Ulid, MmError> {
    mm_core::id::parse_ulid(text)
        .map_err(|e| MmError::Config(format!("{field}: {text:?} is not a ULID: {e}")))
}

fn internal<E: std::fmt::Display>(error: E) -> MmError {
    MmError::Internal(error.to_string())
}
