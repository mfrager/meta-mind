//! Golden artifacts for a fixture tree.
//!
//! These pin the exact bytes the scanner commits — the per-module `metadata.ttl`,
//! `registry.json`, `codex.lock`, and the `/code` graph hash — so a change to the
//! identifier scheme, the emission order, or the record set shows up as a reviewed
//! snapshot diff instead of silently rewriting the committed artifacts.
//!
//! The snapshots live in `bench/codex/golden/` rather than the crate's default
//! `tests/snapshots/`, because they document the generator's output rather than a
//! single test's behaviour. Regenerate them with
//! `INSTA_UPDATE=always cargo test -p mm-codex --test golden`.

use std::path::{Path, PathBuf};

use mm_codex::model::CodexReport;
use mm_codex::{CodexLock, PreviousState};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/codex/fixtures/good")
}

fn report() -> CodexReport {
    mm_codex::scan(&fixtures(), &PreviousState::default(), true)
        .expect("the good fixture must scan")
        .report
}

#[test]
fn canonical_module_metadata_ttl() {
    let report = report();
    let uri = mm_core::codex::module_iri("crates/alpha").into_string();
    let document = mm_codex::emit::module_metadata(&report, &uri).expect("alpha is in the report");
    insta::with_settings!({ snapshot_path => "../../../bench/codex/golden" }, {
        insta::assert_snapshot!("canonical_module_metadata_ttl", document);
    });
}

#[test]
fn canonical_registry_json() {
    let document = mm_codex::registry::registry_json(&report()).expect("the registry renders");
    insta::with_settings!({ snapshot_path => "../../../bench/codex/golden" }, {
        insta::assert_snapshot!("canonical_registry_json", document);
    });
}

#[test]
fn canonical_codex_lock() {
    let lock = CodexLock::from_report(&report());
    insta::with_settings!({ snapshot_path => "../../../bench/codex/golden" }, {
        insta::assert_snapshot!("canonical_codex_lock", lock.to_deterministic_string());
    });
}

#[test]
fn canonical_code_graph_hash() {
    let report = report();
    assert_eq!(report.graph_hash.len(), 64, "the graph hash is sha256 hex");
    assert!(report.graph_hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(
        report.graph_hash,
        mm_codex::emit::graph_hash(&report).expect("the graph hashes"),
        "the report's hash must be the emitter's hash"
    );
    insta::with_settings!({ snapshot_path => "../../../bench/codex/golden" }, {
        insta::assert_snapshot!("canonical_code_graph_hash", report.graph_hash);
    });
}
