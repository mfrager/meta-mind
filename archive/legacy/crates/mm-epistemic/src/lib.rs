//! `mm-epistemic` — epistemic discipline: what the being thinks, kept apart from
//! what the world contains.
//!
//! Design §13 / Phase 6 asks for a layer in which **no silent promotion** is
//! possible. Five mechanisms carry that:
//!
//! * A **status lattice** ([`status`]) where only `OBSERVED` and `VERIFIED`
//!   describe the world, and a **deterministic guard** ([`guard::can_promote`])
//!   that is a pure function of `(from, to, evidence)` and takes no model output.
//! * A **justification graph** ([`justification`]) whose every derived belief
//!   names the assumptions it rests on, so retraction is dependency-directed and
//!   takes the whole support set at once.
//! * **Contradictions as objects** ([`contradiction`]): a conflict is an explicit
//!   record with both sides, never a silent edit.
//! * A **validation barrier** ([`validate`]) that is the only writer permitted to
//!   touch `/world`, and admits only `OBSERVED`/`VERIFIED` claims with evidence.
//! * **Provenance for every change** ([`prov`]): a PROV-O activity/agent pair plus
//!   a reified record of the status the claim held before.
//!
//! Three rules hold across the crate:
//!
//! 1. **The guard never consults a model.** Probabilities and reliabilities enter
//!    from the caller; nothing here computes one.
//! 2. **A conflict is never resolved silently.** Detection writes a record; the
//!    remediation is a later, logged action.
//! 3. **`/world` has one door.** The mirror splits `OBSERVED`/`VERIFIED` claims
//!    into `/world` and everything else into `/epistemic`, and the split is a
//!    function of the status rather than of the caller's intent.
#![forbid(unsafe_code)]

pub mod assumption;
pub mod claim;
pub mod contradiction;
pub mod engine;
pub mod error;
pub mod guard;
pub mod justification;
pub mod outcome;
pub mod prediction;
pub mod proposition;
pub mod prov;
pub mod rdf;
pub mod status;
pub mod store;
pub mod validate;

pub use assumption::{epistemic_budget, verification_priority, Assumption, AssumptionLedger};
pub use claim::{
    Claim, ClaimKind, Evidence, EvidenceKind, Hypothesis, Inference, Observation, CLAIM_KINDS,
    EVIDENCE_KINDS, MAX_IRI_CHARS,
};
pub use contradiction::{
    contradiction_id, ConflictIndex, Contradiction, ContradictionDetector, ContradictionStatus,
    FormalBackend, FormalCheck, IndexedContradictionDetector,
};
pub use engine::EpistemicEngine;
pub use error::{EpistemicError, PromotionDenied, Result, WorldRefusal};
pub use guard::{
    can_promote, observation_requirement, PromotionGuard, FORBIDDEN, PHASE_FOUR_INVARIANT,
};
pub use justification::{Cascade, DepKind, Justification, JustificationGraph, DEP_KINDS};
pub use outcome::{Outcome, OutcomeStatus, OUTCOME_STATUSES};
pub use prediction::{Prediction, PredictionOutcome};
pub use proposition::{Proposition, RiskLevel, RDF_TYPE, XSD};
pub use prov::{StatusChange, PROV};
pub use rdf::{EpistemicSnapshot, EpistemicTriple, EPISTEMIC_GRAPH, PROVENANCE_GRAPH, WORLD_GRAPH};
pub use status::{EpistemicStatus, EPISTEMIC_STATUSES};
pub use store::{EpistemicStore, SqliteEpistemicStore, Transition};
pub use validate::{data_tests, Finding, Severity, ValidationBarrier};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.epistemic";
