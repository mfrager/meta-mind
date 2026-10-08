//! Canonical RDF and term rendering.
//!
//! All RDF that Metamind hashes or diffs goes through the vendored `rdf-codec`
//! canonicalizer: triples are sorted into an order-independent N-Triples
//! rendering, so two graphs containing the same triples produce the same hash no
//! matter how they were assembled. That is the property replay and diffing rest
//! on, and it is asserted here over 10k triples.

use mm_core::MmError;
use oxigraph::model::{Term, TermRef};
use serde_json::{json, Value};

/// Render an RDF term as JSON.
///
/// Named nodes become their IRI and blank nodes become `_:label`; a plain
/// literal becomes its value, and a typed or language-tagged literal becomes an
/// object so the datatype survives the round trip.
pub fn term_to_json(term: &Term) -> Value {
    match term {
        Term::NamedNode(node) => Value::String(node.as_str().to_string()),
        Term::BlankNode(node) => Value::String(format!("_:{}", node.as_str())),
        Term::Literal(literal) => {
            let value = literal.value();
            if let Some(language) = literal.language() {
                return json!({ "value": value, "language": language });
            }
            let datatype = literal.datatype().as_str();
            if datatype == "http://www.w3.org/2001/XMLSchema#string" {
                Value::String(value.to_string())
            } else {
                json!({ "value": value, "datatype": datatype })
            }
        }
    }
}

/// The same rendering for a term reference.
pub fn term_ref_to_json(term: TermRef<'_>) -> Value {
    term_to_json(&term.into_owned())
}

/// Parse Turtle and return its canonical, order-independent N-Triples form.
pub fn canonical_ntriples_of_turtle(turtle: &str) -> Result<String, MmError> {
    let graph = parse(turtle)?;
    Ok(rdf_codec::io::to_canonical_ntriples(&graph))
}

/// The canonical content hash of a Turtle document: **sha256 over the canonical
/// N-Triples rendering**.
///
/// The vendored `rdf-codec` provides the hard part — a deterministic,
/// order-independent triple order (`to_canonical_ntriples`). Its own
/// `canonical_hash` is a 64-bit FNV-1a digest, which is fine for bucketing but
/// too weak to carry an integrity claim, so the digest Metamind records is
/// sha256 of the canonical bytes instead. Canonicalization comes from the
/// reference code; the digest is ours.
pub fn canonical_hash_of_turtle(turtle: &str) -> Result<String, MmError> {
    let canonical = canonical_ntriples_of_turtle(turtle)?;
    Ok(mm_core::content_hash(canonical.as_bytes()))
}

/// Re-serialize Turtle through the canonical writer.
pub fn normalize_turtle(turtle: &str) -> Result<String, MmError> {
    let graph = parse(turtle)?;
    rdf_codec::io::to_turtle(&graph).map_err(|e| MmError::Codec(format!("turtle write error: {e}")))
}

fn parse(turtle: &str) -> Result<oxigraph::model::Graph, MmError> {
    rdf_codec::io::parse_turtle(turtle)
        .map_err(|e| MmError::Codec(format!("turtle parse error: {e}")))
}

/// The prefix declarations every synthetic document uses.
pub const SYNTHETIC_PREFIXES: &str =
    "@prefix mmd: <https://metamind.dev/data/> .\n@prefix mm: <https://metamind.dev/ontology#> .\n";

/// The triple lines of a synthetic document, without prefix declarations.
///
/// Emitting one triple per line lets a test shuffle *triples* while leaving the
/// document well formed — reversing prefix declarations would prove nothing
/// about canonicalization but a lot about the test being wrong.
pub fn synthetic_triples(n: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        out.push(format!("mmd:01h000000000000000000{i:04} mm:index {i} ."));
        out.push(format!(
            "mmd:01h000000000000000000{i:04} mm:label \"node {i}\" ."
        ));
    }
    out
}

/// A well-formed Turtle document containing `2 * n` triples.
pub fn synthetic_turtle(n: usize) -> String {
    let mut out = String::from(SYNTHETIC_PREFIXES);
    for triple in synthetic_triples(n) {
        out.push_str(&triple);
        out.push('\n');
    }
    out
}

/// Rebuild a document from prefixes and an explicit triple order.
pub fn document_from(triples: &[String]) -> String {
    let mut out = String::from(SYNTHETIC_PREFIXES);
    for triple in triples {
        out.push_str(triple);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const TURTLE: &str = r#"
        @prefix mm: <https://metamind.dev/ontology#> .
        @prefix mmd: <https://metamind.dev/data/> .
        mmd:01h0000000000000000000001 a mm:Being ;
            mm:name "Metamind" ;
            mm:created "2024-01-01T00:00:00.000000000Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ;
            mm:note "hello"@en ;
            mm:linked mmd:01h0000000000000000000002 .
    "#;

    #[test]
    fn canonical_hash_is_order_independent_over_10k_triples() {
        let triples = synthetic_triples(5_000); // 10_000 triples
        assert_eq!(triples.len(), 10_000);
        let forward = document_from(&triples);

        let mut reversed_triples = triples.clone();
        reversed_triples.reverse();
        let reversed = document_from(&reversed_triples);

        let a = canonical_hash_of_turtle(&forward).unwrap();
        let b = canonical_hash_of_turtle(&reversed).unwrap();
        assert_eq!(a, b, "the same triple set must hash the same either way");
        assert_eq!(a.len(), 64, "the content digest is sha256 hex");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));

        // The canonical N-Triples rendering is sorted, so it is byte-identical.
        assert_eq!(
            canonical_ntriples_of_turtle(&forward).unwrap(),
            canonical_ntriples_of_turtle(&reversed).unwrap()
        );
    }

    #[test]
    fn export_reimport_preserves_the_hash_after_normalization() {
        let original = synthetic_turtle(200);
        let normalized = normalize_turtle(&original).unwrap();
        assert_eq!(
            canonical_hash_of_turtle(&original).unwrap(),
            canonical_hash_of_turtle(&normalized).unwrap()
        );
    }

    #[test]
    fn different_content_hashes_differently() {
        let a = canonical_hash_of_turtle(&document_from(&[
            "mmd:01h0000000000000000000001 mm:index 1 .".to_string(),
        ]))
        .unwrap();
        let b = canonical_hash_of_turtle(&document_from(&[
            "mmd:01h0000000000000000000001 mm:index 2 .".to_string(),
        ]))
        .unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn malformed_turtle_is_an_error_not_a_panic() {
        let err = canonical_hash_of_turtle("this is not turtle {").unwrap_err();
        assert!(matches!(err, MmError::Codec(_)));
    }

    #[test]
    fn terms_render_with_datatype_language_and_node_kind_preserved() {
        use oxigraph::model::{BlankNode, Literal, NamedNode};
        let plain = Term::Literal(Literal::new_simple_literal("hi"));
        assert_eq!(term_to_json(&plain), json!("hi"));

        let lang = Term::Literal(Literal::new_language_tagged_literal("hi", "en").unwrap());
        assert_eq!(
            term_to_json(&lang),
            json!({"value": "hi", "language": "en"})
        );

        let typed = Term::Literal(Literal::new_typed_literal(
            "42",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        ));
        assert_eq!(
            term_to_json(&typed),
            json!({"value": "42", "datatype": "http://www.w3.org/2001/XMLSchema#integer"})
        );

        let node = Term::NamedNode(NamedNode::new("https://metamind.dev/data/x").unwrap());
        assert_eq!(term_to_json(&node), json!("https://metamind.dev/data/x"));

        let blank = Term::BlankNode(BlankNode::new("b1").unwrap());
        assert_eq!(term_to_json(&blank), json!("_:b1"));
    }

    #[test]
    fn the_sample_document_is_well_formed() {
        assert!(canonical_hash_of_turtle(TURTLE).is_ok());
    }

    proptest! {
        #[test]
        fn canonical_hash_depends_only_on_the_triple_set(count in 1usize..40) {
            let triples = synthetic_triples(count);
            let a = canonical_hash_of_turtle(&document_from(&triples)).unwrap();

            let mut shuffled = triples.clone();
            let len = shuffled.len();
            shuffled.rotate_left(count % len);
            let b = canonical_hash_of_turtle(&document_from(&shuffled)).unwrap();

            prop_assert_eq!(a, b);
        }
    }
}
