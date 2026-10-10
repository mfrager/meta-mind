//! `mm-cli policy` — the policy genome's operator surface.
//!
//! Three read/write views over the same tables:
//!
//! * `history` prints every immutable version of a policy and the parent each one
//!   names — the chain [`Policy::next`](mm_library::policy::Policy::next) enforces,
//!   read back from the store rather than from a policy object;
//! * `fitness` prints the running mean the store holds for one version. It prints
//!   `0 0 0` for a version that has never been tried, because "no evidence" is a
//!   fact about the version and not an error;
//! * `evolve` runs the deterministic genome ([`mm_library::genome::evolve`]), whose
//!   hard budget and retain-only-gains rule live in that module. The CLI's only
//!   decision is where the budget and the evaluation set come from: the committed
//!   `bench/library/evolution/` fixtures unless `--budget`/`--eval` say otherwise,
//!   and `--generations` overrides the budget's own count when it is non-zero.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use mm_core::{Config, MmError};
use mm_library::entry::iri;
use mm_library::genome::{evolve, EvaluationSet, EvoBudget};

use crate::library::{open_manager, shutdown};

/// `mm-cli policy`.
#[derive(Subcommand, Debug)]
pub enum PolicyCommand {
    /// Every immutable version of a policy, oldest first.
    History {
        /// The policy's head or versioned IRI.
        #[arg(value_name = "IRI")]
        iri: String,
    },
    /// The fitness the store holds for a policy version.
    Fitness {
        /// The policy's versioned IRI (`…@2`).
        #[arg(value_name = "IRI")]
        iri: String,
    },
    /// Answer one authorization question with Phase 10's permission engine.
    ///
    /// It belongs under `policy` because it *is* a policy question, and it is answered by
    /// `mm_tools::executor::load_engine` — the same engine the executor decides with — so
    /// a check and a run cannot disagree.
    Check {
        /// Who is acting.
        #[arg(long, value_name = "PRINCIPAL")]
        principal: String,
        /// The capability's verb, e.g. `fs.write`.
        #[arg(long, value_name = "KIND.VERB")]
        action: String,
        /// The capability's pattern, e.g. `data/sandbox/x`.
        #[arg(long, value_name = "PATTERN")]
        resource: String,
        /// The tool the caller would act through. Without it no grant can match, which is
        /// the strictest reading of the same request.
        #[arg(long, value_name = "NAME")]
        tool: Option<String>,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
        /// Exit non-zero unless the decision allows.
        #[arg(long)]
        assert_allow: bool,
    },
    /// Evolve a policy genome, retaining only fitness gains.
    Evolve {
        /// The policy's name (the part before `@`).
        #[arg(value_name = "NAME")]
        name: String,
        /// Generations to run; overrides the budget file when non-zero.
        #[arg(long, default_value_t = 3)]
        generations: u32,
        /// A budget TOML; defaults to `bench/library/evolution/budget.toml`.
        #[arg(long, value_name = "FILE")]
        budget: Option<PathBuf>,
        /// An evaluation set; defaults to `bench/library/evolution/eval.jsonl`.
        #[arg(long, value_name = "FILE")]
        eval: Option<PathBuf>,
    },
}

/// `mm-cli policy`.
pub async fn run(cfg: Config, command: PolicyCommand) -> Result<ExitCode, MmError> {
    match command {
        PolicyCommand::Check {
            principal,
            action,
            resource,
            tool,
            json,
            assert_allow,
        } => {
            crate::tools_cmd::policy_check(
                cfg,
                crate::tools_cmd::CheckArgs {
                    principal,
                    action,
                    resource,
                    tool,
                    json,
                    assert_allow,
                },
            )
            .await
        }
        PolicyCommand::History { iri } => history(cfg, &iri).await,
        PolicyCommand::Fitness { iri } => fitness(cfg, &iri).await,
        PolicyCommand::Evolve {
            name,
            generations,
            budget,
            eval,
        } => evolve_policy(cfg, &name, generations, budget, eval).await,
    }
}

/// The head IRI of a policy IRI: the versioned form without its `@version`.
fn head_of(policy_iri: &str) -> String {
    policy_iri
        .rsplit_once('@')
        .map(|(head, _)| head.to_string())
        .unwrap_or_else(|| policy_iri.to_string())
}

/// Print every version of a policy, oldest first.
async fn history(cfg: Config, policy_iri: &str) -> Result<ExitCode, MmError> {
    let head = head_of(policy_iri);
    let (kernel, manager) = open_manager(cfg).await?;
    let rows = manager.store().policy_history(&head).await?;
    shutdown(kernel).await?;

    if rows.is_empty() {
        return Err(MmError::Config(format!(
            "no such policy: {policy_iri} (looked for {head})"
        )));
    }
    for row in &rows {
        println!(
            "v{} parent={} scope={} confidence={} generation={}",
            row.version,
            row.parent_version
                .map_or_else(|| "none".to_string(), |v| v.to_string()),
            row.scope,
            row.confidence,
            row.generation
        );
    }
    println!("{} version(s)", rows.len());
    Ok(ExitCode::SUCCESS)
}

/// Print the fitness the store holds for one version.
async fn fitness(cfg: Config, policy_iri: &str) -> Result<ExitCode, MmError> {
    let version = iri::version_of(policy_iri).ok_or_else(|| {
        MmError::Config(format!(
            "{policy_iri} names no version; use the versioned IRI (…@2)"
        ))
    })?;
    let head = head_of(policy_iri);
    let (kernel, manager) = open_manager(cfg).await?;
    let found = manager
        .store()
        .fitness_of(&head, i64::from(version))
        .await?;
    shutdown(kernel).await?;

    let (trials, successes, mean) = found.unwrap_or((0, 0, 0.0));
    println!("{trials} {successes} {mean}");
    Ok(ExitCode::SUCCESS)
}

/// Run the genome and report each generation.
async fn evolve_policy(
    cfg: Config,
    name: &str,
    generations: u32,
    budget: Option<PathBuf>,
    eval: Option<PathBuf>,
) -> Result<ExitCode, MmError> {
    let budget_path = budget.unwrap_or_else(|| default_fixture("budget.toml"));
    let eval_path = eval.unwrap_or_else(|| default_fixture("eval.jsonl"));

    let mut budget = EvoBudget::load(&budget_path)?;
    if generations > 0 {
        budget.generations = generations;
    }
    let evals = EvaluationSet::load(&eval_path)?;

    let (kernel, manager) = open_manager(cfg).await?;
    let report = evolve(&manager, name, &budget, &evals).await?;
    shutdown(kernel).await?;

    for generation in &report.generations {
        println!(
            "generation {} retained {} best_fitness {}",
            generation.generation, generation.retained, generation.best_fitness
        );
        for candidate in &generation.candidates {
            println!(
                "  v{} {} fitness={} retained={}",
                candidate.version, candidate.mutation, candidate.fitness, candidate.retained
            );
        }
    }
    match report.best_version {
        Some(version) => println!("best_version {version} fitness {}", report.best_fitness),
        None => println!("best_version none fitness 0"),
    }
    println!("{} version(s) created", report.versions_created);
    Ok(ExitCode::SUCCESS)
}

/// A committed fixture beside the repository root, so the command works from any
/// working directory.
fn default_fixture(file: &str) -> PathBuf {
    Config::repo_root()
        .join("bench")
        .join("library")
        .join("evolution")
        .join(file)
}
