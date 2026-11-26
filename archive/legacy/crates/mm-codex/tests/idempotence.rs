//! Idempotence: the same tree must always produce the same artifacts.
//!
//! The gate compares two scans' graph hashes byte-for-byte, and `codex.lock` is
//! committed, so a scan that depended on filesystem iteration order, wall-clock
//! time, or the previous run's state would produce a diff on every run and make the
//! lock meaningless. These tests pin the property directly, including across the
//! incremental path that reuses unchanged files' symbols verbatim.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mm_codex::model::{ReferenceRecord, SymbolRecord};
use mm_codex::{emit, registry, CodexLock, PreviousState, ScanOutput};

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../bench/codex/fixtures"
    ))
    .join(name)
}

fn scan(name: &str, previous: &PreviousState) -> ScanOutput {
    let dir = fixture_dir(name);
    mm_codex::scan(&dir, previous, false).unwrap_or_else(|e| panic!("scanning {name} failed: {e}"))
}

/// Group symbols by the file that defines them, the shape `PreviousState` wants.
fn symbols_by_file(report: &mm_codex::CodexReport) -> HashMap<String, Vec<SymbolRecord>> {
    let mut out: HashMap<String, Vec<SymbolRecord>> = HashMap::new();
    for symbol in &report.symbols {
        out.entry(symbol.rel_path.clone())
            .or_default()
            .push(symbol.clone());
    }
    out
}

/// Group references by the file they appear in.
fn references_by_file(report: &mm_codex::CodexReport) -> HashMap<String, Vec<ReferenceRecord>> {
    let mut out: HashMap<String, Vec<ReferenceRecord>> = HashMap::new();
    for reference in &report.references {
        out.entry(reference.file_path.clone())
            .or_default()
            .push(reference.clone());
    }
    out
}

#[test]
fn two_scans_of_the_same_tree_are_identical() {
    let first = scan("good", &PreviousState::default());
    let second = scan("good", &PreviousState::default());

    assert_eq!(
        first.report.graph_hash, second.report.graph_hash,
        "the graph hash must not depend on the order of the walk"
    );
    assert_eq!(first.turtle(), second.turtle());
    assert_eq!(
        registry::registry_json(&first.report).unwrap(),
        registry::registry_json(&second.report).unwrap()
    );
    assert_eq!(first.hashes, second.hashes);
}

#[test]
fn the_lock_is_byte_stable_across_scans() {
    let first = CodexLock::from_report(&scan("good", &PreviousState::default()).report);
    let second = CodexLock::from_report(&scan("good", &PreviousState::default()).report);

    assert_eq!(first, second);
    assert_eq!(
        first.to_deterministic_string(),
        second.to_deterministic_string(),
        "a committed lock file must not churn between runs"
    );
    // And it parses back to exactly what was written.
    assert_eq!(
        CodexLock::parse(&first.to_deterministic_string()).unwrap(),
        first
    );
}

#[test]
fn an_incremental_scan_reuses_unchanged_symbols_and_agrees() {
    let first = scan("good", &PreviousState::default());
    assert!(
        first.report.changed_files > 0,
        "the first scan sees changes"
    );

    // Seed the next scan with everything the first one recorded, so every file
    // looks unchanged and its symbols and references are reused verbatim.
    let previous = PreviousState {
        file_hashes: first.hashes.clone(),
        symbols_by_file: symbols_by_file(&first.report),
        references_by_file: references_by_file(&first.report),
    };
    let second = scan("good", &previous);

    assert_eq!(second.report.changed_files, 0, "nothing changed on disk");
    assert_eq!(
        first.report.graph_hash, second.report.graph_hash,
        "the incremental path must reproduce the same graph"
    );
    assert_eq!(first.report.symbols, second.report.symbols);
    assert_eq!(first.turtle(), second.turtle());
}

#[test]
fn the_canonical_graph_hash_matches_the_emitted_document() {
    let report = scan("good", &PreviousState::default()).report;
    assert_eq!(
        report.graph_hash,
        emit::graph_hash(&report).unwrap(),
        "the record's hash must be the hash of the document it emits"
    );
    assert!(!emit::canonical_ntriples(&report).unwrap().is_empty());
}
