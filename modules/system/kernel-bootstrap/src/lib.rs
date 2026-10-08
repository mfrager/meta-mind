//! The `kernel-bootstrap` module.
//!
//! Metamind's modular code is nexus-style: a directory with a `plugin.toml`
//! manifest, a stable `uri`, and T-Box functions the monad can address. This
//! module is the smallest one that still exercises the whole contract — Phase 1
//! has no cognition to put in a module, so it has a bootstrap handler and its
//! manifest is parsed and asserted from the outside.
//!
//! The manifest is embedded at compile time, so a module whose `plugin.toml` is
//! malformed fails to *build* rather than to load.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mm_core::MmError;
use serde::{Deserialize, Serialize};

/// The module's embedded manifest.
pub const MANIFEST_TOML: &str = include_str!("../plugin.toml");

/// The T-Box function this module exposes.
pub const BOOT_PLAN: &str = "system.boot_plan";

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
    /// Module category, e.g. `system`.
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
    pub functions: BTreeMap<String, TboxFunction>,
}

/// One T-Box function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TboxFunction {
    /// The Rust path of the handler.
    pub source: String,
}

/// The monad operations a module contributes to.
///
/// The nexus manifest shape nests the operation under a `[monad.operations]`
/// table, so the section is a wrapper rather than the operation itself.
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

/// Parse the embedded manifest.
pub fn manifest() -> Result<PluginManifest, MmError> {
    toml::from_str(MANIFEST_TOML)
        .map_err(|e| MmError::Config(format!("kernel-bootstrap plugin.toml is invalid: {e}")))
}

/// Read and validate a manifest from disk.
///
/// Used to check that what is on disk matches what was compiled in; a module
/// whose manifest drifted from its binary is not the module we think it is.
pub fn manifest_at(path: &Path) -> Result<PluginManifest, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    toml::from_str(&raw).map_err(|e| MmError::Config(format!("{} is invalid: {e}", path.display())))
}

/// This module's directory, as compiled.
pub fn module_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// This module's manifest path, as compiled.
pub fn manifest_path() -> PathBuf {
    module_dir().join("plugin.toml")
}

/// What the kernel should do at boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootPlan {
    /// Whether every store answered.
    pub stores_ready: bool,
    /// The ordered steps the kernel will take.
    pub steps: Vec<BootStep>,
}

/// One boot step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootStep {
    /// Open the tabular store and apply migrations.
    OpenTabular,
    /// Open the RDF store.
    OpenGraph,
    /// Load the ontology T-Box.
    LoadOntology,
    /// Verify the audit chain.
    VerifyAudit,
    /// Append the kernel-boot event.
    RecordBoot,
    /// Refuse to start and say why.
    Refuse(String),
}

/// The handler behind the `system.boot_plan` T-Box function.
///
/// Phase 1's kernel never refuses to start on a healthy store; the refusal branch
/// exists so a caller can see that an unready store short-circuits the plan
/// instead of being skipped.
pub fn boot_plan(stores_ready: bool) -> BootPlan {
    if !stores_ready {
        return BootPlan {
            stores_ready: false,
            steps: vec![BootStep::Refuse("a store is not writable".to_string())],
        };
    }
    BootPlan {
        stores_ready: true,
        steps: vec![
            BootStep::OpenTabular,
            BootStep::OpenGraph,
            BootStep::LoadOntology,
            BootStep::VerifyAudit,
            BootStep::RecordBoot,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_manifest_parses_and_is_well_formed() {
        let m = manifest().unwrap();
        assert_eq!(m.plugin.name, "kernel-bootstrap");
        assert_eq!(m.plugin.version, "0.1.0");
        assert_eq!(m.metadata.category, "system");
        assert_eq!(m.metadata.owned_by_phase, 1);
        assert_eq!(m.metadata.capability, "mm:KernelBootstrap");
        assert!(m.tbox.functions.contains_key(BOOT_PLAN));
        assert_eq!(m.monad.as_ref().unwrap().operations.name, "system");
        assert_eq!(m.build.as_ref().unwrap().rust_edition, "2021");
    }

    #[test]
    fn the_manifest_on_disk_matches_the_one_compiled_in() {
        let on_disk = manifest_at(&manifest_path()).unwrap();
        assert_eq!(
            on_disk,
            manifest().unwrap(),
            "plugin.toml drifted from the compiled manifest"
        );
    }

    #[test]
    fn the_uri_is_the_path_derived_form_for_this_directory() {
        let m = manifest().unwrap();
        assert_eq!(
            m.plugin.uri,
            mm_core::iri::module("system/kernel-bootstrap").as_str()
        );
        assert_eq!(
            m.plugin.uri,
            "https://metamind.dev/code/module/system/kernel-bootstrap"
        );
    }

    #[test]
    fn an_unready_store_short_circuits_the_plan_into_a_refusal() {
        let plan = boot_plan(false);
        assert!(!plan.stores_ready);
        assert_eq!(plan.steps.len(), 1);
        assert!(matches!(plan.steps[0], BootStep::Refuse(_)));
    }

    #[test]
    fn a_ready_store_yields_the_ordered_boot_steps() {
        let plan = boot_plan(true);
        assert!(plan.stores_ready);
        assert_eq!(
            plan.steps,
            vec![
                BootStep::OpenTabular,
                BootStep::OpenGraph,
                BootStep::LoadOntology,
                BootStep::VerifyAudit,
                BootStep::RecordBoot,
            ]
        );
    }

    #[test]
    fn a_malformed_manifest_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugin.toml");
        std::fs::write(&path, "[plugin]\nname = \"x\"\n").unwrap();
        let err = manifest_at(&path).unwrap_err();
        assert!(matches!(err, MmError::Config(_)), "got {err:?}");
    }
}
