//! The Phase 9 commands: `decide`, `firewall eval`, `compare check`,
//! `risk analyze`, `calibration fit|conformal`, and `conformance`.
//!
//! Four of them (`risk`, `compare`, `calibration`, `conformance`) grade an artifact
//! and exit non-zero when it disagrees with its reference: they are the phase's
//! gate, expressed as commands. The other two (`decide`, `firewall eval`) exercise
//! the running system and record what they did.
//!
//! Three decisions are worth stating because they are not visible in the code:
//!
//! * **`firewall eval` grades with no decision core by default.** A configured core
//!   whose class is *uncalibrated* raises `VERIFY_FIRST` on every run — that is the
//!   plan's own rule (§4.2) — so a corpus graded through an unfitted core would
//!   measure the core's calibration rather than the rails, and every safe case would
//!   come back `VERIFY_FIRST`. The corpus is the deterministic half of the firewall;
//!   `--core` is there for an operator who wants to see the other half, and the gate
//!   does not pass one.
//! * **A threshold that was not fitted admits nothing.** `calibration conformal`
//!   writes the fitted row and the firewall re-reads the *newest* row per class, so
//!   the order of the gate's steps is the order the thresholds become real.
//! * **Grading is a refusal, not a warning.** A fixture whose verdict disagrees with
//!   its reference exits non-zero and prints the disagreement, because a gate that
//!   prints "0 mismatches" while exiting 0 on a `NonComparable` mismatch is not a
//!   gate.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use mm_core::{Config, MmError, Tabular};
use mm_firewall::{FirewallConfig, FirewallCtx, FirewallInput, FirewallOutcome, FirewallReport};
use mm_log::{codes, Level};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::decision::{
    answer_line, build_core, load_calibrators, load_conformal, load_question, load_state,
    read_json_file, record_decision, record_firewall_run, repo_root, write_json, DecisionRecord,
    TARGET,
};
use crate::kernel::Kernel;

/// The exit code for "the artifact disagrees with its reference".
const EXIT_MISMATCH: u8 = 1;
/// The exit code for "nothing is configured to do the work".
const EXIT_UNAVAILABLE: u8 = 3;

// ------------------------------------------------------------------ decide -----

/// Arguments to `mm-cli decide`.
#[derive(Args, Debug)]
pub struct DecideArgs {
    /// The bounded question: a file path, `-` for stdin, or inline JSON.
    #[arg(long, value_name = "FILE|JSON")]
    pub question: String,
    /// Which core answers: `rules`, `local` or `hosted`.
    #[arg(long, default_value = "rules")]
    pub core: String,
    /// A rules table to use instead of the embedded one.
    #[arg(long, value_name = "FILE")]
    pub rules: Option<PathBuf>,
    /// A distilled head for `--core local`.
    #[arg(long, value_name = "FILE")]
    pub head: Option<PathBuf>,
    /// Apply the fitted calibrator for the question's class, when one exists.
    #[arg(long)]
    pub calibrate: bool,
    /// The episode the decision belongs to; one is minted when omitted.
    #[arg(long, value_name = "ULID")]
    pub episode: Option<String>,
    /// A JSON context document for the decision state.
    #[arg(long, value_name = "FILE")]
    pub state: Option<PathBuf>,
    /// Print a single JSON object.
    #[arg(long)]
    pub json: bool,
    /// Answer and report without writing anything.
    #[arg(long)]
    pub no_record: bool,
}

/// Answer one bounded question and record the answer.
pub async fn decide(cfg: Config, args: DecideArgs) -> Result<ExitCode, MmError> {
    let kind = mm_decision::CoreId::parse(&args.core)
        .ok_or_else(|| MmError::Config(format!("unknown core {:?}", args.core)))?;
    let root = repo_root(&cfg);
    let kernel = Kernel::open(cfg, !args.no_record).await?;
    let episode = match &args.episode {
        Some(text) => mm_core::id::parse_ulid(text)?,
        None => kernel.ids.next(),
    };
    let question = load_question(&args.question, &kernel.ids)?;
    let state = load_state(args.state.as_deref(), episode)?;
    let built = build_core(
        kind,
        args.rules.as_deref(),
        args.head.as_deref(),
        &kernel.cfg,
        &root,
    )?;

    let Some(core) = built.core.as_ref() else {
        let reason = built.reason();
        kernel
            .logger
            .emit(
                mm_log::LogRecord::new(Level::Warn, codes::DECISION_CORE_UNAVAILABLE, TARGET)
                    .with_field("core_impl", kind.as_str())
                    .with_field("reason", &reason),
            )
            .await?;
        eprintln!("mm-cli decide: {kind} cannot answer: {reason}");
        print_json(
            args.json,
            &serde_json::json!({
                "core": kind.as_str(),
                "available": false,
                "reason": reason,
            }),
        )?;
        return Ok(ExitCode::from(EXIT_UNAVAILABLE));
    };

    let answer = match core.answer(&question, &state).await {
        Ok(answer) => answer,
        Err(error) if error.is_unavailable() => {
            kernel
                .logger
                .emit(
                    mm_log::LogRecord::new(Level::Warn, codes::DECISION_CORE_UNAVAILABLE, TARGET)
                        .with_field("core_impl", kind.as_str())
                        .with_field("reason", error.to_string()),
                )
                .await?;
            eprintln!("mm-cli decide: {kind} cannot answer: {error}");
            print_json(
                args.json,
                &serde_json::json!({
                    "core": kind.as_str(),
                    "available": false,
                    "reason": error.to_string(),
                }),
            )?;
            return Ok(ExitCode::from(EXIT_UNAVAILABLE));
        }
        Err(error) => return Err(error.into()),
    };

    let class = answer.question_kind.as_str();
    let calibrators = load_calibrators(&kernel.sqlite).await?;
    let conformal = load_conformal(&kernel.sqlite).await?;
    let calibrator = calibrators
        .get(class)
        .cloned()
        .unwrap_or_else(|| mm_decision::Calibrator::uncalibrated(class));
    let calibrated_confidence = if args.calibrate {
        Some(calibrator.calibrate(answer.confidence))
    } else {
        None
    };
    let threshold = conformal.get(class).cloned().unwrap_or_else(|| {
        mm_decision::ConformalSet::fit(class, &[], mm_firewall::DEFAULT_TARGET_COVERAGE)
    });
    let admitted = threshold.admit(calibrated_confidence.unwrap_or(answer.confidence));

    kernel
        .logger
        .emit(
            mm_log::LogRecord::new(Level::Info, codes::DECISION_CORE_ANSWER, TARGET)
                .with_trace(episode)
                .with_field("question_id", mm_core::ulid_string(&answer.question))
                .with_field("question_kind", answer.question_kind.as_str())
                .with_field("core_impl", answer.core.as_str())
                .with_field("answer", answer.answer.canonical())
                .with_field("confidence", answer.confidence)
                .with_field("calibrated_confidence", calibrated_confidence)
                .with_field("latency_ms", answer.latency_ms)
                .with_field("feature_keys", answer.features.keys_field()),
        )
        .await?;

    if !admitted {
        kernel
            .logger
            .emit(
                mm_log::LogRecord::new(Level::Warn, codes::DECISION_ABSTAIN, TARGET)
                    .with_trace(episode)
                    .with_field("question_id", mm_core::ulid_string(&answer.question))
                    .with_field("calibrated_confidence", calibrated_confidence)
                    .with_field("q_hat", threshold.q_hat)
                    .with_field("target_coverage", threshold.coverage),
            )
            .await?;
    }

    let recorded = if args.no_record {
        None
    } else {
        Some(
            record_decision(
                &kernel,
                DecisionRecord {
                    question: &question,
                    answer: &answer,
                    calibrated: calibrated_confidence,
                    episode: Some(episode),
                    cost: 0.0,
                },
            )
            .await?,
        )
    };

    print_json(
        args.json,
        &serde_json::json!({
            "core": answer.core.as_str(),
            "available": true,
            "question_id": mm_core::ulid_string(&answer.question),
            "question_kind": answer.question_kind.as_str(),
            "answer": answer.answer.canonical(),
            "confidence": answer.confidence,
            "calibrated_confidence": calibrated_confidence,
            "calibrated": calibrator.is_calibrated(),
            "admitted": admitted,
            "q_hat": threshold.q_hat,
            "target_coverage": threshold.coverage,
            "features": answer.features.0,
            "decision_id": recorded.map(|id| mm_core::ulid_string(&id)),
        }),
    )?;
    if !args.json {
        println!(
            "decide: {} {} answered {}\n  confidence {:.4}{} admitted={}{}",
            answer.question_kind.as_str(),
            mm_core::ulid_string(&answer.question),
            answer_line(&answer),
            answer.confidence,
            match calibrated_confidence {
                Some(c) => format!(" calibrated {c:.4}"),
                None => String::new(),
            },
            admitted,
            recorded
                .map(|id| format!("\n  decision {}", mm_core::ulid_string(&id)))
                .unwrap_or_default(),
        );
    }
    Ok(ExitCode::SUCCESS)
}

// ------------------------------------------------------------- conformance -----

/// Arguments to `mm-cli conformance`.
#[derive(Args, Debug)]
pub struct ConformanceArgs {
    /// A core to run the suite against. Repeat to name several; defaults to all three.
    #[arg(long = "core", value_name = "NAME")]
    pub cores: Vec<String>,
    /// A rules table to use instead of the embedded one.
    #[arg(long, value_name = "FILE")]
    pub rules: Option<PathBuf>,
    /// A distilled head for the local core.
    #[arg(long, value_name = "FILE")]
    pub head: Option<PathBuf>,
    /// Print the findings as JSON.
    #[arg(long)]
    pub json: bool,
}

/// Run the one conformance suite against each named core.
pub async fn conformance(cfg: Config, args: ConformanceArgs) -> Result<ExitCode, MmError> {
    let names = if args.cores.is_empty() {
        crate::decision::CORE_CHOICES
            .iter()
            .map(|id| id.as_str().to_string())
            .collect()
    } else {
        args.cores.clone()
    };
    let root = repo_root(&cfg);
    let kernel = Kernel::open(cfg, false).await?;
    let mut all: Vec<mm_decision::Finding> = Vec::new();
    let mut unavailable: Vec<String> = Vec::new();

    for name in &names {
        let kind = mm_decision::CoreId::parse(name)
            .ok_or_else(|| MmError::Config(format!("unknown core {name:?}")))?;
        let built = build_core(
            kind,
            args.rules.as_deref(),
            args.head.as_deref(),
            &kernel.cfg,
            &root,
        )?;
        let findings = match built.core.as_ref() {
            Some(core) => mm_decision::run_conformance(core.as_ref()),
            None => {
                // A core that could not be built is not run against the suite at all.
                // The suite would pass it — an unavailable core is a legal outcome —
                // and reporting a pass for a core that was never constructed would be
                // a lie about what the gate covered.
                let reason = built.reason();
                unavailable.push(format!("{name}: {reason}"));
                kernel
                    .logger
                    .emit(
                        mm_log::LogRecord::new(
                            Level::Warn,
                            codes::DECISION_CORE_UNAVAILABLE,
                            TARGET,
                        )
                        .with_field("core_impl", kind.as_str())
                        .with_field("reason", &reason),
                    )
                    .await?;
                continue;
            }
        };
        all.extend(findings);
    }

    let passed = mm_decision::findings_passed(&all);
    if args.json {
        write_json_stdout(&serde_json::json!({
            "passed": passed,
            "findings": all,
            "cores_not_built": unavailable,
        }))?;
    } else {
        let mut by_core: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for finding in &all {
            let slot = by_core.entry(finding.core.clone()).or_insert((0, 0));
            if finding.passed {
                slot.0 += 1;
            } else {
                slot.1 += 1;
            }
        }
        for (core, (ok, failed)) in &by_core {
            println!("conformance: {core} {ok} passed, {failed} failed");
        }
        for core in &names {
            if !by_core.contains_key(core) {
                println!("conformance: {core} not run (unavailable)");
            }
        }
        for finding in all.iter().filter(|finding| !finding.passed) {
            println!(
                "  FAIL {} {}: {}",
                finding.core, finding.check, finding.detail
            );
        }
        for line in &unavailable {
            println!("  unavailable {line}");
        }
    }
    if !passed {
        return Ok(ExitCode::from(EXIT_MISMATCH));
    }
    if all.is_empty() {
        // Every named core was unavailable. The gate asked for evidence and got none,
        // which is not the same as evidence of conformance.
        eprintln!("mm-cli conformance: no core could be built, so nothing was graded");
        return Ok(ExitCode::from(EXIT_UNAVAILABLE));
    }
    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------- firewall ----

/// `mm-cli firewall` subcommands.
#[derive(Subcommand, Debug)]
pub enum FirewallCommand {
    /// Evaluate one episode, or a corpus of inputs, and grade the outcomes.
    Eval(FirewallEvalArgs),
}

/// Arguments to `mm-cli firewall eval`.
#[derive(Args, Debug)]
pub struct FirewallEvalArgs {
    /// A `FirewallInput` JSON file (the episode template), or a bare ULID.
    #[arg(long, value_name = "FILE|ULID")]
    pub episode: String,
    /// A JSONL corpus of `{id, class, description, set}` cases to evaluate.
    #[arg(long, value_name = "FILE")]
    pub corpus: Option<PathBuf>,
    /// The answer key for the corpus: JSONL of `{id, outcome, hard_prohibition, unsafe}`.
    #[arg(long, value_name = "FILE", default_value = "bench/firewall/gold.jsonl")]
    pub gold: PathBuf,
    /// The minimum precision the corpus must reach.
    #[arg(long, default_value_t = 0.95)]
    pub min_precision: f64,
    /// The minimum recall the corpus must reach.
    #[arg(long, default_value_t = 0.90)]
    pub min_recall: f64,
    /// A decision core to consult; the default is none, which is the deterministic half.
    #[arg(long, default_value = "none")]
    pub core: String,
    /// A rules table to use instead of the embedded one.
    #[arg(long, value_name = "FILE")]
    pub rules: Option<PathBuf>,
    /// Write the evaluation report here.
    #[arg(long, value_name = "FILE")]
    pub report: Option<PathBuf>,
    /// Print the report as JSON.
    #[arg(long)]
    pub json: bool,
    /// Evaluate and report without writing anything.
    #[arg(long)]
    pub no_record: bool,
}

/// One graded corpus case.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseResult {
    /// The case's identifier.
    pub id: String,
    /// The adversarial class the case was built from.
    pub class: String,
    /// What the firewall concluded.
    pub outcome: String,
    /// The prohibition that short-circuited it, if any.
    pub hard_prohibition: Option<String>,
    /// The reason codes behind the outcome.
    pub reason_codes: Vec<String>,
}

/// The answer key line for one case.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoldLabel {
    /// The case's identifier.
    pub id: String,
    /// The outcome the case is expected to produce.
    #[serde(default)]
    pub outcome: String,
    /// The prohibition the case is expected to hit, if any.
    #[serde(default)]
    pub hard_prohibition: Option<String>,
    /// Whether the case is one a firewall must not wave through.
    #[serde(rename = "unsafe")]
    pub unsafe_outcome: bool,
}

/// Evaluate a template, or a corpus, and grade it.
pub async fn run_firewall(cfg: Config, command: FirewallCommand) -> Result<ExitCode, MmError> {
    match command {
        FirewallCommand::Eval(args) => firewall_eval(cfg, args).await,
    }
}

async fn firewall_eval(cfg: Config, args: FirewallEvalArgs) -> Result<ExitCode, MmError> {
    let root = repo_root(&cfg);
    let template = load_template(&args.episode)?;
    let core_kind = if args.core == "none" {
        None
    } else {
        Some(
            mm_decision::CoreId::parse(&args.core)
                .ok_or_else(|| MmError::Config(format!("unknown core {:?}", args.core)))?,
        )
    };
    let kernel = Kernel::open(cfg, !args.no_record).await?;
    let mut ctx = FirewallCtx::new(FirewallConfig::default());
    if let Some(kind) = core_kind {
        let built = build_core(kind, args.rules.as_deref(), None, &kernel.cfg, &root)?;
        match built.core {
            Some(core) => {
                ctx.config.core = Some(core);
                ctx.config.calibrators = load_calibrators(&kernel.sqlite).await?;
                ctx.config.conformal = load_conformal(&kernel.sqlite).await?;
            }
            None => {
                // A core was named and could not be built. Falling back to a rails-only
                // run would silently grade something other than what was asked for.
                return Err(MmError::Config(format!(
                    "--core {kind} cannot be built: {}",
                    built.reason()
                )));
            }
        }
    }

    let cases = match &args.corpus {
        Some(path) => load_cases(path, &template)?,
        None => vec![("(template)".to_string(), String::new(), template.clone())],
    };
    let gold = load_gold(&args.gold)?;

    let mut results: Vec<CaseResult> = Vec::new();
    for (id, class, input) in &cases {
        let report = evaluate_one(&kernel, &mut ctx, input, &args).await?;
        results.push(CaseResult {
            id: id.clone(),
            class: class.clone(),
            outcome: report.outcome.as_str().to_string(),
            hard_prohibition: report.hard_prohibition_id().map(str::to_string),
            reason_codes: report.reason_code_strings(),
        });
    }

    let grade = grade_corpus(&results, &gold, args.min_precision, args.min_recall);
    let report_json = serde_json::json!({
        "cases": results,
        "graded": grade.graded,
        "precision": grade.precision,
        "recall": grade.recall,
        "true_positives": grade.true_positives,
        "false_positives": grade.false_positives,
        "false_negatives": grade.false_negatives,
        "prohibition_cases": grade.prohibition_cases,
        "prohibition_rejections": grade.prohibition_rejections,
        "passed": grade.passed,
        "failures": grade.failures,
    });
    if let Some(path) = &args.report {
        write_json(path, &report_json)?;
    }
    if args.json {
        write_json_stdout(&report_json)?;
    } else {
        println!(
            "firewall eval: {} case(s), {} graded",
            results.len(),
            grade.graded
        );
        for result in &results {
            println!(
                "  {} [{}] {} {}",
                result.id,
                result.class,
                result.outcome,
                result
                    .hard_prohibition
                    .as_deref()
                    .map(|p| format!("prohibition={p}"))
                    .unwrap_or_default()
            );
        }
        if grade.graded > 0 {
            println!(
                "  precision {:.4} (>= {:.2}), recall {:.4} (>= {:.2}), hard prohibitions REJECT {}/{}",
                grade.precision,
                args.min_precision,
                grade.recall,
                args.min_recall,
                grade.prohibition_rejections,
                grade.prohibition_cases
            );
        }
        for failure in &grade.failures {
            println!("  FAIL {failure}");
        }
    }
    if grade.passed {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(EXIT_MISMATCH))
    }
}

async fn evaluate_one(
    kernel: &Kernel,
    ctx: &mut FirewallCtx,
    input: &FirewallInput,
    args: &FirewallEvalArgs,
) -> Result<FirewallReport, MmError> {
    kernel
        .logger
        .emit(
            mm_log::LogRecord::new(Level::Info, codes::FIREWALL_SCAN_START, mm_firewall::TARGET)
                .with_trace(input.episode)
                .with_field("episode_id", mm_core::ulid_string(&input.episode))
                .with_field("input_digest", input.digest()),
        )
        .await?;

    let report = mm_firewall::evaluate(input, ctx)
        .await
        .map_err(|e| MmError::Internal(e.to_string()))?;

    if let Some(prohibition) = &report.hard_prohibition {
        kernel
            .logger
            .emit(
                mm_log::LogRecord::new(
                    Level::Warn,
                    codes::FIREWALL_PROHIBITION_SHORTCIRCUIT,
                    mm_firewall::TARGET,
                )
                .with_trace(input.episode)
                .with_field("prohibition", &prohibition.id)
                .with_field("outcome", FirewallOutcome::Reject.as_str())
                .with_field("evidence", &prohibition.evidence),
            )
            .await?;
    }

    kernel
        .logger
        .emit(
            mm_log::LogRecord::new(Level::Info, codes::FIREWALL_AGGREGATE, mm_firewall::TARGET)
                .with_trace(input.episode)
                .with_field("signals", report.reason_code_strings())
                .with_field("outcome", report.outcome.as_str())
                .with_field(
                    "consulted_decision_ids",
                    report
                        .decisions
                        .iter()
                        .map(mm_core::ulid_string)
                        .collect::<Vec<_>>(),
                ),
        )
        .await?;

    if !args.no_record {
        record_firewall_run(kernel, &report, input).await?;
    }
    Ok(report)
}

/// What a corpus evaluation concluded.
struct Grade {
    graded: usize,
    precision: f64,
    recall: f64,
    true_positives: usize,
    false_positives: usize,
    false_negatives: usize,
    prohibition_cases: usize,
    prohibition_rejections: usize,
    passed: bool,
    failures: Vec<String>,
}

/// Grade results against the answer key.
///
/// A result is a *positive* when the firewall did not call the run safe — anything
/// other than `PROCEED` or `PROCEED_WITH_CAUTION`. Caution is counted as safe on
/// purpose: `PROCEED_WITH_CAUTION` is the outcome for a bounded residual risk the
/// caller was told about, and counting it as a fire would make the precision number a
/// measure of the corpus's caution rather than of the firewall's.
fn grade_corpus(
    results: &[CaseResult],
    gold: &BTreeMap<String, GoldLabel>,
    min_precision: f64,
    min_recall: f64,
) -> Grade {
    let safe = |outcome: &str| {
        outcome == FirewallOutcome::Proceed.as_str()
            || outcome == FirewallOutcome::ProceedWithCaution.as_str()
    };
    let mut true_positives = 0usize;
    let mut false_positives = 0usize;
    let mut false_negatives = 0usize;
    let mut prohibition_cases = 0usize;
    let mut prohibition_rejections = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for result in results {
        let Some(label) = gold.get(&result.id) else {
            continue;
        };
        let predicted_unsafe = !safe(&result.outcome);
        match (predicted_unsafe, label.unsafe_outcome) {
            (true, true) => true_positives += 1,
            (true, false) => {
                false_positives += 1;
                failures.push(format!(
                    "{}: gold says safe, firewall said {}",
                    result.id, result.outcome
                ));
            }
            (false, true) => {
                false_negatives += 1;
                failures.push(format!(
                    "{}: gold says unsafe, firewall said {}",
                    result.id, result.outcome
                ));
            }
            (false, false) => {}
        }
        if let Some(expected_prohibition) = &label.hard_prohibition {
            prohibition_cases += 1;
            let rejected = result.outcome == FirewallOutcome::Reject.as_str()
                && result.hard_prohibition.as_deref() == Some(expected_prohibition.as_str());
            if rejected {
                prohibition_rejections += 1;
            } else {
                failures.push(format!(
                    "{}: expected REJECT via {expected_prohibition}, got {} via {:?}",
                    result.id, result.outcome, result.hard_prohibition
                ));
            }
        }
        if !label.outcome.is_empty() && label.outcome != result.outcome {
            failures.push(format!(
                "{}: expected outcome {}, got {}",
                result.id, label.outcome, result.outcome
            ));
        }
    }

    let graded = results.iter().filter(|r| gold.contains_key(&r.id)).count();
    let precision_denominator = true_positives + false_positives;
    let precision = if precision_denominator == 0 {
        1.0
    } else {
        true_positives as f64 / precision_denominator as f64
    };
    let recall_denominator = true_positives + false_negatives;
    let recall = if recall_denominator == 0 {
        1.0
    } else {
        true_positives as f64 / recall_denominator as f64
    };
    let passed = precision + 1e-12 >= min_precision
        && recall + 1e-12 >= min_recall
        && prohibition_rejections == prohibition_cases
        && failures.is_empty();

    Grade {
        graded,
        precision,
        recall,
        true_positives,
        false_positives,
        false_negatives,
        prohibition_cases,
        prohibition_rejections,
        passed,
        failures,
    }
}

/// A `FirewallInput` template: a file, or a bare ULID with a zeroed profile.
fn load_template(arg: &str) -> Result<FirewallInput, MmError> {
    if Path::new(arg).is_file() {
        return serde_json::from_value(read_json_file(Path::new(arg))?)
            .map_err(|e| MmError::Codec(format!("{arg}: is not a firewall input: {e}")));
    }
    let episode = mm_core::id::parse_ulid(arg)?;
    // A bare ULID is accepted because the plan's command line spells the flag that
    // way, and a caller who has not built an input yet still needs a template. The
    // profile is zeroed, which means no risk at all — grade anything that depends on
    // risk through a file, and the input's digest says which of the two it was.
    let profile = serde_json::from_value(serde_json::json!({
        "expected_loss": 0.0,
        "max_loss": 0.0,
        "tail_probability": 0.0,
        "ruin_probability": 0.0,
        "variance": 0.0,
        "downside_asymmetry": 0.0,
        "reversibility": 1.0,
        "optionality": 1.0,
        "measure": "variance",
        "measure_value": 0.0
    }))
    .map_err(|e| MmError::Internal(format!("the zeroed risk profile is malformed: {e}")))?;
    Ok(FirewallInput::new(episode, profile))
}

/// One corpus line: a case identifier, its class, and the fields to override on the
/// template.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CorpusCase {
    /// The case's identifier.
    id: String,
    /// The adversarial class it was built from.
    #[serde(default)]
    class: String,
    /// An optional description.
    #[serde(default)]
    description: String,
    /// Top-level `FirewallInput` fields to replace on the template.
    #[serde(default)]
    set: Value,
}

/// Read a JSONL corpus, merging each case's overrides onto the template.
///
/// The corpus is a list of *deltas* rather than of whole inputs because an input is
/// thirty fields wide and a case changes one or two of them; a reviewer reading the
/// corpus should see the change, not the boilerplate around it.
fn load_cases(
    path: &Path,
    template: &FirewallInput,
) -> Result<Vec<(String, String, FirewallInput)>, MmError> {
    let mut out = Vec::new();
    for (line_no, line) in read_lines(path)?.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let case: CorpusCase = serde_json::from_str(line)
            .map_err(|e| MmError::Codec(format!("{}:{}: {e}", path.display(), line_no + 1)))?;
        let mut base = serde_json::to_value(template)
            .map_err(|e| MmError::Internal(format!("cannot render the template: {e}")))?;
        if let (Value::Object(base), Value::Object(set)) = (&mut base, &case.set) {
            for (key, value) in set {
                base.insert(key.clone(), value.clone());
            }
        }
        let input: FirewallInput = serde_json::from_value(base)
            .map_err(|e| MmError::Codec(format!("{}:{}: {}", path.display(), line_no + 1, e)))?;
        out.push((case.id, case.class, input));
    }
    if out.is_empty() {
        return Err(MmError::Config(format!(
            "{} holds no cases; the corpus appears to be empty rather than to hold zero cases",
            path.display()
        )));
    }
    Ok(out)
}

/// Read the answer key, keyed by case id.
fn load_gold(path: &Path) -> Result<BTreeMap<String, GoldLabel>, MmError> {
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    let mut out = BTreeMap::new();
    for (line_no, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let label: GoldLabel = serde_json::from_str(line)
            .map_err(|e| MmError::Codec(format!("{}:{}: {e}", path.display(), line_no + 1)))?;
        out.insert(label.id.clone(), label);
    }
    Ok(out)
}

fn read_lines(path: &Path) -> Result<Vec<String>, MmError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    Ok(text.lines().map(str::to_string).collect())
}

// ------------------------------------------------------------------ compare ----

/// `mm-cli compare` subcommands.
#[derive(Subcommand, Debug)]
pub enum CompareCommand {
    /// Check a pair of comparison contracts, or a fixtures corpus.
    Check(CompareCheckArgs),
}

/// Arguments to `mm-cli compare check`.
#[derive(Args, Debug)]
pub struct CompareCheckArgs {
    /// A JSONL corpus of `{name, a, b, verdict, violation_codes}` fixtures.
    #[arg(long, value_name = "FILE|STDIN")]
    pub fixtures: Option<String>,
    /// The baseline contract, with `--b`.
    #[arg(long, value_name = "FILE")]
    pub a: Option<PathBuf>,
    /// The candidate contract, with `--a`.
    #[arg(long, value_name = "FILE")]
    pub b: Option<PathBuf>,
    /// Print the checks as JSON.
    #[arg(long)]
    pub json: bool,
    /// Check and report without writing anything.
    #[arg(long)]
    pub no_record: bool,
}

/// One fixture line: two contracts and the verdict they must produce.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompareFixture {
    /// A short name for the fixture.
    pub name: String,
    /// The baseline contract.
    pub a: mm_decision::ComparisonContract,
    /// The candidate contract.
    pub b: mm_decision::ComparisonContract,
    /// The verdict the pair must produce.
    pub verdict: String,
    /// The violation codes the pair must produce, when the verdict is not `valid`.
    #[serde(default)]
    pub violation_codes: Vec<String>,
}

/// Check comparison contracts.
pub async fn run_compare(cfg: Config, command: CompareCommand) -> Result<ExitCode, MmError> {
    match command {
        CompareCommand::Check(args) => compare_check(cfg, args).await,
    }
}

async fn compare_check(cfg: Config, args: CompareCheckArgs) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, !args.no_record).await?;
    let fixtures = match (&args.fixtures, &args.a, &args.b) {
        (Some(source), _, _) => load_fixtures(source)?,
        (None, Some(a), Some(b)) => vec![CompareFixture {
            name: "(a, b)".to_string(),
            a: deserialize_contract(a)?,
            b: deserialize_contract(b)?,
            verdict: String::new(),
            violation_codes: Vec::new(),
        }],
        _ => {
            return Err(MmError::Config(
                "compare check needs --fixtures, or both --a and --b".to_string(),
            ))
        }
    };

    let mut mismatches = 0usize;
    let mut checks: Vec<Value> = Vec::new();
    for fixture in &fixtures {
        kernel
            .logger
            .emit(
                mm_log::LogRecord::new(Level::Info, codes::COMPARE_CONTRACT_START, TARGET)
                    .with_field("objective", &fixture.a.objective)
                    .with_field(
                        "objects",
                        fixture
                            .a
                            .objects
                            .iter()
                            .chain(fixture.b.objects.iter())
                            .cloned()
                            .collect::<Vec<_>>(),
                    )
                    .with_field(
                        "dimensions",
                        fixture
                            .a
                            .dimensions
                            .iter()
                            .chain(fixture.b.dimensions.iter())
                            .map(|d| d.name.clone())
                            .collect::<Vec<_>>(),
                    )
                    .with_field(
                        "units",
                        fixture
                            .a
                            .units
                            .iter()
                            .chain(fixture.b.units.iter())
                            .map(|u| u.name.clone())
                            .collect::<Vec<_>>(),
                    )
                    .with_field("timeframe", fixture.a.timeframe.canonical()),
            )
            .await?;

        let check = mm_decision::check_contract(&fixture.a, &fixture.b);
        let codes = check.violation_codes();
        kernel
            .logger
            .emit(
                mm_log::LogRecord::new(Level::Info, codes::COMPARE_CHECK, TARGET)
                    .with_field("verdict", check.verdict.as_str())
                    .with_field("violation_codes", &codes),
            )
            .await?;

        if let Some(normalized) = &check.normalized {
            // `timeframe_shift` is the window the two timeframes were reconciled to,
            // and `None` when they were already the same — not the whole normalized
            // rendering, which the field's name does not describe.
            let timeframe_shift = normalized
                .conversions
                .iter()
                .find_map(|conversion| conversion.strip_prefix("timeframe -> "))
                .map(str::to_string);
            kernel
                .logger
                .emit(
                    mm_log::LogRecord::new(Level::Info, codes::COMPARE_NORMALIZE, TARGET)
                        .with_field("applied_conversions", &normalized.conversions)
                        .with_field("timeframe_shift", timeframe_shift),
                )
                .await?;
        }

        if !args.no_record {
            record_comparison(&kernel, fixture, &check).await?;
        }

        let mut fixture_failures: Vec<String> = Vec::new();
        if !fixture.verdict.is_empty() && fixture.verdict != check.verdict.as_str() {
            fixture_failures.push(format!(
                "expected {}, got {}",
                fixture.verdict,
                check.verdict.as_str()
            ));
        }
        let mut missing: Vec<String> = fixture
            .violation_codes
            .iter()
            .filter(|code| !codes.contains(code))
            .cloned()
            .collect();
        missing.sort();
        if !missing.is_empty() {
            fixture_failures.push(format!("missing violations {missing:?}"));
        }
        mismatches += fixture_failures.len();

        checks.push(serde_json::json!({
            "name": fixture.name,
            "verdict": check.verdict.as_str(),
            "violation_codes": codes,
            "normalized": check.normalized.as_ref().map(|n| n.canonical.clone()),
            "failures": fixture_failures,
        }));
    }

    if args.json {
        write_json_stdout(&serde_json::json!({
            "fixtures": fixtures.len(),
            "mismatches": mismatches,
            "checks": checks,
        }))?;
    } else {
        println!(
            "compare check: {} fixture(s), {mismatches} mismatch(es)",
            fixtures.len()
        );
        for check in &checks {
            let codes = serde_json::to_string(&check["violation_codes"])
                .unwrap_or_else(|_| "[]".to_string());
            println!("  {} -> {} {codes}", check["name"], check["verdict"]);
            for failure in check["failures"].as_array().into_iter().flatten() {
                println!("    FAIL {failure}");
            }
        }
    }
    if mismatches == 0 {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(EXIT_MISMATCH))
    }
}

async fn record_comparison(
    kernel: &Kernel,
    fixture: &CompareFixture,
    check: &mm_decision::ComparisonCheck,
) -> Result<(), MmError> {
    let id = kernel.ids.next();
    let created = kernel.ids.next();
    let contract = serde_json::to_string(&(fixture.a.clone(), fixture.b.clone()))
        .map_err(|e| MmError::Internal(format!("cannot render the contracts: {e}")))?;
    let violations = serde_json::to_string(&check.violations)
        .map_err(|e| MmError::Internal(format!("cannot render the violations: {e}")))?;
    let normalized = match &check.normalized {
        Some(normalized) => Some(
            serde_json::to_string(normalized)
                .map_err(|e| MmError::Internal(format!("cannot render the normalization: {e}")))?,
        ),
        None => None,
    };
    kernel
        .sqlite
        .execute(
            "INSERT INTO comparisons (id, objective, contract_json, verdict, violations_json, \
             normalized_json, created_ulid) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            vec![
                mm_core::ulid_string(&id).into(),
                fixture.a.objective.clone().into(),
                contract.into(),
                check.verdict.as_str().into(),
                violations.into(),
                normalized.into(),
                mm_core::ulid_string(&created).into(),
            ],
        )
        .await?;
    Ok(())
}

fn deserialize_contract(path: &Path) -> Result<mm_decision::ComparisonContract, MmError> {
    let value = read_json_file(path)?;
    let spec: ContractSpec = serde_json::from_value(value)
        .map_err(|e| MmError::Codec(format!("{}: is not a contract: {e}", path.display())))?;
    Ok(spec.into_contract())
}

fn load_fixtures(source: &str) -> Result<Vec<CompareFixture>, MmError> {
    let text = if source == "-" {
        use std::io::Read;
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| MmError::Config(format!("cannot read stdin: {e}")))?;
        text
    } else {
        std::fs::read_to_string(source)
            .map_err(|e| MmError::Config(format!("cannot read {source}: {e}")))?
    };
    let mut out = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|e| MmError::Codec(format!("{source}:{}: {e}", line_no + 1)))?;
        let mut a_value = value.get("a").cloned().unwrap_or(Value::Null);
        let mut b_value = value.get("b").cloned().unwrap_or(Value::Null);
        crate::decision::lowercase_id_strings(&mut a_value);
        crate::decision::lowercase_id_strings(&mut b_value);
        let a: ContractSpec = serde_json::from_value(a_value)
            .map_err(|e| MmError::Codec(format!("{source}:{}: contract a: {e}", line_no + 1)))?;
        let b: ContractSpec = serde_json::from_value(b_value)
            .map_err(|e| MmError::Codec(format!("{source}:{}: contract b: {e}", line_no + 1)))?;
        out.push(CompareFixture {
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("(unnamed)")
                .to_string(),
            a: a.into_contract(),
            b: b.into_contract(),
            verdict: value
                .get("verdict")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            violation_codes: value
                .get("violation_codes")
                .and_then(Value::as_array)
                .map(|codes| {
                    codes
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    if out.is_empty() {
        return Err(MmError::Config(format!("{source} holds no fixtures")));
    }
    Ok(out)
}

/// A comparison contract as a fixture writes it.
///
/// The typed contract carries a `BTreeMap` of definitions and a `BTreeMap` of values;
/// a fixture that omits either would fail to deserialize, and a reviewer writing a
/// fixture should not have to write `{}` in two places to say "nothing here". The
/// spec fills those in and nothing else.
#[derive(Clone, Debug, Deserialize)]
struct ContractSpec {
    objective: String,
    object_class: String,
    #[serde(default)]
    objects: Vec<String>,
    #[serde(default)]
    dimensions: Vec<mm_decision::Dimension>,
    #[serde(default)]
    units: Vec<mm_decision::Unit>,
    timeframe: mm_decision::Timeframe,
    #[serde(default)]
    conditions: Vec<mm_decision::Condition>,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    definitions: BTreeMap<String, String>,
    #[serde(default)]
    evidence: Vec<String>,
    #[serde(default)]
    values: BTreeMap<String, f64>,
}

impl ContractSpec {
    fn into_contract(self) -> mm_decision::ComparisonContract {
        mm_decision::ComparisonContract {
            objective: self.objective,
            object_class: self.object_class,
            objects: self.objects,
            dimensions: self.dimensions,
            units: self.units,
            timeframe: self.timeframe,
            conditions: self.conditions,
            constraints: self.constraints,
            definitions: self.definitions,
            evidence: self
                .evidence
                .iter()
                .filter_map(|text| mm_core::id::parse_ulid(text).ok())
                .collect(),
            values: self.values,
        }
    }
}

// --------------------------------------------------------------------- risk ----

/// `mm-cli risk` subcommands.
#[derive(Subcommand, Debug)]
pub enum RiskCommand {
    /// Compute the eight measures for every case and compare them with the reference.
    Analyze(RiskAnalyzeArgs),
}

/// Arguments to `mm-cli risk analyze`.
#[derive(Args, Debug)]
pub struct RiskAnalyzeArgs {
    /// The outcomes corpus: `{cases: [{...}]}`.
    #[arg(long, value_name = "FILE")]
    pub outcomes: PathBuf,
    /// Print only this measure's value alongside the profile.
    #[arg(long, value_name = "NAME")]
    pub measure: Option<String>,
    /// How close a computed value must be to its reference.
    #[arg(long, default_value_t = 1e-6)]
    pub tolerance: f64,
    /// Print the profiles as JSON.
    #[arg(long)]
    pub json: bool,
}

/// One risk case as the corpus writes it.
#[derive(Clone, Debug, Deserialize)]
pub struct RiskCase {
    /// A short name.
    pub name: String,
    /// The risk budget a ruin is measured against.
    pub capital: f64,
    /// The caller's reversibility judgment.
    #[serde(default = "half")]
    pub reversibility: f32,
    /// The caller's optionality judgment.
    #[serde(default = "half")]
    pub optionality: f32,
    /// The measure to report.
    pub measure: MeasureSpec,
    /// The loss distribution.
    pub outcomes: Vec<mm_decision::LossOutcome>,
    /// The hand-computed reference values.
    pub expected: BTreeMap<String, f64>,
}

fn half() -> f32 {
    0.5
}

/// A risk measure as the corpus writes it.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeasureSpec {
    /// The variance of the outcome utilities.
    Variance,
    /// The value at risk at level `alpha`.
    Var {
        /// The tail level.
        alpha: f64,
    },
    /// The conditional value at risk at level `alpha`.
    Cvar {
        /// The tail level.
        alpha: f64,
    },
}

impl MeasureSpec {
    fn measure(self) -> mm_decision::RiskMeasure {
        match self {
            MeasureSpec::Variance => mm_decision::RiskMeasure::Variance,
            MeasureSpec::Var { alpha } => mm_decision::RiskMeasure::Var { alpha },
            MeasureSpec::Cvar { alpha } => mm_decision::RiskMeasure::Cvar { alpha },
        }
    }
}

/// Compute every case's profile and compare it with the reference.
pub async fn run_risk(cfg: Config, command: RiskCommand) -> Result<ExitCode, MmError> {
    match command {
        RiskCommand::Analyze(args) => risk_analyze(cfg, args).await,
    }
}

async fn risk_analyze(cfg: Config, args: RiskAnalyzeArgs) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, false).await?;
    let value = read_json_file(&args.outcomes)?;
    let cases: Vec<RiskCase> =
        serde_json::from_value(value.get("cases").cloned().ok_or_else(|| {
            MmError::Codec(format!(
                "{}: needs a `cases` array",
                args.outcomes.display()
            ))
        })?)
        .map_err(|e| MmError::Codec(format!("{}: {e}", args.outcomes.display())))?;

    let mut mismatches = 0usize;
    let mut profiles: Vec<Value> = Vec::new();
    for case in &cases {
        let profile = mm_decision::analyze_risk_with(
            &case.outcomes,
            case.measure.measure(),
            &mm_decision::RiskOptions {
                capital: case.capital,
                reversibility: case.reversibility,
                optionality: case.optionality,
            },
        );
        let computed = profile_fields(&profile);
        let mut failures: Vec<String> = Vec::new();
        for (field, reference) in &case.expected {
            match computed.get(field) {
                Some(actual) if (actual - reference).abs() <= args.tolerance => {}
                Some(actual) => failures.push(format!(
                    "{field}: computed {actual} vs reference {reference} (tolerance {})",
                    args.tolerance
                )),
                None => failures.push(format!("{field}: is not a measure this crate reports")),
            }
        }
        if let Some(wanted) = &args.measure {
            if !computed.contains_key(wanted) {
                failures.push(format!("--measure {wanted} is not a measure"));
            }
        }
        mismatches += failures.len();

        kernel
            .logger
            .emit(
                mm_log::LogRecord::new(Level::Info, codes::RISK_ANALYZE, TARGET)
                    .with_field("measure", profile.measure.name())
                    .with_field("expected_loss", profile.expected_loss)
                    .with_field("max_loss", profile.max_loss)
                    .with_field("tail_probability", profile.tail_probability)
                    .with_field("ruin_probability", profile.ruin_probability)
                    .with_field("variance", profile.variance)
                    .with_field("downside_asymmetry", profile.downside_asymmetry)
                    .with_field("reversibility", profile.reversibility)
                    .with_field("optionality", profile.optionality),
            )
            .await?;

        profiles.push(serde_json::json!({
            "name": case.name,
            "measure": profile.measure.name(),
            "computed": computed,
            "expected": case.expected,
            "failures": failures,
        }));
    }

    if args.json {
        write_json_stdout(&serde_json::json!({
            "cases": cases.len(),
            "mismatches": mismatches,
            "profiles": profiles,
        }))?;
    } else {
        println!(
            "risk analyze: {} case(s), {mismatches} field mismatch(es), tolerance {}",
            cases.len(),
            args.tolerance
        );
        for profile in &profiles {
            println!(
                "  {} [{}] expected_loss={} max_loss={} ruin={}",
                profile["name"],
                profile["measure"],
                profile["computed"]["expected_loss"],
                profile["computed"]["max_loss"],
                profile["computed"]["ruin_probability"]
            );
            for failure in profile["failures"].as_array().into_iter().flatten() {
                println!("    FAIL {failure}");
            }
        }
    }
    if mismatches == 0 {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(EXIT_MISMATCH))
    }
}

fn profile_fields(profile: &mm_decision::RiskProfile) -> BTreeMap<String, f64> {
    BTreeMap::from([
        ("expected_loss".to_string(), profile.expected_loss),
        ("max_loss".to_string(), profile.max_loss),
        ("tail_probability".to_string(), profile.tail_probability),
        ("ruin_probability".to_string(), profile.ruin_probability),
        ("variance".to_string(), profile.variance),
        ("downside_asymmetry".to_string(), profile.downside_asymmetry),
        (
            "reversibility".to_string(),
            f64::from(profile.reversibility),
        ),
        ("optionality".to_string(), f64::from(profile.optionality)),
        ("measure_value".to_string(), profile.measure_value),
    ])
}

// -------------------------------------------------------------- calibration ----

/// `mm-cli calibration` subcommands.
#[derive(Subcommand, Debug)]
pub enum CalibrationCommand {
    /// Fit one temperature per decision class and report ECE before and after.
    Fit(CalibrationFitArgs),
    /// Fit one split-conformal threshold per decision class and report coverage.
    Conformal(CalibrationConformalArgs),
}

/// Arguments to `mm-cli calibration fit`.
#[derive(Args, Debug)]
pub struct CalibrationFitArgs {
    /// JSONL of `{class, predicted, label}`.
    #[arg(
        long,
        value_name = "FILE",
        default_value = "bench/calibration/labeled.jsonl"
    )]
    pub labeled: PathBuf,
    /// Fit only this class.
    #[arg(long, value_name = "NAME")]
    pub class: Option<String>,
    /// Print the calibrators as JSON.
    #[arg(long)]
    pub json: bool,
}

/// One labeled prediction.
#[derive(Clone, Debug, Deserialize)]
pub struct LabeledPoint {
    /// The decision class the prediction belongs to.
    pub class: String,
    /// The pre-calibration confidence.
    pub predicted: f32,
    /// Whether the prediction was right.
    pub label: bool,
}

/// Arguments to `mm-cli calibration conformal`.
#[derive(Args, Debug)]
pub struct CalibrationConformalArgs {
    /// JSONL of `{class, score}` nonconformity scores.
    #[arg(
        long,
        value_name = "FILE",
        default_value = "bench/calibration/conformal.jsonl"
    )]
    pub scores: PathBuf,
    /// The coverage the threshold must achieve.
    #[arg(long, default_value_t = 0.9)]
    pub coverage: f32,
    /// How far the held-out coverage may sit from the target.
    #[arg(long, default_value_t = 0.03)]
    pub tolerance: f32,
    /// Print the thresholds as JSON.
    #[arg(long)]
    pub json: bool,
}

/// One nonconformity score.
#[derive(Clone, Debug, Deserialize)]
pub struct ScorePoint {
    /// The decision class the score belongs to.
    pub class: String,
    /// `1 - p` for the prediction the score came from.
    pub score: f32,
}

/// Fit calibrators and conformal thresholds.
pub async fn run_calibration(
    cfg: Config,
    command: CalibrationCommand,
) -> Result<ExitCode, MmError> {
    match command {
        CalibrationCommand::Fit(args) => calibration_fit(cfg, args).await,
        CalibrationCommand::Conformal(args) => calibration_conformal(cfg, args).await,
    }
}

async fn calibration_fit(cfg: Config, args: CalibrationFitArgs) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, false).await?;
    let points = load_labeled(&args.labeled)?;
    let mut by_class: BTreeMap<String, (Vec<f32>, Vec<bool>)> = BTreeMap::new();
    for point in &points {
        let slot = by_class.entry(point.class.clone()).or_default();
        slot.0.push(point.predicted);
        slot.1.push(point.label);
    }

    let mut rows: Vec<Value> = Vec::new();
    let mut regressions = 0usize;
    for (class, (predicted, labels)) in &by_class {
        if let Some(wanted) = &args.class {
            if wanted != class {
                continue;
            }
        }
        let calibrator = mm_decision::Calibrator::fit(class, predicted, labels);
        if calibrator.ece > calibrator.ece_before + 1e-6 {
            regressions += 1;
        }
        kernel
            .logger
            .emit(
                mm_log::LogRecord::new(Level::Info, codes::CALIBRATION_FIT, TARGET)
                    .with_field("decision_class", class)
                    .with_field("n", calibrator.n)
                    .with_field("temperature", calibrator.temperature)
                    .with_field("ece_before", calibrator.ece_before)
                    .with_field("ece_after", calibrator.ece)
                    .with_field("brier", calibrator.brier),
            )
            .await?;

        let id = kernel.ids.next();
        let created = kernel.ids.next();
        kernel
            .sqlite
            .execute(
                "INSERT INTO calibration (id, decision_class, temperature, ece, brier, n, \
                 created_ulid) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                vec![
                    mm_core::ulid_string(&id).into(),
                    class.clone().into(),
                    f64::from(calibrator.temperature).into(),
                    f64::from(calibrator.ece).into(),
                    f64::from(calibrator.brier).into(),
                    i64::try_from(calibrator.n).unwrap_or(i64::MAX).into(),
                    mm_core::ulid_string(&created).into(),
                ],
            )
            .await?;

        rows.push(serde_json::json!({
            "decision_class": class,
            "n": calibrator.n,
            "calibrated": calibrator.is_calibrated(),
            "temperature": calibrator.temperature,
            "ece_before": calibrator.ece_before,
            "ece_after": calibrator.ece,
            "brier": calibrator.brier,
        }));
    }

    if args.class.is_some() && rows.is_empty() {
        return Err(MmError::Config(format!(
            "{} holds no data for class {:?}",
            args.labeled.display(),
            args.class
        )));
    }

    if args.json {
        write_json_stdout(&serde_json::json!({
            "points": points.len(),
            "classes": rows.len(),
            "regressions": regressions,
            "calibrators": rows,
        }))?;
    } else {
        println!(
            "calibration fit: {} point(s), {} class(es)",
            points.len(),
            rows.len()
        );
        for row in &rows {
            println!(
                "  {} n={} T={:.4} ECE {:.4} -> {:.4} brier {:.4}{}",
                row["decision_class"],
                row["n"],
                row["temperature"],
                row["ece_before"],
                row["ece_after"],
                row["brier"],
                if row["calibrated"].as_bool() == Some(true) {
                    ""
                } else {
                    " (uncalibrated: too few points)"
                }
            );
        }
    }
    if regressions == 0 {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(EXIT_MISMATCH))
    }
}

async fn calibration_conformal(
    cfg: Config,
    args: CalibrationConformalArgs,
) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, false).await?;
    let points = load_scores(&args.scores)?;
    let mut by_class: BTreeMap<String, Vec<f32>> = BTreeMap::new();
    for point in &points {
        by_class
            .entry(point.class.clone())
            .or_default()
            .push(point.score);
    }

    let mut rows: Vec<Value> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for (class, scores) in &by_class {
        let (set, held_out) = mm_decision::fit_held_out(class, scores, args.coverage);
        let deviation = (held_out - args.coverage).abs();
        if deviation > args.tolerance {
            failures.push(format!(
                "{class}: held-out coverage {held_out:.4} deviates {deviation:.4} from {} \
                 (tolerance {})",
                args.coverage, args.tolerance
            ));
        }
        kernel
            .logger
            .emit(
                mm_log::LogRecord::new(Level::Info, codes::CONFORMAL_ADJUST, TARGET)
                    .with_field("decision_class", class)
                    .with_field("target_coverage", args.coverage)
                    .with_field("q_hat", set.q_hat)
                    .with_field("n", set.n)
                    .with_field("empirical_coverage", held_out),
            )
            .await?;

        let id = kernel.ids.next();
        let created = kernel.ids.next();
        // The row's CHECK requires a coverage strictly inside `(0,1)`, and a target of
        // exactly 0 or 1 is not a coverage any threshold can serve — `q_hat` would be
        // the smallest or the largest score, admitting almost nothing or everything.
        // The stored value is nudged inside the open interval, and the *requested*
        // target is what the log record above reports.
        let stored_coverage = args.coverage.clamp(f32::EPSILON, 1.0 - f32::EPSILON);
        kernel
            .sqlite
            .execute(
                "INSERT INTO conformal_thresholds (id, decision_class, target_coverage, q_hat, \
                 n, created_ulid) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                vec![
                    mm_core::ulid_string(&id).into(),
                    class.clone().into(),
                    f64::from(stored_coverage).into(),
                    f64::from(set.q_hat.clamp(0.0, 1.0)).into(),
                    i64::try_from(set.n).unwrap_or(i64::MAX).into(),
                    mm_core::ulid_string(&created).into(),
                ],
            )
            .await?;

        rows.push(serde_json::json!({
            "decision_class": class,
            "n": set.n,
            "target_coverage": set.coverage,
            "q_hat": set.q_hat,
            "empirical_coverage": held_out,
            "deviation": deviation,
        }));
    }

    let passed = failures.is_empty();
    if args.json {
        write_json_stdout(&serde_json::json!({
            "scores": points.len(),
            "target_coverage": args.coverage,
            "tolerance": args.tolerance,
            "passed": passed,
            "failures": failures,
            "thresholds": rows,
        }))?;
    } else {
        println!(
            "calibration conformal: {} score(s), {} class(es), target {:.2} +/- {:.2}",
            points.len(),
            rows.len(),
            args.coverage,
            args.tolerance
        );
        for row in &rows {
            println!(
                "  {} n={} q_hat={:.6} held-out coverage {:.4} (deviation {:.4})",
                row["decision_class"],
                row["n"],
                row["q_hat"],
                row["empirical_coverage"],
                row["deviation"]
            );
        }
        for failure in &failures {
            println!("  FAIL {failure}");
        }
    }
    if passed {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(EXIT_MISMATCH))
    }
}

fn load_labeled(path: &Path) -> Result<Vec<LabeledPoint>, MmError> {
    let mut out = Vec::new();
    for (line_no, line) in read_lines(path)?.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        out.push(
            serde_json::from_str(line)
                .map_err(|e| MmError::Codec(format!("{}:{}: {e}", path.display(), line_no + 1)))?,
        );
    }
    if out.is_empty() {
        return Err(MmError::Config(format!(
            "{} holds no points",
            path.display()
        )));
    }
    Ok(out)
}

fn load_scores(path: &Path) -> Result<Vec<ScorePoint>, MmError> {
    let mut out = Vec::new();
    for (line_no, line) in read_lines(path)?.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        out.push(
            serde_json::from_str(line)
                .map_err(|e| MmError::Codec(format!("{}:{}: {e}", path.display(), line_no + 1)))?,
        );
    }
    if out.is_empty() {
        return Err(MmError::Config(format!(
            "{} holds no scores",
            path.display()
        )));
    }
    Ok(out)
}

// ------------------------------------------------------------------- output ----

fn write_json_stdout(value: &Value) -> Result<(), MmError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| MmError::Internal(format!("cannot serialize: {e}")))?;
    println!("{text}");
    Ok(())
}

fn print_json(as_json: bool, value: &Value) -> Result<(), MmError> {
    if as_json {
        write_json_stdout(value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(id: &str, outcome: FirewallOutcome, prohibition: Option<&str>) -> CaseResult {
        CaseResult {
            id: id.to_string(),
            class: "test".to_string(),
            outcome: outcome.as_str().to_string(),
            hard_prohibition: prohibition.map(str::to_string),
            reason_codes: Vec::new(),
        }
    }

    fn label(id: &str, expected_unsafe: bool, prohibition: Option<&str>) -> GoldLabel {
        GoldLabel {
            id: id.to_string(),
            outcome: String::new(),
            hard_prohibition: prohibition.map(str::to_string),
            unsafe_outcome: expected_unsafe,
        }
    }

    #[test]
    fn a_corpus_with_no_false_alarms_and_no_misses_passes() {
        let results = vec![
            case("a", FirewallOutcome::Reject, Some("unauthorized_tool")),
            case("b", FirewallOutcome::Proceed, None),
        ];
        let gold = BTreeMap::from([
            ("a".to_string(), label("a", true, Some("unauthorized_tool"))),
            ("b".to_string(), label("b", false, None)),
        ]);
        let grade = grade_corpus(&results, &gold, 0.95, 0.90);
        assert!(grade.passed, "{:?}", grade.failures);
        assert_eq!(grade.precision, 1.0);
        assert_eq!(grade.recall, 1.0);
        assert_eq!(grade.prohibition_rejections, 1);
    }

    #[test]
    fn a_caution_is_counted_as_safe_rather_than_as_a_fire() {
        let results = vec![case("a", FirewallOutcome::ProceedWithCaution, None)];
        let gold = BTreeMap::from([("a".to_string(), label("a", false, None))]);
        let grade = grade_corpus(&results, &gold, 0.95, 0.90);
        assert!(grade.passed, "{:?}", grade.failures);
        assert_eq!(grade.false_positives, 0);
        assert_eq!(grade.true_positives, 0);
    }

    #[test]
    fn one_waved_through_unsafe_case_fails_the_recall_gate() {
        let results = vec![
            case("a", FirewallOutcome::Reject, None),
            case("b", FirewallOutcome::Proceed, None),
        ];
        let gold = BTreeMap::from([
            ("a".to_string(), label("a", true, None)),
            ("b".to_string(), label("b", true, None)),
        ]);
        let grade = grade_corpus(&results, &gold, 0.95, 0.90);
        assert!(!grade.passed);
        assert_eq!(grade.false_negatives, 1);
        assert_eq!(grade.recall, 0.5);
    }

    #[test]
    fn a_prohibition_case_that_is_not_rejected_fails_even_when_recall_is_fine() {
        let results = vec![case("a", FirewallOutcome::HumanReview, Some("other"))];
        let gold = BTreeMap::from([("a".to_string(), label("a", true, Some("unauthorized_tool")))]);
        let grade = grade_corpus(&results, &gold, 0.95, 0.01);
        assert!(!grade.passed);
        assert_eq!(grade.prohibition_rejections, 0);
        assert_eq!(grade.prohibition_cases, 1);
    }

    #[test]
    fn a_declared_outcome_that_differs_is_a_failure() {
        let results = vec![case("a", FirewallOutcome::VerifyFirst, None)];
        let gold = BTreeMap::from([(
            "a".to_string(),
            GoldLabel {
                id: "a".to_string(),
                outcome: "REPLAN".to_string(),
                hard_prohibition: None,
                unsafe_outcome: true,
            },
        )]);
        let grade = grade_corpus(&results, &gold, 0.0, 0.0);
        assert!(!grade.passed);
        assert!(grade
            .failures
            .iter()
            .any(|failure| failure.contains("expected outcome REPLAN")));
    }

    #[test]
    fn a_bare_ulid_template_is_accepted_and_zeroed() {
        let template = load_template("01hf7yat000000000000000001").unwrap();
        assert_eq!(template.risks.max_loss, 0.0);
        assert_eq!(template.risks.ruin_probability, 0.0);
    }

    #[test]
    fn the_measure_spec_reads_the_corpus_spelling() {
        let spec: MeasureSpec = serde_json::from_str(r#"{"kind":"var","alpha":0.9}"#).unwrap();
        assert!(matches!(spec.measure(), mm_decision::RiskMeasure::Var { alpha } if alpha == 0.9));
        let spec: MeasureSpec = serde_json::from_str(r#"{"kind":"variance"}"#).unwrap();
        assert!(matches!(spec.measure(), mm_decision::RiskMeasure::Variance));
    }

    #[test]
    fn a_contract_fixture_may_omit_its_maps() {
        let spec: ContractSpec = serde_json::from_str(
            r#"{"objective":"o","object_class":"c","timeframe":{"start":"a","end":"b"},"objects":["x"]}"#,
        )
        .unwrap();
        let contract = spec.into_contract();
        assert!(contract.definitions.is_empty());
        assert!(contract.values.is_empty());
        assert!(contract.evidence.is_empty());
    }

    #[test]
    fn the_defaults_match_the_plan() {
        assert_eq!(mm_firewall::DEFAULT_TARGET_COVERAGE, 0.9);
        assert_eq!(mm_firewall::DECISION_CLASSES.len(), 5);
        assert_eq!(mm_firewall::RUIN_REVIEW_THRESHOLD, 0.05);
    }
}
