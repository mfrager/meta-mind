//! Relationships: a multi-dimensional, event-sourced view of someone.
//!
//! The relationship is *not* the row. The row is a projection of the events, and
//! the invariant that makes the projection trustworthy is stated as a test:
//! rebuilding from the event list must reproduce exactly what incremental
//! application produced. Without that, a replay could disagree with the live
//! state and there would be no way to tell which one was right.
//!
//! Dimensions are clamped to `[0, 1]`: a relationship is a degree, never a
//! quantity that can run past its own bounds.

use serde::{Deserialize, Serialize};

use mm_core::Ulid;

/// The kind of interaction a relationship event records.
///
/// The kind is what selects the delta — a caller cannot hand in an arbitrary
/// delta and call it trust. That keeps "what a conflict does to trust" a
/// property of the model rather than of whoever wrote the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelKind {
    /// Ordinary contact: familiarity grows slowly.
    Interaction,
    /// Something personal was shared: openness and trust grow.
    Disclosure,
    /// A disagreement: trust and recent quality fall, and it stays unresolved.
    Conflict,
    /// A repair after a conflict: trust recovers, the issue is not erased.
    Repair,
    /// A promise kept: reliance grows.
    CommitmentKept,
    /// A promise broken: reliance and trust fall, and it stays unresolved.
    CommitmentBroken,
}

impl RelKind {
    /// The stable wire name, recorded in `relationship_events.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            RelKind::Interaction => "interaction",
            RelKind::Disclosure => "disclosure",
            RelKind::Conflict => "conflict",
            RelKind::Repair => "repair",
            RelKind::CommitmentKept => "commitment_kept",
            RelKind::CommitmentBroken => "commitment_broken",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "interaction" => Some(RelKind::Interaction),
            "disclosure" => Some(RelKind::Disclosure),
            "conflict" => Some(RelKind::Conflict),
            "repair" => Some(RelKind::Repair),
            "commitment_kept" => Some(RelKind::CommitmentKept),
            "commitment_broken" => Some(RelKind::CommitmentBroken),
            _ => None,
        }
    }

    /// True when this kind leaves something unresolved.
    pub fn leaves_an_issue(self) -> bool {
        matches!(self, RelKind::Conflict | RelKind::CommitmentBroken)
    }
}

impl std::fmt::Display for RelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How each dimension moves for one event.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct DimDelta {
    /// How well the other person is known.
    pub familiarity: f32,
    /// Confidence that they mean well and will not harm.
    pub trust: f32,
    /// Balance of give and take.
    pub reciprocity: f32,
    /// Willingness to share.
    pub openness: f32,
    /// Willingness to work together.
    pub cooperation: f32,
    /// How much is rested on them.
    pub reliance: f32,
    /// Quality of the most recent exchanges.
    pub recent_quality: f32,
}

impl DimDelta {
    /// A delta that changes nothing.
    pub fn zero() -> Self {
        Self::default()
    }

    /// The delta implied by a kind, so the model owns the arithmetic.
    fn for_kind(kind: RelKind) -> Self {
        let mut d = Self::zero();
        match kind {
            RelKind::Interaction => {
                d.familiarity = 0.05;
                d.recent_quality = 0.05;
                d.trust = 0.01;
            }
            RelKind::Disclosure => {
                d.openness = 0.08;
                d.trust = 0.04;
                d.familiarity = 0.02;
            }
            RelKind::Conflict => {
                d.trust = -0.10;
                d.recent_quality = -0.15;
                d.cooperation = -0.05;
                d.openness = -0.02;
            }
            RelKind::Repair => {
                d.trust = 0.06;
                d.recent_quality = 0.05;
                d.cooperation = 0.04;
            }
            RelKind::CommitmentKept => {
                d.trust = 0.05;
                d.reliance = 0.06;
                d.reciprocity = 0.03;
            }
            RelKind::CommitmentBroken => {
                d.trust = -0.12;
                d.reliance = -0.08;
                d.recent_quality = -0.10;
            }
        }
        d
    }
}

/// One thing that happened, and what it moved.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RelationshipEvent {
    /// What happened.
    pub kind: RelKind,
    /// How the dimensions moved.
    pub delta: DimDelta,
    /// The row this event refers to, when there is one.
    pub ref_ulid: Option<Ulid>,
}

impl RelationshipEvent {
    /// An event of `kind` with the delta the model assigns to it.
    pub fn for_kind(kind: RelKind, ref_ulid: Option<Ulid>) -> Self {
        RelationshipEvent {
            kind,
            delta: DimDelta::for_kind(kind),
            ref_ulid,
        }
    }
}

/// The projected relationship.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipState {
    /// How well the other person is known.
    pub familiarity: f32,
    /// Confidence that they mean well and will not harm.
    pub trust: f32,
    /// Balance of give and take.
    pub reciprocity: f32,
    /// Willingness to share.
    pub openness: f32,
    /// Willingness to work together.
    pub cooperation: f32,
    /// How much is rested on them.
    pub reliance: f32,
    /// Quality of the most recent exchanges.
    pub recent_quality: f32,
    /// Issues that were raised and not yet repaired.
    pub unresolved: Vec<String>,
}

impl Default for RelationshipState {
    fn default() -> Self {
        RelationshipState::neutral()
    }
}

impl RelationshipState {
    /// Where a relationship starts: unknown, but not hostile.
    ///
    /// `0.1` rather than `0.0` because zero trust is a judgment, and no judgment
    /// has been earned yet.
    pub fn neutral() -> Self {
        RelationshipState {
            familiarity: 0.1,
            trust: 0.1,
            reciprocity: 0.1,
            openness: 0.1,
            cooperation: 0.1,
            reliance: 0.1,
            recent_quality: 0.1,
            unresolved: Vec::new(),
        }
    }

    /// Apply one event to this state.
    pub fn apply(&mut self, e: &RelationshipEvent) {
        self.familiarity = clamp01(self.familiarity + e.delta.familiarity);
        self.trust = clamp01(self.trust + e.delta.trust);
        self.reciprocity = clamp01(self.reciprocity + e.delta.reciprocity);
        self.openness = clamp01(self.openness + e.delta.openness);
        self.cooperation = clamp01(self.cooperation + e.delta.cooperation);
        self.reliance = clamp01(self.reliance + e.delta.reliance);
        self.recent_quality = clamp01(self.recent_quality + e.delta.recent_quality);
        if e.kind.leaves_an_issue() {
            self.unresolved.push(e.kind.as_str().to_string());
        }
    }

    /// Rebuild the state from the whole event list, in order.
    ///
    /// This is the projection rule the store relies on: a replay of the events
    /// must land exactly where incremental application landed.
    pub fn project(events: &[RelationshipEvent]) -> Self {
        let mut state = RelationshipState::neutral();
        for event in events {
            state.apply(event);
        }
        state
    }

    /// One dimension by name, or `None` when the name is not a dimension.
    pub fn dimension(&self, name: &str) -> Option<f32> {
        match name {
            "familiarity" => Some(self.familiarity),
            "trust" => Some(self.trust),
            "reciprocity" => Some(self.reciprocity),
            "openness" => Some(self.openness),
            "cooperation" => Some(self.cooperation),
            "reliance" => Some(self.reliance),
            "recent_quality" => Some(self.recent_quality),
            _ => None,
        }
    }

    /// The dimension names, in a stable order.
    pub fn dimension_names() -> [&'static str; 7] {
        [
            "familiarity",
            "trust",
            "reciprocity",
            "openness",
            "cooperation",
            "reliance",
            "recent_quality",
        ]
    }
}

fn clamp01(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn script() -> Vec<RelationshipEvent> {
        vec![
            RelationshipEvent::for_kind(RelKind::Interaction, Some(id(1))),
            RelationshipEvent::for_kind(RelKind::Disclosure, None),
            RelationshipEvent::for_kind(RelKind::Conflict, Some(id(2))),
            RelationshipEvent::for_kind(RelKind::Repair, None),
            RelationshipEvent::for_kind(RelKind::CommitmentKept, Some(id(3))),
            RelationshipEvent::for_kind(RelKind::CommitmentBroken, None),
        ]
    }

    #[test]
    fn a_projection_rebuilds_exactly_what_incremental_application_produced() {
        let events = script();
        let mut incremental = RelationshipState::neutral();
        for event in &events {
            incremental.apply(event);
        }
        assert_eq!(RelationshipState::project(&events), incremental);
        // And it is a pure function of the list, so a second rebuild agrees.
        assert_eq!(
            RelationshipState::project(&events),
            RelationshipState::project(&events)
        );
    }

    #[test]
    fn dimensions_stay_inside_the_unit_interval() {
        let mut state = RelationshipState::neutral();
        for _ in 0..50 {
            for event in script() {
                state.apply(&event);
            }
        }
        for name in RelationshipState::dimension_names() {
            let value = state.dimension(name).expect("a known dimension");
            assert!((0.0..=1.0).contains(&value), "{name} = {value}");
        }
        assert_eq!(state.dimension("nonsense"), None);
    }

    #[test]
    fn only_conflict_and_a_broken_commitment_leave_an_issue() {
        let conflict =
            RelationshipState::project(&[RelationshipEvent::for_kind(RelKind::Conflict, None)]);
        assert_eq!(conflict.unresolved, vec!["conflict".to_string()]);

        let broken = RelationshipState::project(&[RelationshipEvent::for_kind(
            RelKind::CommitmentBroken,
            None,
        )]);
        assert_eq!(broken.unresolved, vec!["commitment_broken".to_string()]);

        let kept = RelationshipState::project(&[RelationshipEvent::for_kind(
            RelKind::CommitmentKept,
            None,
        )]);
        assert!(kept.unresolved.is_empty());
    }

    #[test]
    fn a_conflict_lowers_trust_and_a_kept_promise_raises_it() {
        let base = RelationshipState::neutral();
        let after_conflict =
            RelationshipState::project(&[RelationshipEvent::for_kind(RelKind::Conflict, None)]);
        let after_kept = RelationshipState::project(&[RelationshipEvent::for_kind(
            RelKind::CommitmentKept,
            None,
        )]);
        assert!(after_conflict.trust < base.trust);
        assert!(after_kept.trust > base.trust);
        assert!(after_conflict.recent_quality < base.recent_quality);
    }

    #[test]
    fn kinds_round_trip_through_their_wire_names() {
        for kind in [
            RelKind::Interaction,
            RelKind::Disclosure,
            RelKind::Conflict,
            RelKind::Repair,
            RelKind::CommitmentKept,
            RelKind::CommitmentBroken,
        ] {
            assert_eq!(RelKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(RelKind::parse("adversarial"), None);
        assert_eq!(DimDelta::zero().trust, 0.0);
    }
}
