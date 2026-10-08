//! The derived Mermaid view.
//!
//! A diagram is a *view*, not a source of truth: it is regenerated from the report
//! and carries a manifest saying how many nodes and edges it drew and how many it
//! left out. That manifest is what makes the rendering honest — a diagram that
//! silently dropped most of the graph would look complete, and the manifest is the
//! record that it did not.

use std::collections::BTreeSet;

use crate::model::CodexReport;

/// How to render the view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewOptions {
    /// Render only this module and the modules it touches directly.
    pub focus: Option<String>,
}

/// What the view contains, so a reader can tell a filtered diagram from a full one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewManifest {
    /// Module nodes drawn.
    pub nodes: usize,
    /// Dependency edges drawn.
    pub edges: usize,
    /// Module nodes present in the report but not drawn.
    pub hidden_nodes: usize,
    /// Dependency edges present in the report but not drawn.
    pub hidden_edges: usize,
}

/// A rendered view and its manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidView {
    /// The `graph LR` document.
    pub mermaid: String,
    /// What was drawn and what was hidden.
    pub manifest: ViewManifest,
}

fn escape(label: &str) -> String {
    label.replace('"', "'")
}

/// Render the module dependency graph as Mermaid.
pub fn mermaid(report: &CodexReport, options: &ViewOptions) -> MermaidView {
    let mut modules: Vec<&crate::model::ModuleRecord> = report.modules.iter().collect();
    modules.sort_by(|a, b| a.module_uri.cmp(&b.module_uri));

    // A focus keeps one module and its immediate neighbours on either side.
    let visible: BTreeSet<&str> = match &options.focus {
        None => modules.iter().map(|m| m.module_uri.as_str()).collect(),
        Some(focus) => {
            let mut set: BTreeSet<&str> = BTreeSet::new();
            set.insert(focus.as_str());
            for dep in &report.dependencies {
                if dep.from_uri == *focus {
                    set.insert(dep.to_uri.as_str());
                }
                if dep.to_uri == *focus {
                    set.insert(dep.from_uri.as_str());
                }
            }
            set
        }
    };

    let drawn: Vec<&crate::model::ModuleRecord> = modules
        .iter()
        .filter(|m| visible.contains(m.module_uri.as_str()))
        .copied()
        .collect();

    let mut ids: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    for (i, m) in drawn.iter().enumerate() {
        ids.insert(m.module_uri.as_str(), format!("n{i}"));
    }

    let mut edges: BTreeSet<(&str, &str)> = BTreeSet::new();
    for dep in &report.dependencies {
        if dep.from_uri == dep.to_uri {
            continue;
        }
        if let (Some(from), Some(to)) =
            (ids.get(dep.from_uri.as_str()), ids.get(dep.to_uri.as_str()))
        {
            edges.insert((from.as_str(), to.as_str()));
        }
    }
    let all_edges: usize = report
        .dependencies
        .iter()
        .filter(|d| d.from_uri != d.to_uri)
        .count();

    let mut out = String::from("graph LR\n");
    for m in &drawn {
        let id = &ids[m.module_uri.as_str()];
        out.push_str(&format!(
            "  {id}[\"{}\"]\n",
            escape(m.capability.trim_start_matches("mm:"))
        ));
    }
    for (from, to) in &edges {
        out.push_str(&format!("  {from} --> {to}\n"));
    }

    MermaidView {
        mermaid: out,
        manifest: ViewManifest {
            nodes: drawn.len(),
            edges: edges.len(),
            hidden_nodes: modules.len() - drawn.len(),
            hidden_edges: all_edges - edges.len(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn module(rel: &str, capability: &str) -> ModuleRecord {
        ModuleRecord {
            rel_path: rel.into(),
            module_uri: mm_core::codex::module_iri(rel).into_string(),
            name: rel.into(),
            version: "0.1.0".into(),
            kind: ModuleKind::RustCrate,
            category: None,
            crate_name: Some(rel.into()),
            owned_phase: 2,
            capability: capability.into(),
            content_hash: "aa".repeat(32),
            copied_from: None,
            copied_revision: None,
            copied_license: None,
            origin: Some("metamind".into()),
            manifest_only: false,
            declared_uri: None,
            tbox_functions: vec![],
            depends_on: vec![],
        }
    }

    fn report() -> CodexReport {
        let a = mm_core::codex::module_iri("crates/a").into_string();
        let b = mm_core::codex::module_iri("crates/b").into_string();
        CodexReport {
            modules: vec![module("crates/a", "mm:A"), module("crates/b", "mm:B")],
            dependencies: vec![DependencyEdge {
                from_uri: a,
                to_uri: b,
                kind: DepKind::Crate,
            }],
            ..CodexReport::default()
        }
    }

    #[test]
    fn the_view_lists_every_module_and_dependency() {
        let view = mermaid(&report(), &ViewOptions::default());
        assert_eq!(view.manifest.nodes, 2);
        assert_eq!(view.manifest.edges, 1);
        assert_eq!(view.manifest.hidden_nodes, 0);
        assert_eq!(view.manifest.hidden_edges, 0);
        assert!(view.mermaid.starts_with("graph LR\n"));
        assert!(view.mermaid.contains("[\"A\"]"), "{}", view.mermaid);
        assert!(view.mermaid.contains("-->"), "{}", view.mermaid);
    }

    #[test]
    fn rendering_is_deterministic() {
        let a = mermaid(&report(), &ViewOptions::default());
        let b = mermaid(&report(), &ViewOptions::default());
        assert_eq!(a, b);
    }

    #[test]
    fn focusing_hides_the_rest_and_says_so() {
        let focus = mm_core::codex::module_iri("crates/a").into_string();
        let view = mermaid(&report(), &ViewOptions { focus: Some(focus) });
        assert_eq!(view.manifest.nodes, 2, "the focus's neighbour is kept");
        assert_eq!(view.manifest.hidden_nodes, 0);
        // A focus on a URI that is not a module matches nothing, and the manifest
        // says exactly how much was left out rather than drawing an empty graph
        // that looks complete.
        let unknown = mermaid(
            &report(),
            &ViewOptions {
                focus: Some("https://metamind.dev/code/module/nothing".into()),
            },
        );
        assert_eq!(unknown.manifest.nodes, 0);
        assert_eq!(unknown.manifest.hidden_nodes, 2);
        assert_eq!(unknown.manifest.hidden_edges, 1);
    }
}
