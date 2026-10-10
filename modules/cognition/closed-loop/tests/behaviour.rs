//! The `closed-loop` module's behaviour, driven through its public API.
//!
//! The unit tests in `src/lib.rs` check the arithmetic; this test checks the contract a
//! *caller* depends on, which is the part a later phase reads: the ten-stage order is the
//! one the controller walks, a run that ran every stage and refused one is not complete,
//! and a malformed record is refused rather than repaired.

use closed_loop::{
    expected_stage, order_matches, summarize, StageStatus, COGNITION_LOOP_STATUS, STAGES,
};

fn run_with(overrides: &[(u32, &str)]) -> Vec<StageStatus> {
    (0..STAGES.len() as u32)
        .map(|idx| {
            let outcome = overrides
                .iter()
                .find(|(at, _)| *at == idx)
                .map(|(_, outcome)| *outcome)
                .unwrap_or("ok");
            StageStatus {
                idx,
                stage: expected_stage(idx as usize)
                    .expect("every index below ten names a stage")
                    .to_string(),
                outcome: outcome.to_string(),
                artifact: format!("artifact-{idx}"),
            }
        })
        .collect()
}

#[test]
fn the_canonical_order_is_the_ten_stages_the_controller_walks() {
    let run = run_with(&[]);
    assert!(order_matches(&run));
    assert_eq!(STAGES[0], "experience");
    assert_eq!(STAGES[4], "change_set");
    assert_eq!(STAGES[9], "new_version");
}

#[test]
fn a_complete_run_summarizes_as_complete_and_names_no_refusal() {
    let summary = summarize(&run_with(&[])).expect("a summary");
    assert!(summary.complete);
    assert_eq!(summary.ok, 10);
    assert_eq!(summary.first_refusal, None);
}

#[test]
fn every_stage_can_run_and_the_run_still_not_be_complete() {
    // The property the summary exists for: ten stages present, one of them refused, and
    // "a stage was refused" is not the same claim as "the loop failed".
    let summary = summarize(&run_with(&[(5, "refused")])).expect("a summary");
    assert_eq!(summary.stages, 10);
    assert_eq!(summary.refused, 1);
    assert!(!summary.complete);
    assert_eq!(
        summary.first_refusal.as_deref(),
        Some("self_engineering: refused")
    );
}

#[test]
fn a_malformed_record_is_refused_rather_than_repaired() {
    let mut run = run_with(&[]);
    run[6].outcome = "abandoned".to_string();
    let error = summarize(&run).expect_err("a refusal");
    assert_eq!(error.code(), "validation");
    assert_eq!(error.function(), COGNITION_LOOP_STATUS);
    assert!(error.to_string().contains("abandoned"), "{error}");

    // A record whose name does not match its index is not silently reordered: the
    // summary is refused.
    let mut misnamed = run_with(&[]);
    misnamed[2].stage = "diagnosis".to_string();
    assert!(summarize(&misnamed).is_err());
    assert!(!order_matches(&misnamed));
}
