//! `mm-cli tool | action | verify | mcp` — Phase 10's operator surface.
//!
//! Every command here goes through the same door a runtime call does:
//!
//! * `tool run` builds an [`mm_tools::Executor`] over the kernel's store and graph and
//!   calls `execute_with_key`. Authorization, the capability guard, the snapshot, the
//!   observation, the ledger and the `/world` claim are the executor's, so the CLI
//!   cannot take a shortcut the runtime would not have.
//! * `policy check` builds the *same* [`mm_tools::executor::load_engine`] the executor
//!   uses and asks it one question. A check that re-implemented the rule order would be
//!   a second opinion, and the whole claim of a deterministic permission engine is that
//!   there is one.
//! * `action ledger --verify` runs the same `verify_chain` the executor runs on open.
//! * `verify run` answers a named obligation and prints a proof, never prose.
//!
//! # Exit codes, and why `policy check` is the exception
//!
//! `tool run` exits non-zero when the action was not `ok`: a denied or failed action is
//! a failure of the command. `policy check` and `tool list` exit zero whenever they
//! *reported* an answer, because their output is the answer — a `deny` line is a
//! successful check, and a check that exited non-zero on deny could not be put in a
//! pipeline at all. `policy check --assert-allow` is the form that exits non-zero on a
//! non-permitting decision, for a gate that wants one.
//!
//! # The sandbox root
//!
//! [`mm_tools::SANDBOX_ROOT`] is `data/sandbox`, relative to the working directory, and
//! `data/` is not committed (it is kernel state). [`ensure_sandbox_root`] therefore
//! creates the root and the fixture the gate reads, so `tool run fs.read --arg
//! path=data/sandbox/README.md` works from a fresh checkout instead of failing on a
//! missing file — a missing fixture would look like a sandbox denial, which is the wrong
//! lesson to learn from it.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::Subcommand;
use mm_core::{Config, MmError, Timestamp, Ulid};
use mm_tools::verify::{obligation_for, ObligationKind, Proof, Verdict};
use mm_tools::{ActionSpec, ActionStatus, Executor, Principal, ToolName};

use crate::kernel::Kernel;

/// `mm-cli tool`.
#[derive(Subcommand, Debug)]
pub enum ToolCommand {
    /// Every registered tool, with its safety declarations.
    List {
        /// Print a single JSON array.
        #[arg(long)]
        json: bool,
    },
    /// One tool's full contract, including the digest policy rules are written against.
    Describe {
        /// The tool's name, e.g. `fs.read`.
        #[arg(value_name = "NAME")]
        name: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Run one tool through the executor.
    Run {
        /// The tool's name, e.g. `fs.read`.
        #[arg(value_name = "NAME")]
        name: String,
        /// An argument as `key=value`; repeatable. A value that parses as JSON is that
        /// value, so `--arg max_bytes=1024` is a number and `--arg text=hello` is a string.
        #[arg(long = "arg", value_name = "KEY=VALUE")]
        args: Vec<String>,
        /// Who is acting. Defaults to `system`.
        #[arg(long, value_name = "PRINCIPAL")]
        principal: Option<String>,
        /// The idempotency key. Omitting it derives one from the call, so a verbatim
        /// retry is still exactly-once.
        #[arg(long, value_name = "KEY")]
        idempotency_key: Option<String>,
        /// A rationale, recorded in the ledger and never used to authorize.
        #[arg(long, value_name = "TEXT")]
        rationale: Option<String>,
        /// Print the recorded outcome as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli action`.
#[derive(Subcommand, Debug)]
pub enum ActionCommand {
    /// The action ledger, newest last.
    Ledger {
        /// How many entries to print.
        #[arg(long, default_value_t = 20)]
        limit: i64,
        /// Print a JSON array instead of lines.
        #[arg(long)]
        json: bool,
        /// Verify the hash chain and report whether it is intact.
        #[arg(long)]
        verify: bool,
    },
    /// One action's record: its call row, its ledger entries and its observations.
    Show {
        /// The action's ULID.
        #[arg(value_name = "ACTION_ID")]
        action_id: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Restore every snapshot an action took, byte for byte.
    Rollback {
        /// The action's ULID.
        #[arg(value_name = "ACTION_ID")]
        action_id: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli verify`.
#[derive(Subcommand, Debug)]
pub enum VerifyCommand {
    /// Discharge one obligation and print its proof.
    Run {
        /// `cargo-build`, `cargo-test`, `static-analysis`, `symbolic` or `math-residual`.
        #[arg(value_name = "KIND")]
        kind: String,
        /// The package name, or a path for a file-backed check.
        #[arg(long, value_name = "SUBJECT")]
        subject: String,
        /// Print the proof as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// `mm-cli mcp`.
#[derive(Subcommand, Debug)]
pub enum McpCommand {
    /// The `tools/list` result: every tool, in MCP's shape.
    List {
        /// Print the raw JSON-RPC result.
        #[arg(long)]
        json: bool,
    },
    /// Call one tool over the bridge, exactly as an MCP client would.
    Call {
        /// The tool's name.
        #[arg(value_name = "NAME")]
        name: String,
        /// The principal the call acts as. Required: the bridge will not act as the
        /// kernel on a client's behalf.
        #[arg(long, value_name = "PRINCIPAL")]
        principal: String,
        /// An argument as `key=value`; repeatable.
        #[arg(long = "arg", value_name = "KEY=VALUE")]
        args: Vec<String>,
        /// Print the JSON-RPC reply.
        #[arg(long)]
        json: bool,
    },
    /// Serve `tools/list` and `tools/call` over stdio, one JSON-RPC message per line.
    Serve,
}

/// The sandbox root, made absolute against the repository.
fn sandbox_root(cfg: &Config) -> PathBuf {
    let root = cfg.root.join(mm_tools::SANDBOX_ROOT);
    ensure_sandbox_root(&root);
    root
}

/// Create the sandbox root and its fixture if they are absent.
///
/// The fixture is written by the kernel, not committed, because `data/` is kernel state:
/// it is ignored by git, and a gate line that read a committed file would be reading
/// something the sandbox is supposed to own. A failure to create it is not fatal here —
/// the capability guard will refuse the read with a typed error, which is the honest
/// outcome — so the result is deliberately ignored.
fn ensure_sandbox_root(root: &Path) {
    let _ = std::fs::create_dir_all(root.join("forbidden"));
    let readme = root.join("README.md");
    if !readme.exists() {
        let _ = std::fs::write(
            &readme,
            "# the sandbox root\n\n\
             This directory is the *only* tree a tier-1 tool may reach: every filesystem\n\
             capability a tool declares is a restriction of it, and `CapabilityGuard`\n\
             resolves each requested path against it. It is kernel state, not a source\n\
             directory, which is why it is ignored by git and why the CLI creates it when\n\
             it is missing.\n\n\
             `forbidden/` is here to make one policy rule reachable: the baseline policy\n\
             set permits `fs.write` on `data/sandbox/**` and explicitly forbids it on\n\
             `data/sandbox/forbidden/**`, so a write into that subdirectory is refused by\n\
             the rule rather than by the absence of a grant. That is what an explicit\n\
             `forbid` winning over a `permit` looks like from the outside.\n",
        );
    }
    let marker = root.join("forbidden").join("DO-NOT-WRITE.md");
    if !marker.exists() {
        let _ = std::fs::write(
            &marker,
            "Writes under data/sandbox/forbidden/** are refused by an explicit policy rule.\n",
        );
    }
}

/// An open kernel with the graph store, which every command here needs.
async fn open(cfg: Config) -> Result<Kernel, MmError> {
    Kernel::open(cfg, true).await
}

/// The executor the CLI drives: the shipped registry, the store's own grants and policy,
/// and the observer that admits observations to `/world`.
async fn build_executor(cfg: &Config, kernel: &Kernel) -> Result<Executor, MmError> {
    let registry = mm_tools::tools::default_registry()
        .map_err(|e| MmError::Internal(format!("the shipped tool registry is invalid: {e}")))?;
    registry
        .persist(&kernel.sqlite)
        .await
        .map_err(|e| MmError::Store(format!("could not persist the tool registry: {e}")))?;
    let engine = mm_tools::executor::load_engine(&kernel.sqlite).await?;
    let store = Arc::new(mm_epistemic::SqliteEpistemicStore::new(
        kernel.sqlite.clone(),
        kernel.logger.clone(),
        kernel.ids.clone(),
    ));
    let observer = Arc::new(mm_epistemic::EpistemicEngine::new(
        store,
        kernel.graph()?.handle().clone(),
    ));
    Ok(Executor::new(
        registry,
        Arc::new(engine),
        kernel.sqlite.clone(),
        kernel.graph()?.handle().clone(),
        kernel.ids.clone(),
    )
    .with_sandbox_root(sandbox_root(cfg))
    .with_observer(observer))
}

/// Parse `key=value` pairs into an arguments object.
///
/// A value that parses as JSON becomes that JSON value, so `--arg max_bytes=1024` is a
/// number and `--arg text=hello` is the string `hello`. The alternative — treating every
/// value as a string — cannot express a number, and guessing by looking at the schema
/// would make the same command mean different things for two tools.
fn parse_args(pairs: &[String]) -> Result<serde_json::Value, MmError> {
    let mut object = serde_json::Map::new();
    for pair in pairs {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| MmError::Config(format!("argument {pair:?} is not `key=value`")))?;
        let parsed = serde_json::from_str(value).unwrap_or_else(|_| serde_json::json!(value));
        object.insert(key.to_string(), parsed);
    }
    Ok(serde_json::Value::Object(object))
}

/// `mm-cli tool`.
pub async fn run_tool(cfg: Config, command: ToolCommand) -> Result<ExitCode, MmError> {
    match command {
        ToolCommand::List { json } => {
            let registry = mm_tools::tools::default_registry().map_err(|e| {
                MmError::Internal(format!("the shipped tool registry is invalid: {e}"))
            })?;
            let specs = registry.list();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&specs)
                        .map_err(|e| MmError::Internal(e.to_string()))?
                );
            } else {
                for spec in &specs {
                    println!(
                        "{:<16} {:<8} {:<14} {:<14} {}",
                        spec.name.as_str(),
                        spec.version,
                        spec.reversibility.as_str(),
                        spec.sandbox_tier.as_str(),
                        spec.description
                    );
                }
                println!("{} tool(s)", specs.len());
            }
            Ok(ExitCode::SUCCESS)
        }
        ToolCommand::Describe { name, json } => {
            let tool = ToolName::new(name).map_err(|e| MmError::Config(e.to_string()))?;
            let registry = mm_tools::tools::default_registry().map_err(|e| {
                MmError::Internal(format!("the shipped tool registry is invalid: {e}"))
            })?;
            let spec = registry
                .describe(&tool)
                .map_err(|e| MmError::Config(e.to_string()))?;
            if json {
                let mut value =
                    serde_json::to_value(&spec).map_err(|e| MmError::Internal(e.to_string()))?;
                if let Some(object) = value.as_object_mut() {
                    object.insert("digest".into(), serde_json::json!(spec.digest()));
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value)
                        .map_err(|e| MmError::Internal(e.to_string()))?
                );
            } else {
                println!("{} {}", spec.name, spec.version);
                println!("{}", spec.description);
                println!("reversibility   {}", spec.reversibility);
                println!("side effects    {}", spec.side_effects);
                println!("sandbox tier    {}", spec.sandbox_tier);
                println!("module          {}", spec.module_uri);
                println!("digest          {}", spec.digest());
                for permission in &spec.permissions {
                    println!("capability      {}", permission.canonical());
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        ToolCommand::Run {
            name,
            args,
            principal,
            idempotency_key,
            rationale,
            json,
        } => {
            let tool = ToolName::new(name).map_err(|e| MmError::Config(e.to_string()))?;
            let principal = Principal(principal.unwrap_or_else(|| "system".to_string()));
            principal.validate().map_err(MmError::Config)?;
            let mut action = ActionSpec::new(tool, parse_args(&args)?, principal);
            action.rationale = rationale;
            let key = idempotency_key.map(mm_tools::IdempotencyKey);

            let kernel = open(cfg.clone()).await?;
            let executor = build_executor(&cfg, &kernel).await?;
            let result = executor
                .execute_with_key(action, key)
                .await
                .map_err(|e| MmError::Internal(e.to_string()))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result)
                        .map_err(|e| MmError::Internal(e.to_string()))?
                );
            } else {
                println!("status          {}", result.status);
                println!(
                    "action_id       {}",
                    mm_core::ulid_string(&result.action_id)
                );
                if let Some(tier) = &result.sandbox_tier {
                    println!("sandbox tier    {tier}");
                }
                if let Some(observation) = result.observation_id {
                    println!("observation     {}", mm_core::ulid_string(&observation));
                }
                if let Some(evidence) = result.evidence_id {
                    println!("evidence        {}", mm_core::ulid_string(&evidence));
                }
                if result.deduplicated {
                    println!("deduplicated    true (an identical call already ran)");
                }
                if let Some(reason) = &result.reason {
                    println!("reason          {reason}");
                }
                if let Some(output) = &result.output {
                    println!("output          {}", mm_tools::canonical_json(output));
                }
            }
            Ok(exit_for_status(result.status))
        }
    }
}

/// A status's exit code: success only for a call that ran and produced a result.
fn exit_for_status(status: ActionStatus) -> ExitCode {
    match status {
        ActionStatus::Ok | ActionStatus::Deduplicated => ExitCode::SUCCESS,
        _ => ExitCode::from(1),
    }
}

/// `mm-cli action`.
pub async fn run_action(cfg: Config, command: ActionCommand) -> Result<ExitCode, MmError> {
    match command {
        ActionCommand::Ledger {
            limit,
            json,
            verify,
        } => {
            let kernel = open(cfg).await?;
            if verify {
                let entries = mm_tools::verify_chain(&kernel.sqlite)
                    .await
                    .map_err(|e| MmError::Store(e.to_string()))?;
                println!("chain ok ({entries} entries)");
                return Ok(ExitCode::SUCCESS);
            }
            let entries = mm_tools::ledger::tail(&kernel.sqlite, limit)
                .await
                .map_err(|e| MmError::Store(e.to_string()))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&entries)
                        .map_err(|e| MmError::Internal(e.to_string()))?
                );
            } else {
                for entry in &entries {
                    println!(
                        "{} {:26} {:<22} {}",
                        entry.seq,
                        mm_core::ulid_string(&entry.action_id),
                        entry.event,
                        entry.entry_hash
                    );
                }
                println!("{} entr(ies)", entries.len());
            }
            Ok(ExitCode::SUCCESS)
        }
        ActionCommand::Show { action_id, json } => {
            let id = parse_ulid(&action_id)?;
            let kernel = open(cfg).await?;
            let entries = mm_tools::ledger::entries_for(&kernel.sqlite, &id)
                .await
                .map_err(|e| MmError::Store(e.to_string()))?;
            let calls: Vec<(String, String, String, String, Option<i64>)> = sqlx_query_as(
                &kernel.sqlite,
                "SELECT tool_name, status, permission_decision, sandbox_tier, latency_ms \
                 FROM tool_calls WHERE id = ?",
                mm_core::ulid_string(&id),
            )
            .await?;
            let observations: Vec<(String, String, String)> = sqlx_query_as(
                &kernel.sqlite,
                "SELECT id, source, evidence_id FROM observed_payloads WHERE action_id = ?",
                mm_core::ulid_string(&id),
            )
            .await?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "action_id": mm_core::ulid_string(&id),
                        "calls": calls,
                        "observations": observations,
                        "ledger": entries,
                    }))
                    .map_err(|e| MmError::Internal(e.to_string()))?
                );
            } else {
                if calls.is_empty() {
                    println!("no tool_calls row for {}", mm_core::ulid_string(&id));
                }
                for (tool, status, decision, tier, latency) in &calls {
                    println!("tool            {tool}");
                    println!("status          {status}");
                    println!("decision        {decision}");
                    println!("sandbox tier    {tier}");
                    println!(
                        "latency         {} ms",
                        latency.map_or_else(|| "-".to_string(), |v| v.to_string())
                    );
                }
                for entry in &entries {
                    println!("{} {:<22} {}", entry.seq, entry.event, entry.entry_hash);
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        ActionCommand::Rollback { action_id, json } => {
            let id = parse_ulid(&action_id)?;
            let kernel = open(cfg).await?;
            let restored = mm_tools::rollback::rollback(&kernel.sqlite, &id)
                .await
                .map_err(|e| MmError::Store(e.to_string()))?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "action_id": mm_core::ulid_string(&id),
                        "restored_hash": restored,
                    })
                );
            } else {
                println!("restored {} to {restored}", mm_core::ulid_string(&id));
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// A tiny `query_as` helper so this module does not need sqlx's macro machinery.
async fn sqlx_query_as<T>(
    sql: &mm_store_sqlite::SqliteStore,
    query: &str,
    bind: String,
) -> Result<Vec<T>, MmError>
where
    T: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
{
    sqlx::query_as::<_, T>(query)
        .bind(bind)
        .fetch_all(sql.pool())
        .await
        .map_err(|e| MmError::Store(e.to_string()))
}

/// Parse a ULID argument.
fn parse_ulid(text: &str) -> Result<Ulid, MmError> {
    mm_core::id::parse_ulid(text).map_err(|e| MmError::Config(format!("invalid ULID: {e}")))
}

/// `mm-cli verify`.
pub async fn run_verify(cfg: Config, command: VerifyCommand) -> Result<ExitCode, MmError> {
    let VerifyCommand::Run {
        kind,
        subject,
        json,
    } = command;
    let obligation = obligation_for(&kind, &subject).ok_or_else(|| {
        MmError::Config(format!(
            "unknown obligation {kind:?} (one of: cargo-build, cargo-test, static-analysis, \
             symbolic, math-residual) or an unusable subject"
        ))
    })?;
    let proof = match obligation.kind {
        ObligationKind::Symbolic => symbolic_proof(&cfg, obligation)?,
        ObligationKind::MathResidual => Proof::new(
            obligation,
            Verdict::Inconclusive,
            "no residual fixture format is implemented in this build; this is not a \
             consistency claim",
            None,
            mm_core::content_hash(subject.as_bytes()),
        ),
        _ => command_proof(&cfg, obligation)?,
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&proof).map_err(|e| MmError::Internal(e.to_string()))?
        );
    } else {
        println!("obligation      {}", proof.obligation.canonical());
        println!("verdict         {}", proof.verdict);
        println!("detail          {}", proof.detail);
        if let Some(counterexample) = &proof.counterexample {
            println!("counterexample  {counterexample}");
        }
        println!("artifact        {}", proof.artifact_hash);
    }
    Ok(match proof.verdict {
        Verdict::Proven => ExitCode::SUCCESS,
        Verdict::Refuted => ExitCode::from(1),
        // Inconclusive is neither. Exiting 0 would let a caller read "no refutation
        // found" as "verified", which the verify module refuses to claim.
        Verdict::Inconclusive => ExitCode::from(3),
    })
}

/// The refutation check, run over a file the operator named.
///
/// This is the one place the CLI reads a file without a capability guard, and it is
/// deliberate: `verify run` is the operator asking the kernel a question about a
/// committed fixture, not a tool reaching for a path. The subject is resolved against the
/// repository root so the gate's `--subject bench/tools/...` works from anywhere.
fn symbolic_proof(
    cfg: &Config,
    obligation: mm_tools::verify::Obligation,
) -> Result<Proof, MmError> {
    let path = resolve_subject(cfg, &obligation.subject);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    let (verdict, counterexample) = mm_tools::verify::check_symbolic(&text);
    let detail = match (&verdict, &counterexample) {
        (Verdict::Refuted, Some(_)) => "unit resolution found a refutation".to_string(),
        (Verdict::Refuted, None) => "the clause set is inconsistent".to_string(),
        _ => counterexample.clone().unwrap_or_else(|| {
            "no refutation was found, which is not a proof of consistency".to_string()
        }),
    };
    Ok(Proof::new(
        obligation,
        verdict,
        detail,
        if verdict == Verdict::Refuted {
            counterexample
        } else {
            None
        },
        mm_core::content_hash(text.as_bytes()),
    ))
}

/// Run a command-backed obligation and read its exit code.
fn command_proof(cfg: &Config, obligation: mm_tools::verify::Obligation) -> Result<Proof, MmError> {
    let words = obligation
        .kind
        .command(&obligation.subject)
        .ok_or_else(|| MmError::Config(format!("{} has no command", obligation.kind)))?;
    let (program, arguments) = words
        .split_first()
        .ok_or_else(|| MmError::Config("the obligation produced an empty command".to_string()))?;
    let output = std::process::Command::new(program)
        .args(arguments)
        .current_dir(&cfg.root)
        .output()
        .map_err(|e| MmError::Config(format!("cannot run {program}: {e}")))?;
    let code = output.status.code().unwrap_or(-1);
    let digest = mm_core::content_hash(&output.stdout);
    let tail = String::from_utf8_lossy(&output.stderr);
    let tail: String = tail.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
    Ok(Proof::new(
        obligation,
        if output.status.success() {
            Verdict::Proven
        } else {
            Verdict::Refuted
        },
        format!("{program} exited {code}"),
        if output.status.success() {
            None
        } else {
            Some(format!("exit code {code}: {tail}"))
        },
        digest,
    ))
}

/// Resolve an obligation subject against the repository root when it names a path.
fn resolve_subject(cfg: &Config, subject: &str) -> PathBuf {
    let path = Path::new(subject);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cfg.root.join(path)
    }
}

/// `mm-cli mcp`.
pub async fn run_mcp(cfg: Config, command: McpCommand) -> Result<ExitCode, MmError> {
    match command {
        McpCommand::List { json } => {
            let kernel = open(cfg.clone()).await?;
            let executor = Arc::new(build_executor(&cfg, &kernel).await?);
            let bridge = mm_tools::mcp::McpBridge::new(executor, "mm-cli");
            let result = bridge.tools_list();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result)
                        .map_err(|e| MmError::Internal(e.to_string()))?
                );
            } else {
                let count = result
                    .get("tools")
                    .and_then(|tools| tools.as_array())
                    .map_or(0, Vec::len);
                for tool in result
                    .get("tools")
                    .and_then(|tools| tools.as_array())
                    .into_iter()
                    .flatten()
                {
                    println!(
                        "{}",
                        tool.get("name").and_then(|n| n.as_str()).unwrap_or("?")
                    );
                }
                println!("{count} tool(s) over MCP");
            }
            Ok(ExitCode::SUCCESS)
        }
        McpCommand::Call {
            name,
            principal,
            args,
            json,
        } => {
            let kernel = open(cfg.clone()).await?;
            let executor = Arc::new(build_executor(&cfg, &kernel).await?);
            let bridge = mm_tools::mcp::McpBridge::new(executor, "mm-cli");
            let request = serde_json::json!({
                "jsonrpc": mm_tools::mcp::JSONRPC_VERSION,
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "principal": principal,
                    "arguments": parse_args(&args)?,
                }
            });
            let reply = bridge.handle(&request).await;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&reply)
                        .map_err(|e| MmError::Internal(e.to_string()))?
                );
            } else if let Some(error) = reply.get("error") {
                println!("error           {error}");
            } else {
                println!(
                    "{}",
                    mm_tools::canonical_json(
                        reply.get("result").unwrap_or(&serde_json::Value::Null)
                    )
                );
            }
            let refused = reply.get("error").is_some()
                || reply
                    .get("result")
                    .and_then(|r| r.get("isError"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
            Ok(if refused {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        McpCommand::Serve => {
            let kernel = open(cfg.clone()).await?;
            let executor = Arc::new(build_executor(&cfg, &kernel).await?);
            let bridge = mm_tools::mcp::McpBridge::new(executor, "mm-cli");
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let line = line.map_err(|e| MmError::Config(format!("stdin: {e}")))?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let reply = match serde_json::from_str::<serde_json::Value>(trimmed) {
                    Ok(request) => bridge.handle(&request).await,
                    Err(e) => serde_json::json!({
                        "jsonrpc": mm_tools::mcp::JSONRPC_VERSION,
                        "id": serde_json::Value::Null,
                        "error": {
                            "code": mm_tools::mcp::RpcErrorCode::InvalidRequest as i32,
                            "message": format!("not a JSON-RPC message: {e}"),
                        }
                    }),
                };
                println!(
                    "{}",
                    serde_json::to_string(&reply).map_err(|e| MmError::Internal(e.to_string()))?
                );
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// `mm-cli policy check` — one deterministic answer about one request.
///
/// It lives here rather than in `policy_cmd` because the engine it asks is Phase 10's:
/// the same `load_engine` the executor builds, so the check and the run cannot disagree.
pub struct CheckArgs {
    /// Who is acting.
    pub principal: String,
    /// The capability asked for, e.g. `fs.write`: the resource kind, a dot, then the
    /// verb. The store spells the same capability `kind:verb:pattern`, so the two
    /// halves are read here in the order the dotted form writes them.
    pub action: String,
    /// The capability's pattern, e.g. `data/sandbox/x`.
    pub resource: String,
    /// The tool the caller would act through. Without it no grant can match, which is
    /// the strictest reading of the same request and the one that denies.
    pub tool: Option<String>,
    /// Print a single JSON object.
    pub json: bool,
    /// Exit non-zero unless the decision allows.
    pub assert_allow: bool,
}

/// Answer one authorization question.
pub async fn policy_check(cfg: Config, args: CheckArgs) -> Result<ExitCode, MmError> {
    let (kind, verb) = args
        .action
        .split_once('.')
        .ok_or_else(|| MmError::Config(format!("action {:?} is not `kind.verb`", args.action)))?;
    let permission_action = mm_tools::PermissionAction::parse(verb).ok_or_else(|| {
        MmError::Config(format!(
            "action {:?} names no known verb (read, write, execute, delete, connect, admin)",
            args.action
        ))
    })?;
    let resource = format!("{kind}:{verb}:{}", args.resource);
    let request = mm_tools::PermissionReq::new(resource.clone(), permission_action);
    request
        .validate()
        .map_err(|reason| MmError::Config(format!("resource {resource:?}: {reason}")))?;
    let principal = Principal(args.principal.clone());
    principal.validate().map_err(MmError::Config)?;
    let tool = args
        .tool
        .as_deref()
        .map(ToolName::new)
        .transpose()
        .map_err(|e| MmError::Config(e.to_string()))?;

    let kernel = open(cfg).await?;
    let engine = mm_tools::executor::load_engine(&kernel.sqlite).await?;
    let request = mm_tools::permissions::PermissionRequest {
        principal: &principal,
        tool: tool.as_ref(),
        req: &request,
        as_of: Timestamp::now(),
        confirmed: false,
    };
    let decision = mm_tools::PermissionEngine::authorize(&engine, &request);
    let policy_effect = mm_tools::policy::PolicyEngine::evaluate(
        mm_tools::PermissionEngine::policy_sets(&engine),
        &request,
    );
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "principal": principal.as_str(),
                "tool": tool.as_ref().map(|t| t.as_str().to_string()),
                "resource": resource,
                "decision": decision.as_str(),
                "reason": decision.reason_text(),
                "policy_effect": policy_effect.as_str(),
            }))
            .map_err(|e| MmError::Internal(e.to_string()))?
        );
    } else {
        println!("{}: {}", decision.as_str(), decision.reason_text());
        println!("policy_effect   {}", policy_effect.as_str());
    }
    if args.assert_allow && !decision.is_allow() {
        return Ok(ExitCode::from(1));
    }
    // A reported decision is a successful check, whatever the decision was. `deny` is an
    // answer, and a command that exited non-zero for it could not be piped into `grep`.
    Ok(ExitCode::SUCCESS)
}
