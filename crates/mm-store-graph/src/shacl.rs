//! SHACL validation.
//!
//! The validator is the vendored `rdf-shacl` native implementation, not a wrapper
//! around an external service. It covers exactly the SHACL-core fragment
//! Metamind's shapes use, and any construct outside that fragment is a hard error
//! rather than a silent skip: a shape that cannot be enforced must fail the gate,
//! never quietly pass it.

use std::path::Path;

use mm_core::store::ShaclReport;
use mm_core::MmError;

use crate::actor::map_shacl_report;

/// Validate a Turtle-encoded instance graph against a Turtle-encoded shapes
/// graph.
///
/// `graph_label` only names the graph in the error message, so a failure says
/// which graph could not be validated.
pub fn validate_turtle_text(
    shapes: &str,
    data: &str,
    graph_label: &str,
) -> Result<ShaclReport, MmError> {
    rdf_shacl::validate::validate_turtle(shapes, data)
        .map(map_shacl_report)
        .map_err(|e| {
            MmError::Codec(format!(
                "SHACL validation of graph {graph_label} could not run: {e}"
            ))
        })
}

/// Read a shapes file.
pub fn load_shapes(path: &Path) -> Result<String, MmError> {
    std::fs::read_to_string(path)
        .map_err(|e| MmError::Graph(format!("cannot read shapes {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPES: &str = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix mm: <https://metamind.dev/ontology#> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        mm:IdentityShape a sh:NodeShape ;
            sh:targetClass mm:Identity ;
            sh:property [
                sh:path mm:iri ;
                sh:minCount 1 ;
                sh:maxCount 1 ;
                sh:nodeKind sh:IRI ;
            ] ;
            sh:property [
                sh:path mm:created ;
                sh:datatype xsd:dateTime ;
                sh:minCount 1 ;
                sh:maxCount 1 ;
            ] .
    "#;

    const CONFORMING: &str = r#"
        @prefix mm: <https://metamind.dev/ontology#> .
        @prefix mmd: <https://metamind.dev/data/> .
        mmd:01h0000000000000000000001 a mm:Identity ;
            mm:iri mmd:01h0000000000000000000001 ;
            mm:created "2024-01-01T00:00:00.000000000Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> .
    "#;

    fn data_with(extra_property: &str) -> String {
        format!(
            r#"
            @prefix mm: <https://metamind.dev/ontology#> .
            @prefix mmd: <https://metamind.dev/data/> .
            mmd:01h0000000000000000000001 a mm:Identity ;
                {extra_property}
                mm:created "2024-01-01T00:00:00.000000000Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> .
            "#
        )
    }

    #[test]
    fn a_clean_graph_yields_zero_violations() {
        let report = validate_turtle_text(SHAPES, CONFORMING, "being").unwrap();
        assert!(report.conforms, "{report:?}");
        assert!(report.violations.is_empty());
    }

    #[test]
    fn a_missing_required_property_is_a_violation() {
        // No mm:iri at all.
        let data = r#"
            @prefix mm: <https://metamind.dev/ontology#> .
            @prefix mmd: <https://metamind.dev/data/> .
            mmd:01h0000000000000000000001 a mm:Identity ;
                mm:created "2024-01-01T00:00:00.000000000Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> .
        "#;
        let report = validate_turtle_text(SHAPES, data, "being").unwrap();
        assert!(!report.conforms);
        assert!(!report.violations.is_empty());
        let violation = &report.violations[0];
        assert!(
            violation.path.contains("iri"),
            "unexpected path {violation:?}"
        );
        assert!(violation.message.contains("minCount"), "{violation:?}");
        assert!(!violation.severity.is_empty());
    }

    #[test]
    fn more_than_one_value_violates_max_count() {
        let data = data_with("mm:iri mmd:a, mmd:b ;");
        let report = validate_turtle_text(SHAPES, data.as_str(), "being").unwrap();
        assert!(!report.conforms);
        assert!(report.violations[0].message.contains("maxCount"));
    }

    #[test]
    fn a_non_iri_value_violates_node_kind() {
        let data = data_with("mm:iri \"not an iri\" ;");
        let report = validate_turtle_text(SHAPES, data.as_str(), "being").unwrap();
        assert!(
            !report.conforms,
            "a literal must not satisfy sh:nodeKind sh:IRI"
        );
    }

    #[test]
    fn an_untyped_instance_is_not_targeted_and_therefore_not_checked() {
        // SHACL targets by class: a node that is not an mm:Identity is out of scope.
        let data = r#"
            @prefix mm: <https://metamind.dev/ontology#> .
            @prefix mmd: <https://metamind.dev/data/> .
            mmd:01h0000000000000000000001 a mm:SomethingElse .
        "#;
        let report = validate_turtle_text(SHAPES, data, "being").unwrap();
        assert!(report.conforms);
    }

    #[test]
    fn malformed_shapes_are_a_hard_error_not_a_silent_pass() {
        let err =
            validate_turtle_text("mm:Broken a sh:NodeShape ; sh:xxxx ", "{}", "being").unwrap_err();
        assert!(matches!(err, MmError::Codec(_)), "got {err:?}");
    }

    #[test]
    fn load_shapes_reports_a_missing_file() {
        let err = load_shapes(Path::new("/nonexistent/shapes.ttl")).unwrap_err();
        assert!(matches!(err, MmError::Graph(_)));
        assert!(err.to_string().contains("shapes.ttl"));
    }
}
