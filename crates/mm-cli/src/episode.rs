//! `mm-cli episode` — the Phase 8 operator surface.
//!
//! Six subcommands, each answering a different question an operator asks about
//! the metacognitive controller:
//!
//! * `run` — deliberate over a corpus and write one artifact per episode.
//! * `replay` — re-run from the fixture and prove the program, the scores and the
//!   trace come out byte-identical.
//! * `verify` — check every artifact against the gold table, and regenerate the
//!   gold table on demand.
//! * `value` — recompute the `OperationValue` arithmetic and compare it with the
//!   committed gold table, exactly.
//! * `budget-audit` — prove no episode exceeded any budget dimension and that a
//!   trivial episode used no more than `k` operations.
//! * `program` — print one compiled program as JSON or Turtle.
//!
//! Every command goes through `Kernel`, so an episode is persisted into the same
//! SQLite store and `/epistemic` graph the rest of the kernel writes, and every
//! record it emits reaches the log and audit chain the rest of the kernel uses.
//! Nothing here opens a store of its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::Subcommand;
use mm_core::{Config, MmError, Tabular, Ulid};
use mm_log::codes;
use mm_metacog::budget::{BudgetResource, CognitiveBudget, BUDGET_RESOURCES};
use mm_metacog::episode::CognitiveEpisode;
use mm_metacog::lower::ToolCatalog;
use mm_metacog::scan::ScanResult;
use mm_metacog::tier::Tier;
use mm_metacog::trace::SqliteTraceStore;
use mm_metacog::{
    lower, MetacognitiveController, MinimumCognitionController, MockScanDriver, ProgramTrace,
    TraceStore,
};
use serde::{Deserialize, Serialize};

use crate::kernel::Kernel;

/// The default fixture directory the pass gate reads.
const DEFAULT_INPUT: &str = "bench/episodes";
/// The default artifact directory the pass gate writes and reads.
const DEFAULT_ARTIFACTS: &str = "data/artifacts/episodes";

/// The `episode` subcommands.
#[derive(Subcommand, Debug)]
pub enum EpisodeCommand {
    /// Deliberate over one episode file or a directory of them.
    Run {
        /// An episode fixture file, or a directory of `*.json` fixtures.
        #[arg(long, value_name = "PATH", default_value = DEFAULT_INPUT)]
        input: PathBuf,
        /// Pin the compute tier: `auto`, or `0`..`5`.
        #[arg(long, value_name = "TIER", default_value = "auto")]
        tier: String,
        /// Override the budget as `max_ops=N,max_llm_calls=N,max_cost=X,max_wall_ms=N`.
        #[arg(long, value_name = "SPEC")]
        budget: Option<String>,
        /// Where the per-episode artifacts are written.
        #[arg(long, value_name = "DIR", default_value = DEFAULT_ARTIFACTS)]
        out: PathBuf,
    },
    /// Re-run from the fixtures and compare with the artifacts.
    Replay {
        /// One episode ULID. Mutually exclusive with `--all`.
        #[arg(long, value_name = "ULID", conflicts_with = "all")]
        episode: Option<String>,
        /// Every episode in the fixture directory.
        #[arg(long)]
        all: bool,
        /// The fixture directory.
        #[arg(long, value_name = "DIR", default_value = DEFAULT_INPUT)]
        input: PathBuf,
        /// Compare against the artifacts in this directory.
        #[arg(long, value_name = "DIR")]
        compare: Option<PathBuf>,
    },
    /// Check every artifact against the gold table.
    Verify {
        /// The artifact directory.
        #[arg(long, value_name = "DIR", default_value = DEFAULT_ARTIFACTS)]
        artifacts: PathBuf,
        /// The gold directory holding `programs.jsonl`.
        #[arg(long, value_name = "DIR")]
        gold: PathBuf,
        /// Regenerate the gold table from the artifacts instead of checking it.
        #[arg(long)]
        write_gold: bool,
    },
    /// Recompute the operation-value arithmetic against the gold table.
    Value {
        /// The gold table: `name,eer,importance,p_change,cost,score`.
        #[arg(long, value_name = "FILE")]
        gold_table: PathBuf,
        /// Rewrite the `score` column from the inputs instead of comparing.
        #[arg(long)]
        write: bool,
    },
    /// Prove no episode exceeded its budget, and that the trivial ones stayed small.
    BudgetAudit {
        /// The artifact directory.
        #[arg(long, value_name = "DIR", default_value = DEFAULT_ARTIFACTS)]
        artifacts: PathBuf,
        /// The thresholds file.
        #[arg(long, value_name = "FILE")]
        thresholds: PathBuf,
    },
    /// Print one compiled program.
    Program {
        /// The episode ULID.
        #[arg(long, value_name = "ULID")]
        episode: String,
        /// The artifact directory to read it from.
        #[arg(long, value_name = "DIR", default_value = DEFAULT_ARTIFACTS)]
        artifacts: PathBuf,
        /// `json` or `turtle`.
        #[arg(long, default_value = "json")]
        format: String,
    },
}

/// One fixture file: the episode to run and the scan that frames it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeFixture {
    /// `trivial`, `standard` or `hard`. The budget audit reads it.
    #[serde(default = "default_difficulty")]
    pub difficulty: String,
    /// The episode.
    pub episode: CognitiveEpisode,
    /// The scan the controller starts from. A fixed scan is what makes the whole
    /// corpus reproducible without a model.
    pub scan: ScanResult,
}

fn default_difficulty() -> String {
    "standard".to_string()
}

/// What one run produced, as written to the artifact directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeArtifact {
    /// The difficulty band the fixture declared.
    pub difficulty: String,
    /// The episode as it ran.
    pub episode: CognitiveEpisode,
    /// The scan it ran from.
    pub scan: ScanResult,
    /// The compiled program.
    pub program: mm_metacog::CognitiveProgram,
    /// The ordered trace.
    pub trace: ProgramTrace,
    /// The one-line summary the other subcommands read.
    pub outcome: OutcomeSummary,
}

/// The summary of a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeSummary {
    /// The episode's ULID, lowercase.
    pub episode_id: String,
    /// The program's content-derived ULID, lowercase.
    pub program_id: String,
    /// The tier it ran at.
    pub tier: String,
    /// Why the loop stopped.
    pub stop: String,
    /// How many operations ran.
    pub executed: usize,
    /// The remaining uncertainty when it stopped.
    pub remaining_uncertainty: f64,
    /// The budget, spent side included.
    pub budget: CognitiveBudget,
    /// The program's content hash.
    pub content_hash: String,
    /// The lowered DAG's hash.
    pub dag_hash: String,
    /// How many nodes the DAG has.
    pub dag_nodes: usize,
}

/// One line of the gold program table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldProgram {
    /// The episode ULID.
    pub episode_id: String,
    /// The program ULID.
    pub program_id: String,
    /// The program's content hash.
    pub content_hash: String,
    /// The lowered DAG's hash.
    pub dag_hash: String,
    /// The tier it ran at.
    pub tier: String,
    /// How many operations the program holds.
    pub node_count: usize,
    /// How many of them ran.
    pub executed: usize,
}

/// The budget-audit thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    /// The most operations a `trivial` episode may run.
    pub trivial_max_ops: usize,
    /// The fewest a `hard` episode must run.
    pub hard_min_ops: usize,
    /// The budget dimensions the audit checks.
    pub budget_dimensions: Vec<String>,
}

/// Dispatch an `episode` subcommand.
pub async fn run(cfg: Config, command: EpisodeCommand) -> Result<ExitCode, MmError> {
    match command {
        EpisodeCommand::Run {
            input,
            tier,
            budget,
            out,
        } => run_corpus(cfg, &input, &tier, budget.as_deref(), &out).await,
        EpisodeCommand::Replay {
            episode,
            all,
            input,
            compare,
        } => replay(cfg, &input, episode.as_deref(), all, compare.as_deref()).await,
        EpisodeCommand::Verify {
            artifacts,
            gold,
            write_gold,
        } => verify(cfg, &artifacts, &gold, write_gold).await,
        EpisodeCommand::Value { gold_table, write } => value(&gold_table, write),
        EpisodeCommand::BudgetAudit {
            artifacts,
            thresholds,
        } => budget_audit(cfg, &artifacts, &thresholds).await,
        EpisodeCommand::Program {
            episode,
            artifacts,
            format,
        } => program(cfg, &artifacts, &episode, &format).await,
    }
}

// ------------------------------------------------------------------ fixtures ----

/// Read one fixture file, or every `*.json` under a directory, sorted by path.
fn load_fixtures(input: &Path) -> Result<Vec<(PathBuf, EpisodeFixture)>, MmError> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if input.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(input)
            .map_err(|e| MmError::Config(format!("cannot read {}: {e}", input.display())))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                // One level of difficulty subdirectories is supported, which is how
                // the corpus is laid out.
                let mut nested: Vec<PathBuf> = std::fs::read_dir(&path)
                    .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?
                    .filter_map(|entry| entry.ok().map(|e| e.path()))
                    .collect();
                nested.sort();
                paths.extend(nested);
            } else {
                paths.push(path);
            }
        }
    } else if input.is_file() {
        paths.push(input.to_path_buf());
    } else {
        return Err(MmError::Config(format!(
            "no such episode fixture: {}",
            input.display()
        )));
    }
    // A fixture is named for the episode it holds, so sibling files in the same
    // directory (`thresholds.json`, gold tables) are not mistaken for episodes —
    // and a genuinely malformed fixture still fails loudly, because its name *is*
    // an episode id.
    paths.retain(|p| p.extension().is_some_and(|e| e == "json") && fixture_ulid(p).is_some());
    if paths.is_empty() {
        return Err(MmError::Config(format!(
            "{} holds no episode fixtures",
            input.display()
        )));
    }
    let mut fixtures = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
        let fixture: EpisodeFixture = serde_json::from_str(&raw).map_err(|e| {
            MmError::Config(format!("{} is not an episode fixture: {e}", path.display()))
        })?;
        if let Some(named) = fixture_ulid(&path) {
            if named != fixture.episode.id {
                return Err(MmError::Config(format!(
                    "{} is named for {} but holds {}",
                    path.display(),
                    mm_core::ulid_string(&named),
                    mm_core::ulid_string(&fixture.episode.id)
                )));
            }
        }
        fixtures.push((path, fixture));
    }
    Ok(fixtures)
}

/// The episode ULID a fixture file is named for, when its stem is one.
fn fixture_ulid(path: &Path) -> Option<Ulid> {
    let stem = path.file_stem()?.to_str()?;
    if stem.len() != mm_core::ULID_LEN {
        return None;
    }
    mm_core::id::parse_ulid(stem).ok()
}

/// Parse `--tier`.
fn parse_tier(asked: &str) -> Result<Option<Tier>, MmError> {
    if asked.eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    Tier::parse(asked)
        .map(Some)
        .ok_or_else(|| MmError::Config(format!("--tier must be auto or 0..5, got {asked:?}")))
}

/// Apply a `key=value,key=value` budget override to an episode's budget.
fn apply_budget(spec: &str, base: CognitiveBudget) -> Result<CognitiveBudget, MmError> {
    let mut budget = base;
    let mut seen = 0usize;
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (key, value) = part.split_once('=').ok_or_else(|| {
            MmError::Config(format!("budget override {part:?} must be key=value"))
        })?;
        let number = |raw: &str| -> Result<f64, MmError> {
            raw.trim().parse::<f64>().map_err(|e| {
                MmError::Config(format!("budget override {key}={raw} is not a number: {e}"))
            })
        };
        match key.trim() {
            "max_ops" => budget.max_ops = number(value)? as u32,
            "max_llm_calls" => budget.max_llm_calls = number(value)? as u32,
            "max_cost" => budget.max_cost = number(value)?,
            "max_wall_ms" => budget.max_wall_ms = number(value)? as u64,
            other => {
                return Err(MmError::Config(format!(
                    "unknown budget dimension {other:?}; known: max_ops, max_llm_calls, \
                     max_cost, max_wall_ms"
                )))
            }
        }
        seen += 1;
    }
    if seen == 0 {
        return Err(MmError::Config(
            "--budget needs at least one key=value".to_string(),
        ));
    }
    budget.validate()?;
    Ok(budget)
}

/// The artifact file for an episode.
fn artifact_path(dir: &Path, episode: &Ulid) -> PathBuf {
    dir.join(format!("{}.json", mm_core::ulid_string(episode)))
}

async fn load_artifact(path: &Path) -> Result<EpisodeArtifact, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    serde_json::from_str(&raw)
        .map_err(|e| MmError::Config(format!("{} is not an artifact: {e}", path.display())))
}

/// Every artifact in a directory, ordered by file name.
fn artifact_paths(dir: &Path) -> Result<Vec<PathBuf>, MmError> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", dir.display())))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(MmError::Config(format!(
            "{} holds no episode artifacts; run `mm-cli episode run` first",
            dir.display()
        )));
    }
    Ok(paths)
}

// ----------------------------------------------------------------------- run ----

/// Run every fixture and write one artifact per episode.
async fn run_corpus(
    cfg: Config,
    input: &Path,
    tier: &str,
    budget: Option<&str>,
    out: &Path,
) -> Result<ExitCode, MmError> {
    let fixtures = load_fixtures(input)?;
    let pinned = parse_tier(tier)?;
    let kernel = Kernel::open(cfg, true).await?;
    kernel.load_ontology().await?;
    let store = SqliteTraceStore::new(
        Arc::new(kernel.sqlite.clone()) as Arc<dyn Tabular>,
        Arc::clone(&kernel.ids),
    );
    std::fs::create_dir_all(out)?;

    println!(
        "episode run: {} fixture(s) from {}",
        fixtures.len(),
        input.display()
    );
    for (path, fixture) in fixtures {
        let mut episode = fixture.episode.clone();
        if let Some(tier) = pinned {
            episode.tier = tier;
            episode.validate()?;
        }
        if let Some(spec) = budget {
            episode.budget = apply_budget(spec, episode.budget)?;
        }
        let path_out = artifact_path(out, &episode.id);
        let artifact = deliberate(
            &kernel,
            &store,
            fixture.difficulty.clone(),
            episode,
            fixture.scan,
        )
        .await?;
        write_json(&path_out, &artifact)?;
        println!(
            "  {} [{}] from {} tier {} ops {}/{} stop {}",
            artifact.outcome.episode_id,
            artifact.difficulty,
            path.display(),
            artifact.outcome.tier,
            artifact.outcome.executed,
            artifact.program.node_count(),
            artifact.outcome.stop,
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Run one episode, persist it, emit it into `/epistemic`, and summarise it.
async fn deliberate(
    kernel: &Kernel,
    store: &SqliteTraceStore,
    difficulty: String,
    episode: CognitiveEpisode,
    scan: ScanResult,
) -> Result<EpisodeArtifact, MmError> {
    let driver = Arc::new(MockScanDriver::new(scan.clone()).map_err(MmError::from)?);
    let controller = MinimumCognitionController::new(driver)
        .with_logger(Arc::clone(&kernel.logger))
        .with_compiler(Arc::new(mm_metacog::DefaultCompiler::new()));
    let outcome = controller.run(episode).await.map_err(MmError::from)?;

    store
        .persist(
            &outcome.episode,
            &outcome.scan,
            &outcome.program,
            &outcome.trace,
        )
        .await
        .map_err(MmError::from)?;

    let graph = kernel.graph()?;
    mm_metacog::rdf::emit_episode(graph, &outcome.episode, &outcome.scan)
        .await
        .map_err(MmError::from)?;
    mm_metacog::rdf::emit_program(graph, &outcome.program)
        .await
        .map_err(MmError::from)?;

    let catalog = ToolCatalog::from_program(&outcome.program);
    let document = lower::to_execution_document(&outcome.program, &catalog)
        .map_err(|e| MmError::from(mm_metacog::MetacogError::from(e)))?;
    kernel
        .logger
        .emit(
            mm_log::LogRecord::new(
                mm_log::Level::Info,
                codes::METACOG_TRACE_PERSIST,
                mm_metacog::TARGET,
            )
            .with_trace(outcome.episode_id)
            .with_field("program_id", mm_core::ulid_string(&outcome.program.id))
            .with_field("row_count", outcome.trace.rows.len())
            .with_field("first_seq", outcome.trace.first_seq())
            .with_field("last_seq", outcome.trace.last_seq()),
        )
        .await?;

    Ok(EpisodeArtifact {
        difficulty,
        outcome: OutcomeSummary {
            episode_id: mm_core::ulid_string(&outcome.episode_id),
            program_id: mm_core::ulid_string(&outcome.program.id),
            tier: outcome.tier.as_str().to_string(),
            stop: outcome.stop.as_str().to_string(),
            executed: outcome.executed,
            remaining_uncertainty: outcome.remaining_uncertainty,
            budget: outcome.budget,
            content_hash: outcome.program.content_hash(),
            dag_hash: document.dag_hash,
            dag_nodes: document.dag.node_count(),
        },
        episode: outcome.episode,
        scan: outcome.scan,
        program: outcome.program,
        trace: outcome.trace,
    })
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), MmError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| MmError::Internal(format!("cannot serialize {}: {e}", path.display())))?;
    std::fs::write(path, format!("{text}\n"))?;
    Ok(())
}

// -------------------------------------------------------------------- replay ----

/// Re-run from the fixtures and compare with what was written.
async fn replay(
    cfg: Config,
    input: &Path,
    episode: Option<&str>,
    all: bool,
    compare: Option<&Path>,
) -> Result<ExitCode, MmError> {
    let mut fixtures = load_fixtures(input)?;
    if let Some(wanted) = episode {
        let wanted = mm_core::id::parse_ulid(wanted)?;
        fixtures.retain(|(_, fixture)| fixture.episode.id == wanted);
        if fixtures.is_empty() {
            return Err(MmError::Config(format!(
                "no fixture for episode {}",
                mm_core::ulid_string(&wanted)
            )));
        }
    } else if !all {
        return Err(MmError::Config(
            "replay needs --episode <ulid> or --all".to_string(),
        ));
    }

    let kernel = Kernel::open(cfg, true).await?;
    let store = SqliteTraceStore::new(
        Arc::new(kernel.sqlite.clone()) as Arc<dyn Tabular>,
        Arc::clone(&kernel.ids),
    );
    let mut diffs = 0usize;
    for (_path, fixture) in fixtures {
        let id = mm_core::ulid_string(&fixture.episode.id);
        let regenerated = deliberate(
            &kernel,
            &store,
            fixture.difficulty.clone(),
            fixture.episode.clone(),
            fixture.scan.clone(),
        )
        .await?;
        match compare {
            Some(dir) => {
                let path = artifact_path(dir, &fixture.episode.id);
                let stored = load_artifact(&path).await?;
                let found = compare_artifacts(&stored, &regenerated);
                diffs += found.len();
                println!(
                    "  episode {id} diff_count {} ({} -> {})",
                    found.len(),
                    stored.difficulty,
                    path.display()
                );
                // A count alone says a replay diverged but not where, which is the
                // one thing an operator needs in order to fix it.
                for difference in &found {
                    println!("    {difference}");
                }
            }
            None => {
                println!(
                    "  episode {id} content_hash {} trace_hash {}",
                    regenerated.program.content_hash(),
                    regenerated.trace.content_hash()
                );
            }
        }
    }
    if diffs == 0 {
        println!("episode replay: replayed, diff_count 0");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("episode replay: {diffs} difference(s)");
        Ok(ExitCode::from(1))
    }
}

/// Which fields of two artifacts disagree, described one per line. The program,
/// every score and the trace must be byte-identical; a replayed program that merely
/// *works* is not a replay.
fn compare_artifacts(stored: &EpisodeArtifact, regenerated: &EpisodeArtifact) -> Vec<String> {
    let mut diffs: Vec<String> = Vec::new();
    let program = stored.program.canonical();
    let replayed = regenerated.program.canonical();
    if program != replayed {
        diffs.push(format!(
            "program canonical: stored {program}, replayed {replayed}"
        ));
    }
    if stored.program.content_hash() != regenerated.program.content_hash() {
        diffs.push(format!(
            "program content hash: stored {}, replayed {}",
            stored.program.content_hash(),
            regenerated.program.content_hash()
        ));
    }
    if stored.trace.canonical() != regenerated.trace.canonical() {
        diffs.push("trace canonical differs".to_string());
    }
    if stored.trace.content_hash() != regenerated.trace.content_hash() {
        diffs.push(format!(
            "trace content hash: stored {}, replayed {}",
            stored.trace.content_hash(),
            regenerated.trace.content_hash()
        ));
    }
    if stored.outcome.dag_hash != regenerated.outcome.dag_hash {
        diffs.push(format!(
            "dag_hash: stored {}, replayed {}",
            stored.outcome.dag_hash, regenerated.outcome.dag_hash
        ));
    }
    if stored.outcome.stop != regenerated.outcome.stop {
        diffs.push(format!(
            "stop: stored {}, replayed {}",
            stored.outcome.stop, regenerated.outcome.stop
        ));
    }
    if stored.outcome.executed != regenerated.outcome.executed {
        diffs.push(format!(
            "executed: stored {}, replayed {}",
            stored.outcome.executed, regenerated.outcome.executed
        ));
    }
    for (a, b) in stored.trace.rows.iter().zip(regenerated.trace.rows.iter()) {
        if a.value.canonical() != b.value.canonical() {
            // The full-precision debug form, not the nine-decimal rendering: a
            // score that differs only in its last bit is still a failed replay, and
            // the canonical form would hide it.
            diffs.push(format!(
                "row {} value: stored {:?}, replayed {:?}",
                a.seq, a.value, b.value
            ));
        }
    }
    if stored.trace.rows.len() != regenerated.trace.rows.len() {
        diffs.push(format!(
            "trace rows: stored {}, replayed {}",
            stored.trace.rows.len(),
            regenerated.trace.rows.len()
        ));
    }
    diffs
}

// -------------------------------------------------------------------- verify ----

/// Check every artifact against the gold table, or regenerate it.
async fn verify(
    cfg: Config,
    artifacts: &Path,
    gold: &Path,
    write_gold: bool,
) -> Result<ExitCode, MmError> {
    // The kernel is opened so the check runs against the same configuration the
    // artifacts were produced under; the artifacts themselves are read from disk.
    let _kernel = Kernel::open(cfg, true).await?;
    let paths = artifact_paths(artifacts)?;
    let mut rows: Vec<GoldProgram> = Vec::with_capacity(paths.len());
    let mut problems: Vec<String> = Vec::new();

    for path in &paths {
        let artifact = load_artifact(path).await?;
        let label = artifact.outcome.episode_id.clone();
        // The artifact's body must be the thing its own summary names. Both ids and
        // the content hash are derived from the program, so an artifact edited in
        // place disagrees with itself here — which is what stops a corrupted
        // benchmark from passing because only its header was compared.
        if mm_core::ulid_string(&artifact.program.id) != artifact.outcome.program_id {
            problems.push(format!(
                "{label}: the program names itself {}, the summary says {}",
                mm_core::ulid_string(&artifact.program.id),
                artifact.outcome.program_id
            ));
        }
        if mm_core::ulid_string(&artifact.episode.id) != artifact.outcome.episode_id {
            problems.push(format!(
                "{label}: the episode names itself {}, the summary says {}",
                mm_core::ulid_string(&artifact.episode.id),
                artifact.outcome.episode_id
            ));
        }
        let content_hash = artifact.program.content_hash();
        if content_hash != artifact.outcome.content_hash {
            problems.push(format!(
                "{label}: the program hashes to {content_hash}, the summary says {}",
                artifact.outcome.content_hash
            ));
        }
        // The DAG hash is the lowering's hash, so the only way to check the recorded
        // one is to lower the program again and compare.
        let catalog = ToolCatalog::from_program(&artifact.program);
        match lower::to_execution_document(&artifact.program, &catalog) {
            Ok(document) => {
                if document.dag_hash != artifact.outcome.dag_hash {
                    problems.push(format!(
                        "{label}: the program lowers to {}, the summary says {}",
                        document.dag_hash, artifact.outcome.dag_hash
                    ));
                }
                if document.dag.node_count() != artifact.outcome.dag_nodes {
                    problems.push(format!(
                        "{label}: the program lowers to {} nodes, the summary says {}",
                        document.dag.node_count(),
                        artifact.outcome.dag_nodes
                    ));
                }
            }
            Err(e) => problems.push(format!("{label}: the program does not lower: {e}")),
        }
        if let Err(e) = artifact.program.validate() {
            problems.push(format!("{label}: program is invalid: {e}"));
        }
        if let Err(e) = artifact.trace.validate() {
            problems.push(format!("{label}: trace is invalid: {e}"));
        }
        let executed = artifact.trace.executed();
        if executed != artifact.outcome.executed {
            problems.push(format!(
                "{label}: the summary says {} operations ran, the trace says {executed}",
                artifact.outcome.executed
            ));
        }
        for resource in BUDGET_RESOURCES {
            if artifact.outcome.budget.spent(resource) > artifact.outcome.budget.limit(resource) {
                problems.push(format!(
                    "{label}: {resource} spent {} exceeds the limit {}",
                    artifact.outcome.budget.spent(resource),
                    artifact.outcome.budget.limit(resource)
                ));
            }
        }
        // The program must emit into `/epistemic` and satisfy the emitter's own
        // contract: every operation names something.
        let quads = mm_metacog::rdf::program_quads(&artifact.program);
        if quads.is_empty() {
            problems.push(format!("{label}: the program emitted no RDF"));
        }
        for step in &artifact.program.steps {
            if step.op.targets().is_empty() {
                problems.push(format!(
                    "{label}: step {} ({}) targets nothing",
                    step.order,
                    step.op.tag()
                ));
            }
        }
        rows.push(GoldProgram {
            episode_id: artifact.outcome.episode_id.clone(),
            program_id: artifact.outcome.program_id.clone(),
            content_hash: artifact.outcome.content_hash.clone(),
            dag_hash: artifact.outcome.dag_hash.clone(),
            tier: artifact.outcome.tier.clone(),
            node_count: artifact.program.node_count(),
            executed,
        });
    }
    rows.sort_by(|a, b| a.episode_id.cmp(&b.episode_id));

    if problems.is_empty() {
        println!(
            "episode verify: {} artifact(s) valid, {} program(s) against {}",
            paths.len(),
            rows.len(),
            gold.display()
        );
    }

    if write_gold {
        std::fs::create_dir_all(gold)?;
        let path = gold.join("programs.jsonl");
        let mut text = String::new();
        for row in &rows {
            text.push_str(
                &serde_json::to_string(row).map_err(|e| {
                    MmError::Internal(format!("cannot serialize the gold row: {e}"))
                })?,
            );
            text.push('\n');
        }
        std::fs::write(&path, text)?;
        println!("episode verify: wrote {}", path.display());
        if problems.is_empty() {
            return Ok(ExitCode::SUCCESS);
        }
    }

    let gold_rows = read_gold(&gold.join("programs.jsonl"))?;
    if gold_rows.len() != rows.len() {
        problems.push(format!(
            "gold holds {} program(s); the artifacts hold {}",
            gold_rows.len(),
            rows.len()
        ));
    }
    let by_episode: BTreeMap<&str, &GoldProgram> = gold_rows
        .iter()
        .map(|row| (row.episode_id.as_str(), row))
        .collect();
    for row in &rows {
        match by_episode.get(row.episode_id.as_str()) {
            None => problems.push(format!("{}: no gold row", row.episode_id)),
            Some(expected) => {
                if expected != &row {
                    problems.push(format!(
                        "{}: gold says {expected:?}, the artifact says {row:?}",
                        row.episode_id
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        println!("episode verify: 0 difference(s), gold matches exactly");
        Ok(ExitCode::SUCCESS)
    } else {
        for problem in &problems {
            println!("  {problem}");
        }
        println!("episode verify: {} problem(s)", problems.len());
        Ok(ExitCode::from(1))
    }
}

fn read_gold(path: &Path) -> Result<Vec<GoldProgram>, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    let mut rows = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        rows.push(serde_json::from_str::<GoldProgram>(line).map_err(|e| {
            MmError::Config(format!(
                "{}:{} is not a gold row: {e}",
                path.display(),
                index + 1
            ))
        })?);
    }
    Ok(rows)
}

// --------------------------------------------------------------------- value ----

/// One row of the operation-value gold table.
struct ValueRow {
    name: String,
    eer: f64,
    importance: f64,
    p_change: f64,
    cost: f64,
    score: String,
}

/// Recompute the score column, or compare it exactly.
fn value(gold_table: &Path, write: bool) -> Result<ExitCode, MmError> {
    let raw = std::fs::read_to_string(gold_table)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", gold_table.display())))?;
    let mut rows = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if index == 0 || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() != 6 {
            return Err(MmError::Config(format!(
                "{}:{} needs 6 comma-separated fields, got {}",
                gold_table.display(),
                index + 1,
                fields.len()
            )));
        }
        let parse = |raw: &str, field: &str| -> Result<f64, MmError> {
            raw.trim().parse::<f64>().map_err(|e| {
                MmError::Config(format!(
                    "{}: {field} is not a number: {e}",
                    gold_table.display()
                ))
            })
        };
        rows.push(ValueRow {
            name: fields[0].trim().to_string(),
            eer: parse(fields[1], "eer")?,
            importance: parse(fields[2], "importance")?,
            p_change: parse(fields[3], "p_change")?,
            cost: parse(fields[4], "cost")?,
            score: fields[5].trim().to_string(),
        });
    }
    if rows.is_empty() {
        return Err(MmError::Config(format!(
            "{} holds no rows beyond the header",
            gold_table.display()
        )));
    }

    let mut mismatches = 0usize;
    for row in &rows {
        let value =
            mm_metacog::OperationValue::new(row.eer, row.importance, row.p_change, row.cost)
                .map_err(MmError::from)?;
        let computed = value.score_column();
        if write {
            println!("  {} score {}", row.name, computed);
        } else if computed != row.score {
            println!(
                "  {} score mismatch: gold {}, computed {computed}",
                row.name, row.score
            );
            mismatches += 1;
        }
    }

    if write {
        let mut text = String::from("name,eer,importance,p_change,cost,score\n");
        for row in &rows {
            let value =
                mm_metacog::OperationValue::new(row.eer, row.importance, row.p_change, row.cost)
                    .map_err(MmError::from)?;
            text.push_str(&format!(
                "{},{:.9},{:.9},{:.9},{:.9},{}\n",
                row.name,
                row.eer,
                row.importance,
                row.p_change,
                row.cost,
                value.score_column()
            ));
        }
        std::fs::write(gold_table, text)?;
        println!(
            "episode value: rewrote {} row(s) in {}",
            rows.len(),
            gold_table.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    if mismatches == 0 {
        println!(
            "episode value: {} row(s) match {} exactly",
            rows.len(),
            gold_table.display()
        );
        Ok(ExitCode::SUCCESS)
    } else {
        println!("episode value: {mismatches} mismatch(es)");
        Ok(ExitCode::from(1))
    }
}

// -------------------------------------------------------------- budget audit ----

/// Prove no artifact overspent, and that the trivial ones stayed small.
async fn budget_audit(
    cfg: Config,
    artifacts: &Path,
    thresholds: &Path,
) -> Result<ExitCode, MmError> {
    // Opening the kernel proves the configuration the audit is running against is
    // the one whose stores the artifacts describe.
    let _kernel = Kernel::open(cfg, true).await?;
    let raw = std::fs::read_to_string(thresholds)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", thresholds.display())))?;
    let thresholds: Thresholds = serde_json::from_str(&raw).map_err(|e| {
        MmError::Config(format!(
            "{} is not a thresholds file: {e}",
            thresholds.display()
        ))
    })?;

    let paths = artifact_paths(artifacts)?;
    let mut problems: Vec<String> = Vec::new();
    let mut over_budget = 0usize;
    let mut trivial_max = 0usize;
    let mut hard_min = usize::MAX;
    let mut trivial_seen = 0usize;
    let mut hard_seen = 0usize;

    for path in &paths {
        let artifact = load_artifact(path).await?;
        let label = artifact.outcome.episode_id.clone();
        for dimension in &thresholds.budget_dimensions {
            let resource = BudgetResource::parse(dimension).ok_or_else(|| {
                MmError::Config(format!("unknown budget dimension {dimension:?}"))
            })?;
            let spent = artifact.outcome.budget.spent(resource);
            let limit = artifact.outcome.budget.limit(resource);
            if spent > limit {
                over_budget += 1;
                problems.push(format!(
                    "{label}: {dimension} spent {spent} exceeds the limit {limit}"
                ));
            }
        }
        match artifact.difficulty.as_str() {
            "trivial" => {
                trivial_seen += 1;
                trivial_max = trivial_max.max(artifact.outcome.executed);
                if artifact.outcome.executed > thresholds.trivial_max_ops {
                    problems.push(format!(
                        "{label}: a trivial episode ran {} operations, more than the {} allowed",
                        artifact.outcome.executed, thresholds.trivial_max_ops
                    ));
                }
            }
            "hard" => {
                hard_seen += 1;
                hard_min = hard_min.min(artifact.outcome.executed);
                if artifact.outcome.executed < thresholds.hard_min_ops {
                    problems.push(format!(
                        "{label}: a hard episode ran only {} operations, fewer than the {} required",
                        artifact.outcome.executed, thresholds.hard_min_ops
                    ));
                }
            }
            // A `standard` episode is bounded only by its budget; the op thresholds
            // are how the gate separates under- and over-thinking, and a middle band
            // is neither.
            "standard" => {}
            other => problems.push(format!("{label}: unknown difficulty {other:?}")),
        }
    }

    println!(
        "episode budget-audit: {} artifact(s), {} over budget",
        paths.len(),
        over_budget
    );
    println!(
        "  trivial: {trivial_seen} episode(s), at most {trivial_max} ops (limit {})",
        thresholds.trivial_max_ops
    );
    if hard_seen > 0 {
        println!(
            "  hard: {hard_seen} episode(s), at least {hard_min} ops (floor {})",
            thresholds.hard_min_ops
        );
    }

    if problems.is_empty() {
        println!("episode budget-audit: 0 problem(s)");
        Ok(ExitCode::SUCCESS)
    } else {
        for problem in &problems {
            println!("  {problem}");
        }
        println!("episode budget-audit: {} problem(s)", problems.len());
        Ok(ExitCode::from(1))
    }
}

// ------------------------------------------------------------------- program ----

/// Print one artifact's program.
async fn program(
    cfg: Config,
    artifacts: &Path,
    episode: &str,
    format: &str,
) -> Result<ExitCode, MmError> {
    let _kernel = Kernel::open(cfg, false).await?;
    let wanted = mm_core::id::parse_ulid(episode)?;
    let path = artifact_path(artifacts, &wanted);
    let artifact = load_artifact(&path).await?;
    match format {
        "json" => {
            let text = serde_json::to_string_pretty(&artifact.program)
                .map_err(|e| MmError::Internal(format!("cannot serialize the program: {e}")))?;
            println!("{text}");
            Ok(ExitCode::SUCCESS)
        }
        "turtle" => {
            print!("{}", mm_metacog::rdf::program_turtle(&artifact.program));
            Ok(ExitCode::SUCCESS)
        }
        other => Err(MmError::Config(format!(
            "--format must be json or turtle, got {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tier_override_parses_and_refuses_nonsense() {
        assert_eq!(parse_tier("auto").unwrap(), None);
        assert_eq!(parse_tier("4").unwrap(), Some(Tier::T4));
        assert_eq!(parse_tier("T5").unwrap(), Some(Tier::T5));
        assert!(parse_tier("9").is_err());
        assert!(parse_tier("high").is_err());
    }

    #[test]
    fn a_budget_override_names_the_dimension_it_touches() {
        let base = CognitiveBudget::from_spec(4, 2, 0.5, 1000).unwrap();
        let changed = apply_budget("max_ops=12,max_cost=0.75", base).unwrap();
        assert_eq!(changed.max_ops, 12);
        assert!((changed.max_cost - 0.75).abs() < 1e-12);
        assert_eq!(changed.max_llm_calls, 2, "untouched dimensions stay");

        assert!(apply_budget("max_ops", base).is_err());
        assert!(apply_budget("max_ops=many", base).is_err());
        assert!(apply_budget("max_nonsense=1", base).is_err());
        assert!(apply_budget("", base).is_err());
    }

    #[test]
    fn an_artifact_path_is_the_lowercase_episode_ulid() {
        let id = Ulid::from_parts(1_700_000_000_000, 9);
        let path = artifact_path(Path::new("/tmp/x"), &id);
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            format!("{}.json", mm_core::ulid_string(&id))
        );
    }

    #[test]
    fn the_thresholds_file_round_trips() {
        let raw = r#"{"trivial_max_ops":4,"hard_min_ops":6,"budget_dimensions":["ops","cost"]}"#;
        let thresholds: Thresholds = serde_json::from_str(raw).unwrap();
        assert_eq!(thresholds.trivial_max_ops, 4);
        assert_eq!(thresholds.hard_min_ops, 6);
        assert_eq!(thresholds.budget_dimensions.len(), 2);
    }
}
