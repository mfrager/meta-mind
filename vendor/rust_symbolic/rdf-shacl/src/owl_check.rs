//! Stage 1 — the OWL validator (rdf_interface_update_plan.md §10.1):
//! structural checks over the loaded ontology hierarchy. Every check is a
//! hard error when violated; warnings are advisory only.

use math_core::model::MathModel;
use math_core::vocabulary::{
    ABS_NS, ANALOGY_NS, CAUS_NS, COMPLEXITY_NS, DECISION_NS, ECONOMY_NS, EPISTEMIC_NS, EVENT_NS,
    EVOLUTION_NS, INFO_NS, LEARNING_NS, LOGIC_NS, MARKET_NS, MECHANISM_NS, MEMORY_NS, MODEL_NS,
    NUMX_NS, PROB_NS, RESOURCE_NS, SOLVER_NS, SYMREG_NS, SYNTHESIS_NS, TEMP_NS, TENS_NS, TOM_NS,
    UNITS_NS,
};
use rdf_codec::io::parse_turtle;
use rdf_codec::NamedNode;

/// The result of checking an ontology.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OntologyReport {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_SUBCLASS: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const RDFS_RANGE: &str = "http://www.w3.org/2000/01/rdf-schema#range";
const OWL_CLASS: &str = "http://www.w3.org/2002/07/owl#Class";
const OWL_RESTRICTION: &str = "http://www.w3.org/2002/07/owl#Restriction";
const OWL_ON_PROPERTY: &str = "http://www.w3.org/2002/07/owl#onProperty";
const OWL_CARDINALITY: &str = "http://www.w3.org/2002/07/owl#cardinality";
const OWL_MIN_CARDINALITY: &str = "http://www.w3.org/2002/07/owl#minCardinality";
const OWL_MAX_CARDINALITY: &str = "http://www.w3.org/2002/07/owl#maxCardinality";
const OWL_FUNCTIONAL: &str = "http://www.w3.org/2002/07/owl#FunctionalProperty";
const OWL_DISJOINT_WITH: &str = "http://www.w3.org/2002/07/owl#disjointWith";

/// All known subsystem namespaces; classes must live in one of them.
const KNOWN_NS: &[&str] = &[
    MODEL_NS,
    UNITS_NS,
    LOGIC_NS,
    NUMX_NS,
    SOLVER_NS,
    PROB_NS,
    CAUS_NS,
    TENS_NS,
    TEMP_NS,
    EVENT_NS,
    ABS_NS,
    INFO_NS,
    COMPLEXITY_NS,
    DECISION_NS,
    EPISTEMIC_NS,
    TOM_NS,
    MECHANISM_NS,
    MARKET_NS,
    LEARNING_NS,
    EVOLUTION_NS,
    ANALOGY_NS,
    SYMREG_NS,
    SYNTHESIS_NS,
    MEMORY_NS,
    RESOURCE_NS,
    ECONOMY_NS,
];

/// Datatype IRIs that are legal `rdfs:range` targets without a class
/// declaration in the hierarchy.
const BUILTIN_RANGES: &[&str] = &[
    "http://www.w3.org/2001/XMLSchema#",
    "http://www.w3.org/2000/01/rdf-schema#Literal",
    "http://www.w3.org/2000/01/rdf-schema#Resource",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#List",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString",
    "http://www.w3.org/2001/XMLSchema#anyURI",
];

/// A named subject for `quads_for_pattern`.
fn subj(iri: &str) -> math_core::oxigraph::model::NamedOrBlankNode {
    math_core::oxigraph::model::NamedNode::new(iri)
        .expect("valid IRI")
        .into()
}

fn type_pred() -> NamedNode {
    NamedNode::new(RDF_TYPE).expect("rdf:type IRI")
}

/// Every class declared in the store: `?c a owl:Class` (unique).
pub fn declared_classes(store: &MathModel) -> Vec<String> {
    let mut out = Vec::new();
    for quad in store
        .store
        .quads_for_pattern(None, Some((&type_pred()).into()), None, None)
        .flatten()
    {
        if let rdf_codec::Term::NamedNode(o) = &quad.object {
            if o.as_str() == OWL_CLASS {
                if let math_core::oxigraph::model::NamedOrBlankNode::NamedNode(s) = &quad.subject {
                    out.push(s.as_str().to_string());
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn local(iri: &str) -> &str {
    iri.rsplit('#').next().unwrap_or(iri)
}

/// Check the ontology hierarchy in `store` against the per-class file list
/// `files` (the same `MODEL_FILES` / `SOLVER_FILES` … tables the loader uses).
pub fn check_ontology(store: &MathModel, files: &[(&str, &str)]) -> OntologyReport {
    let mut report = OntologyReport {
        ok: true,
        ..Default::default()
    };
    let err = |report: &mut OntologyReport, m: String| {
        report.ok = false;
        report.errors.push(m);
    };

    // --- one class per file + duplicate detection ---------------------------
    let mut declared_by_file: Vec<(String, String)> = Vec::new(); // (file stem, class)
    for (stem, ttl) in files {
        if *stem == "properties" || *stem == "index" {
            continue;
        }
        let graph = match parse_turtle(ttl) {
            Ok(g) => g,
            Err(e) => {
                err(&mut report, format!("file `{stem}` does not parse: {e}"));
                continue;
            }
        };
        for t in graph.iter() {
            if t.predicate.as_str() != RDF_TYPE {
                continue;
            }
            if let rdf_codec::TermRef::NamedNode(o) = &t.object {
                if o.as_str() != OWL_CLASS {
                    continue;
                }
            } else {
                continue;
            }
            let class = match &t.subject {
                rdf_codec::NamedOrBlankNodeRef::NamedNode(s) => s.as_str().to_string(),
                _ => continue,
            };
            let l = local(&class);
            if !(l == *stem || l.starts_with(&format!("{stem}/"))) {
                err(
                    &mut report,
                    format!("file `{stem}` declares class `{class}` (one class per file)"),
                );
            }
            declared_by_file.push(((*stem).to_string(), class));
        }
    }
    let mut seen = std::collections::HashMap::<String, String>::new();
    for (stem, class) in &declared_by_file {
        if let Some(first) = seen.insert(class.clone(), stem.clone()) {
            if first != *stem {
                err(
                    &mut report,
                    format!("class `{class}` is declared in both `{first}` and `{stem}`"),
                );
            }
        }
    }

    // --- namespace + vocabulary shape ----------------------------------------
    let declared = declared_classes(store);
    for class in &declared {
        if !KNOWN_NS.iter().any(|ns| class.starts_with(ns)) {
            err(
                &mut report,
                format!("class `{class}` is outside every known subsystem namespace"),
            );
        }
        let l = local(class);
        if l.contains('/') {
            let parent = &l[..l.rfind('/').unwrap()];
            if !declared.iter().any(|c| local(c) == parent) {
                err(
                    &mut report,
                    format!("variant class `{class}` has no declared parent `{parent}`"),
                );
            }
        }
    }

    // --- unknown range classes ------------------------------------------------
    let range_pred = NamedNode::new(RDFS_RANGE).expect("rdfs:range");
    for quad in store
        .store
        .quads_for_pattern(None, Some((&range_pred).into()), None, None)
        .flatten()
    {
        let rdf_codec::Term::NamedNode(o) = &quad.object else {
            continue;
        };
        let range = o.as_str();
        if BUILTIN_RANGES.iter().any(|ns| range.starts_with(ns)) {
            continue;
        }
        if !declared.iter().any(|c| c == range) && !KNOWN_NS.iter().any(|ns| range.starts_with(ns))
        {
            err(
                &mut report,
                format!("property ranges over undeclared class `{range}`"),
            );
        }
    }

    // --- axiom sanity -----------------------------------------------------------
    let sub_pred = NamedNode::new(RDFS_SUBCLASS).expect("rdfs:subClassOf");

    let func_pred = NamedNode::new(OWL_FUNCTIONAL).expect("owl:FunctionalProperty");
    let dis_pred = NamedNode::new(OWL_DISJOINT_WITH).expect("owl:disjointWith");
    let rest_pred = NamedNode::new(RDF_TYPE).expect("rdf:type");

    // Every restriction object must be an owl:Restriction with a single onProperty.
    for quad in store
        .store
        .quads_for_pattern(None, Some((&sub_pred).into()), None, None)
        .flatten()
    {
        let rdf_codec::Term::NamedNode(o) = &quad.object else {
            continue;
        };
        let is_restriction = store
            .store
            .quads_for_pattern(
                Some((&subj(o.as_str())).into()),
                Some((&rest_pred).into()),
                None,
                None,
            )
            .flatten()
            .any(|q| matches!(&q.object, rdf_codec::Term::NamedNode(n) if n.as_str() == OWL_RESTRICTION));
        if !is_restriction {
            continue;
        }
        // A functional property must never carry maxCardinality > 1.
        let mut card: Option<i64> = None;
        let mut min: Option<i64> = None;
        let mut max: Option<i64> = None;
        let mut prop = None;
        for q in store
            .store
            .quads_for_pattern(Some((&subj(o.as_str())).into()), None, None, None)
            .flatten()
        {
            match q.predicate.as_str() {
                p if p == OWL_ON_PROPERTY => {
                    if let rdf_codec::Term::NamedNode(n) = &q.object {
                        prop = Some(n.as_str().to_string());
                    }
                }
                p if p == OWL_CARDINALITY => card = lit_int(&q.object),
                p if p == OWL_MIN_CARDINALITY => min = lit_int(&q.object),
                p if p == OWL_MAX_CARDINALITY => max = lit_int(&q.object),
                _ => {}
            }
        }
        if let Some(p) = &prop {
            if let Some(m) = max {
                if let Some(c) = card {
                    if c > m {
                        err(
                            &mut report,
                            format!("restriction on `{p}`: cardinality {c} > maxCardinality {m}"),
                        );
                    }
                }
                if let Some(mn) = min {
                    if mn > m {
                        err(
                            &mut report,
                            format!(
                                "restriction on `{p}`: minCardinality {mn} > maxCardinality {m}"
                            ),
                        );
                    }
                }
            }
            let is_functional = store
                .store
                .quads_for_pattern(None, Some((&func_pred).into()), None, None)
                .flatten()
                .any(|q| {
                    q.subject
                        == math_core::oxigraph::model::NamedOrBlankNode::NamedNode(
                            rdf_codec::NamedNode::new(p.clone()).expect("iri"),
                        )
                });
            if is_functional {
                let effective_max = max.or(card).unwrap_or(0);
                if effective_max > 1 {
                    err(
                        &mut report,
                        format!("functional property `{p}` also carries maxCardinality > 1"),
                    );
                }
            }
        }
    }

    // disjointWith: symmetric, never self-referential.
    for quad in store
        .store
        .quads_for_pattern(None, Some((&dis_pred).into()), None, None)
        .flatten()
    {
        let (
            math_core::oxigraph::model::NamedOrBlankNode::NamedNode(s),
            rdf_codec::Term::NamedNode(o),
        ) = (&quad.subject, &quad.object)
        else {
            continue;
        };
        if s.as_str() == o.as_str() {
            err(
                &mut report,
                format!("class `{}` is disjointWith itself", s.as_str()),
            );
            continue;
        }
        let back = store
            .store
            .quads_for_pattern(
                Some((&subj(o.as_str())).into()),
                Some((&dis_pred).into()),
                None,
                None,
            )
            .flatten()
            .any(
                |q| matches!(&q.object, rdf_codec::Term::NamedNode(n) if n.as_str() == s.as_str()),
            );
        if !back {
            report.warnings.push(format!(
                "disjointWith is not declared symmetrically for `{}` and `{}`",
                s.as_str(),
                o.as_str()
            ));
        }
    }

    report
}

fn lit_int(t: &rdf_codec::Term) -> Option<i64> {
    match t {
        rdf_codec::Term::Literal(l) => l.value().parse::<i64>().ok(),
        _ => None,
    }
}
