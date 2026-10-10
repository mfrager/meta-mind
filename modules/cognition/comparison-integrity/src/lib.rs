//! The `comparison-integrity` module.
//!
//! Design §20, stated as a contract. The behaviour lives in
//! [`mm_decision::compare`], where the validity rules and the normalization
//! arithmetic are deterministic and already covered by the crate's own tests and by
//! `bench/comparison/fixtures.jsonl`; duplicating any of it here would create a
//! second place for it to drift.
//!
//! What this module owns is *what a caller may rely on*: which surface exists, the
//! tables it reads and writes, the invariant it must respect, and one sentence per
//! surface. A caller that reads [`surfaces`] learns, without opening a store, that a
//! comparison is *checked* before it is used and that a mismatch is a refusal rather
//! than a coerced number — which is the entire point of the subsystem, because most
//! "wrong" decisions are really invalid comparisons that nobody validated.
//!
//! The manifest is embedded at compile time, so a module whose `plugin.toml` is
//! malformed fails to *build* rather than to load.
#![forbid(unsafe_code)]

use mm_core::MmError;
use serde::{Deserialize, Serialize};

/// The module's embedded manifest.
pub const MANIFEST_TOML: &str = include_str!("../plugin.toml");

/// `cognition.compare_contract` — check a pair of comparison contracts before any
/// comparison is made.
pub const COMPARE_CONTRACT: &str = "cognition.compare_contract";

/// Every T-Box function this module exposes, in a stable order.
pub const SURFACE_FUNCTIONS: [&str; 1] = [COMPARE_CONTRACT];

/// The tables a comparison check reads.
pub const READS: [&str; 2] = ["comparisons", "decisions"];

/// The tables a comparison check writes.
pub const WRITES: [&str; 1] = ["comparisons"];

/// The invariant this module must respect, as stored in `comparison_integrity_runs`
/// when the firewall turns a non-`Valid` verdict into `REPLAN`.
pub const INVARIANT: &str = "no_unchecked_comparison";

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
    vec![Surface {
        function: COMPARE_CONTRACT,
        reads: &READS,
        writes: &WRITES,
        invariant: Some(INVARIANT),
        contract: "A comparison is admissible only when `check_contract` says it is: \
                   the same objective and object class, dimensionally consistent units, \
                   overlapping timeframes and matching conditions, definitions and \
                   constraints. A mismatch is `NonComparable` or `Rejected` and is \
                   never coerced into a number, and the normalized values are \n\
                   produced by `normalize` only for a `Valid` pair.",
    }]
}

/// The verdicts a caller can receive, in the compare module's own order.
///
/// Re-exported as names rather than as the enum so a caller that only needs to
/// branch on the three strings does not have to depend on the type's serde shape.
pub fn verdict_names() -> Vec<&'static str> {
    mm_decision::compare::COMPARISON_VERDICTS
        .iter()
        .map(|verdict| verdict.as_str())
        .collect()
}

/// The violation codes a report can carry, in the fixed order they are emitted.
pub fn violation_codes() -> Vec<&'static str> {
    mm_decision::compare::VIOLATION_CODES.to_vec()
}

/// Parse the embedded manifest.
pub fn manifest() -> Result<PluginManifest, MmError> {
    toml::from_str(MANIFEST_TOML)
        .map_err(|e| MmError::Config(format!("comparison-integrity plugin.toml is invalid: {e}")))
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

/// The handler behind `cognition.compare_contract`.
pub fn compare_contract() -> Surface {
    surfaces()[0].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_manifest_parses_and_is_well_formed() {
        let m = manifest().unwrap();
        assert_eq!(m.plugin.name, "comparison-integrity");
        assert_eq!(m.plugin.version, "0.1.0");
        assert_eq!(m.metadata.category, "cognition");
        assert_eq!(m.metadata.owned_by_phase, 9);
        assert_eq!(m.metadata.capability, "mm:ComparisonIntegrity");
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
            mm_core::iri::module("cognition/comparison-integrity").as_str()
        );
    }

    #[test]
    fn the_handler_answers_its_own_surface() {
        assert_eq!(compare_contract().function, COMPARE_CONTRACT);
        assert_eq!(compare_contract().invariant, Some(INVARIANT));
    }

    #[test]
    fn the_verdict_names_and_violation_codes_are_the_compare_modules() {
        assert_eq!(verdict_names(), vec!["valid", "non_comparable", "rejected"]);
        assert_eq!(violation_codes().len(), 11);
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
