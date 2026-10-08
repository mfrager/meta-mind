//! Stage 2 — the SHACL generator (rdf_interface_update_plan.md §10.2):
//! deterministic OWL → SHACL shapes graph. One `sh:NodeShape` per class
//! (the class IRI doubles as the shape IRI), keyed by `sh:targetClass`;
//! property shapes derive from cardinality restrictions and property
//! declarations (domain/range/functional). Enum sibling variants are pairwise
//! disjoint by construction (`sh:not [ sh:class sibling ]`). Output is sorted
//! canonical triples, so the same ontology always yields identical shapes.
//!
//! Design notes (fidelity vs. §10.2):
//! - **List-valued properties** (`rdfs:range rdf:List`) are validated on their
//!   *members* via the standard member path `( rdf:rest* rdf:first )` instead
//!   of `sh:nodeKind` + recursive `sh:node`: the member path carries the
//!   cardinality constraints (a class restriction `owl:minCardinality 2` on
//!   `model:operands` means "at least 2 members") and the member class, which
//!   is expressed as a second `rdfs:range` on the property (e.g. `model:operands`
//!   ranges over both `rdf:List` and `model:Expression` — the list-ness and the
//!   element type are both explicit OWL). Property-level `owl:FunctionalProperty`
//!   does not translate to `sh:maxCount 1` for list properties (it would
//!   wrongly cap the member count at 1); the codec always emits exactly one
//!   list per such property.
//! - **Subclass awareness**: `rdfs:subClassOf` triples of the declared classes
//!   are copied into the shapes graph so the validator's `sh:class` check can
//!   honour the OWL hierarchy (an instance typed `Variable` conforms to a
//!   shape constrained to `Expression`).
//! - **One shape per (property × restriction set)**: two classes restricting
//!   the same property differently (e.g. `Addition` ≥ 2 operands vs. `Cos`
//!   exactly 1) get distinct property shapes; identical restriction sets share
//!   a shape. Names are deterministic: `{prop}-shape`, `{prop}-shape-2`, … in
//!   sorted order of the distinct restriction sets.

use std::collections::{BTreeMap, BTreeSet};

use crate::owl_check::declared_classes;
use math_core::model::MathModel;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
const RDF_LIST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#List";
const RDFS_SUBCLASS: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const RDFS_DOMAIN: &str = "http://www.w3.org/2000/01/rdf-schema#domain";
const RDFS_RANGE: &str = "http://www.w3.org/2000/01/rdf-schema#range";
const RDFS_LITERAL: &str = "http://www.w3.org/2000/01/rdf-schema#Literal";
const OWL_DATATYPE_PROPERTY: &str = "http://www.w3.org/2002/07/owl#DatatypeProperty";
const OWL_OBJECT_PROPERTY: &str = "http://www.w3.org/2002/07/owl#ObjectProperty";
const OWL_RESTRICTION: &str = "http://www.w3.org/2002/07/owl#Restriction";
const OWL_ON_PROPERTY: &str = "http://www.w3.org/2002/07/owl#onProperty";
const OWL_CARDINALITY: &str = "http://www.w3.org/2002/07/owl#cardinality";
const OWL_MIN_CARDINALITY: &str = "http://www.w3.org/2002/07/owl#minCardinality";
const OWL_MAX_CARDINALITY: &str = "http://www.w3.org/2002/07/owl#maxCardinality";
const OWL_FUNCTIONAL: &str = "http://www.w3.org/2002/07/owl#FunctionalProperty";

const SH: &str = "http://www.w3.org/ns/shacl#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// A named subject for `quads_for_pattern`.
fn subj(iri: &str) -> math_core::oxigraph::model::NamedOrBlankNode {
    math_core::oxigraph::model::NamedNode::new(iri)
        .expect("valid IRI")
        .into()
}

fn type_pred() -> rdf_codec::NamedNode {
    rdf_codec::NamedNode::new(RDF_TYPE).expect("rdf:type")
}

fn local(iri: &str) -> &str {
    iri.rsplit('#').next().unwrap_or(iri)
}

/// Canonical string for a sorted set of (predicate, object) fields.
fn fields_key(fields: &BTreeSet<(String, String)>) -> String {
    fields.iter().map(|(p, o)| format!("{p}={o};")).collect()
}

/// Generate the shapes graph for the ontology in `store`, as sorted Turtle.
pub fn generate_shacl(store: &MathModel) -> Result<String, String> {
    let mut triples: BTreeSet<(String, String, String)> = BTreeSet::new();
    let classes = declared_classes(store);
    let type_p = type_pred();

    // One NodeShape per class, keyed by targetClass.
    for c in &classes {
        triples.insert((c.clone(), RDF_TYPE.to_string(), format!("{SH}NodeShape")));
        triples.insert((c.clone(), format!("{SH}targetClass"), c.clone()));
    }

    // ── Phase A: declared properties (domain/range/functional) ────────────
    // decl_fields:  property -> SHACL constraint fields from the declaration.
    // list_props:   properties whose range includes rdf:List.
    // member_cons:  list properties -> member class/datatype constraints
    //               (the non-List ranges).
    let mut decl_fields: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
    let mut list_props: BTreeSet<String> = BTreeSet::new();
    let mut member_cons: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
    let domain_p = rdf_codec::NamedNode::new(RDFS_DOMAIN).expect("rdfs:domain");
    let range_p = rdf_codec::NamedNode::new(RDFS_RANGE).expect("rdfs:range");
    let func_p = rdf_codec::NamedNode::new(OWL_FUNCTIONAL).expect("owl:FunctionalProperty");
    for prop_kind in [OWL_DATATYPE_PROPERTY, OWL_OBJECT_PROPERTY] {
        for quad in store
            .store
            .quads_for_pattern(None, Some((&type_p).into()), None, None)
            .flatten()
        {
            let (
                math_core::oxigraph::model::NamedOrBlankNode::NamedNode(p),
                rdf_codec::Term::NamedNode(kind),
            ) = (&quad.subject, &quad.object)
            else {
                continue;
            };
            if kind.as_str() != prop_kind {
                continue;
            }
            let p = p.as_str().to_string();
            let mut ranges: Vec<String> = Vec::new();
            for q in store
                .store
                .quads_for_pattern(Some((&subj(&p)).into()), None, None, None)
                .flatten()
            {
                if q.predicate == range_p.as_ref() {
                    if let rdf_codec::Term::NamedNode(n) = &q.object {
                        ranges.push(n.as_str().to_string());
                    }
                }
            }
            let is_list = ranges.iter().any(|r| r == RDF_LIST);
            if is_list {
                list_props.insert(p.clone());
                // The non-List ranges describe the members.
                let mut cons = BTreeSet::new();
                for r in ranges.iter().filter(|r| *r != RDF_LIST) {
                    if r.starts_with(XSD) || r == RDFS_LITERAL {
                        cons.insert((format!("{SH}datatype"), r.clone()));
                    } else if *r != RDFS_DOMAIN && *r != RDFS_LITERAL {
                        cons.insert((format!("{SH}class"), r.clone()));
                    }
                }
                member_cons.insert(p.clone(), cons);
            }
            if !is_list {
                let shape = decl_fields.entry(p.clone()).or_default();
                if let Some(r) = ranges.first() {
                    if r.starts_with(XSD) || *r == RDFS_LITERAL {
                        shape.insert((format!("{SH}datatype"), r.clone()));
                    } else {
                        shape.insert((format!("{SH}class"), r.clone()));
                    }
                }
            }
            let is_functional = store
                .store
                .quads_for_pattern(Some((&subj(&p)).into()), Some((&func_p).into()), None, None)
                .flatten()
                .next()
                .is_some();
            // For list properties `owl:FunctionalProperty` means "exactly one
            // list", not "at most one member" — no maxCount on members.
            if is_functional && !is_list {
                let shape = decl_fields.entry(p.clone()).or_default();
                shape.insert((format!("{SH}maxCount"), "1".to_string()));
                shape.insert((format!("{SH}severity"), format!("{SH}Violation")));
            }
            let _ = &domain_p;
        }
    }

    // ── Phase B: class restrictions ───────────────────────────────────────
    // class_restrictions: class -> (property, min, max) from its restrictions.
    // (class, property, min, max) restriction tuples.
    type Restriction = (String, Option<i64>, Option<i64>);
    let mut class_restrictions: BTreeMap<String, BTreeSet<Restriction>> = BTreeMap::new();
    for c in &classes {
        for quad in store
            .store
            .quads_for_pattern(
                Some((&subj(c)).into()),
                Some((&rdf_codec::NamedNode::new(RDFS_SUBCLASS).expect("subClassOf")).into()),
                None,
                None,
            )
            .flatten()
        {
            let Some(r) = as_subject(&quad.object) else {
                continue;
            };
            let is_restriction = store
                .store
                .quads_for_pattern(
                    Some((&r).into()),
                    Some((&type_p).into()),
                    None,
                    None,
                )
                .flatten()
                .any(|q| matches!(&q.object, rdf_codec::Term::NamedNode(n) if n.as_str() == OWL_RESTRICTION));
            if !is_restriction {
                continue;
            }
            let mut prop = None;
            let mut card = None;
            let mut min = None;
            let mut max = None;
            for q in store
                .store
                .quads_for_pattern(Some((&r).into()), None, None, None)
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
            let Some(p) = prop else { continue };
            class_restrictions.entry(c.clone()).or_default().insert((
                p,
                card.or(min),
                card.or(max),
            ));
        }
    }

    // ── Phase C: one property shape per (property × restriction set) ──────
    // Distinct field sets per property, sorted for deterministic naming.
    let mut field_sets_by_prop: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut shape_fields: BTreeMap<(String, String), BTreeSet<(String, String)>> = BTreeMap::new();
    // (property, fields_key) -> shape name
    let mut shape_names: BTreeMap<(String, String), String> = BTreeMap::new();
    // class -> shape (prop, key) links, for the attachment pass.
    let mut class_shape_links: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
    for (c, restrictions) in &class_restrictions {
        for (prop, min, max) in restrictions {
            let mut fields = decl_fields.get(prop).cloned().unwrap_or_default();
            // member constraints for list properties replace the plain
            // class/datatype (which would constrain the list node itself).
            if list_props.contains(prop) {
                if let Some(cons) = member_cons.get(prop) {
                    fields.extend(cons.iter().cloned());
                }
            }
            if let Some(n) = min {
                fields.insert((format!("{SH}minCount"), n.to_string()));
            }
            if let Some(n) = max {
                fields.insert((format!("{SH}maxCount"), n.to_string()));
            }
            fields.insert((format!("{SH}severity"), format!("{SH}Violation")));
            let key = fields_key(&fields);
            field_sets_by_prop
                .entry(prop.clone())
                .or_default()
                .insert(key.clone());
            shape_fields.insert((prop.clone(), key.clone()), fields);
            class_shape_links
                .entry(c.clone())
                .or_default()
                .insert((prop.clone(), key));
        }
    }
    // Deterministic names: `{prop}-shape`, `{prop}-shape-2`, … by sorted
    // field-set key.
    for (prop, keys) in &field_sets_by_prop {
        let mut sorted: Vec<&String> = keys.iter().collect();
        sorted.sort();
        for (idx, key) in sorted.iter().enumerate() {
            let name = if idx == 0 {
                format!("{prop}-shape")
            } else {
                format!("{prop}-shape-{}", idx + 1)
            };
            shape_names.insert((prop.clone(), (*key).clone()), name);
        }
    }

    // ── Phase D: emit property shapes + attach to classes ─────────────────
    for ((prop, key), name) in &shape_names {
        let mut all = BTreeSet::new();
        all.insert((RDF_TYPE.to_string(), format!("{SH}PropertyShape")));
        if list_props.contains(prop) {
            // List-valued: the value must be an rdf:List (`sh:nodeKind`) whose
            // members are constrained by the auxiliary list shape (which
            // carries the member path `( rdf:rest* rdf:first )`).
            let san = local(prop).replace(['/', '#', ' '], "_");
            let member_shape = format!("{name}-member");
            let list_shape = format!("{name}-list");
            all.insert((format!("{SH}path"), prop.clone()));
            all.insert((format!("{SH}nodeKind"), format!("{SH}BlankNodeOrIRI")));
            all.insert((format!("{SH}node"), list_shape.clone()));
            all.insert((format!("{SH}severity"), format!("{SH}Violation")));
            // The auxiliary list shape (a target-less NodeShape).
            triples.insert((
                list_shape.clone(),
                RDF_TYPE.to_string(),
                format!("{SH}NodeShape"),
            ));
            triples.insert((list_shape, format!("{SH}property"), member_shape.clone()));
            // The member property shape: ( rdf:rest* rdf:first ) + constraints.
            let zom = format!("_:path-{san}");
            let l0 = format!("_:list-{san}-0");
            let l1 = format!("_:list-{san}-1");
            let mut member_fields = BTreeSet::new();
            member_fields.insert((RDF_TYPE.to_string(), format!("{SH}PropertyShape")));
            member_fields.insert((format!("{SH}path"), l0.clone()));
            if let Some(cons) = member_cons.get(prop) {
                member_fields.extend(cons.iter().cloned());
            }
            if let Some(fields) = shape_fields.get(&(prop.clone(), key.clone())) {
                for (p, o) in fields {
                    if p.starts_with(SH) && (p.ends_with("Count") || *p == format!("{SH}severity"))
                    {
                        member_fields.insert((p.clone(), o.clone()));
                    }
                }
            }
            for (p, o) in member_fields {
                triples.insert((member_shape.clone(), p, o));
            }
            triples.insert((
                zom.clone(),
                format!("{SH}zeroOrMorePath"),
                RDF_REST.to_string(),
            ));
            triples.insert((l0.clone(), RDF_FIRST.to_string(), zom));
            triples.insert((l0, RDF_REST.to_string(), l1.clone()));
            triples.insert((l1.clone(), RDF_FIRST.to_string(), RDF_FIRST.to_string()));
            triples.insert((l1, RDF_REST.to_string(), RDF_NIL.to_string()));
        } else {
            all.insert((format!("{SH}path"), prop.clone()));
            all.extend(
                shape_fields
                    .get(&(prop.clone(), key.clone()))
                    .unwrap()
                    .iter()
                    .cloned(),
            );
        }
        for (p, o) in all {
            triples.insert((name.clone(), p, o));
        }
    }
    for (c, links) in &class_shape_links {
        for (prop, key) in links {
            if let Some(name) = shape_names.get(&(prop.clone(), key.clone())) {
                triples.insert((c.clone(), format!("{SH}property"), name.clone()));
            }
        }
    }

    // ── Enum variants: pairwise disjoint by construction ──────────────────
    let mut by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in &classes {
        let l = local(c);
        if let Some(idx) = l.rfind('/') {
            by_parent
                .entry(l[..idx].to_string())
                .or_default()
                .push(c.clone());
        }
    }
    let mut not_counter = 0usize;
    for variants in by_parent.values() {
        for (i, v1) in variants.iter().enumerate() {
            for v2 in variants.iter().skip(i + 1) {
                let not = format!("_:not-{not_counter}");
                not_counter += 1;
                triples.insert((v1.clone(), format!("{SH}not"), not.clone()));
                triples.insert((not.clone(), format!("{SH}class"), v2.clone()));
                let not2 = format!("_:not-{not_counter}");
                not_counter += 1;
                triples.insert((v2.clone(), format!("{SH}not"), not2.clone()));
                triples.insert((not2.clone(), format!("{SH}class"), v1.clone()));
            }
        }
    }

    // ── Subclass triples: copied into the shapes graph for class checks ───
    let sub_pred = rdf_codec::NamedNode::new(RDFS_SUBCLASS).expect("subClassOf");
    for c in &classes {
        for quad in store
            .store
            .quads_for_pattern(
                Some((&subj(c)).into()),
                Some((&sub_pred).into()),
                None,
                None,
            )
            .flatten()
        {
            if let rdf_codec::Term::NamedNode(d) = &quad.object {
                triples.insert((c.clone(), RDFS_SUBCLASS.to_string(), d.as_str().to_string()));
            }
        }
    }

    // ── Deterministic emission ────────────────────────────────────────────
    // Group triples by subject, sorted, Turtle with `;`-separated predicates.
    let mut by_subject: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
    for (s, p, o) in triples {
        by_subject.entry(s).or_default().insert((p, o));
    }
    let mut out = String::from(
        "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
         @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
         @prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
         @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\n",
    );
    for (subject, po) in &by_subject {
        out.push_str(&fmt_term(subject));
        let mut first = true;
        for (p, o) in po {
            out.push_str(if first { " " } else { " ;\n    " });
            first = false;
            out.push_str(&format!("<{p}> {}", fmt_term(o)));
        }
        out.push_str(" .\n\n");
    }
    Ok(out)
}

/// Convert a term to a query subject pattern (named or blank node).
fn as_subject(t: &rdf_codec::Term) -> Option<math_core::oxigraph::model::NamedOrBlankNode> {
    match t {
        rdf_codec::Term::NamedNode(n) => Some(
            math_core::oxigraph::model::NamedOrBlankNode::NamedNode(n.clone()),
        ),
        rdf_codec::Term::BlankNode(b) => Some(
            math_core::oxigraph::model::NamedOrBlankNode::BlankNode(b.clone()),
        ),
        _ => None,
    }
}

fn fmt_term(t: &str) -> String {
    if t.starts_with("_:") {
        t.to_string()
    } else if t.parse::<u64>().is_ok() {
        // Cardinalities are bare integer literals, never IRIs.
        t.to_string()
    } else {
        format!("<{t}>")
    }
}

fn lit_int(t: &rdf_codec::Term) -> Option<i64> {
    match t {
        rdf_codec::Term::Literal(l) => l.value().parse::<i64>().ok(),
        _ => None,
    }
}
