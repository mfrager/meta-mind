//! `mm-library` — the cognitive library: a durable repertoire of reusable ways of
//! thinking.
//!
//! Design §108 / Phase 7 asks for a library that stores, validates and *selects*
//! rather than one that merely holds text. Four mechanisms carry that, each with
//! its own module:
//!
//! * **Typed, versioned entries** ([`entry`], [`doctrine`], [`technique`]). Seven
//!   declarative kinds share one shape and one codec; the kind is a field, not a
//!   separate struct, so the shape cannot drift between them. Identifiers are
//!   path-style and human-diffable, so a library entry reads like a named thing
//!   rather than a database row.
//! * **A verified skill library** ([`skill`]). A skill is a draft until its test
//!   passes; retrieval returns verified skills and nothing else, so the state is
//!   load-bearing rather than decorative.
//! * **Structure-mapped analogy** ([`case_db`]). Two problems correspond when they
//!   use the same *relations*, and the mapping is returned rather than hidden
//!   behind a similarity number.
//! * **Frames and policies** ([`frame`], [`policy`]). Frames compose with the plan's
//!   `Frame_child = Frame_parent ⊕ Δ` and report their gaps instead of inventing
//!   values; policies carry a scope, an activation condition and a typed behaviour
//!   fragment in immutable, parented versions.
//!
//! Three rules hold across the crate:
//!
//! 1. **Nothing probabilistic.** Every number here is fixed arithmetic or a value
//!    the caller supplied. The selector's blend and the case utility are pure
//!    functions with a golden test, because a rank nobody can reproduce is a rank
//!    nobody can trust.
//! 2. **A refusal is typed.** Each module refuses with the field, shape, or state
//!    that failed — see [`error`].
//! 3. **Versions are immutable.** An entry or policy is built, never mutated in
//!    place; a change is a new version that names its parent.
#![forbid(unsafe_code)]

pub mod applicability;
pub mod case_db;
pub mod doctrine;
pub mod entry;
pub mod error;
pub mod experience;
pub mod frame;
pub mod genome;
pub mod manager;
pub mod policy;
pub mod rdf;
pub mod seed;
pub mod skill;
pub mod store;
pub mod technique;
pub mod validate;

pub use applicability::{
    activation_score, applicability, rank, selectable, state_from_json, ApplicabilityScore,
    ApplicabilityState, Ranked,
};
pub use case_db::{
    case_utility, retrieve as retrieve_cases, structural_similarity, Case, CaseElement, CaseHit,
    CaseKind, CaseQuery, Correspondence, CASE_KINDS, MAX_CORRESPONDENCES,
};
pub use doctrine::{anti_pattern, doctrine, heuristic, principle, when};
pub use entry::{
    iri, Declarative, EntryKind, LibraryEntry, ENTRY_KINDS, LIBRARY_BASE, MAX_SLUG_CHARS,
    POLICY_BASE,
};
pub use error::{LibraryError, Result, ShapeViolation};
pub use frame::{Frame, FrameInstance, Script, SlotValue};
pub use manager::{CommitOutcome, ImportReport, LibraryManager, ValidationReport};
pub use policy::{FitnessRecord, Policy, PolicyBehavior, PolicyDelta, Scope};
pub use skill::{
    lexical_score, Skill, SkillHit, SkillTestSpec, Verification, Workflow, VERIFICATIONS,
};
pub use store::{EntryRow, LibraryStore, SkillRow};
pub use technique::{evaluation, pattern, technique};
pub use validate::{duplicates, is_external, orphans, Duplicate, OrphanReport};

// The one shared condition type (phase index decision D1): defined in `mm-core`
// and re-exported here rather than declared a second time.
pub use mm_core::ActivationCondition;
pub use rdf::{content_hash, declarative_from_turtle, declaratives_from_turtle, LIBRARY_GRAPH};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.library";
