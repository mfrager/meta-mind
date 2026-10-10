//! The RDF mirror of a bounded decision, its risk profile, its comparison check and
//! its uncertainty measurement.
//!
//! A projection in the same sense every other mirror in the kernel is: the tabular
//! `decisions` / `comparisons` / `factuality_checks` rows are the record, and this is
//! the graph view an operator — or a SHACL gate — reads. Everything the mirror
//! needs is in the record it is handed; nothing here reads a store.
//!
//! Three decisions are worth stating, because none of them is visible in the shape
//! of the code:
//!
//! * **Only declared predicates are used.** Every predicate emitted here is
//!   declared in `ontology/mm.ttl`. The vendored validator treats an *undeclared*
//!   predicate inside a shape as a hard error, so inventing a `mm:factuality` for
//!   convenience would put the T-Box and this module out of step and the shapes
//!   could not describe what is actually written. A factuality score therefore does
//!   not get a node of its own: it reaches the graph as a reason code on the
//!   firewall run that acted on it, which is the only place it had authority.
//! * **A decision carries its own features.** `mm:hasFeature` is one literal per
//!   feature, rendered `key=value`, because the features are what Phase 11 fits a
//!   calibrator on and a graph that dropped them would be a graph of the conclusion
//!   and not of the evidence.
//! * **The graph is spelled locally.** `mm-decision` does not depend on
//!   `mm-store-graph` — the CLI does the inserting — so the graph IRI comes from
//!   [`mm_core::iri::graph`], which is the same function `mm_store_graph::graph_iri`
//!   calls for a bare name. The two agree by construction rather than by convention.
//!
//! Instance IRIs come from [`mm_core::iri::data`], so a decision's graph node and
//! its ULID are the same identity in both stores.
//!
//! One deviation worth naming: a [`DecisionAnswer`] carries no identifier of its own
//! — the caller records the answer — so [`decision_quads`] takes the decision's
//! ULID as an argument. Deriving it from the answer's content hash would make two
//! identical answers about two different episodes one node.

use mm_core::iri;
use mm_core::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Quad, Term};

use crate::compare::ComparisonCheck;
use crate::question::{AnswerValue, DecisionAnswer};
use crate::risk::RiskProfile;

/// The named graph decision-domain records are emitted into.
pub const DECISION_GRAPH: &str = "decision";

/// The named graph the audit links are mirrored into.
pub const PROVENANCE_GRAPH: &str = "provenance";

/// The ontology namespace.
pub const MM: &str = "https://metamind.dev/ontology#";

/// The RDF namespace, for `rdf:type`.
pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// The XSD namespace, for the datatypes the shapes check.
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// The quads one bounded decision contributes to `/decision`.
///
/// `calibrated_confidence` is `Some` exactly when a calibrator was fitted for the
/// decision's class, so the graph distinguishes "this confidence was calibrated"
/// from "the pre-calibration number was reported" without needing a second
/// predicate — and a shape can then require `mm:calibratedConfidence` only where a
/// calibration was claimed.
///
/// `episode` is optional because a decision may be asked outside an episode (the
/// CLI's `decide` does exactly that); when it is absent the decision is emitted
/// with no `mm:hasOutcome` edge rather than with an invented one.
pub fn decision_quads(
    id: &mm_core::Ulid,
    answer: &DecisionAnswer,
    episode: Option<&mm_core::Ulid>,
    calibrated_confidence: Option<f32>,
) -> Vec<Quad> {
    let node = iri::data(id).into_string();
    let question = iri::data(&answer.question).into_string();
    let mut quads = vec![
        q(&node, rdf("type"), node_term(&format!("{}Decision", MM))),
        q(&node, mm("hasQuestion"), node_term(&question)),
        q(
            &node,
            mm("questionKind"),
            literal_str(answer.question_kind.as_str()),
        ),
        q(&node, mm("hasConfidence"), literal_float(answer.confidence)),
        q(&node, mm("decidedBy"), literal_str(answer.core.as_str())),
    ];
    if let Some(episode) = episode {
        quads.push(q(
            &node,
            mm("hasOutcome"),
            node_term(&iri::data(episode).into_string()),
        ));
    }
    if let Some(calibrated) = calibrated_confidence {
        quads.push(q(
            &node,
            mm("calibratedConfidence"),
            literal_float(calibrated),
        ));
    }
    for (key, value) in &answer.features.0 {
        quads.push(q(
            &node,
            mm("hasFeature"),
            literal_str(&format!("{key}={value:.9}")),
        ));
    }
    match &answer.answer {
        AnswerValue::Option { value } => {
            quads.push(q(&node, mm("selectedOption"), literal_str(value)));
        }
        AnswerValue::Score { value } => {
            quads.push(q(&node, mm("measuredValue"), literal_float(*value)));
        }
        AnswerValue::Bool { value } => {
            // A boolean answer is reported as its measured value, 1 or 0, because
            // the vocabulary declares `mm:measuredValue` and not a boolean-answer
            // predicate; the question kind already says how to read it.
            quads.push(q(
                &node,
                mm("measuredValue"),
                literal_float(if *value { 1.0 } else { 0.0 }),
            ));
        }
        AnswerValue::None => {}
    }
    quads
}

/// The quads one risk profile contributes.
///
/// Only the measure's name and its value are emitted, because those are the
/// predicates the vocabulary declares. The other six numbers of the profile are
/// reported by `mm-cli risk analyze` and stored in the `decisions` row's risk
/// column; a richer graph block would need a vocabulary this phase does not own.
pub fn risk_quads(id: &mm_core::Ulid, profile: &RiskProfile) -> Vec<Quad> {
    let node = iri::data(id).into_string();
    vec![
        q(&node, rdf("type"), node_term(&format!("{}Risk", MM))),
        q(
            &node,
            mm("usesMeasure"),
            literal_str(profile.measure.name()),
        ),
        q(
            &node,
            mm("measuredValue"),
            literal_double(profile.measure_value),
        ),
    ]
}

/// The quads one uncertainty measurement contributes.
///
/// `kind` is the scorer's own name ([`crate::uncertainty::UncertaintyKind::as_str`]
/// or the firewall's signal name), and `value` must already be in `[0,1]` — the
/// shape requires it, so a caller passing an out-of-range value gets a violation
/// rather than a silently accepted one.
pub fn uncertainty_quads(id: &mm_core::Ulid, kind: &str, value: f32) -> Vec<Quad> {
    let node = iri::data(id).into_string();
    vec![
        q(&node, rdf("type"), node_term(&format!("{}Uncertainty", MM))),
        q(&node, mm("usesMeasure"), literal_str(kind)),
        q(&node, mm("measuredValue"), literal_float(value)),
    ]
}

/// The quads one comparison check contributes.
///
/// The verdict is a literal rather than an IRI so the shape can constrain it with
/// an enumeration, and every violation becomes one `mm:hasViolation` literal
/// carrying `code: field` — the code is what an operator filters on, and the field
/// is what makes it actionable. Violations are emitted in the check's own fixed
/// order, so the rendering is a function of the check and not of a map's iteration.
pub fn comparison_quads(id: &mm_core::Ulid, check: &ComparisonCheck) -> Vec<Quad> {
    let node = iri::data(id).into_string();
    let mut quads = vec![
        q(&node, rdf("type"), node_term(&format!("{}Comparison", MM))),
        q(
            &node,
            mm("comparisonValid"),
            literal_str(check.verdict.as_str()),
        ),
    ];
    if let Some(normalized) = &check.normalized {
        quads.push(q(
            &node,
            mm("comparedTo"),
            literal_str(&format!(
                "{} vs {}",
                normalized.baseline, normalized.candidate
            )),
        ));
        quads.push(q(
            &node,
            mm("hasFeature"),
            literal_str(&normalized.canonical),
        ));
    }
    for violation in &check.violations {
        quads.push(q(
            &node,
            mm("hasViolation"),
            literal_str(&format!("{}: {}", violation.code, violation.field)),
        ));
    }
    quads
}

/// A deterministic Turtle rendering of a quad set.
pub fn turtle(quads: &[Quad]) -> String {
    let mut out = String::from("@prefix mm:   <https://metamind.dev/ontology#> .\n");
    out.push_str("@prefix mmd:  <https://metamind.dev/data/> .\n");
    out.push_str("@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n");
    out.push_str("@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .\n\n");
    for quad in quads {
        out.push_str(&render_subject(&quad.subject));
        out.push(' ');
        out.push_str(&abbreviate(quad.predicate.as_str()));
        out.push(' ');
        out.push_str(&render_object(&quad.object));
        out.push_str(" .\n");
    }
    out
}

/// Insert a decision's quads, returning how many were written.
pub async fn emit_decision(
    graph: &dyn mm_core::Graph,
    id: &mm_core::Ulid,
    answer: &DecisionAnswer,
    episode: Option<&mm_core::Ulid>,
    calibrated_confidence: Option<f32>,
) -> Result<usize, mm_core::MmError> {
    let quads = decision_quads(id, answer, episode, calibrated_confidence);
    for quad in &quads {
        graph.insert(DECISION_GRAPH, quad.clone()).await?;
    }
    Ok(quads.len())
}

/// Insert a comparison check's quads, returning how many were written.
pub async fn emit_comparison(
    graph: &dyn mm_core::Graph,
    id: &mm_core::Ulid,
    check: &ComparisonCheck,
) -> Result<usize, mm_core::MmError> {
    let quads = comparison_quads(id, check);
    for quad in &quads {
        graph.insert(DECISION_GRAPH, quad.clone()).await?;
    }
    Ok(quads.len())
}

// ---------------------------------------------------------------- builders ----

/// A quad in the decision graph, with a named subject.
///
/// `GraphName` is not re-exported by `mm-core`, so the graph is named through
/// `Quad::new`'s parameter type with an `Into` conversion rather than by naming the
/// type.
fn q(subject: &str, predicate: NamedNode, object: Term) -> Quad {
    Quad::new(
        NamedOrBlankNode::NamedNode(named(subject)),
        predicate,
        object,
        named(iri::graph(DECISION_GRAPH)),
    )
}

/// A `mm:` predicate.
fn mm(local: &str) -> NamedNode {
    named(format!("{MM}{local}"))
}

/// An `rdf:` predicate.
fn rdf(local: &str) -> NamedNode {
    named(format!("{RDF}{local}"))
}

/// A named node from anything that renders as an IRI.
fn named(iri: impl Into<String>) -> NamedNode {
    NamedNode::new_unchecked(iri.into())
}

/// A term naming an IRI.
fn node_term(iri: &str) -> Term {
    Term::from(named(iri))
}

/// A plain string literal.
fn literal_str(value: &str) -> Term {
    Term::Literal(Literal::new_simple_literal(value))
}

/// A typed `xsd:double` literal, rendered with a fixed number of decimals so a
/// replay produces byte-identical bytes.
fn literal_double(value: f64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        format!("{value:.9}"),
        named(format!("{XSD}double")),
    ))
}

/// A typed `xsd:double` literal for a value the system keeps as `f32`.
///
/// The text is the *shortest decimal that round-trips through `f32`*, which is what
/// `Display` for an `f32` produces: `0.8f32` renders `0.8`, which is the number the
/// decision was made with. Widening to `f64` and then printing nine decimals — the
/// first thing this did — renders `0.800000012`, which is neither the number the
/// decision used nor a number a reader of the graph can check by eye.
fn literal_float(value: f32) -> Term {
    Term::Literal(Literal::new_typed_literal(
        format!("{value}"),
        named(format!("{XSD}double")),
    ))
}

/// A blank node, for a caller that needs one.
///
/// Nothing here emits one today; it exists so a test can name a blank subject
/// without reaching through `mm_core` for the constructor.
pub fn blank(label: &str) -> BlankNode {
    BlankNode::new_unchecked(label)
}

// --------------------------------------------------------------- rendering ----

/// Render a subject as Turtle.
fn render_subject(subject: &NamedOrBlankNode) -> String {
    match subject {
        NamedOrBlankNode::NamedNode(node) => abbreviate(node.as_str()),
        NamedOrBlankNode::BlankNode(node) => format!("_:{}", node.as_str()),
    }
}

/// Render an object as Turtle.
fn render_object(object: &Term) -> String {
    match object {
        Term::NamedNode(node) => abbreviate(node.as_str()),
        Term::BlankNode(node) => format!("_:{}", node.as_str()),
        Term::Literal(literal) => {
            let datatype = literal.datatype().as_str();
            let value = escape(literal.value());
            if let Some(language) = literal.language() {
                format!("\"{value}\"@{language}")
            } else if datatype == format!("{XSD}string") {
                format!("\"{value}\"")
            } else {
                format!("\"{value}\"^^{}", abbreviate(datatype))
            }
        }
    }
}

/// Abbreviate an IRI through the declared prefixes, falling back to `<full>`.
fn abbreviate(iri: &str) -> String {
    for (prefix, namespace) in [
        ("mm:", MM),
        ("mmd:", iri::DATA),
        ("rdf:", RDF),
        ("xsd:", XSD),
    ] {
        if let Some(local) = iri.strip_prefix(namespace) {
            return format!("{prefix}{local}");
        }
    }
    format!("<{iri}>")
}

/// Escape a literal's text for Turtle.
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::{check_contract, ComparisonContract, Dimension, Timeframe, Unit};
    use crate::core::CoreId;
    use crate::question::{DecisionQuestion, FeatureVector, ScoreScale};
    use crate::risk::{analyze_risk_with, LossOutcome, RiskMeasure, RiskOptions};
    use std::collections::BTreeMap;

    fn ulid(n: u128) -> mm_core::Ulid {
        mm_core::Ulid::from_parts(1_700_000_000_000, n)
    }

    fn profile() -> RiskProfile {
        analyze_risk_with(
            &[
                LossOutcome {
                    name: "small".into(),
                    probability: 0.75,
                    loss: 1.0,
                },
                LossOutcome {
                    name: "large".into(),
                    probability: 0.25,
                    loss: 10.0,
                },
            ],
            RiskMeasure::Var { alpha: 0.9 },
            &RiskOptions {
                capital: 5.0,
                reversibility: 0.4,
                optionality: 0.6,
            },
        )
    }

    fn contract(unit: &str, value: f64) -> ComparisonContract {
        ComparisonContract {
            objective: "choose the smaller latency".into(),
            object_class: "storage backend".into(),
            objects: vec!["candidate".into()],
            dimensions: vec![Dimension {
                name: "p99 latency".into(),
                unit: Some(unit.into()),
                higher_is_better: false,
            }],
            units: vec![
                Unit {
                    name: "ms".into(),
                    dimension: "duration".into(),
                    to_base: 0.001,
                },
                Unit {
                    name: "s".into(),
                    dimension: "duration".into(),
                    to_base: 1.0,
                },
            ],
            timeframe: Timeframe {
                start: "2026-01-01T00:00:00Z".into(),
                end: "2026-02-01T00:00:00Z".into(),
            },
            conditions: Vec::new(),
            constraints: Vec::new(),
            definitions: BTreeMap::new(),
            evidence: vec![ulid(9)],
            values: BTreeMap::from([("p99 latency".to_string(), value)]),
        }
    }

    #[test]
    fn a_decision_emits_its_kind_confidence_core_and_features() {
        let question = DecisionQuestion::Choice {
            id: ulid(2),
            prompt: "Which mitigation?".into(),
            options: vec!["roll back".into(), "fail over".into()],
            allow_none: true,
        };
        let answer = DecisionAnswer::new(
            &question,
            AnswerValue::Option {
                value: "roll back".into(),
            },
            0.8,
            FeatureVector::new().with("danger", 1.5),
            CoreId::Rules,
            0,
        )
        .unwrap();
        let rendered = turtle(&decision_quads(
            &ulid(30),
            &answer,
            Some(&ulid(1)),
            Some(0.72),
        ));
        assert!(rendered.contains("rdf:type mm:Decision"), "{rendered}");
        assert!(
            rendered.contains("mm:questionKind \"choice\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("mm:selectedOption \"roll back\""),
            "{rendered}"
        );
        assert!(rendered.contains("mm:hasConfidence \"0.8\""), "{rendered}");
        assert!(
            rendered.contains("mm:calibratedConfidence \"0.72\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("mm:hasFeature \"danger=1.500000000\""),
            "{rendered}"
        );
        assert!(rendered.contains("mm:decidedBy \"rules\""), "{rendered}");
        assert!(rendered.contains("mm:hasQuestion mmd:"), "{rendered}");
        assert!(rendered.contains("mm:hasOutcome mmd:"), "{rendered}");
    }

    #[test]
    fn a_decision_outside_an_episode_has_no_outcome_edge() {
        let question = DecisionQuestion::YesNo {
            id: ulid(3),
            prompt: "Is this safe?".into(),
        };
        let answer = DecisionAnswer::new(
            &question,
            AnswerValue::Bool { value: true },
            0.5,
            FeatureVector::new(),
            CoreId::Hosted,
            0,
        )
        .unwrap();
        let rendered = turtle(&decision_quads(&ulid(31), &answer, None, None));
        assert!(!rendered.contains("mm:hasOutcome"), "{rendered}");
        assert!(!rendered.contains("mm:calibratedConfidence"), "{rendered}");
        assert!(rendered.contains("mm:measuredValue \"1\""), "{rendered}");
    }

    #[test]
    fn a_score_answer_is_a_measured_value() {
        let question = DecisionQuestion::Score {
            id: ulid(4),
            prompt: "How much residual risk remains?".into(),
            scale: ScoreScale::unit(),
        };
        let answer = DecisionAnswer::new(
            &question,
            AnswerValue::Score { value: 0.25 },
            0.5,
            FeatureVector::new(),
            CoreId::Local,
            0,
        )
        .unwrap();
        let rendered = turtle(&decision_quads(&ulid(32), &answer, None, None));
        assert!(rendered.contains("mm:measuredValue \"0.25\""), "{rendered}");
    }

    #[test]
    fn a_risk_profile_reports_its_measure_and_value() {
        let rendered = turtle(&risk_quads(&ulid(40), &profile()));
        assert!(rendered.contains("rdf:type mm:Risk"), "{rendered}");
        assert!(rendered.contains("mm:usesMeasure \"var\""), "{rendered}");
        assert!(rendered.contains("mm:measuredValue"), "{rendered}");
    }

    #[test]
    fn an_uncertainty_measurement_stays_in_the_unit_interval() {
        let rendered = turtle(&uncertainty_quads(&ulid(41), "SemanticEntropy", 0.5));
        assert!(rendered.contains("rdf:type mm:Uncertainty"), "{rendered}");
        assert!(
            rendered.contains("mm:usesMeasure \"SemanticEntropy\""),
            "{rendered}"
        );
        assert!(rendered.contains("mm:measuredValue \"0.5\""), "{rendered}");
    }

    #[test]
    fn a_comparison_check_emits_its_verdict_and_violations() {
        let a = contract("ms", 9.0);
        let b = contract("s", 0.09);
        let valid = check_contract(&a, &b);
        let rendered = turtle(&comparison_quads(&ulid(50), &valid));
        assert!(rendered.contains("rdf:type mm:Comparison"), "{rendered}");
        assert!(
            rendered.contains("mm:comparisonValid \"valid\""),
            "{rendered}"
        );
        assert!(rendered.contains("mm:comparedTo"), "{rendered}");

        let mismatched = contract("ms", 9.0);
        let mut other = contract("s", 0.09);
        other.objective = "choose the cheaper backend".into();
        let check = check_contract(&mismatched, &other);
        let rendered = turtle(&comparison_quads(&ulid(51), &check));
        assert!(
            rendered.contains("mm:comparisonValid \"rejected\""),
            "{rendered}"
        );
        assert!(rendered.contains("mm:hasViolation"), "{rendered}");
    }

    #[test]
    fn rendering_a_quad_set_twice_is_byte_identical() {
        let quads = risk_quads(&ulid(40), &profile());
        assert_eq!(turtle(&quads), turtle(&quads));
    }

    #[test]
    fn a_literal_with_a_quote_is_escaped() {
        assert_eq!(escape("say \"hi\"\n"), "say \\\"hi\\\"\\n");
    }

    #[test]
    fn a_blank_node_renders_its_label() {
        let quad = Quad::new(
            NamedOrBlankNode::BlankNode(blank("b0")),
            mm("measuredValue"),
            literal_double(1.0),
            named(iri::graph(DECISION_GRAPH)),
        );
        let rendered = turtle(&[quad]);
        assert!(rendered.contains("_:b0 mm:measuredValue"), "{rendered}");
    }
}
