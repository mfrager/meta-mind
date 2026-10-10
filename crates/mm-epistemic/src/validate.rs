//! The validation barrier: the one door into `/world`.
//!
//! Two jobs, both structural rather than advisory:
//!
//! 1. **It decides admission.** [`ValidationBarrier::admit`] accepts a claim only
//!    when its status is `OBSERVED` or `VERIFIED` *and* it carries evidence. Every
//!    other status is refused with a typed [`WorldRefusal`], and
//!    [`ValidationBarrier::mirror_world`] is the only function in the crate that
//!    writes the `/world` graph.
//! 2. **It runs the data-quality suite.** [`data_tests`] is the RDFUnit idea:
//!    executable checks over the epistemic state that report *findings* rather than
//!    silently repairing anything. A finding is a fact about the store, phrased so
//!    a human can fix the record it names.
//!
//! The barrier takes no model output, and it never repairs. A defect either blocks
//! the write (admission) or is reported (`data-tests`), because a validator that
//! edited what it validated would make its own report meaningless.

use mm_store_graph::GraphHandle;
use serde::{Deserialize, Serialize};

use crate::assumption::Assumption;
use crate::claim::Claim;
use crate::error::{Result, WorldRefusal};
use crate::rdf::WORLD_GRAPH;

/// How serious a data-test finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Worth knowing.
    Info,
    /// Suspicious but not yet wrong.
    Warning,
    /// The record cannot be trusted as it stands.
    Violation,
}

impl Severity {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Violation => "violation",
        }
    }
}

/// One data-quality finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// The test that produced it.
    pub test_id: String,
    /// How serious it is.
    pub severity: Severity,
    /// The record it is about.
    pub subject: String,
    /// What is wrong.
    pub detail: String,
}

impl Finding {
    /// A violation.
    pub fn violation(test_id: &str, subject: impl Into<String>, detail: impl Into<String>) -> Self {
        Finding {
            test_id: test_id.to_string(),
            severity: Severity::Violation,
            subject: subject.into(),
            detail: detail.into(),
        }
    }
}

/// Run the RDFUnit-style suite over the epistemic state.
///
/// The checks are the invariants the shapes cannot express over the *tabulated*
/// state: a claim's confidence, an observed claim's evidence, an assumption's
/// cost/risk fields. Each returns a finding rather than fixing anything, so the
/// suite is safe to run at any time.
pub fn data_tests(claims: &[Claim], assumptions: &[Assumption]) -> Vec<Finding> {
    let mut findings = Vec::new();

    for claim in claims {
        let subject = mm_core::ulid_string(&claim.id);
        if !(0.0..=1.0).contains(&claim.confidence) {
            findings.push(Finding::violation(
                "confidence_range",
                &subject,
                format!("confidence {} is outside [0,1]", claim.confidence),
            ));
        }
        if claim.status.is_world_admissible() && claim.evidence.is_empty() {
            findings.push(Finding::violation(
                "world_requires_evidence",
                &subject,
                format!("a {} claim carries no evidence", claim.status),
            ));
        }
        if claim.proposition.subject_text().trim().is_empty() {
            findings.push(Finding::violation(
                "proposition_subject",
                &subject,
                "a claim names no subject",
            ));
        }
    }

    for assumption in assumptions {
        let subject = mm_core::ulid_string(&assumption.id);
        if let Err(error) = assumption.validate() {
            findings.push(Finding::violation(
                "assumption_well_formed",
                &subject,
                error.to_string(),
            ));
        }
    }

    findings
}

/// The barrier.
///
/// It holds no state: the rule is a pure predicate over the claim, which is what
/// lets the same decision be replayed from the audit log.
#[derive(Debug, Clone, Copy, Default)]
pub struct ValidationBarrier;

impl ValidationBarrier {
    /// A fresh barrier.
    pub fn new() -> Self {
        ValidationBarrier
    }

    /// The admission decision, as a typed refusal when it fails.
    pub fn admit(claim: &Claim) -> std::result::Result<(), WorldRefusal> {
        if !claim.status.is_world_admissible() {
            return Err(WorldRefusal {
                claim: mm_core::ulid_string(&claim.id),
                status: claim.status,
                reason: "only OBSERVED and VERIFIED claims describe the world".to_string(),
            });
        }
        if claim.evidence.is_empty() {
            return Err(WorldRefusal {
                claim: mm_core::ulid_string(&claim.id),
                status: claim.status,
                reason: "a world claim must carry evidence".to_string(),
            });
        }
        Ok(())
    }

    /// Admit or refuse, as a crate [`Result`].
    pub fn check(claim: &Claim) -> Result<()> {
        ValidationBarrier::admit(claim).map_err(crate::error::EpistemicError::World)
    }

    /// Rewrite `/world` from a snapshot of claims: clear, then insert every
    /// admissible claim, refusing the whole write if even one is not admissible.
    ///
    /// Refusing atomically matters: a partial `/world` would hold a claim whose
    /// status nobody admitted, which is exactly the leakage the separation exists
    /// to prevent.
    pub async fn mirror_world(handle: &GraphHandle, claims: &[Claim]) -> Result<usize> {
        for claim in claims {
            ValidationBarrier::check(claim)?;
        }
        handle
            .clear_graph(WORLD_GRAPH)
            .await
            .map_err(crate::error::EpistemicError::from)?;
        let mut written = 0usize;
        for claim in claims {
            for quad in crate::rdf::world_claim_quads(claim) {
                handle
                    .insert(quad)
                    .await
                    .map_err(crate::error::EpistemicError::from)?;
                written += 1;
            }
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::{ClaimKind, Evidence, EvidenceKind};
    use crate::proposition::{Proposition, RiskLevel};
    use crate::status::EpistemicStatus;
    use mm_core::Ulid;
    use ulid::Ulid as UlidType;

    fn id(n: u128) -> Ulid {
        UlidType::from_parts(1_700_000_000_000, n)
    }

    fn claim(n: u128, status: EpistemicStatus) -> Claim {
        Claim::new(
            id(n),
            ClaimKind::Fact,
            Proposition::literal("https://x/s", "https://x/p", "v").unwrap(),
            status,
            0.5,
        )
        .unwrap()
    }

    #[test]
    fn only_supported_world_statuses_get_in() {
        assert!(ValidationBarrier::admit(&claim(1, EpistemicStatus::Assumed)).is_err());
        assert!(ValidationBarrier::admit(&claim(1, EpistemicStatus::Inferred)).is_err());
        // World status but no evidence: still refused.
        let mut observed = claim(1, EpistemicStatus::Observed);
        assert!(ValidationBarrier::admit(&observed).is_err());
        observed.evidence = vec![id(9)];
        ValidationBarrier::admit(&observed).unwrap();
    }

    #[test]
    fn the_refusal_names_why() {
        let refusal = ValidationBarrier::admit(&claim(1, EpistemicStatus::Predicted)).unwrap_err();
        assert_eq!(refusal.status, EpistemicStatus::Predicted);
        assert!(
            refusal.reason.contains("OBSERVED") || refusal.reason.contains("world"),
            "{refusal}"
        );
    }

    #[test]
    fn the_suite_flags_an_unsupported_world_claim() {
        let clean = {
            let mut c = claim(1, EpistemicStatus::Verified);
            c.evidence = vec![id(9)];
            c
        };
        assert!(data_tests(&[clean], &[]).is_empty());

        let broken = claim(1, EpistemicStatus::Observed);
        let findings = data_tests(&[broken], &[]);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].test_id, "world_requires_evidence");
        assert_eq!(findings[0].severity, Severity::Violation);
    }

    #[test]
    fn the_suite_flags_a_malformed_assumption() {
        let mut assumption = Assumption::new(
            id(1),
            Proposition::literal("https://x/s", "https://x/p", "v").unwrap(),
            0.5,
            RiskLevel::Low,
            1.0,
            0.5,
        )
        .unwrap();
        // Break it behind the constructor's back, as a corrupt row would.
        assumption.confidence = 2.0;
        let findings = data_tests(&[], &[assumption]);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].test_id, "assumption_well_formed");
    }

    #[test]
    fn evidence_is_not_required_for_a_non_world_claim() {
        let plausible = claim(1, EpistemicStatus::Assumed);
        assert!(data_tests(&[plausible], &[]).is_empty());
        let _ = Evidence::from_content(id(1), EvidenceKind::Document, None, "x", 0.5).unwrap();
    }
}
