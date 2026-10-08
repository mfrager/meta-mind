//! Fixture-driven scanner tests.
//!
//! Each directory under `bench/codex/fixtures/` is a self-contained mini-workspace
//! with **exactly one planted defect**, so a rule that fires for the wrong reason
//! is caught rather than merely a rule that fires. `good/` proves the happy path
//! really is clean, and each `bad_*` fixture pins the *specific* rule the scanner
//! and verifier must report for it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mm_codex::model::{CodexReport, DepKind, ModuleKind};
use mm_codex::{verify, ModuleRecord, PreviousState};

/// The fixtures live next to the workspace root, not inside the crate; an
/// integration test's working directory is the crate root, so the path is resolved
/// from `CARGO_MANIFEST_DIR` rather than from the process CWD.
fn fixture_dir(name: &str) -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../bench/codex/fixtures"
    ))
    .join(name)
}

fn scan_fixture(name: &str) -> CodexReport {
    let dir = fixture_dir(name);
    assert!(
        dir.is_dir(),
        "fixture {name} is missing at {}",
        dir.display()
    );
    mm_codex::scan(&dir, &PreviousState::default(), true)
        .unwrap_or_else(|e| panic!("scanning {name} failed: {e}"))
        .report
}

/// The set of rule names a report breaks.
fn rules(report: &CodexReport) -> BTreeSet<&'static str> {
    verify(report, None).iter().map(|f| f.rule()).collect()
}

/// Assert a report breaks exactly the planted rule.
fn assert_only_rule(report: &CodexReport, expected: &str) {
    let failures = verify(report, None);
    let found: Vec<&str> = failures.iter().map(|f| f.rule()).collect();
    assert_eq!(
        found,
        vec![expected],
        "expected exactly [{expected}], got {failures:?}"
    );
}

fn module<'a>(report: &'a CodexReport, rel_path: &str) -> &'a ModuleRecord {
    report
        .modules
        .iter()
        .find(|m| m.rel_path == rel_path)
        .unwrap_or_else(|| panic!("no module at {rel_path}"))
}

#[test]
fn the_good_fixture_scans_clean() {
    let report = scan_fixture("good");

    let mut names: Vec<&str> = report.modules.iter().map(|m| m.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["alpha", "beta"]);
    assert!(
        report.orphan_files.is_empty(),
        "the good fixture owns every file: {:?}",
        report.orphan_files
    );
    assert!(rules(&report).is_empty(), "{:?}", verify(&report, None));

    // `alpha` declares a Cargo dependency on `beta` and uses it, so the edge is
    // present and points the one way.
    let alpha = module(&report, "crates/alpha");
    let beta = module(&report, "crates/beta");
    let kinds: BTreeSet<DepKind> = report
        .dependencies
        .iter()
        .filter(|d| d.from_uri == alpha.module_uri && d.to_uri == beta.module_uri)
        .map(|d| d.kind)
        .collect();
    assert!(
        kinds.contains(&DepKind::Crate),
        "the manifest edge must exist"
    );
    assert!(
        !report
            .dependencies
            .iter()
            .any(|d| d.from_uri == beta.module_uri),
        "beta depends on nothing"
    );
}

#[test]
fn the_orphan_file_fixture_reports_its_orphan() {
    let report = scan_fixture("bad_orphan_file");
    assert_eq!(report.orphan_files, vec!["crates/stray.rs"]);
    assert_only_rule(&report, "orphan_file");
}

#[test]
fn the_duplicate_uri_fixture_reports_the_shared_uri() {
    let report = scan_fixture("bad_duplicate_uri");

    // Both modules are nexus plugins, and each keeps its own path-derived identity.
    assert_eq!(
        module(&report, "modules/alpha").kind,
        ModuleKind::NexusPlugin
    );
    assert_eq!(
        module(&report, "modules/beta").kind,
        ModuleKind::NexusPlugin
    );

    assert_only_rule(&report, "duplicate_uri");
    // The declared URI is what collides; the path-derived ones do not.
    let failure = &verify(&report, None)[0];
    assert!(
        failure.to_string().contains("module/duplicated"),
        "{failure}"
    );
}

#[test]
fn the_dependency_cycle_fixture_reports_a_cycle() {
    let report = scan_fixture("bad_dependency_cycle");
    assert_only_rule(&report, "dependency_cycle");
    // The cycle names both ends, whichever edge kind the scanner derived.
    let failure = &verify(&report, None)[0];
    assert!(failure.to_string().contains("crates/alpha"), "{failure}");
    assert!(failure.to_string().contains("crates/beta"), "{failure}");
}

#[test]
fn the_untested_capability_fixture_reports_the_capability() {
    let report = scan_fixture("bad_capability_without_test");
    assert_only_rule(&report, "capability_without_test");
    let capability = report
        .capabilities
        .iter()
        .find(|c| c.capability == "mm:Untested")
        .expect("the fixture declares mm:Untested");
    assert_eq!(capability.test_count, 0);
}

#[test]
fn a_nexus_modules_identity_drops_the_modules_container() {
    let report = scan_fixture("bad_duplicate_uri");
    // `modules/` is a container, not part of the module's identity.
    assert_eq!(
        module(&report, "modules/alpha").module_uri,
        "https://metamind.dev/code/module/alpha"
    );
    // A file, by contrast, keeps its whole repository path.
    let file = report
        .files
        .iter()
        .find(|f| f.rel_path == "modules/alpha/src/lib.rs")
        .expect("the plugin's source file is indexed");
    assert_eq!(
        file.iri().into_string(),
        format!(
            "https://metamind.dev/code/file/modules/alpha/src/lib.rs@{}",
            file.content_hash
        )
    );
}

#[test]
fn vendor_code_is_recorded_from_its_manifest_alone() {
    // The real tree vendors reference crates; they must not be symbol-extracted.
    // This is asserted against the workspace itself so the exemption the verifier
    // relies on is real rather than assumed.
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let report = mm_codex::scan(root, &PreviousState::default(), true)
        .expect("the workspace scans")
        .report;
    let vendored: Vec<&ModuleRecord> = report.modules.iter().filter(|m| m.manifest_only).collect();
    assert!(!vendored.is_empty(), "the workspace vendors reference code");
    for m in vendored {
        assert!(
            m.is_copied(),
            "{} is under vendor/ and must record its provenance",
            m.rel_path
        );
        assert!(
            !report.symbols.iter().any(|s| s.module_uri == m.module_uri),
            "{} is manifest-only and must claim no symbols",
            m.rel_path
        );
    }
}
