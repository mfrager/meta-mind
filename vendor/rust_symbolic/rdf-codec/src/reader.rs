//! Strict RDF decoding (§7.7): root discovery, `rdf:List` traversal, and
//! single-valued property access. Everything unexpected is a hard error.

use std::collections::{HashMap, HashSet};

use math_core::oxigraph::model::vocab::rdf;
use math_core::oxigraph::model::{
    Graph, Literal, NamedNode, NamedOrBlankNode, NamedOrBlankNodeRef, Term, TermRef,
};

use crate::{CodecError, Namespace};

/// The maximum list length the reader will follow (cycle guard).
const LIST_LIMIT: usize = 10_000;

/// Find the root node of an instance graph (§7.7): a schema-typed subject that
/// is not itself the object of any property; falling back to the single typed
/// subject when every typed node is referenced.
pub fn find_root(graph: &Graph, schema: &Namespace) -> Result<NamedNode, CodecError> {
    let type_pred = rdf::TYPE.into_owned();
    let mut typed: Vec<NamedNode> = graph
        .iter()
        .filter(|t| t.predicate == type_pred.as_ref())
        .filter_map(|t| match (&t.subject, &t.object) {
            (NamedOrBlankNodeRef::NamedNode(s), TermRef::NamedNode(o))
                if o.as_str().starts_with(&schema.iri) =>
            {
                Some(s.into_owned())
            }
            _ => None,
        })
        .collect();
    typed.sort_by_key(|n| n.to_string());
    typed.dedup();
    if typed.is_empty() {
        return Err(CodecError::Decode(format!(
            "no schema-typed subject found under `{}`",
            schema.iri
        )));
    }

    let as_object: HashSet<String> = graph
        .iter()
        .filter_map(|t| match t.object {
            TermRef::NamedNode(o) => Some(o.as_str().to_string()),
            _ => None,
        })
        .collect();

    let roots: Vec<NamedNode> = typed
        .iter()
        .filter(|n| !as_object.contains(n.as_str()))
        .cloned()
        .collect();

    match roots.len() {
        1 => Ok(roots[0].clone()),
        0 if typed.len() == 1 => Ok(typed[0].clone()),
        // Multiple unreferenced candidates: prefer a *container* root class
        // (`Model`, `*Problem`, `*Query`) over leaf classes such as Variable,
        // Parameter, Equation, or Fluent. This resolves models where removal
        // of an equation orphans a symbol that then also looks like a root.
        // Only consulted when the heuristic above is ambiguous, so it never
        // changes an already-unambiguous result.
        _ => {
            let type_of: HashMap<String, String> = graph
                .iter()
                .filter(|t| t.predicate == type_pred.as_ref())
                .filter_map(|t| match (&t.subject, &t.object) {
                    (NamedOrBlankNodeRef::NamedNode(s), TermRef::NamedNode(o))
                        if o.as_str().starts_with(&schema.iri) =>
                    {
                        Some((s.as_str().to_string(), o.as_str().to_string()))
                    }
                    _ => None,
                })
                .collect();
            let is_container = |t: &NamedNode| {
                type_of.get(t.as_str()).map_or(false, |ty| {
                    ty.ends_with("Model") || ty.ends_with("Problem") || ty.ends_with("Query")
                })
            };
            let mut containers: Vec<NamedNode> =
                roots.iter().filter(|n| is_container(n)).cloned().collect();
            containers.sort_by_key(|n| n.to_string());
            containers.dedup();
            if containers.len() == 1 {
                return Ok(containers[0].clone());
            }
            match roots.len() {
                0 => Err(CodecError::Decode(format!(
                    "{} schema-typed subjects found, all referenced as objects; root is ambiguous",
                    typed.len()
                ))),
                n => Err(CodecError::Decode(format!(
                    "{n} candidate root subjects found; root is ambiguous"
                ))),
            }
        }
    }
}

/// Traverse an `rdf:List` from its head node (which may be a named or a
/// blank node — standard Turtle `( ... )` lists parse to blank-node cells),
/// returning the element terms in order. A malformed list (missing
/// `rdf:first`, missing/wrong `rdf:rest`, cycles, >1 value per cell) is a
/// hard error.
pub fn list_values(graph: &Graph, head: &NamedOrBlankNode) -> Result<Vec<Term>, CodecError> {
    let first = rdf::FIRST.into_owned();
    let rest = rdf::REST.into_owned();
    let nil = rdf::NIL.into_owned();
    if head.as_ref() == NamedOrBlankNodeRef::NamedNode(nil.as_ref()) {
        return Ok(Vec::new()); // the empty list
    }
    let mut out = Vec::new();
    let mut current = head.clone();
    for _ in 0..LIST_LIMIT {
        let mut firsts: Vec<Term> = graph
            .iter()
            .filter(|t| t.subject == current.as_ref() && t.predicate == first.as_ref())
            .map(|t| t.object.into_owned())
            .collect();
        if firsts.is_empty() {
            return Err(CodecError::Decode(format!(
                "malformed list at `{head}`: missing rdf:first"
            )));
        }
        if firsts.len() > 1 {
            return Err(CodecError::Decode(format!(
                "malformed list at `{head}`: multiple rdf:first"
            )));
        }
        out.push(firsts.remove(0));

        let rests: Vec<Term> = graph
            .iter()
            .filter(|t| t.subject == current.as_ref() && t.predicate == rest.as_ref())
            .map(|t| t.object.into_owned())
            .collect();
        if rests.is_empty() {
            return Err(CodecError::Decode(format!(
                "malformed list at `{head}`: missing rdf:rest"
            )));
        }
        if rests.len() > 1 {
            return Err(CodecError::Decode(format!(
                "malformed list at `{head}`: multiple rdf:rest"
            )));
        }
        match &rests[0] {
            Term::NamedNode(n) if n == &nil => return Ok(out),
            Term::NamedNode(n) => current = NamedOrBlankNode::from(n.clone()),
            Term::BlankNode(b) => current = NamedOrBlankNode::from(b.clone()),
            other => {
                return Err(CodecError::Decode(format!(
                    "malformed list at `{head}`: rdf:rest object must be a node, got {other}"
                )))
            }
        }
    }
    Err(CodecError::Decode(format!(
        "list at `{head}` exceeds the depth limit"
    )))
}

/// A strict reader over one instance node. All property access is
/// single-valued and total: missing or duplicated values are hard errors.
pub struct RdfReader<'a> {
    graph: &'a Graph,
    node: NamedNode,
    schema: &'a Namespace,
}

impl<'a> RdfReader<'a> {
    pub fn new(graph: &'a Graph, node: &NamedNode, schema: &'a Namespace) -> Self {
        Self {
            graph,
            node: node.clone(),
            schema,
        }
    }

    /// The node under inspection.
    pub fn node(&self) -> &NamedNode {
        &self.node
    }

    /// Assert `node rdf:type {schema}{class}` (class may be a variant path
    /// like `Request/Prove`).
    pub fn expect_type(&self, class: &str) -> Result<(), CodecError> {
        let expected =
            NamedNode::new(self.schema.class(class)).map_err(|e| CodecError::Iri(e.to_string()))?;
        let type_pred = rdf::TYPE.into_owned();
        for t in self.graph.iter() {
            if t.subject == NamedOrBlankNodeRef::NamedNode(self.node.as_ref())
                && t.predicate == type_pred.as_ref()
                && matches!(&t.object, TermRef::NamedNode(o) if o.as_str() == expected.as_str())
            {
                return Ok(());
            }
        }
        Err(CodecError::Decode(format!(
            "`{}` is not typed as {class}",
            self.node
        )))
    }

    /// The local name of the schema-typed class of this node (e.g.
    /// `TemporalPredicate/Before`), for variant-subclass dispatch. `None`
    /// when the node has no schema-typed rdf:type.
    pub fn type_local(&self) -> Option<String> {
        let type_pred = rdf::TYPE.into_owned();
        for t in self.graph.iter() {
            if t.subject == NamedOrBlankNodeRef::NamedNode(self.node.as_ref())
                && t.predicate == type_pred.as_ref()
            {
                if let TermRef::NamedNode(o) = &t.object {
                    if let Some(local) = o.as_str().strip_prefix(&self.schema.iri) {
                        return Some(local.to_string());
                    }
                }
            }
        }
        None
    }

    /// Reject any property of this node outside `allowed` (full IRIs) plus
    /// `rdf:type` — the strictness rule: unknown properties are hard errors.
    pub fn check_known(&self, allowed: &[&str]) -> Result<(), CodecError> {
        let type_pred = rdf::TYPE.into_owned();
        for t in self.graph.iter() {
            if t.subject != NamedOrBlankNodeRef::NamedNode(self.node.as_ref()) {
                continue;
            }
            if t.predicate == type_pred.as_ref() {
                continue;
            }
            let p = t.predicate.as_str().to_string();
            if !allowed.contains(&p.as_str()) {
                return Err(CodecError::Decode(format!(
                    "unknown property `{p}` on `{}`",
                    self.node
                )));
            }
        }
        Ok(())
    }

    fn property_node(&self, prop: &str) -> Result<NamedNode, CodecError> {
        NamedNode::new(prop.to_string()).map_err(|e| CodecError::Iri(e.to_string()))
    }

    /// The single literal value of `prop` (must exist, exactly one).
    pub fn literal(&self, prop: &str) -> Result<Literal, CodecError> {
        self.optional_literal(prop)?.ok_or_else(|| {
            CodecError::Decode(format!("missing property `{prop}` on `{}`", self.node))
        })
    }

    /// Zero-or-one literal value of `prop`; more than one is an error.
    pub fn optional_literal(&self, prop: &str) -> Result<Option<Literal>, CodecError> {
        let p = self.property_node(prop)?;
        let mut found = None;
        for t in self.graph.iter() {
            if t.subject == NamedOrBlankNodeRef::NamedNode(self.node.as_ref())
                && t.predicate == p.as_ref()
            {
                match &t.object {
                    TermRef::Literal(l) => {
                        if found.is_some() {
                            return Err(CodecError::Decode(format!(
                                "multiple values for `{prop}` on `{}`",
                                self.node
                            )));
                        }
                        found = Some(l.into_owned());
                    }
                    other => {
                        return Err(CodecError::Decode(format!(
                            "property `{prop}` on `{}` must be a literal, got {other}",
                            self.node
                        )))
                    }
                }
            }
        }
        Ok(found)
    }

    /// The single object node of `prop` (must exist, exactly one).
    pub fn object(&self, prop: &str) -> Result<NamedNode, CodecError> {
        self.optional_object(prop)?.ok_or_else(|| {
            CodecError::Decode(format!("missing property `{prop}` on `{}`", self.node))
        })
    }

    /// Zero-or-one object node of `prop`; more than one is an error.
    pub fn optional_object(&self, prop: &str) -> Result<Option<NamedNode>, CodecError> {
        let p = self.property_node(prop)?;
        let mut found = None;
        for t in self.graph.iter() {
            if t.subject == NamedOrBlankNodeRef::NamedNode(self.node.as_ref())
                && t.predicate == p.as_ref()
            {
                match &t.object {
                    TermRef::NamedNode(o) => {
                        if found.is_some() {
                            return Err(CodecError::Decode(format!(
                                "multiple values for `{prop}` on `{}`",
                                self.node
                            )));
                        }
                        found = Some(o.into_owned());
                    }
                    other => {
                        return Err(CodecError::Decode(format!(
                            "property `{prop}` on `{}` must be an IRI, got {other}",
                            self.node
                        )))
                    }
                }
            }
        }
        Ok(found)
    }

    /// The `rdf:List` value of `prop` (must exist). The list head may be a
    /// blank node (standard Turtle `( ... )` lists) or a named node.
    pub fn list(&self, prop: &str) -> Result<Vec<Term>, CodecError> {
        self.optional_list(prop)?.ok_or_else(|| {
            CodecError::Decode(format!("missing property `{prop}` on `{}`", self.node))
        })
    }

    /// Zero-or-one `rdf:List` value. Additive fields can use this accessor so
    /// legacy graphs may omit them during migration.
    pub fn optional_list(&self, prop: &str) -> Result<Option<Vec<Term>>, CodecError> {
        match self.list_head(prop)? {
            Some(head) => Ok(Some(list_values(self.graph, &head)?)),
            None => Ok(None),
        }
    }

    /// The single object node of `prop` as a subject (named or blank), or
    /// `None` when absent; more than one value is an error.
    fn list_head(&self, prop: &str) -> Result<Option<NamedOrBlankNode>, CodecError> {
        let p = self.property_node(prop)?;
        let mut found = None;
        for t in self.graph.iter() {
            if t.subject == NamedOrBlankNodeRef::NamedNode(self.node.as_ref())
                && t.predicate == p.as_ref()
            {
                match &t.object {
                    TermRef::NamedNode(o) => {
                        if found.is_some() {
                            return Err(CodecError::Decode(format!(
                                "multiple values for `{prop}` on `{}`",
                                self.node
                            )));
                        }
                        found = Some(NamedOrBlankNode::from(*o));
                    }
                    TermRef::BlankNode(b) => {
                        if found.is_some() {
                            return Err(CodecError::Decode(format!(
                                "multiple values for `{prop}` on `{}`",
                                self.node
                            )));
                        }
                        found = Some(NamedOrBlankNode::from(*b));
                    }
                    other => {
                        return Err(CodecError::Decode(format!(
                            "property `{prop}` on `{}` must be a node, got {other}",
                            self.node
                        )))
                    }
                }
            }
        }
        Ok(found)
    }

    /// The `rdf:List` value of `prop` restricted to named-node elements.
    pub fn list_objects(&self, prop: &str) -> Result<Vec<NamedNode>, CodecError> {
        self.optional_list_objects(prop)?.ok_or_else(|| {
            CodecError::Decode(format!("missing property `{prop}` on `{}`", self.node))
        })
    }

    /// Zero-or-one named-node list value.
    pub fn optional_list_objects(&self, prop: &str) -> Result<Option<Vec<NamedNode>>, CodecError> {
        let Some(terms) = self.optional_list(prop)? else {
            return Ok(None);
        };
        Ok(Some(
            terms
                .into_iter()
                .map(|t| match t {
                    Term::NamedNode(n) => Ok(n),
                    other => Err(CodecError::Decode(format!(
                        "expected IRI list elements under `{prop}`, got {other}"
                    ))),
                })
                .collect::<Result<Vec<_>, CodecError>>()?,
        ))
    }

    /// A reader over the single object of `prop`.
    pub fn child(&self, prop: &str) -> Result<RdfReader<'a>, CodecError> {
        Ok(RdfReader::new(self.graph, &self.object(prop)?, self.schema))
    }
}
