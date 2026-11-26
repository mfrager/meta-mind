//! Identity: the small, immutable self that survives model changes.
//!
//! Everything else in this crate can be revised. The identity cannot: it is
//! created once, its invariants are fixed for the life of the being, and a change
//! to the self-description is recorded as a new *version* rather than by rewriting
//! the old text. That is what makes "the same being" checkable after a restart or
//! a model swap instead of a matter of trust.

use mm_core::{Timestamp, Ulid};
use serde::{Deserialize, Serialize};

use crate::error::{InvariantCode, CORE_INVARIANTS};

/// The identifier of an invariant row.
pub type InvariantId = String;
/// The identifier of a version of the self-description.
pub type VersionId = String;

/// One committed revision of the self-description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfVersion {
    /// The version this revision produced, e.g. `v2`.
    pub version: VersionId,
    /// When the revision was made.
    pub at: Timestamp,
    /// The self-description as of this revision.
    pub self_description: String,
    /// Why the being revised its description of itself.
    pub reason: String,
}

/// The being's immutable core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    /// The identity's ULID; also the subject of every `/being` triple about it.
    pub id: Ulid,
    /// When the identity was created. Never changes.
    pub created_at: Timestamp,
    /// The current self-description.
    pub self_description: String,
    /// Every previous revision, oldest first, excluding the current one.
    pub lineage: Vec<SelfVersion>,
    /// The version the current description carries.
    pub current_version: VersionId,
    /// The invariants that bind this identity. Fixed at creation.
    pub invariants: Vec<InvariantId>,
}

impl Identity {
    /// Create an identity, binding the four core invariants to it.
    pub fn new(id: Ulid, created_at: Timestamp, self_description: impl Into<String>) -> Self {
        Identity {
            id,
            created_at,
            self_description: self_description.into(),
            lineage: Vec::new(),
            current_version: "v1".to_string(),
            invariants: CORE_INVARIANTS
                .iter()
                .map(|c| c.as_str().to_string())
                .collect(),
        }
    }

    /// The invariants this identity enforces, parsed back from their wire names.
    ///
    /// A code stored in the row that this build does not know is skipped rather
    /// than guessed: an unknown invariant is a newer phase's, and inventing its
    /// meaning here would be exactly the kind of silent promotion the guard exists
    /// to prevent.
    pub fn invariant_codes(&self) -> Vec<InvariantCode> {
        self.invariants
            .iter()
            .filter_map(|name| InvariantCode::parse(name))
            .collect()
    }

    /// The numeric part of [`Identity::current_version`].
    pub fn version_number(&self) -> u32 {
        self.current_version
            .trim_start_matches('v')
            .parse::<u32>()
            .unwrap_or(1)
    }

    /// Record a new self-description, keeping the old one in the lineage.
    pub fn evolve(
        &mut self,
        self_description: impl Into<String>,
        at: Timestamp,
        reason: impl Into<String>,
    ) -> SelfVersion {
        let next = format!("v{}", self.version_number() + 1);
        let previous = SelfVersion {
            version: self.current_version.clone(),
            at,
            self_description: std::mem::replace(
                &mut self.self_description,
                self_description.into(),
            ),
            reason: reason.into(),
        };
        self.lineage.push(previous.clone());
        self.current_version = next;
        previous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    #[test]
    fn a_new_identity_binds_all_four_invariants() {
        let identity = Identity::new(id(1), Timestamp::now(), "I am Metamind");
        assert_eq!(identity.invariant_codes().len(), 4);
        assert_eq!(identity.current_version, "v1");
        assert_eq!(identity.version_number(), 1);
        assert!(identity.lineage.is_empty());
        assert_eq!(identity.invariants[0], "no_fabricated_autobiography");
    }

    #[test]
    fn evolution_appends_and_bumps_the_version() {
        let mut identity = Identity::new(id(2), Timestamp::now(), "first");
        let previous = identity.evolve("second", Timestamp::now(), "learned more");
        assert_eq!(previous.self_description, "first");
        assert_eq!(previous.version, "v1");
        assert_eq!(identity.current_version, "v2");
        assert_eq!(identity.self_description, "second");
        assert_eq!(identity.lineage.len(), 1);
    }

    #[test]
    fn an_unknown_invariant_code_is_skipped_not_invented() {
        let mut identity = Identity::new(id(3), Timestamp::now(), "x");
        identity.invariants.push("from.a.future.phase".into());
        assert_eq!(identity.invariant_codes().len(), 4);
    }
}
