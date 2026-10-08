//! `codex meta`, exercised against an in-memory `/code` graph.
//!
//! Two of the queries are deliberately answered without a graph — a file may be an
//! orphan only because it is *not* a node, and a cycle is a shape of the dependency
//! triples that is easier to read from the report than with a SPARQL property path —
//! and this file proves both paths work, so a reader knows which store answers what.

use std::path::Path;

use mm_codex::model::{
    CapabilityRecord, CodexReport, DepKind, DependencyEdge, FileRecord, ModuleKind, ModuleRecord,
    SymbolKind, SymbolRecord,
};
use mm_codex::{answer_meta, MetaContext, MetaQuery};

fn module(rel: &str) -> ModuleRecord {
    ModuleRecord {
        rel_path: rel.into(),
        module_uri: mm_core::codex::module_iri(rel).into_string(),
        name: rel.rsplit('/').next().unwrap_or(rel).into(),
        version: "0.1.0".into(),
        kind: ModuleKind::RustCrate,
        category: None,
        crate_name: Some(rel.into()),
        owned_phase: 2,
        capability: "mm:Untested".into(),
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

/// One module, one file, one untested symbol, one capability with no test.
fn report() -> CodexReport {
    let module_uri = mm_core::codex::module_iri("crates/alpha").into_string();
    let rel_path = "crates/alpha/src/lib.rs".to_string();
    CodexReport {
        modules: vec![module("crates/alpha")],
        files: vec![FileRecord {
            rel_path: rel_path.clone(),
            module_uri: module_uri.clone(),
            content_hash: "bb".repeat(32),
            swhid: Some("swh:1:cnt:cc".into()),
            language: "rust".into(),
            changed: false,
        }],
        symbols: vec![SymbolRecord {
            rel_path,
            module_uri: module_uri.clone(),
            descriptor: "alpha().".into(),
            kind: SymbolKind::Fn,
            is_public: true,
            is_test: false,
            is_interface: true,
            byte_start: 0,
            byte_end: 20,
            content_hash: "dd".repeat(32),
        }],
        capabilities: vec![CapabilityRecord {
            capability: "mm:Untested".into(),
            module_uri,
            test_count: 0,
        }],
        graph_hash: "gh".into(),
        ..CodexReport::default()
    }
}

#[test]
fn query_names_parse_case_insensitively_and_reject_unknown() {
    for query in MetaQuery::ALL {
        assert_eq!(
            MetaQuery::parse(query.name()).expect("its own name must parse"),
            query
        );
        assert_eq!(
            MetaQuery::parse(&query.name().to_ascii_uppercase())
                .expect("parsing is case-insensitive"),
            query
        );
    }
    assert!(
        MetaQuery::parse("Nope").is_err(),
        "an unknown query must fail loudly"
    );
}

#[tokio::test]
async fn graph_backed_queries_read_the_code_graph() {
    let store = mm_store_graph::GraphStore::in_memory(Path::new("unused-shapes.ttl"))
        .await
        .expect("an in-memory graph store must open");
    let report = report();
    let inserted = store
        .handle()
        .insert_turtle("code", &mm_codex::emit::turtle(&report))
        .await
        .expect("the emitted graph must parse");
    assert!(inserted > 0, "the document must have inserted triples");

    let graph: &dyn mm_core::store::Graph = &store;
    let ctx = MetaContext {
        report: &report,
        graph: Some(graph),
        sqlite: None,
        lock: None,
    };

    let total = answer_meta(&ctx, MetaQuery::TotalModules)
        .await
        .expect("TotalModules is graph-backed");
    assert_eq!(total.scalar(), Some("1"), "one module was emitted");

    let per_phase = answer_meta(&ctx, MetaQuery::ModulesPerPhase)
        .await
        .expect("ModulesPerPhase is graph-backed");
    assert_eq!(per_phase.rows.len(), 1, "{:?}", per_phase.rows);
    assert!(
        per_phase.rows[0][0].ends_with("/phase/2"),
        "the phase IRI must survive the query: {:?}",
        per_phase.rows
    );

    let per_module = answer_meta(&ctx, MetaQuery::SymbolsPerModule)
        .await
        .expect("SymbolsPerModule is graph-backed");
    assert_eq!(per_module.rows.len(), 1, "{:?}", per_module.rows);
    assert_eq!(per_module.rows[0][1], "1", "the one symbol must be counted");

    let untested = answer_meta(&ctx, MetaQuery::CapabilitiesWithoutTests)
        .await
        .expect("CapabilitiesWithoutTests is graph-backed");
    assert_eq!(untested.rows.len(), 1, "{:?}", untested.rows);
    assert!(
        untested.rows[0][0].ends_with("/capability/Untested"),
        "{:?}",
        untested.rows
    );

    let copied = answer_meta(&ctx, MetaQuery::CopiedFrom)
        .await
        .expect("CopiedFrom is graph-backed");
    assert!(copied.rows.is_empty(), "nothing here was copied in");

    store
        .shutdown()
        .await
        .expect("the graph writer must shut down");
}

#[tokio::test]
async fn a_graph_query_without_a_graph_is_an_error_not_an_empty_answer() {
    let report = report();
    let ctx = MetaContext {
        report: &report,
        graph: None,
        sqlite: None,
        lock: None,
    };
    assert!(
        answer_meta(&ctx, MetaQuery::TotalModules).await.is_err(),
        "a graph-backed query must not silently answer from nothing"
    );
}

#[tokio::test]
async fn report_backed_queries_answer_without_a_graph() {
    let mut report = report();
    report.orphan_files = vec!["crates/stray.rs".into()];
    let alpha = mm_core::codex::module_iri("crates/alpha").into_string();
    let beta = mm_core::codex::module_iri("crates/beta").into_string();
    report.modules.push(module("crates/beta"));
    report.dependencies = vec![
        DependencyEdge {
            from_uri: alpha.clone(),
            to_uri: beta.clone(),
            kind: DepKind::Crate,
        },
        DependencyEdge {
            from_uri: beta,
            to_uri: alpha,
            kind: DepKind::Reference,
        },
    ];

    let ctx = MetaContext {
        report: &report,
        graph: None,
        sqlite: None,
        lock: None,
    };

    let orphans = answer_meta(&ctx, MetaQuery::OrphanFiles)
        .await
        .expect("OrphanFiles is answered from the report");
    assert_eq!(orphans.rows, vec![vec!["crates/stray.rs".to_string()]]);

    let cycles = answer_meta(&ctx, MetaQuery::DependencyCycles)
        .await
        .expect("DependencyCycles is answered from the report");
    assert_eq!(cycles.rows.len(), 1, "{:?}", cycles.rows);
    assert!(
        cycles.rows[0][0].contains("crates/alpha") && cycles.rows[0][0].contains("crates/beta"),
        "{:?}",
        cycles.rows
    );

    let drift = answer_meta(&ctx, MetaQuery::VersionDrift)
        .await
        .expect("VersionDrift answers with no lock as 'no drift'");
    assert!(
        drift.rows.is_empty(),
        "no lock means the versions agree by definition"
    );
}
