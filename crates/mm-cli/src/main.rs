//! `mm-cli` — the Metamind kernel operator surface.
//!
//! Every command either proves a kernel invariant or reports what state the
//! kernel is in. Nothing here reaches into a store directly: commands go through
//! `Kernel`, so no command can open a database the kernel would not open.
#![forbid(unsafe_code)]

mod being;
mod codex_cmd;
mod doctor;
mod graph_cmd;
mod kernel;
mod llm_cmd;
mod logs_cmd;
mod replay_cmd;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "mm-cli",
    version,
    about = "Metamind kernel operator CLI",
    long_about = "Opens the kernel exactly the way the runtime does and reports or verifies its state."
)]
struct Cli {
    /// Configuration file (defaults to `config/metamind.toml`, or `$MM_CONFIG`).
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Use a self-contained data directory instead of the configured one.
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Open every store, load the ontology, and prove the kernel is writable.
    Doctor,
    /// Rebuild state from the event log and report the hash it reached.
    Replay {
        /// First sequence to resume from (the hash always folds the whole log).
        #[arg(long, default_value_t = 0)]
        from: i64,
        /// Replay only up to this event sequence.
        #[arg(long, value_name = "SEQ")]
        until: Option<i64>,
        /// Replay as of an RFC3339 instant or an epoch-second count.
        #[arg(long, value_name = "TS")]
        as_of: Option<String>,
    },
    /// RDF graph operations.
    Graph {
        #[command(subcommand)]
        command: GraphCommand,
    },
    /// Inspect and verify the log and audit streams.
    Logs {
        #[command(subcommand)]
        command: LogsCommand,
    },
    /// Code metadata: scan, verify, query, and render the `/code` graph.
    Codex {
        #[command(subcommand)]
        command: CodexCommand,
    },
    /// The LLM substrate: ledger, replay, cache, schemas, routing, grammars.
    Llm {
        #[command(subcommand)]
        command: LlmCommand,
    },
    /// The persistent being: identity, blocks, beliefs, goals, relationships.
    Being {
        #[command(subcommand)]
        command: being::BeingCommand,
    },
}

#[derive(Subcommand, Debug)]
enum LlmCommand {
    /// Aggregate the call ledger, and check every row is complete.
    Stats {
        /// Only calls created at or after this RFC3339 instant.
        #[arg(long, value_name = "TS")]
        since: Option<String>,
        /// Only calls made for this purpose.
        #[arg(long, value_name = "PURPOSE")]
        purpose: Option<String>,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
        /// Exit non-zero unless every ledger row is complete.
        #[arg(long)]
        assert_complete: bool,
    },
    /// Replay a recorded session with the network guard armed.
    Replay {
        /// Session directory or `session.json`.
        #[arg(long, value_name = "PATH")]
        session: PathBuf,
        /// Exit non-zero unless zero provider calls were made.
        #[arg(long)]
        assert_zero_calls: bool,
    },
    /// Re-hash every cached body and compare it with its index row.
    VerifyCache {
        /// Also fail on a body that no index row covers.
        #[arg(long)]
        strict: bool,
    },
    /// Prove every malformed fixture is rejected, for the right reason.
    SchemaReject {
        /// Directory of `{name, schema, payload, expect}` fixtures.
        #[arg(long, value_name = "DIR")]
        fixtures: PathBuf,
        /// Exit non-zero unless every fixture was rejected.
        #[arg(long)]
        assert_all_rejected: bool,
    },
    /// Report which model the router would choose for a labelled request.
    Routes {
        /// One of: interpret, plan, extract, critique, summarize, code_review,
        /// classify, diagnose.
        #[arg(long)]
        purpose: String,
        /// One of: low, medium, high, very_high.
        #[arg(long, default_value = "medium")]
        complexity: String,
        /// One of: low, medium, high.
        #[arg(long, default_value = "medium")]
        stakes: String,
        /// One of: low, normal, high.
        #[arg(long, default_value = "normal")]
        precision: String,
        /// Latency ceiling in milliseconds; 0 means none.
        #[arg(long, default_value_t = 0)]
        latency_budget: u32,
        /// Cost ceiling in micros; 0 means none.
        #[arg(long, default_value_t = 0)]
        cost_budget: i64,
        /// Pin the model instead of routing.
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Compile every fixture schema with every decoder backend and score them.
    Grammars {
        /// Directory of `{name, schema, valid, mutated}` fixtures.
        #[arg(long, value_name = "DIR")]
        fixtures: PathBuf,
        /// Print a score line per backend.
        #[arg(long)]
        score: bool,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum CodexCommand {
    /// Scan the workspace, rebuild `/code`, and regenerate registry.json.
    Scan {
        /// Tree to scan; defaults to the repository root.
        #[arg(long, value_name = "DIR")]
        root: Option<PathBuf>,
        /// Do not write per-module `metadata.ttl` files.
        #[arg(long)]
        no_emit: bool,
        /// Also regenerate `codex.lock`.
        #[arg(long)]
        update_lock: bool,
        /// Print a single JSON object instead of a report.
        #[arg(long)]
        json: bool,
    },
    /// Check every code-metadata rule; exits non-zero on any violation.
    Verify {
        /// Tree to verify; defaults to the repository root.
        #[arg(long, value_name = "DIR")]
        root: Option<PathBuf>,
        /// Print the failures as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Render the module dependency graph.
    Graph {
        /// `mermaid` or `html`.
        #[arg(long, default_value = "mermaid")]
        format: String,
        /// Write to this file instead of stdout.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Render only this module and its direct neighbours.
        #[arg(long, value_name = "URI")]
        focus: Option<String>,
    },
    /// Report interface drift, or diff against a lock file.
    Diff {
        /// Compare a lock file against a fresh scan instead of reporting drift.
        #[arg(long, value_name = "FILE")]
        lock: Option<PathBuf>,
    },
    /// Answer a question about the code graph.
    Meta {
        /// The query: TotalModules, ModulesPerPhase, SymbolsPerModule,
        /// CapabilitiesWithoutTests, DependencyCycles, VersionDrift, Churn,
        /// OrphanFiles, CopiedFrom, UnreferencedSymbols.
        #[arg(long, value_name = "NAME")]
        query: String,
        /// Tree to scan; defaults to the repository root.
        #[arg(long, value_name = "DIR")]
        root: Option<PathBuf>,
        /// Print the answer as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Write `codex.lock`, or check it is current.
    Lock {
        /// Tree to scan; defaults to the repository root.
        #[arg(long, value_name = "DIR")]
        root: Option<PathBuf>,
        /// Exit non-zero when the committed lock is stale.
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand, Debug)]
enum GraphCommand {
    /// SHACL-validate a named graph; exits non-zero on any violation.
    Validate {
        /// One of: being, memory, epistemic, library, world, provenance, code.
        #[arg(long)]
        graph: String,
    },
}

#[derive(Subcommand, Debug)]
enum LogsCommand {
    /// Schema, audit completeness, chain integrity, redaction, replay.
    Verify,
    /// Print the last N records.
    Tail {
        #[arg(short = 'n', long, default_value_t = 20, value_name = "K")]
        lines: usize,
    },
    /// Print every record correlated with a trace id.
    Trace {
        #[arg(value_name = "ULID")]
        id: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let cfg = match kernel::Kernel::resolve_config(cli.config, cli.data_dir) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("mm-cli: {} ({})", e, e.code());
            return ExitCode::from(2);
        }
    };

    let outcome = match cli.command {
        Command::Doctor => doctor::run(cfg).await,
        Command::Replay { from, until, as_of } => {
            replay_cmd::run(cfg, from, until, as_of.as_deref()).await
        }
        Command::Graph {
            command: GraphCommand::Validate { graph },
        } => graph_cmd::validate(cfg, &graph).await,
        Command::Logs {
            command: LogsCommand::Verify,
        } => logs_cmd::verify(cfg).await,
        Command::Logs {
            command: LogsCommand::Tail { lines },
        } => logs_cmd::tail(cfg, lines).await,
        Command::Logs {
            command: LogsCommand::Trace { id },
        } => logs_cmd::trace(cfg, &id).await,
        Command::Codex {
            command:
                CodexCommand::Scan {
                    root,
                    no_emit,
                    update_lock,
                    json,
                },
        } => codex_cmd::scan(cfg, root, no_emit, update_lock, json).await,
        Command::Codex {
            command: CodexCommand::Verify { root, json },
        } => codex_cmd::verify(cfg, root, json).await,
        Command::Codex {
            command: CodexCommand::Graph { format, out, focus },
        } => codex_cmd::graph(cfg, None, format, out, focus).await,
        Command::Codex {
            command: CodexCommand::Diff { lock },
        } => codex_cmd::diff(cfg, lock).await,
        Command::Codex {
            command: CodexCommand::Meta { query, root, json },
        } => codex_cmd::meta(cfg, root, query, json).await,
        Command::Codex {
            command: CodexCommand::Lock { root, check },
        } => codex_cmd::lock(cfg, root, check).await,
        Command::Llm {
            command:
                LlmCommand::Stats {
                    since,
                    purpose,
                    json,
                    assert_complete,
                },
        } => llm_cmd::stats(cfg, since, purpose, json, assert_complete).await,
        Command::Llm {
            command:
                LlmCommand::Replay {
                    session,
                    assert_zero_calls,
                },
        } => llm_cmd::replay(cfg, session, assert_zero_calls).await,
        Command::Llm {
            command: LlmCommand::VerifyCache { strict },
        } => llm_cmd::verify_cache(cfg, strict).await,
        Command::Llm {
            command:
                LlmCommand::SchemaReject {
                    fixtures,
                    assert_all_rejected,
                },
        } => llm_cmd::schema_reject(cfg, fixtures, assert_all_rejected).await,
        Command::Llm {
            command:
                LlmCommand::Routes {
                    purpose,
                    complexity,
                    stakes,
                    precision,
                    latency_budget,
                    cost_budget,
                    model,
                    json,
                },
        } => {
            llm_cmd::routes(
                cfg,
                llm_cmd::RouteArgs {
                    purpose,
                    complexity,
                    stakes,
                    precision,
                    latency_budget_ms: latency_budget,
                    cost_budget_micros: cost_budget,
                    model,
                },
                json,
            )
            .await
        }
        Command::Llm {
            command:
                LlmCommand::Grammars {
                    fixtures,
                    score,
                    json,
                },
        } => llm_cmd::grammars(cfg, fixtures, score, json).await,
        Command::Being { command } => being::run(cfg, command).await,
    };

    match outcome {
        Ok(code) => code,
        Err(e) => {
            eprintln!("mm-cli: {} ({})", e, e.code());
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// The CLI is a capability like any other, so it must be tested. This asserts
    /// the clap definition itself is well formed — a duplicate flag or a missing
    /// value name is a defect that only shows up when someone types the command.
    #[test]
    fn the_command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn codex_subcommands_parse() {
        let cli = Cli::try_parse_from(["mm-cli", "codex", "scan", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Codex {
                command: CodexCommand::Scan { json: true, .. }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "codex",
            "verify",
            "--root",
            "bench/codex/fixtures/good",
        ])
        .unwrap();
        match cli.command {
            Command::Codex {
                command: CodexCommand::Verify { root, .. },
            } => assert_eq!(root.unwrap().to_string_lossy(), "bench/codex/fixtures/good"),
            other => panic!("unexpected command: {other:?}"),
        }

        let cli =
            Cli::try_parse_from(["mm-cli", "codex", "meta", "--query", "TotalModules"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Codex {
                command: CodexCommand::Meta { .. }
            }
        ));
    }

    #[test]
    fn an_unknown_codex_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["mm-cli", "codex", "frobnicate"]).is_err());
    }

    #[test]
    fn llm_subcommands_parse() {
        let cli =
            Cli::try_parse_from(["mm-cli", "llm", "stats", "--assert-complete", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Llm {
                command: LlmCommand::Stats {
                    assert_complete: true,
                    json: true,
                    ..
                }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "llm",
            "replay",
            "--session",
            "bench/llm/sessions/recorded-session-01",
            "--assert-zero-calls",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Llm {
                command: LlmCommand::Replay {
                    assert_zero_calls: true,
                    ..
                }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "llm",
            "routes",
            "--purpose",
            "plan",
            "--complexity",
            "high",
            "--stakes",
            "high",
            "--cost-budget",
            "50000",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Llm {
                command: LlmCommand::Routes { .. }
            }
        ));

        assert!(Cli::try_parse_from(["mm-cli", "llm", "frobnicate"]).is_err());
    }

    #[test]
    fn being_subcommands_parse() {
        let cli = Cli::try_parse_from(["mm-cli", "being", "verify"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Being {
                command: being::BeingCommand::Verify
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "being",
            "goal",
            "transition",
            "--id",
            "01hf7yat000000000000000001",
            "--to",
            "fulfilled",
        ])
        .unwrap();
        match cli.command {
            Command::Being {
                command: being::BeingCommand::Goal { .. },
            } => {}
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "mm-cli",
            "being",
            "belief",
            "promote",
            "--user",
            "01hf7yat000000000000000001",
            "--proposition",
            "prefers Rust",
            "--to",
            "OBSERVED",
            "--evidence",
            "ev-1",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Being {
                command: being::BeingCommand::Belief { .. }
            }
        ));

        assert!(Cli::try_parse_from(["mm-cli", "being", "frobnicate"]).is_err());
    }
}
