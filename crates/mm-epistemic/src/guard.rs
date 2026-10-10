//! The promotion guard: the deterministic transition table.
//!
//! `can_promote` is the kernel's answer to status inflation. It is a pure
//! function of `(from, to, evidence)` — it takes **no model output**, no clock,
//! and no store handle — so "the being promoted an assumption straight to an
//! observation" is not a bug that can be produced by a bad prompt. It is
//! unreachable code.
//!
//! Three layers decide, in order:
//!
//! 1. **The hard table.** A handful of pairs are refused whatever the evidence
//!    says, because the evidence cannot change what the pair *means*
//!    (`ASSUMED -> OBSERVED` is not weakly supported, it is category error).
//! 2. **Direction.** A promotion moves up [`EpistemicStatus::strength`]. Lowering
//!    a status is a retraction, which is a different, logged operation — the one
//!    exception is the plan's `REPORTED -> INFERRED`, where a report is
//!    re-interpreted once evidence arrives.
//! 3. **The evidence gate for the target.** Reaching `OBSERVED` needs an
//!    observation record, `VERIFIED` needs an authoritative one, and the middle
//!    statuses need at least something.
//!
//! The `mm-being` invariant "an observation requires an observation record"
//! (Phase 4's `no_assumption_to_observation`) is [`observation_requirement`],
//! which is the *same* code layer 3 calls. That is the wiring: the two guards
//! cannot drift because there is one predicate.

use crate::claim::Evidence;
use crate::error::PromotionDenied;
use crate::status::EpistemicStatus;

/// The Phase 4 invariant this table implements.
///
/// `mm-being` stores this name in `invariants.code` and prints it in a refusal, so
/// it is spelled once, here.
pub const PHASE_FOUR_INVARIANT: &str = "no_assumption_to_observation";

/// Pairs a promotion never crosses, whatever the evidence says.
///
/// The reason is a `&'static str` because it comes from this table: the guard has
/// nothing dynamic to interpolate, which is exactly why it can be trusted.
pub const FORBIDDEN: [(EpistemicStatus, EpistemicStatus, &str); 5] = [
    (
        EpistemicStatus::Assumed,
        EpistemicStatus::Observed,
        "an assumption is never an observation; an observation record is what makes one",
    ),
    (
        EpistemicStatus::Assumed,
        EpistemicStatus::Verified,
        "an assumption is never verified; verification needs authoritative evidence",
    ),
    (
        EpistemicStatus::Inferred,
        EpistemicStatus::Observed,
        "an inference is never an observation",
    ),
    (
        EpistemicStatus::Predicted,
        EpistemicStatus::Observed,
        "a prediction is never an observation; the world decides, later, with a record",
    ),
    (
        EpistemicStatus::Simulated,
        EpistemicStatus::Verified,
        "a simulation is never verification; a simulation proves the model, not the world",
    ),
];

/// The reason a promotion to `OBSERVED` is refused when no observation record
/// supports it.
///
/// Exposed because `mm-being`'s `CoreGuard` asks the same question with a being
/// op in hand; sharing the predicate is what the plan's build step 13 means by
/// wiring the Phase 4 invariant to `can_promote`.
pub fn observation_requirement(
    to: EpistemicStatus,
    has_observation_record: bool,
) -> Option<&'static str> {
    if to == EpistemicStatus::Observed && !has_observation_record {
        return Some("an observation requires an observation record");
    }
    None
}

/// True when a promotion from `from` to `to` is on the hard table.
pub fn is_forbidden(from: EpistemicStatus, to: EpistemicStatus) -> Option<&'static str> {
    FORBIDDEN
        .iter()
        .find(|(f, t, _)| *f == from && *t == to)
        .map(|(_, _, reason)| *reason)
}

/// The deterministic transition table.
///
/// Pure. Takes no model output, no clock, and no store.
pub fn can_promote(
    from: EpistemicStatus,
    to: EpistemicStatus,
    evidence: &[Evidence],
) -> std::result::Result<(), PromotionDenied> {
    let deny = |reason: &'static str| Err(PromotionDenied { from, to, reason });

    // Idempotent: re-asserting the status a claim already has is not a promotion,
    // and refusing it would make a replay of an accepted write fail.
    if from == to {
        return Ok(());
    }
    // Layer 1: the hard table, before anything evidence-dependent.
    if let Some(reason) = is_forbidden(from, to) {
        return deny(reason);
    }
    // A fiction or an unclassified proposition is not weakly held; it is not a
    // belief about the world. Nothing raises it in place.
    if !from.can_rise() {
        return deny("a fictional or unknown proposition is not raised in place; a new claim is how it changes");
    }
    // An observation is final. Rewriting it in place would erase the record of
    // what was seen, so a change is a new claim that supersedes it.
    if from == EpistemicStatus::Observed {
        return deny("an observation is never rewritten, only superseded by a new claim");
    }
    // Layer 2: direction. Suspension is a retraction, not a promotion.
    if to.strength() < from.strength()
        && !(from == EpistemicStatus::Reported && to == EpistemicStatus::Inferred)
    {
        return deny("a status is not lowered by promotion; suspension is a retraction");
    }
    // Layer 3: the evidence gate for the target.
    if let Some(reason) =
        observation_requirement(to, evidence.iter().any(|e| e.kind.is_observation()))
    {
        return deny(reason);
    }
    if to == EpistemicStatus::Verified && !evidence.iter().any(|e| e.kind.is_authoritative()) {
        return deny("verification needs authoritative evidence");
    }
    if matches!(to, EpistemicStatus::Reported | EpistemicStatus::Inferred) && evidence.is_empty() {
        return deny("this promotion needs at least one piece of evidence");
    }
    Ok(())
}

/// True when `to` is reachable from `from` with no evidence at all.
///
/// Used by the barrier to distinguish "this claim needs support to rise" from
/// "this claim may simply be created at this status".
pub fn rises_without_evidence(from: EpistemicStatus, to: EpistemicStatus) -> bool {
    can_promote(from, to, &[]).is_ok() && from != to
}

/// The guard, as a nameable object.
///
/// The table is a free function because it must be pure and callable from
/// `mm-being`; this type exists so a caller can hold, pass, and substitute a guard
/// the way Phase 4's `IdentityGuard` is held.
#[derive(Debug, Clone, Copy, Default)]
pub struct PromotionGuard;

impl PromotionGuard {
    /// A fresh guard. It holds no state by construction.
    pub fn new() -> Self {
        PromotionGuard
    }

    /// The same decision [`can_promote`] makes.
    pub fn check(
        &self,
        from: EpistemicStatus,
        to: EpistemicStatus,
        evidence: &[Evidence],
    ) -> std::result::Result<(), PromotionDenied> {
        can_promote(from, to, evidence)
    }

    /// The Phase 4 invariant this guard implements.
    pub fn invariant(&self) -> &'static str {
        PHASE_FOUR_INVARIANT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::EvidenceKind;
    use mm_core::Ulid;
    use ulid::Ulid as UlidType;

    fn id(n: u128) -> Ulid {
        UlidType::from_parts(1_700_000_000_000, n)
    }

    fn evidence(kind: EvidenceKind) -> Evidence {
        Evidence::from_content(id(1), kind, None, "support", 0.9).unwrap()
    }

    fn any_evidence() -> Vec<Evidence> {
        vec![
            evidence(EvidenceKind::Document),
            evidence(EvidenceKind::Observation),
            evidence(EvidenceKind::ExternalTool),
        ]
    }

    #[test]
    fn every_forbidden_pair_is_refused_even_with_every_kind_of_evidence() {
        // This is the table's whole point: the evidence cannot matter.
        for (from, to, reason) in FORBIDDEN {
            let denied = can_promote(from, to, &any_evidence()).unwrap_err();
            assert_eq!(denied.reason, reason, "{from} -> {to}");
            assert_eq!(can_promote(from, to, &[]).unwrap_err().reason, reason);
        }
    }

    #[test]
    fn nothing_rises_out_of_fiction_unknown_or_an_observation() {
        for to in crate::status::EPISTEMIC_STATUSES {
            for from in [
                EpistemicStatus::Fictional,
                EpistemicStatus::Unknown,
                EpistemicStatus::Observed,
            ] {
                if from == to {
                    continue;
                }
                assert!(
                    can_promote(from, to, &any_evidence()).is_err(),
                    "{from} -> {to} must be refused"
                );
            }
        }
        // The same pair with no move at all is accepted: a replay must not fail.
        assert!(can_promote(EpistemicStatus::Unknown, EpistemicStatus::Unknown, &[]).is_ok());
    }

    #[test]
    fn reaching_observed_needs_an_observation_record() {
        let no_observation = vec![evidence(EvidenceKind::ExternalTool)];
        assert!(can_promote(
            EpistemicStatus::Reported,
            EpistemicStatus::Observed,
            &no_observation
        )
        .is_err());
        assert!(can_promote(
            EpistemicStatus::Reported,
            EpistemicStatus::Observed,
            &[evidence(EvidenceKind::Observation)]
        )
        .is_ok());
        // And the shared predicate agrees with the guard.
        assert_eq!(
            observation_requirement(EpistemicStatus::Observed, false),
            Some("an observation requires an observation record")
        );
        assert_eq!(
            observation_requirement(EpistemicStatus::Observed, true),
            None
        );
        assert_eq!(
            observation_requirement(EpistemicStatus::Assumed, false),
            None
        );
    }

    #[test]
    fn verification_needs_an_authoritative_check() {
        assert!(can_promote(
            EpistemicStatus::Inferred,
            EpistemicStatus::Verified,
            &[
                evidence(EvidenceKind::Document),
                evidence(EvidenceKind::Observation)
            ]
        )
        .is_err());
        assert!(can_promote(
            EpistemicStatus::Inferred,
            EpistemicStatus::Verified,
            &[evidence(EvidenceKind::ExternalTool)]
        )
        .is_ok());
    }

    #[test]
    fn the_reported_to_inferred_exception_still_needs_evidence() {
        assert!(can_promote(EpistemicStatus::Reported, EpistemicStatus::Inferred, &[]).is_err());
        assert!(can_promote(
            EpistemicStatus::Reported,
            EpistemicStatus::Inferred,
            &[evidence(EvidenceKind::Testimony)]
        )
        .is_ok());
    }

    #[test]
    fn lowering_a_status_is_a_retraction_not_a_promotion() {
        let denied = can_promote(
            EpistemicStatus::Verified,
            EpistemicStatus::Assumed,
            &any_evidence(),
        )
        .unwrap_err();
        assert!(
            denied.reason.contains("not lowered by promotion"),
            "{denied}"
        );
        // Weakening from a hypothesis to an assumption is still a lowering.
        assert!(
            can_promote(EpistemicStatus::Hypothetical, EpistemicStatus::Assumed, &[]).is_ok(),
            "a hypothesis becoming an assumption is a rise"
        );
        assert!(can_promote(EpistemicStatus::Assumed, EpistemicStatus::Hypothetical, &[]).is_err());
    }

    #[test]
    fn a_rise_that_needs_nothing_is_allowed_and_reported_as_such() {
        assert!(can_promote(EpistemicStatus::Hypothetical, EpistemicStatus::Assumed, &[]).is_ok());
        assert!(rises_without_evidence(
            EpistemicStatus::Hypothetical,
            EpistemicStatus::Assumed
        ));
        assert!(!rises_without_evidence(
            EpistemicStatus::Reported,
            EpistemicStatus::Inferred
        ));
        // No move at all is not a rise.
        assert!(!rises_without_evidence(
            EpistemicStatus::Verified,
            EpistemicStatus::Verified
        ));
    }

    #[test]
    fn the_guard_object_agrees_with_the_table_and_names_the_phase_four_invariant() {
        let guard = PromotionGuard::new();
        assert_eq!(guard.invariant(), PHASE_FOUR_INVARIANT);
        assert_eq!(guard.invariant(), "no_assumption_to_observation");
        assert!(guard
            .check(
                EpistemicStatus::Assumed,
                EpistemicStatus::Observed,
                &any_evidence()
            )
            .is_err());
    }
}
