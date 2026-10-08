//! `mm-cli being` — the persistent being's operator surface.
//!
//! Every subcommand here either proves a Phase 4 guarantee or reports what the
//! being's persistent self holds. Mutations go through `BeingFacade::apply`, so a
//! CLI write takes exactly the same guarded path a runtime write does — there is
//! no second door into the substrate. `being verify` is the gate: it runs the
//! adversarial corpus through the guard and the `/being` mirror through its SHACL
//! shapes, and exits non-zero unless both are clean.

use std::process::ExitCode;
use std::sync::Arc;

use clap::Subcommand;
use mm_being::{
    BeingError, BeingFacade, BlockKind, CommitmentStatus, EpistemicStatus, GoalOrigin, GoalStatus,
    ResourceKind, BEING_GRAPH,
};
use mm_core::{Config, MmError, Ulid};

use crate::kernel::Kernel;

/// `mm-cli being`.
#[derive(Subcommand, Debug)]
pub enum BeingCommand {
    /// Print the persistent self: identity, blocks, affect, goals, accounts.
    Show,
    /// Run the invariant suite and the SHACL shapes; non-zero unless both are clean.
    Verify,
    /// List the invariants the kernel binds.
    Invariants,
    /// Read or replace a core block.
    Block {
        #[command(subcommand)]
        command: BlockCommand,
    },
    /// List or promote a belief about a user.
    Belief {
        #[command(subcommand)]
        command: BeliefCommand,
    },
    /// Add a goal, or move one between statuses.
    Goal {
        #[command(subcommand)]
        command: GoalCommand,
    },
    /// Add a commitment, or move one between statuses.
    Commitment {
        #[command(subcommand)]
        command: CommitmentCommand,
    },
    /// Show a relationship projection.
    Relationship {
        #[command(subcommand)]
        command: RelationshipCommand,
    },
    /// Show the affect state.
    Affect {
        #[command(subcommand)]
        command: AffectCommand,
    },
    /// Show resource accounts and the policies that bound them.
    Budget {
        #[command(subcommand)]
        command: BudgetCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum BlockCommand {
    /// Print a core block.
    Get {
        /// One of: self, human, task.
        #[arg(long, default_value = "self")]
        kind: String,
    },
    /// Replace a core block's content.
    Set {
        /// One of: self, human, task.
        #[arg(long, default_value = "self")]
        kind: String,
        /// The block's label.
        #[arg(long)]
        label: Option<String>,
        /// The new content.
        #[arg(long)]
        content: String,
        /// The character limit; 0 uses the kind's default.
        #[arg(long, default_value_t = 0)]
        limit: u32,
    },
}

#[derive(Subcommand, Debug)]
pub enum BeliefCommand {
    /// List beliefs, optionally only for one user.
    List {
        /// A user ULID.
        #[arg(long)]
        user: Option<String>,
    },
    /// Promote a belief; the evidence gate may refuse.
    Promote {
        /// The user the belief is about.
        #[arg(long)]
        user: String,
        /// The proposition, exactly as it was recorded.
        #[arg(long)]
        proposition: String,
        /// One of the epistemic statuses, e.g. OBSERVED or VERIFIED.
        #[arg(long)]
        to: String,
        /// Evidence ids; repeatable.
        #[arg(long = "evidence")]
        evidence: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum GoalCommand {
    /// Add a goal.
    Add {
        /// What is wanted.
        #[arg(long)]
        description: String,
        /// How much it matters, in [0,1].
        #[arg(long, default_value_t = 0.5)]
        priority: f32,
        /// One of: user, being, inferred.
        #[arg(long, default_value = "being")]
        origin: String,
    },
    /// Move a goal between statuses.
    Transition {
        /// The goal ULID.
        #[arg(long)]
        id: String,
        /// One of: active, pending, blocked, fulfilled, abandoned, superseded.
        #[arg(long)]
        to: String,
        /// Why.
        #[arg(long, default_value = "")]
        reason: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum CommitmentCommand {
    /// Add a commitment.
    Add {
        /// Who it is made to.
        #[arg(long)]
        to_user: String,
        /// What is promised.
        #[arg(long)]
        description: String,
        /// The goal it serves, if any.
        #[arg(long)]
        goal: Option<String>,
    },
    /// Move a commitment between statuses.
    Transition {
        /// The commitment ULID.
        #[arg(long)]
        id: String,
        /// One of: active, fulfilled, revoked, superseded.
        #[arg(long)]
        to: String,
        /// Why.
        #[arg(long, default_value = "")]
        reason: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum RelationshipCommand {
    /// Print a relationship projection (all of them when no user is given).
    Show {
        /// A user ULID.
        #[arg(long)]
        user: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum AffectCommand {
    /// Print the affect axes.
    Show,
}

#[derive(Subcommand, Debug)]
pub enum BudgetCommand {
    /// Print accounts and policies.
    Show {
        /// Only this resource kind.
        #[arg(long)]
        kind: Option<String>,
    },
}

/// The repository root a command runs against, falling back to the build's root.
fn repo_root_of(cfg: &Config) -> std::path::PathBuf {
    if cfg.root.as_os_str().is_empty() {
        Config::repo_root()
    } else {
        cfg.root.clone()
    }
}

/// Map a being failure onto the kernel error type the CLI reports.
fn to_mm(e: BeingError) -> MmError {
    match e {
        BeingError::Db(m) => MmError::Store(m),
        BeingError::Graph(m) => MmError::Graph(m),
        BeingError::Config(m) => MmError::Config(m),
        other => MmError::Internal(format!("{} ({})", other, other.code())),
    }
}

fn parse_ulid(s: &str) -> Result<Ulid, MmError> {
    mm_core::id::parse_ulid(s).map_err(|e| MmError::Config(format!("{s:?}: {e}")))
}

/// Open the kernel with the graph and the being over it.
async fn open_facade(cfg: Config) -> Result<(Kernel, BeingFacade), MmError> {
    let kernel = Kernel::open(cfg, true).await?;
    let graph = kernel.graph()?;
    let facade = BeingFacade::open(
        &kernel.sqlite,
        graph.handle(),
        Arc::clone(&kernel.logger),
        Arc::clone(&kernel.ids),
    )
    .await
    .map_err(to_mm)?;
    Ok((kernel, facade))
}

/// The adversarial corpus `being verify` runs.
fn corpus_path(cfg: &Config) -> std::path::PathBuf {
    repo_root_of(cfg)
        .join("bench")
        .join("being")
        .join("invariants.jsonl")
}

/// `mm-cli being`.
pub async fn run(cfg: Config, command: BeingCommand) -> Result<ExitCode, MmError> {
    match command {
        BeingCommand::Show => show(cfg).await,
        BeingCommand::Verify => verify(cfg).await,
        BeingCommand::Invariants => invariants(cfg).await,
        BeingCommand::Block { command } => block(cfg, command).await,
        BeingCommand::Belief { command } => belief(cfg, command).await,
        BeingCommand::Goal { command } => goal(cfg, command).await,
        BeingCommand::Commitment { command } => commitment(cfg, command).await,
        BeingCommand::Relationship { command } => relationship(cfg, command).await,
        BeingCommand::Affect { command } => affect(cfg, command).await,
        BeingCommand::Budget { command } => budget(cfg, command).await,
    }
}

// ------------------------------------------------------------------ show ------

/// `mm-cli being show`.
async fn show(cfg: Config) -> Result<ExitCode, MmError> {
    let (kernel, facade) = open_facade(cfg).await?;

    let identity = facade.identity();
    println!("being");
    println!("  identity       {}", mm_core::ulid_string(&identity.id));
    println!("  version        {}", identity.current_version);
    println!("  description    {}", identity.self_description);
    println!(
        "  invariants     {}",
        identity
            .invariant_codes()
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    println!("  blocks");
    for (kind, block) in facade.blocks() {
        println!(
            "    {:<6} {:>5} chars, limit {}",
            kind.as_str(),
            block.len(),
            block.limit_chars
        );
    }

    let affect = facade.affect();
    println!(
        "  affect         valence {:.2} arousal {:.2} engagement {:.2} warmth {:.2} caution {:.2} curiosity {:.2} energy {:.2}",
        affect.valence,
        affect.arousal,
        affect.engagement,
        affect.warmth,
        affect.caution,
        affect.curiosity,
        affect.energy
    );

    println!("  goals          {}", facade.goals().len());
    for goal in facade.goals().values() {
        println!(
            "    {} {:<10} p={:.2} {}",
            mm_core::ulid_string(&goal.id),
            goal.status.as_str(),
            goal.priority,
            goal.description
        );
    }
    println!("  commitments    {}", facade.commitments().len());
    for commitment in facade.commitments().values() {
        println!(
            "    {} {:<10} to={} {}",
            mm_core::ulid_string(&commitment.id),
            commitment.status.as_str(),
            mm_core::ulid_string(&commitment.made_to),
            commitment.description
        );
    }

    println!("  accounts");
    for (kind, account) in &facade.resources().accounts {
        println!(
            "    {:<9} {:>12.2} {}",
            kind.as_str(),
            account.balance,
            account.unit
        );
    }

    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------- verify ------

/// `mm-cli being verify` — the Phase 4 gate.
async fn verify(cfg: Config) -> Result<ExitCode, MmError> {
    let corpus = corpus_path(&cfg);
    let (kernel, facade) = open_facade(cfg).await?;

    let report = facade.verify(&corpus).await.map_err(to_mm)?;
    let shapes = kernel.cfg.shapes_file_for(BEING_GRAPH);
    let shacl = kernel.graph()?.validate_with(BEING_GRAPH, &shapes).await?;

    let clean = report.ok() && shacl.conforms;
    println!("being verify");
    println!("  corpus         {}", corpus.display());
    println!("  invariants     {} enforced", report.invariants_enforced);
    println!(
        "  adversarial    {} denied of {}",
        report.corpus_denied, report.corpus_total
    );
    println!(
        "  terminal rows  {} of 2 trigger(s)",
        report.terminal_triggers
    );
    println!(
        "  budget         {}",
        if report.budget_reconciled {
            "reconciled"
        } else {
            "NOT reconciled"
        }
    );
    println!("  observations   {} unbacked", report.unbacked_observations);
    println!(
        "  affect         {} flag violation(s)",
        report.affect_flag_violations
    );
    println!(
        "  shacl /being   {} violation(s) against {}",
        shacl.violations.len(),
        shapes.display()
    );
    for failure in &report.failures {
        println!("    FAIL {failure}");
    }
    for violation in &shacl.violations {
        println!(
            "    FAIL [{}] {} ({})",
            violation.severity, violation.message, violation.path
        );
    }

    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

// ------------------------------------------------------------ invariants ------

/// `mm-cli being invariants`.
async fn invariants(cfg: Config) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, false).await?;
    let index: &dyn mm_core::Tabular = &kernel.sqlite;
    let rows = index
        .query_json(
            "SELECT code, assertion, enforcement FROM invariants ORDER BY code",
            mm_core::Params::new(),
        )
        .await?;
    println!("being invariants");
    for row in &rows {
        println!(
            "  {:<30} {:<14} {}",
            row["code"].as_str().unwrap_or_default(),
            row["enforcement"].as_str().unwrap_or_default(),
            row["assertion"].as_str().unwrap_or_default()
        );
    }
    if rows.is_empty() {
        println!("  (none: no identity has been created yet)");
    }
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------- blocks -----

/// `mm-cli being block`.
async fn block(cfg: Config, command: BlockCommand) -> Result<ExitCode, MmError> {
    let (kernel, mut facade) = open_facade(cfg).await?;
    match command {
        BlockCommand::Get { kind } => {
            let kind = BlockKind::parse(&kind)
                .ok_or_else(|| MmError::Config(format!("unknown block kind {kind:?}")))?;
            match facade.blocks().get(&kind) {
                Some(block) => {
                    println!(
                        "block {} ({} chars, limit {})",
                        kind.as_str(),
                        block.len(),
                        block.limit_chars
                    );
                    println!("{}", block.content);
                }
                None => println!("block {} is not set", kind.as_str()),
            }
        }
        BlockCommand::Set {
            kind,
            label,
            content,
            limit,
        } => {
            let kind = BlockKind::parse(&kind)
                .ok_or_else(|| MmError::Config(format!("unknown block kind {kind:?}")))?;
            let label = label.unwrap_or_else(|| kind.as_str().to_string());
            facade
                .apply(mm_being::BeingOp::SetBlock {
                    kind,
                    label,
                    content,
                    limit_chars: limit,
                })
                .await
                .map_err(to_mm)?;
            let block = facade.blocks().get(&kind).expect("the block was stored");
            println!(
                "block {} set: {} chars, limit {}",
                kind.as_str(),
                block.len(),
                block.limit_chars
            );
        }
    }
    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------- beliefs -----

/// `mm-cli being belief`.
async fn belief(cfg: Config, command: BeliefCommand) -> Result<ExitCode, MmError> {
    let (kernel, mut facade) = open_facade(cfg).await?;
    match command {
        BeliefCommand::List { user } => {
            let wanted = user.as_deref().map(parse_ulid).transpose()?;
            let snapshot = facade.snapshot();
            let mut shown = 0usize;
            println!("being beliefs");
            for (owner, belief) in &snapshot.beliefs {
                if wanted.is_some_and(|w| w != *owner) {
                    continue;
                }
                shown += 1;
                println!(
                    "  {} {:<10} {:.2} {}",
                    mm_core::ulid_string(owner),
                    belief.epistemic_status.as_str(),
                    belief.confidence,
                    belief.proposition
                );
                if !belief.evidence.is_empty() {
                    println!("    evidence {}", belief.evidence.join(", "));
                }
            }
            if shown == 0 {
                println!("  (no beliefs recorded)");
            }
        }
        BeliefCommand::Promote {
            user,
            proposition,
            to,
            evidence,
        } => {
            let user_id = parse_ulid(&user)?;
            let to = EpistemicStatus::parse(&to)
                .ok_or_else(|| MmError::Config(format!("unknown epistemic status {to:?}")))?;
            facade
                .apply(mm_being::BeingOp::PromoteBelief {
                    user_id,
                    proposition: proposition.clone(),
                    to,
                    evidence: evidence.clone(),
                })
                .await
                .map_err(to_mm)?;
            println!(
                "belief promoted: {} -> {} ({} evidence)",
                proposition,
                to.as_str(),
                evidence.len()
            );
        }
    }
    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}

// ------------------------------------------------------------------ goals -----

/// `mm-cli being goal`.
async fn goal(cfg: Config, command: GoalCommand) -> Result<ExitCode, MmError> {
    let (kernel, mut facade) = open_facade(cfg).await?;
    match command {
        GoalCommand::Add {
            description,
            priority,
            origin,
        } => {
            let origin = GoalOrigin::parse(&origin)
                .ok_or_else(|| MmError::Config(format!("unknown goal origin {origin:?}")))?;
            let before: Vec<Ulid> = facade.goals().keys().copied().collect();
            facade
                .apply(mm_being::BeingOp::AddGoal {
                    description,
                    priority,
                    origin,
                })
                .await
                .map_err(to_mm)?;
            let added = facade
                .goals()
                .keys()
                .find(|id| !before.contains(id))
                .copied()
                .expect("the goal was stored");
            println!("goal added: {}", mm_core::ulid_string(&added));
        }
        GoalCommand::Transition { id, to, reason } => {
            let id = parse_ulid(&id)?;
            let to = GoalStatus::parse(&to)
                .ok_or_else(|| MmError::Config(format!("unknown goal status {to:?}")))?;
            facade
                .apply(mm_being::BeingOp::TransitionGoal {
                    id,
                    from: None,
                    to,
                    reason,
                })
                .await
                .map_err(to_mm)?;
            println!("goal {} -> {}", mm_core::ulid_string(&id), to.as_str());
        }
    }
    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}

// ------------------------------------------------------------ commitments -----

/// `mm-cli being commitment`.
async fn commitment(cfg: Config, command: CommitmentCommand) -> Result<ExitCode, MmError> {
    let (kernel, mut facade) = open_facade(cfg).await?;
    match command {
        CommitmentCommand::Add {
            to_user,
            description,
            goal,
        } => {
            let made_to = parse_ulid(&to_user)?;
            let goal_id = goal.as_deref().map(parse_ulid).transpose()?;
            let before: Vec<Ulid> = facade.commitments().keys().copied().collect();
            facade
                .apply(mm_being::BeingOp::AddCommitment {
                    goal_id,
                    made_to,
                    description,
                })
                .await
                .map_err(to_mm)?;
            let added = facade
                .commitments()
                .keys()
                .find(|id| !before.contains(id))
                .copied()
                .expect("the commitment was stored");
            println!("commitment added: {}", mm_core::ulid_string(&added));
        }
        CommitmentCommand::Transition { id, to, reason } => {
            let id = parse_ulid(&id)?;
            let to = CommitmentStatus::parse(&to)
                .ok_or_else(|| MmError::Config(format!("unknown commitment status {to:?}")))?;
            facade
                .apply(mm_being::BeingOp::TransitionCommitment {
                    id,
                    from: None,
                    to,
                    reason,
                })
                .await
                .map_err(to_mm)?;
            println!(
                "commitment {} -> {}",
                mm_core::ulid_string(&id),
                to.as_str()
            );
        }
    }
    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------- relationships -----

/// `mm-cli being relationship`.
async fn relationship(cfg: Config, command: RelationshipCommand) -> Result<ExitCode, MmError> {
    let (kernel, facade) = open_facade(cfg).await?;
    match command {
        RelationshipCommand::Show { user } => {
            let wanted = user.as_deref().map(parse_ulid).transpose()?;
            let snapshot = facade.snapshot();
            println!("being relationships");
            let axes = mm_being::RelationshipState::dimension_names();
            let mut shown = 0usize;
            for (owner, state) in &snapshot.relationships {
                if wanted.is_some_and(|w| w != *owner) {
                    continue;
                }
                shown += 1;
                println!("  {}", mm_core::ulid_string(owner));
                for axis in axes {
                    if let Some(value) = state.dimension(axis) {
                        println!("    {axis:<14} {value:.3}");
                    }
                }
                println!("    unresolved     {}", state.unresolved.len());
            }
            if shown == 0 {
                println!("  (no relationship recorded)");
            }
        }
    }
    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------- affect -----

/// `mm-cli being affect`.
async fn affect(cfg: Config, command: AffectCommand) -> Result<ExitCode, MmError> {
    let (kernel, facade) = open_facade(cfg).await?;
    match command {
        AffectCommand::Show => {
            let affect = facade.affect();
            println!("being affect");
            println!("  valence        {:.3}", affect.valence);
            println!("  arousal        {:.3}", affect.arousal);
            println!("  engagement     {:.3}", affect.engagement);
            println!("  warmth         {:.3}", affect.warmth);
            println!("  caution        {:.3}", affect.caution);
            println!("  curiosity      {:.3}", affect.curiosity);
            println!("  energy         {:.3}", affect.energy);
        }
    }
    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------- budget -----

/// `mm-cli being budget`.
async fn budget(cfg: Config, command: BudgetCommand) -> Result<ExitCode, MmError> {
    let (kernel, facade) = open_facade(cfg).await?;
    match command {
        BudgetCommand::Show { kind } => {
            let wanted = kind
                .as_deref()
                .map(|k| {
                    ResourceKind::parse(k)
                        .ok_or_else(|| MmError::Config(format!("unknown resource kind {k:?}")))
                })
                .transpose()?;
            println!("being budget");
            println!("  accounts");
            for (resource, account) in &facade.resources().accounts {
                if wanted.is_some_and(|w| w != *resource) {
                    continue;
                }
                println!(
                    "    {:<9} {:>12.2} {}",
                    resource.as_str(),
                    account.balance,
                    account.unit
                );
            }
            let index: &dyn mm_core::Tabular = &kernel.sqlite;
            let rows = index
                .query_json(
                    "SELECT kind, period, limit_amount, hard FROM budget_policies ORDER BY kind",
                    mm_core::Params::new(),
                )
                .await?;
            println!("  policies");
            for row in &rows {
                println!(
                    "    {:<9} {:<10} limit {:.2} {}",
                    row["kind"].as_str().unwrap_or_default(),
                    row["period"].as_str().unwrap_or_default(),
                    row["limit_amount"].as_f64().unwrap_or(0.0),
                    if row["hard"].as_i64().unwrap_or(1) != 0 {
                        "hard"
                    } else {
                        "soft"
                    }
                );
            }
            println!(
                "  reconciled     {}",
                facade.budget_reconciles().await.map_err(to_mm)?
            );
        }
    }
    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;
    Ok(ExitCode::SUCCESS)
}
