//! The `/memory` mirror: the memory organ as RDF, written from the same state the
//! tables hold.
//!
//! Two rules shape this file, both inherited from Phase 4's `/being` mirror.
//!
//! 1. **The mirror is a projection, not a second authority.** It is rewritten from
//!    a snapshot of the tables on every mutation, so a triple that is not in the
//!    snapshot cannot survive in the graph. That is what makes
//!    `memory stats --reconcile` a meaningful check rather than a tautology.
//! 2. **A memory always names a source.** The plan asks SHACL to accept "a source
//!    or an explicit `mm:unknownSource`". The vendored validator's enforced
//!    fragment has no `sh:or`, so the disjunction is realized positively instead:
//!    every memory emits exactly one `mm:source`, pointing either at its real
//!    source or at `mm:unknownSource`, and the shape requires exactly one. The
//!    constraint is therefore *stronger* than the plan's, not weaker.
//!
//! Instance IRIs are `https://metamind.dev/data/{ulid}` via [`mm_core::iri`]. The
//! canonical-`xsd:dateTime` handling is the same hardening Phase 4 needed, for the
//! same reason: a triplestore normalizes a datetime on the way in, so the mirror
//! must write the canonical form or a round trip is fragile.

use mm_core::iri;
use mm_core::Ulid;
use mm_store_graph::GraphHandle;
use oxrdf::{GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term};
use serde_json::Value;

use crate::error::Result;
use crate::model::{
    Community, Consolidation, LinkKind, Memory, MemoryKind, MistakeRecord, SummaryNode,
};
use crate::store::SqliteMemoryStore;

/// The named graph the memory organ is mirrored into.
pub const MEMORY_GRAPH: &str = "memory";

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const PROV: &str = "http://www.w3.org/ns/prov#";

/// The whole memory organ, as the mirror sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemorySnapshot {
    /// Every memory row, active and archived.
    pub memories: Vec<Memory>,
    /// Every link.
    pub links: Vec<(Ulid, Ulid, LinkKind, f32)>,
    /// Every consolidation step.
    pub consolidations: Vec<Consolidation>,
    /// Every summary-tree node.
    pub summaries: Vec<SummaryNode>,
    /// Every community.
    pub communities: Vec<Community>,
    /// Every mistake detail row.
    pub mistakes: Vec<MistakeRecord>,
}

impl MemorySnapshot {
    /// The corrective rules the snapshot's mistakes name.
    pub fn corrective_rules(&self) -> Vec<Ulid> {
        let mut rules: Vec<Ulid> = self
            .mistakes
            .iter()
            .filter_map(|mistake| mistake.corrective_rule)
            .collect();
        rules.sort();
        rules.dedup();
        rules
    }

    /// How many quads the mirror should hold.
    pub fn triple_count(&self) -> usize {
        self.to_quads().len()
    }

    /// Every quad the mirror should hold, in a deterministic order.
    pub fn to_quads(&self) -> Vec<Quad> {
        let mut quads: Vec<Quad> = Vec::new();

        for memory in &self.memories {
            let node = iri::data(&memory.id).into_string();
            quads.push(q(&node, mm("Memory"), subject(&node)));
            quads.push(q(
                &node,
                mm(memory.kind.rdf_class()),
                subject(&node),
            ));
            quads.push(q(&node, mm("memoryKind"), literal_str(memory.kind.as_str())));
            quads.push(q(&node, mm("tier"), literal_str(memory.tier.as_str())));
            quads.push(q(&node, mm("status"), literal_str(memory.status.as_str())));
            quads.push(q(&node, mm("content"), literal_str(&memory.content)));
            // Exactly one source, always. A memory with no recorded source says so
            // explicitly rather than omitting the predicate, which is what lets the
            // shape require it with `sh:minCount 1`.
            let source = memory
                .source
                .map(|s| iri::data(&s).into_string())
                .unwrap_or_else(|| format!("{}unknownSource", iri::MM));
            quads.push(q(&node, mm("source"), subject(&source)));
            quads.push(q(&node, mm("confidence"), literal_double(f64::from(memory.confidence))));
            quads.push(q(&node, mm("importance"), literal_double(f64::from(memory.importance))));
            quads.push(q(&node, mm("validFrom"), literal_dt(&memory.validity.from.to_rfc3339())));
            if let Some(until) = memory.validity.until {
                quads.push(q(&node, mm("validUntil"), literal_dt(&until.to_rfc3339())));
            }
            quads.push(q(&node, mm("recordedAt"), literal_dt(&memory.recorded_at.to_rfc3339())));
            quads.push(q(&node, mm("protected"), literal_bool(memory.protected)));
            quads.push(q(
                &node,
                named(format!("{PROV}wasGeneratedBy")),
                subject(&iri::data(&memory.provenance).into_string()),
            ));
            if let Some(source) = memory.source {
                quads.push(q(
                    &node,
                    named(format!("{PROV}wasDerivedFrom")),
                    subject(&iri::data(&source).into_string()),
                ));
            }
            for cue in &memory.cues {
                quads.push(q(&node, mm("retrievalCue"), literal_str(&cue.text)));
            }
            for entity in &memory.entities {
                quads.push(q(
                    &node,
                    mm("relatedEntity"),
                    subject(&iri::data(entity).into_string()),
                ));
            }
        }

        for (from, to, relation, _weight) in &self.links {
            let node = iri::data(from).into_string();
            let target = subject(&iri::data(to).into_string());
            quads.push(q(&node, mm(link_predicate(*relation)), target));
            if *relation == LinkKind::DerivedFrom || *relation == LinkKind::ConsolidatedFrom {
                quads.push(q(
                    &node,
                    named(format!("{PROV}wasDerivedFrom")),
                    subject(&iri::data(to).into_string()),
                ));
            }
        }

        for step in &self.consolidations {
            let node = iri::data(&step.id).into_string();
            quads.push(q(&node, mm("Consolidation"), subject(&node)));
            quads.push(q(&node, mm("consolidationMethod"), literal_str(step.method.as_str())));
            for source in &step.source_ids {
                quads.push(q(
                    &node,
                    mm("consolidatedFrom"),
                    subject(&iri::data(source).into_string()),
                ));
            }
            quads.push(q(
                &node,
                mm("consolidatedInto"),
                subject(&iri::data(&step.target_id).into_string()),
            ));
        }

        for node in &self.summaries {
            let iri_node = iri::data(&node.memory_id).into_string();
            quads.push(q(&iri_node, mm("SummaryNode"), subject(&iri_node)));
            quads.push(q(&iri_node, mm("treeLevel"), literal_int(i64::from(node.level))));
            if let Some(parent) = node.parent_id {
                quads.push(q(
                    &iri_node,
                    mm("summarizedBy"),
                    subject(&iri::data(&parent).into_string()),
                ));
            }
            for member in &node.member_ids {
                quads.push(q(
                    &iri_node,
                    mm("summarizes"),
                    subject(&iri::data(member).into_string()),
                ));
            }
        }

        for community in &self.communities {
            let iri_node = iri::data(&community.memory_id).into_string();
            quads.push(q(&iri_node, mm("Community"), subject(&iri_node)));
            quads.push(q(&iri_node, mm("communityLabel"), literal_str(&community.label)));
            if let Some(modularity) = community.modularity {
                quads.push(q(&iri_node, mm("modularity"), literal_double(modularity)));
            }
            for member in &community.member_ids {
                quads.push(q(
                    &iri_node,
                    mm("summarizes"),
                    subject(&iri::data(member).into_string()),
                ));
            }
        }

        for mistake in &self.mistakes {
            let node = iri::data(&mistake.memory_id).into_string();
            quads.push(q(&node, mm("FailureRecord"), subject(&node)));
            quads.push(q(&node, mm("failureMode"), literal_str(&mistake.failure_mode)));
            quads.push(q(
                &node,
                mm("recurrenceRisk"),
                literal_double(f64::from(mistake.recurrence_risk)),
            ));
            if let Some(root) = mistake.root_cause {
                quads.push(q(
                    &node,
                    mm("rootCause"),
                    subject(&iri::data(&root).into_string()),
                ));
            }
            for signal in &mistake.missed_signal {
                quads.push(q(&node, mm("missedSignal"), literal_str(signal)));
            }
            if let Some(rule) = mistake.corrective_rule {
                let rule_node = iri::data(&rule).into_string();
                quads.push(q(&node, mm("correctiveRule"), subject(&rule_node)));
                quads.push(q(&rule_node, mm("CorrectiveRule"), subject(&rule_node)));
                quads.push(q(
                    &rule_node,
                    named(format!("{PROV}wasGeneratedBy")),
                    subject(&node),
                ));
            }
        }

        quads
    }
}

/// The predicate a link relation renders as.
pub fn link_predicate(relation: LinkKind) -> &'static str {
    match relation {
        LinkKind::Supports => "supports",
        LinkKind::Contradicts => "contradicts",
        LinkKind::SimilarTo => "similarTo",
        LinkKind::DerivedFrom => "derivedFrom",
        LinkKind::ConsolidatedFrom => "consolidatedFrom",
        LinkKind::Summarizes => "summarizes",
        LinkKind::AccessedBy => "accessedBy",
        LinkKind::RelatedEntity => "relatedEntity",
        LinkKind::ServesCommitment => "servesCommitment",
    }
}

/// Read the whole organ out of the store.
pub async fn snapshot(store: &SqliteMemoryStore) -> Result<MemorySnapshot> {
    Ok(MemorySnapshot {
        memories: store.list_all().await?,
        links: store.list_links().await?,
        consolidations: store.list_consolidations().await?,
        summaries: store.list_summaries().await?,
        communities: store.list_communities().await?,
        mistakes: store.list_mistakes().await?,
    })
}

/// Rewrite `/memory` from a snapshot, returning how many quads it holds.
///
/// The graph is cleared first: a memory that was archived out of the snapshot must
/// not survive in the projection.
pub async fn mirror(handle: &GraphHandle, snapshot: &MemorySnapshot) -> Result<usize> {
    handle.clear_graph(MEMORY_GRAPH).await?;
    let quads = snapshot.to_quads();
    for quad in &quads {
        handle.insert(quad.clone()).await?;
    }
    Ok(quads.len())
}

/// One triple, rendered canonically so a round trip compares by value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemoryTriple {
    /// Rendered subject.
    pub subject: String,
    /// Rendered predicate.
    pub predicate: String,
    /// Rendered object.
    pub object: String,
}

impl MemoryTriple {
    /// Render one quad.
    pub fn from_quad(quad: &Quad) -> Self {
        MemoryTriple {
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

impl FromRdf for Vec<MemoryTriple> {
    fn from_rows(rows: &[Value]) -> Self {
        let mut triples: Vec<MemoryTriple> = rows
            .iter()
            .filter_map(|row| {
                Some(MemoryTriple {
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

impl ToRdf for MemorySnapshot {
    fn to_quads(&self) -> Vec<Quad> {
        MemorySnapshot::to_quads(self)
    }
}

/// A deterministic hash of a quad set, independent of insertion order.
pub fn quads_hash(quads: &[Quad]) -> String {
    let mut rendered: Vec<String> = quads
        .iter()
        .map(|quad| {
            let triple = MemoryTriple::from_quad(quad);
            format!("{} {} {}", triple.subject, triple.predicate, triple.object)
        })
        .collect();
    rendered.sort();
    let borrowed: Vec<&str> = rendered.iter().map(String::as_str).collect();
    mm_core::hash_fields(&borrowed)
}

/// The same hash, computed from a graph read.
pub fn triples_hash(triples: &[MemoryTriple]) -> String {
    let mut rendered: Vec<String> = triples
        .iter()
        .map(|t| format!("{} {} {}", t.subject, t.predicate, t.object))
        .collect();
    rendered.sort();
    let borrowed: Vec<&str> = rendered.iter().map(String::as_str).collect();
    mm_core::hash_fields(&borrowed)
}

/// The kinds of memory the mirror can count, for reconciliation.
pub fn kinds() -> [MemoryKind; 10] {
    crate::model::MEMORY_KINDS
}

// ------------------------------------------------------------------ helpers ----

fn graph_name() -> GraphName {
    GraphName::NamedNode(named(iri::graph(MEMORY_GRAPH)))
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

fn literal_bool(value: bool) -> Term {
    Term::Literal(Literal::new_typed_literal(
        if value { "true" } else { "false" },
        named(format!("{XSD}boolean")),
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

fn q(subject_iri: &str, predicate: NamedNode, object: Term) -> Quad {
    Quad::new(
        NamedOrBlankNode::NamedNode(named(subject_iri)),
        predicate,
        object,
        graph_name(),
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
        Some(RDF_TYPE) => format!("\"{value}\""),
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
    use crate::model::{ConsolidationMethod, RetrievalCue, TimeInterval};
    use mm_core::Timestamp;
    use mm_store_graph::GraphStore;

    fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn snapshot() -> MemorySnapshot {
        let memory = Memory::new(
            id(1),
            MemoryKind::Episodic,
            "the build broke",
            TimeInterval::open(Timestamp::from_epoch_seconds(1)),
            0.8,
            0.6,
            id(2),
            Timestamp::from_epoch_seconds(2),
        )
        .unwrap()
        .with_entities(vec![id(9)])
        .with_cue(RetrievalCue::keyword("build"));
        let mistake = MistakeRecord {
            id: id(20),
            memory_id: id(1),
            failure_mode: "skipped the gate".into(),
            root_cause: None,
            missed_signal: vec!["red ci".into()],
            corrective_rule: Some(id(21)),
            recurrence_risk: 0.3,
        };
        MemorySnapshot {
            memories: vec![memory],
            mistakes: vec![mistake],
            ..MemorySnapshot::default()
        }
    }

    #[test]
    fn a_snapshot_renders_deterministically() {
        let snap = snapshot();
        assert_eq!(snap.to_quads(), snap.to_quads());
        assert_eq!(quads_hash(&snap.to_quads()), quads_hash(&snap.to_quads()));
    }

    #[test]
    fn every_memory_names_exactly_one_source() {
        let snap = snapshot();
        let source = mm("source").as_str().to_string();
        let count = snap
            .to_quads()
            .iter()
            .filter(|quad| quad.predicate.as_str() == source)
            .count();
        assert_eq!(count, snap.memories.len());
    }

    #[test]
    fn a_memory_without_a_source_names_unknown_source() {
        let snap = snapshot();
        let unknown = format!("{}unknownSource", iri::MM);
        assert!(snap.to_quads().iter().any(|quad| {
            quad.object
                == Term::NamedNode(NamedNode::new_unchecked(unknown.clone()))
        }));
    }

    #[test]
    fn a_mistake_names_its_corrective_rule() {
        let snap = snapshot();
        let rule = mm("correctiveRule").as_str().to_string();
        assert!(snap
            .to_quads()
            .iter()
            .any(|quad| quad.predicate.as_str() == rule));
        assert_eq!(snap.corrective_rules(), vec![id(21)]);
    }

    #[test]
    fn consolidation_nodes_expose_their_sources() {
        let snap = MemorySnapshot {
            consolidations: vec![Consolidation {
                id: id(30),
                source_ids: vec![id(1), id(3)],
                target_id: id(4),
                method: ConsolidationMethod::Generalize,
                created_ulid: id(5),
            }],
            ..MemorySnapshot::default()
        };
        let predicate = mm("consolidatedFrom").as_str().to_string();
        let sources = snap
            .to_quads()
            .iter()
            .filter(|quad| quad.predicate.as_str() == predicate)
            .count();
        assert_eq!(sources, 2);
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
        assert_eq!(
            canonical_datetime("2026-10-08T20:58:15.123456789Z"),
            "2026-10-08T20:58:15.123456789Z"
        );
    }

    #[tokio::test]
    async fn the_mirror_round_trips_exactly_through_the_graph() {
        let _dir = tempfile::tempdir().unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("memory.ttl");
        let graph = GraphStore::in_memory(&shapes).await.unwrap();
        let snap = snapshot();
        let expected = snap.to_quads();
        let written = mirror(graph.handle(), &snap).await.unwrap();
        assert_eq!(written, expected.len());

        let rows = graph.triples(MEMORY_GRAPH).await.unwrap();
        let rebuilt = <Vec<MemoryTriple> as FromRdf>::from_rows(&rows);
        let mut source: Vec<MemoryTriple> =
            expected.iter().map(MemoryTriple::from_quad).collect();
        source.sort();
        assert_eq!(rebuilt, source, "the mirror must not lose or invent a triple");
        assert_eq!(triples_hash(&rebuilt), quads_hash(&expected));
        graph.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn insertion_order_does_not_change_the_hash() {
        let _dir = tempfile::tempdir().unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("memory.ttl");
        let graph = GraphStore::in_memory(&shapes).await.unwrap();
        let snap = snapshot();
        graph.handle().clear_graph(MEMORY_GRAPH).await.unwrap();
        for quad in snap.to_quads() {
            graph.handle().insert(quad).await.unwrap();
        }
        let forward = graph.canonical_hash(MEMORY_GRAPH).await.unwrap();
        graph.handle().clear_graph(MEMORY_GRAPH).await.unwrap();
        for quad in snap.to_quads().into_iter().rev() {
            graph.handle().insert(quad).await.unwrap();
        }
        let reversed = graph.canonical_hash(MEMORY_GRAPH).await.unwrap();
        assert_eq!(forward, reversed, "a canonical hash is order-independent");
        graph.shutdown().await.unwrap();
    }
}
