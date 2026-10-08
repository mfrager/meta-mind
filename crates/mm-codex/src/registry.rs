//! `registry.json` and the SQLite index mirror.
//!
//! The `/code` RDF graph is the authority; both artifacts here are derived views of
//! the same report. `registry.json` is the committed, human-readable module list
//! (deterministic, so a re-scan produces no diff). The SQLite mirror makes
//! `codex meta` churn queries fast and gives the next scan its incremental
//! `PreviousState`; every row is derived and can be dropped and rebuilt.

use std::collections::HashMap;

use mm_core::{MmError, Param, Params, Tabular, UlidFactory};
use serde::Serialize;

use crate::model::{CodexReport, ModuleRecord};

/// The registry format version. Phase 2 rewrites the Phase 1 seed.
pub const REGISTRY_SCHEMA: u32 = 2;

/// The seed's explanatory line, preserved verbatim in the generated file.
const REGISTRY_COMMENT: &str = "Generated, committed. Regenerate with `mm-cli codex scan`; the file is deterministic, so an unchanged tree produces an unchanged file. Only `registry.json` and `codex.lock` are committed under `modules/`; the per-module `metadata.ttl` files are generated and git-ignored.";

#[derive(Serialize)]
struct Registry<'a> {
    _comment: &'a str,
    schema: u32,
    generated_by: &'a str,
    graph_hash: &'a str,
    modules: Vec<RegistryModule<'a>>,
}

#[derive(Serialize)]
struct RegistryModule<'a> {
    name: &'a str,
    uri: &'a str,
    path: &'a str,
    version: &'a str,
    category: Option<&'a str>,
    owned_by_phase: u8,
    capability: &'a str,
    tbox_functions: &'a [String],
}

/// The deterministic `registry.json` text for a report.
pub fn registry_json(report: &CodexReport) -> Result<String, MmError> {
    let mut modules: Vec<&ModuleRecord> = report.modules.iter().collect();
    modules.sort_by(|a, b| a.module_uri.cmp(&b.module_uri));
    let registry = Registry {
        _comment: REGISTRY_COMMENT,
        schema: REGISTRY_SCHEMA,
        generated_by: "mm-cli codex scan",
        graph_hash: &report.graph_hash,
        modules: modules
            .into_iter()
            .map(|m| RegistryModule {
                name: &m.name,
                uri: &m.module_uri,
                path: &m.rel_path,
                version: &m.version,
                category: m.category.as_deref(),
                owned_by_phase: m.owned_phase,
                capability: &m.capability,
                tbox_functions: &m.tbox_functions,
            })
            .collect(),
    };
    let mut text = serde_json::to_string_pretty(&registry)?;
    text.push('\n');
    Ok(text)
}

/// Write `registry.json`, returning `true` when its bytes changed.
pub fn write_registry(path: &std::path::Path, report: &CodexReport) -> Result<bool, MmError> {
    let text = registry_json(report)?;
    let existing = std::fs::read_to_string(path).ok();
    if existing.as_deref() == Some(text.as_str()) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, &text)?;
    Ok(true)
}

fn text(value: &str) -> Param {
    Param::Text(value.to_string())
}

/// Replace the code-metadata index with a report, and append one `codex_run` row.
///
/// The index tables hold derived rows, so they are cleared and rebuilt; the
/// `first_seen_ulid` of a module already present is preserved so the mirror still
/// records when the module first appeared. Every write goes through the kernel's
/// [`Tabular`] boundary, so this module never opens a connection of its own.
pub async fn mirror(
    store: &mm_store_sqlite::SqliteStore,
    report: &CodexReport,
    ids: &UlidFactory,
    started_at: &str,
    violations: usize,
) -> Result<(), MmError> {
    let index: &dyn Tabular = store;

    // Preserve the first-seen identity of modules the index already knows.
    let existing = index
        .query_json(
            "SELECT module_uri, first_seen_ulid FROM module_index",
            Params::new(),
        )
        .await?;
    let mut first_seen: HashMap<String, String> = HashMap::new();
    for row in existing {
        if let (Some(uri), Some(seen)) = (
            row.get("module_uri").and_then(|v| v.as_str()),
            row.get("first_seen_ulid").and_then(|v| v.as_str()),
        ) {
            first_seen.insert(uri.to_string(), seen.to_string());
        }
    }
    let now_ulid = mm_core::ulid_string(&ids.next());

    for table in [
        "module_dep",
        "capability_index",
        "symbol_reference",
        "symbol_index",
        "module_file",
        "module_index",
    ] {
        index
            .execute(&format!("DELETE FROM {table}"), Params::new())
            .await?;
    }

    for module in &report.modules {
        let seen = first_seen
            .get(&module.module_uri)
            .cloned()
            .unwrap_or_else(|| now_ulid.clone());
        index
            .execute(
                "INSERT INTO module_index \
                 (id, module_uri, rel_path, version, category, crate_name, plugin_type, \
                  owned_phase, content_hash, copied_from, first_seen_ulid, last_seen_ulid) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                vec![
                    text(&mm_core::ulid_string(&ids.next())),
                    text(&module.module_uri),
                    text(&module.rel_path),
                    text(&module.version),
                    Param::opt_text(module.category.clone()),
                    Param::opt_text(module.crate_name.clone()),
                    text(module.kind.as_str()),
                    Param::Int(i64::from(module.owned_phase)),
                    text(&module.content_hash),
                    Param::opt_text(module.copied_from.clone()),
                    text(&seen),
                    text(&now_ulid),
                ],
            )
            .await?;
    }

    for file in &report.files {
        index
            .execute(
                "INSERT INTO module_file (id, module_uri, rel_path, content_hash, swhid, lang) \
                 VALUES (?, ?, ?, ?, ?, ?)",
                vec![
                    text(&mm_core::ulid_string(&ids.next())),
                    text(&file.module_uri),
                    text(&file.rel_path),
                    text(&file.content_hash),
                    Param::opt_text(file.swhid.clone()),
                    text(&file.language),
                ],
            )
            .await?;
    }

    for symbol in &report.symbols {
        index
            .execute(
                "INSERT INTO symbol_index \
                 (id, symbol_uri, descriptor, kind, module_uri, file_path, byte_start, byte_end, \
                  is_public, content_hash) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                vec![
                    text(&mm_core::ulid_string(&ids.next())),
                    text(&symbol.iri().into_string()),
                    text(&symbol.descriptor),
                    text(symbol.kind.as_str()),
                    text(&symbol.module_uri),
                    text(&symbol.rel_path),
                    Param::Int(symbol.byte_start as i64),
                    Param::Int(symbol.byte_end as i64),
                    Param::Int(i64::from(symbol.is_public)),
                    text(&symbol.content_hash),
                ],
            )
            .await?;
    }

    for reference in &report.references {
        index
            .execute(
                "INSERT OR IGNORE INTO symbol_reference \
                 (from_symbol, to_symbol, file_path, byte_start) VALUES (?, ?, ?, ?)",
                vec![
                    text(&reference.from_symbol),
                    text(&reference.to_symbol),
                    text(&reference.file_path),
                    Param::Int(reference.byte_start as i64),
                ],
            )
            .await?;
    }

    for capability in &report.capabilities {
        index
            .execute(
                "INSERT INTO capability_index (id, capability, module_uri, test_count) \
                 VALUES (?, ?, ?, ?)",
                vec![
                    text(&mm_core::ulid_string(&ids.next())),
                    text(&capability.capability),
                    text(&capability.module_uri),
                    Param::Int(capability.test_count as i64),
                ],
            )
            .await?;
    }

    for dep in &report.dependencies {
        index
            .execute(
                "INSERT OR IGNORE INTO module_dep (from_uri, to_uri, kind) VALUES (?, ?, ?)",
                vec![
                    text(&dep.from_uri),
                    text(&dep.to_uri),
                    text(dep.kind.as_str()),
                ],
            )
            .await?;
    }

    index
        .execute(
            "INSERT INTO codex_run \
             (id, started_at, modules, files, symbols, violations, graph_hash, changed_files) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                text(&now_ulid),
                text(started_at),
                Param::Int(report.modules.len() as i64),
                Param::Int(report.files.len() as i64),
                Param::Int(report.symbols.len() as i64),
                Param::Int(violations as i64),
                text(&report.graph_hash),
                Param::Int(report.changed_files as i64),
            ],
        )
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn report() -> CodexReport {
        let beta = mm_core::codex::module_iri("crates/beta").into_string();
        let alpha = mm_core::codex::module_iri("crates/alpha").into_string();
        let module = |rel: &str, uri: &str| ModuleRecord {
            rel_path: rel.into(),
            module_uri: uri.into(),
            name: rel.rsplit('/').next().unwrap().into(),
            version: "0.1.0".into(),
            kind: ModuleKind::RustCrate,
            category: None,
            crate_name: Some(rel.into()),
            owned_phase: 2,
            capability: "mm:CrateAlpha".into(),
            content_hash: "aa".repeat(32),
            copied_from: None,
            copied_revision: None,
            copied_license: None,
            origin: Some("metamind".into()),
            manifest_only: false,
            declared_uri: None,
            tbox_functions: vec!["system.boot_plan".into()],
            depends_on: vec![],
        };
        CodexReport {
            modules: vec![module("crates/beta", &beta), module("crates/alpha", &alpha)],
            graph_hash: "gh".into(),
            ..CodexReport::default()
        }
    }

    #[test]
    fn the_registry_is_deterministic_and_sorted_by_uri() {
        let text = registry_json(&report()).unwrap();
        // Field order is the struct's, not serde_json's map order.
        assert!(text.contains("\"_comment\""), "{text}");
        assert!(text.contains("\"schema\": 2"), "{text}");
        assert!(
            text.contains("\"generated_by\": \"mm-cli codex scan\""),
            "{text}"
        );
        assert!(text.ends_with("\n"));
        // Modules are sorted by URI regardless of input order.
        let alpha = text.find("crates/alpha").unwrap();
        let beta = text.find("crates/beta").unwrap();
        assert!(alpha < beta, "modules must be URI-sorted:\n{text}");
        assert!(text.contains("\"tbox_functions\": ["), "{text}");
        // Deterministic: same input => same bytes.
        assert_eq!(text, registry_json(&report()).unwrap());
    }

    #[test]
    fn the_registry_round_trips_as_json() {
        let text = registry_json(&report()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["modules"].as_array().unwrap().len(), 2);
        assert_eq!(value["schema"], 2);
    }

    #[test]
    fn writing_is_a_no_op_when_nothing_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.json");
        assert!(write_registry(&path, &report()).unwrap());
        assert!(!write_registry(&path, &report()).unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            registry_json(&report()).unwrap()
        );
    }

    #[tokio::test]
    async fn mirroring_populates_every_index_table() {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("m.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let ids = UlidFactory::new();
        mirror(&store, &report(), &ids, "2024-01-01T00:00:00Z", 0)
            .await
            .unwrap();
        assert_eq!(store.row_count("module_index").await.unwrap(), 2);
        assert_eq!(store.row_count("codex_run").await.unwrap(), 1);

        // A second scan preserves first_seen and appends a run.
        mirror(&store, &report(), &ids, "2024-01-01T00:00:01Z", 0)
            .await
            .unwrap();
        assert_eq!(store.row_count("module_index").await.unwrap(), 2);
        assert_eq!(store.row_count("codex_run").await.unwrap(), 2);
        store.close().await;
    }

    #[tokio::test]
    async fn mirroring_populates_symbols_capabilities_and_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("m.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let alpha = mm_core::codex::module_iri("crates/alpha").into_string();
        let beta = mm_core::codex::module_iri("crates/beta").into_string();
        let mut report = report();
        report.files.push(FileRecord {
            rel_path: "crates/alpha/src/lib.rs".into(),
            module_uri: alpha.clone(),
            content_hash: "bb".repeat(32),
            swhid: Some("swh:1:cnt:cc".into()),
            language: "rust".into(),
            changed: true,
        });
        report.symbols.push(SymbolRecord {
            rel_path: "crates/alpha/src/lib.rs".into(),
            module_uri: alpha.clone(),
            descriptor: "alpha().".into(),
            kind: SymbolKind::Fn,
            is_public: true,
            is_test: false,
            is_interface: true,
            byte_start: 0,
            byte_end: 20,
            content_hash: "dd".repeat(32),
        });
        report.capabilities.push(CapabilityRecord {
            capability: "mm:CrateAlpha".into(),
            module_uri: alpha.clone(),
            test_count: 0,
        });
        report.dependencies.push(DependencyEdge {
            from_uri: alpha.clone(),
            to_uri: beta.clone(),
            kind: DepKind::Crate,
        });
        mirror(
            &store,
            &report,
            &UlidFactory::new(),
            "2024-01-01T00:00:00Z",
            1,
        )
        .await
        .unwrap();
        assert_eq!(store.row_count("module_file").await.unwrap(), 1);
        assert_eq!(store.row_count("symbol_index").await.unwrap(), 1);
        assert_eq!(store.row_count("capability_index").await.unwrap(), 1);
        assert_eq!(store.row_count("module_dep").await.unwrap(), 1);
        store.close().await;
    }
}
