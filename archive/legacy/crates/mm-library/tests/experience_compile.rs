//! `experience_compile.rs` — repetition is support, never confidence.
//!
//! The plan asks for one assertion here: repeated insights produce candidate
//! entries whose confidence is **below the promotion threshold**. A compiler that
//! could cross that line on its own would be a way to launder a single run into a
//! general rule, which is why promotion belongs to a later phase.

use std::path::PathBuf;

use mm_core::Config;
use mm_library::experience::{
    compile, draft_confidence, Insight, Trajectory, MAX_DRAFT_CONFIDENCE, PROMOTION_THRESHOLD,
};

fn repo(relative: &str) -> PathBuf {
    Config::repo_root().join(relative)
}

fn fixture(relative: &str) -> Vec<Trajectory> {
    let raw = std::fs::read_to_string(repo(relative))
        .unwrap_or_else(|e| panic!("cannot read {relative}: {e}"));
    Trajectory::from_jsonl(&raw).expect("the committed fixture parses")
}

fn insight(iri: &str, text: &str) -> Insight {
    Insight {
        iri: iri.to_string(),
        text: text.to_string(),
        kind: "pattern".to_string(),
        evidence: vec![],
        upvotes: 3,
        downvotes: 0,
    }
}

#[test]
fn repeated_lessons_merge_into_one_candidate_with_rising_support() {
    let trajectories = fixture("bench/library/trajectories/success.jsonl");
    let insights = vec![
        insight(
            "https://metamind.dev/data/narrow",
            "narrowing the input before reading the implementation reveals the cause",
        ),
        insight(
            "https://metamind.dev/data/assumption",
            "naming an assumption makes the plan falsifiable",
        ),
    ];
    let compiled = compile(&trajectories, &insights);

    let slug = "narrow-the-input-before-reading-the-implementation";
    assert_eq!(
        compiled.support.get(slug).copied(),
        Some(2),
        "identical lessons are one candidate with support 2: {:?}",
        compiled.support
    );
    assert_eq!(
        compiled.techniques.len(),
        2,
        "one candidate per distinct lesson"
    );

    let repeated = compiled
        .techniques
        .iter()
        .find(|entry| entry.slug() == slug)
        .expect("the repeated lesson is emitted");
    let single = compiled
        .techniques
        .iter()
        .find(|entry| entry.slug() != slug)
        .expect("the unrepeated lesson is emitted");
    assert!(
        repeated.confidence.expect("a confidence") > single.confidence.expect("a confidence"),
        "support raises confidence: {:?} vs {:?}",
        repeated.confidence,
        single.confidence
    );
    assert_eq!(repeated.confidence, Some(draft_confidence(2)));
}

#[test]
fn repetition_never_reaches_the_promotion_threshold() {
    for support in 1..=50u32 {
        let confidence = draft_confidence(support);
        assert!(
            confidence < PROMOTION_THRESHOLD,
            "support {support} produced {confidence}, which is promotable"
        );
        assert!(confidence <= MAX_DRAFT_CONFIDENCE);
    }
    // And strictly increasing, so support is worth recording.
    assert!(draft_confidence(2) > draft_confidence(1));
}

#[test]
fn every_compiled_candidate_is_sub_threshold() {
    let trajectories = fixture("bench/library/trajectories/repeated.jsonl");
    let insights = vec![insight(
        "https://metamind.dev/data/narrow",
        "narrowing the input before reading the implementation reveals the cause",
    )];
    let compiled = compile(&trajectories, &insights);
    assert!(
        !compiled.techniques.is_empty(),
        "the fixture supports a candidate"
    );
    assert!(compiled.is_sub_threshold(), "{:?}", compiled.techniques);
    for entry in &compiled.techniques {
        assert!(
            entry.confidence.expect("a confidence") < PROMOTION_THRESHOLD,
            "{} is promotable",
            entry.slug()
        );
    }
}

#[test]
fn two_compiles_of_the_same_input_are_byte_identical() {
    let trajectories = fixture("bench/library/trajectories/success.jsonl");
    let insights = vec![insight(
        "https://metamind.dev/data/narrow",
        "narrowing the input before reading the implementation reveals the cause",
    )];
    let first = compile(&trajectories, &insights);
    let second = compile(&trajectories, &insights);
    assert_eq!(first, second);
    assert_eq!(
        first.to_jsonl().expect("jsonl"),
        second.to_jsonl().expect("jsonl"),
        "the gold file for `experience compile` must be stable"
    );
    assert!(first
        .to_jsonl()
        .expect("jsonl")
        .contains("\"kind\":\"Technique\""));
}

#[test]
fn a_lesson_with_no_supporting_evidence_is_counted_but_not_emitted() {
    let trajectories = vec![Trajectory {
        id: "01HZTRJ0000000000000000B1".to_string(),
        outcome: "success".to_string(),
        steps: vec!["rotate the credentials".to_string()],
        lesson: "Rotate the credentials quarterly".to_string(),
        domains: vec!["ops".to_string()],
        evidence: vec![],
    }];
    let compiled = compile(&trajectories, &[]);
    assert!(
        compiled.techniques.is_empty(),
        "nothing backs it, so nothing is emitted: {:?}",
        compiled.techniques
    );
    assert_eq!(compiled.support["rotate-the-credentials-quarterly"], 0);
    assert!(
        compiled.is_sub_threshold(),
        "an empty compile is trivially safe"
    );
}

#[test]
fn a_malformed_trajectory_line_is_refused_with_its_line_number() {
    let error = Trajectory::from_jsonl("{\"id\":\"01\"}\nnot json\n")
        .expect_err("a malformed line is a refusal, not a skip");
    assert!(
        error.to_string().contains("line 2"),
        "the refusal names the line: {error}"
    );
}
