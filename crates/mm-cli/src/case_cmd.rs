//! `mm-cli case` — retrieve precedents for a problem.
//!
//! Retrieval is **structural**: two problems correspond when they use the same
//! relations, so the query is Turtle and the mapping it produces is printed rather
//! than hidden behind a similarity number. The rank is `case_utility`
//! (`similarity × outcome × transferability × evidence`), which is multiplicative
//! because a precedent that did not work, or cannot transfer, is worth nothing.
//!
//! **What is ranked.** Two sources, concatenated in a deterministic order:
//!
//! 1. every row of the `cases` table, which is the projection `library import`
//!    writes for `mm:Case` entries; and
//! 2. any case corpus the fixture line declares under `"cases"` — a gold fixture
//!    has to be able to state the precedents it expects *and* the distractors it
//!    expects them to beat, or the assertion is vacuous.
//!
//! A case whose problem is not Turtle is reported and skipped rather than failing
//! the run: the seed corpus stores its problems as prose, and a prose problem is
//! an unreadable one, not a fatal error.
//!
//! **Fixture schema** (one JSON object per line, `bench/library/gold_cases.jsonl`):
//!
//! ```json
//! {"problem_ttl": "<turtle>", "expected": ["<iri>"], "top": 3,
//!  "cases": [{"iri": "<iri>", "kind": "success", "problem_ttl": "<turtle>",
//!             "solution_ttl": "<turtle>", "outcome_ttl": "<turtle>",
//!             "outcome_quality": 0.9, "transferability": 0.8,
//!             "evidence_quality": 0.85}]}
//! ```
//!
//! The command exits non-zero when a line names `expected` precedents and the top
//! hit is not among them — that is the whole point of a gold fixture.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use mm_core::{Config, MmError, NamedNode, Param};
use mm_library::case_db::{retrieve as retrieve_cases, Case, CaseKind, CaseQuery};
use mm_log::{codes, Level, LogRecord};
use mm_store_sqlite::SqliteStore;
use serde_json::Value;

use crate::library::{open_manager, shutdown};

/// `mm-cli case`.
#[derive(Subcommand, Debug)]
pub enum CaseCommand {
    /// Retrieve the most useful precedents for a problem fixture.
    Retrieve {
        /// JSONL of `{problem_ttl, expected?, top?, cases?}`.
        #[arg(long, value_name = "FILE")]
        problem: PathBuf,
        /// How many precedents to return per line.
        #[arg(long, default_value_t = 3)]
        top: usize,
    },
}

/// One line of a retrieval fixture.
#[derive(Debug, Clone, PartialEq)]
struct Probe {
    problem_ttl: String,
    expected: Vec<String>,
    top: Option<usize>,
    cases: Vec<Case>,
}

/// `mm-cli case`.
pub async fn run(cfg: Config, command: CaseCommand) -> Result<ExitCode, MmError> {
    match command {
        CaseCommand::Retrieve { problem, top } => retrieve_cmd(cfg, problem, top).await,
    }
}

/// Retrieve precedents for every probe in a fixture.
async fn retrieve_cmd(cfg: Config, problem: PathBuf, top: usize) -> Result<ExitCode, MmError> {
    let raw = std::fs::read_to_string(&problem)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", problem.display())))?;
    let probes = parse_probes(&raw)?;

    let (kernel, manager) = open_manager(cfg).await?;
    let (stored, skipped) = stored_cases(manager.store().sqlite()).await?;
    for (iri, reason) in &skipped {
        println!("skipped {iri}: {reason}");
    }

    let mut failures = 0usize;
    for probe in &probes {
        let mut candidates = stored.clone();
        candidates.extend(probe.cases.iter().cloned());
        let limit = probe.top.unwrap_or(top);
        let hits = retrieve_cases(
            &candidates,
            &CaseQuery {
                problem_ttl: probe.problem_ttl.clone(),
                top: limit,
            },
        )
        .map_err(|e| {
            MmError::Config(format!(
                "problem in {} is not readable Turtle: {e}",
                problem.display()
            ))
        })?;

        println!("problem  {} case(s) ranked, top {limit}", candidates.len());
        for hit in &hits {
            println!("  {:.4} {} ({})", hit.utility, hit.iri, hit.kind);
            for correspondence in &hit.correspondences {
                println!(
                    "      {} = {} → {}",
                    local(&correspondence.relation),
                    correspondence.source.iri,
                    correspondence.target.iri
                );
            }
            manager
                .logger()
                .emit(
                    LogRecord::new(
                        Level::Info,
                        codes::LIBRARY_CASE_RETRIEVE,
                        mm_library::TARGET,
                    )
                    .with_field("query_hash", mm_core::hash_fields(&[&probe.problem_ttl]))
                    .with_field("case_iri", hit.iri.clone())
                    .with_field("similarity", f64::from(hit.utility))
                    .with_field(
                        "correspondences",
                        hit.correspondences
                            .iter()
                            .map(|c| c.relation.clone())
                            .collect::<Vec<_>>(),
                    ),
                )
                .await?;
        }
        if hits.is_empty() {
            println!("  no precedent shares a relation with the problem");
        }

        if !probe.expected.is_empty() {
            match hits.first() {
                Some(top_hit) if probe.expected.contains(&top_hit.iri) => {}
                Some(top_hit) => {
                    failures += 1;
                    println!(
                        "  UNEXPECTED: {} is not one of {}",
                        top_hit.iri,
                        probe.expected.join(", ")
                    );
                }
                None => {
                    failures += 1;
                    println!("  UNEXPECTED: nothing was retrieved");
                }
            }
        }
    }

    shutdown(kernel).await?;
    if failures > 0 {
        eprintln!("case retrieve: {failures} probe(s) did not match their expected precedent");
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Parse a retrieval fixture.
fn parse_probes(raw: &str) -> Result<Vec<Probe>, MmError> {
    let mut out = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|e| {
            MmError::Config(format!("{}: line {}: {e}", "problem fixture", index + 1))
        })?;
        let problem_ttl = value
            .get("problem_ttl")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MmError::Config(format!(
                    "problem fixture line {}: no problem_ttl",
                    index + 1
                ))
            })?
            .to_string();
        let expected = value
            .get("expected")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let top = value.get("top").and_then(Value::as_u64).map(|n| n as usize);
        let cases = value
            .get("cases")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .map(|(case_index, item)| {
                        parse_case(item).map_err(|e| {
                            MmError::Config(format!(
                                "problem fixture line {}, case {}: {e}",
                                index + 1,
                                case_index + 1
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        out.push(Probe {
            problem_ttl,
            expected,
            top,
            cases,
        });
    }
    if out.is_empty() {
        return Err(MmError::Config(
            "the problem fixture contains no probes".to_string(),
        ));
    }
    Ok(out)
}

/// One case declared by a fixture line.
fn parse_case(value: &Value) -> Result<Case, MmError> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let quality = |key: &str| value.get(key).and_then(Value::as_f64).unwrap_or(0.5) as f32;
    let problem = text("problem_ttl")
        .ok_or_else(|| MmError::Config("a case needs problem_ttl".to_string()))?;
    let kind = text("kind")
        .as_deref()
        .and_then(CaseKind::parse)
        .unwrap_or(CaseKind::Success);
    let slug = text("iri")
        .as_deref()
        .and_then(|iri| iri.rsplit('/').next().map(str::to_string))
        .or_else(|| text("slug"))
        .ok_or_else(|| MmError::Config("a case needs an iri or a slug".to_string()))?;
    let mut case = Case::new(
        &slug,
        kind,
        &problem,
        text("solution_ttl")
            .unwrap_or_else(|| ".".to_string())
            .as_str(),
        text("outcome_ttl")
            .unwrap_or_else(|| ".".to_string())
            .as_str(),
        quality("outcome_quality"),
        quality("transferability"),
        quality("evidence_quality"),
    )?;
    if let Some(iri) = text("iri") {
        case.iri = NamedNode::new_unchecked(iri);
    }
    Ok(case)
}

/// The stored cases, and the ones that could not be read.
async fn stored_cases(sqlite: &SqliteStore) -> Result<(Vec<Case>, Vec<(String, String)>), MmError> {
    use mm_core::Tabular;

    let rows = Tabular::query_json(
        sqlite,
        // The `cases` index carries one row per case, not per version: the version
        // is a property of the entry's IRI, so it is read from there below.
        "SELECT iri, kind, problem_ttl, solution_ttl, outcome_ttl, outcome_quality, \
         transferability, evidence_quality FROM cases ORDER BY iri",
        Vec::<Param>::new(),
    )
    .await?;
    let mut cases = Vec::new();
    let mut skipped = Vec::new();
    for row in &rows {
        let text = |key: &str| {
            row.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let number = |key: &str| row.get(key).and_then(Value::as_f64).unwrap_or(0.5) as f32;
        let iri = text("iri");
        let kind = CaseKind::parse(&text("kind")).unwrap_or(CaseKind::Success);
        let problem = text("problem_ttl");
        let slug = iri.rsplit('/').next().unwrap_or("case").to_string();
        match Case::new(
            &slug,
            kind,
            &problem,
            &text("solution_ttl"),
            &text("outcome_ttl"),
            number("outcome_quality"),
            number("transferability"),
            number("evidence_quality"),
        ) {
            Ok(mut case) => {
                case.version = iri
                    .rsplit_once('@')
                    .and_then(|(_, version)| version.parse::<u32>().ok())
                    .unwrap_or(1);
                case.iri = NamedNode::new_unchecked(iri);
                cases.push(case);
            }
            Err(e) => skipped.push((iri, e.to_string())),
        }
    }
    Ok((cases, skipped))
}

/// The local name of a relation IRI.
fn local(iri: &str) -> &str {
    iri.rsplit(['#', '/']).next().unwrap_or(iri)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE: &str = r#"{"problem_ttl": "@prefix mm: <https://metamind.dev/ontology#> . <https://metamind.dev/data/s> mm:regressedAfter <https://metamind.dev/data/d> .", "expected": ["https://metamind.dev/library/case/a"], "cases": [{"iri": "https://metamind.dev/library/case/a", "kind": "success", "problem_ttl": "<https://metamind.dev/data/service> <https://metamind.dev/ontology#regressedAfter> <https://metamind.dev/data/release> ."}]}"#;

    #[test]
    fn a_probe_carries_its_expected_precedents_and_its_corpus() {
        let probes = parse_probes(PROBE).unwrap();
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].expected.len(), 1);
        assert_eq!(probes[0].cases.len(), 1);
        assert_eq!(
            probes[0].cases[0].iri.as_str(),
            "https://metamind.dev/library/case/a"
        );
    }

    #[test]
    fn a_line_without_a_problem_is_refused_with_its_number() {
        let error = parse_probes("{\"expected\": []}").unwrap_err();
        assert!(error.to_string().contains("line 1"), "{error}");
    }

    #[test]
    fn a_blank_or_comment_only_fixture_is_refused() {
        assert!(parse_probes("\n# nothing\n").is_err());
    }
}
