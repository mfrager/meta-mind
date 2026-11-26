//! `mm-loop` — the loop as a long-running process.
//!
//! The phase plan's §10 asks for the loop to be runnable as a monitored BACKGROUND
//! service with readiness, a budget stop and log checks, because a developmental loop is
//! measured in hours and a one-shot command cannot be supervised. This binary is that
//! process, reduced to what it has to be:
//!
//! * **It prints readiness before it works.** `ready` is printed only after every store is
//!   open, so a supervisor that sees it knows the process is not going to fail on a
//!   missing migration.
//! * **It stops on the budget, not on a timer.** The envelope is the run's own cap; a
//!   stage whose debit would cross it ends the run, and the exit status says so.
//! * **`--ticks N` drives the scheduler** `N` times after the run, which is the piece that
//!   distinguishes a loop from a script: a supervised process keeps its clocks running
//!   between runs.
//!
//! Flags are parsed by hand rather than with clap, because the binary's argument surface is
//! five flags and pulling a parser into the runtime crate would put the CLI's concerns in
//! the library.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use mm_core::{Config, UlidFactory};
use mm_eventlog::EventLog;
use mm_log::{codes, Level, LogRecord, Logger};
use mm_runtime::loop_controller::{LoopController, LoopServices};
use mm_runtime::timescale::TableScheduler;
use mm_runtime::{BudgetEnvelope, ClosedLoop, LoopGoal};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;

/// Everything the process was asked for.
struct Args {
    data_dir: Option<PathBuf>,
    goal_file: PathBuf,
    budget_file: Option<PathBuf>,
    ticks: usize,
    assert_completed: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut parsed = Args {
        data_dir: None,
        goal_file: PathBuf::from("bench/qualification/goal_novel.json"),
        budget_file: None,
        ticks: 0,
        assert_completed: false,
    };
    while let Some(arg) = args.next() {
        let mut value = |name: &str| -> Result<String, String> {
            args.next().ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--data-dir" => parsed.data_dir = Some(PathBuf::from(value("--data-dir")?)),
            "--goal-file" => parsed.goal_file = PathBuf::from(value("--goal-file")?),
            "--budget-file" => parsed.budget_file = Some(PathBuf::from(value("--budget-file")?)),
            "--ticks" => {
                parsed.ticks = value("--ticks")?
                    .parse()
                    .map_err(|e| format!("--ticks: {e}"))?
            }
            "--assert-completed" => parsed.assert_completed = true,
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(parsed)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("mm-loop: {message}");
            return ExitCode::from(2);
        }
    };
    match run(args).await {
        Ok(code) => code,
        Err(message) => {
            eprintln!("mm-loop: {message}");
            ExitCode::from(2)
        }
    }
}

async fn run(args: Args) -> Result<ExitCode, String> {
    let cfg = match &args.data_dir {
        Some(dir) => Config::for_data_dir(dir.clone()),
        None => Config::load_default().map_err(|e| e.to_string())?,
    };
    std::fs::create_dir_all(&cfg.store.data_dir).map_err(|e| e.to_string())?;
    let store = SqliteStore::open(&cfg.store.sqlite_file)
        .await
        .map_err(|e| e.to_string())?;
    store.migrate().await.map_err(|e| e.to_string())?;
    let logger = Arc::new(
        Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).map_err(|e| e.to_string())?,
    );
    let ids = Arc::new(UlidFactory::open(&cfg.ulid_watermark_path()).map_err(|e| e.to_string())?);
    let events = EventLog::new(store.clone(), logger.clone(), ids.clone());
    let graph = GraphStore::open(&cfg.store.graph_dir, &cfg.shapes_file())
        .await
        .map_err(|e| e.to_string())?;
    // One owner, many borrowers: the `Arc` is what the run's writers share, and this
    // binding is what keeps the writer thread alive until the explicit shutdown below.
    let graph = Arc::new(graph);

    logger
        .emit(
            LogRecord::new(Level::Info, codes::LOOP_RUN_START, mm_runtime::TARGET)
                .with_field("process", "mm-loop")
                .with_field("ready", true),
        )
        .await
        .map_err(|e| e.to_string())?;
    println!("ready mm-loop {:?}", cfg.store.data_dir);

    let goal = LoopGoal::from_file(&args.goal_file).map_err(|e| e.to_string())?;
    let envelope = match &args.budget_file {
        Some(path) => BudgetEnvelope::from_file(path).map_err(|e| e.to_string())?,
        None => BudgetEnvelope::default(),
    };
    let services = LoopServices::new(
        &cfg,
        store.clone(),
        logger.clone(),
        ids.clone(),
        events,
        Some(graph.clone()),
    );
    let controller = LoopController::new(services);
    let report = controller
        .run(goal.clone(), envelope)
        .await
        .map_err(|e| e.to_string())?;

    println!("run             {}", mm_core::ulid_string(&report.run_id));
    println!(
        "status          {}",
        if report.completed() {
            "completed"
        } else {
            "aborted"
        }
    );
    println!("stages          {}", report.stages.len());
    println!(
        "promoted        {}",
        report
            .promoted
            .map(|id| mm_core::ulid_string(&id.0))
            .unwrap_or_else(|| "none".to_string())
    );
    println!("budget_remaining {}", report.budget_used.canonical());

    if args.ticks > 0 {
        let scheduler = TableScheduler::new(store.clone(), logger.clone(), ids.clone());
        let ticks = scheduler
            .tick_n(args.ticks, mm_core::Timestamp::now())
            .await
            .map_err(|e| e.to_string())?;
        println!("ticks           {}", ticks.len());
        println!(
            "identity_ticks  {}",
            scheduler
                .identity_ticks()
                .await
                .map_err(|e| e.to_string())?
        );
    }

    graph.shutdown().await.map_err(|e| e.to_string())?;
    if args.assert_completed && !report.completed() {
        eprintln!("mm-loop: the run did not complete");
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}
