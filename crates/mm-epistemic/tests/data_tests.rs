//! `data_tests.rs` — the data-quality suite is quiet on clean state and loud on a
//! seeded defect, and the shapes accept a clean instance and reject a broken one.

mod common;

use common::Harness;
use mm_epistemic::{data_tests, Assumption, EpistemicStatus, Proposition, RiskLevel, Severity};

const CLEAN: &str = include_str!("../../../tests/fixtures/epistemic/clean.ttl");
const BROKEN: &str = include_str!("../../../tests/fixtures/epistemic/broken.ttl");
const DEFECTS: &str = include_str!("../../../tests/fixtures/epistemic/data_defects.ttl");

#[test]
fn a_clean_state_has_no_findings_and_a_defect_does() {
    let clean = Harness::observed(1, "green");
    assert!(data_tests(&[clean], &[]).is_empty());

    // An observed claim with no evidence is a data-quality defect at the suite
    // layer, even though the barrier would have refused to write it.
    let defect = Harness::claim(2, "green", EpistemicStatus::Observed);
    let findings = data_tests(&[defect], &[]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, Severity::Violation);
    assert_eq!(findings[0].test_id, "world_requires_evidence");
}

#[test]
fn a_malformed_assumption_is_reported() {
    let mut assumption = Assumption::new(
        Harness::id(1),
        Proposition::literal("https://x/s", "https://x/p", "v").unwrap(),
        0.5,
        RiskLevel::Low,
        1.0,
        0.5,
    )
    .unwrap();
    assumption.confidence = 3.0;
    let findings = data_tests(&[], &[assumption]);
    assert!(findings
        .iter()
        .any(|f| f.test_id == "assumption_well_formed"));
}

#[tokio::test]
async fn the_shapes_accept_clean_and_reject_broken() {
    let harness = Harness::new().await;
    let shapes = Harness::shapes();

    harness
        .graph
        .replace_turtle("epistemic", CLEAN)
        .await
        .unwrap();
    let clean = harness
        .graph
        .validate_with("epistemic", &shapes)
        .await
        .unwrap();
    assert!(clean.conforms, "{:?}", clean.violations);

    harness
        .graph
        .replace_turtle("epistemic", BROKEN)
        .await
        .unwrap();
    let broken = harness
        .graph
        .validate_with("epistemic", &shapes)
        .await
        .unwrap();
    assert!(!broken.conforms, "the broken fixture must violate a shape");

    // A defect the shape fragment can express: two statuses on one claim.
    harness
        .graph
        .replace_turtle("epistemic", DEFECTS)
        .await
        .unwrap();
    let defects = harness
        .graph
        .validate_with("epistemic", &shapes)
        .await
        .unwrap();
    assert!(!defects.conforms);

    harness.shutdown().await;
}
