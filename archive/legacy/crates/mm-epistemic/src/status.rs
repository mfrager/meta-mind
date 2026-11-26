//! The status lattice: what a claim *is*, and what it may become.
//!
//! The ten statuses are design §32's epistemic kinds. Two facts about them matter
//! everywhere else in the crate:
//!
//! 1. **They are ordered by strength**, strongest first, and the order is the
//!    enum's declaration order. `strength` is that order, and it is the only place
//!    the ordering is written down.
//! 2. **Only two of them live in `/world`.** `OBSERVED` and `VERIFIED` are the
//!    statuses that describe the world; everything else describes the being's
//!    attitude toward a proposition. That distinction is what the `/world` graph
//!    separation exists to make impossible to lose.
//!
//! The wire names are the uppercase forms the plan's PROV examples and the gate's
//! SPARQL query agree on (`"OBSERVED"`, `"INFERRED"`), not the Rust variant names.

use serde::{Deserialize, Serialize};

/// The epistemic status of a proposition.
///
/// Declaration order is strength order: `Observed` strongest, `Unknown` weakest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EpistemicStatus {
    /// Seen directly, with an observation record.
    Observed,
    /// Established by an authoritative check.
    Verified,
    /// Asserted by a source, not yet checked.
    Reported,
    /// Derived from other claims.
    Inferred,
    /// Held without support, for the sake of a decision.
    Assumed,
    /// A conjecture that is testable but untested.
    Hypothetical,
    /// An expectation about the future.
    Predicted,
    /// Produced by a simulation.
    Simulated,
    /// Invented; never true of the world.
    Fictional,
    /// Not yet classified.
    Unknown,
}

/// Every status, strongest first.
pub const EPISTEMIC_STATUSES: [EpistemicStatus; 10] = [
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

impl EpistemicStatus {
    /// The stable wire name, matching `mm:status` in the mirror and the plans.
    pub fn as_str(self) -> &'static str {
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

    /// Parse a wire name, case-insensitively.
    pub fn parse(text: &str) -> Option<Self> {
        let wanted = text.trim().to_ascii_uppercase();
        EPISTEMIC_STATUSES
            .into_iter()
            .find(|status| status.as_str() == wanted)
    }

    /// The strength order, as a number: `Observed` is 10 and `Unknown` is 1.
    ///
    /// Higher is stronger. A promotion moves upward in this number, with the one
    /// documented exception [`crate::guard::can_promote`] carves out.
    pub fn strength(self) -> u8 {
        10 - self.rank()
    }

    /// The zero-based position in the declared order, strongest first.
    pub fn rank(self) -> u8 {
        match self {
            EpistemicStatus::Observed => 0,
            EpistemicStatus::Verified => 1,
            EpistemicStatus::Reported => 2,
            EpistemicStatus::Inferred => 3,
            EpistemicStatus::Assumed => 4,
            EpistemicStatus::Hypothetical => 5,
            EpistemicStatus::Predicted => 6,
            EpistemicStatus::Simulated => 7,
            EpistemicStatus::Fictional => 8,
            EpistemicStatus::Unknown => 9,
        }
    }

    /// True when the status describes the world rather than the being's attitude.
    ///
    /// This is the whole admission rule for `/world`, and it is one predicate so
    /// there is one place to change if the rule ever changes.
    pub fn is_world_admissible(self) -> bool {
        matches!(self, EpistemicStatus::Observed | EpistemicStatus::Verified)
    }

    /// True when the status may be *promoted from* at all.
    ///
    /// A fictional or unknown proposition is not a weak belief; it is not a belief
    /// about the world at all. Nothing raises it in place.
    pub fn can_rise(self) -> bool {
        !matches!(self, EpistemicStatus::Fictional | EpistemicStatus::Unknown)
    }
}

impl std::fmt::Display for EpistemicStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_round_trip_and_are_uppercase() {
        for status in EPISTEMIC_STATUSES {
            assert_eq!(status.as_str(), status.as_str().to_ascii_uppercase());
            assert_eq!(EpistemicStatus::parse(status.as_str()), Some(status));
        }
        // The gate's SPARQL query compares against these two literals.
        assert_eq!(EpistemicStatus::Observed.as_str(), "OBSERVED");
        assert_eq!(EpistemicStatus::Verified.as_str(), "VERIFIED");
        assert_eq!(
            EpistemicStatus::parse("observed"),
            Some(EpistemicStatus::Observed)
        );
        assert_eq!(EpistemicStatus::parse("nonsense"), None);
    }

    #[test]
    fn strength_follows_the_declared_order() {
        let mut previous = u8::MAX;
        for status in EPISTEMIC_STATUSES {
            assert!(status.strength() < previous, "{status} must rank lower");
            previous = status.strength();
        }
        assert_eq!(EpistemicStatus::Observed.strength(), 10);
        assert_eq!(EpistemicStatus::Unknown.strength(), 1);
    }

    #[test]
    fn only_two_statuses_describe_the_world() {
        let admissible: Vec<&str> = EPISTEMIC_STATUSES
            .into_iter()
            .filter(|s| s.is_world_admissible())
            .map(EpistemicStatus::as_str)
            .collect();
        assert_eq!(admissible, vec!["OBSERVED", "VERIFIED"]);
        assert!(!EpistemicStatus::Fictional.can_rise());
        assert!(!EpistemicStatus::Unknown.can_rise());
        assert!(EpistemicStatus::Assumed.can_rise());
    }
}
