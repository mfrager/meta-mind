//! Emission: records to canonical Turtle.
//!
//! The same records always produce the same bytes, so the graph hash is a real
//! fingerprint of the codebase rather than of the iteration order. Two rules make
//! that true: every collection is sorted before it is written, and the
//! canonicalization comes from `mm-store-graph` (sha256 over canonical
//! N-Triples), so there is exactly one definition of "the same triples" in the
//! system.
//!
//! Node typing follows one rule worth stating: a node is typed with **both** its
//! concrete class and its base class (`a mmc:Module, mmc:RustCrate`). `sh:targetClass`
//! does no subclass inference, so a node typed only with the concrete class would
//! never be validated.

use std::fmt::Write;

use mm_core::MmError;

use crate::model::{CodexReport, ModuleRecord};

/// The prefixes every emitted document declares.
pub const PREFIXES: &str = "\
@prefix mmc: <https://metamind.dev/code#> .\n\
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n";

/// Escape a Turtle string literal.
fn lit(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Write the triples for one module and everything it owns.
fn write_module(out: &mut String, report: &CodexReport, module: &ModuleRecord) {
    let uri = module.module_uri.as_str();
    let _ = write!(
        out,
        "<{uri}> a mmc:Module, mmc:{} ;\n    mmc:path {} ;\n    mmc:uri {} ;\n    mmc:version {} ;\n    mmc:ownedByPhase <{}> ;\n    mmc:implementsCapability <{}>",
        module.kind.class(),
        lit(&module.rel_path),
        lit(&module.module_uri),
        lit(&module.version),
        module.phase_iri().as_str(),
        module.capability_iri().as_str(),
    );
    let mut deps: Vec<&String> = module.depends_on.iter().collect();
    deps.sort();
    for dep in deps {
        let _ = write!(out, " ;\n    mmc:dependsOn <{dep}>");
    }
    let mut files: Vec<&str> = report
        .files
        .iter()
        .filter(|f| f.module_uri == module.module_uri)
        .map(|f| f.rel_path.as_str())
        .collect();
    files.sort_unstable();
    for rel in &files {
        if let Some(file) = report.files.iter().find(|f| f.rel_path == *rel) {
            let _ = write!(out, " ;\n    mmc:sourceFile <{}>", file.iri().as_str());
        }
    }
    if module.manifest_only {
        let _ = write!(out, " ;\n    mmc:generatedFrom {}", lit("manifest"));
    }
    if let Some(copied) = &module.copied_from {
        let _ = write!(out, " ;\n    mmc:copiedFrom {}", lit(copied));
    }
    if let Some(revision) = &module.copied_revision {
        let _ = write!(out, " ;\n    mmc:copiedRevision {}", lit(revision));
    }
    if let Some(license) = &module.copied_license {
        let _ = write!(out, " ;\n    mmc:copiedLicense {}", lit(license));
    }
    out.push_str(" .\n");

    // Files.
    for file in report
        .files
        .iter()
        .filter(|f| f.module_uri == module.module_uri)
    {
        let _ = write!(
            out,
            "<{}> a mmc:SourceFile ;\n    mmc:path {} ;\n    mmc:inModule <{uri}> ;\n    mmc:contentHash {} ;\n    mmc:language {}",
            file.iri().as_str(),
            lit(&file.rel_path),
            lit(&file.content_hash),
            lit(&file.language),
        );
        if let Some(swhid) = &file.swhid {
            let _ = write!(out, " ;\n    mmc:swhid {}", lit(swhid));
        }
        out.push_str(" .\n");
    }

    // Symbols.
    let mut symbols: Vec<&crate::model::SymbolRecord> = report
        .symbols
        .iter()
        .filter(|s| s.module_uri == module.module_uri)
        .collect();
    symbols.sort_by(|a, b| (&a.rel_path, a.byte_start).cmp(&(&b.rel_path, b.byte_start)));
    for symbol in &symbols {
        let class = if symbol.is_interface {
            "mmc:Symbol, mmc:Interface"
        } else {
            "mmc:Symbol"
        };
        let defined_in =
            mm_core::codex::file_iri(&symbol.rel_path, &file_hash_of(report, &symbol.rel_path));
        let _ = write!(
            out,
            "<{}> a {class} ;\n    mmc:descriptor {} ;\n    mmc:kind {} ;\n    mmc:definedIn <{}> ;\n    mmc:byteStart {} ;\n    mmc:byteEnd {} .\n",
            symbol.iri().as_str(),
            lit(&symbol.descriptor),
            lit(symbol.kind.as_str()),
            defined_in.as_str(),
            symbol.byte_start,
            symbol.byte_end,
        );
    }

    // The capability node, with a hasTest edge for every test the module has.
    let _ = write!(
        out,
        "<{}> a mmc:Capability ;\n    mmc:name {}",
        module.capability_iri().as_str(),
        lit(module.capability.trim_start_matches("mm:")),
    );
    let mut tests: Vec<&crate::model::SymbolRecord> =
        symbols.iter().copied().filter(|s| s.is_test).collect();
    tests.sort_by(|a, b| (&a.rel_path, a.byte_start).cmp(&(&b.rel_path, b.byte_start)));
    for test in tests {
        let _ = write!(out, " ;\n    mmc:hasTest <{}>", test.iri().as_str());
    }
    out.push_str(" .\n");
}

fn file_hash_of(report: &CodexReport, rel_path: &str) -> String {
    report
        .files
        .iter()
        .find(|f| f.rel_path == rel_path)
        .map(|f| f.content_hash.clone())
        .unwrap_or_default()
}

/// The whole `/code` graph as Turtle.
pub fn turtle(report: &CodexReport) -> String {
    let mut out = String::from(PREFIXES);
    let mut modules: Vec<&ModuleRecord> = report.modules.iter().collect();
    modules.sort_by(|a, b| a.module_uri.cmp(&b.module_uri));
    for module in modules {
        write_module(&mut out, report, module);
    }
    out
}

/// One module's `metadata.ttl`, as deterministic Turtle.
///
/// The text is the emitter's own ordering, not the vendored serializer's: re-emitting
/// the same triples through `rdf-codec`'s Turtle writer yields a different byte
/// order on each run, so it cannot be the artifact a committed, diffable file is
/// built from. The document is still validated by parsing it here, so a syntax
/// error surfaces as a hard error rather than in a later `graph validate`.
pub fn module_metadata(report: &CodexReport, module_uri: &str) -> Result<String, MmError> {
    let module = report
        .modules
        .iter()
        .find(|m| m.module_uri == module_uri)
        .ok_or_else(|| MmError::Internal(format!("no such module in the report: {module_uri}")))?;
    let mut out = String::from(PREFIXES);
    write_module(&mut out, report, module);
    mm_store_graph::canonical_ntriples_of_turtle(&out)?;
    Ok(out)
}

/// The canonical N-Triples form of the `/code` graph.
///
/// Sorted, so it is the byte-stable form a hash may be taken over — unlike the
/// vendored Turtle writer's output.
pub fn canonical_ntriples(report: &CodexReport) -> Result<String, MmError> {
    mm_store_graph::canonical_ntriples_of_turtle(&turtle(report))
}

/// The sha256 hash of the canonical `/code` graph.
///
/// This is the `graph_hash` the gate compares across two scans and `codex.lock`
/// records, so it must depend on the triples and nothing else.
pub fn graph_hash(report: &CodexReport) -> Result<String, MmError> {
    mm_store_graph::canonical_hash_of_turtle(&turtle(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn sample() -> CodexReport {
        let module_uri = mm_core::iri::module("crates/demo").into_string();
        let file = FileRecord {
            rel_path: "crates/demo/src/lib.rs".into(),
            module_uri: module_uri.clone(),
            content_hash: "aa".into(),
            swhid: Some("swh:1:cnt:bb".into()),
            language: "rust".into(),
            changed: true,
        };
        let symbol = SymbolRecord {
            rel_path: "crates/demo/src/lib.rs".into(),
            module_uri: module_uri.clone(),
            descriptor: "Thing#run().".into(),
            kind: SymbolKind::Fn,
            is_public: true,
            is_test: false,
            is_interface: true,
            byte_start: 3,
            byte_end: 9,
            content_hash: "cc".into(),
        };
        let test = SymbolRecord {
            descriptor: "tests#it_works().".into(),
            is_test: true,
            is_interface: false,
            byte_start: 20,
            byte_end: 30,
            ..symbol.clone()
        };
        CodexReport {
            modules: vec![ModuleRecord {
                rel_path: "crates/demo".into(),
                module_uri: module_uri.clone(),
                name: "demo".into(),
                version: "0.1.0".into(),
                kind: ModuleKind::RustCrate,
                category: None,
                crate_name: Some("demo".into()),
                owned_phase: 2,
                capability: "mm:CrateDemo".into(),
                content_hash: "dd".into(),
                copied_from: None,
                copied_revision: None,
                copied_license: None,
                origin: Some("metamind".into()),
                manifest_only: false,
                declared_uri: Some(module_uri.clone()),
                tbox_functions: vec![],
                depends_on: vec![],
            }],
            files: vec![file],
            symbols: vec![symbol, test],
            capabilities: vec![CapabilityRecord {
                capability: "mm:CrateDemo".into(),
                module_uri,
                test_count: 1,
            }],
            ..CodexReport::default()
        }
    }

    #[test]
    fn a_module_is_typed_with_both_its_concrete_and_base_class() {
        let t = turtle(&sample());
        assert!(
            t.contains("a mmc:Module, mmc:RustCrate"),
            "sh:targetClass does no subclass inference, so both are needed:\n{t}"
        );
    }

    #[test]
    fn interfaces_are_typed_as_symbols_too() {
        let t = turtle(&sample());
        assert!(t.contains("a mmc:Symbol, mmc:Interface"), "{t}");
        // A non-interface symbol stays a plain Symbol.
        assert_eq!(t.matches("a mmc:Symbol ;").count(), 1, "{t}");
    }

    #[test]
    fn every_module_carries_the_fields_the_shapes_require() {
        let t = turtle(&sample());
        for required in [
            "mmc:path",
            "mmc:uri",
            "mmc:version",
            "mmc:ownedByPhase",
            "mmc:implementsCapability",
            "mmc:contentHash",
            "mmc:inModule",
            "mmc:descriptor",
            "mmc:definedIn",
        ] {
            assert!(t.contains(required), "missing {required}:\n{t}");
        }
    }

    #[test]
    fn a_capability_points_at_the_tests_that_cover_it() {
        let t = turtle(&sample());
        assert!(t.contains("mmc:hasTest"), "{t}");
        assert!(t.contains(r#"mmc:name "CrateDemo""#), "{t}");
    }

    #[test]
    fn emission_is_deterministic_and_order_independent() {
        let mut a = sample();
        let mut b = sample();
        b.files.reverse();
        b.symbols.reverse();
        assert_eq!(turtle(&a), turtle(&b));
        assert_eq!(graph_hash(&a).unwrap(), graph_hash(&b).unwrap());

        // A real change shows up in the hash.
        a.symbols[0].descriptor = "Thing#run_all().".into();
        assert_ne!(graph_hash(&a).unwrap(), graph_hash(&b).unwrap());
    }

    #[test]
    fn the_emitted_graph_parses_and_canonicalizes() {
        let document = turtle(&sample());
        let ntriples = canonical_ntriples(&sample())
            .unwrap_or_else(|e| panic!("{e}\n--- document ---\n{document}\n--- end ---"));
        assert!(ntriples.contains("mmc:RustCrate") || ntriples.contains("code#RustCrate"));
        assert_eq!(graph_hash(&sample()).unwrap().len(), 64);
    }

    #[test]
    fn copied_modules_record_their_provenance() {
        let mut report = sample();
        report.modules[0].copied_from =
            Some("https://github.com/mfrager/blanc-scripts@c03aee5".into());
        report.modules[0].copied_license = Some("unspecified".into());
        report.modules[0].origin = Some("rust_symbolic".into());
        report.modules[0].rel_path = "vendor/rust_symbolic/demo".into();
        let t = turtle(&report);
        assert!(t.contains("mmc:copiedFrom"), "{t}");
        assert!(t.contains(r#"mmc:copiedLicense "unspecified""#), "{t}");
    }

    #[test]
    fn module_metadata_is_canonical_turtle() {
        let report = sample();
        let ttl = module_metadata(&report, &report.modules[0].module_uri).unwrap();
        assert!(ttl.contains("mmc:RustCrate"), "{ttl}");
        assert!(
            ttl.contains("<https://metamind.dev/code/module/crates/demo>"),
            "{ttl}"
        );
        // Canonical form is stable across repeat emission.
        assert_eq!(
            ttl,
            module_metadata(&report, &report.modules[0].module_uri).unwrap()
        );
    }

    #[test]
    fn asking_for_a_module_that_is_not_there_is_an_error() {
        let err = module_metadata(&sample(), "https://metamind.dev/code/module/nope").unwrap_err();
        assert!(matches!(err, MmError::Internal(_)), "got {err:?}");
    }
}
