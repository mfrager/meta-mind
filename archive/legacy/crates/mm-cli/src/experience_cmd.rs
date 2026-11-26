//! `mm-cli experience` — the experience compiler's operator surface.
//!
//! The compiler reads a trajectory fixture and the library's insight ledger and
//! emits **candidate** techniques, each carrying the confidence its support
//! justifies. Everything it emits is a draft: the confidence is capped at
//! `MAX_DRAFT_CONFIDENCE` (0.5), strictly below `PROMOTION_THRESHOLD` (0.6), so a
//! lesson repeated a hundred times still cannot become a committed belief here.
//! Promotion is Phase 11's job, with evidence and a reviewed change-set, and this
//! surface must never be a way around it — which is why a compile that somehow
//! produced a promotable entry is refused outright rather than written.
//!
//! Two inputs, one output, no model:
//!
//! * `--trajectories` is a JSONL of runs; a malformed line is a typed refusal
//!   naming the line rather than a quietly skipped lesson.
//! * the insights come from the library index (`insights`), because an insight is
//!   a library record and reading it from anywhere else would be a second source
//!   of truth.
//!
//! An empty compile is a fact, not a failure: the (empty) output file is still
//! written, so a caller can diff it and see that nothing was distilled.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use mm_core::{Config, MmError};
use mm_library::experience::{compile, Insight, Trajectory};
use mm_library::manager::TARGET;
use mm_library::LibraryEntry;
use mm_log::{codes, Level, LogRecord};

use crate::library::{open_manager, shutdown};

/// `mm-cli experience`.
#[derive(Subcommand, Debug)]
pub enum ExperienceCommand {
    /// Compile trajectories and insights into sub-threshold candidate drafts.
    Compile {
        /// JSONL of trajectories, one run per line.
        #[arg(long, value_name = "FILE")]
        trajectories: PathBuf,
        /// Where the candidate drafts are written, one JSON object per line.
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
}

/// `mm-cli experience`.
pub async fn run(cfg: Config, command: ExperienceCommand) -> Result<ExitCode, MmError> {
    match command {
        ExperienceCommand::Compile { trajectories, out } => {
            compile_cmd(cfg, trajectories, out).await
        }
    }
}

/// `mm-cli experience compile`.
async fn compile_cmd(
    cfg: Config,
    trajectories: PathBuf,
    out: PathBuf,
) -> Result<ExitCode, MmError> {
    let (kernel, manager) = open_manager(cfg).await?;

    let result = async {
        let raw = std::fs::read_to_string(&trajectories)
            .map_err(|e| MmError::Config(format!("cannot read {}: {e}", trajectories.display())))?;
        let trajectories = Trajectory::from_jsonl(&raw)?;

        let insights = manager
            .store()
            .list_insights()
            .await?
            .into_iter()
            .map(|(iri, text, kind, upvotes, downvotes)| Insight {
                iri,
                text,
                kind,
                evidence: Vec::new(),
                upvotes,
                downvotes,
            })
            .collect::<Vec<_>>();

        let compiled = compile(&trajectories, &insights);

        // The one refusal that matters: nothing from here may be promotable.
        if !compiled.is_sub_threshold() {
            return Err(MmError::Internal(
                "experience compile produced a promotable candidate; promotion belongs to Phase 11"
                    .to_string(),
            ));
        }

        let jsonl = compiled.to_jsonl()?;
        if let Some(parent) = out.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&out, jsonl)?;

        let technique_iris: Vec<String> = compiled
            .techniques
            .iter()
            .map(|entry| entry.version_iri().into_string())
            .collect();

        manager
            .logger()
            .emit(
                LogRecord::new(Level::Info, codes::EXPERIENCE_COMPILE, TARGET)
                    .with_field("job_ulid", mm_core::ulid_string(&manager.store().next_id()))
                    .with_field("trajectories", trajectories.len())
                    .with_field("insights", insights.len())
                    .with_field("techniques", technique_iris)
                    .with_field("policies", compiled.policies.len()),
            )
            .await?;

        println!(
            "compiled {} candidate(s) to {}",
            compiled.len(),
            out.display()
        );
        println!(
            "  techniques {}  policies {}  lessons {}",
            compiled.techniques.len(),
            compiled.policies.len(),
            compiled.support.len()
        );
        Ok(ExitCode::SUCCESS)
    }
    .await;

    shutdown(kernel).await?;
    result
}
