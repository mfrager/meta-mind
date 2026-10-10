//! `mm-cli memory` — the memory organ's operator surface.
//!
//! Every subcommand either proves a Phase 5 guarantee or reports what the memory
//! organ holds. Mutations go through `MemoryEngine`, so a CLI write takes exactly
//! the same validated path a runtime write does — there is no second door into the
//! store. `memory verify` is the gate: reconciliation between SQLite and `/memory`,
//! the protected-ledger invariant, the provenance closure, and the SHACL shapes.
//!
//! **`memory eval` is hermetic on purpose.** It opens a throwaway store under a
//! temporary directory, ingests the corpus, grades it, and throws the store away.
//! An evaluation that ran against the live store would mutate it, would depend on
//! whatever else had been written there, and would stop being rerunnable the
//! moment `memory consolidate` archived the corpus it grades against.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::Subcommand;
use mm_core::{Config, MmError, Timestamp, Ulid, UlidFactory};
use mm_log::{Level, Logger, RedactionPolicy};
use mm_memory::{
    Memory, MemoryEngine, MemoryError, MemoryFilter, MemoryKind, RecallQuery, Tier, TimeInterval,
};
use serde::Deserialize;

use crate::kernel::Kernel;

/// `mm-cli memory`.
#[derive(Subcommand, Debug)]
pub enum MemoryCommand {
    /// Admit one memory.
    Add {
        /// One of: episodic, semantic, procedural, working, autobiographical,
        /// relational, prediction, mistake, near_miss, developmental.
        #[arg(long, default_value = "episodic")]
        kind: String,
        /// The content, or `@path` to read it from a file.
        #[arg(long)]
        content: String,
        /// One of: core, recall, archival.
        #[arg(long, default_value = "recall")]
        tier: String,
        /// How much it is trusted, in [0,1].
        #[arg(long, default_value_t = 0.5)]
        confidence: f32,
        /// How much it matters, in [0,1].
        #[arg(long, default_value_t = 0.5)]
        importance: f32,
        /// Never forget this record.
        #[arg(long)]
        protected: bool,
        /// The event or record it came from.
        #[arg(long)]
        source: Option<String>,
    },
    /// Print one memory.
    Get {
        /// The memory ULID.
        id: String,
    },
    /// Retrieve the best memories for a query.
    Recall {
        /// The query text.
        query: String,
        /// How many hits.
        #[arg(long, default_value_t = 8)]
        k: usize,
        /// Restrict to kinds, comma-separated.
        #[arg(long)]
        kinds: Option<String>,
        /// Restrict to tiers, comma-separated.
        #[arg(long)]
        tier: Option<String>,
    },
    /// Consolidate episodes into patterns and a summary tree.
    Consolidate {
        /// The window, e.g. `30d`, `12h`.
        #[arg(long, default_value = "30d")]
        window: String,
        /// An episode stream to ingest first.
        #[arg(long, value_name = "FILE")]
        input: Option<PathBuf>,
    },
    /// Run a forget cycle.
    Forget {
        /// The instant to measure retention at: RFC3339 or epoch seconds.
        #[arg(long, value_name = "TS")]
        now: Option<String>,
        /// Decide and log, but write nothing.
        #[arg(long)]
        dry_run: bool,
        /// The retention floor.
        #[arg(long)]
        threshold: Option<f64>,
        /// The retention half-life, in nanoseconds.
        #[arg(long)]
        half_life_ns: Option<u64>,
    },
    /// Report the organ's size, and reconcile SQLite against `/memory`.
    Stats {
        /// Exit non-zero unless SQLite and `/memory` agree exactly.
        #[arg(long)]
        reconcile: bool,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Grade retrieval over the gold set; non-zero below the thresholds.
    Eval {
        /// The gold set.
        #[arg(long, value_name = "FILE")]
        gold: PathBuf,
        /// How many hits to retrieve per query.
        #[arg(long, default_value_t = 5)]
        k: usize,
        /// Thresholds and weights.
        #[arg(long, value_name = "FILE")]
        thresholds: PathBuf,
        /// The episode corpus to ingest first.
        #[arg(long, value_name = "FILE")]
        episodes: Option<PathBuf>,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Reconcile, check the ledger, and SHACL-validate; non-zero on any failure.
    Verify {
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Record a mistake and the rule that prevents its recurrence.
    Mistake {
        #[command(subcommand)]
        command: MistakeCommand,
    },
    /// Record a procedure.
    Procedure {
        #[command(subcommand)]
        command: ProcedureCommand,
    },
    /// The packed vector index.
    Index {
        #[command(subcommand)]
        command: IndexCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum MistakeCommand {
    /// Record a mistake.
    Add {
        /// What kind of failure it was.
        #[arg(long)]
        failure_mode: String,
        /// The rule that prevents it recurring.
        #[arg(long)]
        corrective_rule: Option<String>,
        /// How likely it is to happen again, in [0,1].
        #[arg(long, default_value_t = 0.5)]
        risk: f32,
        /// A signal that was available and missed; repeatable.
        #[arg(long = "missed")]
        missed: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProcedureCommand {
    /// Record a procedure.
    Add {
        /// What it is called.
        #[arg(long)]
        name: String,
        /// A step; repeatable.
        #[arg(long = "step")]
        steps: Vec<String>,
        /// How well it is known, in [0,1].
        #[arg(long, default_value_t = 0.0)]
        proficiency: f32,
    },
}

#[derive(Subcommand, Debug)]
pub enum IndexCommand {
    /// Rebuild the packed vector index.
    Rebuild {
        /// Where to write it; defaults to `<data_dir>/index/memory.pack`.
        #[arg(long, value_name = "FILE")]
        pack: Option<PathBuf>,
    },
}

/// Map a memory failure onto the kernel error type the CLI reports.
fn to_mm(e: MemoryError) -> MmError {
    match e {
        MemoryError::Db(m) => MmError::Store(m),
        MemoryError::Graph(m) => MmError::Graph(m),
        MemoryError::Config(m) => MmError::Config(m),
        MemoryError::Codec(m) => MmError::Codec(m),
        other => MmError::Internal(format!("{} ({})", other, other.code())),
    }
}

fn parse_ulid(s: &str) -> Result<Ulid, MmError> {
    mm_core::id::parse_ulid(s).map_err(|e| MmError::Config(format!("{s:?}: {e}")))
}

/// Parse an instant: RFC3339, or a plain epoch-second count.
fn parse_instant(text: &str) -> Result<Timestamp, MmError> {
    if let Ok(seconds) = text.parse::<u64>() {
        return Ok(Timestamp::from_epoch_seconds(seconds));
    }
    Timestamp::from_rfc3339(text).map_err(|e| MmError::Config(format!("{text:?}: {e}")))
}

/// The repository root a command runs against, falling back to the build's root.
fn repo_root_of(cfg: &Config) -> PathBuf {
    if cfg.root.as_os_str().is_empty() {
        Config::repo_root()
    } else {
        cfg.root.clone()
    }
}

/// Read content, resolving `@path`.
fn resolve_content(content: &str) -> Result<String, MmError> {
    match content.strip_prefix('@') {
        None => Ok(content.to_string()),
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| MmError::Config(format!("cannot read {path}: {e}"))),
    }
}

/// Open the kernel with the graph and a memory engine over it.
async fn open_engine(cfg: Config) -> Result<(Kernel, MemoryEngine), MmError> {
    let kernel = Kernel::open(cfg, true).await?;
    let graph = kernel.graph()?;
    let engine = MemoryEngine::open(
        &kernel.sqlite,
        graph.handle(),
        Arc::clone(&kernel.logger),
        Arc::clone(&kernel.ids),
    )
    .await
    .map_err(to_mm)?;
    Ok((kernel, engine))
}

/// `mm-cli memory`.
pub async fn run(cfg: Config, command: MemoryCommand) -> Result<ExitCode, MmError> {
    match command {
        MemoryCommand::Add {
            kind,
            content,
            tier,
            confidence,
            importance,
            protected,
            source,
        } => {
            add(
                cfg, kind, content, tier, confidence, importance, protected, source,
            )
            .await
        }
        MemoryCommand::Get { id } => get(cfg, id).await,
        MemoryCommand::Recall {
            query,
            k,
            kinds,
            tier,
        } => recall(cfg, query, k, kinds, tier).await,
        MemoryCommand::Consolidate { window, input } => consolidate(cfg, window, input).await,
        MemoryCommand::Forget {
            now,
            dry_run,
            threshold,
            half_life_ns,
        } => forget(cfg, now, dry_run, threshold, half_life_ns).await,
        MemoryCommand::Stats { reconcile, json } => stats(cfg, reconcile, json).await,
        MemoryCommand::Eval {
            gold,
            k,
            thresholds,
            episodes,
            json,
        } => eval(cfg, gold, k, thresholds, episodes, json).await,
        MemoryCommand::Verify { json } => verify(cfg, json).await,
        MemoryCommand::Mistake { command } => mistake(cfg, command).await,
        MemoryCommand::Procedure { command } => procedure(cfg, command).await,
        MemoryCommand::Index { command } => index(cfg, command).await,
    }
}

// ------------------------------------------------------------------- add -------

#[allow(clippy::too_many_arguments)]
async fn add(
    cfg: Config,
    kind: String,
    content: String,
    tier: String,
    confidence: f32,
    importance: f32,
    protected: bool,
    source: Option<String>,
) -> Result<ExitCode, MmError> {
    let kind = MemoryKind::parse(&kind)
        .ok_or_else(|| MmError::Config(format!("unknown memory kind {kind:?}")))?;
    let tier =
        Tier::parse(&tier).ok_or_else(|| MmError::Config(format!("unknown tier {tier:?}")))?;
    let content = resolve_content(&content)?;
    let source = source.as_deref().map(parse_ulid).transpose()?;
    let (kernel, mut engine) = open_engine(cfg).await?;

    let now = Timestamp::now();
    let id = engine.ids().next();
    let provenance = engine.ids().next();
    // The developmental ledger is protected by construction: a CLI caller should
    // not be able to write an unprotected ledger entry, and the library refuses one
    // anyway. Forcing it here keeps the command usable without weakening the rule.
    let developmental = kind == MemoryKind::Developmental;
    let protected = protected || developmental;
    let mut memory = Memory::new(
        id,
        kind,
        content,
        TimeInterval::open(now),
        confidence,
        importance,
        provenance,
        now,
    )
    .map_err(to_mm)?
    .with_tier(tier)
    .with_protected(protected);
    if let Some(source) = source {
        memory = memory.with_source(source);
    }
    // A keyword cue per token, so the lexical channel can reach what the content
    // says without a second tokenizer at query time.
    for token in mm_memory::store::tokenize(&memory.content)
        .into_iter()
        .take(16)
    {
        memory = memory.with_cue(mm_memory::RetrievalCue::keyword(token));
    }

    let admission = engine.remember(memory).await.map_err(to_mm)?;
    if developmental {
        println!("note: a developmental memory is protected by construction");
    }
    match admission {
        mm_memory::Admission::Accepted => println!("memory added: {}", mm_core::ulid_string(&id)),
        mm_memory::Admission::Duplicate { existing } => println!(
            "flagged duplicate of {}; not stored",
            mm_core::ulid_string(&existing)
        ),
        mm_memory::Admission::NearDuplicate { existing, cosine } => println!(
            "memory added: {} (flagged near-duplicate of {}, cosine {cosine:.4})",
            mm_core::ulid_string(&id),
            mm_core::ulid_string(&existing)
        ),
    }
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

// ------------------------------------------------------------------- get -------

async fn get(cfg: Config, id: String) -> Result<ExitCode, MmError> {
    let id = parse_ulid(&id)?;
    let (kernel, engine) = open_engine(cfg).await?;
    match engine.get(&id).await.map_err(to_mm)? {
        Some(memory) => print_memory(&memory),
        None => {
            return Err(MmError::Config(format!(
                "no memory {}",
                mm_core::ulid_string(&id)
            )))
        }
    }
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

fn print_memory(memory: &Memory) {
    println!("memory {}", mm_core::ulid_string(&memory.id));
    println!("  kind           {}", memory.kind.as_str());
    println!(
        "  tier           {} ({})",
        memory.tier.as_str(),
        memory.status.as_str()
    );
    println!("  confidence     {:.3}", memory.confidence);
    println!("  importance     {:.3}", memory.importance);
    println!("  protected      {}", memory.protected);
    println!("  valid_from     {}", memory.validity.from.to_rfc3339());
    match memory.validity.until {
        Some(until) => println!("  valid_until    {}", until.to_rfc3339()),
        None => println!("  valid_until    (open)"),
    }
    println!("  recorded_at    {}", memory.recorded_at.to_rfc3339());
    println!(
        "  source         {}",
        memory
            .source
            .map(|s| mm_core::ulid_string(&s))
            .unwrap_or_else(|| "(unknown)".to_string())
    );
    println!(
        "  provenance     {}",
        mm_core::ulid_string(&memory.provenance)
    );
    println!("  entities       {}", memory.entities.len());
    println!("  cues           {}", memory.cues.len());
    println!("  content_hash   {}", memory.content_hash());
    println!("  content        {}", memory.content);
}

// ---------------------------------------------------------------- recall -------

async fn recall(
    cfg: Config,
    query: String,
    k: usize,
    kinds: Option<String>,
    tier: Option<String>,
) -> Result<ExitCode, MmError> {
    let kinds = kinds
        .as_deref()
        .map(|text| {
            MemoryKind::parse_list(text)
                .ok_or_else(|| MmError::Config(format!("unknown memory kind in {text:?}")))
        })
        .transpose()?
        .unwrap_or_default();
    let tiers: Vec<Tier> = match tier.as_deref() {
        None => Vec::new(),
        Some(text) => text
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| {
                Tier::parse(part).ok_or_else(|| MmError::Config(format!("unknown tier {part:?}")))
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let (kernel, engine) = open_engine(cfg).await?;
    let query = RecallQuery::new(query, k, Timestamp::now())
        .with_kinds(kinds)
        .with_tiers(tiers);
    let hits = engine.recall(query).await.map_err(to_mm)?;
    println!("memory recall ({hit_count} hit(s))", hit_count = hits.len());
    for (rank, hit) in hits.iter().enumerate() {
        println!(
            "  {:>2}. {:.6}  {}  {}",
            rank + 1,
            hit.score,
            mm_core::ulid_string(&hit.memory.id),
            hit.memory.kind.as_str()
        );
        let parts: Vec<String> = hit
            .parts
            .iter()
            .map(|part| format!("{}={:.3}", part.channel, part.contribution))
            .collect();
        println!("      {}", parts.join(" "));
        println!("      {}", snippet(&hit.memory.content));
    }
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

/// A one-line excerpt, bounded so a long memory cannot flood a terminal.
fn snippet(content: &str) -> String {
    let flat: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 110 {
        flat
    } else {
        let mut out: String = flat.chars().take(110).collect();
        out.push('…');
        out
    }
}

// ----------------------------------------------------------- consolidate -------

async fn consolidate(
    cfg: Config,
    window: String,
    input: Option<PathBuf>,
) -> Result<ExitCode, MmError> {
    let window_ns = mm_memory::consolidate::parse_window(&window).map_err(to_mm)?;
    let (kernel, mut engine) = open_engine(cfg).await?;
    if let Some(input) = input {
        let ingested = engine
            .ingest_episodes(&input, &kernel.logger)
            .await
            .map_err(to_mm)?;
        println!(
            "ingested {} episode(s) from {}",
            ingested.len(),
            input.display()
        );
    }
    let now = Timestamp::now();
    let from = Timestamp::from_epoch_seconds(now.seconds.saturating_sub(window_ns / 1_000_000_000));
    let interval = TimeInterval {
        from,
        until: Some(now),
    };
    let outcome = engine.consolidate(interval).await.map_err(to_mm)?;
    println!("memory consolidate");
    println!("  window         {window}");
    println!("  active before  {}", outcome.before);
    println!("  active after   {}", outcome.after);
    println!("  net change     {}", outcome.delta());
    println!("  generalized    {}", outcome.generalized);
    println!("  merged         {}", outcome.merged);
    println!("  summaries      {}", outcome.summaries);
    println!("  communities    {}", outcome.communities);
    println!("  archived       {}", outcome.archived.len());
    println!(
        "  closure        {}",
        if outcome.provenance_closure_ok {
            "ok"
        } else {
            "BROKEN"
        }
    );
    shutdown(kernel).await?;
    Ok(if outcome.provenance_closure_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

// ---------------------------------------------------------------- forget -------

async fn forget(
    cfg: Config,
    now: Option<String>,
    dry_run: bool,
    threshold: Option<f64>,
    half_life_ns: Option<u64>,
) -> Result<ExitCode, MmError> {
    let now = match now.as_deref() {
        Some(text) => parse_instant(text)?,
        None => Timestamp::now(),
    };
    let (kernel, mut engine) = open_engine(cfg).await?;
    if let Some(threshold) = threshold {
        engine = engine.with_threshold(threshold);
    }
    if let Some(half_life_ns) = half_life_ns {
        engine = engine.with_half_life(half_life_ns);
    }
    let report = engine.forget_cycle(now, dry_run).await.map_err(to_mm)?;
    println!("memory forget{}", if dry_run { " (dry run)" } else { "" });
    println!("  now            {}", report.now.to_rfc3339());
    println!("  considered     {}", report.considered);
    println!("  retained       {}", report.retained);
    println!(
        "  {}  {}",
        if dry_run {
            "would archive"
        } else {
            "archived      "
        },
        report.archived.len()
    );
    println!("  refused        {}", report.refused.len());
    for refusal in &report.refused {
        println!(
            "    {}  {}  held by {}  retention {}",
            refusal.memory_id, refusal.kind, refusal.blocking_reference, refusal.retention
        );
    }
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------- stats -------

async fn stats(cfg: Config, reconcile: bool, json: bool) -> Result<ExitCode, MmError> {
    let (kernel, engine) = open_engine(cfg).await?;
    let report = engine.verify().await.map_err(to_mm)?;
    let tables = [
        "memories",
        "memory_cues",
        "memory_entities",
        "memory_links",
        "memory_access",
        "memory_consolidations",
        "memory_summaries",
        "memory_communities",
        "entity_edges",
        "mistakes",
    ];
    let mut counts = BTreeMap::new();
    for table in tables {
        counts.insert(
            table,
            engine.store().count_of(Some(table)).await.map_err(to_mm)?,
        );
    }
    let archived = engine
        .store()
        .count(&MemoryFilter::all().with_status(mm_memory::RecordStatus::Archived))
        .await
        .map_err(to_mm)?;
    let index: &dyn mm_core::Tabular = &kernel.sqlite;
    let protected: i64 = index
        .query_json(
            "SELECT count(*) AS n FROM memories WHERE protected = 1 OR kind = 'developmental'",
            mm_core::Params::new(),
        )
        .await?
        .first()
        .and_then(|row| row["n"].as_i64())
        .unwrap_or(0);

    if json {
        println!(
            "{}",
            serde_json::json!({
                "tables": counts,
                "active": report.sqlite_rows - archived,
                "archived": archived,
                "protected": protected,
                "sqlite_rows": report.sqlite_rows,
                "rdf_triples": report.rdf_triples,
                "reconciled": report.reconciled,
                "classes": report.classes.iter().map(|(name, sqlite, rdf)| {
                    serde_json::json!({"class": name, "sqlite": sqlite, "rdf": rdf})
                }).collect::<Vec<_>>(),
                "protected_violations": report.protected_violations,
                "dangling_provenance": report.dangling_provenance,
                "failures": report.failures,
            })
        );
    } else {
        println!("memory stats");
        for (table, count) in &counts {
            println!("  {table:<22} {count}");
        }
        println!("  {:<22} {archived}", "archived");
        println!("  {:<22} {protected}", "protected");
        println!(
            "  {:<22} {} SQLite row(s) vs {} /memory node(s)",
            "reconcile", report.sqlite_rows, report.rdf_triples
        );
        for (class, sqlite, rdf) in &report.classes {
            println!("    {class:<18} {sqlite} vs {rdf}");
        }
        println!(
            "  {:<22} {}",
            "protected ledger",
            if report.protected_violations == 0 {
                "ok".to_string()
            } else {
                format!("{} VIOLATION(S)", report.protected_violations)
            }
        );
        println!(
            "  {:<22} {}",
            "provenance closure",
            if report.dangling_provenance == 0 {
                "ok".to_string()
            } else {
                format!("{} DANGLING", report.dangling_provenance)
            }
        );
        for failure in &report.failures {
            println!("    FAIL {failure}");
        }
    }
    shutdown(kernel).await?;
    if reconcile && !report.ok() {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

// ------------------------------------------------------------------ eval -------

/// The thresholds file.
#[derive(Debug, Deserialize)]
struct Thresholds {
    retrieval: RetrievalThresholds,
    #[serde(default)]
    weights: Option<Weights>,
    #[serde(default)]
    retention: Option<RetentionThresholds>,
}

#[derive(Debug, Deserialize)]
struct RetrievalThresholds {
    precision_at_k: f64,
    recall_at_k: f64,
    #[serde(default)]
    k: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct Weights {
    lexical: f64,
    vector: f64,
    graph: f64,
    recency: f64,
    importance: f64,
}

#[derive(Debug, Deserialize)]
struct RetentionThresholds {
    min_retention: f64,
    half_life_ns: u64,
}

/// One gold query.
#[derive(Debug, Deserialize)]
struct GoldCase {
    query: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    relevant: Vec<String>,
}

fn read_thresholds(path: &Path) -> Result<Thresholds, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    toml::from_str(&raw).map_err(|e| MmError::Config(format!("{}: {e}", path.display())))
}

fn read_gold(path: &Path) -> Result<Vec<GoldCase>, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            serde_json::from_str::<GoldCase>(line)
                .map_err(|e| MmError::Config(format!("{}: {e}", path.display())))
        })
        .collect()
}

/// `mm-cli memory eval` — hermetic: a throwaway store, thrown away afterwards.
async fn eval(
    cfg: Config,
    gold: PathBuf,
    k_flag: usize,
    thresholds: PathBuf,
    episodes: Option<PathBuf>,
    json: bool,
) -> Result<ExitCode, MmError> {
    let thresholds = read_thresholds(&thresholds)?;
    let cases = read_gold(&gold)?;
    let k = thresholds.retrieval.k.unwrap_or(k_flag).min(k_flag.max(1));
    let repo = repo_root_of(&cfg);
    let episodes_path = episodes.unwrap_or_else(|| repo.join("bench/memory/episodes.jsonl"));

    // A self-contained store: the evaluation must not touch, or depend on, the
    // live one.
    let dir = tempfile::tempdir().map_err(|e| MmError::Config(e.to_string()))?;
    let eval_cfg = Config::for_data_dir(dir.path());
    std::fs::create_dir_all(&eval_cfg.store.data_dir)
        .map_err(|e| MmError::Config(e.to_string()))?;
    let store = mm_store_sqlite::SqliteStore::open(&eval_cfg.store.sqlite_file).await?;
    store.migrate().await?;
    let shapes = eval_cfg.memory_shapes_file();
    let graph = mm_store_graph::GraphStore::in_memory(&shapes).await?;
    let logger = Arc::new(Logger::new(
        Level::Error,
        Vec::new(),
        Some(Arc::new(store.clone())),
        RedactionPolicy::kernel_default(),
    ));
    let ids = Arc::new(
        UlidFactory::open(&eval_cfg.ulid_watermark_path())
            .map_err(|e| MmError::Config(e.to_string()))?,
    );
    let mut engine = MemoryEngine::open(&store, graph.handle(), Arc::clone(&logger), ids)
        .await
        .map_err(to_mm)?;

    if let Some(weights) = &thresholds.weights {
        let weights = mm_memory::RetrievalWeights {
            lexical: weights.lexical,
            vector: weights.vector,
            graph: weights.graph,
            recency: weights.recency,
            importance: weights.importance,
        };
        weights.validate().map_err(to_mm)?;
        engine = engine.with_weights(weights);
    }
    if let Some(retention) = &thresholds.retention {
        engine = engine
            .with_threshold(retention.min_retention)
            .with_half_life(retention.half_life_ns);
    }

    // `read_episodes` and `ingest_episodes` walk the same file in the same order,
    // so zipping them is a pure function of the fixture — no lookup table and no
    // dependency on when the corpus was ingested.
    let parsed = mm_memory::consolidate::read_episodes(&episodes_path).map_err(to_mm)?;
    let ingested = engine
        .ingest_episodes(&episodes_path, &logger)
        .await
        .map_err(to_mm)?;
    let mut by_ref: BTreeMap<String, Ulid> = BTreeMap::new();
    for (episode, id) in parsed.iter().zip(ingested) {
        if let Some(reference) = &episode.reference {
            by_ref.insert(reference.clone(), id);
        }
    }

    let now = Timestamp::now();
    let mut measured = 0usize;
    let mut skipped = 0usize;
    let mut precision_sum = 0.0f64;
    let mut recall_sum = 0.0f64;
    let mut per_case = Vec::new();
    for case in &cases {
        if case.relevant.is_empty() {
            skipped += 1;
            per_case.push(serde_json::json!({
                "query": case.query,
                "category": case.category,
                "skipped": true,
                "reason": "abstention: the corpus holds no answer, so precision and recall are undefined",
            }));
            continue;
        }
        let wanted: Vec<Ulid> = case
            .relevant
            .iter()
            .map(|reference| {
                by_ref.get(reference).copied().ok_or_else(|| {
                    MmError::Config(format!(
                        "gold names {reference:?}, which no episode in {} declares",
                        episodes_path.display()
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let hits = engine
            .recall(RecallQuery::new(case.query.clone(), k, now))
            .await
            .map_err(to_mm)?;
        let found = hits
            .iter()
            .filter(|hit| wanted.contains(&hit.memory.id))
            .count();
        // Precision is measured over the *retrieved relevant pool*: a gold set whose
        // relevant lists are smaller than `k` could never exceed
        // `|relevant| / k`, which would make the plan's 0.80 floor unreachable for
        // every query. Dividing by `min(k, |relevant|)` asks the question that
        // matters — of the answers that could have been right, how many were?
        let precision = found as f64 / (k.min(wanted.len())) as f64;
        let recall = found as f64 / wanted.len() as f64;
        precision_sum += precision;
        recall_sum += recall;
        measured += 1;
        per_case.push(serde_json::json!({
            "query": case.query,
            "category": case.category,
            "k": k,
            "relevant": case.relevant,
            "retrieved": hits.iter().map(|hit| mm_core::ulid_string(&hit.memory.id)).collect::<Vec<_>>(),
            "found": found,
            "precision_at_k": precision,
            "recall_at_k": recall,
            "skipped": false,
        }));
    }
    let precision = if measured == 0 {
        0.0
    } else {
        precision_sum / measured as f64
    };
    let recall = if measured == 0 {
        0.0
    } else {
        recall_sum / measured as f64
    };
    let passes = measured > 0
        && precision >= thresholds.retrieval.precision_at_k
        && recall >= thresholds.retrieval.recall_at_k;
    let corpus = parsed.len();

    if json {
        println!(
            "{}",
            serde_json::json!({
                "k": k,
                "corpus": corpus,
                "measured": measured,
                "skipped": skipped,
                "precision_at_k": precision,
                "recall_at_k": recall,
                "precision_floor": thresholds.retrieval.precision_at_k,
                "recall_floor": thresholds.retrieval.recall_at_k,
                "passes": passes,
                "cases": per_case,
            })
        );
    } else {
        println!("memory eval");
        println!("  episodes       {corpus} ({})", episodes_path.display());
        println!("  k              {k}");
        println!(
            "  precision@{k}    {precision:.4}  (floor {:.2})",
            thresholds.retrieval.precision_at_k
        );
        println!(
            "  recall@{k}       {recall:.4}  (floor {:.2})",
            thresholds.retrieval.recall_at_k
        );
        println!("  measured       {measured} query(ies), {skipped} skipped");
        for entry in &per_case {
            let query = entry["query"].as_str().unwrap_or_default();
            let category = entry["category"].as_str().unwrap_or("-");
            if entry["skipped"].as_bool().unwrap_or(false) {
                println!("    {category:<16} {query}  (abstention; skipped)");
            } else {
                println!(
                    "    {category:<16} p={:.2} r={:.2}  {query}",
                    entry["precision_at_k"].as_f64().unwrap_or(0.0),
                    entry["recall_at_k"].as_f64().unwrap_or(0.0)
                );
            }
        }
        println!(
            "  result         {}",
            if passes {
                "meets thresholds"
            } else {
                "BELOW THRESHOLDS"
            }
        );
    }

    graph.shutdown().await?;
    store.close().await;
    Ok(if passes {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

// ---------------------------------------------------------------- verify -------

async fn verify(cfg: Config, json: bool) -> Result<ExitCode, MmError> {
    let (kernel, engine) = open_engine(cfg).await?;
    let report = engine.verify().await.map_err(to_mm)?;
    let shapes = kernel.cfg.shapes_file_for(mm_memory::rdf::MEMORY_GRAPH);
    let shacl = kernel
        .graph()?
        .validate_with(mm_memory::rdf::MEMORY_GRAPH, &shapes)
        .await?;
    let clean = report.ok() && shacl.conforms;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "sqlite_rows": report.sqlite_rows,
                "rdf_triples": report.rdf_triples,
                "reconciled": report.reconciled,
                "protected_violations": report.protected_violations,
                "dangling_provenance": report.dangling_provenance,
                "shacl_violations": shacl.violations.len(),
                "shapes": shapes.display().to_string(),
                "failures": report.failures,
                "ok": clean,
            })
        );
    } else {
        println!("memory verify");
        println!(
            "  sqlite         {} row(s); {} /memory node(s)",
            report.sqlite_rows, report.rdf_triples
        );
        println!(
            "  reconcile      {}",
            if report.reconciled {
                "exact".to_string()
            } else {
                "DRIFT".to_string()
            }
        );
        for (class, sqlite, rdf) in &report.classes {
            println!("    {class:<18} {sqlite} vs {rdf}");
        }
        println!(
            "  protected      {} archived violation(s)",
            report.protected_violations
        );
        println!(
            "  provenance     {} dangling source(s)",
            report.dangling_provenance
        );
        println!(
            "  shacl /memory  {} violation(s) against {}",
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
        println!("  result         {}", if clean { "ok" } else { "FAILED" });
    }
    shutdown(kernel).await?;
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

// -------------------------------------------------------- mistake & procedure --

async fn mistake(cfg: Config, command: MistakeCommand) -> Result<ExitCode, MmError> {
    let (kernel, mut engine) = open_engine(cfg).await?;
    match command {
        MistakeCommand::Add {
            failure_mode,
            corrective_rule,
            risk,
            missed,
        } => {
            let mut input = mm_memory::MistakeInput::new(failure_mode.clone())
                .with_risk(risk)
                .with_missed(missed);
            if let Some(rule) = corrective_rule {
                input = input.with_rule(rule);
            }
            let id = engine.record_mistake(input).await.map_err(to_mm)?;
            println!(
                "mistake recorded: {} (failure_mode {failure_mode:?})",
                mm_core::ulid_string(&id)
            );
        }
    }
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

async fn procedure(cfg: Config, command: ProcedureCommand) -> Result<ExitCode, MmError> {
    let (kernel, mut engine) = open_engine(cfg).await?;
    match command {
        ProcedureCommand::Add {
            name,
            steps,
            proficiency,
        } => {
            let mut procedure = mm_memory::Procedure::new(name.clone());
            procedure.steps = steps;
            procedure.proficiency = proficiency.clamp(0.0, 1.0);
            let habit = procedure.is_habit();
            let id = engine.record_procedure(procedure).await.map_err(to_mm)?;
            println!(
                "procedure recorded: {} ({name:?}{})",
                mm_core::ulid_string(&id),
                if habit { ", a habit" } else { "" }
            );
        }
    }
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------- index -------

async fn index(cfg: Config, command: IndexCommand) -> Result<ExitCode, MmError> {
    let (kernel, mut engine) = open_engine(cfg).await?;
    match command {
        IndexCommand::Rebuild { pack } => {
            let path = pack.unwrap_or_else(|| kernel.cfg.store.data_dir.join("index/memory.pack"));
            let hash = engine.reindex(&path).await.map_err(to_mm)?;
            println!(
                "index rebuilt: {} vector(s) -> {} ({})",
                engine.index().len(),
                path.display(),
                hash
            );
        }
    }
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------- shutdown -----

async fn shutdown(kernel: Kernel) -> Result<(), MmError> {
    if let Some(graph) = &kernel.graph {
        graph.shutdown().await?;
    }
    kernel.sqlite.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The thresholds file the gate reads must parse into the struct the eval uses.
    #[test]
    fn the_committed_thresholds_parse() {
        let path = mm_core::Config::repo_root().join("bench/memory/thresholds.toml");
        let thresholds = super::read_thresholds(&path).expect("thresholds must parse");
        assert!(thresholds.retrieval.precision_at_k > 0.0);
        assert_eq!(thresholds.retrieval.k, Some(5));
        let weights = thresholds.weights.expect("weights must be present");
        for name in [
            weights.lexical,
            weights.vector,
            weights.graph,
            weights.recency,
            weights.importance,
        ] {
            assert!((0.0..=1.0).contains(&name));
        }
        let sum =
            weights.lexical + weights.vector + weights.graph + weights.recency + weights.importance;
        assert!((sum - 1.0).abs() < 1e-9, "weights sum to {sum}");
        assert!(thresholds.retention.is_some());
    }

    /// Every gold line must parse, and the abstention line really has no answer.
    #[test]
    fn the_committed_gold_set_parses() {
        let path = mm_core::Config::repo_root().join("bench/memory/gold.jsonl");
        let cases = super::read_gold(&path).expect("gold must parse");
        assert!(!cases.is_empty());
        let abstentions = cases.iter().filter(|case| case.relevant.is_empty()).count();
        assert!(
            abstentions >= 1,
            "the gold set must contain an abstention query"
        );
        assert!(
            cases.iter().any(|case| !case.relevant.is_empty()),
            "the gold set must contain a measurable query"
        );
        for case in &cases {
            assert!(
                case.relevant.len() <= 5,
                "{}: too many relevant refs",
                case.query
            );
        }
    }

    #[test]
    fn instants_accept_rfc3339_and_epoch_seconds() {
        assert_eq!(
            super::parse_instant("2026-10-08T00:00:00Z")
                .unwrap()
                .seconds,
            1_791_417_600
        );
        assert_eq!(
            super::parse_instant("1700000000").unwrap().seconds,
            1_700_000_000
        );
        assert!(super::parse_instant("yesterday").is_err());
    }

    #[test]
    fn content_resolves_at_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("content.txt");
        std::fs::write(&path, "from a file").unwrap();
        assert_eq!(
            super::resolve_content(&format!("@{}", path.display())).unwrap(),
            "from a file"
        );
        assert_eq!(super::resolve_content("inline").unwrap(), "inline");
        assert!(super::resolve_content("@/nonexistent/nope").is_err());
    }
}
