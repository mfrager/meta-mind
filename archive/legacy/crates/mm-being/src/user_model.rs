//! The user model: core prose plus probabilistic beliefs with an evidence ladder.
//!
//! The dangerous failure of a user model is not being wrong — it is *silently
//! becoming certain*. A guess about the user's preference must not drift into
//! something the being will act on as though it were observed. So a belief carries
//! an [`EpistemicStatus`], promotion is a separate, explicit act, and
//! [`UserModel::promote`] refuses any promotion the evidence does not support.
//! There is no code path that quietly upgrades a status.

use std::collections::BTreeMap;

use mm_core::{Timestamp, Ulid};
use serde::{Deserialize, Serialize};

use crate::blocks::CoreBlock;
use crate::error::{BeingError, PromotionDenied, Result};

/// How sure the being is entitled to be, and on what basis.
///
/// The ladder is ordered by evidence strength, not by confidence: a `REPORTED`
/// belief can be held with high confidence and still never license an action the
/// way an `OBSERVED` one does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EpistemicStatus {
    /// The being saw it happen.
    Observed,
    /// A second, independent source confirmed it.
    Verified,
    /// The user or a document said so.
    Reported,
    /// Derived from something stronger.
    Inferred,
    /// Taken as true for now, with no evidence.
    Assumed,
    /// A possibility under consideration.
    Hypothetical,
    /// A forward-looking guess.
    Predicted,
    /// Produced by a simulation, not by the world.
    Simulated,
    /// Deliberately invented.
    Fictional,
    /// Nothing is claimed.
    Unknown,
}

impl EpistemicStatus {
    /// The wire name, matching the migration's `CHECK (epistemic_status IN …)`.
    pub fn as_str(&self) -> &'static str {
        match self {
            EpistemicStatus::Observed => "OBSERVED",
            EpistemicStatus::Verified => "VERIFIED",
            EpistemicStatus::Reported => "REPORTED",
            EpistemicStatus::Inferred => "INFERRED",
            EpistemicStatus::Assumed => "ASSUMED",
            EpistemicStatus::Hypothetical => "HYPOTHETICAL",
            EpistemicStatus::Predicted => "PREDICTED",
            EpistemicStatus::Simulated => "SIMULATED",
            EpistemicStatus::Fictional => "FICTIONAL",
            EpistemicStatus::Unknown => "UNKNOWN",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "OBSERVED" => EpistemicStatus::Observed,
            "VERIFIED" => EpistemicStatus::Verified,
            "REPORTED" => EpistemicStatus::Reported,
            "INFERRED" => EpistemicStatus::Inferred,
            "ASSUMED" => EpistemicStatus::Assumed,
            "HYPOTHETICAL" => EpistemicStatus::Hypothetical,
            "PREDICTED" => EpistemicStatus::Predicted,
            "SIMULATED" => EpistemicStatus::Simulated,
            "FICTIONAL" => EpistemicStatus::Fictional,
            "UNKNOWN" => EpistemicStatus::Unknown,
            _ => return None,
        })
    }

    /// How many evidence records reaching this status requires.
    ///
    /// An observation is the only thing that may produce `OBSERVED`, and a
    /// confirmation is what makes `VERIFIED` more than one report.
    pub fn required_evidence(&self) -> usize {
        match self {
            EpistemicStatus::Observed => 1,
            EpistemicStatus::Verified => 2,
            EpistemicStatus::Inferred => 1,
            _ => 0,
        }
    }

    /// The evidence rank, used to decide whether a change is an upgrade.
    pub fn rank(&self) -> u8 {
        match self {
            EpistemicStatus::Verified => 4,
            EpistemicStatus::Observed => 3,
            EpistemicStatus::Reported | EpistemicStatus::Inferred => 2,
            EpistemicStatus::Predicted
            | EpistemicStatus::Assumed
            | EpistemicStatus::Hypothetical
            | EpistemicStatus::Simulated => 1,
            EpistemicStatus::Fictional | EpistemicStatus::Unknown => 0,
        }
    }

    /// True only for a status that is itself an observation.
    pub fn is_observation(&self) -> bool {
        matches!(self, EpistemicStatus::Observed)
    }
}

impl std::fmt::Display for EpistemicStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The identifier of an evidence record. The evidence ledger itself is Phase 6's;
/// here it is only an id that a promotion must be able to name.
pub type EvidenceId = String;

/// The key a belief is stored under.
pub type PropositionId = String;

/// One belief about the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Belief {
    /// What is believed, as a proposition.
    pub proposition: String,
    /// How the being is entitled to hold it.
    pub epistemic_status: EpistemicStatus,
    /// How strongly it is held, in `[0, 1]`.
    pub confidence: f32,
    /// The evidence records this belief rests on.
    pub evidence: Vec<EvidenceId>,
    /// When it became valid, if known.
    pub valid_from: Option<Timestamp>,
    /// When it stopped being valid, if it has.
    pub valid_until: Option<Timestamp>,
}

impl Belief {
    /// A belief at a status, with no evidence yet.
    pub fn new(proposition: impl Into<String>, status: EpistemicStatus, confidence: f32) -> Self {
        Belief {
            proposition: proposition.into(),
            epistemic_status: status,
            confidence: confidence.clamp(0.0, 1.0),
            evidence: Vec::new(),
            valid_from: None,
            valid_until: None,
        }
    }

    /// The same belief with its evidence attached.
    pub fn with_evidence(mut self, ids: Vec<EvidenceId>) -> Self {
        self.evidence = ids;
        self
    }

    /// The content hash of the proposition, so a belief can be correlated without
    /// recording the user's words.
    pub fn proposition_hash(&self) -> String {
        mm_core::content_hash(self.proposition.as_bytes())
    }
}

/// The being's model of one user: prose in a core block, beliefs in a map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserModel {
    /// The user this model is about.
    pub user_id: Ulid,
    /// The `human` core block.
    pub block: CoreBlock,
    /// The beliefs, keyed by proposition id.
    pub beliefs: BTreeMap<PropositionId, Belief>,
}

impl UserModel {
    /// An empty model over a `human` block.
    pub fn new(user_id: Ulid, block: CoreBlock) -> Self {
        UserModel {
            user_id,
            block,
            beliefs: BTreeMap::new(),
        }
    }

    /// Insert a belief, returning the one it replaced.
    pub fn insert(&mut self, id: PropositionId, belief: Belief) -> Option<Belief> {
        self.beliefs.insert(id, belief)
    }

    /// Look a belief up.
    pub fn get(&self, id: &str) -> Option<&Belief> {
        self.beliefs.get(id)
    }

    /// Promote a belief to a status, but only if the evidence supports it.
    ///
    /// Two rules, both refusals rather than coercions: the target status's
    /// required evidence must be present, and an *upgrade* may not rest on the
    /// same or less evidence than the belief already had. Everything else —
    /// holding a status, moving sideways or down — is allowed, because forgetting
    /// or reclassifying is legitimate and only *silent upgrading* is not.
    pub fn promote(
        &mut self,
        id: &str,
        to: EpistemicStatus,
        evidence: &[EvidenceId],
    ) -> Result<&Belief> {
        let belief = self
            .beliefs
            .get_mut(id)
            .ok_or_else(|| BeingError::NotFound(format!("belief `{id}`")))?;

        // Built eagerly rather than captured lazily: the refusal path must not
        // keep a borrow of `belief` alive across the assignment on success.
        let denial = |from: EpistemicStatus, reason: String| {
            BeingError::Promotion(PromotionDenied {
                proposition: id.to_string(),
                from: from.as_str().to_string(),
                to: to.as_str().to_string(),
                reason,
            })
        };

        let required = to.required_evidence();
        if evidence.len() < required {
            return Err(denial(
                belief.epistemic_status,
                format!(
                    "{to} requires {required} evidence record(s), {} given",
                    evidence.len()
                ),
            ));
        }
        let upgrading = to.rank() > belief.epistemic_status.rank();
        if upgrading && evidence.len() <= belief.evidence.len() {
            return Err(denial(
                belief.epistemic_status,
                format!(
                    "an upgrade to {to} needs more evidence than the {} already held",
                    belief.evidence.len()
                ),
            ));
        }

        belief.epistemic_status = to;
        belief.evidence = evidence.to_vec();
        Ok(belief)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::BlockKind;

    fn model() -> UserModel {
        let block = CoreBlock::new(
            BlockKind::Human,
            "human",
            "the primary user",
            1500,
            Ulid::from_parts(1, 1),
        )
        .unwrap();
        let mut model = UserModel::new(Ulid::from_parts(1, 2), block);
        model.insert(
            "likes_rust".into(),
            Belief::new("likes Rust", EpistemicStatus::Assumed, 0.4),
        );
        model
    }

    #[test]
    fn statuses_round_trip_on_their_uppercase_wire_names() {
        let all = [
            EpistemicStatus::Observed,
            EpistemicStatus::Verified,
            EpistemicStatus::Reported,
            EpistemicStatus::Inferred,
            EpistemicStatus::Assumed,
            EpistemicStatus::Hypothetical,
            EpistemicStatus::Predicted,
            EpistemicStatus::Simulated,
            EpistemicStatus::Fictional,
            EpistemicStatus::Unknown,
        ];
        for s in all {
            assert_eq!(EpistemicStatus::parse(s.as_str()), Some(s));
            assert_eq!(s.as_str(), s.as_str().to_uppercase());
        }
        assert_eq!(
            EpistemicStatus::parse("observed"),
            None,
            "wire names are exact"
        );
    }

    #[test]
    fn an_assumption_never_becomes_an_observation_without_evidence() {
        let mut model = model();
        let err = model
            .promote("likes_rust", EpistemicStatus::Observed, &[])
            .unwrap_err();
        assert_eq!(err.kind(), "promotion_denied");
        match err {
            BeingError::Promotion(d) => {
                assert_eq!(d.from, "ASSUMED");
                assert_eq!(d.to, "OBSERVED");
                assert!(d.reason.contains("1 evidence"), "{d}");
            }
            other => panic!("wrong error: {other:?}"),
        }
        assert_eq!(
            model.get("likes_rust").unwrap().epistemic_status,
            EpistemicStatus::Assumed,
            "a denied promotion must not move the status"
        );
    }

    #[test]
    fn an_observation_with_one_evidence_record_is_allowed() {
        let mut model = model();
        let belief = model
            .promote("likes_rust", EpistemicStatus::Observed, &["ev-1".into()])
            .unwrap();
        assert_eq!(belief.epistemic_status, EpistemicStatus::Observed);
        assert!(belief.epistemic_status.is_observation());
        assert_eq!(belief.evidence, vec!["ev-1".to_string()]);
        assert_eq!(belief.proposition_hash().len(), 64);
    }

    #[test]
    fn verification_needs_two_independent_records() {
        let mut model = model();
        assert!(model
            .promote("likes_rust", EpistemicStatus::Verified, &["ev-1".into()])
            .is_err());
        let belief = model
            .promote(
                "likes_rust",
                EpistemicStatus::Verified,
                &["ev-1".into(), "ev-2".into()],
            )
            .unwrap();
        assert_eq!(belief.epistemic_status, EpistemicStatus::Verified);
    }

    #[test]
    fn an_upgrade_may_not_rest_on_a_smaller_evidence_set() {
        let mut model = model();
        model
            .promote(
                "likes_rust",
                EpistemicStatus::Reported,
                &["a".into(), "b".into(), "c".into()],
            )
            .unwrap();
        let was = model.get("likes_rust").unwrap().clone();
        let err = model
            .promote(
                "likes_rust",
                EpistemicStatus::Observed,
                &["a".into(), "b".into()],
            )
            .unwrap_err();
        assert_eq!(err.kind(), "promotion_denied");
        assert_eq!(model.get("likes_rust").unwrap(), &was);
    }

    #[test]
    fn a_missing_proposition_is_not_found() {
        let mut model = model();
        let err = model
            .promote("nope", EpistemicStatus::Observed, &["ev".into()])
            .unwrap_err();
        assert_eq!(err.kind(), "not_found");
    }
}
