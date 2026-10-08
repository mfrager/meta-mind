//! Manifest parsing and validation.
//!
//! Two manifests define a module. A `plugin.toml` declares a nexus plugin — its
//! stable `uri`, its version, the phase that owns it, and the T-Box functions it
//! adds. A `Cargo.toml` declares a crate — its name, its version, and the
//! workspace-internal crates it depends on.
//!
//! A missing required key is a hard error, never a defaulted field: a module
//! whose `uri` is absent has no identity, and inventing one would put a fiction
//! in the code graph that every later query would then trust.

use std::collections::BTreeMap;
use std::path::Path;

use mm_core::MmError;
use serde::Deserialize;

/// A nexus `plugin.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginManifest {
    /// Identity of the plugin.
    pub plugin: PluginSection,
    /// Who owns it and what it provides.
    pub metadata: PluginMetadata,
    /// T-Box functions, if any.
    #[serde(default)]
    pub tbox: TboxSection,
    /// The monad operation the module contributes to.
    #[serde(default)]
    pub monad: Option<MonadSection>,
    /// Source provenance, for copied-in code.
    #[serde(default)]
    pub source: Option<SourceSection>,
    /// Build settings.
    #[serde(default)]
    pub build: Option<BuildSection>,
}

/// Plugin identity. `uri` and `version` are required.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginSection {
    /// Directory or crate name.
    pub name: String,
    /// The stable, path-derived module IRI.
    pub uri: String,
    /// The module version.
    pub version: String,
    /// One-line description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Ownership and capability.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginMetadata {
    /// `system`, `cognition`, `tools`, ...
    pub category: String,
    /// The phase that introduced the module.
    pub owned_by_phase: u8,
    /// The capability the module implements, e.g. `mm:KernelBootstrap`.
    pub capability: String,
}

/// The T-Box functions a module exposes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct TboxSection {
    /// Function name to implementation.
    #[serde(default)]
    pub functions: BTreeMap<String, TboxFunction>,
}

/// One T-Box function.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TboxFunction {
    /// The Rust path of the handler.
    pub source: String,
}

/// The monad operation a module contributes to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MonadSection {
    /// The operation.
    pub operations: MonadOperation,
}

/// One monad operation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MonadOperation {
    /// Operation name.
    pub name: String,
    /// Operation arity.
    pub arity: u32,
}

/// Where copied-in code came from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SourceSection {
    /// `metamind`, `nexus`, `rust_extract`, or `rust_symbolic`.
    pub origin: String,
    /// The required `https://github.com/<origin>@<revision>` string when the
    /// origin is not `metamind`.
    #[serde(default)]
    pub copied_from: Option<String>,
    /// The origin's license, or `unspecified`.
    #[serde(default)]
    pub license: Option<String>,
}

/// Build settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BuildSection {
    /// The Rust edition the module is written in.
    pub rust_edition: String,
}

impl PluginManifest {
    /// Parse and validate a `plugin.toml`.
    pub fn parse(raw: &str, at: &Path) -> Result<Self, MmError> {
        let manifest: PluginManifest = toml::from_str(raw).map_err(|e| {
            MmError::Config(format!("{}: invalid plugin manifest: {e}", at.display()))
        })?;
        manifest.validate(at)?;
        Ok(manifest)
    }

    /// The rules a manifest must satisfy beyond well-formedness.
    fn validate(&self, at: &Path) -> Result<(), MmError> {
        if self.plugin.uri.trim().is_empty() {
            return Err(MmError::Config(format!(
                "{}: [plugin].uri must not be empty",
                at.display()
            )));
        }
        if self.plugin.version.trim().is_empty() {
            return Err(MmError::Config(format!(
                "{}: [plugin].version must not be empty",
                at.display()
            )));
        }
        if !self.plugin.uri.starts_with(mm_core::iri::CODE) {
            return Err(MmError::Config(format!(
                "{}: [plugin].uri must start with {}, got {:?}",
                at.display(),
                mm_core::iri::CODE,
                self.plugin.uri
            )));
        }
        if let Some(source) = &self.source {
            if source.origin != "metamind" && source.copied_from.is_none() {
                return Err(MmError::Config(format!(
                    "{}: [source].copied_from is required when origin is {:?}",
                    at.display(),
                    source.origin
                )));
            }
        }
        Ok(())
    }

    /// The declared capability, normalized to its `mm:X` form.
    pub fn capability(&self) -> String {
        let c = &self.metadata.capability;
        if c.starts_with("mm:") {
            c.clone()
        } else {
            format!("mm:{c}")
        }
    }

    /// The T-Box function names, sorted (`system.boot_plan`).
    pub fn tbox_function_names(&self) -> Vec<String> {
        self.tbox.functions.keys().cloned().collect()
    }

    /// The names of the Rust functions that implement the T-Box functions, sorted.
    ///
    /// `{ source = "handlers::boot_plan" }` implements the Rust function
    /// `boot_plan`, which is the name the symbol table knows it by, so the last
    /// path segment is what links a manifest entry to a definition.
    pub fn tbox_handler_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .tbox
            .functions
            .values()
            .map(|f| {
                f.source
                    .rsplit("::")
                    .next()
                    .unwrap_or(f.source.as_str())
                    .to_string()
            })
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

/// A `Cargo.toml`, reduced to what the scanner needs.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CargoManifest {
    /// The `[package]` table, if this is a package.
    #[serde(default)]
    pub package: Option<CargoPackage>,
    /// The `[workspace]` table, if this is a workspace root.
    #[serde(default)]
    pub workspace: Option<CargoWorkspace>,
    /// `[dependencies]`, in any of TOML's value shapes.
    #[serde(default)]
    pub dependencies: BTreeMap<String, toml::Value>,
    /// `[dev-dependencies]`.
    #[serde(default, rename = "dev-dependencies")]
    pub dev_dependencies: BTreeMap<String, toml::Value>,
    /// `[build-dependencies]`.
    #[serde(default, rename = "build-dependencies")]
    pub build_dependencies: BTreeMap<String, toml::Value>,
}

/// The `[package]` table.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CargoPackage {
    /// The crate name.
    pub name: String,
    /// The crate version.
    ///
    /// Typed loosely on purpose: this workspace writes `version.workspace = true`,
    /// which TOML parses as a *table*, not a string. A `String` here would make
    /// the scanner fail on every one of Metamind's own crates.
    #[serde(default)]
    pub version: Option<toml::Value>,
    /// `[package.metadata]`; only the namespaced `metamind` table is read.
    #[serde(default)]
    pub metadata: Option<CargoMetadata>,
}

/// `[package.metadata]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CargoMetadata {
    /// `[package.metadata.metamind]`.
    #[serde(default)]
    pub metamind: Option<MetamindMetadata>,
}

/// `[package.metadata.metamind]` — who owns a crate and where it came from.
///
/// A plugin declares this in its `plugin.toml`; a plain crate has nowhere else to
/// put it, and the module shapes require an owning phase and a capability for
/// every module. Defaulting either would put a fiction in the code graph.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MetamindMetadata {
    /// The phase that owns the crate.
    pub owned_by_phase: u8,
    /// The capability the crate implements, e.g. `mm:CrateMmCore`.
    #[serde(default)]
    pub capability: Option<String>,
    /// `metamind`, `nexus`, `rust_extract`, or `rust_symbolic`.
    #[serde(default)]
    pub origin: Option<String>,
    /// Where the code was copied from.
    #[serde(default)]
    pub copied_from: Option<String>,
    /// The revision recorded alongside `copied_from`.
    #[serde(default)]
    pub copied_revision: Option<String>,
    /// The origin's license, or `unspecified`.
    #[serde(default)]
    pub license: Option<String>,
}

impl CargoManifest {
    /// The `[package.metadata.metamind]` table, if the crate declares one.
    pub fn metamind_metadata(&self) -> Option<&MetamindMetadata> {
        self.package
            .as_ref()
            .and_then(|p| p.metadata.as_ref())
            .and_then(|m| m.metamind.as_ref())
    }
}

impl CargoPackage {
    /// The literal version, or `None` when it is inherited from the workspace.
    pub fn version_string(&self) -> Option<String> {
        self.version
            .as_ref()
            .and_then(toml::Value::as_str)
            .map(str::to_string)
    }
}

/// The `[workspace]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CargoWorkspace {
    /// Member globs, relative to the workspace root.
    #[serde(default)]
    pub members: Vec<String>,
    /// `[workspace.package]`, which members inherit from.
    #[serde(default)]
    pub package: Option<WorkspacePackage>,
}

/// `[workspace.package]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct WorkspacePackage {
    /// The version members inherit with `version.workspace = true`.
    #[serde(default)]
    pub version: Option<String>,
}

impl CargoManifest {
    /// Parse a `Cargo.toml`.
    pub fn parse(raw: &str, at: &Path) -> Result<Self, MmError> {
        toml::from_str(raw)
            .map_err(|e| MmError::Config(format!("{}: invalid Cargo.toml: {e}", at.display())))
    }

    /// Every dependency name, production, dev, and build.
    pub fn dependency_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .dependencies
            .keys()
            .chain(self.dev_dependencies.keys())
            .chain(self.build_dependencies.keys())
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }
}

/// Read and parse `plugin.toml` in `dir`, if it is there.
pub fn plugin_manifest(dir: &Path) -> Result<Option<PluginManifest>, MmError> {
    let path = dir.join("plugin.toml");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    PluginManifest::parse(&raw, &path).map(Some)
}

/// Read and parse `Cargo.toml` in `dir`, if it is there.
pub fn cargo_manifest(dir: &Path) -> Result<Option<CargoManifest>, MmError> {
    let path = dir.join("Cargo.toml");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    CargoManifest::parse(&raw, &path).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn at() -> PathBuf {
        PathBuf::from("modules/system/kernel-bootstrap/plugin.toml")
    }

    const KERNEL_BOOTSTRAP: &str = r#"
[plugin]
name = "kernel-bootstrap"
uri = "https://metamind.dev/code/module/system/kernel-bootstrap"
version = "0.1.0"

[metadata]
category = "system"
owned_by_phase = 1
capability = "mm:KernelBootstrap"

[tbox.functions]
"system.boot_plan" = { source = "handlers::boot_plan" }

[monad.operations]
name = "system"
arity = 0

[build]
rust_edition = "2021"
"#;

    #[test]
    fn the_real_bootstrap_manifest_parses() {
        let m = PluginManifest::parse(KERNEL_BOOTSTRAP, &at()).unwrap();
        assert_eq!(m.metadata.owned_by_phase, 1);
        assert_eq!(m.capability(), "mm:KernelBootstrap");
        assert_eq!(m.tbox_function_names(), vec!["system.boot_plan"]);
        assert_eq!(m.tbox_handler_names(), vec!["boot_plan"]);
        assert_eq!(m.monad.as_ref().unwrap().operations.name, "system");
    }

    #[test]
    fn a_missing_uri_is_a_hard_error() {
        let raw = "[plugin]\nname = \"x\"\nversion = \"1\"\n[metadata]\ncategory = \"c\"\nowned_by_phase = 1\ncapability = \"mm:X\"\n";
        let err = PluginManifest::parse(raw, &at()).unwrap_err();
        assert!(matches!(err, MmError::Config(_)), "got {err:?}");
        assert!(err.to_string().contains("uri"), "{err}");
    }

    #[test]
    fn a_missing_owning_phase_is_a_hard_error() {
        let raw = "[plugin]\nname = \"x\"\nuri = \"https://metamind.dev/code/module/x\"\nversion = \"1\"\n[metadata]\ncategory = \"c\"\ncapability = \"mm:X\"\n";
        assert!(PluginManifest::parse(raw, &at()).is_err());
    }

    #[test]
    fn an_emptied_uri_is_rejected_even_though_the_key_is_present() {
        let raw = "[plugin]\nname = \"x\"\nuri = \"  \"\nversion = \"1\"\n[metadata]\ncategory = \"c\"\nowned_by_phase = 1\ncapability = \"mm:X\"\n";
        let err = PluginManifest::parse(raw, &at()).unwrap_err();
        assert!(err.to_string().contains("must not be empty"), "{err}");
    }

    #[test]
    fn a_uri_outside_the_code_namespace_is_rejected() {
        let raw = "[plugin]\nname = \"x\"\nuri = \"https://example.com/x\"\nversion = \"1\"\n[metadata]\ncategory = \"c\"\nowned_by_phase = 1\ncapability = \"mm:X\"\n";
        let err = PluginManifest::parse(raw, &at()).unwrap_err();
        assert!(
            err.to_string().contains("[plugin].uri must start with"),
            "{err}"
        );
    }

    #[test]
    fn copied_code_must_declare_its_origin() {
        let base = "[plugin]\nname = \"rdf-codec\"\nuri = \"https://metamind.dev/code/module/vendor/rust_symbolic/rdf-codec\"\nversion = \"0.1.0\"\n[metadata]\ncategory = \"vendor\"\nowned_by_phase = 2\ncapability = \"mm:RdfCodec\"\n";
        // origin != metamind with no copied_from => error
        let raw = format!("{base}\n[source]\norigin = \"rust_symbolic\"\n");
        let err = PluginManifest::parse(&raw, &at()).unwrap_err();
        assert!(err.to_string().contains("copied_from"), "{err}");

        // the same manifest with copied_from is fine
        let ok = format!(
            "{base}\n[source]\norigin = \"rust_symbolic\"\ncopied_from = \"https://github.com/mfrager/blanc-scripts@c03aee5\"\nlicense = \"unspecified\"\n"
        );
        let m = PluginManifest::parse(&ok, &at()).unwrap();
        assert_eq!(m.source.as_ref().unwrap().origin, "rust_symbolic");
    }

    #[test]
    fn a_bare_capability_is_normalized_to_its_prefixed_form() {
        let raw = "[plugin]\nname = \"x\"\nuri = \"https://metamind.dev/code/module/x\"\nversion = \"1\"\n[metadata]\ncategory = \"c\"\nowned_by_phase = 1\ncapability = \"KernelBootstrap\"\n";
        assert_eq!(
            PluginManifest::parse(raw, &at()).unwrap().capability(),
            "mm:KernelBootstrap"
        );
    }

    #[test]
    fn workspace_members_are_read() {
        let raw = "[workspace]\nresolver = \"2\"\nmembers = [\"crates/mm-core\", \"modules/system/kernel-bootstrap\"]\n";
        let m = CargoManifest::parse(raw, Path::new("Cargo.toml")).unwrap();
        assert_eq!(
            m.workspace.unwrap().members,
            vec!["crates/mm-core", "modules/system/kernel-bootstrap"]
        );
        assert!(m.package.is_none());
    }

    #[test]
    fn package_dependencies_are_collected_once_and_sorted() {
        let raw = r#"
[package]
name = "mm-codex"
version = "0.1.0"

[dependencies]
mm-core = { workspace = true }
serde = "1"

[dev-dependencies]
mm-core = { workspace = true }
insta = "1"
"#;
        let m = CargoManifest::parse(raw, Path::new("Cargo.toml")).unwrap();
        assert_eq!(m.package.as_ref().unwrap().name, "mm-codex");
        assert_eq!(m.dependency_names(), vec!["insta", "mm-core", "serde"]);
    }

    /// Metamind's own crates inherit their version, which TOML renders as a table.
    /// Failing to accept that would make the scanner refuse to read the workspace.
    #[test]
    fn a_workspace_inherited_version_is_accepted() {
        let raw =
            "[package]\nname = \"mm-core\"\nversion.workspace = true\nedition.workspace = true\n";
        let m = CargoManifest::parse(raw, Path::new("crates/mm-core/Cargo.toml")).unwrap();
        let p = m.package.unwrap();
        assert_eq!(p.name, "mm-core");
        assert_eq!(
            p.version_string(),
            None,
            "the version lives in the workspace"
        );
    }

    /// The metadata table every crate in this workspace carries, and which the
    /// scanner needs in order to know a crate's owning phase and capability.
    #[test]
    fn crate_owned_by_metadata_is_read() {
        let raw = r#"
[package]
name = "rdf-codec"
version.workspace = true

[package.metadata.metamind]
owned_by_phase = 1
capability = "mm:RdfCodec"
origin = "rust_symbolic"
copied_from = "https://github.com/mfrager/blanc-scripts@c03aee5"
license = "unspecified"

[lints]
workspace = true
"#;
        let m = CargoManifest::parse(raw, Path::new("vendor/rust_symbolic/rdf-codec/Cargo.toml"))
            .unwrap();
        let meta = m.metamind_metadata().expect("metadata is declared");
        assert_eq!(meta.owned_by_phase, 1);
        assert_eq!(meta.capability.as_deref(), Some("mm:RdfCodec"));
        assert_eq!(meta.origin.as_deref(), Some("rust_symbolic"));

        let literal = "[package]\nname = \"mm-x\"\nversion = \"1.2.3\"\n";
        let p = CargoManifest::parse(literal, Path::new("Cargo.toml"))
            .unwrap()
            .package
            .unwrap();
        assert_eq!(p.version_string().as_deref(), Some("1.2.3"));
    }
}
