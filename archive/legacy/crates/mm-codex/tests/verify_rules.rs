//! `codex verify` rules, one test per way a report can be wrong.
//!
//! The pass gate fails a *specific* rule for each negative fixture, so the point of
//! this file is not merely "verify returns something" but "verify returns exactly
//! the rule that this defect is". Each test builds the smallest report that
//! expresses one defect and asserts the rule list, which would catch a verifier that
//! reported everything for everything.

use mm_codex::model::{
    CapabilityRecord, CodexReport, DepKind, DependencyEdge, ModuleKind, ModuleRecord,
};
use mm_codex::{verify, CodexLock, LockedModule};

/// A module with a stable URI and, when asked, a declared one.
fn module(rel: &str, declared_uri: Option<&str>) -> ModuleRecord {
    ModuleRecord {
        rel_path: rel.into(),
        module_uri: mm_core::codex::module_iri(rel).into_string(),
        name: rel.rsplit('/').next().unwrap_or(rel).into(),
        version: "0.1.0".into(),
        kind: if declared_uri.is_some() {
            ModuleKind::NexusPlugin
        } else {
            ModuleKind::RustCrate
        },
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

fn capability(rel: &str, test_count: usize) -> CapabilityRecord {
    CapabilityRecord {
        capability: "mm:X".into(),
        module_uri: mm_core::codex::module_iri(rel).into_string(),
        test_count,
    }
}

fn rules(failures: &[mm_codex::VerifyFailure]) -> Vec<&'static str> {
    failures.iter().map(mm_codex::VerifyFailure::rule).collect()
}

/// A two-crate workspace where `a` depends on `b` and both are tested: the shape
/// every other test perturbs.
fn clean() -> CodexReport {
    CodexReport {
        modules: vec![module("crates/a", None), module("crates/b", None)],
        dependencies: vec![edge("crates/a", "crates/b")],
        capabilities: vec![capability("crates/a", 1), capability("crates/b", 1)],
        ..CodexReport::default()
    }
}

#[test]
fn a_clean_report_has_no_failures() {
    assert!(
        verify(&clean(), None).is_empty(),
        "a well-formed report must verify clean, or every other assertion is noise"
    );
}

#[test]
fn an_orphan_file_is_the_only_failure() {
    let mut report = clean();
    report.orphan_files = vec!["crates/stray.rs".into()];
    let failures = verify(&report, None);
    assert_eq!(rules(&failures), vec!["orphan_file"]);
    assert!(
        failures[0].to_string().contains("crates/stray.rs"),
        "the failure must name the file: {}",
        failures[0]
    );
}

#[test]
fn a_duplicate_declared_uri_is_the_only_failure() {
    let mut report = clean();
    report.modules = vec![
        module(
            "modules/alpha",
            Some("https://metamind.dev/code/module/duplicated"),
        ),
        module(
            "modules/beta",
            Some("https://metamind.dev/code/module/duplicated"),
        ),
    ];
    report.dependencies.clear();
    report.capabilities = vec![
        capability("modules/alpha", 1),
        capability("modules/beta", 1),
    ];
    let failures = verify(&report, None);
    assert_eq!(rules(&failures), vec!["duplicate_uri"]);
    let text = failures[0].to_string();
    assert!(
        text.contains("modules/alpha") && text.contains("modules/beta"),
        "both claimants must be named: {text}"
    );
}

#[test]
fn distinct_declared_uris_are_not_a_duplicate() {
    let mut report = clean();
    report.modules = vec![
        module(
            "modules/alpha",
            Some("https://metamind.dev/code/module/alpha"),
        ),
        module(
            "modules/beta",
            Some("https://metamind.dev/code/module/beta"),
        ),
    ];
    report.dependencies.clear();
    report.capabilities = vec![
        capability("modules/alpha", 1),
        capability("modules/beta", 1),
    ];
    assert!(
        verify(&report, None).is_empty(),
        "one URI per module is fine"
    );
}

#[test]
fn a_two_module_cycle_is_the_only_failure() {
    let mut report = clean();
    report.dependencies = vec![edge("crates/a", "crates/b"), edge("crates/b", "crates/a")];
    let failures = verify(&report, None);
    assert_eq!(rules(&failures), vec!["dependency_cycle"]);
    assert!(
        failures[0].to_string().contains("crates/a")
            && failures[0].to_string().contains("crates/b"),
        "the cycle must name both members: {}",
        failures[0]
    );
}

#[test]
fn a_self_edge_is_not_a_cycle() {
    let mut report = clean();
    // A module that re-exports itself has not created a cycle.
    report.dependencies = vec![edge("crates/a", "crates/a")];
    assert!(
        verify(&report, None).is_empty(),
        "a self-edge must not be reported as a cycle"
    );
}

#[test]
fn a_longer_acyclic_chain_is_not_a_cycle() {
    let mut report = clean();
    report.modules.push(module("crates/c", None));
    report.capabilities.push(capability("crates/c", 1));
    report.dependencies = vec![edge("crates/a", "crates/b"), edge("crates/b", "crates/c")];
    assert!(verify(&report, None).is_empty(), "a chain is not a cycle");
}

#[test]
fn a_capability_without_a_test_is_the_only_failure() {
    let mut report = clean();
    report.capabilities = vec![capability("crates/a", 0), capability("crates/b", 1)];
    let failures = verify(&report, None);
    assert_eq!(rules(&failures), vec!["capability_without_test"]);
    assert!(
        failures[0].to_string().contains("mm:X"),
        "the failure must name the capability: {}",
        failures[0]
    );
}

#[test]
fn a_manifest_only_module_may_have_an_untested_capability() {
    let mut report = clean();
    // A `vendor/` tree is recorded from its manifest alone, so its tests are not
    // something the scanner can see and cannot be required.
    report.modules[0].manifest_only = true;
    report.capabilities = vec![capability("crates/a", 0), capability("crates/b", 1)];
    assert!(
        verify(&report, None).is_empty(),
        "copied-in code promises nothing the scanner can check"
    );
}

fn lock_for(rel: &str, version: &str, content_hash: &str) -> CodexLock {
    CodexLock {
        schema: 1,
        graph_hash: String::new(),
        module: vec![LockedModule {
            uri: mm_core::codex::module_iri(rel).into_string(),
            path: rel.into(),
            version: version.into(),
            content_hash: content_hash.into(),
            capabilities: vec!["mm:X".into()],
        }],
    }
}

#[test]
fn a_changed_hash_at_the_same_version_is_drift() {
    let report = clean();
    let lock = lock_for("crates/a", "0.1.0", "different-from-the-report");
    let failures = verify(&report, Some(&lock));
    assert_eq!(rules(&failures), vec!["version_drift"]);
    assert!(
        failures[0].to_string().contains("version_drift"),
        "{}",
        failures[0]
    );
}

#[test]
fn a_version_bump_is_a_release_not_drift() {
    let report = clean();
    let lock = lock_for("crates/a", "0.2.0", "different-from-the-report");
    assert!(
        verify(&report, Some(&lock)).is_empty(),
        "changing the version is how a change is announced"
    );
}

#[test]
fn a_matching_lock_is_not_drift() {
    let report = clean();
    let lock = CodexLock::from_report(&report);
    assert!(
        verify(&report, Some(&lock)).is_empty(),
        "a lock taken from this very report cannot drift from it"
    );
}

#[test]
fn verification_is_deterministic() {
    let mut report = clean();
    report.orphan_files = vec!["crates/zz.rs".into(), "crates/aa.rs".into()];
    report.modules.push(module(
        "modules/one",
        Some("https://metamind.dev/code/module/dup"),
    ));
    report.modules.push(module(
        "modules/two",
        Some("https://metamind.dev/code/module/dup"),
    ));
    report.capabilities.clear();
    report.dependencies.clear();

    let first = verify(&report, None);
    let second = verify(&report, None);
    assert_eq!(
        first, second,
        "the same report must yield the same failures"
    );
    assert!(!first.is_empty());
    // The failures are ordered by rule then detail, so a diff of two runs is empty.
    let mut sorted = first.clone();
    sorted.sort_by_key(|f| (f.rule(), f.to_string()));
    assert_eq!(first, sorted, "failures must be returned in a stable order");
}
