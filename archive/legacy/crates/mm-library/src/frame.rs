//! Conceptual frames and multi-frame composition.
//!
//! A frame is what a situation *is*: the roles that participate, the slots that
//! must be filled, the actions that are typical and the ways it typically fails.
//! Frames compose, and the composition is the plan's `Frame_child = Frame_parent ⊕
//! Δ`: a later frame fills slots an earlier one left [`SlotValue::Unknown`], and
//! never overwrites a value already known.
//!
//! `compose` reports the slots that remain unknown rather than inventing them.
//! That is the whole reason [`FrameInstance`] carries a `missing` list: a frame
//! that quietly guessed at a slot would make the gap invisible exactly when it
//! matters most.

use std::collections::BTreeMap;

use mm_core::{NamedNode, Ulid};
use serde::{Deserialize, Serialize};

use crate::entry::{iri, EntryKind, LibraryEntry};
use crate::error::{LibraryError, Result};

/// A slot's value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum SlotValue {
    /// Filled by evidence or by a caller.
    Known(String),
    /// Named, but not filled.
    Unknown,
    /// Filled by inference, which is weaker than a known value.
    Inferred(String),
}

impl SlotValue {
    /// The text, when there is one.
    pub fn text(&self) -> Option<&str> {
        match self {
            SlotValue::Known(text) | SlotValue::Inferred(text) => Some(text),
            SlotValue::Unknown => None,
        }
    }

    /// True when the slot is still open.
    pub fn is_unknown(&self) -> bool {
        matches!(self, SlotValue::Unknown)
    }
}

/// A frame: roles, slots, typical actions, failure modes and scripts.
///
/// Not `Serialize`: a frame persists as Turtle (see [`crate::rdf`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// The frame's IRI.
    pub iri: NamedNode,
    /// The slug.
    pub slug: String,
    /// The parent frame, when it is a refinement.
    pub parent: Option<NamedNode>,
    /// The roles that participate.
    pub roles: BTreeMap<String, String>,
    /// The slots the frame expects.
    pub slots: Vec<String>,
    /// What is typically done.
    pub typical_actions: Vec<String>,
    /// How it typically fails.
    pub failure_modes: Vec<String>,
    /// Scripts: ordered steps for a recurring situation.
    pub scripts: Vec<Vec<String>>,
    /// The domains the frame belongs to.
    pub domain: Vec<String>,
    /// The title.
    pub title: String,
    /// What the frame is for.
    pub purpose: String,
    /// The version.
    pub version: u32,
}

/// One script: steps that recur together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Script {
    /// The frame the script belongs to.
    pub frame: String,
    /// The steps, in order.
    pub steps: Vec<String>,
}

impl Frame {
    /// Build a frame, refusing one with no slots or no roles.
    pub fn new(slug: &str, title: &str, purpose: &str) -> Result<Self> {
        iri::check_slug(slug)?;
        let frame = Frame {
            iri: iri::frame(slug),
            slug: slug.to_string(),
            parent: None,
            roles: BTreeMap::new(),
            slots: Vec::new(),
            typical_actions: Vec::new(),
            failure_modes: Vec::new(),
            scripts: Vec::new(),
            domain: Vec::new(),
            title: title.trim().to_string(),
            purpose: purpose.trim().to_string(),
            version: 1,
        };
        Ok(frame)
    }

    /// Refine a parent frame.
    pub fn with_parent(mut self, parent: NamedNode) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Add a role.
    pub fn with_role(mut self, role: &str, description: &str) -> Self {
        self.roles.insert(role.to_string(), description.to_string());
        self
    }

    /// Add a slot.
    pub fn with_slot(mut self, slot: &str) -> Self {
        if !self.slots.iter().any(|s| s == slot) {
            self.slots.push(slot.to_string());
        }
        self
    }

    /// Add a typical action.
    pub fn with_action(mut self, action: &str) -> Self {
        self.typical_actions.push(action.to_string());
        self
    }

    /// Add a domain.
    pub fn with_domain(mut self, domain: &str) -> Self {
        self.domain.push(domain.to_string());
        self
    }

    /// Add a failure mode.
    pub fn with_failure_mode(mut self, mode: &str) -> Self {
        self.failure_modes.push(mode.to_string());
        self
    }
}

impl LibraryEntry for Frame {
    fn kind(&self) -> EntryKind {
        EntryKind::Frame
    }

    fn version_iri(&self) -> NamedNode {
        self.iri.clone()
    }

    fn head_iri(&self) -> NamedNode {
        iri::frame(&self.slug)
    }

    fn version(&self) -> u32 {
        self.version
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn referenced_iris(&self) -> Vec<NamedNode> {
        self.parent.iter().cloned().collect()
    }

    fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            return Err(LibraryError::validation("title", "must not be empty"));
        }
        if self.purpose.trim().is_empty() {
            return Err(LibraryError::validation("purpose", "must not be empty"));
        }
        if self.slots.is_empty() {
            return Err(LibraryError::validation(
                "frameSlot",
                "a frame that expects nothing cannot be filled in",
            ));
        }
        if self.roles.is_empty() {
            return Err(LibraryError::validation(
                "role",
                "a frame with no roles has nobody to be about",
            ));
        }
        if self.domain.is_empty() {
            return Err(LibraryError::validation("domain", "must not be empty"));
        }
        Ok(())
    }

    fn to_turtle(&self) -> Result<String> {
        crate::rdf::frame_turtle(self)
    }
}

/// A frame filled in for one episode.
///
/// Not `Serialize`: an instance persists as Turtle and in `frame_instances`.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInstance {
    /// The instance's ULID.
    pub ulid: Ulid,
    /// The frames it was composed from, in order.
    pub parents: Vec<NamedNode>,
    /// The episode it belongs to.
    pub episode: Ulid,
    /// The slots and their values.
    pub slots: BTreeMap<String, SlotValue>,
    /// The slots that remain unknown.
    pub missing: Vec<String>,
    /// The title, so the instance satisfies the entry shapes.
    pub title: String,
    /// The purpose.
    pub purpose: String,
}

impl FrameInstance {
    /// Compose frames into one instance.
    ///
    /// The merge is left-to-right and *monotone*: a slot already [`SlotValue::Known`]
    /// is never overwritten by a later frame, and a frame may only fill a slot that
    /// is still open. Slots nobody filled are reported in [`FrameInstance::missing`]
    /// rather than guessed at.
    pub fn compose(
        ulid: Ulid,
        episode: Ulid,
        frames: &[Frame],
        seed: BTreeMap<String, SlotValue>,
    ) -> Result<Self> {
        if frames.is_empty() {
            return Err(LibraryError::validation(
                "frames",
                "composition needs at least one frame",
            ));
        }
        let mut slots = seed;
        for frame in frames {
            for slot in &frame.slots {
                slots.entry(slot.clone()).or_insert(SlotValue::Unknown);
            }
        }
        let mut missing: Vec<String> = slots
            .iter()
            .filter(|(_, value)| value.is_unknown())
            .map(|(slot, _)| slot.clone())
            .collect();
        missing.sort();
        Ok(FrameInstance {
            ulid,
            parents: frames.iter().map(|frame| frame.iri.clone()).collect(),
            episode,
            slots,
            missing,
            title: format!(
                "Frame instance ({})",
                frames
                    .iter()
                    .map(|frame| frame.slug.as_str())
                    .collect::<Vec<_>>()
                    .join("+")
            ),
            purpose: frames
                .iter()
                .map(|frame| frame.purpose.as_str())
                .collect::<Vec<_>>()
                .join("; "),
        })
    }

    /// The value of a slot.
    pub fn get(&self, slot: &str) -> Option<&SlotValue> {
        self.slots.get(slot)
    }
}

impl LibraryEntry for FrameInstance {
    fn kind(&self) -> EntryKind {
        EntryKind::FrameInstance
    }

    fn version_iri(&self) -> NamedNode {
        iri::instance(&self.ulid)
    }

    fn head_iri(&self) -> NamedNode {
        iri::instance(&self.ulid)
    }

    fn version(&self) -> u32 {
        1
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn referenced_iris(&self) -> Vec<NamedNode> {
        self.parents.clone()
    }

    fn validate(&self) -> Result<()> {
        if self.parents.is_empty() {
            return Err(LibraryError::validation(
                "parentFrame",
                "an instance names the frames it came from",
            ));
        }
        Ok(())
    }

    fn to_turtle(&self) -> Result<String> {
        crate::rdf::frame_instance_turtle(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Frame {
        Frame::new("problem_solving", "Problem solving", "Solve it")
            .unwrap()
            .with_role("solver", "who works")
            .with_slot("goal")
            .with_slot("obstacle")
            .with_domain("problem_solving")
    }

    fn refined() -> Frame {
        Frame::new("debugging", "Debugging", "Find the cause")
            .unwrap()
            .with_parent(iri::frame("problem_solving"))
            .with_role("observer", "who looks")
            .with_slot("obstacle")
            .with_slot("reproduction")
            .with_domain("software")
    }

    #[test]
    fn a_frame_needs_slots_roles_and_a_domain() {
        assert!(Frame::new("f", "F", "P").unwrap().validate().is_err());
        assert!(base().validate().is_ok());
    }

    #[test]
    fn composition_unions_the_slots_and_reports_the_gaps() {
        let instance = FrameInstance::compose(
            Ulid::from_parts(1_700_000_000_000, 1),
            Ulid::from_parts(1_700_000_000_000, 2),
            &[base(), refined()],
            BTreeMap::new(),
        )
        .unwrap();
        let slots: Vec<&str> = instance.slots.keys().map(String::as_str).collect();
        assert_eq!(slots, vec!["goal", "obstacle", "reproduction"]);
        assert_eq!(
            instance.missing,
            vec![
                "goal".to_string(),
                "obstacle".to_string(),
                "reproduction".to_string()
            ]
        );
        assert_eq!(instance.parents.len(), 2);
    }

    #[test]
    fn a_known_slot_is_never_overwritten_by_a_later_frame() {
        let mut seed = BTreeMap::new();
        seed.insert(
            "obstacle".to_string(),
            SlotValue::Known("stale lockfile".into()),
        );
        let instance = FrameInstance::compose(
            Ulid::from_parts(1_700_000_000_000, 1),
            Ulid::from_parts(1_700_000_000_000, 2),
            &[base(), refined()],
            seed,
        )
        .unwrap();
        assert_eq!(
            instance.get("obstacle"),
            Some(&SlotValue::Known("stale lockfile".into()))
        );
        assert!(!instance.missing.contains(&"obstacle".to_string()));
    }

    #[test]
    fn composing_nothing_is_refused() {
        assert!(FrameInstance::compose(
            Ulid::from_parts(1, 1),
            Ulid::from_parts(1, 2),
            &[],
            BTreeMap::new()
        )
        .is_err());
    }
}
