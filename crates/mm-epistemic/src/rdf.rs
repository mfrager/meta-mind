//! The RDF mirrors: `/epistemic`, `/world`, and `/provenance`.
//!
//! Three graphs, one projection each, all rewritten from the same tabulated state
//! on every mutation. The rule the separation exists to enforce is visible in the
//! code: [`EPISTEMIC_GRAPH`] carries every claim whatever its status, while
//! [`WORLD_GRAPH`] is written only through [`crate::validate::ValidationBarrier`]
//! and therefore holds only `OBSERVED`/`VERIFIED`.
//!
//! Provenance is PROV-O, with the RDF-star change annotation of plan §4.4 realized
//! as a **reification fallback**: a `mm:StatusChange` node carries the prior
//! status, the new status, and the time. The vendored store does not promise
//! RDF-star, and the plan itself asks for the fallback when a reader lacks it; the
//! fallback is what is written here, and it is documented rather than silent.
//!
//! Instance IRIs are `https://metamind.dev/data/{ulid}` via [`mm_core::iri`], and
//! datetimes are written in the canonical form the store normalizes to.

use std::collections::BTreeSet;

use mm_core::iri;
use mm_core::Ulid;
use mm_store_graph::GraphHandle;
use oxrdf::{GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::assumption::Assumption;
use crate::claim::{Claim, Evidence};
use crate::contradiction::Contradiction;
use crate::error::Result;
use crate::justification::Justification;
use crate::prediction::Prediction;
use crate::store::Transition;

/// The graph holding every epistemic object, whatever its status.
pub const EPISTEMIC_GRAPH: &str = "epistemic";
/// The graph holding only `OBSERVED`/`VERIFIED` claims.
pub const WORLD_GRAPH: &str = "world";
/// The graph holding PROV-O and the status-change records.
pub const PROVENANCE_GRAPH: &str = "provenance";

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const PROV: &str = "http://www.w3.org/ns/prov#";

/// Everything the `/epistemic` mirror projects.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EpistemicSnapshot {
    /// Every claim.
    pub claims: Vec<Claim>,
    /// Every evidence row, with the claim it supports.
    pub evidence: Vec<(Ulid, Evidence)>,
    /// Every assumption.
    pub assumptions: Vec<Assumption>,
    /// Every prediction.
    pub predictions: Vec<Prediction>,
    /// Every contradiction.
    pub contradictions: Vec<Contradiction>,
    /// Every justification edge.
    pub justifications: Vec<Justification>,
    /// Every recorded transition, for `/provenance`.
    pub transitions: Vec<Transition>,
}

impl EpistemicSnapshot {
    /// The claims that may enter `/world`.
    pub fn admissible(&self) -> Vec<&Claim> {
        self.claims
            .iter()
            .filter(|claim| claim.is_world_admissible())
            .collect()
    }

    /// Every quad the `/epistemic` mirror should hold.
    pub fn to_quads(&self) -> Vec<Quad> {
        let mut quads = Vec::new();
        for claim in &self.claims {
            quads.extend(claim_quads(claim));
        }
        for (claim, evidence) in &self.evidence {
            quads.extend(evidence_quads(claim, evidence));
        }
        for assumption in &self.assumptions {
            quads.extend(assumption_quads(assumption));
        }
        for prediction in &self.predictions {
            quads.extend(prediction_quads(prediction));
        }
        for contradiction in &self.contradictions {
            quads.extend(contradiction_quads(contradiction));
        }
        for justification in &self.justifications {
            for antecedent in &justification.antecedents {
                quads.extend(dependency_quads(
                    &justification.consequent,
                    antecedent,
                    justification.kind.as_str(),
                    justification.criticality,
                ));
            }
        }
        dedup(quads)
    }

    /// How many quads the mirror should hold.
    pub fn triple_count(&self) -> usize {
        self.to_quads().len()
    }

    /// Every quad the `/provenance` mirror should hold.
    pub fn provenance_to_quads(&self) -> Vec<Quad> {
        let mut quads = Vec::new();
        for transition in &self.transitions {
            quads.extend(transition_quads(transition));
        }
        dedup(quads)
    }
}

/// The quads one claim contributes to `/epistemic` (and, when admissible,
/// `/world`).
pub fn claim_quads(claim: &Claim) -> Vec<Quad> {
    let graph = EPISTEMIC_GRAPH;
    let node = iri::data(&claim.id).into_string();
    let mut quads = vec![
        q(graph, &node, mm("Claim"), subject(&node)),
        q(graph, &node, mm(claim.kind.rdf_class()), subject(&node)),
        q(
            graph,
            &node,
            mm("status"),
            literal_str(claim.status.as_str()),
        ),
        q(
            graph,
            &node,
            mm("confidence"),
            literal_double(f64::from(claim.confidence)),
        ),
        // The proposition itself, so the graph answers "what does this claim say".
        Quad::new(
            NamedOrBlankNode::NamedNode(named(&node)),
            claim.proposition.predicate.clone(),
            claim.proposition.object.clone(),
            GraphName::NamedNode(named(iri::graph(graph))),
        ),
    ];
    for evidence in &claim.evidence {
        quads.push(q(
            graph,
            &node,
            mm("evidenceFor"),
            subject(&iri::data(evidence).into_string()),
        ));
    }
    if let Some(from) = claim.valid_from {
        quads.push(q(
            graph,
            &node,
            mm("validFrom"),
            literal_dt(&from.to_rfc3339()),
        ));
    }
    if let Some(until) = claim.valid_until {
        quads.push(q(
            graph,
            &node,
            mm("validUntil"),
            literal_dt(&until.to_rfc3339()),
        ));
    }
    quads
}

/// The quads one evidence row contributes.
pub fn evidence_quads(claim: &Ulid, evidence: &Evidence) -> Vec<Quad> {
    let graph = EPISTEMIC_GRAPH;
    let node = iri::data(&evidence.id).into_string();
    let mut quads = vec![q(graph, &node, mm("Evidence"), subject(&node))];
    quads.push(q(
        graph,
        &node,
        mm("supports"),
        subject(&iri::data(claim).into_string()),
    ));
    quads.push(q(
        graph,
        &node,
        mm("evidenceKind"),
        literal_str(evidence.kind.as_str()),
    ));
    quads.push(q(
        graph,
        &node,
        mm("confidence"),
        literal_double(f64::from(evidence.reliability)),
    ));
    if let Some(source) = &evidence.source_uri {
        quads.push(q(graph, &node, mm("source"), subject(source.as_str())));
        quads.push(q(
            graph,
            &node,
            named(format!("{PROV}wasAttributedTo")),
            subject(source.as_str()),
        ));
    }
    quads
}

/// The quads one assumption contributes.
pub fn assumption_quads(assumption: &Assumption) -> Vec<Quad> {
    let graph = EPISTEMIC_GRAPH;
    let node = iri::data(&assumption.id).into_string();
    let mut quads = vec![
        q(graph, &node, mm("Assumption"), subject(&node)),
        q(
            graph,
            &node,
            mm("status"),
            literal_str(assumption.status.as_str()),
        ),
        q(
            graph,
            &node,
            mm("confidence"),
            literal_double(f64::from(assumption.confidence)),
        ),
        q(
            graph,
            &node,
            mm("consequenceIfFalse"),
            literal_str(assumption.consequence_if_false.as_str()),
        ),
        q(
            graph,
            &node,
            mm("verificationCost"),
            literal_double(assumption.verification_cost),
        ),
        q(
            graph,
            &node,
            mm("decisionDependence"),
            literal_double(assumption.decision_dependence),
        ),
    ];
    for dependency in &assumption.dependencies {
        quads.push(q(
            graph,
            &node,
            mm("dependsOn"),
            subject(&iri::data(dependency).into_string()),
        ));
    }
    quads
}

/// The quads one prediction contributes.
pub fn prediction_quads(prediction: &Prediction) -> Vec<Quad> {
    let graph = EPISTEMIC_GRAPH;
    let node = iri::data(&prediction.id).into_string();
    let status = prediction
        .outcome
        .as_ref()
        .map(|outcome| outcome.status.as_str())
        .unwrap_or("predicted");
    let mut quads = vec![
        q(graph, &node, mm("Prediction"), subject(&node)),
        q(graph, &node, mm("status"), literal_str(status)),
        q(
            graph,
            &node,
            mm("confidence"),
            literal_double(prediction.probability),
        ),
        q(
            graph,
            &node,
            mm("horizonSecs"),
            literal_int(prediction.horizon_secs as i64),
        ),
    ];
    for condition in &prediction.conditions {
        quads.push(q(graph, &node, mm("condition"), literal_str(condition)));
    }
    quads
}

/// The quads one contradiction contributes. Exactly two `mm:contradicts` links,
/// which is what the contradiction shape requires.
pub fn contradiction_quads(contradiction: &Contradiction) -> Vec<Quad> {
    let graph = EPISTEMIC_GRAPH;
    let node = iri::data(&contradiction.id).into_string();
    let mut quads = vec![
        q(graph, &node, mm("Contradiction"), subject(&node)),
        q(
            graph,
            &node,
            mm("status"),
            literal_str(contradiction.status.as_str()),
        ),
        q(
            graph,
            &node,
            mm("contradicts"),
            subject(&iri::data(&contradiction.claim_a).into_string()),
        ),
        q(
            graph,
            &node,
            mm("contradicts"),
            subject(&iri::data(&contradiction.claim_b).into_string()),
        ),
        q(
            graph,
            &node,
            mm("reason"),
            literal_str(&contradiction.reason),
        ),
    ];
    if let Some(evidence) = contradiction.evidence_a {
        quads.push(q(
            graph,
            &node,
            mm("evidenceFor"),
            subject(&iri::data(&evidence).into_string()),
        ));
    }
    if let Some(evidence) = contradiction.evidence_b {
        quads.push(q(
            graph,
            &node,
            mm("evidenceFor"),
            subject(&iri::data(&evidence).into_string()),
        ));
    }
    quads
}

/// The quads one status transition contributes to `/provenance`.
///
/// The change is a `prov:Activity` and a `mm:StatusChange`; the prior value is
/// carried by `mm:priorStatus`. This is the reification fallback for the plan's
/// RDF-star annotation.
pub fn transition_quads(transition: &Transition) -> Vec<Quad> {
    let graph = PROVENANCE_GRAPH;
    let node = iri::data(&transition.id).into_string();
    let subject_node = iri::data(&transition.subject).into_string();
    let mut quads = vec![
        q(
            graph,
            &node,
            named(format!("{PROV}Activity")),
            subject(&node),
        ),
        q(graph, &node, mm("StatusChange"), subject(&node)),
        q(graph, &node, mm("reason"), literal_str(&transition.reason)),
        q(
            graph,
            &node,
            named(format!("{PROV}used")),
            subject(&subject_node),
        ),
        q(
            graph,
            &subject_node,
            named(format!("{PROV}wasGeneratedBy")),
            subject(&node),
        ),
    ];
    if let Some(from) = transition.from_status {
        quads.push(q(
            graph,
            &node,
            mm("priorStatus"),
            literal_str(from.as_str()),
        ));
    }
    if let Some(to) = transition.to_status {
        quads.push(q(graph, &node, mm("newStatus"), literal_str(to.as_str())));
    }
    quads
}

/// The `/world` quads for an admissible claim.
pub fn world_claim_quads(claim: &Claim) -> Vec<Quad> {
    claim_quads(claim)
        .into_iter()
        .map(|quad| {
            Quad::new(
                quad.subject,
                quad.predicate,
                quad.object,
                GraphName::NamedNode(named(iri::graph(WORLD_GRAPH))),
            )
        })
        .collect()
}

/// Rewrite `/epistemic` and `/provenance` from a snapshot.
pub async fn mirror(handle: &GraphHandle, snapshot: &EpistemicSnapshot) -> Result<usize> {
    handle
        .clear_graph(EPISTEMIC_GRAPH)
        .await
        .map_err(crate::error::EpistemicError::from)?;
    let quads = snapshot.to_quads();
    for quad in &quads {
        handle
            .insert(quad.clone())
            .await
            .map_err(crate::error::EpistemicError::from)?;
    }
    handle
        .clear_graph(PROVENANCE_GRAPH)
        .await
        .map_err(crate::error::EpistemicError::from)?;
    let provenance = snapshot.provenance_to_quads();
    for quad in &provenance {
        handle
            .insert(quad.clone())
            .await
            .map_err(crate::error::EpistemicError::from)?;
    }
    Ok(quads.len() + provenance.len())
}

/// One triple, rendered canonically so a round trip compares by value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EpistemicTriple {
    /// Rendered subject.
    pub subject: String,
    /// Rendered predicate.
    pub predicate: String,
    /// Rendered object.
    pub object: String,
}

impl EpistemicTriple {
    /// Render one quad.
    pub fn from_quad(quad: &Quad) -> Self {
        EpistemicTriple {
            subject: render_subject(&quad.subject),
            predicate: format!("<{}>", quad.predicate.as_str()),
            object: render_term(&quad.object),
        }
    }
}

/// The read half of the mirror.
pub trait FromRdf: Sized {
    /// Rebuild from `?s ?p ?o` rows, sorted so two reads compare by value.
    fn from_rows(rows: &[Value]) -> Self;
}

impl FromRdf for Vec<EpistemicTriple> {
    fn from_rows(rows: &[Value]) -> Self {
        let mut triples: Vec<EpistemicTriple> = rows
            .iter()
            .filter_map(|row| {
                Some(EpistemicTriple {
                    subject: render_node_value(row.get("s")?),
                    predicate: render_node_value(row.get("p")?),
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

impl ToRdf for EpistemicSnapshot {
    fn to_quads(&self) -> Vec<Quad> {
        EpistemicSnapshot::to_quads(self)
    }
}

/// A deterministic hash of a quad set, independent of insertion order.
pub fn quads_hash(quads: &[Quad]) -> String {
    let mut rendered: Vec<String> = quads.iter().map(render).collect();
    rendered.sort();
    let borrowed: Vec<&str> = rendered.iter().map(String::as_str).collect();
    mm_core::hash_fields(&borrowed)
}

/// The same hash, computed from a graph read.
pub fn triples_hash(triples: &[EpistemicTriple]) -> String {
    let mut rendered: Vec<String> = triples
        .iter()
        .map(|t| format!("{} {} {}", t.subject, t.predicate, t.object))
        .collect();
    rendered.sort();
    let borrowed: Vec<&str> = rendered.iter().map(String::as_str).collect();
    mm_core::hash_fields(&borrowed)
}

fn render(quad: &Quad) -> String {
    let triple = EpistemicTriple::from_quad(quad);
    format!("{} {} {}", triple.subject, triple.predicate, triple.object)
}

/// The dependency id a `(consequent, antecedent)` pair implies.
pub fn dependency_id(consequent: &Ulid, antecedent: &Ulid) -> Ulid {
    let mut hasher = Sha256::new();
    hasher.update(b"mm.epistemic.dependency\0");
    hasher.update(consequent.to_string().as_bytes());
    hasher.update([0u8]);
    hasher.update(antecedent.to_string().as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Ulid::from_bytes(bytes)
}

/// The quads a justification edge contributes: a reified `mm:Dependency` node.
pub fn dependency_quads(
    consequent: &Ulid,
    antecedent: &Ulid,
    kind: &str,
    criticality: f32,
) -> Vec<Quad> {
    let graph = EPISTEMIC_GRAPH;
    let node = iri::data(&dependency_id(consequent, antecedent)).into_string();
    vec![
        q(graph, &node, mm("Dependency"), subject(&node)),
        q(
            graph,
            &node,
            mm("dependsOn"),
            subject(&iri::data(antecedent).into_string()),
        ),
        q(
            graph,
            &node,
            mm("supports"),
            subject(&iri::data(consequent).into_string()),
        ),
        q(graph, &node, mm("dependencyKind"), literal_str(kind)),
        q(
            graph,
            &node,
            mm("criticality"),
            literal_double(f64::from(criticality)),
        ),
    ]
}

// ------------------------------------------------------------------ helpers ----

fn dedup(mut quads: Vec<Quad>) -> Vec<Quad> {
    let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
    quads.retain(|quad| {
        let triple = EpistemicTriple::from_quad(quad);
        seen.insert((triple.subject, triple.predicate, triple.object))
    });
    quads
}

fn graph_name(graph: &str) -> GraphName {
    GraphName::NamedNode(named(iri::graph(graph)))
}

fn named(iri: impl Into<String>) -> NamedNode {
    NamedNode::new_unchecked(iri.into())
}

fn mm(local: &str) -> NamedNode {
    named(format!("{}{local}", iri::MM))
}

fn subject(iri: &str) -> Term {
    Term::from(NamedOrBlankNode::NamedNode(named(iri)))
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

fn q(graph: &str, subject_iri: &str, predicate: NamedNode, object: Term) -> Quad {
    Quad::new(
        NamedOrBlankNode::NamedNode(named(subject_iri)),
        predicate,
        object,
        graph_name(graph),
    )
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
        Some(dt) if dt == format!("{XSD}string") => format!("\"{value}\""),
        Some(dt) => format!("\"{value}\"^^<{dt}>"),
    }
}

fn render_node_value(value: &Value) -> String {
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
        other => render_node_value(other),
    }
}

fn looks_like_iri(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::ClaimKind;
    use crate::proposition::Proposition;
    use crate::status::EpistemicStatus;
    use mm_store_graph::GraphStore;
    use ulid::Ulid as UlidType;

    fn id(n: u128) -> Ulid {
        UlidType::from_parts(1_700_000_000_000, n)
    }

    fn snapshot() -> EpistemicSnapshot {
        EpistemicSnapshot {
            claims: vec![
                Claim::new(
                    id(1),
                    ClaimKind::Fact,
                    Proposition::literal("https://x/s", "https://x/p", "alpha").unwrap(),
                    EpistemicStatus::Reported,
                    0.7,
                )
                .unwrap(),
                {
                    let mut c = Claim::new(
                        id(2),
                        ClaimKind::Observation,
                        Proposition::literal("https://x/s", "https://x/p", "beta").unwrap(),
                        EpistemicStatus::Observed,
                        0.9,
                    )
                    .unwrap();
                    c.evidence = vec![id(9)];
                    c
                },
            ],
            ..EpistemicSnapshot::default()
        }
    }

    #[test]
    fn a_snapshot_renders_deterministically() {
        let snap = snapshot();
        assert_eq!(snap.to_quads(), snap.to_quads());
        assert_eq!(quads_hash(&snap.to_quads()), quads_hash(&snap.to_quads()));
        assert_eq!(snap.triple_count(), snap.to_quads().len());
    }

    #[test]
    fn only_admissible_claims_reach_the_world() {
        let snap = snapshot();
        let admissible = snap.admissible();
        assert_eq!(admissible.len(), 1);
        assert_eq!(admissible[0].id, id(2));
    }

    #[test]
    fn a_datetime_literal_is_written_canonically() {
        assert_eq!(
            canonical_datetime("2026-10-08T20:58:15.500000000Z"),
            "2026-10-08T20:58:15.5Z"
        );
        assert_eq!(
            canonical_datetime("2026-10-08T20:58:15.000000000Z"),
            "2026-10-08T20:58:15Z"
        );
    }

    #[test]
    fn a_dependency_id_is_a_function_of_the_pair() {
        assert_eq!(dependency_id(&id(1), &id(2)), dependency_id(&id(1), &id(2)));
        assert_ne!(dependency_id(&id(1), &id(2)), dependency_id(&id(2), &id(1)));
    }

    #[tokio::test]
    async fn the_mirror_round_trips_exactly_through_the_graph() {
        let _dir = tempfile::tempdir().unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("epistemic_shapes.ttl");
        let graph = GraphStore::in_memory(&shapes).await.unwrap();
        let snap = snapshot();
        let expected = snap.to_quads();
        mirror(graph.handle(), &snap).await.unwrap();

        let rows = graph.triples(EPISTEMIC_GRAPH).await.unwrap();
        let rebuilt = <Vec<EpistemicTriple> as FromRdf>::from_rows(&rows);
        let mut source: Vec<EpistemicTriple> =
            expected.iter().map(EpistemicTriple::from_quad).collect();
        source.sort();
        assert_eq!(
            rebuilt, source,
            "the mirror must not lose or invent a triple"
        );
        assert_eq!(triples_hash(&rebuilt), quads_hash(&expected));
        graph.shutdown().await.unwrap();
    }
}
