//! The `/being` mirror: persistent self as RDF, written from the same state the
//! tables hold.
//!
//! Two rules shape this file.
//!
//! 1. **Only persistent state is mirrored.** Affect appears as an `mm:AffectState`
//!    node with numeric axes and *no status-bearing predicate*, because an impulse
//!    that could be read back as a fact would be affect asserting something.
//! 2. **The mirror is a projection, not a second authority.** It is rewritten from
//!    the in-memory snapshot on every commit, so a triple that is not in the
//!    snapshot cannot survive in the graph.

use std::collections::BTreeMap;

use mm_core::iri;
use mm_core::Ulid;
use mm_store_graph::GraphHandle;
use oxrdf::{GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term};
use serde_json::Value;

use crate::affect::Affect;
use crate::blocks::CoreBlock;
use crate::error::{BeingError, Result};
use crate::goals::{Commitment, Goal};
use crate::identity::Identity;
use crate::motivation::Motivation;
use crate::personality::Personality;
use crate::relationships::RelationshipState;
use crate::resources::ResourceKind;
use crate::resources::ResourceState;
use crate::user_model::Belief;

/// The named graph the being's persistent self is mirrored into.
pub const BEING_GRAPH: &str = "being";

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The whole persistent self, as the mirror sees it.
#[derive(Debug, Clone, Default)]
pub struct BeingSnapshot {
    /// The immutable core.
    pub identity: Option<Identity>,
    /// Core blocks, in insertion order.
    pub blocks: Vec<CoreBlock>,
    /// The personality constitution.
    pub personality: Option<Personality>,
    /// The affect state.
    pub affect: Option<Affect>,
    /// Intrinsic drives.
    pub motivations: Vec<Motivation>,
    /// Beliefs, keyed by the user they are about.
    pub beliefs: Vec<(Ulid, Belief)>,
    /// Relationship projections, keyed by user.
    pub relationships: Vec<(Ulid, RelationshipState)>,
    /// Goals (desire).
    pub goals: Vec<Goal>,
    /// Commitments (intention).
    pub commitments: Vec<Commitment>,
    /// Resource accounts.
    pub resources: Vec<(ResourceKind, f64, String)>,
}

impl BeingSnapshot {
    /// Every quad the mirror should hold, in a deterministic order.
    pub fn to_quads(&self) -> Vec<Quad> {
        let mut quads: Vec<Quad> = Vec::new();
        let Some(identity) = &self.identity else {
            return quads;
        };
        let me = iri::data(&identity.id).into_string();

        // The being and its identity are the same node: one identity, one being, so
        // `mm:hasIdentity` cannot disagree with the node it describes.
        quads.push(q(&me, mm("Being"), Term::from(subject(&me))));
        quads.push(q(&me, mm("Identity"), Term::from(subject(&me))));
        // An IRI, not a string: the identity's name is the node itself, which is
        // what makes `mm:hasIdentity` and `mm:iri` unable to disagree.
        quads.push(q(&me, mm("iri"), Term::from(subject(&me))));
        quads.push(q(&me, mm("hasIdentity"), Term::from(subject(&me))));
        quads.push(q(
            &me,
            mm("created"),
            literal_dt(&identity.created_at.to_rfc3339()),
        ));
        quads.push(q(
            &me,
            mm("selfDescription"),
            literal_str(&identity.self_description),
        ));
        quads.push(q(
            &me,
            mm("version"),
            literal_str(&identity.current_version),
        ));
        for code in &identity.invariants {
            quads.push(q(&me, mm("invariantOf"), literal_str(code)));
        }

        for block in &self.blocks {
            let iri = format!("{me}#block-{}", block.kind.as_str());
            quads.push(q(&iri, mm("CoreBlock"), Term::from(subject(&iri))));
            quads.push(q(&iri, mm("hasIdentity"), Term::from(subject(&me))));
            quads.push(q(&iri, mm("blockKind"), literal_str(block.kind.as_str())));
            quads.push(q(&iri, mm("label"), literal_str(&block.label)));
            quads.push(q(
                &iri,
                mm("contentLength"),
                literal_int(block.len() as i64),
            ));
            quads.push(q(
                &iri,
                mm("limitChars"),
                literal_int(i64::from(block.limit_chars)),
            ));
        }

        if let Some(personality) = &self.personality {
            for (key, value) in &personality.dispositions {
                let iri = format!("{me}#trait-{key}");
                quads.push(q(&iri, mm("PersonalityTrait"), Term::from(subject(&iri))));
                quads.push(q(&iri, mm("hasIdentity"), Term::from(subject(&me))));
                quads.push(q(&iri, mm("traitValue"), literal_double(f64::from(*value))));
            }
            for (key, value) in &personality.values {
                let iri = format!("{me}#value-{key}");
                quads.push(q(&iri, mm("PersonalityValue"), Term::from(subject(&iri))));
                quads.push(q(&iri, mm("hasIdentity"), Term::from(subject(&me))));
                quads.push(q(
                    &iri,
                    mm("valueWeight"),
                    literal_double(f64::from(*value)),
                ));
            }
            for (key, kind) in &personality.constraints {
                let iri = format!("{me}#constraint-{key}");
                quads.push(q(&iri, mm("PersonalityValue"), Term::from(subject(&iri))));
                quads.push(q(&iri, mm("hasIdentity"), Term::from(subject(&me))));
                quads.push(q(&iri, mm("constraintKind"), literal_str(kind.as_str())));
            }
        }

        if let Some(affect) = &self.affect {
            let iri = format!("{me}#affect");
            quads.push(q(&iri, mm("AffectState"), Term::from(subject(&iri))));
            quads.push(q(&iri, mm("hasIdentity"), Term::from(subject(&me))));
            for (axis, value) in affect_axes(affect) {
                quads.push(q(&iri, mm(axis), literal_double(f64::from(value))));
            }
        }

        for motivation in &self.motivations {
            let iri = format!("{me}#motivation-{}", motivation.drive.as_str());
            quads.push(q(&iri, mm("Motivation"), Term::from(subject(&iri))));
            quads.push(q(&iri, mm("hasIdentity"), Term::from(subject(&me))));
            quads.push(q(
                &iri,
                mm("motivationStrength"),
                literal_double(f64::from(motivation.strength)),
            ));
        }

        for (user, belief) in &self.beliefs {
            let id = mm_core::hash_fields(&[&iri::data(user).into_string(), &belief.proposition]);
            let iri = format!("{}#{id}", iri::data(user).into_string());
            quads.push(q(&iri, mm("UserBelief"), Term::from(subject(&iri))));
            quads.push(q(
                &iri,
                mm("heldBy"),
                Term::from(subject(&iri::data(user).into_string())),
            ));
            quads.push(q(
                &iri,
                mm("beliefStatus"),
                literal_str(belief.epistemic_status.as_str()),
            ));
            quads.push(q(
                &iri,
                mm("confidence"),
                literal_double(f64::from(belief.confidence)),
            ));
            quads.push(q(
                &iri,
                mm("about"),
                literal_str(&belief.proposition_hash()),
            ));
            quads.push(q(
                &iri,
                mm("evidenceCount"),
                literal_int(belief.evidence.len() as i64),
            ));
        }

        for (user, state) in &self.relationships {
            let iri = format!("{me}#relationship-{}", mm_core::ulid_string(user));
            quads.push(q(&iri, mm("Relationship"), Term::from(subject(&iri))));
            quads.push(q(
                &iri,
                mm("inRelationshipWith"),
                Term::from(subject(&iri::data(user).into_string())),
            ));
            quads.push(q(
                &iri,
                mm("trusts"),
                literal_double(f64::from(state.trust)),
            ));
            quads.push(q(
                &iri,
                mm("familiarity"),
                literal_double(f64::from(state.familiarity)),
            ));
            quads.push(q(
                &iri,
                mm("unresolvedCount"),
                literal_int(state.unresolved.len() as i64),
            ));
        }

        for goal in &self.goals {
            let iri = iri::data(&goal.id).into_string();
            quads.push(q(&iri, mm("Goal"), Term::from(subject(&iri))));
            quads.push(q(&iri, mm("goalStatus"), literal_str(goal.status.as_str())));
            quads.push(q(
                &iri,
                mm("originatesFrom"),
                literal_str(goal.origin.as_str()),
            ));
            quads.push(q(
                &iri,
                mm("priority"),
                literal_double(f64::from(goal.priority)),
            ));
        }

        for commitment in &self.commitments {
            let iri = iri::data(&commitment.id).into_string();
            quads.push(q(&iri, mm("Commitment"), Term::from(subject(&iri))));
            quads.push(q(&iri, mm("commitmentFor"), Term::from(subject(&iri))));
            quads.push(q(
                &iri,
                mm("madeTo"),
                Term::from(subject(&iri::data(&commitment.made_to).into_string())),
            ));
            quads.push(q(
                &iri,
                mm("commitmentStatus"),
                literal_str(commitment.status.as_str()),
            ));
        }

        for (kind, balance, unit) in &self.resources {
            let iri = format!("{me}#account-{}", kind.as_str());
            quads.push(q(&iri, mm("ResourceAccount"), Term::from(subject(&iri))));
            quads.push(q(&iri, mm("hasIdentity"), Term::from(subject(&me))));
            quads.push(q(&iri, mm("budgetLimit"), literal_double(*balance)));
            quads.push(q(&iri, mm("unit"), literal_str(unit)));
        }

        quads
    }
}

fn affect_axes(affect: &Affect) -> [(&'static str, f32); 7] {
    [
        ("valence", affect.valence),
        ("arousal", affect.arousal),
        ("engagement", affect.engagement),
        ("warmth", affect.warmth),
        ("caution", affect.caution),
        ("curiosity", affect.curiosity),
        ("energy", affect.energy),
    ]
}

/// One triple, rendered canonically so a round trip can be compared by value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BeingTriple {
    /// Rendered subject.
    pub subject: String,
    /// Rendered predicate.
    pub predicate: String,
    /// Rendered object.
    pub object: String,
}

impl BeingTriple {
    /// Render one quad.
    pub fn from_quad(quad: &Quad) -> Self {
        BeingTriple {
            subject: render_subject(&quad.subject),
            predicate: format!("<{}>", quad.predicate.as_str()),
            object: render_term(&quad.object),
        }
    }
}

/// The read half of the mirror: rebuild the triple set from SPARQL rows.
pub trait FromRdf: Sized {
    /// Rebuild from `?s ?p ?o` rows, sorted so two reads compare by value.
    fn from_rows(rows: &[Value]) -> Self;
}

impl FromRdf for Vec<BeingTriple> {
    fn from_rows(rows: &[Value]) -> Self {
        let mut triples: Vec<BeingTriple> = rows
            .iter()
            .filter_map(|row| {
                Some(BeingTriple {
                    subject: render_subject_value(row.get("s")?),
                    predicate: render_subject_value(row.get("p")?),
                    object: render_term_value(row.get("o")?),
                })
            })
            .collect();
        triples.sort();
        triples
    }
}

/// The write half of the mirror.
pub trait ToRdf {
    /// The quads this value contributes.
    fn to_quads(&self) -> Vec<Quad>;
}

/// Rewrite `/being` from a snapshot, returning how many quads it holds.
///
/// The graph is cleared first: a node that vanished from the snapshot must not
/// survive in the projection, or the mirror would be a second, stale authority.
pub async fn mirror(handle: &GraphHandle, snapshot: &BeingSnapshot) -> Result<usize> {
    handle.clear_graph(BEING_GRAPH).await?;
    let quads = snapshot.to_quads();
    for quad in &quads {
        handle.insert(quad.clone()).await?;
    }
    Ok(quads.len())
}

/// A deterministic hash of a quad set, independent of insertion order.
pub fn quads_hash(quads: &[Quad]) -> String {
    let mut rendered: Vec<String> = quads
        .iter()
        .map(|quad| {
            let triple = BeingTriple::from_quad(quad);
            format!("{} {} {}", triple.subject, triple.predicate, triple.object)
        })
        .collect();
    rendered.sort();
    let borrowed: Vec<&str> = rendered.iter().map(String::as_str).collect();
    mm_core::hash_fields(&borrowed)
}

/// The same hash, computed from a graph read.
pub fn triples_hash(triples: &[BeingTriple]) -> String {
    let mut rendered: Vec<String> = triples
        .iter()
        .map(|t| format!("{} {} {}", t.subject, t.predicate, t.object))
        .collect();
    rendered.sort();
    let borrowed: Vec<&str> = rendered.iter().map(String::as_str).collect();
    mm_core::hash_fields(&borrowed)
}

// ------------------------------------------------------------------ helpers ----

fn graph_name() -> GraphName {
    GraphName::NamedNode(named(iri::graph(BEING_GRAPH)))
}

fn named(iri: impl Into<String>) -> NamedNode {
    NamedNode::new_unchecked(iri.into())
}

fn mm(local: &str) -> NamedNode {
    named(format!("{}{local}", iri::MM))
}

fn subject(iri: &str) -> NamedOrBlankNode {
    NamedOrBlankNode::NamedNode(named(iri))
}

fn literal_str(value: &str) -> Term {
    Term::Literal(Literal::new_simple_literal(value))
}

fn literal_int(value: i64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        value.to_string(),
        named(format!("{XSD}integer")),
    ))
}

fn literal_double(value: f64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        value.to_string(),
        named(format!("{XSD}double")),
    ))
}

fn literal_dt(value: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(
        canonical_datetime(value),
        named(format!("{XSD}dateTime")),
    ))
}

/// Write an RFC3339 instant in the store's canonical `xsd:dateTime` form.
///
/// A triplestore normalises a `xsd:dateTime` literal on the way in — trailing
/// zeros are dropped from the fractional second, and a zero fraction disappears —
/// so a value written verbatim would read back *shorter* than it was written for
/// the one nanosecond in ten that ends in `0`. Rendering the canonical form here
/// keeps a mirror round-trip exact rather than fragile.
fn canonical_datetime(value: &str) -> String {
    let Some(dot) = value.find('.') else {
        return value.to_string();
    };
    let Some(offset) = value[dot..].find(['Z', '+', '-']) else {
        return value.to_string();
    };
    let fraction = &value[dot + 1..dot + offset];
    let trimmed = fraction.trim_end_matches('0');
    if trimmed.len() == fraction.len() {
        return value.to_string();
    }
    if trimmed.is_empty() {
        format!("{}{}", &value[..dot], &value[dot + offset..])
    } else {
        format!("{}.{}{}", &value[..dot], trimmed, &value[dot + offset..])
    }
}

fn q(subject_iri: &str, predicate: NamedNode, object: Term) -> Quad {
    Quad::new(subject(subject_iri), predicate, object, graph_name())
}

fn render_subject(subject: &NamedOrBlankNode) -> String {
    match subject {
        NamedOrBlankNode::NamedNode(node) => format!("<{}>", node.as_str()),
        NamedOrBlankNode::BlankNode(node) => format!("_:{}", node.as_str()),
    }
}

fn render_term(term: &Term) -> String {
    match term {
        Term::NamedNode(node) => format!("<{}>", node.as_str()),
        Term::BlankNode(node) => format!("_:{}", node.as_str()),
        Term::Literal(literal) => render_literal(
            literal.value(),
            Some(literal.datatype().as_str()),
            literal.language(),
        ),
    }
}

fn render_literal(value: &str, datatype: Option<&str>, language: Option<&str>) -> String {
    if let Some(language) = language {
        return format!("\"{value}\"@{language}");
    }
    match datatype {
        None => format!("\"{value}\""),
        Some(RDF_TYPE) => format!("\"{value}\""),
        Some(dt) if dt == format!("{XSD}string") => format!("\"{value}\""),
        Some(dt) => format!("\"{value}\"^^<{dt}>"),
    }
}

fn render_subject_value(value: &Value) -> String {
    match value {
        Value::String(s) if s.starts_with("_:") => s.clone(),
        Value::String(s) if looks_like_iri(s) => format!("<{s}>"),
        Value::String(s) => format!("\"{s}\""),
        other => format!("{other}"),
    }
}

fn render_term_value(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let text = map.get("value").and_then(Value::as_str).unwrap_or_default();
            let datatype = map.get("datatype").and_then(Value::as_str);
            let language = map.get("language").and_then(Value::as_str);
            render_literal(text, datatype, language)
        }
        other => render_subject_value(other),
    }
}

fn looks_like_iri(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

/// The account map the snapshot needs, without exposing `Account` here twice.
pub fn accounts(state: &ResourceState) -> Vec<(ResourceKind, f64, String)> {
    state
        .accounts
        .iter()
        .map(|(kind, account)| (*kind, account.balance, account.unit.clone()))
        .collect()
}

/// Group personality dispositions without touching private fields.
pub fn dispositions(personality: &Personality) -> BTreeMap<String, f32> {
    personality.dispositions.clone()
}

/// The mirror refuses to run without an identity: an empty graph would read as
/// "this being has no self", which is a different claim from "not yet created".
pub fn require_identity(snapshot: &BeingSnapshot) -> Result<()> {
    if snapshot.identity.is_none() {
        return Err(BeingError::Config(
            "the /being mirror needs an identity; open the facade first".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Timestamp;
    use mm_store_graph::GraphStore;

    fn snapshot() -> BeingSnapshot {
        let id = Ulid::from_parts(1_700_000_000_000, 7);
        let identity = Identity::new(id, Timestamp::now(), "I am Metamind");
        BeingSnapshot {
            identity: Some(identity),
            resources: vec![(ResourceKind::Time, 42.0, "ms".to_string())],
            ..BeingSnapshot::default()
        }
    }

    #[test]
    fn a_snapshot_renders_deterministically() {
        // The same snapshot renders to the same quads on every call; building a
        // second snapshot would move its `created` timestamp, so render one twice.
        let snap = snapshot();
        let a = snap.to_quads();
        let b = snap.to_quads();
        assert_eq!(a, b);
        assert!(!a.is_empty());
        assert_eq!(quads_hash(&a), quads_hash(&b));
    }

    #[test]
    fn a_datetime_literal_is_written_canonically() {
        // Trailing zeros come off the fraction; an all-zero fraction disappears.
        assert_eq!(
            canonical_datetime("2026-10-08T20:58:15.535040180Z"),
            "2026-10-08T20:58:15.53504018Z"
        );
        assert_eq!(
            canonical_datetime("2026-10-08T20:58:15.500000000Z"),
            "2026-10-08T20:58:15.5Z"
        );
        assert_eq!(
            canonical_datetime("2026-10-08T20:58:15.000000000Z"),
            "2026-10-08T20:58:15Z"
        );
        // An already-canonical value, and one with no fraction, are untouched.
        assert_eq!(
            canonical_datetime("2026-10-08T20:58:15.535040181Z"),
            "2026-10-08T20:58:15.535040181Z"
        );
        assert_eq!(
            canonical_datetime("2026-10-08T20:58:15Z"),
            "2026-10-08T20:58:15Z"
        );
        assert_eq!(
            canonical_datetime("2026-10-08T20:58:15.535040180+02:00"),
            "2026-10-08T20:58:15.53504018+02:00"
        );
    }

    #[test]
    fn affect_axes_are_numeric_and_carry_no_status() {
        let mut snap = snapshot();
        snap.affect = Some(Affect::neutral());
        let quads = snap.to_quads();
        let affect: Vec<&Quad> = quads
            .iter()
            .filter(|quad| {
                quad.subject
                    == subject(&format!(
                        "{}#affect",
                        iri::data(&snap.identity.as_ref().unwrap().id).into_string()
                    ))
            })
            .collect();
        assert!(!affect.is_empty());
        for quad in &affect {
            assert!(
                !quad.predicate.as_str().contains("status"),
                "affect must not carry a status-bearing predicate"
            );
        }
    }

    #[tokio::test]
    async fn the_mirror_round_trips_exactly_through_the_graph() {
        let _dir = tempfile::tempdir().unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("mm-shapes.ttl");
        let graph = GraphStore::in_memory(&shapes).await.unwrap();
        let snap = snapshot();
        let expected = snap.to_quads();
        let written = mirror(graph.handle(), &snap).await.unwrap();
        assert_eq!(written, expected.len());

        let rows = graph.triples(BEING_GRAPH).await.unwrap();
        let rebuilt = <Vec<BeingTriple> as FromRdf>::from_rows(&rows);

        let mut source: Vec<BeingTriple> = expected.iter().map(BeingTriple::from_quad).collect();
        source.sort();
        assert_eq!(
            rebuilt, source,
            "the mirror must not lose or invent a triple"
        );
        assert_eq!(triples_hash(&rebuilt), quads_hash(&expected));
        graph.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn insertion_order_does_not_change_the_hash() {
        let _dir = tempfile::tempdir().unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("mm-shapes.ttl");
        let graph = GraphStore::in_memory(&shapes).await.unwrap();
        let snap = snapshot();
        graph.handle().clear_graph(BEING_GRAPH).await.unwrap();
        for quad in snap.to_quads() {
            graph.handle().insert(quad).await.unwrap();
        }
        let forward = graph.canonical_hash(BEING_GRAPH).await.unwrap();

        graph.handle().clear_graph(BEING_GRAPH).await.unwrap();
        for quad in snap.to_quads().into_iter().rev() {
            graph.handle().insert(quad).await.unwrap();
        }
        let reversed = graph.canonical_hash(BEING_GRAPH).await.unwrap();
        assert_eq!(forward, reversed, "a canonical hash is order-independent");
        graph.shutdown().await.unwrap();
    }

    #[test]
    fn a_mirror_without_an_identity_is_refused() {
        let err = require_identity(&BeingSnapshot::default()).unwrap_err();
        assert_eq!(err.kind(), "config");
    }
}
