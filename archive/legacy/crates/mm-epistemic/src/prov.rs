//! PROV-O provenance: every change in the epistemic layer names its activity,
//! its source, and the value that was replaced.
//!
//! The plan borrows the W3C PROV-O model (`prov:Entity`, `prov:Activity`,
//! `prov:Agent`) and asks for RDF-star annotations on individual changes (the
//! PROV-STAR idea). This workspace's store does not promise RDF-star, so the
//! change annotation is realized as a **reification fallback**: a
//! [`StatusChange`] is a `prov:Activity` that carries `mm:priorStatus`,
//! `mm:newStatus`, and `prov:startedAtTime`. The fallback is the plan's own, and it
//! is applied uniformly — a reader never has to understand two encodings.
//!
//! This module holds the vocabulary and the small pure builders. It writes no
//! graph: the mirror in [`crate::rdf`] turns these records into quads.

use mm_core::{Timestamp, Ulid};
use serde::{Deserialize, Serialize};

use crate::status::EpistemicStatus;

/// The `prov:` namespace.
pub const PROV: &str = "http://www.w3.org/ns/prov#";
/// The `xsd:` namespace.
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// One recorded change, in PROV-O terms.
///
/// It is an *activity*: it has an agent (the source that prompted it), it used the
/// claim as an entity, and it generated the claim's new state. `prior_status` and
/// `new_status` are the change annotation the RDF-star form would have carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusChange {
    /// The activity's ULID.
    pub id: Ulid,
    /// The claim (entity) that changed.
    pub subject: Ulid,
    /// The status before the change, when there was one.
    pub prior_status: Option<EpistemicStatus>,
    /// The status after the change, when there is one.
    pub new_status: Option<EpistemicStatus>,
    /// Why the change happened.
    pub reason: String,
    /// When it happened.
    pub at: Timestamp,
    /// The agent (source) that prompted it, when one did.
    pub agent: Option<String>,
}

impl StatusChange {
    /// A change of `subject` to `new_status`.
    pub fn new(id: Ulid, subject: Ulid, new_status: Option<EpistemicStatus>, reason: &str) -> Self {
        StatusChange {
            id,
            subject,
            prior_status: None,
            new_status,
            reason: reason.to_string(),
            at: Timestamp::now(),
            agent: None,
        }
    }

    /// The status the subject held before.
    pub fn with_prior_status(mut self, prior: EpistemicStatus) -> Self {
        self.prior_status = Some(prior);
        self
    }

    /// The agent that prompted the change.
    pub fn attributed_to(mut self, agent: impl Into<String>) -> Self {
        self.agent = Some(agent.into());
        self
    }

    /// The PROV-O type an activity carries.
    pub fn activity_type() -> &'static str {
        "prov:Activity"
    }

    /// The PROV-O type an agent carries.
    pub fn agent_type() -> &'static str {
        "prov:Agent"
    }
}

/// The `prov:` IRI of a local name.
pub fn prov_iri(local: &str) -> String {
    format!("{PROV}{local}")
}

/// The `mm:` IRI of a local name (the local vocabulary this crate reuses for the
/// reified change annotation).
pub fn mm_iri(local: &str) -> String {
    format!("{}{local}", mm_core::iri::MM)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulid::Ulid as UlidType;

    fn id(n: u128) -> Ulid {
        UlidType::from_parts(1_700_000_000_000, n)
    }

    #[test]
    fn a_change_names_both_values_and_its_agent() {
        let change = StatusChange::new(id(1), id(2), Some(EpistemicStatus::Verified), "checked")
            .with_prior_status(EpistemicStatus::Inferred)
            .attributed_to("https://metamind.dev/tool/rustc");
        assert_eq!(change.prior_status, Some(EpistemicStatus::Inferred));
        assert_eq!(change.new_status, Some(EpistemicStatus::Verified));
        assert_eq!(
            change.agent.as_deref(),
            Some("https://metamind.dev/tool/rustc")
        );
        assert_eq!(StatusChange::activity_type(), "prov:Activity");
        assert_eq!(StatusChange::agent_type(), "prov:Agent");
    }

    #[test]
    fn the_namespaces_are_the_canonical_ones() {
        assert_eq!(PROV, "http://www.w3.org/ns/prov#");
        assert_eq!(
            prov_iri("wasDerivedFrom"),
            "http://www.w3.org/ns/prov#wasDerivedFrom"
        );
        assert!(mm_iri("priorStatus").starts_with("https://metamind.dev/ontology#"));
    }
}
