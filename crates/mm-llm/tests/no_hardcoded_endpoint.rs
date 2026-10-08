//! No endpoint is spelled in this crate's source.
//!
//! A hard-coded fallback host is how a misconfigured kernel silently sends prompts
//! somewhere else, so the substrate's rule is that an endpoint arrives only from
//! the environment. This test is the enforcement: it scans every source file of the
//! crate for an `http://`/`https://` literal, and it carries a positive control so
//! that a scanner that stopped matching would fail rather than pass.
//!
//! Only the *production* half of each file is scanned. A unit test's loopback
//! address cannot be reached by a deployment, and counting it would push real code
//! into string gymnastics to satisfy the check. Namespace IRIs are exempted by
//! construction: they come from `mm_core::iri`, so no file spells one out.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    mm_core::Config::repo_root()
}

/// Everything before the first `#[cfg(test)]`, which in this crate is the whole
/// production half of the file.
fn production_half(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

/// The `(line number, line)` pairs that spell an endpoint.
fn offending_lines(source: &str) -> Vec<(usize, String)> {
    production_half(source)
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("http://") || line.contains("https://"))
        .map(|(index, line)| (index + 1, line.trim().to_string()))
        .collect()
}

fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn no_source_file_spells_an_endpoint_literally() {
    let src = repo_root().join("crates").join("mm-llm").join("src");
    let files = rust_sources(&src);
    assert!(
        files.len() >= 15,
        "the scan must cover the crate's modules, saw {} under {}",
        files.len(),
        src.display()
    );

    let mut offenders = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for (line, text) in offending_lines(&source) {
            offenders.push(format!("{}:{line}: {text}", file.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "an endpoint must come from the environment, never from source:\n{}",
        offenders.join("\n")
    );
}

/// The control: the scanner still recognises the thing it forbids.
#[test]
fn the_scanner_catches_a_hard_coded_endpoint() {
    let sample = "    let url = \"https://api.example.invalid/v1/chat\";\n";
    assert_eq!(offending_lines(sample).len(), 1);

    let sample = "    let base = \"http://127.0.0.1:8080/v1\";\n";
    assert_eq!(offending_lines(sample).len(), 1);

    // A namespace built from a constant is not a literal, which is why the
    // provenance emitter can name the `prov:` IRI without tripping this check.
    let sample = "    let prefix = format!(\"@prefix prov: <{}> .\", mm_core::iri::PROV);\n";
    assert!(offending_lines(sample).is_empty());

    // And the test half of a file is out of scope for the scan.
    let sample =
        "#[cfg(test)]\nmod tests {\n    const LOOPBACK: &str = \"http://127.0.0.1:9/v1\";\n}\n";
    assert!(offending_lines(sample).is_empty());
}
