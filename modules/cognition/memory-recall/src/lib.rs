//! The `memory-recall` module.
//!
//! Phase 5's long-term memory organ, stated as a contract. The behaviour lives in
//! `mm-memory`, where the retention curve, the admission gate, and the forget
//! refusals are deterministic; duplicating any of that here would create a second
//! place for it to drift.
//!
//! What this module owns is *what a caller may rely on*: which surfaces exist, the
//! tables each reads and writes, the invariant each must respect, and one sentence
//! per surface. A caller that reads [`surfaces`] learns, without opening a store,
//! that `memory.forget` may refuse and that a refusal is a control event rather
//! than a failure.
//!
//! The manifest is embedded at compile time, so a module whose `plugin.toml` is
//! malformed fails to *build* rather than to load.
#![forbid(unsafe_code)]

use mm_core::MmError;
use serde::{Deserialize, Serialize};

/// The module's embedded manifest.
pub const MANIFEST_TOML: &str = include_str!("../plugin.toml");

/// `memory.remember` — admit a memory.
pub const REMEMBER: &str = "memory.remember";
/// `memory.recall` — hybrid retrieval.
pub const RECALL: &str = "memory.recall";
/// `memory.consolidate` — episodes into patterns into a summary tree.
pub const CONSOLIDATE: &str = "memory.consolidate";
/// `memory.forget` — retention decay, with refusals.
pub const FORGET: &str = "memory.forget";
/// `memory.mistakes` — mistakes and near misses, with their corrective rules.
pub const MISTAKES: &str = "memory.mistakes";
/// `memory.procedures` — skill records and habit promotion.
pub const PROCEDURES: &str = "memory.procedures";
/// `memory.summaries` — the summary tree and its communities.
pub const SUMMARIES: &str = "memory.summaries";

/// Every T-Box function this module exposes, in a stable order.
pub const SURFACE_FUNCTIONS: [&str; 7] = [
    REMEMBER,
    RECALL,
    CONSOLIDATE,
    FORGET,
    MISTAKES,
    PROCEDURES,
    SUMMARIES,
];

/// A module manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// The plugin identity.
    pub plugin: PluginSection,
    /// Phase and capability metadata.
    pub metadata: Metadata,
    /// T-Box functions, if the module has any.
    #[serde(default)]
    pub tbox: TboxSection,
    /// The monad operation the module belongs to.
    #[serde(default)]
    pub monad: Option<MonadSection>,
    /// Build settings.
    #[serde(default)]
    pub build: Option<BuildSection>,
}

/// Identity of a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSection {
    /// Directory name of the module.
    pub name: String,
    /// The stable, path-derived module IRI.
    pub uri: String,
    /// The module version.
    pub version: String,
}

/// Who owns the module and what it provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// Module category, e.g. `cognition`.
    pub category: String,
    /// The phase that introduced the module.
    pub owned_by_phase: u32,
    /// The ontology capability the module implements.
    pub capability: String,
}

/// The T-Box functions a module exposes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TboxSection {
    /// Function name to implementation.
    #[serde(default)]
    pub functions: std::collections::BTreeMap<String, TboxFunction>,
}

/// One T-Box function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TboxFunction {
    /// The Rust path of the handler.
    pub source: String,
}

/// The monad operations a module contributes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonadSection {
    /// The operation this module contributes.
    pub operations: MonadOperation,
}

/// One monad operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonadOperation {
    /// Operation name.
    pub name: String,
    /// Operation arity.
    pub arity: u32,
}

/// Build settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildSection {
    /// The Rust edition the module is written in.
    pub rust_edition: String,
}

/// What one surface exposes.
///
/// Only `Serialize`: the tables and invariants are borrowed from `'static`
/// constants, so a surface is written out, never read back from a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Surface {
    /// The T-Box function name.
    pub function: &'static str,
    /// The tables the surface reads.
    pub reads: &'static [&'static str],
    /// The tables the surface writes.
    pub writes: &'static [&'static str],
    /// The invariant the surface must respect, if any.
    pub invariant: Option<&'static str>,
    /// One sentence a caller can rely on.
    pub contract: &'static str,
}

/// Every surface, in the same order as [`SURFACE_FUNCTIONS`].
pub fn surfaces() -> Vec<Surface> {
    vec![
        Surface {
            function: REMEMBER,
            reads: &["memories", "entity_edges"],
            writes: &["memories", "memory_cues", "memory_entities", "memory_links"],
            invariant: Some("no_memory_without_provenance"),
            contract: "A memory is validated before it is stored: a nil provenance \
                       node, empty content, or an out-of-range confidence is refused. \
                       A duplicate is flagged and not stored twice.",
        },
        Surface {
            function: RECALL,
            reads: &["memories", "memory_cues", "memory_entities", "memory_links"],
            writes: &["memory_access"],
            invariant: None,
            contract: "Retrieval is deterministic: fixed channel weights, a fixed \
                       ULID tie-break, and no clock other than the instant the \
                       caller passes in.",
        },
        Surface {
            function: CONSOLIDATE,
            reads: &["memories", "memory_links"],
            writes: &[
                "memories",
                "memory_links",
                "memory_consolidations",
                "memory_summaries",
                "memory_communities",
            ],
            invariant: Some("no_history_rewrite"),
            contract: "Sources are archived only after the target commits, and the \
                       lineage stays queryable: every source resolves from its \
                       target through `consolidatedFrom`.",
        },
        Surface {
            function: FORGET,
            reads: &[
                "memories",
                "memory_access",
                "memory_links",
                "commitments",
            ],
            writes: &["memories"],
            invariant: Some("no_fabricated_autobiography"),
            contract: "Forgetting archives and never deletes. A protected record, a \
                       developmental record, and a record serving an open commitment \
                       are refused, and every refusal is logged with what protected it.",
        },
        Surface {
            function: MISTAKES,
            reads: &["memories", "mistakes", "memory_links"],
            writes: &["memories", "mistakes", "memory_links"],
            invariant: Some("no_action_without_evidence"),
            contract: "A mistake always names a corrective rule: one is derived when \
                       the caller does not supply one, so a lesson can never be stored \
                       without something to do about it.",
        },
        Surface {
            function: PROCEDURES,
            reads: &["memories", "memory_cues"],
            writes: &["memories", "memory_cues"],
            invariant: None,
            contract: "A procedure is a `procedural` memory whose content is canonical \
                       JSON; it becomes a habit at three successes and a proficiency \
                       of 0.8, never by repetition alone.",
        },
        Surface {
            function: SUMMARIES,
            reads: &["memories", "memory_summaries", "memory_communities", "memory_links"],
            writes: &["memories", "memory_summaries", "memory_communities", "memory_links"],
            invariant: Some("no_history_rewrite"),
            contract: "The summary tree is a property of the store, not of one run: a \
                       node covers at least two memories, and a root exists only when \
                       there is more than one child to cover.",
        },
    ]
}

/// Parse the embedded manifest.
pub fn manifest() -> Result<PluginManifest, MmError> {
    toml::from_str(MANIFEST_TOML)
        .map_err(|e| MmError::Config(format!("memory-recall plugin.toml is invalid: {e}")))
}

/// Read and validate a manifest from disk.
pub fn manifest_at(path: &std::path::Path) -> Result<PluginManifest, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    toml::from_str(&raw).map_err(|e| MmError::Config(format!("{} is invalid: {e}", path.display())))
}

/// This module's directory, as compiled.
pub fn module_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// This module's manifest path, as compiled.
pub fn manifest_path() -> std::path::PathBuf {
    module_dir().join("plugin.toml")
}

// ------------------------------------------------------------------ handlers ---

/// The handler behind `memory.remember`.
pub fn remember() -> Surface {
    surfaces()[0].clone()
}

/// The handler behind `memory.recall`.
pub fn recall() -> Surface {
    surfaces()[1].clone()
}

/// The handler behind `memory.consolidate`.
pub fn consolidate() -> Surface {
    surfaces()[2].clone()
}

/// The handler behind `memory.forget`.
pub fn forget() -> Surface {
    surfaces()[3].clone()
}

/// The handler behind `memory.mistakes`.
pub fn mistakes() -> Surface {
    surfaces()[4].clone()
}

/// The handler behind `memory.procedures`.
pub fn procedures() -> Surface {
    surfaces()[5].clone()
}

/// The handler behind `memory.summaries`.
pub fn summaries() -> Surface {
    surfaces()[6].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_manifest_parses_and_is_well_formed() {
        let m = manifest().unwrap();
        assert_eq!(m.plugin.name, "memory-recall");
        assert_eq!(m.plugin.version, "0.1.0");
        assert_eq!(m.metadata.category, "cognition");
        assert_eq!(m.metadata.owned_by_phase, 5);
        assert_eq!(m.metadata.capability, "mm:MemoryStore");
        assert_eq!(m.tbox.functions.len(), SURFACE_FUNCTIONS.len());
        for function in SURFACE_FUNCTIONS {
            assert!(
                m.tbox.functions.contains_key(function),
                "{function} is not declared"
            );
        }
        assert_eq!(m.monad.as_ref().unwrap().operations.name, "cognition");
        assert_eq!(m.build.as_ref().unwrap().rust_edition, "2021");
    }

    #[test]
    fn the_manifest_on_disk_matches_the_one_compiled_in() {
        assert_eq!(
            manifest_at(&manifest_path()).unwrap(),
            manifest().unwrap(),
            "plugin.toml drifted from the compiled manifest"
        );
    }

    #[test]
    fn the_uri_is_the_path_derived_form_for_this_directory() {
        let m = manifest().unwrap();
        assert_eq!(m.plugin.uri, mm_core::iri::module("cognition/memory-recall").as_str());
    }

    #[test]
    fn each_handler_answers_its_own_surface() {
        assert_eq!(remember().function, REMEMBER);
        assert_eq!(recall().function, RECALL);
        assert_eq!(consolidate().function, CONSOLIDATE);
        assert_eq!(forget().function, FORGET);
        assert_eq!(mistakes().function, MISTAKES);
        assert_eq!(procedures().function, PROCEDURES);
        assert_eq!(summaries().function, SUMMARIES);
    }

    #[test]
    fn a_malformed_manifest_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugin.toml");
        std::fs::write(&path, "[plugin]\nname = \"x\"\n").unwrap();
        assert!(matches!(manifest_at(&path).unwrap_err(), MmError::Config(_)));
    }
}
