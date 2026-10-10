//! `mm-cli calibrate | meta | regression | changeset | sandbox | promote | reject |
//! budget | pi | audit` — Phase 11's operator surface.
//!
//! Every command here drives the same crate the runtime would. `calibrate` scores a
//! corpus through `mm-metaanalysis`'s calibrator and writes a `calibration_runs` row
//! through the same `record_run` the loop uses; `sandbox run` prepares the same git
//! worktree `mm-selfeng` prepares; `promote` calls `DeterministicGate::promote`, which
//! is the whole commit path — evaluate, record the decision, compute the version, append
//! the journal entry and stage the change set. A command that re-implemented any of those
//! steps would be a second opinion about the phase's central rules.
//!
//! Three properties shape the whole module:
//!
//! * **Deterministic and provider-free.** `meta analyze` uses the heuristic diagnoser, so
//!   the gate runs with no API key and no network; `pi run --offline` replays a recorded
//!   session instead of spawning the installed `pi` binary. `--offline` is also the
//!   default when no `pi` is on `PATH`, and the command says which mode it ran so nobody
//!   has to guess which one produced a session.
//! * **Machine-readable output.** Every command prints `key value` lines on stdout, so a
//!   gate can capture `changeset <ulid>` and pass it to the next command.
//! * **An assertion is an exit code, never a panic.** `--assert-*` flags exit non-zero
//!   with a written reason; nothing here panics on a breach, because a gate line that
//!   reads a panic as a failure cannot tell it from a crash.
//!
//! # The assertions, and what each one claims
//!
//! * `calibrate --assert-brier-le N` / `--assert-ece-le N`: the *calibrated* metric is at
//!   most `N`. `--assert-improves-baseline` additionally requires the calibrated Brier to
//!   beat the baseline — the reference file's when one sits beside the corpus, and the
//!   set's own uncalibrated Brier otherwise, which is stated in the output as
//!   `baseline_source` so the claim is never ambiguous.
//! * `meta analyze --assert-lessons-ge N`: at least `N` lessons were extracted.
//! * `regression run --assert-fail-before-pass-after NAME`: that case fails before the fix
//!   and passes after it.
//! * `pi run --assert-session-ingested`: the session row exists, its events are gapless,
//!   and the `/code` mirror accepted triples.
//! * `sandbox run --assert-isolated`: the write audit found nothing outside the sandbox.
//! * `promote --assert-reason-present`: the decision carries a written reason. A promotion
//!   has one too — "the gate found nothing to reject" — because the gate records the
//!   reason for both decisions rather than only the refusals.
//! * `budget show --assert-no-overspend`: no row has spent beyond its limit.
//! * `audit production-tree --assert-unmodified-outside-promotion`: nothing outside the
//!   sandbox changed since the cycle began.
//!
//! # What `audit production-tree` does *not* claim
//!
//! The audit is mtime-based and scoped: it asks whether anything outside the sandbox was
//! modified after the newest change set was created, and it treats `data/` (kernel state),
//! `bench/` (the test corpus, whose frozen harness rewrites the seeded case's own subject
//! as it runs) and `modules/` (where a promotion registers a module) as not-production.
//! It is a check on *this cycle*, not a proof that the tree matches its commit; the
//! Phase 2 lock file is what answers that question, and `codex verify` is what checks it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use clap::Subcommand;
use mm_core::{Config, MmError, Timestamp, Ulid};
use mm_metaanalysis::calibration::{
    load_labeled, load_reference, record_run, Calibrator, MetricsCalibrator,
    DEFAULT_TARGET_PRECISION,
};
use mm_metaanalysis::diagnosis::{analyze_fixture, MetaContext};
use mm_metaanalysis::ledger::{Prediction, PredictionLedger, PredictionOutcome};
use mm_metaanalysis::lessons;
use mm_metaanalysis::triggers;
use mm_selfeng::budget::{BudgetKind, BudgetLedger};
// `from_gap` is called fully qualified below, because the `changeset new` subcommand's own
// field is named `from_gap` (the path it was passed) and would shadow the function.
use mm_selfeng::changeset::{load_gap, BenchmarkId, ChangeSet, ChangeSetStatus, TestId};
use mm_selfeng::journal::EvolutionJournal;
use mm_selfeng::promotion::{DeterministicGate, EvidenceBundle, PromotionDecision};
use mm_selfeng::sandbox::{audit_writes, Sandbox, WorktreeSandbox};
use mm_selfeng::{benchmark, ChangeSetStore};

use crate::kernel::Kernel;

/// `mm-cli meta`.
#[derive(Subcommand, Debug)]
pub enum MetaCommand {
    /// Diagnose one episode and extract its lesson.
    Analyze {
        /// The episode fixture, e.g. `bench/episodes/seeded_failure_01.json`.
        #[arg(long, value_name = "PATH")]
        episode: PathBuf,
        /// Exit non-zero unless at least this many lessons were extracted.
        #[arg(long, value_name = "N")]
        assert_lessons_ge: Option<u32>,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Every trigger that has fired, ordered by priority.
    Queue {
        /// Print a single JSON array.
        #[arg(long)]
        json: bool,
    },
    /// The lessons the analyses yielded.
    Lessons {
        /// Print a single JSON array.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli changeset`.
#[derive(Subcommand, Debug)]
pub enum ChangeSetCommand {
    /// Assemble a typed change set from a gap.
    New {
        /// The gap fixture, e.g. `bench/gaps/gap_01.json`.
        #[arg(long, value_name = "PATH")]
        from_gap: PathBuf,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Read a change set back, with its status.
    Show {
        /// Its ULID.
        #[arg(value_name = "ULID")]
        id: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli sandbox`.
#[derive(Subcommand, Debug)]
pub enum SandboxCommand {
    /// Prepare a sandbox for a change set, apply it, and audit the filesystem.
    Run {
        /// The change set's ULID.
        #[arg(value_name = "ULID")]
        id: String,
        /// Write the change set's patches into the sandbox.
        #[arg(long)]
        apply: bool,
        /// Build inside the sandbox.
        #[arg(long)]
        build: bool,
        /// Test inside the sandbox.
        #[arg(long)]
        test: bool,
        /// Exit non-zero unless nothing outside the sandbox changed.
        #[arg(long)]
        assert_isolated: bool,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli budget`.
#[derive(Subcommand, Debug)]
pub enum BudgetCommand {
    /// Every budget row, with what is left.
    Show {
        /// Exit non-zero if any row has spent beyond its limit.
        #[arg(long)]
        assert_no_overspend: bool,
        /// Print a single JSON array.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli pi`.
#[derive(Subcommand, Debug)]
pub enum PiCli {
    /// Run the module-scaffold task, or replay a recorded session for it.
    Run {
        /// The task contract, e.g. `bench/pi/module_scaffold_01.json`.
        #[arg(long, value_name = "PATH")]
        task: PathBuf,
        /// Ingest the task's recorded session instead of spawning `pi`.
        ///
        /// This is the deterministic form of the command and the one the pass gate runs:
        /// it needs no provider, no network and no model, and the session it ingests is
        /// the recorded one the contract names.
        #[arg(long)]
        offline: bool,
        /// The provider a live session routes to, e.g. `google`.
        ///
        /// Required without `--offline`. The phase's own configuration names no real
        /// provider because a recorded session needs none, and a live session started
        /// with that placeholder fails inside `pi` with a message about the provider
        /// rather than about the flag that was not passed.
        #[arg(long, value_name = "NAME")]
        provider: Option<String>,
        /// The model a live session asks for, in Pi's `provider/id` form.
        #[arg(long, value_name = "MODEL", requires = "provider")]
        model: Option<String>,
        /// The `pi` binary to spawn.
        #[arg(long, value_name = "PATH", default_value = "pi")]
        binary: PathBuf,
        /// Exit non-zero unless the session was ingested and mirrored.
        #[arg(long)]
        assert_session_ingested: bool,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Ingest one recorded session file.
    Ingest {
        /// The session JSONL.
        #[arg(value_name = "PATH")]
        file: PathBuf,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli regression`.
#[derive(Subcommand, Debug)]
pub enum RegressionCommand {
    /// Run a suite, or one case of it.
    Run {
        /// The suite directory; defaults to `bench/regression`.
        #[arg(long, value_name = "DIR", default_value = "bench/regression")]
        suite: PathBuf,
        /// Exit non-zero unless this case reported both halves of its contract.
        #[arg(long, value_name = "NAME")]
        assert_fail_before_pass_after: Option<String>,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli audit`.
#[derive(Subcommand, Debug)]
pub enum AuditCommand {
    /// Report production writes outside a promotion.
    ProductionTree {
        /// Exit non-zero when anything outside the sandbox changed.
        #[arg(long)]
        assert_unmodified_outside_promotion: bool,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// Open the kernel with the graph store, which every command here needs.
async fn open(cfg: Config) -> Result<Kernel, MmError> {
    Kernel::open(cfg, true).await
}

/// A crate error as a kernel error.
///
/// Every refusal in this module is surfaced as `MmError::Internal`, which is the honest
/// classification: a calibrator that cannot read its corpus, a gate that cannot record
/// its decision and a sandbox that cannot prepare a worktree are all "the kernel could
/// not do what was asked", and the command's own message says which.
fn internal<E: std::fmt::Display>(error: E) -> MmError {
    MmError::Internal(error.to_string())
}

/// The repository this run is about, with the fallback every `codex` command uses.
///
/// A configuration loaded from `config/metamind.toml` by a *relative* path resolves its
/// root to the empty path, because the paths inside it stay relative to the working
/// directory; an empty root therefore means the repository, and [`Config::repo_root`] is
/// what names it. This matters more here than anywhere else: the sandbox prepares a git
/// worktree of that root, and `git worktree add` run with `current_dir("")` fails with
/// ENOENT — a directory that does not exist rather than a repository that is not one.
fn repo_root(cfg: &Config) -> PathBuf {
    if cfg.root.as_os_str().is_empty() {
        Config::repo_root()
    } else {
        cfg.root.clone()
    }
}

/// Resolve a path argument against the repository root.
///
/// The gate runs from the repository root and uses relative paths; a test may not, so an
/// absolute path is taken as given.
fn repo_path(cfg: &Config, relative: impl AsRef<Path>) -> PathBuf {
    let path = relative.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root(cfg).join(path)
    }
}

/// The sandbox root, `data/sandbox`, under the repository.
fn sandbox_root(cfg: &Config) -> PathBuf {
    repo_root(cfg).join("data").join("sandbox")
}

/// Parse a ULID argument.
fn parse_ulid(text: &str) -> Result<Ulid, MmError> {
    mm_core::id::parse_ulid(text).map_err(|e| MmError::Config(format!("invalid ULID: {e}")))
}

/// The instant the current improvement cycle began.
///
/// Read from the newest change set, because that is the record the cycle's first command
/// writes; a fallback to "now" is stated in the audit's own output rather than left
/// implicit, since an audit with no baseline is a vacuous audit and an operator should see
/// that it is.
async fn cycle_baseline(kernel: &Kernel) -> Result<(SystemTime, &'static str), MmError> {
    let newest: Option<i64> = sqlx::query_scalar(
        "SELECT CAST(strftime('%s', max(created_at)) AS INTEGER) FROM change_sets",
    )
    .fetch_one(kernel.sqlite.pool())
    .await
    .map_err(|e| MmError::Store(format!("cannot read the change-set baseline: {e}")))?;
    Ok(match newest {
        Some(seconds) if seconds > 0 => (
            SystemTime::UNIX_EPOCH + Duration::from_secs(seconds as u64),
            "newest-change-set",
        ),
        _ => (
            SystemTime::now(),
            "process-start (no change set is recorded)",
        ),
    })
}

/// `mm-cli calibrate`.
pub async fn calibrate(
    cfg: Config,
    bench: PathBuf,
    assert_brier_le: Option<f64>,
    assert_ece_le: Option<f64>,
    assert_improves_baseline: bool,
    record_ledger: bool,
    json: bool,
) -> Result<ExitCode, MmError> {
    let kernel = open(cfg.clone()).await?;
    let corpus = repo_path(&cfg, &bench);
    let labeled = load_labeled(&corpus).await.map_err(internal)?;
    if labeled.is_empty() {
        return Err(MmError::Config(format!(
            "{} holds no labeled points, so there is nothing to score",
            corpus.display()
        )));
    }

    // The baseline: the reference file's when one sits beside the corpus, otherwise the
    // set's own uncalibrated Brier. The second is computed by scoring the set with unit
    // temperatures, which is the definition of "uncalibrated" and therefore needs no
    // second implementation.
    let reference_path = corpus
        .parent()
        .map(|dir| dir.join("reference.json"))
        .unwrap_or_else(|| PathBuf::from("reference.json"));
    let (baseline_brier, baseline_source) = if reference_path.exists() {
        let reference = load_reference(&reference_path).await.map_err(internal)?;
        (reference.baseline_brier, "reference.json")
    } else {
        let unit: BTreeMap<String, f64> = mm_metaanalysis::classes_of(&labeled)
            .into_iter()
            .map(|class| (class, 1.0))
            .collect();
        let raw =
            MetricsCalibrator::with_temperatures(&labeled, 0.0, DEFAULT_TARGET_PRECISION, unit)
                .score(&labeled);
        (raw.brier, "uncalibrated")
    };

    let calibrator = MetricsCalibrator::fit(&labeled, baseline_brier, DEFAULT_TARGET_PRECISION);
    let report = calibrator.score(&labeled);
    let run_id = record_run(
        &kernel.sqlite,
        kernel.logger.as_ref(),
        kernel.ids.as_ref(),
        &report,
        baseline_source,
    )
    .await
    .map_err(internal)?;

    // Optional: stake every labeled point in the ledger and resolve it, which is what makes
    // the corpus a *prediction* history rather than only a scoring input. Off by default
    // because it writes one row per point and a re-run therefore grows the ledger.
    let mut ledger_rows = 0_u32;
    if record_ledger {
        let ledger = PredictionLedger::new(
            kernel.sqlite.clone(),
            kernel.logger.clone(),
            kernel.ids.clone(),
        );
        for point in &labeled {
            let mut prediction = Prediction::new(
                kernel.ids.next(),
                point.class.clone(),
                format!(
                    "the labeled {} point staked at {:.4} comes true",
                    point.class, point.predicted
                ),
                point.predicted,
                Duration::from_secs(86_400),
            );
            prediction.conditions = vec![format!("corpus {}", corpus.display())];
            let recorded = ledger.record(&prediction).await.map_err(internal)?;
            let outcome = PredictionOutcome {
                observed: point.label,
                resolved_at: Timestamp::now(),
                evidence: kernel.ids.next(),
            };
            ledger.resolve(recorded, &outcome).await.map_err(internal)?;
            ledger_rows += 1;
        }
    }

    let classes: Vec<String> = calibrator
        .classes()
        .into_iter()
        .map(|class| {
            let points: Vec<mm_metaanalysis::calibration::Labeled> = labeled
                .iter()
                .filter(|point| point.class == class)
                .cloned()
                .collect();
            let class_report = calibrator.score(&points);
            format!("{class}:{}", class_report.summary())
        })
        .collect();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "run_id": mm_core::ulid_string(&run_id),
                "corpus": corpus.display().to_string(),
                "baseline_source": baseline_source,
                "baseline_brier": baseline_brier,
                "report": report,
                "classes": classes,
                "ledger_rows": ledger_rows,
            }))
            .map_err(internal)?
        );
    } else {
        println!("run_id          {}", mm_core::ulid_string(&run_id));
        println!("corpus          {}", corpus.display());
        println!("baseline_source {baseline_source}");
        println!("baseline_brier  {baseline_brier:.9}");
        println!("n               {}", report.n);
        println!("brier           {:.9}", report.brier);
        println!("log_loss        {:.9}", report.log_loss);
        println!("ece             {:.9}", report.ece);
        println!("coverage        {:.6}", report.coverage);
        println!("selective_risk  {:.9}", report.selective_risk);
        for line in &classes {
            println!("class           {line}");
        }
        if record_ledger {
            println!("ledger_rows     {ledger_rows}");
        }
    }

    let mut failures: Vec<String> = Vec::new();
    if let Some(limit) = assert_brier_le {
        if report.brier > limit {
            failures.push(format!(
                "brier {:.9} exceeds the asserted {limit:.9}",
                report.brier
            ));
        }
    }
    if let Some(limit) = assert_ece_le {
        if report.ece > limit {
            failures.push(format!(
                "ece {:.9} exceeds the asserted {limit:.9}",
                report.ece
            ));
        }
    }
    if assert_improves_baseline && report.brier >= baseline_brier {
        failures.push(format!(
            "brier {:.9} does not improve on the baseline {:.9}",
            report.brier, baseline_brier
        ));
    }
    if failures.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        for failure in &failures {
            eprintln!("calibrate: {failure}");
        }
        Ok(ExitCode::from(1))
    }
}

/// `mm-cli meta`.
pub async fn meta(cfg: Config, command: MetaCommand) -> Result<ExitCode, MmError> {
    let kernel = open(cfg.clone()).await?;
    match command {
        MetaCommand::Analyze {
            episode,
            assert_lessons_ge,
            json,
        } => {
            let path = repo_path(&cfg, &episode);
            // An analysis spends the meta-analysis budget, and the debit happens before the
            // work: a budget that is only consulted afterwards is a report, not a limit.
            let budget = BudgetLedger::new(kernel.sqlite.clone(), kernel.logger.clone());
            budget
                .debit(BudgetKind::MetaAnalysis, 1.0, kernel.ids.next())
                .await
                .map_err(internal)?;
            let ctx = MetaContext::local(
                kernel.sqlite.clone(),
                kernel.logger.clone(),
                kernel.ids.clone(),
            );
            let analysis = analyze_fixture(&path, &ctx).await.map_err(internal)?;
            // The evidence count belongs to the episode, and the episode fixture is the only
            // record of it that is not the analysis row's own prose.
            let evidence = fixture_evidence_count(&path)?;
            let lesson = lessons::extract(&analysis, evidence, &ctx)
                .await
                .map_err(internal)?;
            let classes: Vec<&str> = analysis.error_classes.iter().map(|c| c.as_str()).collect();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "analysis_id": mm_core::ulid_string(&analysis.id),
                        "episode": mm_core::ulid_string(&analysis.episode),
                        "trigger": analysis.trigger.as_str(),
                        "diagnosis": analysis.diagnosis,
                        "error_classes": classes,
                        "recurrence": analysis.recurrence,
                        "impact": analysis.impact,
                        "lesson": {
                            "id": mm_core::ulid_string(&lesson.id),
                            "confidence": lesson.confidence,
                            "evidence_count": lesson.evidence_count,
                            "domains": lesson.domains,
                            "text": lesson.text,
                        },
                        "lessons": 1,
                    }))
                    .map_err(internal)?
                );
            } else {
                println!("analysis        {}", mm_core::ulid_string(&analysis.id));
                println!(
                    "episode         {}",
                    mm_core::ulid_string(&analysis.episode)
                );
                println!("trigger         {}", analysis.trigger.as_str());
                println!("error_classes   {}", classes.join(","));
                println!("recurrence      {:.6}", analysis.recurrence);
                println!("impact          {:.6}", analysis.impact);
                println!("diagnosis       {}", analysis.diagnosis);
                println!("lesson          {}", mm_core::ulid_string(&lesson.id));
                println!("lesson_conf     {:.6}", lesson.confidence);
                println!("evidence_count  {}", lesson.evidence_count);
                println!("lessons         1");
            }
            // One analysis yields one lesson in this build; the count is still asserted
            // rather than assumed, so a later change that produces zero fails the gate.
            let produced = 1_u32;
            if let Some(minimum) = assert_lessons_ge {
                if produced < minimum {
                    eprintln!("meta analyze: {produced} lesson(s) extracted, {minimum} asserted");
                    return Ok(ExitCode::from(1));
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        MetaCommand::Queue { json } => {
            let fired = triggers::fire(&kernel.sqlite, kernel.logger.as_ref(), Timestamp::now())
                .await
                .map_err(internal)?;
            if json {
                let rows: Vec<serde_json::Value> = fired
                    .iter()
                    .map(|trigger| {
                        serde_json::json!({
                            "trigger": trigger.trigger.as_str(),
                            "episode": mm_core::ulid_string(&trigger.episode),
                            "score": trigger.score(),
                            "priority": {
                                "novelty": trigger.priority.novelty,
                                "failure": trigger.priority.failure,
                                "impact": trigger.priority.impact,
                                "recurrence": trigger.priority.recurrence,
                                "uncertainty": trigger.priority.uncertainty,
                            },
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows).map_err(internal)?);
            } else {
                for trigger in &fired {
                    println!(
                        "trigger         {} score {:.6} episode {}",
                        trigger.trigger.as_str(),
                        trigger.score(),
                        mm_core::ulid_string(&trigger.episode)
                    );
                }
                println!("count           {}", fired.len());
            }
            Ok(ExitCode::SUCCESS)
        }
        MetaCommand::Lessons { json } => {
            let found = lessons::list(&kernel.sqlite).await.map_err(internal)?;
            if json {
                // Rendered field by field rather than derived: `Lesson` is a domain type
                // whose shape is the kernel's business, and a `Serialize` derive on it
                // would make the wire format of a lesson a second, silent API.
                let rows: Vec<serde_json::Value> = found
                    .iter()
                    .map(|lesson| {
                        serde_json::json!({
                            "lesson": mm_core::ulid_string(&lesson.id),
                            "text": lesson.text,
                            "confidence": lesson.confidence,
                            "evidence_count": lesson.evidence_count,
                            "domains": lesson.domains,
                            "policy_effect": lesson.policy_effect,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows).map_err(internal)?);
            } else {
                for lesson in &found {
                    println!(
                        "lesson          {} confidence {:.6} evidence {}",
                        mm_core::ulid_string(&lesson.id),
                        lesson.confidence,
                        lesson.evidence_count
                    );
                }
                println!("count           {}", found.len());
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// How many evidence ULIDs an episode fixture lists.
fn fixture_evidence_count(path: &Path) -> Result<u32, MmError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| MmError::Config(format!("{} is not JSON: {e}", path.display())))?;
    Ok(value
        .get("evidence")
        .and_then(serde_json::Value::as_array)
        .map_or(0, |items| items.len() as u32))
}

/// `mm-cli changeset`.
pub async fn changeset(cfg: Config, command: ChangeSetCommand) -> Result<ExitCode, MmError> {
    let kernel = open(cfg.clone()).await?;
    let store = ChangeSetStore::new(
        kernel.sqlite.clone(),
        kernel.logger.clone(),
        kernel.ids.clone(),
    );
    match command {
        ChangeSetCommand::New { from_gap, json } => {
            let gap = load_gap(&repo_path(&cfg, &from_gap)).map_err(internal)?;
            // The change set carries one patch, because an empty candidate is refused at
            // validation: the sandbox is asked to materialise *something*, and what it
            // writes is a note derived from the gap, inside the sandbox only.
            let note = format!(
                "# {}\n\n{}\n\nEvidence: {}\n",
                gap.statement,
                gap.kind,
                gap.evidence
                    .iter()
                    .map(mm_core::ulid_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let patch =
                mm_selfeng::changeset::Patch::create(gap.target_path.join("SESSION.md"), note);
            // Fully qualified: the clap variant's field is also called `from_gap` (the
            // path it was given), and a bare name here would call that `PathBuf`.
            let change_set = mm_selfeng::changeset::from_gap(
                &gap,
                vec![patch],
                vec![TestId::new("bench/regression/seeded_bug_01")],
                vec![BenchmarkId::new("promotion_bench")],
                kernel.ids.as_ref(),
            )
            .map_err(internal)?;
            store.create(&change_set).await.map_err(internal)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "changeset": mm_core::ulid_string(&change_set.id),
                        "status": ChangeSetStatus::Draft.as_str(),
                        "reason": change_set.reason,
                        "hypothesis": change_set.hypothesis,
                        "artifacts": change_set.artifact_count(),
                        "paths": change_set.paths(),
                        "rollback": change_set.rollback.steps,
                    }))
                    .map_err(internal)?
                );
            } else {
                println!("changeset       {}", mm_core::ulid_string(&change_set.id));
                println!("status          {}", ChangeSetStatus::Draft.as_str());
                println!("artifacts       {}", change_set.artifact_count());
                println!("reason          {}", change_set.reason);
                // `lines()` already borrows the string, so it is consumed by value here:
                // an iterator is, and `&Lines` is not itself iterable.
                for topic in change_set.hypothesis.lines() {
                    println!("hypothesis      {topic}");
                }
                for step in &change_set.rollback.steps {
                    println!("rollback        {step}");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        ChangeSetCommand::Show { id, json } => {
            let id = parse_ulid(&id)?;
            let change_set = store.get(id).await.map_err(internal)?.ok_or_else(|| {
                MmError::Config(format!("no change set {}", mm_core::ulid_string(&id)))
            })?;
            let status = store.status(id).await.map_err(internal)?;
            let status = status.unwrap_or(ChangeSetStatus::Draft);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "changeset": mm_core::ulid_string(&id),
                        "status": status.as_str(),
                        "change_set": change_set,
                    }))
                    .map_err(internal)?
                );
            } else {
                println!("changeset       {}", mm_core::ulid_string(&id));
                println!("status          {}", status.as_str());
                println!("reason          {}", change_set.reason);
                println!("artifacts       {}", change_set.artifact_count());
                for path in change_set.paths() {
                    println!("path            {}", path.display());
                }
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// `mm-cli sandbox`.
pub async fn sandbox(cfg: Config, command: SandboxCommand) -> Result<ExitCode, MmError> {
    let SandboxCommand::Run {
        id,
        apply,
        build,
        test,
        assert_isolated,
        json,
    } = command;
    let kernel = open(cfg.clone()).await?;
    let id = parse_ulid(&id)?;
    let change_sets = ChangeSetStore::new(
        kernel.sqlite.clone(),
        kernel.logger.clone(),
        kernel.ids.clone(),
    );
    let change_set = change_sets
        .get(id)
        .await
        .map_err(internal)?
        .ok_or_else(|| MmError::Config(format!("no change set {}", mm_core::ulid_string(&id))))?;

    let sandbox = WorktreeSandbox::new(
        repo_root(&cfg),
        sandbox_root(&cfg),
        kernel.logger.clone(),
        kernel.ids.clone(),
    );
    let dir = sandbox.prepare(&change_set).await.map_err(internal)?;
    change_sets
        .set_status(change_set.id, ChangeSetStatus::Sandboxed)
        .await
        .map_err(internal)?;

    let mut touched: Vec<String> = Vec::new();
    if apply {
        touched = sandbox
            .apply(&dir, &change_set)
            .await
            .map_err(internal)?
            .into_iter()
            .map(|path| path.display().to_string())
            .collect();
    }
    let mut build_line = None;
    if build {
        let result = sandbox.build(&dir).await.map_err(internal)?;
        change_sets
            .set_status(change_set.id, ChangeSetStatus::Built)
            .await
            .map_err(internal)?;
        build_line = Some(format!(
            "exit_code {} warnings {}",
            result.exit_code, result.warnings
        ));
    }
    let mut test_line = None;
    if test {
        let result = sandbox.test(&dir).await.map_err(internal)?;
        change_sets
            .set_status(change_set.id, ChangeSetStatus::Tested)
            .await
            .map_err(internal)?;
        test_line = Some(format!(
            "exit_code {} passed {} failed {}",
            result.exit_code, result.passed, result.failed
        ));
    }

    let (since, baseline) = cycle_baseline(&kernel).await?;
    let audit = audit_writes(&repo_root(&cfg), &sandbox_root(&cfg), since);
    let outside = production_writes(&cfg, &audit.outside);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "changeset": mm_core::ulid_string(&id),
                "sandbox": dir.root.display().to_string(),
                "touched": touched,
                "build": build_line,
                "test": test_line,
                "baseline": baseline,
                "outside": outside.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "audit_complete": audit.complete,
            }))
            .map_err(internal)?
        );
    } else {
        println!("changeset       {}", mm_core::ulid_string(&id));
        println!("sandbox         {}", dir.root.display());
        println!("baseline        {baseline}");
        for path in &touched {
            println!("touched         {path}");
        }
        if let Some(line) = &build_line {
            println!("build           {line}");
        }
        if let Some(line) = &test_line {
            println!("test            {line}");
        }
        println!("audit_complete  {}", audit.complete);
        for path in &outside {
            println!("outside         {}", path.display());
        }
        println!("outside_count   {}", outside.len());
    }

    if assert_isolated && (!outside.is_empty() || !audit.complete) {
        eprintln!(
            "sandbox run: the audit reported {} production write(s) and completeness {}",
            outside.len(),
            audit.complete
        );
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// The paths a write audit reports that count as *production*.
///
/// Three trees are excluded, each for a stated reason: `data/` is kernel state (the
/// database, the logs, the sandbox itself); `bench/` is the test corpus, and the frozen
/// promotion harness legitimately rewrites the seeded case's own subject file as it runs;
/// and `modules/` is where a promotion registers a module, which is the one write the
/// phase allows outside the sandbox.
fn production_writes(cfg: &Config, outside: &[PathBuf]) -> Vec<PathBuf> {
    let root = repo_root(cfg);
    let skip = [
        root.join("data"),
        root.join("bench"),
        root.join("modules"),
        root.join("target"),
    ];
    outside
        .iter()
        .filter(|path| !skip.iter().any(|root| path.starts_with(root)))
        .cloned()
        .collect()
}

/// `mm-cli promote`.
pub async fn promote(
    cfg: Config,
    id: String,
    assert_reason_present: bool,
    json: bool,
) -> Result<ExitCode, MmError> {
    let kernel = open(cfg.clone()).await?;
    let id = parse_ulid(&id)?;
    let change_sets = ChangeSetStore::new(
        kernel.sqlite.clone(),
        kernel.logger.clone(),
        kernel.ids.clone(),
    );
    let change_set = change_sets
        .get(id)
        .await
        .map_err(internal)?
        .ok_or_else(|| MmError::Config(format!("no change set {}", mm_core::ulid_string(&id))))?;
    let evidence = evidence_for(&cfg, &kernel, &change_set).await?;

    let gate = DeterministicGate::new();
    let journal = EvolutionJournal::new(
        kernel.sqlite.clone(),
        kernel.logger.clone(),
        kernel.ids.clone(),
    );
    // Promoting spends the evolution budget, and it spends it *before* deciding: a budget
    // consulted only after a decision would make the cap a report. A denial is a refusal
    // to promote, which is the phase's "exhausted means denied" rule.
    let budget = BudgetLedger::new(kernel.sqlite.clone(), kernel.logger.clone());
    budget
        .debit(BudgetKind::Evolution, 1.0, change_set.id)
        .await
        .map_err(internal)?;

    let outcome = gate
        .promote(&change_set, &evidence, &change_sets, &journal)
        .await
        .map_err(internal)?;
    let reason = outcome
        .decision
        .reason()
        .unwrap_or("the gate found nothing to reject")
        .to_string();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "changeset": mm_core::ulid_string(&change_set.id),
                "promotion": mm_core::ulid_string(&outcome.id),
                "decision": outcome.decision.as_str(),
                "reason": reason,
                "self_version": outcome.self_version,
                "status": outcome.decision.resulting_status().as_str(),
            }))
            .map_err(internal)?
        );
    } else {
        println!("changeset       {}", mm_core::ulid_string(&change_set.id));
        println!("promotion       {}", mm_core::ulid_string(&outcome.id));
        println!("decision        {}", outcome.decision.as_str());
        println!("reason          {reason}");
        println!("self_version    {}", outcome.self_version);
        println!(
            "status          {}",
            outcome.decision.resulting_status().as_str()
        );
    }

    if assert_reason_present && reason.trim().is_empty() {
        eprintln!("promote: the decision carries no written reason");
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// `mm-cli reject`.
///
/// The gate's own path is `promote`, which evaluates the six rules first. An operator's
/// rejection is a different thing — a human saying no, with their reason — so it records
/// the decision, appends the journal entry and stages the change set without pretending to
/// have consulted the rules.
pub async fn reject(
    cfg: Config,
    id: String,
    reason: String,
    json: bool,
) -> Result<ExitCode, MmError> {
    if reason.trim().is_empty() {
        return Err(MmError::Config(
            "a rejection without a reason is not a decision: pass --reason".to_string(),
        ));
    }
    let kernel = open(cfg.clone()).await?;
    let id = parse_ulid(&id)?;
    let change_sets = ChangeSetStore::new(
        kernel.sqlite.clone(),
        kernel.logger.clone(),
        kernel.ids.clone(),
    );
    let change_set = change_sets
        .get(id)
        .await
        .map_err(internal)?
        .ok_or_else(|| MmError::Config(format!("no change set {}", mm_core::ulid_string(&id))))?;
    let evidence = evidence_for(&cfg, &kernel, &change_set).await?;
    let gate = DeterministicGate::new();
    let journal = EvolutionJournal::new(
        kernel.sqlite.clone(),
        kernel.logger.clone(),
        kernel.ids.clone(),
    );
    let decision = PromotionDecision::Reject {
        reason: reason.clone(),
    };
    let promotion_id = gate
        .record_decision(
            &change_set,
            &decision,
            &evidence,
            &kernel.sqlite,
            kernel.logger.as_ref(),
            kernel.ids.as_ref(),
        )
        .await
        .map_err(internal)?;
    let self_version = journal
        .next_version("reject", change_set.id)
        .await
        .map_err(internal)?;
    let mut event = mm_selfeng::journal::EvolutionEvent::new(
        kernel.ids.as_ref(),
        change_set.reason.clone(),
        change_set.hypothesis.clone(),
        reason.clone(),
        "reject",
    )
    .with_changeset(change_set.id);
    event.self_version = self_version.clone();
    event.evidence_json = evidence.evidence_json().map_err(internal)?;
    journal.append(&event).await.map_err(internal)?;
    change_sets
        .set_status(change_set.id, ChangeSetStatus::Rejected)
        .await
        .map_err(internal)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "changeset": mm_core::ulid_string(&change_set.id),
                "promotion": mm_core::ulid_string(&promotion_id),
                "decision": "reject",
                "reason": reason,
                "self_version": self_version,
            }))
            .map_err(internal)?
        );
    } else {
        println!("changeset       {}", mm_core::ulid_string(&change_set.id));
        println!("promotion       {}", mm_core::ulid_string(&promotion_id));
        println!("decision        reject");
        println!("reason          {reason}");
        println!("self_version    {self_version}");
    }
    Ok(ExitCode::SUCCESS)
}

/// The evidence a decision is recorded against.
///
/// The regression suite is run now, the benchmark comes from `bench/promotion`, and the
/// risk the gate weighs is what is left of the evolution budget: a change that would cost
/// more than the budget holds is refused by rule 3, and the number the gate compares with
/// is the number the budget will actually allow.
async fn evidence_for(
    cfg: &Config,
    kernel: &Kernel,
    change_set: &ChangeSet,
) -> Result<EvidenceBundle, MmError> {
    let suite = repo_path(cfg, "bench/regression");
    let regression = mm_mistakes::run_suite(&suite).await.map_err(internal)?;
    let budget = BudgetLedger::new(kernel.sqlite.clone(), kernel.logger.clone());
    let remaining = budget
        .remaining(BudgetKind::Evolution)
        .await
        .map_err(internal)?;
    let mut bundle = EvidenceBundle::new(change_set.id, regression).with_risk(0.0, remaining);

    let bench_file = repo_path(cfg, "bench/promotion/promotion_bench.json");
    if bench_file.exists() {
        let specs = benchmark::load_specs(&bench_file).map_err(internal)?;
        let spec = specs.first().ok_or_else(|| {
            MmError::Config(format!("{} lists no benchmark", bench_file.display()))
        })?;
        // The spec's command is relative to the repository, so it runs from the root: the
        // harness is the *frozen* one, not the candidate's copy.
        let result = benchmark::run_benchmark(spec, &repo_root(cfg), kernel.logger.as_ref())
            .await
            .map_err(internal)?;
        bundle = bundle.with_bench(result);
    }

    // Rule 6's own verdict, computed here rather than by the gate: the gate takes the
    // answer, and the policy is what decides it.
    let gate = DeterministicGate::new();
    if gate.policy().touches_immutable(change_set).is_some() {
        bundle = bundle.touching_immutable();
    }
    Ok(bundle)
}

/// `mm-cli budget`.
pub async fn budget(cfg: Config, command: BudgetCommand) -> Result<ExitCode, MmError> {
    let BudgetCommand::Show {
        assert_no_overspend,
        json,
    } = command;
    let kernel = open(cfg).await?;
    let ledger = BudgetLedger::new(kernel.sqlite.clone(), kernel.logger.clone());
    let rows = ledger.show().await.map_err(internal)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&rows).map_err(internal)?);
    } else {
        for row in &rows {
            println!(
                "budget          {} {} limit {:.6} spent {:.6} remaining {:.6}",
                row.kind.as_str(),
                row.period,
                row.limit,
                row.spent,
                row.remaining()
            );
        }
        println!("rows            {}", rows.len());
    }
    let overspent: Vec<String> = rows
        .iter()
        .filter(|row| row.spent > row.limit)
        .map(|row| format!("{} {}", row.kind.as_str(), row.period))
        .collect();
    println!("overspent       {}", overspent.len());
    if assert_no_overspend && !overspent.is_empty() {
        eprintln!(
            "budget show: {} row(s) spent beyond their limit: {}",
            overspent.len(),
            overspent.join(", ")
        );
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// `mm-cli regression`.
pub async fn regression(cfg: Config, command: RegressionCommand) -> Result<ExitCode, MmError> {
    let RegressionCommand::Run {
        suite,
        assert_fail_before_pass_after,
        json,
    } = command;
    let suite = repo_path(&cfg, &suite);
    let report = mm_mistakes::run_suite(&suite).await.map_err(internal)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(internal)?
        );
    } else {
        for case in &report.cases {
            println!(
                "case            {} fails_before {} passes_after {} ({})",
                case.name, case.fails_before, case.passes_after, case.detail
            );
        }
        println!("cases           {}", report.cases.len());
        println!("failed          {}", report.failed);
    }
    if let Some(name) = assert_fail_before_pass_after {
        let case = report
            .cases
            .iter()
            .find(|case| case.name == name)
            .ok_or_else(|| {
                MmError::Config(format!(
                    "no case named {name:?} in {} (cases: {})",
                    suite.display(),
                    report
                        .cases
                        .iter()
                        .map(|case| case.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
        if !case.passed() {
            eprintln!(
                "regression run: {name} did not hold its contract: fails_before {} passes_after {} ({})",
                case.fails_before, case.passes_after, case.detail
            );
            return Ok(ExitCode::from(1));
        }
    }
    Ok(if report.failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// A Pi task contract.
#[derive(Debug, Clone, serde::Deserialize)]
struct PiTask {
    /// Its identifier.
    task_id: String,
    /// What the session is asked to do.
    goal: String,
    /// The skill the session follows.
    #[serde(default)]
    skill: Option<String>,
    /// The prompt template, when the contract names one.
    #[serde(default)]
    prompt_file: Option<String>,
    /// The paths the session may write.
    #[serde(default)]
    allowed_paths: Vec<String>,
    /// The recorded session to replay.
    #[serde(default)]
    session_file: Option<String>,
}

/// `mm-cli pi`.
pub async fn pi(cfg: Config, command: PiCli) -> Result<ExitCode, MmError> {
    let kernel = open(cfg.clone()).await?;
    match command {
        PiCli::Ingest { file, json } => {
            let path = repo_path(&cfg, &file);
            let (session, triples) = ingest_and_mirror(&kernel, &path, None).await?;
            report_session(&session, triples, json)?;
            Ok(ExitCode::SUCCESS)
        }
        PiCli::Run {
            task,
            offline,
            provider,
            model,
            binary,
            assert_session_ingested,
            json,
        } => {
            let path = repo_path(&cfg, &task);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
            let contract: PiTask = serde_json::from_str(&text).map_err(|e| {
                MmError::Config(format!("{} is not a Pi task: {e}", path.display()))
            })?;
            let session_path = contract
                .session_file
                .as_ref()
                .map(|relative| repo_path(&cfg, relative));

            // Offline is the default when there is no binary to run, and the mode is
            // reported either way: an operator must never have to guess whether a session
            // row came from a real agent or from a recording.
            let have_binary = pi_binary_available();
            let replay = offline || !have_binary;
            let prompt = pi_prompt(&cfg, &contract)?;

            let mode = if replay {
                "recorded"
            } else {
                let (Some(provider), Some(model)) = (provider, model) else {
                    return Err(MmError::Config(
                        "a live run needs --provider and --model; pass --offline to replay the \
                         task's recorded session instead"
                            .to_string(),
                    ));
                };
                let mut config = mm_pi::PiConfig::default_for(&repo_root(&cfg));
                config.provider = provider;
                config.model = model;
                config.binary = binary;
                let mut client = mm_pi::PiClient::spawn(&config)
                    .await
                    .map_err(internal)?
                    .with_logger(kernel.logger.clone());
                client
                    .prompt(&contract.task_id, &prompt)
                    .await
                    .map_err(internal)?;
                // Drain until the response that answers the prompt, or the stream ends.
                // The session's records are ingested from its file afterwards, which is
                // what makes the ingestion independent of this loop.
                let mut answered = false;
                for _ in 0..10_000 {
                    match client.next_event().await.map_err(internal)? {
                        None => break,
                        Some(event) => {
                            if event.is_response_to(&contract.task_id, "prompt") {
                                answered = true;
                                break;
                            }
                        }
                    }
                }
                if !answered {
                    eprintln!(
                        "pi run: the session ended without answering {}",
                        contract.task_id
                    );
                    return Ok(ExitCode::from(1));
                }
                "live"
            };

            let Some(session_path) = session_path else {
                return Err(MmError::Config(format!(
                    "{} names no session_file, so there is nothing to ingest",
                    path.display()
                )));
            };
            if !session_path.exists() {
                return Err(MmError::Config(format!(
                    "the session file {} is missing",
                    session_path.display()
                )));
            }
            let (session, triples) = ingest_and_mirror(&kernel, &session_path, None).await?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "task_id": contract.task_id,
                        "mode": mode,
                        "goal": contract.goal,
                        "skill": contract.skill,
                        "allowed_paths": contract.allowed_paths,
                        "session": mm_core::ulid_string(&session.session_id),
                        "session_file": session.session_file.display().to_string(),
                        "events": session.event_count,
                        "edits": session.edits.len(),
                        "tool_calls": session.tool_calls.len(),
                        "triples": triples,
                        "turtle_hash": mm_core::content_hash(session.canonical_turtle().as_bytes()),
                    }))
                    .map_err(internal)?
                );
            } else {
                println!("task_id         {}", contract.task_id);
                println!("mode            {mode}");
                println!(
                    "session         {}",
                    mm_core::ulid_string(&session.session_id)
                );
                println!("session_file    {}", session.session_file.display());
                println!("events          {}", session.event_count);
                println!("edits           {}", session.edits.len());
                println!("tool_calls      {}", session.tool_calls.len());
                println!("triples         {triples}");
                println!(
                    "turtle_hash     {}",
                    mm_core::content_hash(session.canonical_turtle().as_bytes())
                );
            }

            if assert_session_ingested {
                let mismatch = session_ingest_problem(&kernel, &session, triples).await?;
                if let Some(problem) = mismatch {
                    eprintln!("pi run: {problem}");
                    return Ok(ExitCode::from(1));
                }
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Ingest a recorded session and mirror it into `/code`.
async fn ingest_and_mirror(
    kernel: &Kernel,
    path: &Path,
    changeset: Option<Ulid>,
) -> Result<(mm_pi::PiSessionGraph, usize), MmError> {
    let session = mm_pi::ingest_session_for(
        path,
        &kernel.sqlite,
        kernel.logger.as_ref(),
        kernel.ids.as_ref(),
        changeset,
    )
    .await
    .map_err(internal)?;
    let triples = session.mirror(kernel.graph()?).await.map_err(internal)?;
    Ok((session, triples))
}

/// Everything `--assert-session-ingested` checks.
async fn session_ingest_problem(
    kernel: &Kernel,
    session: &mm_pi::PiSessionGraph,
    triples: usize,
) -> Result<Option<String>, MmError> {
    let id = mm_core::ulid_string(&session.session_id);
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM pi_sessions WHERE id = ?")
        .bind(&id)
        .fetch_one(kernel.sqlite.pool())
        .await
        .map_err(|e| MmError::Store(format!("cannot read pi_sessions: {e}")))?;
    if sessions == 0 {
        return Ok(Some(format!("no pi_sessions row for {id}")));
    }
    // Gapless: the count and the maximum are the two halves of "1..=n with no holes".
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM pi_events WHERE session_ulid = ?")
        .bind(&id)
        .fetch_one(kernel.sqlite.pool())
        .await
        .map_err(|e| MmError::Store(format!("cannot read pi_events: {e}")))?;
    let highest: Option<i64> =
        sqlx::query_scalar("SELECT max(seq) FROM pi_events WHERE session_ulid = ?")
            .bind(&id)
            .fetch_one(kernel.sqlite.pool())
            .await
            .map_err(|e| MmError::Store(format!("cannot read pi_events: {e}")))?;
    if events != i64::from(session.event_count) {
        return Ok(Some(format!(
            "pi_events holds {events} row(s) for {id} but the session has {} record(s)",
            session.event_count
        )));
    }
    if highest != Some(events) {
        return Ok(Some(format!(
            "pi_events is not gapless for {id}: {events} row(s), highest seq {highest:?}"
        )));
    }
    if triples == 0 {
        return Ok(Some(format!(
            "the /code mirror accepted no triples for {id}"
        )));
    }
    Ok(None)
}

/// Print a session's rows and mirror counts.
fn report_session(
    session: &mm_pi::PiSessionGraph,
    triples: usize,
    json: bool,
) -> Result<(), MmError> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "session": mm_core::ulid_string(&session.session_id),
                "session_file": session.session_file.display().to_string(),
                "events": session.event_count,
                "edits": session.edits.len(),
                "tool_calls": session.tool_calls.len(),
                "triples": triples,
                "turtle_hash": mm_core::content_hash(session.canonical_turtle().as_bytes()),
            }))
            .map_err(internal)?
        );
    } else {
        println!(
            "session         {}",
            mm_core::ulid_string(&session.session_id)
        );
        println!("session_file    {}", session.session_file.display());
        println!("events          {}", session.event_count);
        println!("edits           {}", session.edits.len());
        println!("tool_calls      {}", session.tool_calls.len());
        println!("triples         {triples}");
        println!(
            "turtle_hash     {}",
            mm_core::content_hash(session.canonical_turtle().as_bytes())
        );
    }
    Ok(())
}

/// True when a `pi` binary is on `PATH`.
///
/// Checked by walking `PATH` rather than by spawning: `spawn` with a missing binary is a
/// refusal this command can avoid, and an operator running the gate should get a recorded
/// replay rather than a failed spawn.
fn pi_binary_available() -> bool {
    let path = match std::env::var_os("PATH") {
        Some(path) => path,
        None => return false,
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join("pi");
        candidate.is_file()
    })
}

/// The prompt a session is sent.
///
/// The skill and the task's own words, never the whole tree: the phase's context rule is
/// that a task carries a bounded allowlist, and the contract is where it is written down.
fn pi_prompt(cfg: &Config, task: &PiTask) -> Result<String, MmError> {
    let mut prompt = String::new();
    prompt.push_str(&format!("# {}\n\n{}\n\n", task.task_id, task.goal));
    if let Some(skill) = &task.skill {
        let path = repo_path(cfg, skill);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            MmError::Config(format!("cannot read the skill {}: {e}", path.display()))
        })?;
        prompt.push_str(&text);
        prompt.push('\n');
    }
    if let Some(template) = &task.prompt_file {
        let path = repo_path(cfg, template);
        if path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|e| {
                MmError::Config(format!("cannot read the prompt {}: {e}", path.display()))
            })?;
            prompt.push_str(
                &text
                    .replace("{{goal}}", &task.goal)
                    .replace("{{allowed_paths}}", &task.allowed_paths.join(", "))
                    .replace("{{task_id}}", &task.task_id),
            );
            prompt.push('\n');
        }
    }
    if !task.allowed_paths.is_empty() {
        prompt.push_str("\nAllowed paths:\n");
        for allowed in &task.allowed_paths {
            prompt.push_str(&format!("- {allowed}\n"));
        }
    }
    Ok(prompt)
}

/// `mm-cli audit`.
pub async fn audit(cfg: Config, command: AuditCommand) -> Result<ExitCode, MmError> {
    let AuditCommand::ProductionTree {
        assert_unmodified_outside_promotion,
        json,
    } = command;
    let kernel = open(cfg.clone()).await?;
    let (since, baseline) = cycle_baseline(&kernel).await?;
    let audit = audit_writes(&repo_root(&cfg), &sandbox_root(&cfg), since);
    let outside = production_writes(&cfg, &audit.outside);
    let promotions: i64 = sqlx::query_scalar("SELECT count(*) FROM promotions")
        .fetch_one(kernel.sqlite.pool())
        .await
        .map_err(|e| MmError::Store(format!("cannot read promotions: {e}")))?;
    let journal: i64 = sqlx::query_scalar("SELECT count(*) FROM evolution_journal")
        .fetch_one(kernel.sqlite.pool())
        .await
        .map_err(|e| MmError::Store(format!("cannot read the journal: {e}")))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "baseline": baseline,
                "audit_complete": audit.complete,
                "outside": outside.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "promotions": promotions,
                "journal": journal,
            }))
            .map_err(internal)?
        );
    } else {
        println!("baseline        {baseline}");
        println!("audit_complete  {}", audit.complete);
        for path in &outside {
            println!("outside         {}", path.display());
        }
        println!("outside_count   {}", outside.len());
        println!("promotions      {promotions}");
        println!("journal         {journal}");
    }

    if assert_unmodified_outside_promotion && (!outside.is_empty() || !audit.complete) {
        eprintln!(
            "audit production-tree: {} production write(s) outside a promotion, audit complete {}",
            outside.len(),
            audit.complete
        );
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}
