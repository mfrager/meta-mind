//! The `being` module.
//!
//! Metamind's modular code is nexus-style: a directory with a `plugin.toml`
//! manifest, a stable `uri`, and T-Box functions the monad can address. This
//! module states, in one place, what the persistent self *is* — which surfaces it
//! exposes, what each one reads and writes, and which of the four core invariants
//! the surface has to respect.
//!
//! It is deliberately declarative. The state itself lives in `mm-being`, where the
//! guard can enforce the invariants; duplicating the rules here would create a
//! second place for them to drift. What this module owns is the *contract*: a
//! caller that reads `surfaces()` learns, without opening a store, that
//! `being.affect` writes nothing a fact could be derived from.
//!
//! The manifest is embedded at compile time, so a module whose `plugin.toml` is
//! malformed fails to *build* rather than to load.
#![forbid(unsafe_code)]

use mm_core::MmError;
use serde::{Deserialize, Serialize};

/// The module's embedded manifest.
pub const MANIFEST_TOML: &str = include_str!("../plugin.toml");

/// `being.identity` — the immutable core.
pub const IDENTITY: &str = "being.identity";
/// `being.personality` — the constitution a model elaborates.
pub const PERSONALITY: &str = "being.personality";
/// `being.affect` — the control variable.
pub const AFFECT: &str = "being.affect";
/// `being.beliefs` — the evidence-gated user model.
pub const BELIEFS: &str = "being.beliefs";
/// `being.relationships` — the event-sourced relationship projections.
pub const RELATIONSHIPS: &str = "being.relationships";
/// `being.goals` — goals and commitments.
pub const GOALS: &str = "being.goals";
/// `being.resources` — accounts, ledger, and policies.
pub const RESOURCES: &str = "being.resources";

/// Every T-Box function this module exposes, in a stable order.
pub const SURFACE_FUNCTIONS: [&str; 7] = [
    IDENTITY,
    PERSONALITY,
    AFFECT,
    BELIEFS,
    RELATIONSHIPS,
    GOALS,
    RESOURCES,
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
            function: IDENTITY,
            reads: &["identity", "invariants", "core_blocks"],
            writes: &["identity", "invariants", "core_blocks"],
            invariant: Some("no_history_rewrite"),
            contract: "The identity is created once; a revision appends a version \
                       rather than rewriting the previous one.",
        },
        Surface {
            function: PERSONALITY,
            reads: &["personality_dispositions", "personality_values"],
            writes: &["personality_dispositions"],
            invariant: None,
            contract: "A small constitution of values, dispositions, and constraints. \
                       Behavior is derived from it, never enumerated.",
        },
        Surface {
            function: AFFECT,
            reads: &["affect_state", "affect_impulses"],
            writes: &["affect_state", "affect_impulses"],
            // The headline separation rule: affect moves a control variable and can
            // never be read back as an assertion.
            invariant: None,
            contract: "Affect biases behavior only. affects_internal_state and \
                       affects_reasoning are unsettable and always false, and the \
                       state carries no status-bearing predicate.",
        },
        Surface {
            function: BELIEFS,
            reads: &["user_beliefs"],
            writes: &["user_beliefs"],
            invariant: Some("no_assumption_to_observation"),
            contract: "A belief may only reach a status its evidence supports; \
                       OBSERVED requires an observation record.",
        },
        Surface {
            function: RELATIONSHIPS,
            reads: &["relationships", "relationship_events"],
            writes: &["relationships", "relationship_events"],
            invariant: None,
            contract: "State is a projection of the appended deltas; projecting the \
                       events in order reproduces the row exactly.",
        },
        Surface {
            function: GOALS,
            reads: &[
                "goals",
                "goal_transitions",
                "commitments",
                "commitment_transitions",
            ],
            writes: &[
                "goals",
                "goal_transitions",
                "commitments",
                "commitment_transitions",
            ],
            invariant: Some("no_history_rewrite"),
            contract: "Transitions append; a terminal status is never rewritten, and \
                       the schema's trigger aborts the attempt.",
        },
        Surface {
            function: RESOURCES,
            reads: &["resource_accounts", "resource_ledger", "budget_policies"],
            writes: &["resource_accounts", "resource_ledger"],
            invariant: None,
            contract: "Debits are arithmetic. A hard policy refuses an overspend and \
                       the ledger reconciles exactly with the account.",
        },
    ]
}

/// Parse the embedded manifest.
pub fn manifest() -> Result<PluginManifest, MmError> {
    toml::from_str(MANIFEST_TOML)
        .map_err(|e| MmError::Config(format!("being plugin.toml is invalid: {e}")))
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

/// The handler behind `being.identity`.
pub fn identity() -> Surface {
    surfaces()[0].clone()
}

/// The handler behind `being.personality`.
pub fn personality() -> Surface {
    surfaces()[1].clone()
}

/// The handler behind `being.affect`.
pub fn affect() -> Surface {
    surfaces()[2].clone()
}

/// The handler behind `being.beliefs`.
pub fn beliefs() -> Surface {
    surfaces()[3].clone()
}

/// The handler behind `being.relationships`.
pub fn relationships() -> Surface {
    surfaces()[4].clone()
}

/// The handler behind `being.goals`.
pub fn goals() -> Surface {
    surfaces()[5].clone()
}

/// The handler behind `being.resources`.
pub fn resources() -> Surface {
    surfaces()[6].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_manifest_parses_and_is_well_formed() {
        let m = manifest().unwrap();
        assert_eq!(m.plugin.name, "being");
        assert_eq!(m.plugin.version, "0.1.0");
        assert_eq!(m.metadata.category, "cognition");
        assert_eq!(m.metadata.owned_by_phase, 4);
        assert_eq!(m.metadata.capability, "mm:BeingState");
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
            mm_core::iri::module("cognition/being").as_str()
        );
        assert_eq!(
            m.plugin.uri,
            "https://metamind.dev/code/module/cognition/being"
        );
    }

    #[test]
    fn each_handler_answers_its_own_surface() {
        assert_eq!(identity().function, IDENTITY);
        assert_eq!(personality().function, PERSONALITY);
        assert_eq!(affect().function, AFFECT);
        assert_eq!(beliefs().function, BELIEFS);
        assert_eq!(relationships().function, RELATIONSHIPS);
        assert_eq!(goals().function, GOALS);
        assert_eq!(resources().function, RESOURCES);
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
