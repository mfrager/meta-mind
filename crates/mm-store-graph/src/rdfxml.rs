//! RDF/XML and N-Quads: the portable snapshot and the lossless backup.
//!
//! RDF/XML has no named-graph syntax, so [`dump_graph_rdfxml`] exports a quad
//! set as triples. Graph membership survives a portable snapshot only as an
//! explicit `mm:sourceGraph` annotation written by the caller;
//! [`dump_store_nquads`] is the lossless copy that keeps the graph names.

use oxigraph::io::{RdfFormat, RdfParser, RdfSerializer};
use oxigraph::model::{GraphNameRef, NamedNode, Quad, Triple};

use mm_core::MmError;

/// Parse an RDF/XML document and return its triples.
pub fn parse_rdfxml(xml: &[u8]) -> Result<Vec<Triple>, MmError> {
    let parser = RdfParser::from_format(RdfFormat::RdfXml);
    let mut triples = Vec::new();
    for quad in parser.for_slice(xml) {
        let quad = quad.map_err(|e| MmError::Codec(format!("rdf/xml parse error: {e}")))?;
        triples.push(Triple::new(quad.subject, quad.predicate, quad.object));
    }
    Ok(triples)
}

/// Serialize a stream of quads as one RDF/XML document (graph names dropped).
pub fn dump_quads_rdfxml(
    quads: impl Iterator<Item = Result<Quad, MmError>>,
) -> Result<String, MmError> {
    let mut serializer = RdfSerializer::from_format(RdfFormat::RdfXml).for_writer(Vec::new());
    for quad in quads {
        let quad = quad?;
        let triple = Triple::new(quad.subject, quad.predicate, quad.object);
        serializer
            .serialize_triple(&triple)
            .map_err(|e| MmError::Codec(format!("rdf/xml serialization failed: {e}")))?;
    }
    let bytes = serializer
        .finish()
        .map_err(|e| MmError::Codec(format!("rdf/xml serialization failed: {e}")))?;
    String::from_utf8(bytes).map_err(|e| MmError::Codec(format!("rdf/xml dump is not UTF-8: {e}")))
}

/// Serialize every quad, graph names included, as N-Quads.
pub fn dump_store_nquads(
    quads: impl Iterator<Item = Result<Quad, MmError>>,
) -> Result<String, MmError> {
    let mut out = String::new();
    for quad in quads {
        let quad = quad?;
        // `Quad`'s display renders the four terms; N-Quads needs the terminator.
        out.push_str(&quad.to_string());
        out.push_str(" .\n");
    }
    Ok(out)
}

/// Parse an RDF/XML document into quads for one graph.
pub fn parse_rdfxml_into(graph: &NamedNode, xml: &[u8]) -> Result<Vec<Quad>, MmError> {
    let mut quads = Vec::with_capacity(16);
    for triple in parse_rdfxml(xml)? {
        quads.push(Quad::new(
            triple.subject,
            triple.predicate,
            triple.object,
            graph.clone(),
        ));
    }
    Ok(quads)
}

/// The graph name reference used when dumping.
pub fn named_ref(graph: &NamedNode) -> GraphNameRef<'_> {
    GraphNameRef::NamedNode(graph.as_ref())
}
