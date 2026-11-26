//! Named-graph versioning (master_design.md §4.8): snapshot a model's default
//! graph into a named graph, and diff two named graphs by their triples.

use std::collections::BTreeSet;

use oxigraph::model::{GraphName, NamedNode, Quad};

use crate::model::{named_node_iri, MathModel};

/// A triple as `(subject, predicate, object)` strings.
pub type Triple = (String, String, String);

/// Copy every default-graph quad into the named graph `graph_iri`.
pub fn snapshot_default_graph(model: &MathModel, graph_iri: &str) -> Result<(), String> {
    let g = NamedNode::new(graph_iri).map_err(|e| e.to_string())?;
    let mut to_insert = Vec::new();
    for quad in model
        .store
        .quads_for_pattern(None, None, None, None)
        .flatten()
    {
        if matches!(quad.graph_name, GraphName::DefaultGraph) {
            to_insert.push(Quad::new(
                quad.subject,
                quad.predicate,
                quad.object,
                GraphName::NamedNode(g.clone()),
            ));
        }
    }
    for q in to_insert {
        model.store.insert(&q).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// All triples stored in the named graph `graph_iri`.
pub fn triples_in_graph(model: &MathModel, graph_iri: &str) -> BTreeSet<Triple> {
    let mut out = BTreeSet::new();
    for quad in model
        .store
        .quads_for_pattern(None, None, None, None)
        .flatten()
    {
        if let GraphName::NamedNode(n) = &quad.graph_name {
            if named_node_iri(n) == graph_iri {
                out.insert((
                    quad.subject.to_string(),
                    quad.predicate.to_string(),
                    quad.object.to_string(),
                ));
            }
        }
    }
    out
}

/// The difference between two named graphs: triples added and removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionDiff {
    pub added: BTreeSet<Triple>,
    pub removed: BTreeSet<Triple>,
}

impl VersionDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// Diff `from` → `to`, where both are named-graph IRIs in the store.
pub fn diff_versions(model: &MathModel, from: &str, to: &str) -> VersionDiff {
    let a = triples_in_graph(model, from);
    let b = triples_in_graph(model, to);
    VersionDiff {
        added: b.difference(&a).cloned().collect(),
        removed: a.difference(&b).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocabulary::PRED_VALUE;

    const TTL: &str = "@prefix model: <https://example.org/ns/model#> .\n@prefix ex: <https://example.org/us-gdp/> .\nex:MPC a model:Parameter ; model:value 0.8 .\n";
    const MPC: &str = "https://example.org/us-gdp/MPC";

    #[test]
    fn version_diff_reports_changed_triples() {
        let model = MathModel::new().unwrap();
        model.load_turtle(TTL, "https://example.org/").unwrap();

        snapshot_default_graph(&model, "https://example.org/version/v1").unwrap();
        model.set_numeric_value(MPC, PRED_VALUE, 0.75).unwrap();
        snapshot_default_graph(&model, "https://example.org/version/v2").unwrap();

        let diff = diff_versions(
            &model,
            "https://example.org/version/v1",
            "https://example.org/version/v2",
        );
        assert_eq!(diff.added.len(), 1, "added = {:?}", diff.added);
        assert_eq!(diff.removed.len(), 1, "removed = {:?}", diff.removed);
        assert!(diff.added.iter().any(|(_, _, o)| o.contains("0.75")));
        assert!(diff.removed.iter().any(|(_, _, o)| o.contains("0.8")));
    }

    #[test]
    fn identical_versions_have_empty_diff() {
        let model = MathModel::new().unwrap();
        model.load_turtle(TTL, "https://example.org/").unwrap();
        snapshot_default_graph(&model, "https://example.org/version/v1").unwrap();
        snapshot_default_graph(&model, "https://example.org/version/v1copy").unwrap();
        let diff = diff_versions(
            &model,
            "https://example.org/version/v1",
            "https://example.org/version/v1copy",
        );
        assert!(diff.is_empty());
    }
}
