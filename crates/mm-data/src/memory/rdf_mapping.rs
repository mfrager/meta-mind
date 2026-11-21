//! RDF mapping for the memory tiers: graph IRIs, prefixes and query patterns.
//!
//! These are the shapes later phases and tools query directly. The Rust stores
//! use the same graph IRIs, so a pattern here and a record there cannot drift.

use crate::rdf::{
    project_memory_graph, segment_graph, session_graph, user_memory_graph, MEM_NS, MM_NS, PROV_NS,
};

/// A tier and the graph family that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierGraph {
    /// Tier number, 1–4.
    pub tier: u8,
    /// Human-readable name.
    pub name: &'static str,
    /// Graph IRI pattern (`{id}` is substituted).
    pub pattern: &'static str,
}

/// The four tier graph patterns.
pub const TIER_GRAPHS: &[TierGraph] = &[
    TierGraph {
        tier: 1,
        name: "episodic",
        pattern: "https://metamind.dev/agentstream/session/{id}",
    },
    TierGraph {
        tier: 2,
        name: "segments",
        pattern: "https://metamind.dev/memory/segments/{id}",
    },
    TierGraph {
        tier: 3,
        name: "projects",
        pattern: "https://metamind.dev/memory/projects/{id}",
    },
    TierGraph {
        tier: 4,
        name: "users",
        pattern: "https://metamind.dev/memory/users/{id}",
    },
];

/// The graph for a tier and identifier.
pub fn graph_for_tier(tier: u8, id: &str) -> Option<String> {
    match tier {
        1 => Some(session_graph(id)),
        2 => Some(segment_graph(id)),
        3 => Some(project_memory_graph(id)),
        4 => Some(user_memory_graph(id)),
        _ => None,
    }
}

/// The prefixes every memory query starts with.
pub const MEMORY_PREFIXES: &str = "PREFIX mm: <https://metamind.dev/ontology#>\n\
PREFIX mem: <https://metamind.dev/memory#>\n\
PREFIX as: <https://metamind.dev/agentstream#>\n\
PREFIX prov: <http://www.w3.org/ns/prov#>\n\
PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
PREFIX skos: <http://www.w3.org/2004/02/skos/core#>\n\
PREFIX dct: <http://purl.org/dc/terms/>\n\
PREFIX doap: <http://usefulinc.com/ns/doap#>";

/// Find decisions about a topic in a project's segments.
pub fn decisions_about_pattern(project_id: &str) -> String {
    format!(
        "{MEMORY_PREFIXES}\n\
SELECT ?decision ?topic ?choice ?rationale ?confidence\n\
WHERE {{\n\
  GRAPH <{}> {{\n\
    ?decision a mem:DecisionRecord ;\n\
              mem:topic ?topic ;\n\
              mem:choice ?choice .\n\
  }}\n\
  FILTER(CONTAINS(LCASE(STR(?topic)), LCASE(\"{{topic}}\")))\n\
}}",
        project_memory_graph(project_id)
    )
}

/// Find open commitments for a user.
pub fn open_commitments_pattern(user_id: &str) -> String {
    format!(
        "{MEMORY_PREFIXES}\n\
SELECT ?commitment ?description ?status\n\
WHERE {{\n\
  GRAPH <{}> {{\n\
    ?commitment a mem:Commitment ;\n\
                mem:description ?description ;\n\
                mem:status ?status .\n\
  }}\n\
  FILTER(?status IN (\"open\", \"in_progress\", \"blocked\"))\n\
}}",
        user_memory_graph(user_id)
    )
}

/// Find segments whose narrative mentions a topic.
pub fn segments_about_pattern(project_id: &str) -> String {
    format!(
        "{MEMORY_PREFIXES}\n\
SELECT ?segment ?narrative\n\
WHERE {{\n\
  GRAPH <{}> {{\n\
    ?segment a mem:CompressedSegment ;\n\
             mm:name ?narrative .\n\
  }}\n\
}}",
        project_memory_graph(project_id)
    )
}

/// The ontology prefixes used when writing memory triples.
pub const ONTOLOGY_PREFIXES: &str = "https://metamind.dev/ontology#";

/// The memory class namespace.
pub const MEMORY_CLASS_NS: &str = MEM_NS;

/// The provenance namespace, re-exported for query builders.
pub const PROVENANCE_NS: &str = PROV_NS;

/// The MetaMind namespace.
pub const METAMIND_NS: &str = MM_NS;
