//! `MathModel` — a thin wrapper over an Oxigraph `Store`.

use oxigraph::io::{RdfFormat, RdfParser};
use oxigraph::model::{GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term};
use oxigraph::store::Store;
use std::path::Path;

use crate::vocabulary::RDF_TYPE;

/// Strip surrounding angle brackets from an IRI string (a no-op if absent).
pub fn bare_iri(iri: &str) -> &str {
    iri.trim_start_matches('<').trim_end_matches('>')
}

/// The bare IRI string of a `NamedNode`.
pub fn named_node_iri(n: &NamedNode) -> String {
    bare_iri(&n.to_string()).to_string()
}

/// Extract the local name from an IRI (possibly angle-bracketed).
pub fn local_name(iri: &str) -> &str {
    let s = bare_iri(iri);
    if let Some(i) = s.rfind('#') {
        return &s[i + 1..];
    }
    if let Some(i) = s.rfind('/') {
        return &s[i + 1..];
    }
    s
}

pub struct MathModel {
    pub store: Store,
}

impl MathModel {
    /// In-memory store (no RocksDB).
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            store: Store::new().map_err(|e| e.to_string())?,
        })
    }

    /// Open a persistent RocksDB-backed store.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        Ok(Self {
            store: Store::open(path).map_err(|e| e.to_string())?,
        })
    }

    /// Load Turtle into the store.
    pub fn load_turtle(&self, data: &str, base: &str) -> Result<(), String> {
        let parser = RdfParser::from_format(RdfFormat::Turtle)
            .with_base_iri(base.to_string())
            .map_err(|e| e.to_string())?;
        for quad in parser.for_slice(data.as_bytes()) {
            let quad = quad.map_err(|e| e.to_string())?;
            self.store.insert(&quad).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn insert(&self, quad: Quad) -> Result<(), String> {
        self.store.insert(&quad).map_err(|e| e.to_string())
    }

    /// All objects of `(subject, predicate)` for an IRI-identified subject.
    pub fn objects(&self, subject: &str, predicate: &str) -> Result<Vec<Term>, String> {
        let s = NamedNode::new(subject).map_err(|e| e.to_string())?;
        let p = NamedNode::new(predicate).map_err(|e| e.to_string())?;
        Ok(self.objects_of(&s.into(), &p))
    }

    /// All objects for a resolved subject + predicate.
    pub fn objects_of(&self, subject: &NamedOrBlankNode, predicate: &NamedNode) -> Vec<Term> {
        let mut out = Vec::new();
        for quad in self
            .store
            .quads_for_pattern(Some(subject.into()), Some(predicate.into()), None, None)
            .flatten()
        {
            out.push(quad.object);
        }
        out
    }

    /// All objects for a `Term` subject (blank node or named node).
    pub fn objects_of_term(&self, subject: &Term, predicate: &str) -> Vec<Term> {
        let subj = match subject {
            Term::NamedNode(n) => NamedOrBlankNode::from(n.clone()),
            Term::BlankNode(b) => NamedOrBlankNode::from(b.clone()),
            _ => return Vec::new(),
        };
        let Ok(p) = NamedNode::new(predicate) else {
            return Vec::new();
        };
        self.objects_of(&subj, &p)
    }

    /// First object for `(subject, predicate)`, if any.
    pub fn single_object(&self, subject: &str, predicate: &str) -> Option<Term> {
        self.objects(subject, predicate).ok()?.into_iter().next()
    }

    /// All subjects having `rdf:type <class>`.
    pub fn subjects_of_type(&self, class: &str) -> Vec<String> {
        let Ok(p) = NamedNode::new(RDF_TYPE) else {
            return Vec::new();
        };
        let Ok(c) = NamedNode::new(class) else {
            return Vec::new();
        };
        let o: Term = c.into();
        let mut out = Vec::new();
        for quad in self
            .store
            .quads_for_pattern(None, Some((&p).into()), Some((&o).into()), None)
            .flatten()
        {
            out.push(bare_iri(&quad.subject.to_string()).to_string());
        }
        out
    }

    /// The numeric value of `(subject, predicate)`, if the object is a
    /// parseable numeric literal.
    pub fn numeric_value(&self, subject: &str, predicate: &str) -> Option<f64> {
        self.single_object(subject, predicate)
            .and_then(|t| match t {
                Term::Literal(l) => l.value().parse::<f64>().ok(),
                _ => None,
            })
    }

    /// The `rdf:type` of a subject (bare IRI), if any.
    pub fn type_of(&self, subject: &str) -> Option<String> {
        self.objects(subject, RDF_TYPE)
            .ok()?
            .into_iter()
            .find_map(|t| match t {
                Term::NamedNode(n) => Some(named_node_iri(&n)),
                _ => None,
            })
    }

    /// Set (or replace) the numeric value of `(subject, predicate)`.
    pub fn set_numeric_value(
        &self,
        subject: &str,
        predicate: &str,
        value: f64,
    ) -> Result<(), String> {
        let s = NamedNode::new(subject).map_err(|e| e.to_string())?;
        let p = NamedNode::new(predicate).map_err(|e| e.to_string())?;

        // Replace any existing value with the same subject/predicate.
        if let Some(old) = self.single_object(subject, predicate) {
            let old_quad = Quad::new(s.clone(), p.clone(), old, GraphName::DefaultGraph);
            self.store.remove(&old_quad).map_err(|e| e.to_string())?;
        }

        let dt =
            NamedNode::new("http://www.w3.org/2001/XMLSchema#double").map_err(|e| e.to_string())?;
        let lit = Literal::new_typed_literal(value.to_string(), dt);
        let quad = Quad::new(s, p, Term::from(lit), GraphName::DefaultGraph);
        self.store.insert(&quad).map_err(|e| e.to_string())
    }
}
