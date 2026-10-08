//! `mm-cli llm` — the LLM substrate operator surface.
//!
//! Every subcommand here either proves a Phase 3 guarantee or reports the ledger's
//! state, and none of them reaches into a provider: `replay` is the only command
//! that runs calls, and it runs them through a recorded session with the network
//! guard armed. The fixture commands (`schema-reject`, `grammars`) drive the
//! substrate with the mock client for the same reason — an adversarial fixture has
//! to be able to produce output a real provider would never produce.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use mm_core::{Config, MmError};
use mm_llm::accounting::{self, StatsWindow};
use mm_llm::cached;
use mm_llm::client::{CacheMode, LlmRequest, Message, Purpose, SchemaId};
use mm_llm::config::{Complexity, LlmConfig, Precision, Stakes};
use mm_llm::error::LlmError;
use mm_llm::grammar::{self, GrammarSpec};
use mm_llm::mock::{MockClient, ScriptedResponse};
use mm_llm::replay::{NetworkGuard, ReplayLlmClient, Session};
use mm_llm::routing::{Router, RoutingRequest, ThresholdRouter};
use mm_llm::service::{resolve_cache_dir, LlmService};
use mm_log::{codes, Level, LogRecord};

use crate::kernel::Kernel;

/// The target every record from this command carries.
const TARGET: &str = "mm.llm";

/// The repository root a command runs against.
///
/// A configuration loaded from `config/metamind.toml` by a relative path resolves
/// its root to the empty path, so an empty root falls back to the compiled-in
/// repository root. That keeps `llm` commands working from any directory.
fn repo_root_of(cfg: &Config) -> PathBuf {
    if cfg.root.as_os_str().is_empty() {
        Config::repo_root()
    } else {
        cfg.root.clone()
    }
}

/// Resolve `--session` / `--fixtures`, relative paths resolving against the root.
fn resolve(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn to_mm(e: LlmError) -> MmError {
    match e {
        LlmError::Config(m) => MmError::Config(m),
        LlmError::Db(m) | LlmError::Io(m) => MmError::Store(m),
        other => MmError::Internal(format!("{} ({})", other, other.code())),
    }
}

/// Load `config/llm.toml` from the repository root.
fn llm_config(root: &Path) -> Result<LlmConfig, MmError> {
    LlmConfig::load_or_default(&root.join("config").join("llm.toml")).map_err(to_mm)
}

/// Register every schema in `bench/llm/schemas.json`, when it exists.
///
/// The registry is per-run rather than global: a schema a command does not need
/// cannot silently satisfy a call that should have been refused.
fn register_bench_schemas(service: &mut LlmService, root: &Path) -> Result<usize, MmError> {
    let path = root.join("bench").join("llm").join("schemas.json");
    if !path.is_file() {
        return Ok(0);
    }
    let raw = std::fs::read_to_string(&path)?;
    let entries: std::collections::BTreeMap<String, serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    let mut count = 0;
    for (id, schema) in entries {
        service
            .register_schema(SchemaId::new(id), schema)
            .map_err(to_mm)?;
        count += 1;
    }
    Ok(count)
}

/// A service over the kernel's store, graph, and logger.
fn service_for(
    kernel: &Kernel,
    llm_cfg: &LlmConfig,
    root: &Path,
    client: Arc<dyn mm_llm::client::LlmClient>,
) -> Result<LlmService, MmError> {
    let service = LlmService::new(
        llm_cfg.clone(),
        kernel.sqlite.clone(),
        kernel.graph()?.handle().clone(),
        Arc::clone(&kernel.logger),
        client,
    )
    .map_err(to_mm)?
    .with_cache_dir(resolve_cache_dir(root, &llm_cfg.cache_dir));
    Ok(service)
}

/// The files of a fixture directory, sorted by name.
fn fixture_files(dir: &Path) -> Result<Vec<PathBuf>, MmError> {
    if !dir.is_dir() {
        return Err(MmError::Config(format!(
            "{} is not a fixture directory",
            dir.display()
        )));
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    Ok(files)
}

fn read_json(path: &Path) -> Result<serde_json::Value, MmError> {
    let raw = std::fs::read_to_string(path)?;
    serde_json::from_str(&raw)
        .map_err(|e| MmError::Config(format!("{} is not valid JSON: {e}", path.display())))
}

// ---------------------------------------------------------------- stats -------

/// `mm-cli llm stats`.
pub async fn stats(
    cfg: Config,
    since: Option<String>,
    purpose: Option<String>,
    json: bool,
    assert_complete: bool,
) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, false).await?;
    let window = match (since, purpose) {
        (Some(instant), _) => StatsWindow::Since(instant),
        (None, Some(name)) => {
            let purpose = Purpose::parse(&name)
                .ok_or_else(|| MmError::Config(format!("unknown purpose {name:?}")))?;
            StatsWindow::Purpose(purpose)
        }
        (None, None) => StatsWindow::All,
    };

    let mut incomplete: Option<String> = None;
    let complete_rows = if assert_complete {
        match accounting::assert_complete(&kernel.sqlite).await {
            Ok(rows) => Some(rows),
            Err(e) => {
                incomplete = Some(e.to_string());
                None
            }
        }
    } else {
        None
    };
    let stats = accounting::stats(&kernel.sqlite, &window)
        .await
        .map_err(to_mm)?;

    if json {
        let mut value = serde_json::to_value(&stats)?;
        if let Some(rows) = complete_rows {
            value["complete_rows"] = serde_json::json!(rows);
            value["complete"] = serde_json::json!(true);
        }
        if let Some(detail) = &incomplete {
            value["complete"] = serde_json::json!(false);
            value["complete_detail"] = serde_json::json!(detail);
        }
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("llm stats");
        println!("  calls        {}", stats.calls);
        println!(
            "  tokens       {} in / {} out",
            stats.tokens_in, stats.tokens_out
        );
        println!("  cost         {} micros", stats.cost_micros);
        println!("  cache hits   {}", stats.cache_hits);
        println!("  rejects      {}", stats.schema_rejects);
        println!("  purpose      calls  tokens_in  tokens_out  cost_micros");
        for (purpose, per) in &stats.by_purpose {
            println!(
                "  {:<12} {:>5}  {:>9}  {:>10}  {:>11}",
                purpose, per.calls, per.tokens_in, per.tokens_out, per.cost_micros
            );
        }
        if let Some(rows) = complete_rows {
            println!("  complete     {rows} row(s)");
        }
        if let Some(detail) = &incomplete {
            println!("  INCOMPLETE   {detail}");
        }
    }

    kernel.sqlite.close().await;
    if incomplete.is_some() {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

// --------------------------------------------------------------- replay -------

/// `mm-cli llm replay`.
pub async fn replay(
    cfg: Config,
    session: PathBuf,
    assert_zero_calls: bool,
) -> Result<ExitCode, MmError> {
    let root = repo_root_of(&cfg);
    let path = resolve(&root, session);
    let session = Session::load(&path).map_err(to_mm)?;
    let llm_cfg = llm_config(&root)?;
    let kernel = Kernel::open(cfg, true).await?;

    // The recorded session *is* the transport: the client can serve nothing else.
    let client = Arc::new(ReplayLlmClient::new(&session, "replay"));
    let mut service = service_for(&kernel, &llm_cfg, &root, client.clone())?;
    register_bench_schemas(&mut service, &root)?;

    kernel
        .logger
        .emit(
            LogRecord::new(Level::Info, codes::LLM_REPLAY_START, TARGET)
                .with_field("session_id", session.session_id.clone())
                .with_field("records", session.len())
                .with_field("fingerprint", session.fingerprint()),
        )
        .await?;

    // Anything that tries to reach a provider while this is held fails, so a
    // "zero provider calls" claim is checked by the thing that would refuse one.
    let permit = NetworkGuard::arm();
    let mut mismatches = 0usize;
    let mut cached = 0usize;
    for record in &session.records {
        let mut request = record.request.clone();
        // The cache policy belongs to the caller, not the recording, so the run
        // exercises the exact cache as well: a second run must serve byte-identical
        // output from the cache.
        request.cache = CacheMode::Use;
        let response = service.call(request).await.map_err(to_mm)?;
        cached += usize::from(response.cached);
        if response.text != record.response.text {
            mismatches += 1;
            eprintln!(
                "llm replay: recorded text differs for prompt_hash {}",
                cached::prompt_hash("replay", &record.response.model, &record.request)
            );
        }
    }
    let provider_calls = client.provider_calls();
    let refused = NetworkGuard::refused();
    drop(permit);

    kernel
        .logger
        .emit(
            LogRecord::new(Level::Info, codes::LLM_REPLAY_END, TARGET)
                .with_field("session_id", session.session_id.clone())
                .with_field("records", session.len())
                .with_field("provider_calls", provider_calls)
                .with_field("refused_attempts", refused)
                .with_field("mismatches", mismatches),
        )
        .await?;

    let ledger = accounting::call_count(&kernel.sqlite)
        .await
        .map_err(to_mm)?;
    println!("llm replay");
    println!("  session      {}", session.session_id);
    println!("  records      {}", session.len());
    println!("  provider      {provider_calls} call(s), {refused} refused attempt(s)");
    println!("  cached       {cached} of {}", session.len());
    println!("  mismatches   {mismatches}");
    println!("  ledger       {ledger} row(s)");

    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;

    if (assert_zero_calls && (provider_calls != 0 || refused != 0)) || mismatches > 0 {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------- verify-cache ------

/// `mm-cli llm verify-cache`.
pub async fn verify_cache(cfg: Config, strict: bool) -> Result<ExitCode, MmError> {
    let root = repo_root_of(&cfg);
    let llm_cfg = llm_config(&root)?;
    let dir = resolve_cache_dir(&root, &llm_cfg.cache_dir);
    let kernel = Kernel::open(cfg, false).await?;

    let report = match cached::verify_index(&kernel.sqlite, &dir).await {
        Ok(report) => report,
        Err(e) => {
            println!("llm verify-cache");
            println!("  FAIL  {}", e);
            kernel.sqlite.close().await;
            return Ok(ExitCode::from(1));
        }
    };

    let clean = report.is_clean();
    println!("llm verify-cache");
    println!("  dir          {}", dir.display());
    println!("  rows         {}", report.rows);
    println!("  verified     {}", report.verified);
    println!("  strays       {}", report.strays.len());
    for stray in &report.strays {
        println!("    stray      {stray}");
    }
    println!(
        "  {}",
        if clean {
            "every row resolves"
        } else {
            "NOT CLEAN"
        }
    );
    kernel.sqlite.close().await;
    if strict && !clean {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

// --------------------------------------------------------- schema-reject ------

/// The malformed fixtures: a strict schema, a payload that violates it, and the
/// failure the violation must be reported as.
#[derive(Debug, serde::Deserialize)]
struct MalformedFixture {
    name: String,
    schema: serde_json::Value,
    payload: String,
    expect: String,
}

/// `mm-cli llm schema-reject`.
///
/// Every fixture is answered by a client that returns the payload verbatim, so the
/// substrate is the only thing between a violating payload and a typed value. The
/// command fails if any fixture is accepted, or is rejected for the wrong reason.
pub async fn schema_reject(
    cfg: Config,
    fixtures: PathBuf,
    assert_all_rejected: bool,
) -> Result<ExitCode, MmError> {
    let root = repo_root_of(&cfg);
    let dir = resolve(&root, fixtures);
    let llm_cfg = llm_config(&root)?;
    let kernel = Kernel::open(cfg, true).await?;
    let files = fixture_files(&dir)?;
    if files.is_empty() {
        return Err(MmError::Config(format!(
            "{} holds no fixtures",
            dir.display()
        )));
    }

    let handle = kernel.graph()?.handle().clone();
    let before = repair_rows(&kernel).await?;
    let mut rejected = 0usize;
    let mut wrong = Vec::new();

    println!("llm schema-reject");
    for file in &files {
        let fixture: MalformedFixture = serde_json::from_value(read_json(file)?)
            .map_err(|e| MmError::Config(format!("{}: {e}", file.display())))?;
        let client = Arc::new(MockClient::constant(ScriptedResponse::text(
            fixture.payload.clone(),
        )));
        let mut service = LlmService::new(
            llm_cfg.clone(),
            kernel.sqlite.clone(),
            handle.clone(),
            Arc::clone(&kernel.logger),
            client,
        )
        .map_err(to_mm)?
        .with_cache_dir(resolve_cache_dir(&root, &llm_cfg.cache_dir));
        service
            .register_schema(SchemaId::new(fixture.name.clone()), fixture.schema.clone())
            .map_err(to_mm)?;

        let request = LlmRequest::new(Purpose::Extract, vec![Message::user("adversarial fixture")])
            .with_schema(SchemaId::new(fixture.name.clone()))
            // Bypass so each fixture always reaches the client, whatever ran before.
            .with_cache(CacheMode::Bypass);

        match service.call(request).await {
            Err(LlmError::SchemaRejected { detail, .. }) if detail.contains(&fixture.expect) => {
                rejected += 1;
                println!(
                    "  [ok]    {:<22} rejected as {}",
                    fixture.name, fixture.expect
                );
            }
            Err(LlmError::SchemaRejected { detail, .. }) => {
                wrong.push(format!(
                    "{}: rejected as the wrong kind (wanted {}, got {detail})",
                    fixture.name, fixture.expect
                ));
            }
            Err(other) => wrong.push(format!("{}: failed with {other}", fixture.name)),
            Ok(_) => wrong.push(format!("{}: ACCEPTED a violating payload", fixture.name)),
        }
    }

    let repairs = repair_rows(&kernel).await?.saturating_sub(before);
    println!("  rejected     {rejected} of {}", files.len());
    println!("  repair rows  {repairs}");
    for detail in &wrong {
        println!("  FAIL         {detail}");
    }
    kernel.graph()?.shutdown().await?;
    kernel.sqlite.close().await;

    let expectations_held = repairs == rejected as i64;
    if !expectations_held {
        eprintln!(
            "llm schema-reject: {rejected} rejection(s) but {repairs} repair row(s); \
             every rejected call must leave exactly one"
        );
    }
    if !wrong.is_empty() || (assert_all_rejected && rejected != files.len()) || !expectations_held {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

async fn repair_rows(kernel: &Kernel) -> Result<i64, MmError> {
    kernel.sqlite.row_count("llm_repair_attempts").await
}

// ---------------------------------------------------------------- routes ------

/// The labelled requirement a `routes` query describes.
///
/// A struct rather than a long argument list: the labels belong together, and it
/// keeps the command signature reviewable.
pub struct RouteArgs {
    /// One of the `Purpose` names: interpret, plan, extract, critique, summarize,
    /// code_review, classify, diagnose.
    pub purpose: String,
    /// One of: low, medium, high, very_high.
    pub complexity: String,
    /// One of: low, medium, high.
    pub stakes: String,
    /// One of: low, normal, high.
    pub precision: String,
    /// Latency ceiling in milliseconds; 0 means none.
    pub latency_budget_ms: u32,
    /// Cost ceiling in micros; 0 means none.
    pub cost_budget_micros: i64,
    /// A model to pin, if any.
    pub model: Option<String>,
}

/// `mm-cli llm routes`.
pub async fn routes(cfg: Config, args: RouteArgs, json: bool) -> Result<ExitCode, MmError> {
    let root = repo_root_of(&cfg);
    let llm_cfg = llm_config(&root)?;
    let purpose = Purpose::parse(&args.purpose)
        .ok_or_else(|| MmError::Config(format!("unknown purpose {:?}", args.purpose)))?;
    let complexity = Complexity::parse(&args.complexity)
        .ok_or_else(|| MmError::Config(format!("unknown complexity {:?}", args.complexity)))?;
    let stakes = parse_stakes(&args.stakes)?;
    let precision = parse_precision(&args.precision)?;

    let mut request = RoutingRequest::new(purpose)
        .with_complexity(complexity)
        .with_stakes(stakes)
        .with_precision(precision)
        .with_latency_budget(args.latency_budget_ms)
        .with_cost_budget(args.cost_budget_micros);
    if let Some(model) = args.model.clone() {
        request = request.with_pinned_model(model);
    }

    let router = ThresholdRouter::new(&llm_cfg);
    let decision = router.choose(&request).map_err(to_mm)?;

    if json {
        println!("{}", serde_json::to_string(&decision)?);
    } else {
        println!("llm routes");
        println!("  purpose      {}", purpose.as_str());
        println!("  complexity   {}", complexity.as_str());
        println!("  chosen       {}", decision.model);
        println!("  reason       {}", decision.reason.as_str());
        println!("  est. cost    {} micros", decision.cost_micros);
    }
    Ok(ExitCode::SUCCESS)
}

fn parse_stakes(name: &str) -> Result<Stakes, MmError> {
    match name {
        "low" => Ok(Stakes::Low),
        "medium" => Ok(Stakes::Medium),
        "high" => Ok(Stakes::High),
        other => Err(MmError::Config(format!("unknown stakes {other:?}"))),
    }
}

fn parse_precision(name: &str) -> Result<Precision, MmError> {
    match name {
        "low" => Ok(Precision::Low),
        "normal" => Ok(Precision::Normal),
        "high" => Ok(Precision::High),
        other => Err(MmError::Config(format!("unknown precision {other:?}"))),
    }
}

// -------------------------------------------------------------- grammars ------

/// A grammar fixture: a strict schema plus a value it accepts and a value a
/// permissive JSON grammar would accept but it must not.
#[derive(Debug, serde::Deserialize)]
struct GrammarFixture {
    name: String,
    schema: serde_json::Value,
    #[serde(default)]
    valid: serde_json::Value,
    #[serde(default)]
    mutated: serde_json::Value,
    #[serde(default)]
    expect_compile_error: bool,
}

/// `mm-cli llm grammars`.
///
/// Compiles every fixture schema with every backend and scores them. The point is
/// to choose a default from evidence — a backend that cannot handle the vocabulary
/// should lose here, not in production.
pub async fn grammars(
    cfg: Config,
    fixtures: PathBuf,
    score: bool,
    json: bool,
) -> Result<ExitCode, MmError> {
    let root = repo_root_of(&cfg);
    let dir = resolve(&root, fixtures);
    let files = fixture_files(&dir)?;
    if files.is_empty() {
        return Err(MmError::Config(format!(
            "{} holds no fixtures",
            dir.display()
        )));
    }

    let mut report = Vec::new();
    let mut failures = Vec::new();
    for backend in grammar::backends() {
        let mut compiled = 0usize;
        let mut conformed = 0usize;
        for file in &files {
            let fixture: GrammarFixture = serde_json::from_value(read_json(file)?)
                .map_err(|e| MmError::Config(format!("{}: {e}", file.display())))?;
            let id = SchemaId::new(fixture.name.clone());
            let outcome: Result<GrammarSpec, LlmError> = backend.compile(&id, &fixture.schema);
            match outcome {
                Ok(_) if fixture.expect_compile_error => failures.push(format!(
                    "{}: {} compiled a schema it must refuse",
                    fixture.name,
                    backend.kind().as_str()
                )),
                Ok(spec) => {
                    compiled += 1;
                    let accepts_valid = spec.accepts(&fixture.valid);
                    let rejects_mutated = !spec.accepts(&fixture.mutated);
                    if accepts_valid && rejects_mutated {
                        conformed += 1;
                    } else {
                        failures.push(format!(
                            "{}: {} accepts_valid={accepts_valid} rejects_mutated={rejects_mutated}",
                            fixture.name,
                            backend.kind().as_str()
                        ));
                    }
                }
                Err(_) if fixture.expect_compile_error => {
                    // The refusal is the conformance: a compiler that turned an
                    // unsupported construct into a *looser* grammar would be worse
                    // than one that failed.
                    compiled += 1;
                    conformed += 1;
                }
                Err(e) => failures.push(format!(
                    "{}: {} cannot compile it: {e}",
                    fixture.name,
                    backend.kind().as_str()
                )),
            }
        }
        report.push((backend.kind().as_str(), compiled, conformed, files.len()));
    }

    if json {
        let value = serde_json::json!({
            "fixtures": files.len(),
            "backends": report.iter().map(|(kind, compiled, conformed, total)| serde_json::json!({
                "decoder": kind,
                "compiled": compiled,
                "conformed": conformed,
                "fixtures": total,
            })).collect::<Vec<_>>(),
            "failures": failures,
        });
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("llm grammars");
        for (kind, compiled, conformed, total) in &report {
            println!(
                "  {:<14} compiled {compiled}/{total}  conformed {conformed}/{total}  score {:.2}",
                kind,
                if *total == 0 {
                    0.0
                } else {
                    *conformed as f64 / *total as f64
                }
            );
        }
        for detail in &failures {
            println!("  FAIL         {detail}");
        }
        if score
            && report
                .iter()
                .all(|(_, _, conformed, total)| conformed < total)
        {
            println!("  no backend handles the whole fixture set");
        }
    }
    Ok(if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}
