//! The deterministic incremental workspace scan.
//!
//! The walk is sorted, so the record set depends on the tree and not on the
//! filesystem's iteration order. A file belongs to exactly one module — the
//! longest module directory that is a prefix of its path — and a file that
//! belongs to none is reported as an orphan rather than quietly dropped, because
//! "no module owns this" is exactly the fact the gate asks about.
//!
//! **Incrementality.** A file whose sha256 matches the previous scan is not
//! re-parsed and its symbols and references are reused verbatim, so an unchanged
//! tree produces an identical record set. The attribute/import pass still runs
//! for every file: it is what produces cross-crate dependency edges, and a
//! missing edge would be a silently wrong graph rather than a slow one.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use mm_core::codex::module_iri;
use mm_core::MmError;
use walkdir::{DirEntry, WalkDir};

use crate::hash;
use crate::manifest::{self, CargoManifest, PluginManifest};
use crate::model::{
    CapabilityRecord, CodexReport, DepKind, DependencyEdge, FileRecord, ModuleKind, ModuleRecord,
    ReferenceRecord, SymbolKind, SymbolRecord,
};
use crate::{emit, parse, rust_analyze, symbol};

/// Directories walked even when they hold no workspace member.
const WALK_ROOTS: [&str; 3] = ["crates", "modules", "vendor"];

/// What the previous scan recorded, so unchanged files are not re-parsed.
#[derive(Debug, Clone, Default)]
pub struct PreviousState {
    /// Repository-relative path to the sha256 recorded last time.
    pub file_hashes: HashMap<String, String>,
    /// Repository-relative path to the symbols recorded last time.
    pub symbols_by_file: HashMap<String, Vec<SymbolRecord>>,
    /// Repository-relative path to the references recorded last time.
    pub references_by_file: HashMap<String, Vec<ReferenceRecord>>,
}

/// The result of one scan.
#[derive(Debug, Clone)]
pub struct ScanOutput {
    /// Everything the scan found; its `graph_hash` is filled in.
    pub report: CodexReport,
    /// Repository-relative path to sha256, to seed the next scan's [`PreviousState`].
    pub hashes: HashMap<String, String>,
}

impl ScanOutput {
    /// The canonical Turtle for the `/code` graph.
    pub fn turtle(&self) -> String {
        emit::turtle(&self.report)
    }
}

/// Everything the scanner needs to know about a module directory.
#[derive(Debug, Clone)]
struct Facts {
    rel_path: String,
    kind: ModuleKind,
    name: String,
    version: String,
    owned_phase: u8,
    capability: String,
    category: Option<String>,
    crate_name: Option<String>,
    origin: Option<String>,
    copied_from: Option<String>,
    copied_revision: Option<String>,
    copied_license: Option<String>,
    manifest_only: bool,
    declared_uri: Option<String>,
    declared_dependencies: Vec<String>,
    /// The T-Box function names the manifest declares (`system.boot_plan`).
    tbox_functions: Vec<String>,
    /// The Rust handler names those entries resolve to (`boot_plan`).
    tbox_handlers: Vec<String>,
}

/// Files the walk never indexes: generated artifacts and provenance/licence
/// files. They are *about* the code rather than code, and counting them would make
/// `modules/registry.json` and a vendored `COPYING.md` orphans of every module.
fn is_generated_or_provenance(rel_path: &str) -> bool {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    matches!(
        name,
        "metadata.ttl"
            | "registry.json"
            | "COPYING"
            | "COPYING.md"
            | "LICENSE"
            | "LICENSE.md"
            | "LICENSE.txt"
    )
}

/// The capability a crate gets when it declares none: its own crate name.
///
/// Deterministic and derived from data already in the manifest, so no crate has
/// to carry a hand-written capability just to satisfy a shape. `mm-log` becomes
/// `mm:CrateMmLog`.
pub fn derived_capability(crate_name: &str) -> String {
    let mut out = String::from("mm:Crate");
    for part in crate_name.split('-').filter(|p| !p.is_empty()) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(&chars.as_str().to_ascii_lowercase());
        }
    }
    out
}

fn is_ignored_dir(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !(name.starts_with('.') || name == "target" || name == "node_modules")
}

fn is_scanned(rel_path: &str) -> bool {
    parse::is_indexed(rel_path) && !is_generated_or_provenance(rel_path)
}

fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The module that owns a path: the longest module directory that prefixes it.
fn owner_of<'a>(rel: &str, modules: &'a BTreeMap<String, ModuleKind>) -> Option<&'a str> {
    modules
        .keys()
        .filter(|m| rel.starts_with(&format!("{m}/")))
        .max_by_key(|m| m.len())
        .map(String::as_str)
}

/// Expand a `[workspace] members` entry into module directories.
fn expand_member(root: &Path, pattern: &str) -> Result<Vec<String>, MmError> {
    let trimmed = pattern.trim_end_matches('/');
    if let Some(prefix) = trimmed.strip_suffix("/*") {
        let dir = root.join(prefix);
        let mut out = Vec::new();
        if dir.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map_err(|e| MmError::Config(format!("cannot read {}: {e}", dir.display())))?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            entries.sort();
            for entry in entries {
                out.push(rel_path(root, &entry));
            }
        }
        return Ok(out);
    }
    Ok(vec![trimmed.to_string()])
}

/// Read the manifests of one module directory.
fn build_facts(
    rel_path: &str,
    kind: ModuleKind,
    dir: &Path,
    inherited_version: &str,
) -> Result<Facts, MmError> {
    let plugin: Option<PluginManifest> = manifest::plugin_manifest(dir)?;
    let cargo: Option<CargoManifest> = manifest::cargo_manifest(dir)?;
    let meta = cargo.as_ref().and_then(CargoManifest::metamind_metadata);
    let crate_name = cargo
        .as_ref()
        .and_then(|c| c.package.as_ref())
        .map(|p| p.name.clone());
    let declared_uri = plugin.as_ref().map(|p| p.plugin.uri.clone());
    let tbox_declared = plugin
        .as_ref()
        .map(PluginManifest::tbox_function_names)
        .unwrap_or_default();
    let tbox_handlers = plugin
        .as_ref()
        .map(PluginManifest::tbox_handler_names)
        .unwrap_or_default();

    let (name, version, owned_phase, category, capability, origin, copied_from, revision, license) =
        match (&plugin, &cargo) {
            (Some(p), _) => {
                let source = p.source.as_ref();
                (
                    p.plugin.name.clone(),
                    p.plugin.version.clone(),
                    p.metadata.owned_by_phase,
                    Some(p.metadata.category.clone()),
                    p.capability(),
                    source.map(|s| s.origin.clone()),
                    source.and_then(|s| s.copied_from.clone()),
                    None,
                    source.and_then(|s| s.license.clone()),
                )
            }
            (None, Some(c)) => {
                let package = c.package.as_ref().ok_or_else(|| {
                    MmError::Config(format!("{rel_path}: no [package] and no plugin.toml"))
                })?;
                let phase = meta.map(|m| m.owned_by_phase).ok_or_else(|| {
                    MmError::Config(format!(
                        "{rel_path}: {}.Cargo.toml declares no [package.metadata.metamind].owned_by_phase",
                        rel_path
                    ))
                })?;
                let capability = meta
                    .and_then(|m| m.capability.clone())
                    .unwrap_or_else(|| derived_capability(&package.name));
                (
                    package.name.clone(),
                    package
                        .version_string()
                        .unwrap_or_else(|| inherited_version.to_string()),
                    phase,
                    None,
                    if capability.starts_with("mm:") {
                        capability
                    } else {
                        format!("mm:{capability}")
                    },
                    meta.and_then(|m| m.origin.clone()),
                    meta.and_then(|m| m.copied_from.clone()),
                    meta.and_then(|m| m.copied_revision.clone()),
                    meta.and_then(|m| m.license.clone()),
                )
            }
            (None, None) => {
                return Err(MmError::Config(format!(
                    "{rel_path}: no plugin.toml and no Cargo.toml"
                )))
            }
        };

    Ok(Facts {
        rel_path: rel_path.to_string(),
        kind,
        name,
        version,
        owned_phase,
        capability,
        category,
        crate_name,
        origin,
        copied_from,
        copied_revision: revision,
        copied_license: license,
        manifest_only: rel_path.starts_with("vendor/"),
        declared_uri,
        declared_dependencies: cargo
            .as_ref()
            .map(|c| {
                c.dependency_names()
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        tbox_functions: tbox_declared,
        tbox_handlers,
    })
}

/// Discover every module directory in a workspace.
fn module_directories(root: &Path) -> Result<BTreeMap<String, ModuleKind>, MmError> {
    let mut modules: BTreeMap<String, ModuleKind> = BTreeMap::new();

    if let Some(cargo) = manifest::cargo_manifest(root)? {
        if let Some(workspace) = &cargo.workspace {
            for pattern in &workspace.members {
                for rel in expand_member(root, pattern)? {
                    let dir = root.join(&rel);
                    if dir.join("plugin.toml").is_file() {
                        modules.insert(rel, ModuleKind::NexusPlugin);
                    } else if dir.join("Cargo.toml").is_file() {
                        modules.insert(rel, ModuleKind::RustCrate);
                    }
                }
            }
        }
    }

    for walk_root in WALK_ROOTS {
        let dir = root.join(walk_root);
        if !dir.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&dir)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(is_ignored_dir)
        {
            let entry = entry.map_err(|e| MmError::Config(format!("walk failed: {e}")))?;
            if entry.file_type().is_file() && entry.file_name() == "plugin.toml" {
                if let Some(parent) = entry.path().parent() {
                    modules.insert(rel_path(root, parent), ModuleKind::NexusPlugin);
                }
            }
        }
    }

    Ok(modules)
}

/// Scan a workspace.
pub fn scan(root: &Path, previous: &PreviousState, full: bool) -> Result<ScanOutput, MmError> {
    let root = root
        .canonicalize()
        .map_err(|e| MmError::Config(format!("cannot resolve {}: {e}", root.display())))?;

    let inherited_version = manifest::cargo_manifest(&root)?
        .and_then(|c| c.workspace)
        .and_then(|w| w.package)
        .and_then(|p| p.version)
        .unwrap_or_else(|| "0.0.0".to_string());

    let module_dirs = module_directories(&root)?;

    // ---- walk ---------------------------------------------------------------
    let mut walk_roots: BTreeSet<PathBuf> = module_dirs.keys().map(|m| root.join(m)).collect();
    for walk_root in WALK_ROOTS {
        let dir = root.join(walk_root);
        if dir.is_dir() {
            walk_roots.insert(dir);
        }
    }
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    for dir in &walk_roots {
        for entry in WalkDir::new(dir)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(is_ignored_dir)
        {
            let entry = entry.map_err(|e| MmError::Config(format!("walk failed: {e}")))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = rel_path(&root, entry.path());
            if is_scanned(&rel) {
                candidates.insert(rel);
            }
        }
    }

    let mut files_by_module: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut orphan_files: Vec<String> = Vec::new();
    for rel in &candidates {
        match owner_of(rel, &module_dirs) {
            Some(module) => files_by_module
                .entry(module.to_string())
                .or_default()
                .push(rel.clone()),
            None => orphan_files.push(rel.clone()),
        }
    }

    // ---- facts --------------------------------------------------------------
    let mut facts: BTreeMap<String, Facts> = BTreeMap::new();
    for (rel, kind) in &module_dirs {
        facts.insert(
            rel.clone(),
            build_facts(rel, *kind, &root.join(rel), &inherited_version)?,
        );
    }
    // `NamedNode`'s `Display` writes `<iri>`; the records and the emitter want the
    // bare IRI, so `into_string` is used throughout rather than `to_string`.
    let crate_to_module: HashMap<String, String> = facts
        .values()
        .filter_map(|f| {
            f.crate_name
                .as_ref()
                .map(|n| (n.clone(), module_iri(&f.rel_path).into_string()))
        })
        .collect();

    // ---- records ------------------------------------------------------------
    let mut modules = Vec::new();
    let mut files_out = Vec::new();
    let mut symbols_out: Vec<SymbolRecord> = Vec::new();
    let mut references_out: Vec<ReferenceRecord> = Vec::new();
    let mut capabilities = Vec::new();
    let mut dependencies: BTreeSet<DependencyEdge> = BTreeSet::new();
    let mut hashes: HashMap<String, String> = HashMap::new();
    let mut changed_files = 0usize;

    for (rel, facts) in &facts {
        let module_uri = module_iri(rel).into_string();
        let owned = files_by_module.get(rel).cloned().unwrap_or_default();
        let mut file_pairs: Vec<(String, String)> = Vec::new();
        let mut module_symbols: Vec<SymbolRecord> = Vec::new();
        let mut module_refs: Vec<ReferenceRecord> = Vec::new();
        let mut occurrences_by_file: HashMap<String, Vec<(String, usize)>> = HashMap::new();

        for file_rel in &owned {
            let abs = root.join(file_rel);
            let bytes = std::fs::read(&abs)
                .map_err(|e| MmError::Store(format!("cannot read {}: {e}", abs.display())))?;
            let sha = hash::file_hash(&bytes);
            hashes.insert(file_rel.clone(), sha.clone());
            let changed = full || previous.file_hashes.get(file_rel) != Some(&sha);
            if changed {
                changed_files += 1;
            }
            file_pairs.push((file_rel.clone(), sha.clone()));
            files_out.push(FileRecord {
                rel_path: file_rel.clone(),
                module_uri: module_uri.clone(),
                content_hash: sha.clone(),
                swhid: Some(hash::swhid_content(&bytes)),
                language: parse::language_of(file_rel)
                    .unwrap_or("unknown")
                    .to_string(),
                changed,
            });

            if facts.manifest_only || !rust_analyze::is_rust_file(file_rel) {
                continue;
            }

            // The import/attribute pass always runs: it is what yields cross-crate
            // dependency edges, and a missing edge is worse than a slow scan.
            let source = String::from_utf8(bytes.clone())
                .map_err(|e| MmError::Codec(format!("{file_rel}: not UTF-8: {e}")))?;
            let analysis = rust_analyze::analyze(&source, file_rel)?;

            if !changed {
                if let Some(previous_symbols) = previous.symbols_by_file.get(file_rel) {
                    module_symbols.extend(previous_symbols.iter().cloned());
                }
                if let Some(previous_refs) = previous.references_by_file.get(file_rel) {
                    module_refs.extend(previous_refs.iter().cloned());
                }
            } else {
                let scope = parse::file_module_scope(file_rel, rel);
                let parsed = parse::parse_rust_in(&source, &scope)?;
                for item in &parsed.items {
                    if !symbol::is_valid_descriptor(&item.descriptor) {
                        return Err(MmError::Codec(format!(
                            "{file_rel}: descriptor {:?} cannot be carried in a symbol IRI",
                            item.descriptor
                        )));
                    }
                    // A T-Box function is an interface whether it is marked with
                    // `#[tbox_fn]` in source or declared in the manifest's
                    // `[tbox.functions]` table (Phase 1's module uses the latter).
                    let is_tbox = analysis.is_tbox_function(&item.name)
                        || facts
                            .tbox_handlers
                            .iter()
                            .any(|handler| handler == &item.name);
                    let kind = if is_tbox {
                        SymbolKind::Interface
                    } else {
                        item.kind
                    };
                    let text = source
                        .get(item.byte_start..item.byte_end)
                        .unwrap_or_default();
                    module_symbols.push(SymbolRecord {
                        rel_path: file_rel.clone(),
                        module_uri: module_uri.clone(),
                        descriptor: item.descriptor.clone(),
                        kind,
                        is_public: item.is_public,
                        is_test: item.is_test,
                        is_interface: item.is_public
                            || matches!(kind, SymbolKind::Trait | SymbolKind::Interface),
                        byte_start: item.byte_start,
                        byte_end: item.byte_end,
                        content_hash: hash::file_hash(text.as_bytes()),
                    });
                }
                occurrences_by_file.insert(file_rel.clone(), parsed.occurrences);
            }

            for root_name in &analysis.use_roots {
                if let Some(to_uri) = crate_to_module.get(root_name) {
                    if to_uri != &module_uri {
                        dependencies.insert(DependencyEdge {
                            from_uri: module_uri.clone(),
                            to_uri: to_uri.clone(),
                            kind: DepKind::Reference,
                        });
                    }
                }
            }
        }

        // Descriptors are unique within a module by construction; assert it rather
        // than let a duplicate become a SQLite uniqueness failure far from here.
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for s in &module_symbols {
            if !seen.insert(s.descriptor.as_str()) {
                return Err(MmError::Codec(format!(
                    "{}: two definitions share the descriptor {:?}",
                    s.rel_path, s.descriptor
                )));
            }
        }

        // Name-based reference resolution: a name resolves only when exactly one
        // symbol in the module has it, so an ambiguous call is left unresolved
        // instead of being attributed to an arbitrary target.
        let mut name_index: BTreeMap<&str, Vec<&SymbolRecord>> = BTreeMap::new();
        for s in &module_symbols {
            name_index
                .entry(symbol::local_name(&s.descriptor))
                .or_default()
                .push(s);
        }
        for (file_rel, occurrences) in &occurrences_by_file {
            let file_symbols: Vec<&SymbolRecord> = module_symbols
                .iter()
                .filter(|s| &s.rel_path == file_rel)
                .collect();
            for (name, byte_start) in occurrences {
                let Some(candidates) = name_index.get(name.as_str()) else {
                    continue;
                };
                if candidates.len() != 1 {
                    continue;
                }
                let target = candidates[0];
                let enclosing = file_symbols
                    .iter()
                    .filter(|s| s.byte_start <= *byte_start && *byte_start < s.byte_end)
                    .max_by_key(|s| s.byte_start);
                let Some(enclosing) = enclosing else {
                    continue;
                };
                if enclosing.descriptor == target.descriptor
                    && enclosing.rel_path == target.rel_path
                {
                    continue;
                }
                module_refs.push(ReferenceRecord {
                    from_symbol: enclosing.descriptor.clone(),
                    to_symbol: target.descriptor.clone(),
                    file_path: file_rel.clone(),
                    byte_start: *byte_start,
                });
            }
        }
        module_refs.sort_by(|a, b| {
            (&a.file_path, a.byte_start, &a.from_symbol, &a.to_symbol).cmp(&(
                &b.file_path,
                b.byte_start,
                &b.from_symbol,
                &b.to_symbol,
            ))
        });
        module_refs.dedup();

        let test_count = module_symbols.iter().filter(|s| s.is_test).count();
        capabilities.push(CapabilityRecord {
            capability: facts.capability.clone(),
            module_uri: module_uri.clone(),
            test_count,
        });

        for dep in &facts.declared_dependencies {
            if let Some(to_uri) = crate_to_module.get(dep) {
                if to_uri != &module_uri {
                    dependencies.insert(DependencyEdge {
                        from_uri: module_uri.clone(),
                        to_uri: to_uri.clone(),
                        kind: DepKind::Crate,
                    });
                }
            }
        }

        let depends_on: Vec<String> = dependencies
            .iter()
            .filter(|d| d.from_uri == module_uri)
            .map(|d| d.to_uri.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        modules.push(ModuleRecord {
            rel_path: rel.clone(),
            module_uri: module_uri.clone(),
            name: facts.name.clone(),
            version: facts.version.clone(),
            kind: facts.kind,
            category: facts.category.clone(),
            crate_name: facts.crate_name.clone(),
            owned_phase: facts.owned_phase,
            capability: facts.capability.clone(),
            content_hash: hash::module_content_hash(&file_pairs),
            copied_from: facts.copied_from.clone(),
            copied_revision: facts.copied_revision.clone(),
            copied_license: facts.copied_license.clone(),
            origin: facts.origin.clone(),
            manifest_only: facts.manifest_only,
            declared_uri: facts.declared_uri.clone(),
            tbox_functions: facts.tbox_functions.clone(),
            depends_on,
        });

        symbols_out.extend(module_symbols);
        references_out.extend(module_refs);
    }

    symbols_out.sort_by(|a, b| {
        (&a.rel_path, a.byte_start, &a.descriptor).cmp(&(&b.rel_path, b.byte_start, &b.descriptor))
    });
    references_out.sort_by(|a, b| {
        (&a.file_path, a.byte_start, &a.from_symbol, &a.to_symbol).cmp(&(
            &b.file_path,
            b.byte_start,
            &b.from_symbol,
            &b.to_symbol,
        ))
    });
    references_out.dedup();
    orphan_files.sort();

    let mut report = CodexReport {
        modules,
        files: files_out,
        symbols: symbols_out,
        references: references_out,
        dependencies: dependencies.into_iter().collect(),
        capabilities,
        orphan_files,
        graph_hash: String::new(),
        changed_files,
    };
    report.graph_hash = emit::graph_hash(&report)?;
    Ok(ScanOutput { report, hashes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    /// A two-crate workspace: `alpha` depends on `beta` and uses it.
    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "Cargo.toml",
            "[workspace]\nresolver = \"2\"\nmembers = [\"crates/alpha\", \"crates/beta\"]\n\n[workspace.package]\nversion = \"0.1.0\"\n",
        );
        write(
            root,
            "crates/alpha/Cargo.toml",
            "[package]\nname = \"alpha\"\nversion.workspace = true\n\n[dependencies]\nbeta = { path = \"../beta\" }\n\n[package.metadata.metamind]\nowned_by_phase = 2\ncapability = \"mm:Alpha\"\n",
        );
        write(
            root,
            "crates/alpha/src/lib.rs",
            "use beta::helper;\n\npub fn alpha() {\n    helper();\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn it_works() {}\n}\n",
        );
        write(
            root,
            "crates/beta/Cargo.toml",
            "[package]\nname = \"beta\"\nversion.workspace = true\n\n[package.metadata.metamind]\nowned_by_phase = 2\n",
        );
        write(root, "crates/beta/src/lib.rs", "pub fn helper() {}\n");
        dir
    }

    #[test]
    fn a_workspace_becomes_modules_files_and_symbols() {
        let dir = workspace();
        let out = scan(dir.path(), &PreviousState::default(), false).unwrap();
        let report = &out.report;

        let mut names: Vec<&str> = report.modules.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["alpha", "beta"]);

        // Both crates inherit 0.1.0 from [workspace.package].
        assert!(report.modules.iter().all(|m| m.version == "0.1.0"));
        // A crate that declares no capability gets one derived from its name.
        assert_eq!(
            report
                .modules
                .iter()
                .find(|m| m.name == "beta")
                .unwrap()
                .capability,
            "mm:CrateBeta"
        );
        assert!(report.orphan_files.is_empty(), "{:?}", report.orphan_files);

        let descriptors: Vec<&str> = report
            .symbols
            .iter()
            .map(|s| s.descriptor.as_str())
            .collect();
        for expected in ["alpha().", "tests#", "tests#it_works().", "helper()."] {
            assert!(
                descriptors.contains(&expected),
                "missing {expected} in {descriptors:?}"
            );
        }
        assert_eq!(
            report
                .capabilities
                .iter()
                .find(|c| c.capability == "mm:Alpha")
                .unwrap()
                .test_count,
            1
        );
        assert_eq!(
            report
                .capabilities
                .iter()
                .find(|c| c.capability == "mm:CrateBeta")
                .unwrap()
                .test_count,
            0
        );
    }

    #[test]
    fn dependencies_come_from_both_the_manifest_and_the_use_path() {
        let dir = workspace();
        let out = scan(dir.path(), &PreviousState::default(), false).unwrap();
        let alpha = module_iri("crates/alpha").into_string();
        let beta = module_iri("crates/beta").into_string();
        let kinds: BTreeSet<DepKind> = out
            .report
            .dependencies
            .iter()
            .filter(|d| d.from_uri == alpha && d.to_uri == beta)
            .map(|d| d.kind)
            .collect();
        assert!(
            kinds.contains(&DepKind::Crate),
            "the Cargo dependency must be an edge"
        );
        assert!(
            kinds.contains(&DepKind::Reference),
            "the `use` must be an edge"
        );
        // The dependency is visible on the module record too.
        let alpha_record = out
            .report
            .modules
            .iter()
            .find(|m| m.name == "alpha")
            .unwrap();
        assert_eq!(alpha_record.depends_on, vec![beta]);
    }

    #[test]
    fn a_file_no_module_owns_is_reported_as_an_orphan() {
        let dir = workspace();
        write(
            dir.path(),
            "crates/loose.rs",
            "pub fn nobody_owns_me() {}\n",
        );
        let out = scan(dir.path(), &PreviousState::default(), false).unwrap();
        assert_eq!(out.report.orphan_files, vec!["crates/loose.rs"]);
    }

    #[test]
    fn a_rust_file_without_an_owning_phase_is_a_hard_error() {
        let dir = workspace();
        write(
            dir.path(),
            "crates/beta/Cargo.toml",
            "[package]\nname = \"beta\"\nversion = \"0.1.0\"\n",
        );
        let err = scan(dir.path(), &PreviousState::default(), false).unwrap_err();
        assert!(matches!(err, MmError::Config(_)), "got {err:?}");
        assert!(err.to_string().contains("owned_by_phase"), "{err}");
    }

    #[test]
    fn two_scans_of_the_same_tree_agree_exactly() {
        let dir = workspace();
        let first = scan(dir.path(), &PreviousState::default(), false).unwrap();
        let second = scan(dir.path(), &PreviousState::default(), false).unwrap();
        assert_eq!(first.report.graph_hash, second.report.graph_hash);
        assert_eq!(first.turtle(), second.turtle());
        assert_eq!(first.report.files.len(), second.report.files.len());
    }

    #[test]
    fn an_incremental_scan_reuses_unchanged_symbols_verbatim() {
        let dir = workspace();
        let first = scan(dir.path(), &PreviousState::default(), false).unwrap();
        assert!(first.report.changed_files > 0);

        let mut symbols_by_file: HashMap<String, Vec<SymbolRecord>> = HashMap::new();
        for symbol in &first.report.symbols {
            symbols_by_file
                .entry(symbol.rel_path.clone())
                .or_default()
                .push(symbol.clone());
        }
        let previous = PreviousState {
            file_hashes: first.hashes.clone(),
            symbols_by_file,
            references_by_file: HashMap::new(),
        };
        let second = scan(dir.path(), &previous, false).unwrap();

        assert_eq!(second.report.changed_files, 0, "nothing changed on disk");
        assert_eq!(first.report.symbols, second.report.symbols);
        assert_eq!(first.report.graph_hash, second.report.graph_hash);

        // Editing one file changes exactly one file's worth of state.
        write(
            dir.path(),
            "crates/beta/src/lib.rs",
            "pub fn helper() {}\npub fn other() {}\n",
        );
        let third = scan(dir.path(), &previous, false).unwrap();
        assert_eq!(third.report.changed_files, 1);
        assert_ne!(third.report.graph_hash, first.report.graph_hash);
    }

    #[test]
    fn a_full_scan_recomputes_everything() {
        let dir = workspace();
        let previous = PreviousState {
            file_hashes: HashMap::new(),
            symbols_by_file: HashMap::new(),
            references_by_file: HashMap::new(),
        };
        let out = scan(dir.path(), &previous, true).unwrap();
        assert!(out.report.changed_files >= out.report.files.len());
    }

    #[test]
    fn a_plugin_module_is_discovered_from_its_manifest() {
        let dir = workspace();
        write(
            dir.path(),
            "modules/system/thing/plugin.toml",
            "[plugin]\nname = \"thing\"\nuri = \"https://metamind.dev/code/module/modules/system/thing\"\nversion = \"0.2.0\"\n\n[metadata]\ncategory = \"system\"\nowned_by_phase = 2\ncapability = \"mm:Thing\"\n\n[tbox.functions]\n\"system.thing\" = { source = \"handlers::thing\" }\n",
        );
        write(
            dir.path(),
            "modules/system/thing/Cargo.toml",
            "[package]\nname = \"thing\"\nversion = \"0.2.0\"\n",
        );
        write(
            dir.path(),
            "modules/system/thing/src/lib.rs",
            "#[tbox_fn]\npub fn thing() {}\n",
        );
        let out = scan(dir.path(), &PreviousState::default(), false).unwrap();
        let module = out
            .report
            .modules
            .iter()
            .find(|m| m.rel_path == "modules/system/thing")
            .expect("the plugin module is discovered");
        assert_eq!(module.kind, ModuleKind::NexusPlugin);
        assert_eq!(module.capability, "mm:Thing");
        assert_eq!(module.owned_phase, 2);
        // The T-Box function is classified as the interface, not a plain fn.
        let symbol = out
            .report
            .symbols
            .iter()
            .find(|s| s.descriptor == "thing().")
            .expect("the T-Box function is extracted");
        assert_eq!(symbol.kind, SymbolKind::Interface);
        assert!(symbol.is_interface);
    }

    #[test]
    fn vendor_trees_are_recorded_from_their_manifest_alone() {
        let dir = workspace();
        write(
            dir.path(),
            "vendor/acme/Cargo.toml",
            "[package]\nname = \"acme\"\nversion = \"0.1.0\"\n\n[package.metadata.metamind]\nowned_by_phase = 2\ncapability = \"mm:Acme\"\norigin = \"acme\"\ncopied_from = \"https://github.com/acme/acme@abc\"\nlicense = \"MIT\"\n",
        );
        write(
            dir.path(),
            "vendor/acme/src/lib.rs",
            "pub fn copied_thing() {}\n",
        );
        // Register the vendor crate as a member so it becomes a module.
        write(
            dir.path(),
            "Cargo.toml",
            "[workspace]\nresolver = \"2\"\nmembers = [\"crates/alpha\", \"crates/beta\", \"vendor/acme\"]\n\n[workspace.package]\nversion = \"0.1.0\"\n",
        );
        let out = scan(dir.path(), &PreviousState::default(), false).unwrap();
        let module = out
            .report
            .modules
            .iter()
            .find(|m| m.rel_path == "vendor/acme")
            .unwrap();
        assert!(module.manifest_only);
        assert!(module.is_copied(), "anything under vendor/ is copied");
        assert_eq!(module.copied_license.as_deref(), Some("MIT"));
        // Manifest-only means no symbols are claimed for copied code.
        assert!(
            !out.report
                .symbols
                .iter()
                .any(|s| s.rel_path.starts_with("vendor/acme/src")),
            "vendor code is not symbol-extracted"
        );
    }

    #[test]
    fn capability_derivation_is_deterministic() {
        assert_eq!(derived_capability("mm-core"), "mm:CrateMmCore");
        assert_eq!(
            derived_capability("mm-store-sqlite"),
            "mm:CrateMmStoreSqlite"
        );
        assert_eq!(derived_capability("math-types"), "mm:CrateMathTypes");
        assert_eq!(derived_capability("rdf-codec"), "mm:CrateRdfCodec");
    }
}
