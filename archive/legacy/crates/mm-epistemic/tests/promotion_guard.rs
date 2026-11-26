//! `promotion_guard.rs` — every forbidden transition is refused, and only the two
//! world statuses are admissible.

mod common;

use common::Harness;
use mm_epistemic::{
    can_promote, observation_requirement, EpistemicStatus, Evidence, EvidenceKind,
    ValidationBarrier, EPISTEMIC_STATUSES, FORBIDDEN, PHASE_FOUR_INVARIANT,
};

fn every_kind() -> Vec<Evidence> {
    [
        EvidenceKind::Document,
        EvidenceKind::ExternalTool,
        EvidenceKind::Observation,
        EvidenceKind::Testimony,
        EvidenceKind::Trace,
        EvidenceKind::ModelOutput,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, kind)| Harness::evidence(index as u128 + 1, kind))
    .collect()
}

#[test]
fn every_forbidden_transition_is_refused_whatever_the_evidence() {
    let full = every_kind();
    for (from, to, reason) in FORBIDDEN {
        let denied = can_promote(from, to, &full).unwrap_err();
        assert_eq!(denied.reason, reason, "{from} -> {to}");
        assert_eq!(can_promote(from, to, &[]).unwrap_err().reason, reason);
    }
}

#[test]
fn only_observed_and_verified_are_world_admissible() {
    let admissible: Vec<&str> = EPISTEMIC_STATUSES
        .into_iter()
        .filter(|status| status.is_world_admissible())
        .map(EpistemicStatus::as_str)
        .collect();
    assert_eq!(admissible, vec!["OBSERVED", "VERIFIED"]);
}

#[test]
fn the_barrier_admits_only_supported_world_claims() {
    let mut claim = Harness::claim(1, "green", EpistemicStatus::Observed);
    // Observed but unsupported: refused.
    assert!(ValidationBarrier::admit(&claim).is_err());
    claim.evidence = vec![Harness::id(2)];
    ValidationBarrier::admit(&claim).unwrap();
    // An assumption is never admissible, evidence or not.
    let mut assumed = Harness::claim(3, "green", EpistemicStatus::Assumed);
    assumed.evidence = vec![Harness::id(4)];
    assert!(ValidationBarrier::admit(&assumed).is_err());
}

#[test]
fn the_phase_four_invariant_is_one_predicate() {
    assert_eq!(PHASE_FOUR_INVARIANT, "no_assumption_to_observation");
    // The reason the guard gives is the reason the shared predicate gives: for a
    // pair the hard table does not already cover, layer 3 *is* the shared
    // predicate.
    let denied =
        can_promote(EpistemicStatus::Reported, EpistemicStatus::Observed, &[]).unwrap_err();
    let shared = observation_requirement(EpistemicStatus::Observed, false).unwrap();
    assert_eq!(denied.reason, shared);
    // And an observation record clears the requirement.
    let observation = vec![Harness::evidence(1, EvidenceKind::Observation)];
    assert!(can_promote(
        EpistemicStatus::Reported,
        EpistemicStatus::Observed,
        &observation
    )
    .is_ok());
    // An assumption is refused by the hard table whatever the record says.
    assert!(can_promote(
        EpistemicStatus::Assumed,
        EpistemicStatus::Observed,
        &observation
    )
    .is_err());
}

#[test]
fn an_unrelated_promotion_is_not_touched() {
    // The table is a table: an allowed rise stays allowed.
    assert!(can_promote(EpistemicStatus::Hypothetical, EpistemicStatus::Assumed, &[]).is_ok());
    // And a claim that cannot rise at all is refused.
    assert!(can_promote(
        EpistemicStatus::Fictional,
        EpistemicStatus::Reported,
        &every_kind()
    )
    .is_err());
}
