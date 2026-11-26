//! `entry_roundtrip.rs` — the codec is a codec, not a one-way writer.
//!
//! What this file proves: an entry survives encode → decode → encode with the
//! **same content hash**, and the decoded value carries every field the encoder
//! wrote. A writer that silently dropped `domain_fitness`, or a reader that lost
//! the contraindications, would still produce valid Turtle and a stable hash — so
//! the hash alone is not the assertion; the fields are.
//!
//! Two properties of the shapes are stated here rather than assumed:
//!
//! * the committed shapes are **open**: an extra `mm:` property does not violate
//!   them, because "no unknown properties" is enforced where a document is
//!   *authored* (the extraction surface refuses an unknown key) rather than where
//!   it is validated. Asserting the opposite would be asserting something the
//!   validator does not do.
//! * a document missing a required property is refused, so the gate the round trip
//!   runs behind is load-bearing.

use mm_core::Config;
use mm_library::applicability::NEUTRAL_FITNESS;
use mm_library::entry::{iri, Declarative, EntryKind, LibraryEntry};
use mm_library::rdf::{content_hash, declarative_from_turtle, declaratives_from_turtle};
use mm_library::validate;
use mm_library::{
    anti_pattern, doctrine, evaluation, heuristic, pattern, principle, technique, when,
};

/// A case IRI, the backing every fixture entry names.
fn case(name: &str) -> mm_core::NamedNode {
    iri::library(EntryKind::Case, name)
}

/// Encode, decode, re-encode — and require the two documents to hash the same.
fn round_trip(entry: &Declarative) -> Declarative {
    let first = entry.to_turtle().expect("a valid entry encodes");
    let decoded = declarative_from_turtle(&first).expect("the document decodes");
    let second = decoded.to_turtle().expect("the decoded entry re-encodes");
    assert_eq!(
        content_hash(&first).expect("the first document hashes"),
        content_hash(&second).expect("the second document hashes"),
        "{}: a round trip must not change the content hash",
        entry.kind
    );
    decoded
}

/// A technique with every optional field populated, so the round trip has
/// something to lose.
fn rich_technique() -> Declarative {
    technique(
        "decompose-and-conquer",
        "Decompose and conquer",
        "Split a problem into independent parts.",
    )
    .expect("the constructor accepts a legal slug")
    .with_domain("software")
    .with_domain("debugging")
    .when(when("the parts can be solved independently").unwrap())
    .unless(when("the parts interact").unwrap())
    .contraindicated_when(when("high stakes and no rollback").unwrap())
    .backed_by(case("outage-rollback"))
    .illustrated_by(case("schema-migration-regression"))
    .countered_by(case("api-latency-misattribution"))
    .tested_by(iri::library(EntryKind::Evaluation, "regression-suite"))
    .with_domain_fitness("software", 0.75)
    .expect("0.75 is in range")
    .with_confidence(0.62)
    .expect("0.62 is in range")
    .with_activation(0.58)
    .expect("0.58 is in range")
    .derived_from("trajectory-0001")
}

#[test]
fn a_technique_round_trips_with_every_field_the_encoder_wrote() {
    let entry = rich_technique();
    let decoded = round_trip(&entry);

    assert_eq!(decoded.kind, EntryKind::Technique);
    assert_eq!(decoded.version, 1);
    assert_eq!(
        decoded.iri.as_str(),
        "https://metamind.dev/library/technique/decompose-and-conquer@1"
    );
    assert_eq!(decoded.title, "Decompose and conquer");
    assert_eq!(decoded.purpose, "Split a problem into independent parts.");
    // Compared as a set: a graph has no statement order, so the decoder is free to
    // return the domains in any order and only their membership is a promise.
    let mut domains = decoded.domain.clone();
    domains.sort();
    assert_eq!(
        domains,
        vec!["debugging".to_string(), "software".to_string()],
        "both domains survive"
    );
    assert_eq!(
        decoded.applicable_when.len(),
        1,
        "the `when` condition survives"
    );
    assert_eq!(decoded.inapplicable_when.len(), 1);
    assert_eq!(decoded.contraindications.len(), 1);
    assert_eq!(decoded.evidence.len(), 1);
    assert_eq!(
        decoded.evidence[0].as_str(),
        case("outage-rollback").as_str(),
        "a named node keeps its IRI, not its bracket rendering"
    );
    assert_eq!(decoded.examples.len(), 1);
    assert_eq!(decoded.counterexamples.len(), 1);
    assert_eq!(decoded.tested_by.len(), 1);
    assert_eq!(
        decoded.domain_fitness.get("software").copied(),
        Some(0.75),
        "domain fitness is written and read back"
    );
    assert_eq!(decoded.confidence, Some(0.62));
    assert!(
        (decoded.activation_score - 0.58).abs() < 1e-6,
        "activation survives: {}",
        decoded.activation_score
    );
    assert_eq!(
        decoded.derived_from_trajectory,
        vec!["trajectory-0001".to_string()]
    );
}

#[test]
fn every_declarative_kind_round_trips() {
    let entries = vec![
        doctrine(
            "evidence-before-action",
            "Evidence before action",
            "Act on evidence.",
        )
        .unwrap()
        .when(when("a decision depends on an unverified claim").unwrap())
        .backed_by(case("outage-rollback")),
        principle(
            "charitable-interpretation",
            "Charitable interpretation",
            "Read the strongest version.",
        )
        .unwrap()
        .backed_by(case("api-latency-misattribution")),
        heuristic(
            "simplest-explanation",
            "Simplest explanation",
            "Prefer the explanation with fewest parts.",
        )
        .unwrap()
        .when(when("several explanations fit").unwrap())
        .backed_by(case("outage-rollback")),
        pattern(
            "reproduce-then-fix",
            "Reproduce then fix",
            "Reproduce before changing anything.",
        )
        .unwrap()
        .when(when("a defect is reported but not reproduced").unwrap())
        .backed_by(case("schema-migration-regression")),
        anti_pattern(
            "premature-optimization",
            "Premature optimization",
            "Tuning what nobody measured.",
            "measure before you tune",
        )
        .unwrap()
        .backed_by(case("api-latency-misattribution")),
        evaluation(
            "regression-suite",
            "Regression suite",
            "Judge a change by what it breaks.",
        )
        .unwrap()
        .backed_by(case("schema-migration-regression")),
        rich_technique(),
    ];

    let kinds: Vec<EntryKind> = entries.iter().map(|entry| entry.kind).collect();
    assert_eq!(kinds.len(), 7, "every declarative kind is covered");
    for entry in &entries {
        let decoded = round_trip(entry);
        assert_eq!(decoded.kind, entry.kind);
        assert_eq!(decoded.title, entry.title);
        assert_eq!(decoded.purpose, entry.purpose);
        assert_eq!(
            decoded.evidence.len(),
            entry.evidence.len(),
            "{} keeps its evidence",
            entry.kind
        );
    }

    // The corrective rule is the anti-pattern's reason to exist, so it gets its own
    // assertion rather than riding on the evidence count.
    let anti = round_trip(&entries[4]);
    assert_eq!(
        anti.corrective_rule.as_deref(),
        Some("measure before you tune")
    );
}

#[test]
fn the_committed_seed_decodes_into_entries_with_titles() {
    let seed = std::fs::read_to_string(Config::repo_root().join("ontology/seed/library_seed.ttl"))
        .expect("the committed seed corpus");

    let entries = declaratives_from_turtle(&seed).expect("the seed decodes");
    assert!(
        entries.len() >= 7,
        "the seed holds one entry per declarative kind, got {}",
        entries.len()
    );
    for entry in &entries {
        assert!(
            !entry.title.trim().is_empty(),
            "{} decoded without a title",
            entry.iri.as_str()
        );
        assert!(
            entry.iri.as_str().contains('@'),
            "a decoded entry names its version: {}",
            entry.iri.as_str()
        );
        assert!(
            entry.validate().is_ok(),
            "{} must still satisfy its own kind's rules",
            entry.iri.as_str()
        );
    }

    // And the seed's technique decodes with the fitness the ranker reads.
    let technique = entries
        .iter()
        .find(|entry| entry.kind == EntryKind::Technique)
        .expect("the seed has a technique");
    assert!(
        !technique.domain_fitness.is_empty(),
        "the seed technique carries domain fitness"
    );
}

#[test]
fn an_unknown_property_is_not_a_shape_violation_but_a_missing_one_is() {
    // The shapes are open by design: closing them would mean every later phase's
    // property violates the library's shapes. The rule "an unknown property is a
    // hard error" therefore lives on the authoring path (the extraction surface),
    // not here — and this test states that rather than assuming the opposite.
    let open = r#"
@prefix mm: <https://metamind.dev/ontology#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<https://metamind.dev/library/principle/open> a mm:LibraryEntry, mm:Principle ;
    mm:title "Open" ;
    mm:purpose "Carries a property the shapes do not know." ;
    mm:version "1"^^xsd:integer ;
    mm:evidence <https://metamind.dev/library/case/outage-rollback> ;
    mm:somethingNew "written by a later phase" .
"#;
    validate::validate_turtle_with_shape_bytes(open, "open")
        .expect("the committed shapes are open, so an extra property conforms");

    // The same document without its evidence is refused: the gate is real.
    let broken = r#"
@prefix mm: <https://metamind.dev/ontology#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<https://metamind.dev/library/principle/broken> a mm:LibraryEntry, mm:Principle ;
    mm:title "Broken" ;
    mm:purpose "Says nothing about what backs it." ;
    mm:version "1"^^xsd:integer .
"#;
    let error = validate::validate_turtle_with_shape_bytes(broken, "broken")
        .expect_err("a principle without evidence must be refused");
    assert_eq!(error.code(), "shape");
    assert!(
        error.to_string().contains("PrincipleShape"),
        "the refusal names the shape: {error}"
    );
}

#[test]
fn a_document_that_is_not_an_entry_decodes_to_nothing_rather_than_a_stub() {
    let document = r#"
@prefix mm: <https://metamind.dev/ontology#> .
<https://metamind.dev/library/case/x> a mm:Case ;
    mm:title "A case is not a declarative entry" .
"#;
    let entries = declaratives_from_turtle(document).expect("the document parses");
    assert!(
        entries.is_empty(),
        "a non-declarative kind must not be decoded into a declarative stub"
    );
    assert!(
        declarative_from_turtle(document).is_err(),
        "and asking for one entry must be a refusal"
    );
}

#[test]
fn the_neutral_fitness_constant_is_the_documented_value() {
    // The round trip is about fields; this pins the one constant the decoded entry
    // is compared against elsewhere, so a change to it cannot slip through.
    assert!((NEUTRAL_FITNESS - 0.5).abs() < 1e-6);
}
