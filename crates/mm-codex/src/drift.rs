//! Structural drift: a module's public interface changed under a stable version.
//!
//! `version_drift` (in [`crate::verify`]) catches *any* byte change while the
//! version is unchanged. That is the right alarm for the gate but a blunt one for
//! review: reformatting, a new test, or a private helper should not read the same as
//! a changed public signature. This module answers the finer question — which
//! definitions in a module's contract appeared or disappeared — by comparing the
//! current report against the interfaces the previous scan recorded in SQLite.

use std::collections::{BTreeMap, BTreeSet};

use mm_core::{MmError, Params, Tabular};

use crate::model::{CodexReport, SymbolKind};

/// A module's contract changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceDrift {
    /// The drifted module.
    pub module_uri: String,
    /// Descriptors that are new.
    pub added: Vec<String>,
    /// Descriptors that are gone.
    pub removed: Vec<String>,
}

impl InterfaceDrift {
    /// True when nothing was added or removed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// The public interface of every module in a report.
///
/// The contract is the public definitions plus every trait and T-Box interface,
/// keyed by module URI. Descriptors (not IRIs) are compared so that moving a
/// definition within its file is not drift.
pub fn interfaces(report: &CodexReport) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for symbol in &report.symbols {
        if symbol.is_test {
            continue;
        }
        let in_contract =
            symbol.is_public || matches!(symbol.kind, SymbolKind::Trait | SymbolKind::Interface);
        if in_contract {
            out.entry(symbol.module_uri.clone())
                .or_default()
                .insert(symbol.descriptor.clone());
        }
    }
    out
}

/// Compare a previously recorded interface set against a report.
pub fn detect(
    previous: &BTreeMap<String, BTreeSet<String>>,
    report: &CodexReport,
) -> Vec<InterfaceDrift> {
    let current = interfaces(report);
    let mut out = Vec::new();

    let mut modules: BTreeSet<&String> = previous.keys().collect();
    modules.extend(current.keys());
    for module in modules {
        let empty = BTreeSet::new();
        let before = previous.get(module).unwrap_or(&empty);
        let after = current.get(module).unwrap_or(&empty);
        let added: Vec<String> = after.difference(before).cloned().collect();
        let removed: Vec<String> = before.difference(after).cloned().collect();
        if !added.is_empty() || !removed.is_empty() {
            out.push(InterfaceDrift {
                module_uri: module.clone(),
                added,
                removed,
            });
        }
    }
    out.sort_by(|a, b| a.module_uri.cmp(&b.module_uri));
    out
}

/// The interface set the last scan recorded, read from `symbol_index`.
///
/// An empty table means "no previous scan", which yields an empty map and therefore
/// no drift — a first scan reports nothing as changed.
pub async fn previous_interfaces(
    sqlite: &mm_store_sqlite::SqliteStore,
) -> Result<BTreeMap<String, BTreeSet<String>>, MmError> {
    let index: &dyn Tabular = sqlite;
    let rows = index
        .query_json(
            "SELECT module_uri, descriptor, kind, is_public FROM symbol_index",
            Params::new(),
        )
        .await?;
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        let get = |key: &str| row.get(key).and_then(|v| v.as_str()).unwrap_or_default();
        let public = row
            .get("is_public")
            .and_then(|v| v.as_i64())
            .is_some_and(|n| n != 0);
        let kind = get("kind");
        if public || kind == "trait" || kind == "interface" {
            out.entry(get("module_uri").to_string())
                .or_default()
                .insert(get("descriptor").to_string());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ModuleKind, ModuleRecord, SymbolRecord};

    fn symbol(module: &str, descriptor: &str, kind: SymbolKind) -> SymbolRecord {
        SymbolRecord {
            rel_path: format!("{module}/src/lib.rs"),
            module_uri: module.to_string(),
            descriptor: descriptor.to_string(),
            kind,
            is_public: true,
            is_test: false,
            is_interface: true,
            byte_start: 0,
            byte_end: 1,
            content_hash: "aa".into(),
        }
    }

    fn module(rel: &str) -> ModuleRecord {
        ModuleRecord {
            rel_path: rel.into(),
            module_uri: mm_core::codex::module_iri(rel).into_string(),
            name: rel.into(),
            version: "0.1.0".into(),
            kind: ModuleKind::RustCrate,
            category: None,
            crate_name: Some(rel.into()),
            owned_phase: 2,
            capability: "mm:X".into(),
            content_hash: "aa".repeat(32),
            copied_from: None,
            copied_revision: None,
            copied_license: None,
            origin: Some("metamind".into()),
            manifest_only: false,
            declared_uri: None,
            tbox_functions: vec![],
            depends_on: vec![],
        }
    }

    #[test]
    fn a_new_public_definition_is_drift() {
        let module_uri = mm_core::codex::module_iri("crates/a").into_string();
        let before = BTreeMap::from([(module_uri.clone(), BTreeSet::from(["a".to_string()]))]);
        let report = CodexReport {
            modules: vec![module("crates/a")],
            symbols: vec![
                symbol(&module_uri, "a", SymbolKind::Fn),
                symbol(&module_uri, "b", SymbolKind::Fn),
            ],
            ..CodexReport::default()
        };
        let drift = detect(&before, &report);
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].added, vec!["b"]);
        assert!(drift[0].removed.is_empty());
    }

    #[test]
    fn a_removed_definition_is_drift() {
        let module_uri = mm_core::codex::module_iri("crates/a").into_string();
        let before = BTreeMap::from([(
            module_uri.clone(),
            BTreeSet::from(["a".to_string(), "b".to_string()]),
        )]);
        let report = CodexReport {
            modules: vec![module("crates/a")],
            symbols: vec![symbol(&module_uri, "a", SymbolKind::Fn)],
            ..CodexReport::default()
        };
        let drift = detect(&before, &report);
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].removed, vec!["b"]);
    }

    #[test]
    fn an_unchanged_interface_does_not_drift() {
        let module_uri = mm_core::codex::module_iri("crates/a").into_string();
        let report = CodexReport {
            modules: vec![module("crates/a")],
            symbols: vec![symbol(&module_uri, "a", SymbolKind::Fn)],
            ..CodexReport::default()
        };
        let before = interfaces(&report);
        assert!(detect(&before, &report).is_empty());
    }

    #[test]
    fn a_private_change_is_not_interface_drift() {
        let module_uri = mm_core::codex::module_iri("crates/a").into_string();
        let mut private = symbol(&module_uri, "helper", SymbolKind::Fn);
        private.is_public = false;
        private.is_interface = false;
        let report = CodexReport {
            modules: vec![module("crates/a")],
            symbols: vec![symbol(&module_uri, "a", SymbolKind::Fn), private],
            ..CodexReport::default()
        };
        let before = BTreeMap::from([(module_uri, BTreeSet::from(["a".to_string()]))]);
        assert!(
            detect(&before, &report).is_empty(),
            "a private fn is not the contract"
        );
    }

    #[tokio::test]
    async fn previous_interfaces_reads_the_symbol_index() {
        let dir = tempfile::tempdir().unwrap();
        let store = mm_store_sqlite::SqliteStore::open(&dir.path().join("m.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        assert!(previous_interfaces(&store).await.unwrap().is_empty());
        store.close().await;
    }
}
