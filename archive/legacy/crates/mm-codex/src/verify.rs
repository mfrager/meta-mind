//! `codex verify` — the rules a graph alone cannot express.
//!
//! SHACL checks the *shape* of a node: that a module has a path, a version, an
//! owning phase. The rules here are the ones a shape language cannot state — that
//! no file is unowned, that two modules do not claim one URI, that the dependency
//! graph is acyclic, that a capability has a test behind it, and that a module's
//! bytes did not change underneath its version number.
//!
//! Each failure carries a stable [`VerifyFailure::rule`] name. The pass gate
//! asserts *which* rule fired for each negative fixture, so a verifier that merely
//! reported "something is wrong" would not pass.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::lock::CodexLock;
use crate::model::{CodexReport, ModuleKind};

/// One rule a scanned report can break.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerifyFailure {
    /// A file under a scanned root belongs to no module.
    OrphanFile {
        /// The unowned file's repository-relative path.
        rel_path: String,
    },
    /// Two modules declare the same stable URI.
    DuplicateUri {
        /// The URI both modules claim.
        uri: String,
        /// The first claimant's path.
        a: String,
        /// The second claimant's path.
        b: String,
    },
    /// The module dependency graph contains a cycle.
    DependencyCycle {
        /// The modules in the cycle, sorted.
        cycle: Vec<String>,
    },
    /// A module provides a capability with no test backing it.
    CapabilityWithoutTest {
        /// The capability with no test.
        capability: String,
        /// The module that provides it.
        module_uri: String,
    },
    /// A plugin module carries no stable URI.
    MissingStableUri {
        /// The module without a URI.
        module_uri: String,
    },
    /// A module declares no owning phase.
    MissingOwningPhase {
        /// The module without a phase.
        module_uri: String,
    },
    /// A module's content changed while its declared version stayed the same.
    VersionDrift {
        /// The drifted module.
        module_uri: String,
        /// The version the lock recorded.
        recorded: String,
        /// The module's current content hash.
        content_hash: String,
    },
}

impl VerifyFailure {
    /// The stable rule name, used by the gate to assert which rule fired.
    pub fn rule(&self) -> &'static str {
        match self {
            VerifyFailure::OrphanFile { .. } => "orphan_file",
            VerifyFailure::DuplicateUri { .. } => "duplicate_uri",
            VerifyFailure::DependencyCycle { .. } => "dependency_cycle",
            VerifyFailure::CapabilityWithoutTest { .. } => "capability_without_test",
            VerifyFailure::MissingStableUri { .. } => "missing_stable_uri",
            VerifyFailure::MissingOwningPhase { .. } => "missing_owning_phase",
            VerifyFailure::VersionDrift { .. } => "version_drift",
        }
    }
}

impl fmt::Display for VerifyFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerifyFailure::OrphanFile { rel_path } => {
                write!(f, "orphan_file: `{rel_path}` is owned by no module")
            }
            VerifyFailure::DuplicateUri { uri, a, b } => write!(
                f,
                "duplicate_uri: `{uri}` is declared by both `{a}` and `{b}`"
            ),
            VerifyFailure::DependencyCycle { cycle } => {
                write!(f, "dependency_cycle: {}", cycle.join(" -> "))
            }
            VerifyFailure::CapabilityWithoutTest {
                capability,
                module_uri,
            } => write!(
                f,
                "capability_without_test: `{capability}` ({module_uri}) has no test"
            ),
            VerifyFailure::MissingStableUri { module_uri } => {
                write!(f, "missing_stable_uri: `{module_uri}` declares no URI")
            }
            VerifyFailure::MissingOwningPhase { module_uri } => {
                write!(f, "missing_owning_phase: `{module_uri}` declares no owning phase")
            }
            VerifyFailure::VersionDrift {
                module_uri,
                recorded,
                content_hash,
            } => write!(
                f,
                "version_drift: `{module_uri}` changed (content {content_hash}) while still at version {recorded}"
            ),
        }
    }
}

/// Check every rule against a report.
///
/// When a [`CodexLock`] is supplied, `version_drift` is also checked: a module
/// whose content hash differs from the lock while its version is unchanged was
/// edited without a release. The failures are returned in a deterministic order.
pub fn verify(report: &CodexReport, lock: Option<&CodexLock>) -> Vec<VerifyFailure> {
    let mut out = Vec::new();

    for rel_path in &report.orphan_files {
        out.push(VerifyFailure::OrphanFile {
            rel_path: rel_path.clone(),
        });
    }

    // A stable URI is an identity: two modules pointing at one is a defect the
    // path-derived URI can never reveal, which is why the *declared* value is
    // compared. One failure is reported per shared URI.
    let mut by_uri: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for m in &report.modules {
        if let Some(uri) = &m.declared_uri {
            by_uri
                .entry(uri.as_str())
                .or_default()
                .push(m.rel_path.as_str());
        }
    }
    for (uri, mut paths) in by_uri {
        paths.sort_unstable();
        if paths.len() > 1 {
            out.push(VerifyFailure::DuplicateUri {
                uri: uri.to_string(),
                a: paths[0].to_string(),
                b: paths[1].to_string(),
            });
        }
    }

    for cycle in dependency_cycles(report) {
        out.push(VerifyFailure::DependencyCycle { cycle });
    }

    // A capability is a promise that something is tested. A manifest-only module
    // (a `vendor/` tree recorded from its manifest alone) promises nothing we can
    // check, so it is exempt rather than reported.
    for capability in &report.capabilities {
        let manifest_only = report
            .modules
            .iter()
            .find(|m| m.module_uri == capability.module_uri)
            .is_some_and(|m| m.manifest_only);
        if capability.test_count == 0 && !manifest_only {
            out.push(VerifyFailure::CapabilityWithoutTest {
                capability: capability.capability.clone(),
                module_uri: capability.module_uri.clone(),
            });
        }
    }

    for m in &report.modules {
        let declared = m.declared_uri.as_deref().unwrap_or("").trim();
        if m.kind == ModuleKind::NexusPlugin && declared.is_empty() {
            out.push(VerifyFailure::MissingStableUri {
                module_uri: m.module_uri.clone(),
            });
        }
        if m.owned_phase == 0 {
            out.push(VerifyFailure::MissingOwningPhase {
                module_uri: m.module_uri.clone(),
            });
        }
    }

    if let Some(lock) = lock {
        for locked in &lock.module {
            if let Some(m) = report.modules.iter().find(|m| m.module_uri == locked.uri) {
                if m.content_hash != locked.content_hash && m.version == locked.version {
                    out.push(VerifyFailure::VersionDrift {
                        module_uri: m.module_uri.clone(),
                        recorded: locked.version.clone(),
                        content_hash: m.content_hash.clone(),
                    });
                }
            }
        }
    }

    out.sort_by(|a, b| (a.rule(), a.to_string()).cmp(&(b.rule(), b.to_string())));
    out
}

/// Every cycle in the module dependency graph, each as a sorted node list.
///
/// A module that merely depends on itself is ignored: re-exporting one's own path
/// is not a cycle. Cycles are the strongly connected components with more than one
/// node, so each distinct cycle is reported exactly once no matter where the walk
/// entered it.
pub fn dependency_cycles(report: &CodexReport) -> Vec<Vec<String>> {
    let nodes: BTreeSet<String> = report
        .modules
        .iter()
        .map(|m| m.module_uri.clone())
        .collect();
    let mut adj: BTreeMap<String, Vec<String>> =
        nodes.iter().map(|n| (n.clone(), Vec::new())).collect();
    for edge in &report.dependencies {
        if edge.from_uri == edge.to_uri {
            continue;
        }
        if !nodes.contains(&edge.from_uri) || !nodes.contains(&edge.to_uri) {
            continue;
        }
        if let Some(list) = adj.get_mut(&edge.from_uri) {
            list.push(edge.to_uri.clone());
        }
    }
    for list in adj.values_mut() {
        list.sort();
        list.dedup();
    }

    let mut index = 0usize;
    let mut indices: BTreeMap<String, usize> = BTreeMap::new();
    let mut low: BTreeMap<String, usize> = BTreeMap::new();
    let mut on_stack: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();
    let mut sccs: Vec<Vec<String>> = Vec::new();

    for v in &nodes {
        if !indices.contains_key(v) {
            strongconnect(
                v,
                &adj,
                &mut index,
                &mut indices,
                &mut low,
                &mut on_stack,
                &mut stack,
                &mut sccs,
            );
        }
    }

    sccs.sort();
    sccs
}

/// Tarjan's algorithm, iterative in spirit but expressed recursively: the module
/// graph is small and bounded, and recursion keeps the bookkeeping legible.
#[allow(clippy::too_many_arguments)]
fn strongconnect(
    v: &str,
    adj: &BTreeMap<String, Vec<String>>,
    index: &mut usize,
    indices: &mut BTreeMap<String, usize>,
    low: &mut BTreeMap<String, usize>,
    on_stack: &mut BTreeSet<String>,
    stack: &mut Vec<String>,
    sccs: &mut Vec<Vec<String>>,
) {
    indices.insert(v.to_string(), *index);
    low.insert(v.to_string(), *index);
    *index += 1;
    stack.push(v.to_string());
    on_stack.insert(v.to_string());

    for w in adj.get(v).map(Vec::as_slice).unwrap_or(&[]) {
        if !indices.contains_key(w) {
            strongconnect(w, adj, index, indices, low, on_stack, stack, sccs);
            let candidate = low[v].min(low[w]);
            low.insert(v.to_string(), candidate);
        } else if on_stack.contains(w) {
            let candidate = low[v].min(indices[w]);
            low.insert(v.to_string(), candidate);
        }
    }

    if low[v] == indices[v] {
        let mut component = Vec::new();
        while let Some(w) = stack.pop() {
            on_stack.remove(&w);
            component.push(w.clone());
            if w == v {
                break;
            }
        }
        if component.len() > 1 {
            component.sort();
            sccs.push(component);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn module(rel_path: &str, declared_uri: Option<&str>) -> ModuleRecord {
        ModuleRecord {
            rel_path: rel_path.into(),
            module_uri: mm_core::codex::module_iri(rel_path).into_string(),
            name: rel_path.rsplit('/').next().unwrap().into(),
            version: "0.1.0".into(),
            kind: if declared_uri.is_some() {
                ModuleKind::NexusPlugin
            } else {
                ModuleKind::RustCrate
            },
            category: None,
            crate_name: Some(rel_path.into()),
            owned_phase: 2,
            capability: "mm:X".into(),
            content_hash: "aa".repeat(32),
            copied_from: None,
            copied_revision: None,
            copied_license: None,
            origin: Some("metamind".into()),
            manifest_only: false,
            declared_uri: declared_uri.map(str::to_string),
            tbox_functions: vec![],
            depends_on: vec![],
        }
    }

    fn edge(from: &str, to: &str) -> DependencyEdge {
        DependencyEdge {
            from_uri: mm_core::codex::module_iri(from).into_string(),
            to_uri: mm_core::codex::module_iri(to).into_string(),
            kind: DepKind::Crate,
        }
    }

    fn capability(rel_path: &str, test_count: usize) -> CapabilityRecord {
        CapabilityRecord {
            capability: "mm:X".into(),
            module_uri: mm_core::codex::module_iri(rel_path).into_string(),
            test_count,
        }
    }

    fn rules(failures: &[VerifyFailure]) -> Vec<&'static str> {
        failures.iter().map(VerifyFailure::rule).collect()
    }

    #[test]
    fn a_clean_report_has_no_failures() {
        let report = CodexReport {
            modules: vec![module("crates/a", None), module("crates/b", None)],
            dependencies: vec![edge("crates/a", "crates/b")],
            capabilities: vec![capability("crates/a", 1), capability("crates/b", 1)],
            ..CodexReport::default()
        };
        assert!(verify(&report, None).is_empty());
    }

    #[test]
    fn an_orphan_file_is_reported_by_name() {
        let report = CodexReport {
            orphan_files: vec!["crates/stray.rs".into()],
            ..CodexReport::default()
        };
        let failures = verify(&report, None);
        assert_eq!(rules(&failures), vec!["orphan_file"]);
        assert!(failures[0].to_string().contains("crates/stray.rs"));
    }

    #[test]
    fn two_modules_sharing_a_declared_uri_are_a_failure() {
        let report = CodexReport {
            modules: vec![
                module(
                    "modules/alpha",
                    Some("https://metamind.dev/code/module/duplicated"),
                ),
                module(
                    "modules/beta",
                    Some("https://metamind.dev/code/module/duplicated"),
                ),
            ],
            capabilities: vec![
                capability("modules/alpha", 1),
                capability("modules/beta", 1),
            ],
            ..CodexReport::default()
        };
        let failures = verify(&report, None);
        assert_eq!(rules(&failures), vec!["duplicate_uri"]);
        let text = failures[0].to_string();
        assert!(
            text.contains("modules/alpha") && text.contains("modules/beta"),
            "{text}"
        );
    }

    #[test]
    fn a_distinct_declared_uri_per_module_is_fine() {
        let report = CodexReport {
            modules: vec![
                module(
                    "modules/alpha",
                    Some("https://metamind.dev/code/module/alpha"),
                ),
                module(
                    "modules/beta",
                    Some("https://metamind.dev/code/module/beta"),
                ),
            ],
            capabilities: vec![
                capability("modules/alpha", 1),
                capability("modules/beta", 1),
            ],
            ..CodexReport::default()
        };
        assert!(verify(&report, None).is_empty());
    }

    #[test]
    fn a_two_module_cycle_is_detected_once() {
        let report = CodexReport {
            modules: vec![module("crates/a", None), module("crates/b", None)],
            dependencies: vec![edge("crates/a", "crates/b"), edge("crates/b", "crates/a")],
            capabilities: vec![capability("crates/a", 1), capability("crates/b", 1)],
            ..CodexReport::default()
        };
        let cycles = dependency_cycles(&report);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), 2);
        assert_eq!(rules(&verify(&report, None)), vec!["dependency_cycle"]);
    }

    #[test]
    fn a_self_edge_is_not_a_cycle() {
        let report = CodexReport {
            modules: vec![module("crates/a", None)],
            dependencies: vec![edge("crates/a", "crates/a")],
            capabilities: vec![capability("crates/a", 1)],
            ..CodexReport::default()
        };
        assert!(dependency_cycles(&report).is_empty());
        assert!(verify(&report, None).is_empty());
    }

    #[test]
    fn an_acyclic_chain_has_no_cycle() {
        let report = CodexReport {
            modules: vec![
                module("crates/a", None),
                module("crates/b", None),
                module("crates/c", None),
            ],
            dependencies: vec![edge("crates/a", "crates/b"), edge("crates/b", "crates/c")],
            capabilities: vec![
                capability("crates/a", 1),
                capability("crates/b", 1),
                capability("crates/c", 1),
            ],
            ..CodexReport::default()
        };
        assert!(dependency_cycles(&report).is_empty());
    }

    #[test]
    fn a_capability_without_a_test_is_reported() {
        let report = CodexReport {
            modules: vec![module("crates/a", None)],
            capabilities: vec![capability("crates/a", 0)],
            ..CodexReport::default()
        };
        let failures = verify(&report, None);
        assert_eq!(rules(&failures), vec!["capability_without_test"]);
        assert!(failures[0].to_string().contains("mm:X"));
    }

    #[test]
    fn a_manifest_only_module_may_have_an_untested_capability() {
        let mut m = module("vendor/acme", None);
        m.manifest_only = true;
        let report = CodexReport {
            modules: vec![m],
            capabilities: vec![capability("vendor/acme", 0)],
            ..CodexReport::default()
        };
        assert!(verify(&report, None).is_empty());
    }

    #[test]
    fn version_drift_needs_a_lock_and_a_same_version_change() {
        let report = CodexReport {
            modules: vec![module("crates/a", None)],
            capabilities: vec![capability("crates/a", 1)],
            ..CodexReport::default()
        };
        let lock = crate::lock::CodexLock {
            schema: 1,
            graph_hash: String::new(),
            module: vec![crate::lock::LockedModule {
                uri: mm_core::codex::module_iri("crates/a").into_string(),
                path: "crates/a".into(),
                version: "0.1.0".into(),
                content_hash: "different".into(),
                capabilities: vec!["mm:X".into()],
            }],
        };
        assert_eq!(rules(&verify(&report, Some(&lock))), vec!["version_drift"]);
        // No lock => no drift checking.
        assert!(verify(&report, None).is_empty());

        // A version bump is a release, not drift.
        let mut bumped = lock.clone();
        bumped.module[0].version = "0.2.0".into();
        assert!(verify(&report, Some(&bumped)).is_empty());
    }
}
