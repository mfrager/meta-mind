//! `rdf-codec` — shared machinery for the fine-grained RDF interface
//! (`planning/rdf_interface_update_plan.md`): flat, direct Rust ↔ RDF mapping
//! with lowercase-ULID instance IDs, canonical deterministic serialization,
//! and strict decoding (anything unexpected is a hard error, never a silent
//! default).

pub mod crdf;
pub mod id;
pub mod io;
pub mod ns;
pub mod reader;
pub mod scalar;
pub mod sem;

pub use id::{new_ulid, ulid_from_content, IdPolicy};
pub use ns::Namespace;
pub use reader::{find_root, list_values, RdfReader};
pub use scalar::Prim;

// The oxrdf model types, re-exported so per-IR codecs can name them without
// depending on oxigraph directly (same convention as `math-core`).
pub use math_core::oxigraph::model::{
    Graph, Literal, NamedNode, NamedOrBlankNode, NamedOrBlankNodeRef, Term, TermRef, Triple,
};

use std::collections::HashMap;

/// Errors produced by the codec. Decoding is total on the supported fragment:
/// unknown classes, unknown properties, missing required properties, and
/// malformed structures are all hard errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// IRI construction failed.
    Iri(String),
    /// Serialization-side problem (non-finite float, unsupported value).
    Encode(String),
    /// Deserialization-side problem (unknown class/property, bad literal).
    Decode(String),
    /// Turtle parse failure.
    Parse(String),
    /// Construct outside the supported fragment.
    Unsupported(String),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::Iri(s) => write!(f, "invalid IRI: {s}"),
            CodecError::Encode(s) => write!(f, "encode error: {s}"),
            CodecError::Decode(s) => write!(f, "decode error: {s}"),
            CodecError::Parse(s) => write!(f, "parse error: {s}"),
            CodecError::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl std::error::Error for CodecError {}

/// The serialization context: the graph being built, the schema namespace,
/// the data (instance) namespace, and the ID policy (§7.1, §7.6).
pub struct RdfContext {
    pub graph: Graph,
    pub schema: Namespace,
    pub data: Namespace,
    pub id_policy: IdPolicy,
    /// Dedup for shared substructures keyed by content hash.
    id_cache: HashMap<u64, NamedNode>,
    /// Monotonic ordinal for content-derived instance IDs.
    instance_counter: u64,
}

impl RdfContext {
    pub fn new(schema: Namespace, data: Namespace) -> Self {
        Self {
            graph: Graph::new(),
            schema,
            data,
            id_policy: IdPolicy::UlidRandom,
            id_cache: HashMap::new(),
            instance_counter: 0,
        }
    }

    pub fn with_id_policy(mut self, policy: IdPolicy) -> Self {
        self.id_policy = policy;
        self
    }

    /// A fresh instance node: `{data_ns}{lowercase-ulid}` (§4.2). Under
    /// `FromContentHash` the ID is derived from the context identity plus the
    /// ordinal position of the instance, so identical structures serialized
    /// through identical contexts produce identical IRIs (reproducible
    /// artifacts), while remaining distinct within one graph.
    pub fn instance(&mut self) -> NamedNode {
        let id = match self.id_policy {
            IdPolicy::UlidRandom => id::new_ulid(),
            IdPolicy::FromContentHash => {
                let ordinal = self.instance_counter;
                id::ulid_from_content(&format!("{}|{}|{ordinal}", self.schema.iri, self.data.iri))
            }
        };
        self.instance_counter += 1;
        NamedNode::new(format!("{}{}", self.data.iri, id)).expect("instance IRI")
    }

    /// A stable constant instance IRI for an enum unit variant (§7.5):
    /// `{data_ns}{Enum}/{variant}` with a lowercased local part. Deterministic
    /// across runs — no ULID, no randomness.
    pub fn constant(&self, enm: &str, variant: &str) -> NamedNode {
        NamedNode::new(format!(
            "{}{}/{}",
            self.data.iri,
            enm.to_lowercase(),
            variant.to_lowercase()
        ))
        .expect("constant IRI")
    }

    /// Dedup helper for shared substructures: reuse an existing node when the
    /// content hash matches.
    pub fn cached(&mut self, key: u64, node: NamedNode) -> NamedNode {
        self.id_cache.entry(key).or_insert(node).clone()
    }

    /// `node rdf:type {schema}{class}`.
    pub fn emit_type(&mut self, node: &NamedNode, class: &str) {
        let class_node = NamedNode::new(self.schema.class(class)).expect("class IRI");
        self.graph.insert(&Triple::new(
            node.clone(),
            math_core::oxigraph::model::vocab::rdf::TYPE.into_owned(),
            Term::from(class_node),
        ));
    }

    /// `node {schema}{class}/{prop} literal` (a datatype property).
    pub fn emit_literal(&mut self, node: &NamedNode, class: &str, prop: &str, value: Literal) {
        let p = NamedNode::new(self.schema.property(class, prop)).expect("property IRI");
        self.graph
            .insert(&Triple::new(node.clone(), p, Term::from(value)));
    }

    /// `node {schema}{class}/{prop} obj` (an object property).
    pub fn emit_object(&mut self, node: &NamedNode, class: &str, prop: &str, obj: &NamedNode) {
        let p = NamedNode::new(self.schema.property(class, prop)).expect("property IRI");
        self.graph
            .insert(&Triple::new(node.clone(), p, Term::from(obj.clone())));
    }

    /// Build a standard `rdf:List` of terms; every cons cell is a ULID-named
    /// instance (no blank nodes anywhere in instance output, §7.2). Returns
    /// `rdf:nil` for the empty list.
    pub fn list(&mut self, items: &[Term]) -> Result<NamedNode, CodecError> {
        if items.is_empty() {
            return Ok(math_core::oxigraph::model::vocab::rdf::NIL.into_owned());
        }
        let head = self.instance();
        let mut cell = head.clone();
        for (i, item) in items.iter().enumerate() {
            self.graph.insert(&Triple::new(
                cell.clone(),
                math_core::oxigraph::model::vocab::rdf::FIRST.into_owned(),
                item.clone(),
            ));
            if i + 1 < items.len() {
                let next = self.instance();
                self.graph.insert(&Triple::new(
                    cell.clone(),
                    math_core::oxigraph::model::vocab::rdf::REST.into_owned(),
                    Term::from(next.clone()),
                ));
                cell = next;
            }
        }
        self.graph.insert(&Triple::new(
            cell,
            math_core::oxigraph::model::vocab::rdf::REST.into_owned(),
            Term::from(math_core::oxigraph::model::vocab::rdf::NIL.into_owned()),
        ));
        Ok(head)
    }

    /// Convenience: serialize the graph to canonical N-Triples for hashing.
    pub fn to_canonical(&self) -> String {
        io::to_canonical_ntriples(&self.graph)
    }

    /// Convenience: the deterministic content hash of the graph so far.
    pub fn canonical_hash(&self) -> String {
        io::canonical_hash(&self.graph)
    }
}

/// The serialization side of the codec (§7.1): emit this value into `ctx` and
/// return its instance node.
pub trait ToRdf {
    fn to_rdf(&self, ctx: &mut RdfContext) -> Result<NamedNode, CodecError>;
}

/// The deserialization side of the codec (§7.1): reconstruct this value from
/// the instance node `node` in `graph`. Decoding is strict.
pub trait FromRdf: Sized {
    fn from_rdf(node: &NamedNode, graph: &Graph) -> Result<Self, CodecError>;
}

/// Emit a value into a fresh context and return its instance node.
pub fn emit<T: ToRdf>(
    value: &T,
    schema: Namespace,
    data: Namespace,
) -> Result<NamedNode, CodecError> {
    let mut ctx = RdfContext::new(schema, data);
    value.to_rdf(&mut ctx)
}

/// Decode a value from a Turtle document. The root is located automatically
/// (§7.7) and decoding is strict.
pub fn decode<T: FromRdf>(turtle: &str, schema: Namespace) -> Result<T, CodecError> {
    let graph = io::parse_turtle(turtle)?;
    let root = find_root(&graph, &schema)?;
    T::from_rdf(&root, &graph)
}
