//! Graph aliases and routing.
//!
//! The design registers the kernel's semantic stores as satellite graphs
//! (`being`, `memory`, …). This layer keeps the aliases in one place and
//! resolves them to the graph IRIs the registries write, so a caller never
//! embeds an IRI by hand.

use std::collections::BTreeMap;

use crate::rdf::{MMD_NS, MM_NS};

/// A named graph alias and the IRI it resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphAlias {
    /// Short name used in configuration and tools.
    pub name: String,
    /// Full graph IRI.
    pub iri: String,
}

/// The interactive alias table.
#[derive(Debug, Clone)]
pub struct GraphAliases {
    aliases: BTreeMap<String, String>,
}

impl Default for GraphAliases {
    fn default() -> Self {
        let mut aliases = BTreeMap::new();
        for name in [
            "being",
            "memory",
            "epistemic",
            "library",
            "decision",
            "tools",
            "selfeng",
            "metacog",
            "code",
        ] {
            aliases.insert(name.to_owned(), format!("{MMD_NS}graph/{name}"));
        }
        aliases.insert(
            "ontology".to_owned(),
            "https://metamind.dev/graph/ontology".to_owned(),
        );
        Self { aliases }
    }
}

impl GraphAliases {
    /// Add or replace an alias.
    pub fn insert(&mut self, name: impl Into<String>, iri: impl Into<String>) {
        self.aliases.insert(name.into(), iri.into());
    }

    /// Resolve an alias. A full IRI (`https://…` or `urn:…`) passes through.
    pub fn resolve(&self, name: &str) -> String {
        if name.starts_with("https://") || name.starts_with("urn:") {
            return name.to_owned();
        }
        self.aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| format!("{MMD_NS}graph/{name}"))
    }

    /// Every alias, sorted.
    pub fn all(&self) -> Vec<GraphAlias> {
        self.aliases
            .iter()
            .map(|(name, iri)| GraphAlias {
                name: name.clone(),
                iri: iri.clone(),
            })
            .collect()
    }
}

/// The graph holding semantic instance data for one store alias.
pub fn store_graph(alias: &str) -> String {
    format!("{MMD_NS}graph/{alias}")
}

/// The ontology classes that describe a graph, for documentation and tools.
pub fn ontology_class(local: &str) -> String {
    format!("{MM_NS}{local}")
}
