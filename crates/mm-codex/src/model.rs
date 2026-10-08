//! The records the scanner produces.
//!
//! Every record is a plain data structure with no store handle in it, so the same
//! records can be emitted as Turtle, mirrored into SQLite, written into
//! `registry.json`/`codex.lock`, and compared by `codex verify` without any of
//! those four paths being able to disagree about the facts.
//!
//! The record set is deliberately closed: the `/code` graph contains exactly the
//! nodes these types describe, and nothing else.

use mm_core::codex::symbol_iri;
use mm_core::NamedNode;
use serde::{Deserialize, Serialize};

/// What kind of thing a module directory is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleKind {
    /// A cargo package: a directory with a `Cargo.toml` carrying `[package]`.
    RustCrate,
    /// A nexus module: a directory with a `plugin.toml` manifest.
    NexusPlugin,
}

impl ModuleKind {
    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            ModuleKind::RustCrate => "rust_crate",
            ModuleKind::NexusPlugin => "nexus_plugin",
        }
    }

    /// The `mmc:` class, emitted *in addition to* `mmc:Module`.
    ///
    /// `sh:targetClass` does no subclass inference, so a node typed only
    /// `mmc:RustCrate` would never be checked. Emitting both the concrete class
    /// and the base class keeps one shape enough for every module kind.
    pub fn class(self) -> &'static str {
        match self {
            ModuleKind::RustCrate => "RustCrate",
            ModuleKind::NexusPlugin => "NexusPlugin",
        }
    }

    /// Parse the stored form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rust_crate" => Some(ModuleKind::RustCrate),
            "nexus_plugin" => Some(ModuleKind::NexusPlugin),
            _ => None,
        }
    }
}

/// What kind of definition a symbol is.
///
/// This is the vocabulary `symbol_index.kind` stores, and it is closed on
/// purpose: `struct`/`enum`/`union`/type alias all land on `Type`, and
/// `const`/`static`/macro on `Const`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    /// A module (`mod foo`).
    Module,
    /// A trait.
    Trait,
    /// A free function or an associated function.
    Fn,
    /// A struct, enum, union, or type alias.
    Type,
    /// A const, static, or macro.
    Const,
    /// A function a `plugin.toml` declares as a T-Box function.
    Interface,
}

impl SymbolKind {
    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Module => "module",
            SymbolKind::Trait => "trait",
            SymbolKind::Fn => "fn",
            SymbolKind::Type => "type",
            SymbolKind::Const => "const",
            SymbolKind::Interface => "interface",
        }
    }
}

/// A module: a crate or a nexus plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleRecord {
    /// Repository-relative path of the module directory.
    pub rel_path: String,
    /// The stable module IRI.
    pub module_uri: String,
    /// Crate name or plugin name.
    pub name: String,
    /// The module's declared version.
    pub version: String,
    /// Crate or plugin.
    pub kind: ModuleKind,
    /// `[metadata].category`, for plugins.
    pub category: Option<String>,
    /// The `[package].name`, for crates.
    pub crate_name: Option<String>,
    /// The phase that owns the module.
    pub owned_phase: u8,
    /// The declared capability (`mm:X`), for plugins; derived (`mm:CrateName`) for crates.
    pub capability: String,
    /// Canonical hash of the module's file set.
    pub content_hash: String,
    /// Where the code was copied from, when it was.
    pub copied_from: Option<String>,
    /// The revision recorded alongside [`Self::copied_from`].
    pub copied_revision: Option<String>,
    /// The license recorded alongside [`Self::copied_from`].
    pub copied_license: Option<String>,
    /// One of `metamind`, `nexus`, `rust_extract`, `rust_symbolic`.
    pub origin: Option<String>,
    /// True for third-party trees recorded from their manifest alone.
    pub manifest_only: bool,
    /// The URI the module's own `plugin.toml` declares, when it is a plugin.
    ///
    /// Kept separate from [`Self::module_uri`] (which is path-derived) so
    /// `codex verify` can compare *declared* URIs: two modules that point at one
    /// stable URI are a defect the path-derived value can never reveal.
    pub declared_uri: Option<String>,
    /// The T-Box function names the manifest declares, sorted.
    pub tbox_functions: Vec<String>,
    /// Module IRIs this module depends on.
    pub depends_on: Vec<String>,
}

impl ModuleRecord {
    /// The module IRI.
    pub fn iri(&self) -> NamedNode {
        mm_core::codex::module_iri(&self.rel_path)
    }

    /// The phase IRI.
    pub fn phase_iri(&self) -> NamedNode {
        mm_core::codex::phase_iri(self.owned_phase)
    }

    /// The capability IRI.
    pub fn capability_iri(&self) -> NamedNode {
        mm_core::codex::capability_iri(&self.capability)
    }

    /// True when the module must carry `mmc:copiedFrom`.
    pub fn is_copied(&self) -> bool {
        self.rel_path.starts_with("vendor/")
            || self.origin.as_deref().is_some_and(|o| o != "metamind")
    }
}

/// A source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// Repository-relative path.
    pub rel_path: String,
    /// The owning module IRI.
    pub module_uri: String,
    /// sha256 of the bytes.
    pub content_hash: String,
    /// `swh:1:cnt:<sha1 of the git blob>`, when it can be computed.
    pub swhid: Option<String>,
    /// `rust`, `toml`, `sql`, `turtle`, ...
    pub language: String,
    /// Whether the bytes differed from what the last scan recorded.
    pub changed: bool,
}

impl FileRecord {
    /// The content-addressed file IRI.
    pub fn iri(&self) -> NamedNode {
        mm_core::codex::file_iri(&self.rel_path, &self.content_hash)
    }
}

/// A definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolRecord {
    /// The file the definition lives in.
    pub rel_path: String,
    /// The owning module IRI.
    pub module_uri: String,
    /// The SCIP-style descriptor, unique within the module.
    pub descriptor: String,
    /// What kind of definition it is.
    pub kind: SymbolKind,
    /// Whether the definition is `pub`.
    pub is_public: bool,
    /// Whether the definition is a test.
    pub is_test: bool,
    /// Whether the definition forms part of the module's public contract.
    pub is_interface: bool,
    /// First byte of the definition in its file.
    pub byte_start: usize,
    /// One past the last byte of the definition.
    pub byte_end: usize,
    /// sha256 of the definition's own source text.
    pub content_hash: String,
}

impl SymbolRecord {
    /// The symbol IRI.
    pub fn iri(&self) -> NamedNode {
        symbol_iri(&self.rel_path, &self.descriptor)
    }
}

/// One use of a symbol defined in the same crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceRecord {
    /// The descriptor of the symbol that uses the name.
    pub from_symbol: String,
    /// The descriptor of the symbol the name resolves to.
    pub to_symbol: String,
    /// The file the use appears in.
    pub file_path: String,
    /// Where in the file the name appears.
    pub byte_start: usize,
}

/// A capability and the module that implements it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRecord {
    /// `mm:X`.
    pub capability: String,
    /// The implementing module IRI.
    pub module_uri: String,
    /// How many test symbols the module has.
    pub test_count: usize,
}

impl CapabilityRecord {
    /// The capability IRI.
    pub fn iri(&self) -> NamedNode {
        mm_core::codex::capability_iri(&self.capability)
    }
}

/// What a dependency edge means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepKind {
    /// Declared in a `Cargo.toml` `[dependencies]` table.
    Crate,
    /// A nexus module depending on another module.
    Module,
    /// A `use` of another module's path from a source file.
    Reference,
}

impl DepKind {
    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            DepKind::Crate => "crate",
            DepKind::Module => "module",
            DepKind::Reference => "reference",
        }
    }
}

/// An edge between two modules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DependencyEdge {
    /// The depending module IRI.
    pub from_uri: String,
    /// The module depended on.
    pub to_uri: String,
    /// Why the edge exists.
    pub kind: DepKind,
}

/// A `vendor/` provenance record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopiedRecord {
    /// The copied module IRI.
    pub module_uri: String,
    /// Repository the code came from.
    pub origin: Option<String>,
    /// The `https://github.com/<origin>@<revision>` string.
    pub copied_from: String,
    /// The revision.
    pub revision: Option<String>,
    /// What the origin's license is, or `unspecified`.
    pub license: String,
}

/// Everything one scan found.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CodexReport {
    /// Every module.
    pub modules: Vec<ModuleRecord>,
    /// Every file belonging to a module.
    pub files: Vec<FileRecord>,
    /// Every definition.
    pub symbols: Vec<SymbolRecord>,
    /// Every intra-crate use of a symbol defined in the same crate.
    pub references: Vec<ReferenceRecord>,
    /// Every module-level dependency edge.
    pub dependencies: Vec<DependencyEdge>,
    /// Every capability.
    pub capabilities: Vec<CapabilityRecord>,
    /// Files under a scanned root that belong to no module.
    pub orphan_files: Vec<String>,
    /// sha256 of the canonical `/code` graph for this report.
    pub graph_hash: String,
    /// How many files changed since the previous scan.
    pub changed_files: usize,
}

impl CodexReport {
    /// Symbols of one module, in file then byte order.
    pub fn symbols_of<'a>(&'a self, module_uri: &'a str) -> impl Iterator<Item = &'a SymbolRecord> {
        self.symbols
            .iter()
            .filter(move |s| s.module_uri == module_uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::iri;

    #[test]
    fn module_kind_round_trips_and_names_its_class() {
        for kind in [ModuleKind::RustCrate, ModuleKind::NexusPlugin] {
            assert_eq!(ModuleKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ModuleKind::RustCrate.class(), "RustCrate");
        assert_eq!(ModuleKind::parse("workspace"), None);
    }

    #[test]
    fn copied_is_decided_by_origin_or_path() {
        let mut m = ModuleRecord {
            rel_path: "crates/mm-core".into(),
            module_uri: iri::module("crates/mm-core").to_string(),
            name: "mm-core".into(),
            version: "0.1.0".into(),
            kind: ModuleKind::RustCrate,
            category: None,
            crate_name: Some("mm-core".into()),
            owned_phase: 1,
            capability: "mm:CrateMmCore".into(),
            content_hash: "0".repeat(64),
            copied_from: None,
            copied_revision: None,
            copied_license: None,
            origin: Some("metamind".into()),
            manifest_only: false,
            declared_uri: None,
            tbox_functions: vec![],
            depends_on: vec![],
        };
        assert!(!m.is_copied());
        m.origin = Some("rust_symbolic".into());
        assert!(m.is_copied());
        m.origin = Some("metamind".into());
        m.rel_path = "vendor/rust_symbolic/rdf-codec".into();
        assert!(
            m.is_copied(),
            "anything under vendor/ is copied by definition"
        );
    }

    #[test]
    fn a_symbol_iri_is_derived_from_its_file_and_descriptor() {
        let s = SymbolRecord {
            rel_path: "crates/mm-codex/src/symbol.rs".into(),
            module_uri: iri::module("crates/mm-codex").to_string(),
            descriptor: "descriptor().".into(),
            kind: SymbolKind::Fn,
            is_public: true,
            is_test: false,
            is_interface: true,
            byte_start: 10,
            byte_end: 20,
            content_hash: "abc".into(),
        };
        assert_eq!(
            s.iri().as_str(),
            "https://metamind.dev/code/symbol/crates/mm-codex/src/symbol.rs#descriptor()."
        );
    }
}
