//! Contradictions, as explicit objects.
//!
//! The invariant is that a conflict is never a silent edit. When two claims say
//! incompatible things about the same subject and relation, the being records a
//! [`Contradiction`] naming both sides and keeps both claims: reconciliation is a
//! later, logged action, not something detection does on its own.
//!
//! Detection is *indexed*, not pairwise. [`ConflictIndex`] buckets claims by
//! `(subject, predicate)`, so a new claim is only ever compared with the claims it
//! can actually conflict with — the plan's mitigation for the O(n²) risk. The
//! detector is a pure function over the bucket; it takes no model output.
//!
//! **Deviation, recorded here rather than hidden:** the plan asks a formalized
//! fragment to be handed to an ASP (`clingo`) or SMT (`Z3`) solver, with "no
//! stable model ⇒ contradiction". Neither backend is vendored in this workspace,
//! so [`FormalCheck`] reports [`FormalCheck::Unavailable`] and detection stays
//! structural. The seam exists ([`FormalBackend`]) so a later phase with a solver
//! can fill it without changing the detector's contract.

use std::collections::BTreeMap;

use mm_core::Ulid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::claim::Claim;
use crate::error::Result;

/// Where a recorded contradiction stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContradictionStatus {
    /// Detected and unreconciled.
    Open,
    /// One side has been weakened but the record stands.
    Mitigated,
    /// The conflict was settled by a later claim.
    Resolved,
    /// The conflict was judged immaterial.
    Dismissed,
}

/// Every contradiction status, in a stable order.
pub const CONTRADICTION_STATUSES: [ContradictionStatus; 4] = [
    ContradictionStatus::Open,
    ContradictionStatus::Mitigated,
    ContradictionStatus::Resolved,
    ContradictionStatus::Dismissed,
];

impl ContradictionStatus {
    /// The stable wire name, matching `contradictions.status`.
    pub fn as_str(self) -> &'static str {
        match self {
            ContradictionStatus::Open => "open",
            ContradictionStatus::Mitigated => "mitigated",
            ContradictionStatus::Resolved => "resolved",
            ContradictionStatus::Dismissed => "dismissed",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        CONTRADICTION_STATUSES
            .into_iter()
            .find(|status| status.as_str() == text)
    }

    /// True when the record still needs attention.
    pub fn is_open(self) -> bool {
        self == ContradictionStatus::Open
    }
}

impl std::fmt::Display for ContradictionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a formal consistency check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormalCheck {
    /// The pair is not in a formalizable fragment.
    NotApplicable,
    /// A solver was wanted but the workspace has none (the recorded deviation).
    Unavailable,
    /// A stable model exists; the pair is formally consistent.
    Stable,
    /// No stable model exists; the pair is formally inconsistent.
    NoStableModel,
}

impl FormalCheck {
    /// The stable wire name, matching `epistemic.contradiction.detect`'s field.
    pub fn as_str(self) -> &'static str {
        match self {
            FormalCheck::NotApplicable => "not_applicable",
            FormalCheck::Unavailable => "unavailable",
            FormalCheck::Stable => "stable",
            FormalCheck::NoStableModel => "no_stable_model",
        }
    }
}

/// A solver seam. The workspace has no ASP/SMT backend, so the only
/// implementation is [`NoFormalBackend`].
pub trait FormalBackend: Send + Sync {
    /// Check a pair of claims for formal inconsistency.
    fn check_pair(&self, a: &Claim, b: &Claim) -> FormalCheck;
}

/// The backend used when no solver is vendored.
///
/// It never claims a result it did not compute: it reports
/// [`FormalCheck::Unavailable`], which is why an adversarial fixture still yields
/// the structural contradiction rather than a false negative.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFormalBackend;

impl FormalBackend for NoFormalBackend {
    fn check_pair(&self, _a: &Claim, _b: &Claim) -> FormalCheck {
        FormalCheck::Unavailable
    }
}

/// One recorded conflict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contradiction {
    /// The contradiction's own ULID, derived from the pair so a re-detection is
    /// the same record.
    pub id: Ulid,
    /// One side.
    pub claim_a: Ulid,
    /// The other side.
    pub claim_b: Ulid,
    /// Why they conflict, phrased so a human can act on it.
    pub reason: String,
    /// The evidence behind the first side, when it has any.
    pub evidence_a: Option<Ulid>,
    /// The evidence behind the second side, when it has any.
    pub evidence_b: Option<Ulid>,
    /// Where the record stands.
    pub status: ContradictionStatus,
}

impl Contradiction {
    /// Both sides, first then second.
    pub fn sides(&self) -> [Ulid; 2] {
        [self.claim_a, self.claim_b]
    }

    /// True when the record names both sides.
    pub fn names_both_sides(&self) -> bool {
        !self.claim_a.is_nil() && !self.claim_b.is_nil() && self.claim_a != self.claim_b
    }
}

/// The detection interface the plan specifies.
pub trait ContradictionDetector {
    /// The conflicts `new` has with the claims already held.
    fn check(&self, new: &Claim, existing: &[Claim]) -> Vec<Contradiction>;
}

/// The `(subject, predicate)` bucket index.
///
/// Built once from the existing claims; a new claim then only meets the bucket it
/// belongs to. That is the whole O(n²) mitigation.
#[derive(Debug, Clone, Default)]
pub struct ConflictIndex {
    buckets: BTreeMap<(String, String), Vec<Ulid>>,
    by_id: BTreeMap<Ulid, Claim>,
}

impl ConflictIndex {
    /// An empty index.
    pub fn new() -> Self {
        ConflictIndex::default()
    }

    /// Index a set of claims.
    pub fn build(claims: &[Claim]) -> Self {
        let mut index = ConflictIndex::new();
        for claim in claims {
            index.insert(claim.clone());
        }
        index
    }

    /// Add a claim to its bucket.
    pub fn insert(&mut self, claim: Claim) {
        self.buckets
            .entry(claim.proposition.key())
            .or_default()
            .push(claim.id);
        self.by_id.insert(claim.id, claim);
    }

    /// The claims comparable with `claim`: same subject and relation.
    pub fn candidates(&self, claim: &Claim) -> Vec<&Claim> {
        self.buckets
            .get(&claim.proposition.key())
            .into_iter()
            .flatten()
            .filter_map(|id| self.by_id.get(id))
            .filter(|other| other.id != claim.id)
            .collect()
    }

    /// How many buckets the index holds.
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }
}

/// The indexed structural detector.
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexedContradictionDetector {
    backend: NoFormalBackend,
}

impl IndexedContradictionDetector {
    /// A detector with the (absent) formal backend.
    pub fn new() -> Self {
        IndexedContradictionDetector {
            backend: NoFormalBackend,
        }
    }

    /// Check one claim against an index.
    pub fn check_indexed(&self, new: &Claim, index: &ConflictIndex) -> Vec<Contradiction> {
        let candidates: Vec<Claim> = index.candidates(new).into_iter().cloned().collect();
        self.check(new, &candidates)
    }
}

impl ContradictionDetector for IndexedContradictionDetector {
    fn check(&self, new: &Claim, existing: &[Claim]) -> Vec<Contradiction> {
        let mut out = Vec::new();
        for other in existing {
            if !new
                .proposition
                .same_subject_and_predicate(&other.proposition)
            {
                continue;
            }
            if new.proposition.object == other.proposition.object {
                continue;
            }
            // Objects differ on the same subject and relation: the two claims
            // cannot both hold. Record it; do not reconcile it.
            let _formal = self.backend.check_pair(new, other);
            out.push(Contradiction {
                id: contradiction_id(&new.id, &other.id),
                claim_a: new.id,
                claim_b: other.id,
                reason: format!(
                    "{} {} asserts both {} and {}",
                    new.proposition.subject_text(),
                    new.proposition.predicate_text(),
                    new.proposition.object_text(),
                    other.proposition.object_text(),
                ),
                evidence_a: new.evidence.first().copied(),
                evidence_b: other.evidence.first().copied(),
                status: ContradictionStatus::Open,
            });
        }
        out.sort_by_key(|c| c.id);
        out
    }
}

/// The contradiction id a pair implies, independent of which side is "new".
pub fn contradiction_id(a: &Ulid, b: &Ulid) -> Ulid {
    let (first, second) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = Sha256::new();
    hasher.update(b"mm.epistemic.contradiction\0");
    hasher.update(first.to_string().as_bytes());
    hasher.update([0u8]);
    hasher.update(second.to_string().as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Ulid::from_bytes(bytes)
}

/// Detect conflicts in a whole claim set, one record per conflicting pair.
pub fn detect_all(claims: &[Claim]) -> Vec<Contradiction> {
    let detector = IndexedContradictionDetector::new();
    let mut index = ConflictIndex::new();
    let mut out: Vec<Contradiction> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for claim in claims {
        for contradiction in detector.check_indexed(claim, &index) {
            if seen.insert(contradiction.id) {
                out.push(contradiction);
            }
        }
        index.insert(claim.clone());
    }
    out
}

/// True when these two claims can be compared at all.
pub fn comparable(a: &Claim, b: &Claim) -> Result<bool> {
    Ok(a.proposition.same_subject_and_predicate(&b.proposition))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::ClaimKind;
    use crate::status::EpistemicStatus;
    use ulid::Ulid as UlidType;

    fn id(n: u128) -> Ulid {
        UlidType::from_parts(1_700_000_000_000, n)
    }

    fn claim(n: u128, object: &str) -> Claim {
        Claim::new(
            id(n),
            ClaimKind::Fact,
            crate::proposition::Proposition::literal(
                "https://metamind.dev/data/pipeline",
                "https://metamind.dev/ontology#status",
                object,
            )
            .unwrap(),
            EpistemicStatus::Reported,
            0.7,
        )
        .unwrap()
    }

    #[test]
    fn two_claims_with_different_objects_on_the_same_relation_conflict() {
        let a = claim(1, "green");
        let b = claim(2, "red");
        let detector = IndexedContradictionDetector::new();
        let found = detector.check(&b, std::slice::from_ref(&a));
        assert_eq!(found.len(), 1);
        let c = &found[0];
        assert_eq!(c.claim_a, b.id);
        assert_eq!(c.claim_b, a.id);
        assert!(c.names_both_sides());
        assert!(c.status.is_open());
        assert!(
            c.reason.contains("green") && c.reason.contains("red"),
            "{}",
            c.reason
        );
    }

    #[test]
    fn the_same_claim_does_not_conflict_with_itself() {
        let a = claim(1, "green");
        let detector = IndexedContradictionDetector::new();
        assert!(detector.check(&a, std::slice::from_ref(&a)).is_empty());
        // And two claims that agree do not conflict.
        assert!(detector.check(&claim(2, "green"), &[a]).is_empty());
    }

    #[test]
    fn a_different_predicate_is_not_comparable() {
        let a = claim(1, "green");
        let mut b = claim(2, "red");
        b.proposition.predicate =
            crate::proposition::iri_node("https://metamind.dev/ontology#color").unwrap();
        let detector = IndexedContradictionDetector::new();
        assert!(detector.check(&b, &[a]).is_empty());
    }

    #[test]
    fn the_index_only_offers_comparable_candidates() {
        let index = ConflictIndex::build(&[claim(1, "green"), claim(2, "blue")]);
        let candidates = index.candidates(&claim(3, "red"));
        assert_eq!(candidates.len(), 2);
        assert_eq!(index.bucket_count(), 1);
        let mut other = claim(4, "x");
        other.proposition.predicate =
            crate::proposition::iri_node("https://metamind.dev/ontology#owner").unwrap();
        assert!(index.candidates(&other).is_empty());
    }

    #[test]
    fn the_id_is_a_function_of_the_pair_not_its_order() {
        assert_eq!(
            contradiction_id(&id(1), &id(2)),
            contradiction_id(&id(2), &id(1))
        );
        assert_ne!(
            contradiction_id(&id(1), &id(2)),
            contradiction_id(&id(1), &id(3))
        );
        assert!(!contradiction_id(&id(1), &id(2)).is_nil());
    }

    #[test]
    fn detect_all_reports_each_pair_once() {
        let claims = vec![claim(1, "green"), claim(2, "red"), claim(3, "blue")];
        let found = detect_all(&claims);
        // Three claims on one relation with three different objects: three pairs.
        assert_eq!(found.len(), 3);
        let ids: std::collections::BTreeSet<Ulid> = found.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn the_formal_backend_reports_itself_unavailable_rather_than_guessing() {
        let backend = NoFormalBackend;
        assert_eq!(
            backend.check_pair(&claim(1, "a"), &claim(2, "b")),
            FormalCheck::Unavailable
        );
        assert_eq!(FormalCheck::Unavailable.as_str(), "unavailable");
    }
}
