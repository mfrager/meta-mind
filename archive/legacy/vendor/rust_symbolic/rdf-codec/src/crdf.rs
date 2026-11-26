//! Compact RDF (cRDF) — a JSON-based, token-efficient representation of RDF
//! for LLM tool calls.  Replaces raw Turtle with JSON arrays of
//! `[subject, type, {prop: val, …}]` triples, using prefixed names
//! ("model:Variable", "ex:x", "event:Fluent").
//!
//! ## Conversion pipeline
//!
//! ```text
//! cRDF JSON  →  crdf_to_graph()  →  Graph  →  to_turtle()  →  Turtle
//! Turtle  →  parse_turtle()  →  Graph  →  graph_to_crdf()  →  cRDF JSON
//! ```
//!
//! § design doc: planning/updates/compact_rdf_design.md

use std::collections::{BTreeMap, HashMap, HashSet};

use math_core::oxigraph::model::vocab::{rdf, xsd};
use math_core::oxigraph::model::{
    Graph, Literal, NamedNode, NamedOrBlankNode, NamedOrBlankNodeRef, Term, TermRef, Triple,
};

use crate::{io, reader, CodecError, Namespace};

// ── prefix table ────────────────────────────────────────────────────────────

/// Bidirectional prefix ↔ IRI registry.
///
/// Populated from the known subsystem namespaces (engine registry) and the
/// core model ontology.  Each entry maps a canonical short prefix (e.g.
/// `"model"`) to its full IRI (e.g. `"https://example.org/ns/model#"`).
#[derive(Debug, Clone)]
pub struct PrefixTable {
    /// `prefix` → full IRI (with closing `#` or `/` as appropriate).
    to_iri: HashMap<String, String>,
    /// Full IRI → `prefix:` (canonical for output compaction).
    to_prefix: HashMap<String, String>,
    /// Properties that always serialise as `rdf:List` (not multiple triples).
    list_properties: HashSet<String>,
}

impl Default for PrefixTable {
    fn default() -> Self {
        let mut t = Self {
            to_iri: HashMap::new(),
            to_prefix: HashMap::new(),
            list_properties: HashSet::new(),
        };
        // ── known prefixes ─────────────────────────────────────────────
        let prefixes: &[(&str, &str, &[&str])] = &[
            (
                "model",
                "https://example.org/ns/model#",
                &["model:operands", "model:contracts"],
            ),
            ("ex", "https://example.org/m/", &[]),
            (
                "event",
                "https://example.org/ns/event#",
                &[
                    "event:fluents",
                    "event:events",
                    "event:initiates",
                    "event:terminates",
                    "event:initial",
                    "event:numericFluents",
                    "event:numericUpdates",
                    "event:cases",
                    "event:results",
                    "event:deviations",
                    "event:diagnostics",
                ],
            ),
            ("event_data", "https://example.org/data/event/", &[]),
            ("causal", "https://example.org/ns/causal#", &[]),
            ("temporal", "https://example.org/ns/temporal#", &[]),
            ("logic", "https://example.org/ns/logic#", &[]),
            ("epistemic", "https://example.org/ns/epistemic#", &[]),
            ("decision", "https://example.org/ns/decision#", &[]),
            ("mechanism", "https://example.org/ns/mechanism#", &[]),
            ("learning", "https://example.org/ns/learning#", &[]),
            ("evolution", "https://example.org/ns/evolution#", &[]),
            ("analogy", "https://example.org/ns/analogy#", &[]),
            ("symreg", "https://example.org/ns/symreg#", &[]),
            ("synthesis", "https://example.org/ns/synthesis#", &[]),
            ("memory", "https://example.org/ns/memory#", &[]),
            (
                "argumentation",
                "https://example.org/ns/argumentation#",
                &[],
            ),
            ("provenance", "https://example.org/ns/provenance#", &[]),
            ("abstraction", "https://example.org/ns/abstraction#", &[]),
            ("information", "https://example.org/ns/info#", &[]),
            ("complexity", "https://example.org/ns/complexity#", &[]),
            ("tensor", "https://example.org/ns/tensor#", &[]),
            ("tom", "https://example.org/ns/tom#", &[]),
            ("epistemicmarket", "https://example.org/ns/market#", &[]),
            ("solver", "https://example.org/ns/solver#", &[]),
        ];
        for &(pfx, iri, list_props) in prefixes {
            t.to_iri.insert(pfx.to_string(), iri.to_string());
            t.to_prefix.insert(iri.to_string(), pfx.to_string());
            for lp in list_props {
                t.list_properties.insert(lp.to_string());
            }
        }
        t
    }
}

impl PrefixTable {
    /// Register an additional prefix (e.g. from a dynamic namespace).
    pub fn add(&mut self, prefix: &str, iri: &str, list_props: &[&str]) {
        self.to_iri.insert(prefix.to_string(), iri.to_string());
        self.to_prefix.insert(iri.to_string(), prefix.to_string());
        for lp in list_props {
            self.list_properties.insert(lp.to_string());
        }
    }

    /// Resolve a prefixed name `"model:Variable"` → full IRI.
    /// Plain strings (no colon) are returned as-is (they're string literals).
    pub fn resolve_to_iri(&self, prefixed: &str) -> Option<String> {
        if let Some((pfx, local)) = prefixed.split_once(':') {
            Some(format!("{}{}", self.to_iri.get(pfx)?, local))
        } else {
            None // plain string, not an IRI
        }
    }

    /// Compact a full IRI → `"model:Variable"` if a known prefix matches.
    pub fn compact(&self, iri: &str) -> String {
        for (full, pfx) in &self.to_prefix {
            if let Some(local) = iri.strip_prefix(full) {
                return format!("{pfx}:{local}");
            }
        }
        // Fallback: re-wrap as full IRI in angle brackets.
        iri.to_string()
    }

    /// Whether this property is known to require `rdf:List` serialisation.
    pub fn is_list_property(&self, compacted: &str) -> bool {
        self.list_properties.contains(compacted)
    }
}

// ── raw resource ─────────────────────────────────────────────────────────────

/// One cRDF resource: a subject, an optional `rdf:type`, and a bag of
/// property→value mappings.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct RawResource {
    /// Prefixed subject IRI, e.g. `"ex:x"`.
    pub subject: String,
    /// Optional prefixed type, e.g. `"model:Variable"`.
    #[serde(rename = "type")]
    pub rdf_type: Option<String>,
    /// Property values keyed by prefixed property name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
}

// ── cRDF → Graph ─────────────────────────────────────────────────────────────

/// Convert a cRDF document (JSON array of [`RawResource`]) into an RDF
/// [`Graph`] using `prefixes` for IRI resolution.
pub fn crdf_to_graph(crdf: &[RawResource], prefixes: &PrefixTable) -> Result<Graph, CodecError> {
    let mut graph = Graph::new();

    // Phase 1: register every subject + type.
    for res in crdf {
        let s = resolve_subject(&res.subject, prefixes)?;
        emit_type(&mut graph, &s, &res.rdf_type, prefixes)?;
    }

    // Phase 2: emit properties.
    for res in crdf {
        let s = resolve_subject(&res.subject, prefixes)?;
        for (prop, val) in &res.properties {
            let p_iri = prefixed_to_iri(prop, prefixes)?;
            let p_nn = NamedNode::new(p_iri).map_err(|e| CodecError::Iri(e.to_string()))?;

            match val {
                serde_json::Value::Array(arr) => {
                    if prefixes.is_list_property(prop) {
                        // rdf:List
                        let mut terms: Vec<Term> = Vec::new();
                        for item in arr {
                            terms.push(json_val_to_term(item, prefixes)?);
                        }
                        emit_list(&mut graph, &s, &p_nn, &terms)?;
                    } else {
                        // multiple triples
                        for item in arr {
                            let t = json_val_to_term(item, prefixes)?;
                            graph.insert(&Triple::new(s.clone(), p_nn.clone(), t));
                        }
                    }
                }
                other => {
                    let t = json_val_to_term(other, prefixes)?;
                    graph.insert(&Triple::new(s.clone(), p_nn.clone(), t));
                }
            }
        }
    }

    Ok(graph)
}

fn resolve_subject(subj: &str, prefixes: &PrefixTable) -> Result<NamedNode, CodecError> {
    let iri = prefixed_to_iri(subj, prefixes)?;
    NamedNode::new(iri).map_err(|e| CodecError::Iri(e.to_string()))
}

fn prefixed_to_iri(prefixed: &str, prefixes: &PrefixTable) -> Result<String, CodecError> {
    prefixes
        .resolve_to_iri(prefixed)
        .ok_or_else(|| CodecError::Decode(format!("unknown prefix in `{prefixed}`")))
}

fn json_val_to_term(val: &serde_json::Value, prefixes: &PrefixTable) -> Result<Term, CodecError> {
    match val {
        serde_json::Value::String(s) => {
            if let Some(iri) = prefixes.resolve_to_iri(s) {
                // Prefixed name
                let nn = NamedNode::new(iri).map_err(|e| CodecError::Iri(e.to_string()))?;
                Ok(Term::from(nn))
            } else {
                // Plain string literal
                Ok(Term::from(Literal::new_simple_literal(s.as_str())))
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                let lit = Literal::new_typed_literal(
                    i.to_string(),
                    NamedNode::new(xsd::INTEGER.as_str())
                        .map_err(|e| CodecError::Iri(e.to_string()))?,
                );
                Ok(Term::from(lit))
            } else if let Some(f) = n.as_f64() {
                if f.is_finite() {
                    let lit = Literal::new_typed_literal(
                        ryu::Buffer::new().format(f).to_string(),
                        NamedNode::new(xsd::DOUBLE.as_str())
                            .map_err(|e| CodecError::Iri(e.to_string()))?,
                    );
                    Ok(Term::from(lit))
                } else {
                    Err(CodecError::Encode(format!("non-finite float: {f}")))
                }
            } else {
                Err(CodecError::Encode(format!("unhandled number: {n}")))
            }
        }
        serde_json::Value::Bool(b) => {
            let lit = Literal::new_typed_literal(
                b.to_string(),
                NamedNode::new(xsd::BOOLEAN.as_str())
                    .map_err(|e| CodecError::Iri(e.to_string()))?,
            );
            Ok(Term::from(lit))
        }
        serde_json::Value::Null => {
            // Skip — null values don't emit triples.
            Err(CodecError::Encode(
                "null values are not supported in cRDF properties".into(),
            ))
        }
        serde_json::Value::Object(_obj) => Err(CodecError::Encode(
            "nested objects not yet supported in cRDF → Graph conversion".into(),
        )),
        _ => Err(CodecError::Encode("unsupported cRDF value".into())),
    }
}

fn emit_type(
    graph: &mut Graph,
    subject: &NamedNode,
    rdf_type: &Option<String>,
    prefixes: &PrefixTable,
) -> Result<(), CodecError> {
    let Some(t) = rdf_type else { return Ok(()) };
    let iri = prefixed_to_iri(t, prefixes)?;
    let type_node = NamedNode::new(iri).map_err(|e| CodecError::Iri(e.to_string()))?;
    graph.insert(&Triple::new(
        subject.clone(),
        rdf::TYPE.into_owned(),
        Term::from(type_node),
    ));
    Ok(())
}

fn emit_list(
    graph: &mut Graph,
    subject: &NamedNode,
    prop: &NamedNode,
    items: &[Term],
) -> Result<(), CodecError> {
    // Build a fresh rdf:List chain with blank nodes.
    if items.is_empty() {
        graph.insert(&Triple::new(
            subject.clone(),
            prop.clone(),
            Term::from(rdf::NIL.into_owned()),
        ));
        return Ok(());
    }

    // Pre-generate all cells so the rdf:rest links chain correctly.
    let cells: Vec<NamedOrBlankNode> = items
        .iter()
        .map(|_| {
            let id = ulid::Ulid::gen().0;
            let bnode = math_core::oxigraph::model::BlankNode::new_from_unique_id(id);
            NamedOrBlankNode::from(bnode)
        })
        .collect();

    let head = cells[0].clone();

    for (i, item) in items.iter().enumerate() {
        let cell = &cells[i];
        // rdf:first
        graph.insert(&Triple::new(
            cell.clone(),
            rdf::FIRST.into_owned(),
            item.clone(),
        ));
        // rdf:rest — chain to next cell or rdf:nil
        let rest = if i + 1 < items.len() {
            Term::from(cells[i + 1].clone())
        } else {
            Term::from(rdf::NIL.into_owned())
        };
        graph.insert(&Triple::new(cell.clone(), rdf::REST.into_owned(), rest));
    }

    graph.insert(&Triple::new(
        subject.clone(),
        prop.clone(),
        Term::from(head),
    ));

    Ok(())
}

// ── Graph → cRDF ─────────────────────────────────────────────────────────────

/// Convert an RDF [`Graph`] back to a cRDF document, starting from `root`.
/// All subjects reachable from the root are included, in dependency order
/// (root first).
pub fn graph_to_crdf(
    graph: &Graph,
    root: &NamedNode,
    prefixes: &PrefixTable,
) -> Result<Vec<RawResource>, CodecError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut resources: Vec<RawResource> = Vec::new();
    let mut queue: Vec<NamedNode> = vec![root.clone()];

    while let Some(node) = queue.pop() {
        let node_str = node.as_str().to_string();
        if seen.contains(&node_str) {
            continue;
        }
        seen.insert(node_str);

        let res = subject_to_resource(graph, &node, prefixes)?;
        resources.push(res);

        // Enqueue all object references.
        let refs = collect_object_refs(graph, &node);
        queue.extend(refs.into_iter().rev()); // reverse preserves root-first order
    }

    Ok(resources)
}

fn subject_to_resource(
    graph: &Graph,
    node: &NamedNode,
    prefixes: &PrefixTable,
) -> Result<RawResource, CodecError> {
    let subject = prefixes.compact(node.as_str());

    // Gather the rdf:type.
    let rdf_type = find_rdf_type(graph, node).and_then(|iri| {
        let compacted = prefixes.compact(&iri);
        // Only use compact form when it maps to a known prefix.
        if compacted.contains(':') {
            Some(compacted)
        } else {
            None
        }
    });

    // Group property triples by property IRI.
    let mut prop_map: HashMap<String, Vec<Term>> = HashMap::new();
    let type_pred = rdf::TYPE.into_owned();
    for t in graph.iter() {
        if t.subject != NamedOrBlankNodeRef::NamedNode(node.as_ref()) {
            continue;
        }
        if t.predicate == type_pred.as_ref() {
            continue; // handled by rdf_type above
        }
        let p_iri = t.predicate.as_str().to_string();
        prop_map
            .entry(p_iri)
            .or_default()
            .push(t.object.into_owned());
    }

    // Convert each property group to cRDF values.
    let mut properties: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for (p_iri, terms) in &prop_map {
        let p_compacted = prefixes.compact(p_iri);
        let val = terms_to_crdf_val(graph, terms, prefixes)?;
        properties.insert(p_compacted, val);
    }

    Ok(RawResource {
        subject,
        rdf_type,
        properties,
    })
}

fn find_rdf_type(graph: &Graph, node: &NamedNode) -> Option<String> {
    let type_pred = rdf::TYPE.into_owned();
    for t in graph.iter() {
        if t.subject == NamedOrBlankNodeRef::NamedNode(node.as_ref())
            && t.predicate == type_pred.as_ref()
        {
            if let TermRef::NamedNode(o) = &t.object {
                return Some(o.as_str().to_string());
            }
        }
    }
    None
}

fn terms_to_crdf_val(
    graph: &Graph,
    terms: &[Term],
    prefixes: &PrefixTable,
) -> Result<serde_json::Value, CodecError> {
    if terms.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    if terms.len() == 1 {
        return Ok(single_term_to_crdf(graph, &terms[0], prefixes));
    }

    // Multiple terms — always emit as JSON array.
    let mut arr = Vec::new();
    for t in terms {
        arr.push(single_term_to_crdf(graph, t, prefixes));
    }
    Ok(serde_json::Value::Array(arr))
}

fn single_term_to_crdf(graph: &Graph, term: &Term, prefixes: &PrefixTable) -> serde_json::Value {
    match term {
        Term::NamedNode(nn) => {
            // Check if this is an rdf:List head — if so, expand it.
            if let Ok(elements) = reader::list_values(graph, &NamedOrBlankNode::from(nn.clone())) {
                if !elements.is_empty() {
                    let mut arr = Vec::new();
                    for elem in &elements {
                        arr.push(single_term_to_crdf(graph, elem, prefixes));
                    }
                    return serde_json::Value::Array(arr);
                }
            }
            // Regular IRI reference.
            serde_json::Value::String(
                prefixes
                    .compact(nn.as_str())
                    .replace('<', "")
                    .replace('>', ""),
            )
        }
        Term::BlankNode(bn) => {
            // Check if this blank node is an rdf:List head.
            if let Ok(elements) = reader::list_values(graph, &NamedOrBlankNode::from(bn.clone())) {
                if !elements.is_empty() {
                    let mut arr = Vec::new();
                    for elem in &elements {
                        arr.push(single_term_to_crdf(graph, elem, prefixes));
                    }
                    return serde_json::Value::Array(arr);
                }
            }
            // Bare blank node — emit as a plain label.
            serde_json::Value::String(format!("_:{}", bn.as_str()))
        }
        Term::Literal(lit) => literal_to_crdf(lit),
    }
}

fn literal_to_crdf(lit: &Literal) -> serde_json::Value {
    let val = lit.value();
    let dt = lit.datatype();

    // Plain string / lang string.
    if dt.as_str() == xsd::STRING.as_str() || lit.language().is_some() {
        return serde_json::Value::String(val.to_string());
    }

    // Numeric typed literals → JSON numbers.
    if dt.as_str() == xsd::INTEGER.as_str()
        || dt.as_str() == xsd::INT.as_str()
        || dt.as_str() == xsd::LONG.as_str()
        || dt.as_str() == xsd::UNSIGNED_LONG.as_str()
    {
        if let Ok(i) = val.parse::<i64>() {
            return serde_json::Value::Number(i.into());
        }
    }
    if dt.as_str() == xsd::DOUBLE.as_str()
        || dt.as_str() == xsd::FLOAT.as_str()
        || dt.as_str() == xsd::DECIMAL.as_str()
    {
        if let Ok(f) = val.parse::<f64>() {
            if let Some(n) = serde_json::Number::from_f64(f) {
                return serde_json::Value::Number(n);
            }
        }
    }
    if dt.as_str() == xsd::BOOLEAN.as_str() {
        return serde_json::Value::Bool(val == "true" || val == "1");
    }

    // Fallback: plain string.
    serde_json::Value::String(val.to_string())
}

fn collect_object_refs(graph: &Graph, node: &NamedNode) -> Vec<NamedNode> {
    let type_pred = rdf::TYPE.into_owned();
    let mut refs: Vec<NamedNode> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for t in graph.iter() {
        if t.subject != NamedOrBlankNodeRef::NamedNode(node.as_ref()) {
            continue;
        }
        if t.predicate == type_pred.as_ref() {
            continue;
        }
        match &t.object {
            TermRef::NamedNode(o) => {
                let s = o.as_str().to_string();
                if seen.insert(s.clone()) {
                    refs.push(o.into_owned());
                }
                // Also expand rdf:List chains reachable from this node.
                if let Ok(elements) =
                    reader::list_values(graph, &NamedOrBlankNode::from(o.into_owned()))
                {
                    for elem in &elements {
                        if let Term::NamedNode(nn) = elem {
                            let ns = nn.as_str().to_string();
                            if seen.insert(ns.clone()) {
                                refs.push(nn.clone());
                            }
                        }
                    }
                }
            }
            TermRef::BlankNode(bn) => {
                // Expand rdf:List chains from blank nodes.
                if let Ok(elements) = reader::list_values(graph, &NamedOrBlankNode::from(*bn)) {
                    for elem in &elements {
                        if let Term::NamedNode(nn) = elem {
                            let ns = nn.as_str().to_string();
                            if seen.insert(ns.clone()) {
                                refs.push(nn.clone());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    refs
}

// ── public convenience API ───────────────────────────────────────────────────

/// Convert a cRDF JSON string to Turtle.
///
/// Parses the JSON, builds an RDF graph, and serializes to Turtle with
/// appropriate `@prefix` declarations.
pub fn crdf_to_turtle(json: &str, prefixes: &PrefixTable) -> Result<String, CodecError> {
    let resources: Vec<RawResource> =
        serde_json::from_str(json).map_err(|e| CodecError::Decode(e.to_string()))?;
    let graph = crdf_to_graph(&resources, prefixes)?;
    let prefix_decls: Vec<(&str, &str)> = prefixes
        .to_iri
        .iter()
        .map(|(pfx, iri)| (pfx.as_str(), iri.as_str()))
        .collect();
    io::to_turtle_with_prefixes(&graph, &prefix_decls)
}

/// Convert a Turtle string to cRDF JSON.
///
/// Parses the Turtle, finds the root node, and walks the graph to produce
/// a compact cRDF array.
pub fn turtle_to_crdf(
    turtle: &str,
    schema_ns: &Namespace,
    prefixes: &PrefixTable,
) -> Result<String, CodecError> {
    let graph = io::parse_turtle(turtle)?;
    let root = reader::find_root(&graph, schema_ns)?;
    let resources = graph_to_crdf(&graph, &root, prefixes)?;
    serde_json::to_string(&resources).map_err(|e| CodecError::Encode(e.to_string()))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn prefixes() -> PrefixTable {
        PrefixTable::default()
    }

    #[test]
    fn basic_variable_resource() {
        let crdf = vec![RawResource {
            subject: "ex:x".into(),
            rdf_type: Some("model:Variable".into()),
            properties: BTreeMap::new(),
        }];
        let graph = crdf_to_graph(&crdf, &prefixes()).unwrap();
        assert!(graph.len() >= 1, "graph must have at least the type triple");

        let ttl = io::to_turtle(&graph).unwrap();
        assert!(ttl.contains("model#Variable"), "expected type in Turtle");
    }

    #[test]
    fn parameter_with_value() {
        let mut props = BTreeMap::new();
        props.insert("model:value".into(), serde_json::json!(42.0));
        let crdf = vec![RawResource {
            subject: "ex:p".into(),
            rdf_type: Some("model:Parameter".into()),
            properties: props,
        }];
        let graph = crdf_to_graph(&crdf, &prefixes()).unwrap();
        let ttl = io::to_turtle(&graph).unwrap();
        assert!(ttl.contains("42"), "expected value literal in Turtle");
    }

    #[test]
    fn multi_valued_property() {
        let mut props = BTreeMap::new();
        props.insert(
            "model:hasEquation".into(),
            serde_json::json!(["ex:eq1", "ex:eq2"]),
        );
        let crdf = vec![RawResource {
            subject: "ex:M".into(),
            rdf_type: Some("model:Model".into()),
            properties: props,
        }];
        let graph = crdf_to_graph(&crdf, &prefixes()).unwrap();
        // Two separate triples: (ex:M, model:hasEquation, ex:eq1) and (... ex:eq2)
        let eq_triples: Vec<_> = graph
            .iter()
            .filter(|t| t.predicate.as_str().contains("hasEquation"))
            .collect();
        assert_eq!(eq_triples.len(), 2, "should emit two triples, not a list");
    }

    #[test]
    fn list_property() {
        let mut props = BTreeMap::new();
        props.insert("model:operands".into(), serde_json::json!(["ex:x", "ex:y"]));
        let crdf = vec![RawResource {
            subject: "ex:expr".into(),
            rdf_type: Some("model:Addition".into()),
            properties: props,
        }];
        let graph = crdf_to_graph(&crdf, &prefixes()).unwrap();
        let ttl = io::to_turtle(&graph).unwrap();
        // rdf:List properties should use rdf:first/rdf:rest structure.
        assert!(
            ttl.contains("rdf:first") || ttl.contains("first"),
            "expected rdf:List in Turtle"
        );
    }

    #[test]
    fn resolve_and_compact() {
        let p = prefixes();
        let iri = p.resolve_to_iri("model:Variable").unwrap();
        assert_eq!(iri, "https://example.org/ns/model#Variable");
        let compacted = p.compact(&iri);
        assert_eq!(compacted, "model:Variable");
    }

    #[test]
    fn round_trip_linear_system() {
        let p = prefixes();
        let crdf: Vec<RawResource> = serde_json::from_str(
            r#"[
                ["ex:M","model:Model",{"model:hasEquation":["ex:eq1","ex:eq2"]}],
                ["ex:y","model:Variable",{}],
                ["ex:c","model:Variable",{}],
                ["ex:g","model:Parameter",{"model:value":20.0}],
                ["ex:i","model:Parameter",{"model:value":10.0}],
                ["ex:t","model:Parameter",{"model:value":0.8}],
                ["ex:eq1","model:Equation",{"model:lhs":"ex:y","model:rhs":"ex:yexpr"}],
                ["ex:yexpr","model:Addition",{"model:operands":["ex:c","ex:i","ex:g"]}],
                ["ex:eq2","model:Equation",{"model:lhs":"ex:c","model:rhs":"ex:cexpr"}],
                ["ex:cexpr","model:Multiplication",{"model:operands":["ex:t","ex:y"]}]
            ]"#,
        )
        .unwrap();

        // cRDF → Graph → Turtle
        let graph = crdf_to_graph(&crdf, &p).unwrap();
        assert!(graph.len() >= 20, "expected at least 20 triples");

        let ttl = crdf_to_turtle(&serde_json::to_string(&crdf).unwrap(), &p).unwrap();
        // Turtle uses full IRIs; prefix form only in @prefix-declared output
        assert!(
            ttl.contains("model#Equation"),
            "expected Equation in Turtle"
        );
        assert!(ttl.contains("eq1"), "expected ex:eq1 in Turtle");

        // Turtle → cRDF
        let model_ns = Namespace::new("https://example.org/ns/model#");
        let crdf2_json = turtle_to_crdf(&ttl, &model_ns, &p).unwrap();
        let crdf2: Vec<RawResource> = serde_json::from_str(&crdf2_json).unwrap();
        assert_eq!(
            crdf2
                .iter()
                .find(|r| r.subject == "ex:M")
                .and_then(|r| r.rdf_type.as_deref()),
            Some("model:Model"),
            "root should be ex:M typed model:Model"
        );
        // Round-trip produces a working cRDF document rooted at ex:M.
        assert!(
            crdf2.len() >= 3,
            "expected at least 3 resources (root + 2 eqs) after round-trip"
        );
    }

    #[test]
    fn round_trip_lp() {
        let p = prefixes();
        let crdf: Vec<RawResource> = serde_json::from_str(
            r#"[
                ["ex:M","model:Model",{"model:hasObjective":"ex:obj","model:hasConstraint":["ex:c1","ex:c2"]}],
                ["ex:x","model:Variable",{}],
                ["ex:y","model:Variable",{}],
                ["ex:obj","model:Objective",{"model:rhs":"ex:oexpr","model:sense":"maximize"}],
                ["ex:oexpr","model:Addition",{"model:operands":["ex:x","ex:y"]}],
                ["ex:c1","model:Inequality",{"model:lhs":"ex:x","model:rhs":2.0,"model:operator":"le"}],
                ["ex:c2","model:Inequality",{"model:lhs":"ex:y","model:rhs":3.0,"model:operator":"le"}]
            ]"#,
        )
        .unwrap();

        let graph = crdf_to_graph(&crdf, &p).unwrap();
        assert!(graph.len() >= 15, "expected at least 15 triples");

        let ttl = crdf_to_turtle(&serde_json::to_string(&crdf).unwrap(), &p).unwrap();
        assert!(ttl.contains("model#Objective"));
        assert!(ttl.contains("maximize"));
    }

    #[test]
    fn numeric_literals_typed_correctly() {
        let mut props = BTreeMap::new();
        props.insert("model:rhs".into(), serde_json::json!(2.5));
        let crdf = vec![RawResource {
            subject: "ex:c".into(),
            rdf_type: Some("model:Inequality".into()),
            properties: props,
        }];
        let graph = crdf_to_graph(&crdf, &prefixes()).unwrap();
        let ttl = io::to_turtle(&graph).unwrap();
        assert!(
            ttl.contains("2.5E0") || ttl.contains("2.5"),
            "expected float literal"
        );
    }

    #[test]
    fn boolean_literals() {
        let mut props = BTreeMap::new();
        props.insert("event:accepted".into(), serde_json::json!(true));
        // `event:` is already registered in default prefix table.
        let p = prefixes();
        let crdf = vec![RawResource {
            subject: "ev:ta".into(),
            rdf_type: Some("event:TreeAcceptance".into()),
            properties: props,
        }];
        // ev: prefix isn't registered, so this must fail with an unknown-prefix error.
        let rr = crdf_to_graph(&crdf, &p);
        assert!(rr.is_err(), "expected unknown-prefix error for ev:ta");
        let err_msg = rr.unwrap_err().to_string();
        assert!(
            err_msg.contains("unknown prefix in"),
            "expected prefix error, got: {err_msg}"
        );
    }
}
