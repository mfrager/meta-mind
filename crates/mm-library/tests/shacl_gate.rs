//! `shacl_gate.rs` — the shapes decide, and the committed seed passes them.
//!
//! The gate is the plan's invariant 1: no entry reaches `/library` without passing
//! its shapes. This test runs the real shapes file over the real seed corpus, so a
//! shape that the seed does not satisfy fails *here* rather than in a gate run, and
//! a broken document proves the shapes are load-bearing rather than decorative.

use std::path::PathBuf;

use mm_core::Config;

fn repo(relative: &str) -> PathBuf {
    Config::repo_root().join(relative)
}

fn shapes() -> String {
    std::fs::read_to_string(repo("ontology/shapes/library.shacl.ttl"))
        .expect("the library shapes file")
}

fn seed() -> String {
    std::fs::read_to_string(repo("ontology/seed/library_seed.ttl")).expect("the seed corpus")
}

fn violations(shapes: &str, data: &str) -> usize {
    mm_store_graph::validate_turtle_text(shapes, data, "library")
        .expect("the shapes and data must both parse")
        .violations
        .len()
}

#[test]
fn the_committed_seed_conforms_to_the_shapes() {
    let report = mm_store_graph::validate_turtle_text(&shapes(), &seed(), "library")
        .expect("the seed and the shapes must both parse");
    assert!(
        report.conforms,
        "the seed corpus must conform; violations: {:?}",
        report.violations
    );
    assert_eq!(report.violations.len(), 0);
}

#[test]
fn a_technique_missing_its_conditions_is_a_violation() {
    let broken = r#"
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix mm: <https://metamind.dev/ontology#> .
<https://metamind.dev/library/technique/broken> a mm:LibraryEntry, mm:Technique ;
    mm:title "Broken" ;
    mm:purpose "Says nothing about when it applies." ;
    mm:version "1"^^xsd:integer ;
    mm:evidence <https://metamind.dev/library/case/outage-rollback> ;
    mm:example <https://metamind.dev/library/case/outage-rollback> ;
    mm:confidence "0.5"^^xsd:double .
"#;
    let found = violations(&shapes(), broken);
    assert!(
        found >= 1,
        "a technique without applicableWhen must violate"
    );
}

#[test]
fn an_anti_pattern_without_a_corrective_rule_is_a_violation() {
    let broken = r#"
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix mm: <https://metamind.dev/ontology#> .
<https://metamind.dev/library/antipattern/broken> a mm:LibraryEntry, mm:AntiPattern ;
    mm:title "Broken" ;
    mm:purpose "Complains without saying what to do." ;
    mm:version "1"^^xsd:integer ;
    mm:evidence <https://metamind.dev/library/case/outage-rollback> .
"#;
    assert!(
        violations(&shapes(), broken) >= 1,
        "an anti-pattern without a corrective rule must violate"
    );
}

#[test]
fn an_entry_without_a_purpose_is_a_violation() {
    let broken = r#"
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix mm: <https://metamind.dev/ontology#> .
<https://metamind.dev/library/principle/broken> a mm:LibraryEntry, mm:Principle ;
    mm:title "Broken" ;
    mm:version "1"^^xsd:integer ;
    mm:evidence <https://metamind.dev/library/case/outage-rollback> .
"#;
    assert!(
        violations(&shapes(), broken) >= 1,
        "an entry without a purpose must violate"
    );
}

/// Every entry is typed both `mm:LibraryEntry` and its own kind, and every kind the
/// gate lists is present.
#[test]
fn the_seed_has_one_entry_for_every_kind_the_gate_asks_for() {
    let seed = seed();
    let graph = rdf_codec::io::parse_turtle(&seed).expect("the seed parses");
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut entry_typed = 0usize;
    for triple in graph.iter() {
        if triple.predicate.as_str() != "http://www.w3.org/1999/02/22-rdf-syntax-ns#type" {
            continue;
        }
        // `Term`'s rendering wraps an IRI in angle brackets; the local name is what
        // matters here.
        let rendered = triple.object.to_string();
        let class = rendered.trim_start_matches('<').trim_end_matches('>');
        if class.ends_with("#LibraryEntry") {
            entry_typed += 1;
            continue;
        }
        let local = class.rsplit('#').next().unwrap_or_default().to_string();
        *counts.entry(local).or_default() += 1;
    }
    assert!(entry_typed > 0);
    assert_eq!(
        entry_typed,
        counts.values().sum::<usize>(),
        "every typed entry is an mm:LibraryEntry"
    );
    for kind in [
        "Doctrine",
        "Principle",
        "Heuristic",
        "Technique",
        "Pattern",
        "Case",
        "AntiPattern",
        "Skill",
        "Policy",
        "Frame",
        "Evaluation",
        "Insight",
        "Workflow",
    ] {
        assert!(
            counts.get(kind).copied().unwrap_or(0) >= 1,
            "the seed has no {kind} entry; counts: {counts:?}"
        );
    }
    // The gate composes four named frames, so all four must be in the seed.
    for frame in ["problem_solving", "software", "debugging", "high_stakes"] {
        assert!(
            seed.contains(&format!("/library/frame/{frame}>")),
            "the seed is missing frame {frame}"
        );
    }
    // And the skill the gate verifies.
    assert!(seed.contains("/library/skill/api-capability-check@1>"));
    // And the policy the gate evolves.
    assert!(seed.contains("/policy/prefer_simpler_solution@1>"));
}
