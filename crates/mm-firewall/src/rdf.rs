//! The RDF mirror: the firewall run that graded a decision.
//!
//! `mm-firewall` emits one node — the `mm:FirewallRun` — and links it to the
//! episode it graded, the reason codes it concluded with, the determinations it
//! consulted and the prohibition it hit. Everything decision-shaped (a
//! `mm:Decision`, a `mm:Risk`, a `mm:Comparison`, a `mm:Uncertainty`) belongs to
//! [`mm_decision::rdf`], which is the single authority for that part of the
//! vocabulary; this module delegates to it rather than growing a second rendering
//! of the same nodes that could drift from it.
//!
//! This is a *projection* in the same sense every other mirror in the kernel is:
//! the `firewall_runs` row is the record, and this is the graph view an operator —
//! or a SHACL gate — reads. Two consequences follow, and both are deliberate:
//!
//! * **Only declared predicates are used.** Every predicate emitted here is
//!   declared in `ontology/mm.ttl`. The vendored validator treats an *undeclared*
//!   predicate inside a shape as a hard error, so inventing one for convenience
//!   would put the T-Box and this module out of step and the shapes could not
//!   describe what is actually written.
//! * **The named graph is spelled locally.** `mm-firewall` does not depend on
//!   `mm-store-graph` — the CLI does the inserting — so the graph IRI is built from
//!   [`mm_core::iri::graph`]. That is the same function `mm_store_graph::graph_iri`
//!   calls for a bare name, so the two agree by construction rather than by
//!   convention.
//!
//! Instance IRIs come from [`mm_core::iri::data`], so a run's graph node and its
//! ULID are the same identity in both stores.

use mm_core::iri;
use mm_core::{NamedNode, NamedOrBlankNode, Quad, Term};
use mm_decision::question::DecisionAnswer;

use crate::report::{FirewallInput, FirewallReport};

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

/// The quads one firewall run contributes.
pub fn firewall_quads(report: &FirewallReport) -> Vec<Quad> {
    let node = iri::data(&report.id).into_string();
    let mut quads = vec![
        q(&node, rdf("type"), node_term(&format!("{}FirewallRun", MM))),
        q(
            &node,
            mm("firewallOutcome"),
            literal_str(report.outcome.as_str()),
        ),
        q(&node, mm("inputDigest"), literal_str(&report.input_digest)),
        q(
            &node,
            mm("derivedFrom"),
            node_term(&iri::data(&report.episode).into_string()),
        ),
    ];
    for reason in &report.reason_codes {
        quads.push(q(&node, mm("hasReasonCode"), literal_str(&reason.code)));
    }
    for decision in &report.decisions {
        quads.push(q(
            &node,
            mm("consultedDecision"),
            node_term(&iri::data(decision).into_string()),
        ));
    }
    if let Some(prohibition) = &report.hard_prohibition {
        // A literal rather than a linked node: the vocabulary declares
        // `mm:hardProhibition` and does not declare a `mm:HardProhibition` class, and
        // the SHACL shape the plan asks for says a `REJECT` due to a prohibition
        // *must carry* this predicate — which it does, carrying the prohibition id.
        quads.push(q(
            &node,
            mm("hardProhibition"),
            literal_str(&prohibition.id),
        ));
    }
    quads
}

/// A deterministic Turtle rendering of a quad set.
///
/// Delegated: the decision-domain mirror owns the renderer, so a firewall report
/// and a decision report render identically.
pub fn turtle(quads: &[Quad]) -> String {
    mm_decision::rdf::turtle(quads)
}

/// Insert a decision's quads, returning how many were written.
///
/// A thin delegation, kept here so the firewall's callers do not have to know that
/// the decision mirror lives in another crate.
pub async fn emit_decision(
    graph: &dyn mm_core::Graph,
    id: &mm_core::Ulid,
    answer: &DecisionAnswer,
    input: &FirewallInput,
    calibrated_confidence: Option<f32>,
) -> Result<usize, mm_core::MmError> {
    mm_decision::rdf::emit_decision(
        graph,
        id,
        answer,
        Some(&input.episode),
        calibrated_confidence,
    )
    .await
}

/// Insert a firewall run's quads, returning how many were written.
pub async fn emit_firewall(
    graph: &dyn mm_core::Graph,
    report: &FirewallReport,
) -> Result<usize, mm_core::MmError> {
    let quads = firewall_quads(report);
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
    Term::Literal(mm_core::Literal::new_simple_literal(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::{FirewallOutcome, ReasonCode};
    use crate::report::{HardProhibition, Reversibility, Stakes};
    use mm_core::Ulid;
    use mm_decision::risk::{analyze_risk_with, LossOutcome, RiskMeasure, RiskOptions};

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    fn input() -> FirewallInput {
        let mut input = FirewallInput::new(
            ulid(1),
            analyze_risk_with(
                &[LossOutcome {
                    name: "small".into(),
                    probability: 1.0,
                    loss: 1.0,
                }],
                RiskMeasure::Variance,
                &RiskOptions {
                    capital: 1.0e9,
                    reversibility: 0.5,
                    optionality: 0.5,
                },
            ),
        );
        input.reversibility = Reversibility::PartiallyReversible;
        input.stakes = Stakes {
            level: 0.3,
            irreversibility: 0.2,
        };
        input
    }

    fn report() -> FirewallReport {
        let input = input();
        FirewallReport::new(
            ulid(20),
            &input,
            FirewallOutcome::Reject,
            vec![ReasonCode::new(
                "prohibition.unauthorized_tool",
                "prohibition.unauthorized_tool",
                "tool=shell",
                FirewallOutcome::Reject,
            )],
            vec![ulid(21), ulid(22)],
            Some(HardProhibition::new(
                "unauthorized_tool",
                "the action uses a tool with no permission grant",
                vec!["tool=shell".into()],
            )),
            std::collections::BTreeMap::new(),
        )
    }

    #[test]
    fn a_firewall_run_emits_its_outcome_reasons_and_prohibition() {
        let quads = firewall_quads(&report());
        let turtle = turtle(&quads);
        assert!(turtle.contains("rdf:type mm:FirewallRun"), "{turtle}");
        assert!(turtle.contains("mm:firewallOutcome \"REJECT\""), "{turtle}");
        assert!(
            turtle.contains("mm:hasReasonCode \"prohibition.unauthorized_tool\""),
            "{turtle}"
        );
        assert!(
            turtle.contains("mm:hardProhibition \"unauthorized_tool\""),
            "{turtle}"
        );
        assert!(turtle.contains("mm:inputDigest"), "{turtle}");
        assert!(turtle.contains("mm:consultedDecision mmd:"), "{turtle}");
        assert!(turtle.contains("mm:derivedFrom mmd:"), "{turtle}");
        assert_eq!(
            quads.len(),
            8,
            "four scalars, one reason, two consulted decisions, one prohibition"
        );
    }

    #[test]
    fn a_clean_run_emits_no_prohibition() {
        let input = input();
        let report = FirewallReport::new(
            ulid(20),
            &input,
            FirewallOutcome::Proceed,
            vec![ReasonCode::new(
                "signal.nominal",
                "signal.nominal",
                "nothing objected",
                FirewallOutcome::Proceed,
            )],
            Vec::new(),
            None,
            std::collections::BTreeMap::new(),
        );
        let turtle = turtle(&firewall_quads(&report));
        assert!(
            turtle.contains("mm:firewallOutcome \"PROCEED\""),
            "{turtle}"
        );
        assert!(!turtle.contains("mm:hardProhibition"), "{turtle}");
        assert!(
            turtle.contains("mm:hasReasonCode \"signal.nominal\""),
            "{turtle}"
        );
    }

    #[test]
    fn rendering_is_byte_identical_across_calls() {
        let quads = firewall_quads(&report());
        assert_eq!(turtle(&quads), turtle(&quads));
    }
}
