//! RDF I/O (§9.3): Turtle parse/serialize plus **canonical** N-Triples
//! serialization (sorted triples) for stable content hashing.

use math_core::oxigraph::io::{RdfFormat, RdfParser, RdfSerializer};
use math_core::oxigraph::model::vocab::xsd;
use math_core::oxigraph::model::{Graph, LiteralRef, NamedOrBlankNodeRef, Term, TermRef, Triple};

use crate::{id, CodecError};

/// Parse Turtle into an in-memory graph.
pub fn parse_turtle(text: &str) -> Result<Graph, CodecError> {
    let parser = RdfParser::from_format(RdfFormat::Turtle)
        .with_base_iri("https://example.org/")
        .map_err(|e| CodecError::Parse(e.to_string()))?;
    let mut graph = Graph::new();
    for quad in parser.for_slice(text.as_bytes()) {
        let quad = quad.map_err(|e| CodecError::Parse(e.to_string()))?;
        graph.insert(&Triple::new(quad.subject, quad.predicate, quad.object));
    }
    Ok(graph)
}

/// Serialize a graph to Turtle (full IRIs; prefixes are prepended by the
/// caller via [`to_turtle_with_prefixes`]).
pub fn to_turtle(graph: &Graph) -> Result<String, CodecError> {
    let mut output = Vec::new();
    let serializer = RdfSerializer::from_format(RdfFormat::Turtle);
    let mut writer = serializer.for_writer(&mut output);
    for triple in graph.iter() {
        writer
            .serialize_triple(triple)
            .map_err(|e| CodecError::Encode(e.to_string()))?;
    }
    writer
        .finish()
        .map_err(|e| CodecError::Encode(e.to_string()))?;
    String::from_utf8(output).map_err(|e| CodecError::Encode(e.to_string()))
}

/// Serialize a graph to Turtle with `@prefix` declarations prepended.
pub fn to_turtle_with_prefixes(
    graph: &Graph,
    prefixes: &[(&str, &str)],
) -> Result<String, CodecError> {
    let mut out = String::new();
    for (prefix, iri) in prefixes {
        out.push_str(&format!("@prefix {prefix}: <{iri}> .\n"));
    }
    if !prefixes.is_empty() {
        out.push('\n');
    }
    out.push_str(&to_turtle(graph)?);
    Ok(out)
}

/// Serialize a graph to RDF/XML.
pub fn to_rdf_xml(graph: &Graph) -> Result<String, CodecError> {
    let mut output = Vec::new();
    let serializer = RdfSerializer::from_format(RdfFormat::RdfXml);
    let mut writer = serializer.for_writer(&mut output);
    for triple in graph.iter() {
        writer
            .serialize_triple(triple)
            .map_err(|e| CodecError::Encode(e.to_string()))?;
    }
    writer
        .finish()
        .map_err(|e| CodecError::Encode(e.to_string()))?;
    String::from_utf8(output).map_err(|e| CodecError::Encode(e.to_string()))
}

/// Parse Turtle and return RDF/XML.
pub fn turtle_to_rdf_xml(turtle: &str) -> Result<String, CodecError> {
    let graph = parse_turtle(turtle)?;
    to_rdf_xml(&graph)
}

/// A canonical, order-independent N-Triples rendering of a graph: triples are
/// sorted lexically by `(subject, predicate, object)`.
pub fn to_canonical_ntriples(graph: &Graph) -> String {
    let mut triples: Vec<String> = graph.iter().map(canonical_triple).collect();
    triples.sort();
    triples.join("\n")
}

/// The deterministic content hash of a graph (FNV-1a-128 over the canonical
/// N-Triples, hex-encoded).
pub fn canonical_hash(graph: &Graph) -> String {
    id::fnv1a_hex(&to_canonical_ntriples(graph))
}

fn canonical_triple(t: math_core::oxigraph::model::TripleRef<'_>) -> String {
    let s = subject_canonical(&t.subject);
    let p = format!("<{}>", t.predicate.as_str());
    let o = term_canonical(&t.object);
    format!("{s} {p} {o} .")
}

fn subject_canonical(s: &NamedOrBlankNodeRef<'_>) -> String {
    match s {
        NamedOrBlankNodeRef::NamedNode(n) => format!("<{}>", n.as_str()),
        NamedOrBlankNodeRef::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

fn term_canonical(t: &TermRef<'_>) -> String {
    match t {
        TermRef::NamedNode(n) => format!("<{}>", n.as_str()),
        TermRef::BlankNode(b) => format!("_:{}", b.as_str()),
        TermRef::Literal(l) => literal_canonical(l),
    }
}

fn literal_canonical(l: &LiteralRef<'_>) -> String {
    let value = l.value().replace('\\', "\\\\").replace('"', "\\\"");
    if let Some(lang) = l.language() {
        return format!("\"{value}\"@{lang}");
    }
    if l.datatype().as_str() == xsd::STRING.as_str() {
        return format!("\"{value}\"");
    }
    format!("\"{value}\"^^<{}>", l.datatype().as_str())
}

/// Convenience: canonical N-Triples of a single term in object position.
pub fn term_nt(t: &Term) -> String {
    term_canonical(&t.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_graph() -> Graph {
        let mut g = Graph::new();
        let s = math_core::oxigraph::model::NamedNode::new("https://example.org/a").unwrap();
        let p = math_core::oxigraph::model::NamedNode::new("https://example.org/p").unwrap();
        g.insert(&Triple::new(
            s.clone(),
            p.clone(),
            Term::from(math_core::oxigraph::model::Literal::new_simple_literal("x")),
        ));
        let dt = math_core::oxigraph::model::NamedNode::new(xsd::DOUBLE.as_str()).unwrap();
        g.insert(&Triple::new(
            s,
            p,
            Term::from(math_core::oxigraph::model::Literal::new_typed_literal(
                "3.5", dt,
            )),
        ));
        g
    }

    #[test]
    fn canonical_is_order_independent() {
        let a = to_canonical_ntriples(&tiny_graph());
        // Rebuild in a different insertion order by round-tripping.
        let turtle = to_turtle(&tiny_graph()).unwrap();
        let reparsed = parse_turtle(&turtle).unwrap();
        let b = to_canonical_ntriples(&reparsed);
        assert_eq!(a, b);
    }

    #[test]
    fn parse_round_trip_preserves_triples() {
        let g = tiny_graph();
        let turtle = to_turtle(&g).unwrap();
        let reparsed = parse_turtle(&turtle).unwrap();
        assert_eq!(g.len(), reparsed.len());
        assert_eq!(canonical_hash(&g), canonical_hash(&reparsed));
    }

    #[test]
    fn hash_changes_with_content() {
        let g1 = tiny_graph();
        let mut g2 = tiny_graph();
        let dt = math_core::oxigraph::model::NamedNode::new(xsd::DOUBLE.as_str()).unwrap();
        g2.insert(&Triple::new(
            math_core::oxigraph::model::NamedNode::new("https://example.org/b").unwrap(),
            math_core::oxigraph::model::NamedNode::new("https://example.org/p").unwrap(),
            Term::from(math_core::oxigraph::model::Literal::new_typed_literal(
                "1.0", dt,
            )),
        ));
        assert_ne!(canonical_hash(&g1), canonical_hash(&g2));
    }
}
