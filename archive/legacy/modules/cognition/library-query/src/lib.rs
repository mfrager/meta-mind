//! The `library-query` module.
//!
//! Phase 7's cognitive library, stated as a contract. The behaviour lives in
//! `mm-library`, where the applicability arithmetic, the structure-mapped case
//! retrieval, and the verification gate on skills are deterministic; duplicating
//! any of that here would create a second place for it to drift.
//!
//! What this module owns is *what a caller may rely on*: which surfaces exist, the
//! tables each reads and writes, the invariant each must respect, and one sentence
//! per surface. A caller that reads [`surfaces`] learns, without opening a store,
//! that selection is ranked rather than defaulted and that an unverified skill is
//! never returned.
//!
//! The manifest is embedded at compile time, so a module whose `plugin.toml` is
//! malformed fails to *build* rather than to load.
#![forbid(unsafe_code)]

use mm_core::MmError;
use serde::{Deserialize, Serialize};

/// The module's embedded manifest.
pub const MANIFEST_TOML: &str = include_str!("../plugin.toml");

/// `cognition.library_applicable` — rank the techniques applicable to a state.
pub const LIBRARY_APPLICABLE: &str = "cognition.library_applicable";
/// `cognition.skill_retrieve` — retrieve only verified skills.
pub const SKILL_RETRIEVE: &str = "cognition.skill_retrieve";
/// `cognition.case_retrieve` — retrieve precedents by structure-mapped analogy.
pub const CASE_RETRIEVE: &str = "cognition.case_retrieve";

/// Every T-Box function this module exposes, in a stable order.
pub const SURFACE_FUNCTIONS: [&str; 3] = [LIBRARY_APPLICABLE, SKILL_RETRIEVE, CASE_RETRIEVE];

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
            function: LIBRARY_APPLICABLE,
            reads: &["library_entries", "applicability_runs"],
            writes: &["applicability_runs"],
            invariant: Some("no_silent_default_technique"),
            contract: "Selection is ranked and never a default: every returned \
                       technique carries an applicability score, and the score is pure \
                       arithmetic over the state the caller passes in.",
        },
        Surface {
            function: SKILL_RETRIEVE,
            reads: &["skills"],
            writes: &[],
            invariant: Some("no_unverified_skill"),
            contract: "Only a Verified skill is retrieved: a Draft or Failing skill is \
                       never returned, whatever its score.",
        },
        Surface {
            function: CASE_RETRIEVE,
            reads: &["cases", "case_map"],
            writes: &[],
            invariant: None,
            contract: "Cases are ranked by case_utility and explained by \
                       structure-mapped correspondences, so an analogy is structural \
                       rather than surface.",
        },
    ]
}

/// Parse the embedded manifest.
pub fn manifest() -> Result<PluginManifest, MmError> {
    toml::from_str(MANIFEST_TOML)
        .map_err(|e| MmError::Config(format!("library-query plugin.toml is invalid: {e}")))
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

/// The handler behind `cognition.library_applicable`.
pub fn library_applicable() -> Surface {
    surfaces()[0].clone()
}

/// The handler behind `cognition.skill_retrieve`.
pub fn skill_retrieve() -> Surface {
    surfaces()[1].clone()
}

/// The handler behind `cognition.case_retrieve`.
pub fn case_retrieve() -> Surface {
    surfaces()[2].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_manifest_parses_and_is_well_formed() {
        let m = manifest().unwrap();
        assert_eq!(m.plugin.name, "library-query");
        assert_eq!(m.plugin.version, "0.1.0");
        assert_eq!(m.metadata.category, "cognition");
        assert_eq!(m.metadata.owned_by_phase, 7);
        assert_eq!(m.metadata.capability, "mm:TechniqueSelection");
        assert_eq!(m.tbox.functions.len(), SURFACE_FUNCTIONS.len());
        for function in SURFACE_FUNCTIONS {
            assert!(
                m.tbox.functions.contains_key(function),
                "{function} is not declared"
            );
        }
        assert_eq!(m.monad.as_ref().unwrap().operations.name, "cognition");
        assert_eq!(m.monad.as_ref().unwrap().operations.arity, 1);
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
        assert_eq!(
            m.plugin.uri,
            mm_core::iri::module("cognition/library-query").as_str()
        );
    }

    #[test]
    fn each_handler_answers_its_own_surface() {
        assert_eq!(library_applicable().function, LIBRARY_APPLICABLE);
        assert_eq!(skill_retrieve().function, SKILL_RETRIEVE);
        assert_eq!(case_retrieve().function, CASE_RETRIEVE);
    }

    #[test]
    fn a_malformed_manifest_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugin.toml");
        std::fs::write(&path, "[plugin]\nname = \"x\"\n").unwrap();
        assert!(matches!(
            manifest_at(&path).unwrap_err(),
            MmError::Config(_)
        ));
    }
}
