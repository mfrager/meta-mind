//! `mm-cli` — the Metamind kernel operator surface.
//!
//! Every command either proves a kernel invariant or reports what state the
//! kernel is in. Nothing here reaches into a store directly: commands go through
//! `Kernel`, so no command can open a database the kernel would not open.
#![forbid(unsafe_code)]

mod being;
mod case_cmd;
mod codex_cmd;
mod decision;
mod decision_cmd;
mod doctor;
mod episode;
mod epistemic;
mod experience_cmd;
mod frame_cmd;
mod graph_cmd;
mod kernel;
mod library;
mod llm_cmd;
mod logs_cmd;
mod memory;
mod policy_cmd;
mod replay_cmd;
mod runtime_cmd;
mod selfeng_cmd;
mod skill_cmd;
mod tools_cmd;

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
    /// The long-term memory organ: add, recall, consolidate, forget, eval, verify.
    Memory {
        #[command(subcommand)]
        command: memory::MemoryCommand,
    },
    /// The epistemic discipline: claims, promotion, contradictions, justification.
    Epistemic {
        #[command(subcommand)]
        command: epistemic::EpistemicCommand,
    },
    /// The cognitive library: entries, validation, applicability, extraction.
    Library {
        #[command(subcommand)]
        command: library::LibraryCommand,
    },
    /// The verified skill library: register, verify, retrieve.
    Skill {
        #[command(subcommand)]
        command: skill_cmd::SkillCommand,
    },
    /// Structure-mapped case retrieval.
    Case {
        #[command(subcommand)]
        command: case_cmd::CaseCommand,
    },
    /// The experience compiler: trajectories and insights into draft candidates.
    Experience {
        #[command(subcommand)]
        command: experience_cmd::ExperienceCommand,
    },
    /// The policy genome: immutable versions, fitness, evolution.
    Policy {
        #[command(subcommand)]
        command: policy_cmd::PolicyCommand,
    },
    /// Conceptual frames: composition and missing slots.
    Frame {
        #[command(subcommand)]
        command: frame_cmd::FrameCommand,
    },
    /// The metacognitive controller: run, replay, verify, value, audit, program.
    Episode {
        #[command(subcommand)]
        command: episode::EpisodeCommand,
    },
    /// Answer one bounded question and record the answer.
    Decide(decision_cmd::DecideArgs),
    /// The sanity firewall: evaluate one episode, or a corpus of inputs.
    Firewall {
        #[command(subcommand)]
        command: decision_cmd::FirewallCommand,
    },
    /// Comparison integrity: check a pair of contracts, or a fixtures corpus.
    Compare {
        #[command(subcommand)]
        command: decision_cmd::CompareCommand,
    },
    /// Risk measures over an outcome distribution, graded against references.
    Risk {
        #[command(subcommand)]
        command: decision_cmd::RiskCommand,
    },
    /// Calibration and conformal abstention thresholds.
    Calibration {
        #[command(subcommand)]
        command: decision_cmd::CalibrationCommand,
    },
    /// Run the one conformance suite against the named cores.
    Conformance(decision_cmd::ConformanceArgs),
    /// Tool execution: what is registered, what it declares, and what happens when it runs.
    Tool {
        #[command(subcommand)]
        command: tools_cmd::ToolCommand,
    },
    /// The action ledger: list it, verify its chain, and roll an action back.
    Action {
        #[command(subcommand)]
        command: tools_cmd::ActionCommand,
    },
    /// Verification obligations: a build, a test run, static analysis, a refutation, residuals.
    Verify {
        #[command(subcommand)]
        command: tools_cmd::VerifyCommand,
    },
    /// The MCP bridge: list tools, call one, or serve over stdio.
    Mcp {
        #[command(subcommand)]
        command: tools_cmd::McpCommand,
    },
    /// Score a staked probability against what happened, and record the run.
    Calibrate {
        /// The labeled corpus: one `{class, predicted, label}` object per line.
        #[arg(long, value_name = "PATH")]
        bench: PathBuf,
        /// Exit non-zero unless the calibrated Brier is at most this.
        #[arg(long, value_name = "F")]
        assert_brier_le: Option<f64>,
        /// Exit non-zero unless the calibrated ECE is at most this.
        #[arg(long, value_name = "F")]
        assert_ece_le: Option<f64>,
        /// Exit non-zero unless the calibrated Brier beats the baseline.
        #[arg(long)]
        assert_improves_baseline: bool,
        /// Stake every labeled point in the prediction ledger and resolve it.
        #[arg(long)]
        record_ledger: bool,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Self-observation: the trigger queue, the diagnosis, and the lessons.
    Meta {
        #[command(subcommand)]
        command: selfeng_cmd::MetaCommand,
    },
    /// The regression suite the mistake compiler writes.
    Regression {
        #[command(subcommand)]
        command: selfeng_cmd::RegressionCommand,
    },
    /// Typed change sets: assemble one from a gap, and read it back.
    Changeset {
        #[command(subcommand)]
        command: selfeng_cmd::ChangeSetCommand,
    },
    /// The sandbox a candidate is built and tested in.
    Sandbox {
        #[command(subcommand)]
        command: selfeng_cmd::SandboxCommand,
    },
    /// Judge a change set: evaluate the gate, record the decision, version the lineage.
    Promote {
        /// The change set's ULID.
        #[arg(value_name = "ULID")]
        id: String,
        /// Exit non-zero unless the decision carries a written reason.
        #[arg(long)]
        assert_reason_present: bool,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Reject a change set by hand, with a written reason.
    Reject {
        /// The change set's ULID.
        #[arg(value_name = "ULID")]
        id: String,
        /// Why it is refused. Required: a rejection without one is not a decision.
        #[arg(long, value_name = "TEXT")]
        reason: String,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// The four self-engineering budgets.
    Budget {
        #[command(subcommand)]
        command: selfeng_cmd::BudgetCommand,
    },
    /// The Pi code agent: run a task, or ingest a recorded session.
    Pi {
        #[command(subcommand)]
        command: selfeng_cmd::PiCli,
    },
    /// Prove the production tree was not written outside a promotion.
    Audit {
        #[command(subcommand)]
        command: selfeng_cmd::AuditCommand,
    },
    /// The closed developmental loop: run it, read what it produced, reproduce it.
    Loop {
        #[command(subcommand)]
        command: runtime_cmd::LoopCommand,
    },
    /// The numeric self-model: Actual vs Model vs Ideal, per run.
    SelfModel {
        #[command(subcommand)]
        command: runtime_cmd::SelfModelCommand,
    },
    /// Architectural-debt scanning over the code graph, the memory and the policies.
    Debt {
        #[command(subcommand)]
        command: runtime_cmd::DebtCommand,
    },
    /// Reversible cognitive garbage collection, cheapest intervention first.
    Gc {
        /// Apply the proposed actions instead of only proposing them.
        #[arg(long)]
        apply: bool,
        /// Print a single JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Hot-load a promoted module version, without restarting the process.
    Module {
        #[command(subcommand)]
        command: runtime_cmd::ModuleCommand,
    },
    /// Design revisions, which reach a document only through a promoted change set.
    Design {
        #[command(subcommand)]
        command: runtime_cmd::DesignCommand,
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
        Command::Memory { command } => memory::run(cfg, command).await,
        Command::Epistemic { command } => epistemic::run(cfg, command).await,
        Command::Library { command } => library::run(cfg, command).await,
        Command::Skill { command } => skill_cmd::run(cfg, command).await,
        Command::Case { command } => case_cmd::run(cfg, command).await,
        Command::Experience { command } => experience_cmd::run(cfg, command).await,
        Command::Policy { command } => policy_cmd::run(cfg, command).await,
        Command::Frame { command } => frame_cmd::run(cfg, command).await,
        Command::Episode { command } => episode::run(cfg, command).await,
        Command::Decide(args) => decision_cmd::decide(cfg, args).await,
        Command::Firewall { command } => decision_cmd::run_firewall(cfg, command).await,
        Command::Compare { command } => decision_cmd::run_compare(cfg, command).await,
        Command::Risk { command } => decision_cmd::run_risk(cfg, command).await,
        Command::Calibration { command } => decision_cmd::run_calibration(cfg, command).await,
        Command::Conformance(args) => decision_cmd::conformance(cfg, args).await,
        Command::Tool { command } => tools_cmd::run_tool(cfg, command).await,
        Command::Action { command } => tools_cmd::run_action(cfg, command).await,
        Command::Verify { command } => tools_cmd::run_verify(cfg, command).await,
        Command::Mcp { command } => tools_cmd::run_mcp(cfg, command).await,
        Command::Calibrate {
            bench,
            assert_brier_le,
            assert_ece_le,
            assert_improves_baseline,
            record_ledger,
            json,
        } => {
            selfeng_cmd::calibrate(
                cfg,
                bench,
                assert_brier_le,
                assert_ece_le,
                assert_improves_baseline,
                record_ledger,
                json,
            )
            .await
        }
        Command::Meta { command } => selfeng_cmd::meta(cfg, command).await,
        Command::Regression { command } => selfeng_cmd::regression(cfg, command).await,
        Command::Changeset { command } => selfeng_cmd::changeset(cfg, command).await,
        Command::Sandbox { command } => selfeng_cmd::sandbox(cfg, command).await,
        Command::Promote {
            id,
            assert_reason_present,
            json,
        } => selfeng_cmd::promote(cfg, id, assert_reason_present, json).await,
        Command::Reject { id, reason, json } => selfeng_cmd::reject(cfg, id, reason, json).await,
        Command::Budget { command } => selfeng_cmd::budget(cfg, command).await,
        Command::Pi { command } => selfeng_cmd::pi(cfg, command).await,
        Command::Audit { command } => selfeng_cmd::audit(cfg, command).await,
        Command::Loop { command } => runtime_cmd::run_loop(cfg, command).await,
        Command::SelfModel { command } => runtime_cmd::self_model(cfg, command).await,
        Command::Debt { command } => runtime_cmd::debt(cfg, command).await,
        Command::Gc { apply, json } => runtime_cmd::gc(cfg, apply, json).await,
        Command::Module { command } => runtime_cmd::module(cfg, command).await,
        Command::Design { command } => runtime_cmd::design(cfg, command).await,
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

    #[test]
    fn memory_subcommands_parse() {
        let cli = Cli::try_parse_from(["mm-cli", "memory", "verify"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Memory {
                command: memory::MemoryCommand::Verify { .. }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "memory",
            "recall",
            "what fixed the build",
            "--k",
            "5",
            "--kinds",
            "semantic,episodic",
            "--tier",
            "recall",
        ])
        .unwrap();
        match cli.command {
            Command::Memory {
                command: memory::MemoryCommand::Recall { k, kinds, .. },
            } => {
                assert_eq!(k, 5);
                assert_eq!(kinds.as_deref(), Some("semantic,episodic"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "mm-cli",
            "memory",
            "forget",
            "--now",
            "2026-10-08T00:00:00Z",
            "--dry-run",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Memory {
                command: memory::MemoryCommand::Forget { dry_run: true, .. }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "memory",
            "eval",
            "--gold",
            "bench/memory/gold.jsonl",
            "--k",
            "5",
            "--thresholds",
            "bench/memory/thresholds.toml",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Memory {
                command: memory::MemoryCommand::Eval { k: 5, .. }
            }
        ));

        assert!(Cli::try_parse_from(["mm-cli", "memory", "frobnicate"]).is_err());
    }

    #[test]
    fn library_subcommands_parse() {
        let cli = Cli::try_parse_from([
            "mm-cli",
            "library",
            "import",
            "ontology/seed/library_seed.ttl",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Library {
                command: library::LibraryCommand::Import { .. }
            }
        ));

        let cli =
            Cli::try_parse_from(["mm-cli", "library", "validate", "--graph", "library"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Library {
                command: library::LibraryCommand::Validate { .. }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "library",
            "applicable",
            "--state",
            "bench/library/state_uncertain_strategy.json",
            "--top",
            "5",
            "--gold",
            "bench/library/gold_applicability.jsonl",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Library {
                command: library::LibraryCommand::Applicable { top: 5, .. }
            }
        ));

        assert!(Cli::try_parse_from(["mm-cli", "library", "frobnicate"]).is_err());
    }

    #[test]
    fn library_organ_subcommands_parse() {
        let cli = Cli::try_parse_from([
            "mm-cli",
            "skill",
            "retrieve",
            "--query",
            "verify external api before use",
            "--top",
            "3",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Skill {
                command: skill_cmd::SkillCommand::Retrieve { top: 3, .. }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "policy",
            "evolve",
            "prefer_simpler_solution",
            "--generations",
            "3",
            "--budget",
            "bench/library/evolution/budget.toml",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Policy {
                command: policy_cmd::PolicyCommand::Evolve { generations: 3, .. }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "frame",
            "compose",
            "--frames",
            "problem_solving,software,debugging,high_stakes",
            "--episode",
            "01J0000000000000000000000A",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Frame {
                command: frame_cmd::FrameCommand::Compose { .. }
            }
        ));

        assert!(Cli::try_parse_from(["mm-cli", "policy", "frobnicate"]).is_err());
    }

    #[test]
    fn tool_subcommands_parse() {
        let cli = Cli::try_parse_from(["mm-cli", "tool", "list", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Tool {
                command: tools_cmd::ToolCommand::List { json: true }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "tool",
            "run",
            "fs.read",
            "--arg",
            "path=data/sandbox/README.md",
            "--principal",
            "01hf7yat000000000000000001",
            "--idempotency-key",
            "k1",
            "--json",
        ])
        .unwrap();
        match cli.command {
            Command::Tool {
                command:
                    tools_cmd::ToolCommand::Run {
                        name,
                        args,
                        idempotency_key,
                        json,
                        ..
                    },
            } => {
                assert_eq!(name, "fs.read");
                assert_eq!(args, vec!["path=data/sandbox/README.md".to_string()]);
                assert_eq!(idempotency_key.as_deref(), Some("k1"));
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert!(Cli::try_parse_from(["mm-cli", "tool", "frobnicate"]).is_err());
    }

    #[test]
    fn action_verify_and_mcp_subcommands_parse() {
        let cli = Cli::try_parse_from(["mm-cli", "action", "ledger", "--verify"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Action {
                command: tools_cmd::ActionCommand::Ledger { verify: true, .. }
            }
        ));

        let cli =
            Cli::try_parse_from(["mm-cli", "action", "rollback", "01hf7yat000000000000000001"])
                .unwrap();
        assert!(matches!(
            cli.command,
            Command::Action {
                command: tools_cmd::ActionCommand::Rollback { .. }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "verify",
            "run",
            "symbolic",
            "--subject",
            "bench/tools/seeded_inconsistency.logic",
        ])
        .unwrap();
        match cli.command {
            Command::Verify {
                command: tools_cmd::VerifyCommand::Run { kind, subject, .. },
            } => {
                assert_eq!(kind, "symbolic");
                assert_eq!(subject, "bench/tools/seeded_inconsistency.logic");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = Cli::try_parse_from(["mm-cli", "mcp", "list", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Mcp {
                command: tools_cmd::McpCommand::List { json: true }
            }
        ));

        assert!(Cli::try_parse_from(["mm-cli", "verify", "frobnicate"]).is_err());
        assert!(Cli::try_parse_from(["mm-cli", "mcp", "frobnicate"]).is_err());
    }

    #[test]
    fn self_engineering_subcommands_parse() {
        let cli = Cli::try_parse_from([
            "mm-cli",
            "calibrate",
            "--bench",
            "bench/calibration/predictions.jsonl",
            "--assert-brier-le",
            "0.20",
            "--assert-ece-le",
            "0.10",
            "--assert-improves-baseline",
        ])
        .unwrap();
        match cli.command {
            Command::Calibrate {
                bench,
                assert_brier_le,
                assert_ece_le,
                assert_improves_baseline,
                ..
            } => {
                assert!(bench.ends_with("bench/calibration/predictions.jsonl"));
                assert_eq!(assert_brier_le, Some(0.20));
                assert_eq!(assert_ece_le, Some(0.10));
                assert!(assert_improves_baseline);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "mm-cli",
            "meta",
            "analyze",
            "--episode",
            "bench/episodes/seeded_failure_01.json",
            "--assert-lessons-ge",
            "1",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Meta {
                command: selfeng_cmd::MetaCommand::Analyze {
                    assert_lessons_ge: Some(1),
                    ..
                }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "regression",
            "run",
            "--suite",
            "bench/regression",
            "--assert-fail-before-pass-after",
            "seeded_bug_01",
        ])
        .unwrap();
        match cli.command {
            Command::Regression {
                command:
                    selfeng_cmd::RegressionCommand::Run {
                        assert_fail_before_pass_after,
                        ..
                    },
            } => assert_eq!(
                assert_fail_before_pass_after.as_deref(),
                Some("seeded_bug_01")
            ),
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "mm-cli",
            "changeset",
            "new",
            "--from-gap",
            "bench/gaps/gap_01.json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Changeset {
                command: selfeng_cmd::ChangeSetCommand::New { .. }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "sandbox",
            "run",
            "01hf7yat000000000000000001",
            "--apply",
            "--assert-isolated",
        ])
        .unwrap();
        match cli.command {
            Command::Sandbox {
                command:
                    selfeng_cmd::SandboxCommand::Run {
                        apply,
                        assert_isolated,
                        ..
                    },
            } => {
                assert!(apply && assert_isolated);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "mm-cli",
            "promote",
            "01hf7yat000000000000000001",
            "--assert-reason-present",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Promote {
                assert_reason_present: true,
                ..
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "reject",
            "01hf7yat000000000000000001",
            "--reason",
            "the bench does not cover it",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Reject { .. }));

        let cli =
            Cli::try_parse_from(["mm-cli", "budget", "show", "--assert-no-overspend"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Budget {
                command: selfeng_cmd::BudgetCommand::Show {
                    assert_no_overspend: true,
                    ..
                }
            }
        ));

        let cli = Cli::try_parse_from([
            "mm-cli",
            "pi",
            "run",
            "--task",
            "bench/pi/module_scaffold_01.json",
            "--offline",
            "--assert-session-ingested",
        ])
        .unwrap();
        match cli.command {
            Command::Pi {
                command:
                    selfeng_cmd::PiCli::Run {
                        offline,
                        assert_session_ingested,
                        ..
                    },
            } => {
                assert!(offline && assert_session_ingested);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "mm-cli",
            "audit",
            "production-tree",
            "--assert-unmodified-outside-promotion",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Audit {
                command: selfeng_cmd::AuditCommand::ProductionTree {
                    assert_unmodified_outside_promotion: true,
                    ..
                }
            }
        ));

        assert!(Cli::try_parse_from(["mm-cli", "meta", "frobnicate"]).is_err());
        assert!(Cli::try_parse_from(["mm-cli", "sandbox", "frobnicate"]).is_err());
    }

    #[test]
    fn policy_check_parses_under_the_policy_command() {
        let cli = Cli::try_parse_from([
            "mm-cli",
            "policy",
            "check",
            "--principal",
            "system",
            "--action",
            "fs.write",
            "--resource",
            "data/sandbox/x",
        ])
        .unwrap();
        match cli.command {
            Command::Policy {
                command:
                    policy_cmd::PolicyCommand::Check {
                        action, resource, ..
                    },
            } => {
                assert_eq!(action, "fs.write");
                assert_eq!(resource, "data/sandbox/x");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
