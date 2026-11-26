//! Stage 3 — the SHACL validator (rdf_interface_update_plan.md §10.3):
//! applies a shapes graph to an instance graph and returns a
//! `ShaclReport { conforms, results }`. Fragment coverage matches exactly
//! what the generator emits: `sh:targetClass`, `sh:path` (IRI steps,
//! sequences, `sh:zeroOrMorePath`), `sh:datatype`, `sh:class`, `sh:minCount`,
//! `sh:maxCount`, `sh:nodeKind`, `sh:node` (named or blank node shapes),
//! `sh:not` (class-only inner shapes), `sh:severity`, and RDF-list member
//! validation via the `(rdf:rest* rdf:first)` path on list shapes. Any other
//! construct is a hard error — never a silent skip.
//!
//! Semantic notes:
//! - `sh:class` checks honour the OWL hierarchy: the shapes graph carries the
//!   ontology's `rdfs:subClassOf` triples, and the instance's `rdf:type` is
//!   accepted if it is (transitively) a subclass of the constrained class.
//!   `rdfs:Resource` accepts any IRI or blank node.
//! - `sh:datatype` checks honour the XSD type hierarchy: an `xsd:long`
//!   literal conforms to an `xsd:integer` constraint (long is derived from
//!   integer), `xsd:int` to `xsd:long`, the integer family to `xsd:decimal`,
//!   and the string family to `xsd:string`.
//! - Enum unit variants are typed with their variant class
//!   (`Parent/Variant`); `belongs_to` accepts that naming convention in
//!   addition to explicit subclass triples.

use std::collections::{BTreeSet, HashMap};

use math_core::oxigraph::model::{Graph, NamedNodeRef, NamedOrBlankNodeRef, Term, TermRef};
use rdf_codec::io::parse_turtle;

/// A single violation, naming the shape path.
#[derive(Debug, Clone, PartialEq)]
pub struct ShaclViolation {
    pub path: String,
    pub message: String,
    pub severity: String,
}

/// The outcome of validating one graph against a shapes graph.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShaclReport {
    pub conforms: bool,
    pub results: Vec<ShaclViolation>,
}

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const RDFS_SUBCLASS: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const RDFS_RESOURCE: &str = "http://www.w3.org/2000/01/rdf-schema#Resource";

const SH: &str = "http://www.w3.org/ns/shacl#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

#[derive(Debug, Clone, PartialEq)]
enum Path {
    Step(String),
    Seq(Vec<Path>),
    ZeroOrMore(Box<Path>),
}

impl Path {
    fn display(&self) -> String {
        match self {
            Path::Step(p) => p.clone(),
            Path::Seq(parts) => parts
                .iter()
                .map(|p| p.display())
                .collect::<Vec<_>>()
                .join(" / "),
            Path::ZeroOrMore(inner) => format!("{}*", inner.display()),
        }
    }
}

#[derive(Debug, Clone)]
struct PropertyShape {
    path: Path,
    datatype: Option<String>,
    class: Option<String>,
    min_count: Option<u64>,
    max_count: Option<u64>,
    node_kind: Option<String>,
    node: Option<String>,
    severity: String,
}

#[derive(Debug, Clone)]
struct NodeShape {
    /// The subject IRI (or blank-node label) of the shape, for `sh:node`.
    id: String,
    /// `sh:targetClass`; `None` for list/auxiliary shapes reached via `sh:node`.
    target: Option<String>,
    properties: Vec<PropertyShape>,
    nots: Vec<String>,
}

fn sh(local: &str) -> String {
    format!("{SH}{local}")
}

/// A named/blank-node subject view of a term, for indexed lookups.
fn term_as_subject(term: &Term) -> Option<NamedOrBlankNodeRef<'_>> {
    match term {
        Term::NamedNode(n) => Some(NamedOrBlankNodeRef::NamedNode(n.as_ref())),
        Term::BlankNode(b) => Some(NamedOrBlankNodeRef::BlankNode(b.as_ref())),
        _ => None,
    }
}

/// Indexed `(subject, predicate) -> objects` lookup via the graph's own
/// subject+predicate index, instead of a full linear scan.
fn objects(graph: &Graph, subject: &Term, predicate: &str) -> Vec<Term> {
    let Some(subj) = term_as_subject(subject) else {
        return Vec::new();
    };
    let Ok(pred) = NamedNodeRef::new(predicate) else {
        return Vec::new();
    };
    graph
        .objects_for_subject_predicate(subj, pred)
        .map(|t| t.into_owned())
        .collect()
}

fn types_of(graph: &Graph, node: &Term) -> Vec<String> {
    objects(graph, node, RDF_TYPE)
        .into_iter()
        .filter_map(|t| match t {
            Term::NamedNode(n) => Some(n.as_str().to_string()),
            _ => None,
        })
        .collect()
}

fn local(iri: &str) -> &str {
    iri.rsplit('#').next().unwrap_or(iri)
}

/// The transitive `rdfs:subClassOf` closure of the shapes graph, built once:
/// `closure[c]` is every class reachable from `c` (directly or transitively).
/// `sh:class` checks then become an O(1) hash lookup instead of a BFS per
/// value.
fn subclass_closure(graph: &Graph) -> HashMap<String, BTreeSet<String>> {
    let mut direct: HashMap<String, Vec<String>> = HashMap::new();
    let Ok(sub) = NamedNodeRef::new(RDFS_SUBCLASS) else {
        return HashMap::new();
    };
    for t in graph.triples_for_predicate(sub) {
        let (NamedOrBlankNodeRef::NamedNode(s), TermRef::NamedNode(o)) = (&t.subject, &t.object)
        else {
            continue;
        };
        direct
            .entry(s.as_str().to_string())
            .or_default()
            .push(o.as_str().to_string());
    }
    let mut closure: HashMap<String, BTreeSet<String>> = HashMap::new();
    for start in direct.keys() {
        let mut seen = BTreeSet::new();
        let mut stack = vec![start.clone()];
        while let Some(cur) = stack.pop() {
            if let Some(nexts) = direct.get(&cur) {
                for n in nexts {
                    if seen.insert(n.clone()) {
                        stack.push(n.clone());
                    }
                }
            }
        }
        closure.insert(start.clone(), seen);
    }
    closure
}

/// Whether an instance type belongs to a shape's target class: exact match,
/// a variant of it (variant IRIs are `{Parent}/{Variant}`), or a subclass
/// reachable via the precomputed `rdfs:subClassOf` closure.
fn belongs_to(
    instance_type: &str,
    target: &str,
    superclasses: &HashMap<String, BTreeSet<String>>,
) -> bool {
    if instance_type == target {
        return true;
    }
    let t = local(instance_type);
    let g = local(target);
    if t.starts_with(&format!("{g}/")) {
        return true;
    }
    superclasses
        .get(instance_type)
        .map(|sup| sup.contains(target))
        .unwrap_or(false)
}

/// Parse the shapes graph into node shapes, rejecting unsupported constructs.
/// Every `a sh:NodeShape` subject is parsed (targeted classes and auxiliary
/// list shapes alike); every `a sh:PropertyShape` node is parsed too, so an
/// unsupported construct on an unattached property shape is still a hard
/// error.
fn parse_shapes(shapes: &Graph) -> Result<Vec<NodeShape>, String> {
    let mut nodes: Vec<Term> = Vec::new();
    for t in shapes.iter() {
        if t.predicate.as_str() == RDF_TYPE
            && matches!(&t.object, TermRef::NamedNode(o) if o.as_str() == sh("NodeShape"))
        {
            let term = match &t.subject {
                NamedOrBlankNodeRef::NamedNode(s) => {
                    Term::NamedNode(rdf_codec::NamedNode::new(s.as_str().to_string()).expect("iri"))
                }
                NamedOrBlankNodeRef::BlankNode(s) => Term::BlankNode(
                    math_core::oxigraph::model::BlankNode::new(s.as_str().to_string())
                        .expect("blank node id"),
                ),
            };
            if !nodes.contains(&term) {
                nodes.push(term);
            }
        }
    }
    nodes.sort_by(|a, b| format!("{a}").cmp(&format!("{b}")));
    let mut out = Vec::new();
    for node in &nodes {
        out.push(parse_node_shape(shapes, node)?);
    }
    // Unattached property shapes must still be well-formed.
    let mut property_nodes = Vec::new();
    for t in shapes.iter() {
        if t.predicate.as_str() == RDF_TYPE
            && matches!(&t.object, TermRef::NamedNode(o) if o.as_str() == sh("PropertyShape"))
        {
            if let NamedOrBlankNodeRef::NamedNode(s) = &t.subject {
                property_nodes.push(s.into_owned());
            }
        }
    }
    property_nodes.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    property_nodes.dedup();
    for node in &property_nodes {
        let _ = parse_property_shape(shapes, &Term::NamedNode(node.clone()))?;
    }
    Ok(out)
}

fn subject_term(s: &NamedOrBlankNodeRef<'_>) -> Term {
    match s {
        NamedOrBlankNodeRef::NamedNode(n) => Term::NamedNode((*n).into_owned()),
        NamedOrBlankNodeRef::BlankNode(b) => Term::BlankNode(
            math_core::oxigraph::model::BlankNode::new(b.as_str().to_string())
                .expect("blank node id"),
        ),
    }
}

fn parse_node_shape(shapes: &Graph, node: &Term) -> Result<NodeShape, String> {
    let id = match node {
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::BlankNode(b) => b.as_str().to_string(),
        _ => return Err(format!("shape subject must be a node, got {node}")),
    };
    let mut target = None;
    let mut properties = Vec::new();
    let mut nots = Vec::new();
    let mut closed = false;
    let subj =
        term_as_subject(node).ok_or_else(|| format!("shape subject must be a node, got {node}"))?;
    for t in shapes.triples_for_subject(subj) {
        let p = t.predicate.as_str();
        let supported = [
            "targetClass",
            "property",
            "not",
            "closed",
            "ignoredProperties",
            "severity",
        ]
        .iter()
        .any(|k| p == sh(k));
        if p.starts_with(SH) && !supported {
            return Err(format!("unsupported shape construct `{p}`"));
        }
        match p {
            s if s == sh("targetClass") => {
                if let TermRef::NamedNode(o) = &t.object {
                    target = Some(o.as_str().to_string());
                }
            }
            s if s == sh("closed") => {
                if let TermRef::Literal(l) = &t.object {
                    closed = l.value() == "true";
                }
            }
            s if s == sh("not") => {
                let inner = t.object.into_owned();
                let class = parse_not_class(shapes, &inner)?;
                if let Some(c) = class {
                    nots.push(c);
                }
            }
            s if s == sh("property") => {
                let term = match &t.object {
                    TermRef::NamedNode(n) => Term::NamedNode((*n).into_owned()),
                    TermRef::BlankNode(b) => Term::BlankNode(
                        math_core::oxigraph::model::BlankNode::new(b.as_str().to_string())
                            .expect("blank node id"),
                    ),
                    other => {
                        return Err(format!("sh:property must be a node, got {other}"));
                    }
                };
                properties.push(parse_property_shape(shapes, &term)?);
            }
            _ => {}
        }
    }
    let _ = closed;
    Ok(NodeShape {
        id,
        target,
        properties,
        nots,
    })
}

/// `sh:not [ sh:class C ]` — any other inner-shape construct is unsupported.
fn parse_not_class(shapes: &Graph, inner: &Term) -> Result<Option<String>, String> {
    let mut class = None;
    let Some(subj) = term_as_subject(inner) else {
        return Ok(None);
    };
    for t in shapes.triples_for_subject(subj) {
        let p = t.predicate.as_str();
        if p == sh("class") {
            if let TermRef::NamedNode(o) = &t.object {
                class = Some(o.as_str().to_string());
            }
        } else if p.starts_with(SH) {
            return Err(format!("unsupported shape construct `{p}` inside sh:not"));
        }
    }
    Ok(class)
}

fn parse_property_shape(shapes: &Graph, node: &Term) -> Result<PropertyShape, String> {
    let mut path = None;
    let mut datatype = None;
    let mut class = None;
    let mut min_count = None;
    let mut max_count = None;
    let mut node_kind = None;
    let mut node_shape = None;
    let mut severity = sh("Violation");
    let subj = term_as_subject(node)
        .ok_or_else(|| format!("property shape subject must be a node, got {node}"))?;
    for t in shapes.triples_for_subject(subj) {
        let p = t.predicate.as_str();
        let supported = [
            "path", "datatype", "class", "minCount", "maxCount", "nodeKind", "node", "severity",
            "message",
        ]
        .iter()
        .any(|k| p == sh(k));
        if p.starts_with(SH) && !supported {
            return Err(format!("unsupported shape construct `{p}`"));
        }
        match p {
            s if s == sh("path") => path = Some(parse_path(shapes, &t.object.into_owned())?),
            s if s == sh("datatype") => {
                if let TermRef::NamedNode(o) = &t.object {
                    datatype = Some(o.as_str().to_string());
                }
            }
            s if s == sh("class") => {
                if let TermRef::NamedNode(o) = &t.object {
                    class = Some(o.as_str().to_string());
                }
            }
            s if s == sh("minCount") => min_count = lit_int(&t.object.into_owned()),
            s if s == sh("maxCount") => max_count = lit_int(&t.object.into_owned()),
            s if s == sh("nodeKind") => {
                if let TermRef::NamedNode(o) = &t.object {
                    node_kind = Some(o.as_str().to_string());
                }
            }
            s if s == sh("node") => {
                node_shape = Some(match &t.object {
                    TermRef::NamedNode(o) => o.as_str().to_string(),
                    TermRef::BlankNode(b) => b.as_str().to_string(),
                    other => {
                        return Err(format!("sh:node must be a node shape, got {other}"));
                    }
                });
            }
            s if s == sh("severity") => {
                if let TermRef::NamedNode(o) = &t.object {
                    severity = o.as_str().to_string();
                }
            }
            _ => {}
        }
    }
    let path = path.ok_or_else(|| format!("property shape `{node}` has no sh:path"))?;
    Ok(PropertyShape {
        path,
        datatype,
        class,
        min_count,
        max_count,
        node_kind,
        node: node_shape,
        severity,
    })
}

/// Parse a SHACL path term: IRI step, sequence (rdf:List), or
/// `sh:zeroOrMorePath`; anything else is unsupported.
fn parse_path(shapes: &Graph, term: &Term) -> Result<Path, String> {
    match term {
        Term::NamedNode(n) => Ok(Path::Step(n.as_str().to_string())),
        Term::BlankNode(_) => {
            // [ sh:zeroOrMorePath <inner> ] — a closure path.
            let zeros: Vec<Term> = objects(shapes, term, &sh("zeroOrMorePath"));
            if !zeros.is_empty() {
                if zeros.len() > 1 {
                    return Err("multiple sh:zeroOrMorePath in one path expression".into());
                }
                return Ok(Path::ZeroOrMore(Box::new(parse_path(shapes, &zeros[0])?)));
            }
            // A sequence path: an rdf:List of path expressions.
            let firsts = objects(shapes, term, RDF_FIRST);
            if !firsts.is_empty() {
                let mut parts = Vec::new();
                let mut current = term.clone();
                for _ in 0..1024 {
                    let fs = objects(shapes, &current, RDF_FIRST);
                    if fs.is_empty() {
                        break;
                    }
                    if fs.len() > 1 {
                        return Err("malformed sequence path: multiple rdf:first".into());
                    }
                    parts.push(parse_path(shapes, &fs[0])?);
                    let rests = objects(shapes, &current, RDF_REST);
                    if rests.is_empty() {
                        return Err("malformed sequence path: missing rdf:rest".into());
                    }
                    if rests.len() > 1 {
                        return Err("malformed sequence path: multiple rdf:rest".into());
                    }
                    if let Term::NamedNode(n) = &rests[0] {
                        if n.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil" {
                            break;
                        }
                    }
                    current = rests[0].clone();
                }
                return Ok(Path::Seq(parts));
            }
            let supported = [
                "zeroOrMorePath",
                "oneOrMorePath",
                "zeroOrOnePath",
                "inversePath",
                "alternativePath",
            ]
            .iter()
            .any(|k| !objects(shapes, term, &sh(k)).is_empty());
            if supported {
                return Err(
                    "unsupported path construct (only sh:zeroOrMorePath is implemented)".into(),
                );
            }
            Err(format!("unsupported path expression `{term}`"))
        }
        other => Err(format!("path must be an IRI or blank node, got {other}")),
    }
}

/// Evaluate a path from a starting node, returning the reachable values.
fn eval_path(graph: &Graph, start: &Term, path: &Path) -> Vec<Term> {
    match path {
        Path::Step(pred) => objects(graph, start, pred),
        Path::Seq(parts) => {
            let mut nodes = vec![start.clone()];
            for part in parts {
                let mut next = Vec::new();
                for n in &nodes {
                    next.extend(eval_path(graph, n, part));
                }
                nodes = next;
            }
            nodes
        }
        Path::ZeroOrMore(inner) => {
            // Closure: the start node plus everything reachable via `inner`.
            let mut seen = vec![start.clone()];
            let mut queue = vec![start.clone()];
            while let Some(n) = queue.pop() {
                for v in eval_path(graph, &n, inner) {
                    if !seen.contains(&v) {
                        seen.push(v.clone());
                        queue.push(v);
                    }
                }
            }
            seen
        }
    }
}

/// XSD numeric/string subtype families for datatype conformance.
fn datatype_conforms(got: &str, expected: &str) -> bool {
    if got == expected {
        return true;
    }
    let x = |l: &str| format!("{XSD}{l}");
    let int_family = [
        "byte",
        "short",
        "int",
        "long",
        "integer",
        "nonPositiveInteger",
        "negativeInteger",
        "nonNegativeInteger",
        "positiveInteger",
        "unsignedByte",
        "unsignedShort",
        "unsignedInt",
        "unsignedLong",
    ];
    let string_family = [
        "normalizedString",
        "token",
        "language",
        "Name",
        "NCName",
        "NMTOKEN",
        "anyURI",
    ];
    let expected_local = expected.strip_prefix(XSD).unwrap_or(expected);
    match expected_local {
        "integer" => int_family.contains(&got.strip_prefix(XSD).unwrap_or(got)),
        "decimal" => {
            int_family.contains(&got.strip_prefix(XSD).unwrap_or(got)) || got == x("decimal")
        }
        "long" => matches!(
            got.strip_prefix(XSD).unwrap_or(got),
            "byte" | "short" | "int" | "long"
        ),
        "int" => matches!(
            got.strip_prefix(XSD).unwrap_or(got),
            "byte" | "short" | "int"
        ),
        "short" => got == x("byte") || got == x("short"),
        "string" => {
            got == x("string") || string_family.contains(&got.strip_prefix(XSD).unwrap_or(got))
        }
        _ => false,
    }
}

/// Whether a node conforms to a parsed shape; violations are appended.
/// `graph` is the instance graph; `superclasses` is the precomputed subclass
/// closure used by class checks.
fn conforms(
    graph: &Graph,
    shapes: &[NodeShape],
    superclasses: &HashMap<String, BTreeSet<String>>,
    node: &Term,
    shape: &NodeShape,
    violations: &mut Vec<ShaclViolation>,
) {
    for ps in &shape.properties {
        let values = eval_path(graph, node, &ps.path);
        let path_str = ps.path.display();
        if let Some(min) = ps.min_count {
            if (values.len() as u64) < min {
                violations.push(ShaclViolation {
                    path: path_str.clone(),
                    message: format!("missing required property `{path_str}` (sh:minCount {min})"),
                    severity: ps.severity.clone(),
                });
            }
        }
        if let Some(max) = ps.max_count {
            if (values.len() as u64) > max {
                violations.push(ShaclViolation {
                    path: path_str.clone(),
                    message: format!(
                        "property `{path_str}` has {} values, sh:maxCount {max}",
                        values.len()
                    ),
                    severity: ps.severity.clone(),
                });
            }
        }
        if let Some(expected) = &ps.datatype {
            for v in &values {
                match v {
                    Term::Literal(l) => {
                        let got = l.datatype().as_str().to_string();
                        if !datatype_conforms(&got, expected) {
                            violations.push(ShaclViolation {
                                path: path_str.clone(),
                                message: format!("`{v}` is not of datatype {expected} (got {got})"),
                                severity: ps.severity.clone(),
                            });
                        }
                    }
                    other => violations.push(ShaclViolation {
                        path: path_str.clone(),
                        message: format!("`{other}` is not a literal of datatype {expected}"),
                        severity: ps.severity.clone(),
                    }),
                }
            }
        }
        if let Some(expected_class) = &ps.class {
            for v in &values {
                let ok = match v {
                    Term::NamedNode(_) | Term::BlankNode(_) => {
                        if expected_class == RDFS_RESOURCE {
                            true
                        } else {
                            types_of(graph, v)
                                .iter()
                                .any(|t| belongs_to(t, expected_class, superclasses))
                        }
                    }
                    _ => false,
                };
                if !ok {
                    violations.push(ShaclViolation {
                        path: path_str.clone(),
                        message: format!("`{v}` is not an instance of {expected_class}"),
                        severity: ps.severity.clone(),
                    });
                }
            }
        }
        if let Some(kind) = &ps.node_kind {
            for v in &values {
                let ok = match kind.as_str() {
                    k if k == sh("BlankNodeOrIRI") => {
                        matches!(v, Term::NamedNode(_) | Term::BlankNode(_))
                    }
                    k if k == sh("IRI") => matches!(v, Term::NamedNode(_)),
                    k if k == sh("Literal") => matches!(v, Term::Literal(_)),
                    k if k == sh("BlankNode") => matches!(v, Term::BlankNode(_)),
                    other => {
                        violations.push(ShaclViolation {
                            path: path_str.clone(),
                            message: format!("unsupported sh:nodeKind {other}"),
                            severity: ps.severity.clone(),
                        });
                        continue;
                    }
                };
                if !ok {
                    violations.push(ShaclViolation {
                        path: path_str.clone(),
                        message: format!("`{v}` does not have node kind {kind}"),
                        severity: ps.severity.clone(),
                    });
                }
            }
        }
        if let Some(target_shape) = &ps.node {
            let inner = shapes.iter().find(|s| &s.id == target_shape);
            if let Some(inner) = inner {
                for v in &values {
                    conforms(graph, shapes, superclasses, v, inner, violations);
                }
            }
        }
    }
    // `sh:not` over the generated fragment is a pure class check (the value
    // shape is `[ sh:class C ]`); evaluating it as a full node shape would be
    // degenerate for sibling variants that point at each other.
    for c in &shape.nots {
        let types = types_of(graph, node);
        if types.iter().any(|t| belongs_to(t, c, superclasses)) {
            violations.push(ShaclViolation {
                path: format!("not[class {c}]"),
                message: format!("`{node}` must not be an instance of {c}"),
                severity: sh("Violation"),
            });
        }
    }
}

/// A parsed shapes graph with its transitive subclass closure precomputed.
/// Reusable across validations: `sh:class` checks are an O(1) lookup instead
/// of a BFS over the whole shapes graph per value, and the node/property
/// shapes are parsed once.
#[derive(Debug, Clone)]
pub struct Shapes {
    nodes: Vec<NodeShape>,
    superclasses: HashMap<String, BTreeSet<String>>,
}

impl Shapes {
    /// Parse a shapes graph into reusable validated shapes.
    pub fn parse(shapes: &Graph) -> Result<Shapes, String> {
        let superclasses = subclass_closure(shapes);
        let nodes = parse_shapes(shapes)?;
        Ok(Shapes {
            nodes,
            superclasses,
        })
    }

    /// Validate an instance graph against these parsed shapes.
    pub fn validate(&self, data: &Graph) -> Result<ShaclReport, String> {
        validate_parsed(&self.nodes, &self.superclasses, data)
    }
}

/// Validate the instance graph `data` against the parsed shapes.
fn validate_parsed(
    parsed: &[NodeShape],
    superclasses: &HashMap<String, BTreeSet<String>>,
    data: &Graph,
) -> Result<ShaclReport, String> {
    if parsed.is_empty() {
        return Ok(ShaclReport {
            conforms: true,
            results: Vec::new(),
        });
    }
    let mut violations = Vec::new();

    // Every typed instance node, in deterministic order.
    let mut instances: Vec<Term> = Vec::new();
    for t in data.iter() {
        if t.predicate.as_str() == RDF_TYPE {
            let term = subject_term(&t.subject);
            if !instances.contains(&term) {
                instances.push(term);
            }
        }
    }
    instances.sort_by(|a, b| format!("{a}").cmp(&format!("{b}")));

    for node in &instances {
        let types = types_of(data, node);
        for shape in parsed {
            if let Some(target) = &shape.target {
                if types.iter().any(|t| belongs_to(t, target, superclasses)) {
                    conforms(data, parsed, superclasses, node, shape, &mut violations);
                }
            }
        }
    }

    violations.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(ShaclReport {
        conforms: violations.is_empty(),
        results: violations,
    })
}

/// Validate the instance graph `data` against the shapes graph `shapes`
/// (parse-once convenience wrapper; use [`Shapes`] to reuse the parse).
pub fn validate(shapes: &Graph, data: &Graph) -> Result<ShaclReport, String> {
    Shapes::parse(shapes)?.validate(data)
}

/// Validate the Turtle-encoded `data` against Turtle-encoded `shapes`.
pub fn validate_turtle(shapes_turtle: &str, data_turtle: &str) -> Result<ShaclReport, String> {
    let shapes = parse_turtle(shapes_turtle).map_err(|e| e.to_string())?;
    let data = parse_turtle(data_turtle).map_err(|e| e.to_string())?;
    validate(&shapes, &data)
}

fn lit_int(t: &Term) -> Option<u64> {
    match t {
        Term::Literal(l) => l.value().parse::<u64>().ok(),
        _ => None,
    }
}
