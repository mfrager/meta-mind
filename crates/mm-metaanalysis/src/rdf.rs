//! The `/selfeng` mirror: predictions, scoring passes, analyses and lessons as RDF.
//!
//! Phase 11's records reach the graph through one named graph, `selfeng`, added to
//! [`mm_core::iri::NAMED_GRAPHS`] by migration `0011_self_engineering.sql`. It is *not*
//! `/provenance`: Phase 6's epistemic mirror clears and rewrites `/provenance` from its
//! snapshot on every mutation, so a journal written there would lose the entry that
//! justified a promotion. The two graphs have different lifetimes, so they are two
//! graphs. (The phase plan's §4 names `/provenance`; the deviation is recorded in
//! `mm-core::iri` where the graph was added, for the same reason Phase 10's `/tools`
//! records the same one.)
//!
//! What lives here is the *shape of the records*: which prediction was staked with what
//! probability and class, what a scoring pass measured, which trigger fired and what
//! was diagnosed, what a lesson says. What does not: the budgets, because a budget is a
//! `(kind, period)` row with an arithmetic invariant the graph cannot state, and the
//! ledger's own immutability, because the SQL triggers already enforce it on the rows
//! this mirrors.
//!
//! Everything is rendered through [`quads_hash`], which hashes the *sorted* rendering
//! of a quad set, so a replay's graph compares equal whether or not the quads were
//! inserted in the same order.

use mm_core::{iri, Literal, NamedNode, NamedOrBlankNode, Quad, Term, Timestamp, Ulid};

use crate::calibration::CalibrationReport;
use crate::diagnosis::MetaAnalysis;
use crate::ledger::{Prediction, PredictionOutcome};
use crate::lessons::Lesson;

/// The named graph Phase 11 writes.
pub const SELFENG_GRAPH: &str = "selfeng";

/// The `rdf:type` predicate of a quad.
///
/// It is its own helper because it is the one predicate that must NOT go through
/// [`predicate`]: that function prefixes a *local* name with the ontology namespace, and
/// prefixing a full IRI produces `…ontology#http://…rdf-syntax-ns#type`, an IRI with two
/// fragments that the parser refuses outright.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn type_quad(subject: &NamedOrBlankNode, class_iri: String) -> Quad {
    let graph = named(&iri::graph(SELFENG_GRAPH));
    Quad::new(subject.clone(), named(RDF_TYPE), term(class_iri), graph)
}

fn named(text: &str) -> NamedNode {
    NamedNode::new_unchecked(text)
}

fn node(text: &str) -> NamedOrBlankNode {
    NamedOrBlankNode::NamedNode(named(text))
}

fn literal(text: impl Into<String>) -> Term {
    Term::Literal(Literal::new_simple_literal(text.into()))
}

/// The XSD namespace, for the typed literals the shapes require.
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// A typed numeric literal.
///
/// The typed form is not decoration: `ontology/shapes/selfeng.ttl` says
/// `sh:datatype xsd:double` for a probability and `sh:datatype xsd:integer` for a
/// horizon, and a simple literal is `xsd:string`, so emitting one would fail the gate's
/// validation rather than merely looking odd.
fn typed(value: impl Into<String>, datatype: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(
        value.into(),
        named(&format!("{XSD}{datatype}")),
    ))
}

/// An `xsd:double` literal for an `f32` value.
///
/// The ledger stores `REAL`, which Rust reads back as `f32`; formatting the *widened*
/// `f64` would print `0.800000011920929` for a probability the kernel wrote as `0.8`.
/// An `f32`'s own `Display` is the shortest decimal that round-trips it, so this is both
/// readable and exactly the stored value.
fn double_f32(value: f32) -> Term {
    typed(format!("{value}"), "double")
}

fn double(value: f64) -> Term {
    typed(format!("{value}"), "double")
}

fn integer(value: i64) -> Term {
    typed(value.to_string(), "integer")
}

fn boolean(value: bool) -> Term {
    typed(if value { "true" } else { "false" }, "boolean")
}

fn term(iri_text: String) -> Term {
    Term::NamedNode(named(&iri_text))
}

fn predicate(local: &str) -> String {
    iri::mm(local).into_string()
}

/// One quad in `/selfeng`.
///
/// The graph name's type is inferred from `Quad::new` rather than spelled out: this
/// crate reaches the RDF types through `mm_core`'s re-exports, which deliberately do
/// not expose `GraphName` (nothing outside the graph store needs to name it), and a
/// `NamedNode` converts into it.
fn quad(subject: &NamedOrBlankNode, local: &str, object: Term) -> Quad {
    let graph = named(&iri::graph(SELFENG_GRAPH));
    Quad::new(subject.clone(), named(&predicate(local)), object, graph)
}

/// The node IRI of a prediction, a run or an analysis.
pub fn data_node(id: &Ulid) -> String {
    iri::data(id).into_string()
}

/// The quads one prediction contributes, with its resolution when it has one.
pub fn prediction_quads(prediction: &Prediction, outcome: Option<&PredictionOutcome>) -> Vec<Quad> {
    let subject = node(&data_node(&prediction.id));
    let mut quads = vec![
        type_quad(&subject, iri::mm("CalibratedPrediction").into_string()),
        quad(&subject, "probability", double_f32(prediction.probability)),
        quad(
            &subject,
            "predictionClass",
            literal(prediction.class.clone()),
        ),
        quad(
            &subject,
            "horizonSeconds",
            integer(prediction.horizon_seconds()),
        ),
        quad(
            &subject,
            "created",
            literal(prediction.created_at.to_rfc3339()),
        ),
    ];
    for condition in &prediction.conditions {
        quads.push(quad(
            &subject,
            "predictionCondition",
            literal(condition.clone()),
        ));
    }
    if let Some(outcome) = outcome {
        quads.push(quad(&subject, "observedOutcome", boolean(outcome.observed)));
        quads.push(quad(
            &subject,
            "resolvedAt",
            literal(outcome.resolved_at.to_rfc3339()),
        ));
        quads.push(quad(
            &subject,
            "producedEvidence",
            term(data_node(&outcome.evidence)),
        ));
    }
    dedup(quads)
}

/// The quads one scoring pass contributes.
pub fn calibration_run_quads(
    id: &Ulid,
    subject_class: &str,
    report: &CalibrationReport,
) -> Vec<Quad> {
    let subject = node(&data_node(id));
    dedup(vec![
        type_quad(&subject, iri::mm("CalibrationRun").into_string()),
        quad(
            &subject,
            "calibrationSubject",
            literal(subject_class.to_string()),
        ),
        quad(&subject, "brierScore", double(report.brier)),
        quad(&subject, "logLoss", double(report.log_loss)),
        quad(&subject, "eceScore", double(report.ece)),
        quad(&subject, "coverage", double(report.coverage)),
        quad(&subject, "selectiveRisk", double(report.selective_risk)),
        quad(&subject, "baselineBrier", double(report.baseline_brier)),
        quad(&subject, "created", literal(Timestamp::now().to_rfc3339())),
    ])
}

/// The quads one meta-analysis contributes. The error classes come out as one
/// `mm:errorClass` triple each, so a SHACL `sh:minCount 1` is a real constraint.
pub fn analysis_quads(analysis: &MetaAnalysis) -> Vec<Quad> {
    let subject = node(&data_node(&analysis.id));
    let mut quads = vec![
        type_quad(&subject, iri::mm("MetaAnalysis").into_string()),
        quad(&subject, "triggerKind", literal(analysis.trigger.as_str())),
        quad(&subject, "diagnosis", literal(analysis.diagnosis.clone())),
        quad(&subject, "recurrence", double(analysis.recurrence)),
        quad(&subject, "impact", double(analysis.impact)),
        quad(
            &subject,
            "created",
            literal(analysis.created_at.to_rfc3339()),
        ),
        quad(
            &subject,
            "analysedEpisode",
            term(data_node(&analysis.episode)),
        ),
    ];
    for class in &analysis.error_classes {
        quads.push(quad(&subject, "errorClass", literal(class.as_str())));
    }
    dedup(quads)
}

/// The quads one lesson contributes, linked to the analysis it came from.
pub fn lesson_quads(lesson: &Lesson, analysis: &MetaAnalysis) -> Vec<Quad> {
    let subject = node(&data_node(&lesson.id));
    let mut quads = vec![
        type_quad(&subject, iri::mm("Lesson").into_string()),
        quad(&subject, "lessonText", literal(lesson.text.clone())),
        quad(
            &subject,
            "lessonConfidence",
            double(f64::from(lesson.confidence)),
        ),
        quad(
            &subject,
            "evidenceCount",
            integer(i64::from(lesson.evidence_count)),
        ),
        quad(
            &subject,
            "derivedFromAnalysis",
            term(data_node(&analysis.id)),
        ),
        quad(&subject, "created", literal(Timestamp::now().to_rfc3339())),
    ];
    for domain in &lesson.domains {
        quads.push(quad(&subject, "lessonDomain", literal(domain.clone())));
    }
    if let Some(effect) = &lesson.policy_effect {
        quads.push(quad(&subject, "policyEffect", literal(effect.clone())));
    } else {
        // A lesson with no policy effect says so, rather than being silent: the
        // difference between "it changes nothing" and "nobody recorded what it
        // changes" has to be readable in the graph.
        quads.push(quad(&subject, "policyEffect", literal("none")));
    }
    dedup(quads)
}

/// A canonical rendering of one quad, for hashing and comparison.
pub fn render_quad(quad: &Quad) -> String {
    let subject = match &quad.subject {
        NamedOrBlankNode::NamedNode(n) => format!("<{}>", n.as_str()),
        NamedOrBlankNode::BlankNode(b) => format!("_:{b}"),
    };
    let object = match &quad.object {
        Term::NamedNode(n) => format!("<{}>", n.as_str()),
        Term::BlankNode(b) => format!("_:{b}"),
        Term::Literal(l) => format!("\"{}\"", l.value()),
    };
    format!("{subject} <{}> {object}", quad.predicate.as_str())
}

/// A hash of a quad set that does not depend on insertion order.
pub fn quads_hash(quads: &[Quad]) -> String {
    let mut rendered: Vec<String> = quads.iter().map(render_quad).collect();
    rendered.sort();
    rendered.dedup();
    mm_core::content_hash(rendered.join("\n").as_bytes())
}

fn dedup(quads: Vec<Quad>) -> Vec<Quad> {
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<Quad> = Vec::new();
    for quad in quads {
        let key = render_quad(&quad);
        if !seen.contains(&key) {
            seen.push(key);
            out.push(quad);
        }
    }
    out
}

/// Insert a quad set into `/selfeng`.
///
/// Insertion is idempotent in RDF, so mirroring the same rows twice does not duplicate
/// their record, and the graph is not cleared first: analyses and predictions
/// accumulate, and a clear would erase every earlier analysis's record.
pub async fn mirror(
    handle: &mm_store_graph::GraphHandle,
    quads: Vec<Quad>,
) -> Result<usize, mm_core::MmError> {
    let count = quads.len();
    for quad in quads {
        handle.insert(quad).await?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taxonomy::{ErrorClass, Trigger};
    use std::time::Duration;

    fn prediction() -> Prediction {
        Prediction::new(
            Ulid::from_parts(1_700_000_000_000, 1),
            "safety",
            "the migration applies cleanly",
            0.8,
            Duration::from_secs(3600),
        )
        .when(vec!["the store was empty".into()])
        .at(Timestamp::from_epoch_seconds(1_700_000_000))
    }

    fn analysis() -> MetaAnalysis {
        MetaAnalysis {
            id: Ulid::from_parts(1_700_000_000_000, 3),
            trigger: Trigger::RepeatedFailure,
            episode: Ulid::from_parts(1_700_000_000_000, 4),
            diagnosis: "the retrieval step did not re-read the schema".into(),
            error_classes: vec![ErrorClass::Data, ErrorClass::Retrieval],
            recurrence: 0.8,
            impact: 0.6,
            created_at: Timestamp::from_epoch_seconds(1_700_000_000),
        }
    }

    #[test]
    fn an_unresolved_prediction_has_no_outcome_triples() {
        let quads = prediction_quads(&prediction(), None);
        let rendered = quads.iter().map(render_quad).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("CalibratedPrediction"), "{rendered}");
        assert!(rendered.contains("predictionClass"), "{rendered}");
        assert!(rendered.contains("0.8"), "{rendered}");
        // The graph name is not asserted here: `render_quad` renders the triple for a
        // human, and the graph *is* asserted where it can be observed — `tests/golden.rs`
        // mirrors through the store and reads the result back from `/selfeng`.
        assert!(!rendered.contains("observedOutcome"), "{rendered}");
        assert_eq!(prediction_quads(&prediction(), None), quads);
    }

    #[test]
    fn a_resolved_prediction_carries_its_evidence_exactly_once() {
        let outcome = PredictionOutcome::new(
            true,
            Ulid::from_parts(1_700_000_000_000, 5),
            Timestamp::from_epoch_seconds(1_700_000_100),
        );
        let quads = prediction_quads(&prediction(), Some(&outcome));
        let evidence: Vec<&Quad> = quads
            .iter()
            .filter(|quad| quad.predicate.as_str().ends_with("producedEvidence"))
            .collect();
        assert_eq!(
            evidence.len(),
            1,
            "a shape requiring exactly one must see one"
        );
        let rendered = render_quad(evidence[0]);
        assert!(rendered.contains(&mm_core::ulid_string(&outcome.evidence)));
    }

    #[test]
    fn an_analysis_carries_one_error_class_triple_per_class() {
        let quads = analysis_quads(&analysis());
        let classes: Vec<&Quad> = quads
            .iter()
            .filter(|quad| quad.predicate.as_str().ends_with("errorClass"))
            .collect();
        assert_eq!(classes.len(), 2);
        let rendered = quads.iter().map(render_quad).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("repeated_failure"), "{rendered}");
    }

    #[test]
    fn a_lesson_with_no_policy_effect_says_none_rather_than_nothing() {
        let lesson = Lesson {
            id: Ulid::from_parts(1_700_000_000_000, 6),
            text: "x".into(),
            confidence: 0.4,
            evidence_count: 1,
            domains: vec!["data".into()],
            policy_effect: None,
        };
        let rendered = lesson_quads(&lesson, &analysis())
            .iter()
            .map(render_quad)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("policyEffect"), "{rendered}");
        assert!(rendered.contains("\"none\""), "{rendered}");
    }

    #[test]
    fn the_hash_ignores_insertion_order_and_duplicates() {
        let quads = prediction_quads(&prediction(), None);
        let mut reversed = quads.clone();
        reversed.reverse();
        assert_eq!(quads_hash(&quads), quads_hash(&reversed));
        let mut duplicated = quads.clone();
        duplicated.extend(quads.clone());
        assert_eq!(quads_hash(&quads), quads_hash(&duplicated));
        let mut changed = quads.clone();
        changed.push(quad(
            &node("https://metamind.dev/data/x"),
            "extra",
            literal("x"),
        ));
        assert_ne!(quads_hash(&quads), quads_hash(&changed));
    }
}
