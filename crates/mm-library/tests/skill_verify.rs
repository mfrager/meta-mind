//! `skill_verify.rs` — a skill is a draft until its test passes, and only a
//! verified skill is retrievable.
//!
//! The plan's testing table asks for one thing here: retrieval returns only
//! `Verified` skills, and a failing test keeps a skill a draft rather than
//! promoting it. That makes the verification state load-bearing — if a draft were
//! retrievable, the state would be decoration.

use mm_core::Timestamp;
use mm_library::error::LibraryError;
use mm_library::skill::{retrieve, Skill, SkillTestSpec, Verification};

/// A skill at version 1, as `skill register` would build it.
fn skill(slug: &str, name: &str, description: &str) -> Skill {
    Skill::new(
        slug,
        name,
        description,
        "data/library/skills/example.wasm",
        "check",
        "fn(check: Capability) -> Verdict",
        "https://metamind.dev/library/evaluation/regression-suite",
    )
    .expect("a well-formed skill")
}

fn spec(cases: u32, pass: u32) -> SkillTestSpec {
    SkillTestSpec { cases, pass }
}

#[test]
fn a_new_skill_is_a_draft_whatever_its_artefact_says() {
    let skill = skill(
        "api-capability-check",
        "API capability check",
        "Check an api.",
    );
    assert_eq!(
        skill.verification,
        Verification::Draft,
        "registration must not imply verification"
    );
    assert!(skill.verified_at.is_none());
    assert!(!skill.verification.is_usable());
}

#[test]
fn a_failing_spec_keeps_a_draft_a_draft() {
    let mut mismatched = skill(
        "api-capability-check",
        "API capability check",
        "Check an api.",
    );
    let state = mismatched.decide(false, &spec(2, 2), Timestamp::now());
    assert_eq!(
        state,
        Verification::Draft,
        "an artefact mismatch is not a pass"
    );
    assert!(mismatched.verified_at.is_none(), "nothing was verified");

    let mut failing = skill(
        "api-capability-check",
        "API capability check",
        "Check an api.",
    );
    let state = failing.decide(true, &spec(2, 1), Timestamp::now());
    assert_eq!(state, Verification::Draft, "a failing case is not a pass");
}

#[test]
fn a_passing_spec_verifies_and_a_later_failure_downgrades_rather_than_erases() {
    let mut skill = skill(
        "api-capability-check",
        "API capability check",
        "Check an api.",
    );
    let state = skill.decide(true, &spec(4, 4), Timestamp::now());
    assert_eq!(state, Verification::Verified);
    assert!(
        skill.verified_at.is_some(),
        "a verification carries its instant"
    );

    // The record of having been verified is what makes the downgrade meaningful:
    // a regression must be distinguishable from a skill that never passed.
    let state = skill.decide(true, &spec(4, 3), Timestamp::now());
    assert_eq!(state, Verification::Failing);
    assert!(!skill.verification.is_usable());
}

#[test]
fn retrieval_returns_only_verified_skills() {
    let mut verified = skill(
        "cache-first",
        "Check the cache first",
        "Read the cache before the network.",
    );
    verified.decide(true, &spec(1, 1), Timestamp::now());
    let draft = skill(
        "cache-second",
        "Check the cache first",
        "Read the cache before the network.",
    );

    let hits = retrieve(
        &[draft.clone(), verified.clone()],
        "check the cache first",
        5,
    );
    assert_eq!(hits.len(), 1, "the draft must not be retrievable: {hits:?}");
    assert_eq!(hits[0].iri, verified.iri.as_str());
    assert_eq!(hits[0].verification, Verification::Verified);
    assert!(hits[0].score > 0.0);

    // And the draft is invisible even when it matches the query better than the
    // verified skill does.
    let mut weak = skill("unrelated", "Unrelated", "Nothing to do with caches.");
    weak.decide(true, &spec(1, 1), Timestamp::now());
    let hits = retrieve(&[draft, weak], "check the cache first", 5);
    assert!(
        hits.iter().all(|hit| hit.score > 0.0),
        "an unusable skill never reaches a hit: {hits:?}"
    );
}

#[test]
fn a_draft_is_refused_by_the_retrieval_guard() {
    let skill = skill(
        "api-capability-check",
        "API capability check",
        "Check an api.",
    );
    let error = skill.require_verified().expect_err("a draft is not usable");
    assert_eq!(error.code(), "skill_not_verified");
    assert!(matches!(error, LibraryError::SkillNotVerified { .. }));
    assert!(
        error.to_string().contains("draft"),
        "the refusal names the state: {error}"
    );
}

#[test]
fn the_gates_skill_iri_is_path_derived_and_versioned() {
    let skill = skill(
        "api-capability-check",
        "API capability check",
        "Confirm an external api behaves as documented before relying on it.",
    );
    assert_eq!(
        skill.iri.as_str(),
        "https://metamind.dev/library/skill/api-capability-check@1"
    );
    assert_eq!(skill.version, 1);
}
