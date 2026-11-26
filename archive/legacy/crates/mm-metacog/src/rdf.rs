//! The RDF mirror: a cognitive episode and the program compiled from it.
//!
//! The phase's plan asks for episodes to be emitted into
//! `https://metamind.dev/graph/epistemic` and to be described by the `mm:`
//! vocabulary in `ontology/metacog.ttl`. This module is that projection, and it is
//! a *projection* in the same sense the other mirrors are: the tabular
//! `episodes`/`programs`/`program_traces` rows are the record, and this is the
//! graph view an operator (or a SHACL gate) reads.
//!
//! Three decisions are worth stating because they are not visible in the shape of
//! the code:
//!
//! * **Only declared predicates are used.** Every predicate emitted here is
//!   declared in `ontology/metacog.ttl`. The vendored validator treats an
//!   undeclared predicate inside a *shape* as a hard error, so inventing a
//!   predicate for convenience would put the T-Box and this module out of step and
//!   the shapes could not describe what is actually written.
//! * **An operation carries its own value block.** There is no `mm:opValue`
//!   predicate in the T-Box, so rather than invent one the op node is typed both
//!   `mm:CognitiveOp` and `mm:OperationValue` and the four components plus the
//!   score hang directly off it. The alternative — a nested value node joined by an
//!   undeclared linking predicate — would be a second vocabulary.
//! * **Derivation, not provenance.** A program is linked to the episode it was
//!   compiled from with `mm:derivedFrom`. That is the only link this module writes.
//!   The PROV-O activity for an episode belongs to the epistemic layer, which is
//!   the only writer that may create provenance records (Phase 6), so a
//!   `/provenance` activity is deliberately *not* invented here.
//!
//! Instance IRIs come from [`mm_core::iri::data`], so an episode's graph node and
//! its ULID are the same identity in both stores.

use mm_core::iri;
use mm_core::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Quad, Term};

use crate::episode::CognitiveEpisode;
use crate::error::Result;
use crate::program::CognitiveProgram;
use crate::scan::ScanResult;

/// The named graph episodes and their programs are emitted into.
pub const EPISODE_GRAPH: &str = "epistemic";

/// The XSD namespace, for the datatypes the shapes check.
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// The RDF namespace, for `rdf:type`.
pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// The quads one episode contributes to `/epistemic`.
///
/// The episode's own numeric beliefs come from the scan that opened it: the
/// episode holds the *references*, and the scan holds the values, so reading the
/// two apart would be reading two different deliberations. Uncertainty is the same
/// factor the tier selector used, so the graph reports the number the controller
/// actually acted on.
///
/// `mm:budgetSpent` is deliberately absent: the spent side of a budget is not a
/// property of the episode record, it is the sum of the trace rows in
/// `program_traces`, and a graph that guessed at it would be a second authority for
/// a number the trace already owns.
pub fn episode_quads(episode: &CognitiveEpisode, scan: &ScanResult) -> Vec<Quad> {
    let node = iri::data(&episode.id).into_string();
    vec![
        q(
            &node,
            rdf("type"),
            node_term(&format!("{}CognitiveEpisode", iri::MM)),
        ),
        q(
            &node,
            rdf("type"),
            node_term(&format!("{}Episode", iri::MM)),
        ),
        q(
            &node,
            mm("tier"),
            literal_int(i64::from(episode.tier.as_u8())),
        ),
        q(&node, mm("stakes"), literal_double(scan.stakes)),
        q(
            &node,
            mm("uncertainty"),
            literal_double(crate::tier::uncertainty_factor(episode, scan)),
        ),
        q(
            &node,
            mm("reversibility"),
            literal_double((1.0 - scan.irreversibility).clamp(0.0, 1.0)),
        ),
        q(
            &node,
            mm("budget"),
            literal_int(i64::from(episode.budget.max_ops)),
        ),
    ]
}

/// The quads one program contributes to `/epistemic`.
///
/// Every node of the program graph becomes one op node, with its class, its
/// position in the execution order, its dependencies, and the value it was ranked
/// by. Emitting the *dependencies* is what makes the graph a program rather than a
/// list, which is the property the shapes check.
pub fn program_quads(program: &CognitiveProgram) -> Vec<Quad> {
    let node = iri::data(&program.id).into_string();
    let episode = iri::data(&program.episode_id).into_string();
    let mut quads = vec![
        q(
            &node,
            rdf("type"),
            node_term(&format!("{}CognitiveProgram", iri::MM)),
        ),
        q(&node, mm("derivedFrom"), node_term(&episode)),
        // The episode names the program compiled from it. It is emitted here, with
        // the program, because `episode_quads` runs before anything has been
        // compiled and has no program to point at.
        q(&episode, mm("compiledProgram"), node_term(&node)),
        q(
            &node,
            mm("tier"),
            literal_int(i64::from(program.tier.as_u8())),
        ),
        q(
            &node,
            mm("budget"),
            literal_int(i64::from(program.policy().max_ops)),
        ),
    ];

    for frame in &program.frames {
        quads.push(q(&node, mm("activatedFrame"), node_term(frame)));
    }
    for doctrine in &program.doctrines {
        quads.push(q(&node, mm("activatedDoctrine"), node_term(doctrine)));
    }
    for technique in &program.techniques {
        quads.push(q(&node, mm("activatedTechnique"), node_term(technique)));
    }
    for reference in &program.reference_classes {
        quads.push(q(&node, mm("referenceClass"), node_term(reference)));
    }
    for tool in &program.required_tools {
        quads.push(q(&node, mm("requiredTool"), literal_str(tool)));
    }
    for criterion in &program.evaluation_criteria {
        quads.push(q(&node, mm("evaluationCriterion"), literal_str(criterion)));
    }
    for condition in &program.stopping {
        quads.push(q(
            &node,
            mm("stoppingCondition"),
            literal_str(&condition.canonical()),
        ));
    }

    // The program owns its operations. Emitting the edge first keeps the whole
    // `hasOperation` block together, so a reader sees the members before their
    // descriptions.
    for step in &program.steps {
        quads.push(q(
            &node,
            mm("hasOperation"),
            Term::from(op_blank(&program.id, step.node)),
        ));
    }

    for step in &program.steps {
        let op_node = op_blank(&program.id, step.node);
        quads.push(q_blank(
            &op_node,
            rdf("type"),
            node_term(&format!("{}CognitiveOp", iri::MM)),
        ));
        quads.push(q_blank(
            &op_node,
            rdf("type"),
            node_term(&format!("{}OperationValue", iri::MM)),
        ));
        quads.push(q_blank(
            &op_node,
            mm("opClass"),
            literal_str(step.op.class().as_str()),
        ));
        quads.push(q_blank(
            &op_node,
            mm("opOrder"),
            literal_int(i64::from(step.order)),
        ));
        for target in step.op.targets() {
            quads.push(q_blank(&op_node, mm("targets"), literal_str(&target)));
        }
        for dependency in program.graph.dependencies(step.node) {
            quads.push(q_blank(
                &op_node,
                mm("opDependsOn"),
                Term::from(op_blank(&program.id, dependency)),
            ));
        }
        quads.push(q_blank(
            &op_node,
            mm("valueEer"),
            literal_double(step.value.expected_error_reduction),
        ));
        quads.push(q_blank(
            &op_node,
            mm("valueImportance"),
            literal_double(step.value.decision_importance),
        ));
        quads.push(q_blank(
            &op_node,
            mm("valueChangeProbability"),
            literal_double(step.value.probability_change),
        ));
        quads.push(q_blank(
            &op_node,
            mm("valueCost"),
            literal_double(step.value.cost),
        ));
        quads.push(q_blank(
            &op_node,
            mm("score"),
            literal_double(step.value.score()),
        ));
    }

    quads
}

/// A deterministic Turtle rendering of an episode.
pub fn episode_turtle(episode: &CognitiveEpisode, scan: &ScanResult) -> String {
    render(&episode_quads(episode, scan))
}

/// A deterministic Turtle rendering of a program.
pub fn program_turtle(program: &CognitiveProgram) -> String {
    render(&program_quads(program))
}

/// Insert every episode quad into `graph`, returning how many were written.
pub async fn emit_episode(
    graph: &dyn mm_core::Graph,
    episode: &CognitiveEpisode,
    scan: &ScanResult,
) -> Result<usize> {
    let quads = episode_quads(episode, scan);
    for quad in &quads {
        graph.insert(EPISODE_GRAPH, quad.clone()).await?;
    }
    Ok(quads.len())
}

/// Insert every program quad into `graph`, returning how many were written.
pub async fn emit_program(graph: &dyn mm_core::Graph, program: &CognitiveProgram) -> Result<usize> {
    let quads = program_quads(program);
    for quad in &quads {
        graph.insert(EPISODE_GRAPH, quad.clone()).await?;
    }
    Ok(quads.len())
}

// ---------------------------------------------------------------- builders ----

/// The blank node an op appears as.
///
/// A blank node label identifies one node *across the whole store*, not just
/// inside one insert, so a label built from the node id alone (`_:op0`) would make
/// every program's first operation the same node — nine programs would pile nine
/// classes, orders and scores onto one subject and the shapes, which allow exactly
/// one of each, would report violations for a graph that is individually correct.
/// The program's ULID is part of the label for that reason, and because it is
/// content-derived the rendering stays a function of the program rather than of the
/// emission order.
fn op_blank(program: &mm_core::Ulid, node: crate::graph::NodeId) -> BlankNode {
    BlankNode::new_unchecked(op_label(program, node))
}

/// The label [`op_blank`] wraps, so a test can name the node it means.
fn op_label(program: &mm_core::Ulid, node: crate::graph::NodeId) -> String {
    format!("op{}{node}", mm_core::ulid_string(program))
}

/// A quad in the episode graph, with a named subject.
///
/// `GraphName` is not re-exported by `mm-core`, so the graph is named through the
/// parameter type of `Quad::new` with an `Into` conversion rather than by naming
/// the type.
fn q(subject: &str, predicate: NamedNode, object: Term) -> Quad {
    Quad::new(
        NamedOrBlankNode::NamedNode(named(subject)),
        predicate,
        object,
        named(iri::graph(EPISODE_GRAPH)),
    )
}

/// A quad in the episode graph, with a blank subject.
fn q_blank(subject: &BlankNode, predicate: NamedNode, object: Term) -> Quad {
    Quad::new(
        NamedOrBlankNode::BlankNode(subject.clone()),
        predicate,
        object,
        named(iri::graph(EPISODE_GRAPH)),
    )
}

/// A `mm:` predicate.
fn mm(local: &str) -> NamedNode {
    named(format!("{}{local}", iri::MM))
}

/// A `rdf:` predicate.
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

/// A typed `xsd:integer` literal.
fn literal_int(value: i64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        value.to_string(),
        named(format!("{XSD}integer")),
    ))
}

/// A typed `xsd:double` literal, rendered with a fixed number of decimals so a
/// replay produces byte-identical bytes.
fn literal_double(value: f64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        format!("{value:.9}"),
        named(format!("{XSD}double")),
    ))
}

// --------------------------------------------------------------- rendering ----

/// Render quads as Turtle, one statement per line, prefixed and abbreviated.
///
/// The `rdf:` prefix is part of the header because `rdf:type` is used; the plan's
/// three prefixes alone would leave the type statements unrenderable. IRIs in a
/// declared namespace are written in their prefixed form and everything else as a
/// full `<iri>`, which is what keeps a library IRIs readable without declaring a
/// prefix that would grow.
fn render(quads: &[Quad]) -> String {
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
                // No language-tagged literal is emitted here; if one ever is, the
                // tag is carried rather than dropped.
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
        ("mm:", iri::MM),
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

/// Escape a string for a double-quoted Turtle literal.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::CognitiveBudget;
    use crate::episode::{CognitiveEpisode, Context};
    use crate::program::{DefaultCompiler, ProgramCompiler};
    use crate::scan::{ComparisonSpec, FailureMode, ScanIssue, ScanResult, Uncertainty};
    use crate::tier::Tier;
    use mm_core::Ulid;
    use std::collections::BTreeSet;

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    /// A full predicate IRI in the `mm:` namespace.
    fn mm_iri(local: &str) -> String {
        format!("{}{local}", iri::MM)
    }

    fn scan() -> ScanResult {
        ScanResult {
            issues: vec![ScanIssue {
                kind: "missing_evidence".into(),
                materiality: 0.8,
                target: "rollback".into(),
            }],
            uncertainties: vec![Uncertainty {
                id: "u1".into(),
                description: "does the window hold".into(),
                magnitude: 0.6,
            }],
            comparison_requirements: vec![ComparisonSpec {
                subject: "rollback".into(),
                versus: "roll forward".into(),
                criterion: "downtime".into(),
            }],
            failure_modes: vec![FailureMode {
                mode: "partial rollback".into(),
                likelihood: 0.2,
                impact: 0.8,
            }],
            simpler_alternatives: vec!["ask the on-call".into()],
            stakes: 0.8,
            irreversibility: 0.4,
            verification_value: 0.7,
        }
    }

    fn episode() -> CognitiveEpisode {
        let mut episode = CognitiveEpisode::new(
            ulid(1),
            ulid(2),
            Context::new("roll back the deploy").with_novelty(0.5),
            CognitiveBudget::from_spec(16, 8, 2.0, 60_000).unwrap(),
        )
        .unwrap()
        .with_claims(vec![ulid(3)])
        .with_assumptions(vec![ulid(4)])
        .with_candidates(vec!["rollback".into(), "roll forward".into()])
        .with_frames(vec!["https://metamind.dev/library/frame/software".into()])
        .with_techniques(vec!["https://metamind.dev/library/technique/bisect".into()]);
        episode.tier = Tier::T4;
        episode
    }

    fn program() -> CognitiveProgram {
        let episode = episode();
        DefaultCompiler::new()
            .compile(&episode, &scan(), &episode.budget)
            .unwrap()
    }

    /// Every object of `<subject_iri> predicate ?o`.
    fn objects_from(quads: &[Quad], subject_iri: &str, predicate: &str) -> Vec<Term> {
        quads
            .iter()
            .filter(|quad| {
                matches!(
                    &quad.subject,
                    NamedOrBlankNode::NamedNode(node) if node.as_str() == subject_iri
                ) && quad.predicate.as_str() == predicate
            })
            .map(|quad| quad.object.clone())
            .collect()
    }

    /// Every object of `_:blank predicate ?o`.
    fn objects_from_blank(quads: &[Quad], blank: &str, predicate: &str) -> Vec<Term> {
        quads
            .iter()
            .filter(|quad| {
                matches!(
                    &quad.subject,
                    NamedOrBlankNode::BlankNode(node) if node.as_str() == blank
                ) && quad.predicate.as_str() == predicate
            })
            .map(|quad| quad.object.clone())
            .collect()
    }

    /// The literal values of every statement with this predicate.
    fn literals(quads: &[Quad], predicate: &str) -> Vec<String> {
        quads
            .iter()
            .filter(|quad| quad.predicate.as_str() == predicate)
            .filter_map(|quad| match &quad.object {
                Term::Literal(literal) => Some(literal.value().to_string()),
                _ => None,
            })
            .collect()
    }

    /// The IRIs of every named object of `<subject_iri> predicate ?o`.
    fn named_targets(quads: &[Quad], subject_iri: &str, predicate: &str) -> Vec<String> {
        objects_from(quads, subject_iri, predicate)
            .into_iter()
            .filter_map(|object| match object {
                Term::NamedNode(node) => Some(node.as_str().to_string()),
                _ => None,
            })
            .collect()
    }

    /// The blank node ids of `_:?s predicate ?o` where the object is blank.
    fn blank_targets_of(quads: &[Quad], blank: &str, predicate: &str) -> Vec<String> {
        objects_from_blank(quads, blank, predicate)
            .into_iter()
            .filter_map(|object| match object {
                Term::BlankNode(node) => Some(node.as_str().to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_episode_emits_the_expected_types_and_counts() {
        let episode = episode();
        let quads = episode_quads(&episode, &scan());
        let node = iri::data(&episode.id).into_string();

        let types = named_targets(&quads, &node, &format!("{RDF}type"));
        assert!(types.contains(&mm_iri("CognitiveEpisode")), "{types:?}");
        assert!(
            types.contains(&mm_iri("Episode")),
            "an episode is also an Episode: {types:?}"
        );
        assert_eq!(
            literals(&quads, &mm_iri("tier")),
            vec![episode.tier.as_u8().to_string()]
        );
        // The scan's own beliefs are what the episode reports.
        assert_eq!(
            literals(&quads, &mm_iri("stakes")),
            vec!["0.800000000".to_string()]
        );
        assert_eq!(quads.len(), 7, "one quad per emitted property");
        for quad in &quads {
            let predicate = quad.predicate.as_str();
            assert!(
                predicate.starts_with(iri::MM) || predicate.starts_with(RDF),
                "unexpected predicate {predicate}"
            );
        }
    }

    #[test]
    fn a_program_emits_one_op_node_per_step_with_contiguous_order() {
        let program = program();
        let quads = program_quads(&program);
        let node = iri::data(&program.id).into_string();

        let operations: Vec<String> = objects_from(&quads, &node, &mm_iri("hasOperation"))
            .into_iter()
            .filter_map(|object| match object {
                Term::BlankNode(blank) => Some(blank.as_str().to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(operations.len(), program.steps.len());
        assert!(!operations.is_empty(), "a program has at least one op");
        let unique: BTreeSet<&String> = operations.iter().collect();
        assert_eq!(
            unique.len(),
            operations.len(),
            "each op node appears exactly once"
        );

        let mut orders: Vec<u32> = literals(&quads, &mm_iri("opOrder"))
            .iter()
            .map(|order| order.parse::<u32>().unwrap_or_default())
            .collect();
        orders.sort_unstable();
        let expected: Vec<u32> = (1..=program.steps.len() as u32).collect();
        assert_eq!(orders, expected, "opOrder must be contiguous from 1");

        let classes = literals(&quads, &mm_iri("opClass"));
        assert_eq!(classes.len(), program.steps.len());
        for class in &classes {
            assert!(
                crate::op::OpClass::parse(class).is_some(),
                "unknown op class {class}"
            );
        }
        let scores = literals(&quads, &mm_iri("score"));
        assert_eq!(scores.len(), program.steps.len());
    }

    #[test]
    fn dependency_edges_follow_the_graph() {
        let program = program();
        let quads = program_quads(&program);
        let mut saw_an_edge = false;
        for step in &program.steps {
            let blank = op_label(&program.id, step.node);
            let mut actual = blank_targets_of(&quads, &blank, &mm_iri("opDependsOn"));
            actual.sort();
            let mut expected: Vec<String> = program
                .graph
                .dependencies(step.node)
                .into_iter()
                .map(|dependency| op_label(&program.id, dependency))
                .collect();
            expected.sort();
            assert_eq!(actual, expected, "node {} dependency edges", step.node);
            if !actual.is_empty() {
                saw_an_edge = true;
            }
        }
        assert!(
            saw_an_edge,
            "a compiled program has at least one dependency"
        );
    }

    #[test]
    fn two_programs_never_share_an_op_node() {
        // A blank node label names one node across the whole store, not just inside
        // one insert. Two programs emitted into `/epistemic` that both used `_:op0`
        // would be one subject carrying two classes, two orders and two scores —
        // which is exactly what `mm:CognitiveOpShape` and `mm:OperationValueShape`
        // forbid, so the label has to carry the program's identity.
        let first = program();
        let mut second = first.clone();
        second.id = ulid(99);
        let mut labels: BTreeSet<String> = BTreeSet::new();
        for program in [&first, &second] {
            for step in &program.steps {
                let label = op_label(&program.id, step.node);
                assert!(
                    labels.insert(label.clone()),
                    "{} reused the op label {label}",
                    mm_core::ulid_string(&program.id)
                );
            }
        }
        assert_eq!(labels.len(), first.steps.len() + second.steps.len());
    }

    #[test]
    fn every_op_carries_its_class_its_value_block_and_its_targets() {
        let program = program();
        let quads = program_quads(&program);
        for step in &program.steps {
            let blank = op_label(&program.id, step.node);
            let types: Vec<String> = objects_from_blank(&quads, &blank, &format!("{RDF}type"))
                .into_iter()
                .filter_map(|object| match object {
                    Term::NamedNode(node) => Some(node.as_str().to_string()),
                    _ => None,
                })
                .collect();
            assert!(
                types.contains(&mm_iri("CognitiveOp")),
                "op {blank}: {types:?}"
            );
            assert!(
                types.contains(&mm_iri("OperationValue")),
                "the value block is typed: {types:?}"
            );
            assert_eq!(
                literals(
                    &quads
                        .iter()
                        .filter(|quad| {
                            matches!(&quad.subject, NamedOrBlankNode::BlankNode(node)
                                if node.as_str() == blank)
                        })
                        .cloned()
                        .collect::<Vec<_>>(),
                    &mm_iri("opClass")
                ),
                vec![step.op.class().as_str().to_string()]
            );
            for component in [
                "valueEer",
                "valueImportance",
                "valueChangeProbability",
                "valueCost",
            ] {
                let values: Vec<String> = objects_from_blank(&quads, &blank, &mm_iri(component))
                    .into_iter()
                    .filter_map(|object| match object {
                        Term::Literal(literal) => Some(literal.value().to_string()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(values.len(), 1, "{blank} {component}");
            }
            let targets: Vec<String> = objects_from_blank(&quads, &blank, &mm_iri("targets"))
                .into_iter()
                .filter_map(|object| match object {
                    Term::Literal(literal) => Some(literal.value().to_string()),
                    _ => None,
                })
                .collect();
            assert_eq!(targets.len(), step.op.targets().len());
        }
    }

    #[test]
    fn a_derivation_is_the_only_program_to_episode_link() {
        let program = program();
        let quads = program_quads(&program);
        let node = iri::data(&program.id).into_string();
        let derived = named_targets(&quads, &node, &mm_iri("derivedFrom"));
        assert_eq!(derived, vec![iri::data(&program.episode_id).into_string()]);
    }

    #[test]
    fn the_turtle_rendering_is_deterministic() {
        let episode = episode();
        let first = episode_turtle(&episode, &scan());
        let second = episode_turtle(&episode, &scan());
        assert_eq!(first, second);
        assert!(first.starts_with("@prefix mm:   <https://metamind.dev/ontology#> ."));
        assert!(first.contains("@prefix xsd:"));
        assert!(first.contains("@prefix rdf:"));
        assert!(first.contains("rdf:type mm:CognitiveEpisode"), "{first}");
        assert!(
            first.contains("mm:stakes \"0.800000000\"^^xsd:double"),
            "{first}"
        );

        let program = program();
        let first = program_turtle(&program);
        let second = program_turtle(&program);
        assert_eq!(first, second);
        assert!(first.contains("rdf:type mm:CognitiveProgram"));
        assert!(first.contains("mm:opClass"));
        assert!(first.contains("mm:hasOperation"));
        assert!(first.contains("mm:derivedFrom"));
        assert_eq!(first.lines().count(), program_quads(&program).len() + 5);
    }

    #[test]
    fn a_literal_with_a_quote_or_a_backslash_is_escaped() {
        let mut program = program();
        if let Some(step) = program.steps.first_mut() {
            if let crate::op::CognitiveOp::Recall { query } = &mut step.op {
                *query = "say \"hi\" and \\ ".to_string();
            }
        }
        let text = program_turtle(&program);
        assert!(text.contains("\\\"hi\\\""), "{text}");
        assert!(text.contains("\\\\"), "{text}");
    }
}
