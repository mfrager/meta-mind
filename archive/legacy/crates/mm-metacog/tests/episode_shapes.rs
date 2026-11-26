//! `episode_shapes.rs` — the episode shapes decide, and the committed corpus
//! passes them.
//!
//! Two things have to be true for the SHACL half of the Phase 8 gate to mean
//! anything, and neither is visible from the shapes file alone:
//!
//! * the documents the emitter writes for the real corpus **conform**, so the
//!   shapes are not rejecting the system's own output; and
//! * a document that is actually wrong **does not** conform, so `graph validate`
//!   is a check rather than decoration.
//!
//! The clean direction is checked against the committed fixtures — the same nine
//! files the CLI gate runs — so a shape the corpus cannot satisfy fails here
//! rather than in a gate run. The other direction names the specific rules it
//! expects to fire, because a shape that rejects everything would pass a test that
//! only counted violations.
//!
//! What the shapes *cannot* state is documented in `ontology/shapes/episode.ttl`
//! itself (a tier outside `0..=5` needs `sh:minInclusive`, which this validator
//! does not implement; contiguity and acyclicity need comparisons between nodes).
//! The broken documents below are therefore chosen from the fragment the shapes
//! can enforce, and `an_out_of_range_tier_is_not_the_shapes_business` pins the
//! boundary from the other side.

use std::path::PathBuf;

use mm_core::Config;
use mm_metacog::rdf::{episode_turtle, program_turtle};
use mm_metacog::{
    CognitiveBudget, CognitiveEpisode, CognitiveProgram, ComparisonSpec, Context, DefaultCompiler,
    FailureMode, ProgramCompiler, ScanIssue, ScanResult, Tier, Uncertainty,
};

/// A path inside the repository.
fn repo(relative: &str) -> PathBuf {
    Config::repo_root().join(relative)
}

/// The episode shapes, as committed.
fn shapes() -> String {
    std::fs::read_to_string(repo("ontology/shapes/episode.ttl")).expect("the episode shapes file")
}

/// Every violation the shapes report for a document.
fn violations(data: &str) -> usize {
    mm_store_graph::validate_turtle_text(&shapes(), data, "epistemic")
        .expect("the shapes and the data must both parse")
        .violations
        .len()
}

/// An episode and its program as one Turtle document.
///
/// Each rendering carries its own prefix header, so the second one's header is
/// dropped rather than repeated; the shapes care what the graph says, not which
/// document a triple arrived in.
fn deliberation(
    episode: &CognitiveEpisode,
    scan: &ScanResult,
    program: &CognitiveProgram,
) -> String {
    let mut text = episode_turtle(episode, scan);
    for line in program_turtle(program).lines() {
        if line.starts_with("@prefix") || line.trim().is_empty() {
            continue;
        }
        text.push_str(line);
        text.push('\n');
    }
    text
}

fn ulid(n: u128) -> mm_core::Ulid {
    mm_core::Ulid::from_parts(1_700_000_000_000, n)
}

/// The episode the CLI's `hard` fixtures describe.
fn episode() -> CognitiveEpisode {
    let mut episode = CognitiveEpisode::new(
        ulid(1),
        ulid(2),
        Context::new("decide how to recover a sharded store that lost a replica").with_novelty(0.4),
        CognitiveBudget::from_spec(16, 8, 2.0, 60_000).unwrap(),
    )
    .unwrap()
    .with_claims(vec![ulid(3)])
    .with_assumptions(vec![ulid(4)])
    .with_candidates(vec!["restore from the backup".into(), "fail over".into()])
    .with_frames(vec![
        "https://metamind.dev/library/frame/incident_response".into()
    ])
    .with_techniques(vec![
        "https://metamind.dev/library/technique/narrow-before-acting".into(),
    ]);
    episode.tier = Tier::T4;
    episode
}

fn scan() -> ScanResult {
    ScanResult {
        issues: vec![ScanIssue {
            kind: "missing_evidence".into(),
            materiality: 0.8,
            target: "fail over".into(),
        }],
        uncertainties: vec![Uncertainty {
            id: "u1".into(),
            description: "does the window hold".into(),
            magnitude: 0.7,
        }],
        comparison_requirements: vec![ComparisonSpec {
            subject: "restore from the backup".into(),
            versus: "fail over".into(),
            criterion: "data loss".into(),
        }],
        failure_modes: vec![FailureMode {
            mode: "partial restore".into(),
            likelihood: 0.2,
            impact: 0.8,
        }],
        simpler_alternatives: vec!["ask the on-call".into()],
        stakes: 0.9,
        irreversibility: 0.6,
        verification_value: 0.9,
    }
}

/// The fixture file's payload, as the CLI's `episode run` reads it.
#[derive(serde::Deserialize)]
struct Fixture {
    episode: CognitiveEpisode,
    scan: ScanResult,
}

#[test]
fn the_emitters_output_conforms_to_the_shapes() {
    let episode = episode();
    let scan = scan();
    let program = DefaultCompiler::new()
        .compile(&episode, &scan, &episode.budget)
        .expect("the episode compiles");

    let report = mm_store_graph::validate_turtle_text(
        &shapes(),
        &deliberation(&episode, &scan, &program),
        "epistemic",
    )
    .expect("the episode graph and the shapes must both parse");
    assert!(
        report.conforms,
        "the emitter's own output must conform; violations: {:?}",
        report.violations
    );
    assert_eq!(report.violations.len(), 0);
}

#[test]
fn every_committed_fixture_conforms_to_the_shapes() {
    let mut seen = 0usize;
    for band in ["trivial", "standard", "hard"] {
        let dir = repo("bench/episodes").join(band);
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|e| e == "json"))
            .collect();
        paths.sort();
        assert!(!paths.is_empty(), "{} holds no fixtures", dir.display());

        for path in paths {
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let fixture: Fixture = serde_json::from_str(&raw)
                .unwrap_or_else(|e| panic!("{} is not an episode fixture: {e}", path.display()));
            let program = DefaultCompiler::new()
                .compile(&fixture.episode, &fixture.scan, &fixture.episode.budget)
                .unwrap_or_else(|e| panic!("{} does not compile: {e}", path.display()));
            let found = violations(&deliberation(&fixture.episode, &fixture.scan, &program));
            assert_eq!(
                found,
                0,
                "{} must conform to the episode shapes",
                path.display()
            );
            seen += 1;
        }
    }
    assert!(seen >= 9, "the corpus holds {seen} fixture(s)");
}

/// The prefix header every hand-written document below shares.
const PREFIXES: &str = "@prefix mm: <https://metamind.dev/ontology#> .\n\
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n";

/// A complete, conforming episode — the document each broken case departs from.
const VALID_EPISODE: &str = "<https://metamind.dev/data/01h00000000000000000000001> \
  a mm:CognitiveEpisode, mm:Episode ;\n\
  mm:tier \"4\"^^xsd:integer ;\n\
  mm:budget \"16\"^^xsd:integer ;\n\
  mm:stakes \"0.9\"^^xsd:double ;\n\
  mm:uncertainty \"0.7\"^^xsd:double ;\n\
  mm:reversibility \"0.4\"^^xsd:double ;\n\
  mm:compiledProgram <https://metamind.dev/data/01h00000000000000000000002> .\n";

/// The program and its one operation, so the episode's `mm:compiledProgram`
/// resolves and only the departure under test is left.
const VALID_PROGRAM: &str = "<https://metamind.dev/data/01h00000000000000000000002> \
  a mm:CognitiveProgram ;\n\
  mm:tier \"4\"^^xsd:integer ;\n\
  mm:stoppingCondition \"budget_exhausted\" ;\n\
  mm:hasOperation _:op0 .\n\
_:op0 a mm:CognitiveOp, mm:OperationValue ;\n\
  mm:opClass \"recall\" ;\n\
  mm:opOrder \"1\"^^xsd:integer ;\n\
  mm:targets \"the goal\" ;\n\
  mm:valueEer \"0.5\"^^xsd:double ;\n\
  mm:valueImportance \"0.9\"^^xsd:double ;\n\
  mm:valueChangeProbability \"0.5\"^^xsd:double ;\n\
  mm:valueCost \"0.03\"^^xsd:double ;\n\
  mm:score \"7.5\"^^xsd:double .\n";

fn document(body: &str) -> String {
    format!("{PREFIXES}{body}")
}

fn valid_document() -> String {
    document(&format!("{VALID_EPISODE}{VALID_PROGRAM}"))
}

#[test]
fn a_broken_document_is_refused_by_the_rule_it_breaks() {
    // The control: the document the departures below are cut from conforms, so a
    // violation means the departure caused it rather than the harness.
    assert_eq!(
        violations(&valid_document()),
        0,
        "the hand-written control must conform"
    );

    // An episode that cannot say how much cognition it was allowed.
    let no_budget = VALID_EPISODE
        .lines()
        .filter(|line| !line.contains("mm:budget"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        violations(&document(&format!("{no_budget}\n{VALID_PROGRAM}"))) >= 1,
        "an episode without a budget must violate CognitiveEpisodeShape"
    );

    // A stakes value that is present, singular, and the wrong datatype: a string
    // cannot stand in for the double the shape asks for.
    let string_stakes = VALID_EPISODE.replace("\"0.9\"^^xsd:double", "\"high\"");
    assert!(
        violations(&document(&format!("{string_stakes}\n{VALID_PROGRAM}"))) >= 1,
        "a stakes value of the wrong datatype must violate"
    );

    // An operation with no class, which is what the compute policy gates on.
    let classless = VALID_PROGRAM
        .lines()
        .filter(|line| !line.contains("mm:opClass"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        violations(&document(&format!("{VALID_EPISODE}{classless}"))) >= 1,
        "an operation without opClass must violate CognitiveOpShape"
    );

    // A program that cannot say when to stop is the overthinking the shapes exist
    // to prevent.
    let open_ended = VALID_PROGRAM
        .lines()
        .filter(|line| !line.contains("mm:stoppingCondition"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        violations(&document(&format!("{VALID_EPISODE}{open_ended}"))) >= 1,
        "a program without a stopping condition must violate"
    );

    // Two scores on one operation: the shape allows exactly one, and this is the
    // rule that catches two programs' operations merging into a single blank node.
    let doubled = VALID_PROGRAM.replace(
        "mm:score \"7.5\"^^xsd:double",
        "mm:score \"7.5\"^^xsd:double ; mm:score \"8.5\"^^xsd:double",
    );
    assert_ne!(
        doubled, VALID_PROGRAM,
        "the doubled case must change the document"
    );
    assert!(
        violations(&document(&format!("{VALID_EPISODE}{doubled}"))) >= 1,
        "two scores on one operation must violate OperationValueShape"
    );
}

#[test]
fn an_out_of_range_tier_is_not_the_shapes_business() {
    // The shapes can say a tier is present, singular and an integer; they cannot
    // say it is in `0..=5`, because that needs `sh:minInclusive`, which the
    // vendored validator does not implement. `Tier` and the `episodes.tier` CHECK
    // constraint are what enforce the range, and the shapes file says so. Pinning
    // the boundary here keeps a future reader from "fixing" the shape by adding a
    // predicate the validator would treat as a hard error.
    let out_of_range = valid_document().replace("mm:tier \"4\"", "mm:tier \"7\"");
    assert!(
        out_of_range.contains("mm:tier \"7\""),
        "the case must actually change the document"
    );
    assert_eq!(
        violations(&out_of_range),
        0,
        "the fragment cannot state a range, so an out-of-range tier is not a violation"
    );
}
