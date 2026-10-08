//! `codex meta` — the questions the code graph answers.
//!
//! Where a fact is in the `/code` graph, it is answered with SPARQL rather than by
//! walking the report in Rust: the graph is the authority, and a bespoke loop would
//! be a second, silently divergent definition of the same fact. The exceptions are
//! facts the graph does not carry by construction — churn lives in SQLite, and
//! orphan files are files that are *not* nodes at all — which are answered from
//! their own store and labelled as such.

use mm_core::store::Graph;
use mm_core::MmError;
use serde_json::Value;

use crate::lock::CodexLock;
use crate::model::CodexReport;
use crate::verify;

/// A question `codex meta` can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaQuery {
    /// How many modules the workspace has.
    TotalModules,
    /// How many modules each phase owns.
    ModulesPerPhase,
    /// How many symbols each module defines, most first.
    SymbolsPerModule,
    /// Capabilities that no test covers (manifest-only modules excluded).
    CapabilitiesWithoutTests,
    /// Cycles in the module dependency graph.
    DependencyCycles,
    /// Modules changed without a version bump since the lock.
    VersionDrift,
    /// Scan history: modules, files, symbols, changed files per run.
    Churn,
    /// Files under a scanned root that belong to no module.
    OrphanFiles,
    /// Modules recorded as copied in, with their origin.
    CopiedFrom,
    /// Symbols nothing in their module references.
    UnreferencedSymbols,
}

impl MetaQuery {
    /// Every query, in a stable order.
    pub const ALL: [MetaQuery; 10] = [
        MetaQuery::TotalModules,
        MetaQuery::ModulesPerPhase,
        MetaQuery::SymbolsPerModule,
        MetaQuery::CapabilitiesWithoutTests,
        MetaQuery::DependencyCycles,
        MetaQuery::VersionDrift,
        MetaQuery::Churn,
        MetaQuery::OrphanFiles,
        MetaQuery::CopiedFrom,
        MetaQuery::UnreferencedSymbols,
    ];

    /// The query's canonical name.
    pub fn name(self) -> &'static str {
        match self {
            MetaQuery::TotalModules => "TotalModules",
            MetaQuery::ModulesPerPhase => "ModulesPerPhase",
            MetaQuery::SymbolsPerModule => "SymbolsPerModule",
            MetaQuery::CapabilitiesWithoutTests => "CapabilitiesWithoutTests",
            MetaQuery::DependencyCycles => "DependencyCycles",
            MetaQuery::VersionDrift => "VersionDrift",
            MetaQuery::Churn => "Churn",
            MetaQuery::OrphanFiles => "OrphanFiles",
            MetaQuery::CopiedFrom => "CopiedFrom",
            MetaQuery::UnreferencedSymbols => "UnreferencedSymbols",
        }
    }

    /// Parse a query name, case-insensitively.
    pub fn parse(name: &str) -> Result<Self, MmError> {
        MetaQuery::ALL
            .into_iter()
            .find(|q| q.name().eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| {
                let known: Vec<&str> = MetaQuery::ALL.iter().map(|q| q.name()).collect();
                MmError::Config(format!(
                    "unknown meta query {name:?}; known queries are {}",
                    known.join(", ")
                ))
            })
    }
}

/// The answer to one query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaAnswer {
    /// The query that was answered.
    pub query: MetaQuery,
    /// Column headers, in order.
    pub columns: Vec<String>,
    /// Rows, each cell already rendered as text.
    pub rows: Vec<Vec<String>>,
}

impl MetaAnswer {
    fn new(query: MetaQuery, columns: &[&str], rows: Vec<Vec<String>>) -> Self {
        MetaAnswer {
            query,
            columns: columns.iter().map(|c| (*c).to_string()).collect(),
            rows,
        }
    }

    /// The node count a simple counting query reports, or `None`.
    pub fn scalar(&self) -> Option<&str> {
        if self.rows.len() == 1 && self.rows[0].len() == 1 {
            Some(self.rows[0][0].as_str())
        } else {
            None
        }
    }
}

/// Everything a meta query can read.
pub struct MetaContext<'a> {
    /// The scanned report (authoritative for the facts it holds).
    pub report: &'a CodexReport,
    /// The `/code` graph, when the kernel is open.
    pub graph: Option<&'a dyn Graph>,
    /// The tabular store, for churn.
    pub sqlite: Option<&'a mm_store_sqlite::SqliteStore>,
    /// The recorded lock, for version drift.
    pub lock: Option<&'a CodexLock>,
}

fn code_graph() -> String {
    mm_core::iri::graph("code")
}

/// Extract a variable from a result row, rendered as text.
///
/// A SPARQL result renders a named node as a bare string and a literal as an
/// object with a `value`, so an `xsd:integer` count arrives as
/// `{"value":"13","datatype":…}`. A SQLite row renders numbers as JSON numbers.
/// All three shapes reduce to their text here.
fn cell(row: &Value, variable: &str) -> String {
    match row.get(variable) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Object(map)) => map
            .get("value")
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn require_graph<'a>(ctx: &MetaContext<'a>) -> Result<&'a dyn Graph, MmError> {
    ctx.graph.ok_or_else(|| {
        MmError::Graph(format!(
            "{} needs the `/code` graph; run `mm-cli codex scan` first",
            "this query"
        ))
    })
}

/// Answer one query.
pub async fn answer(ctx: &MetaContext<'_>, query: MetaQuery) -> Result<MetaAnswer, MmError> {
    match query {
        MetaQuery::TotalModules => {
            let graph = require_graph(ctx)?;
            let iri = code_graph();
            let sparql = format!(
                "SELECT (COUNT(DISTINCT ?m) AS ?n) WHERE {{ GRAPH <{iri}> {{ ?m a <https://metamind.dev/code#Module> }} }}"
            );
            let rows = graph.sparql("code", &sparql).await?;
            let n = rows.first().map(|r| cell(r, "n")).unwrap_or_default();
            Ok(MetaAnswer::new(query, &["modules"], vec![vec![n]]))
        }
        MetaQuery::ModulesPerPhase => {
            let graph = require_graph(ctx)?;
            let iri = code_graph();
            let sparql = format!(
                "SELECT ?phase (COUNT(DISTINCT ?m) AS ?n) WHERE {{ \
                 GRAPH <{iri}> {{ ?m a <https://metamind.dev/code#Module> ; <https://metamind.dev/code#ownedByPhase> ?phase }} \
                 }} GROUP BY ?phase ORDER BY ?phase"
            );
            let rows = graph.sparql("code", &sparql).await?;
            Ok(MetaAnswer::new(
                query,
                &["phase", "modules"],
                rows.iter()
                    .map(|r| vec![cell(r, "phase"), cell(r, "n")])
                    .collect(),
            ))
        }
        MetaQuery::SymbolsPerModule => {
            let graph = require_graph(ctx)?;
            let iri = code_graph();
            let sparql = format!(
                "SELECT ?module (COUNT(?s) AS ?n) WHERE {{ \
                 GRAPH <{iri}> {{ ?s a <https://metamind.dev/code#Symbol> ; <https://metamind.dev/code#definedIn> ?f . \
                 ?f <https://metamind.dev/code#inModule> ?module }} \
                 }} GROUP BY ?module ORDER BY DESC(?n) ?module"
            );
            let rows = graph.sparql("code", &sparql).await?;
            Ok(MetaAnswer::new(
                query,
                &["module", "symbols"],
                rows.iter()
                    .map(|r| vec![cell(r, "module"), cell(r, "n")])
                    .collect(),
            ))
        }
        MetaQuery::CapabilitiesWithoutTests => {
            let graph = require_graph(ctx)?;
            let iri = code_graph();
            // A manifest-only module promises nothing checkable, so it is excluded —
            // the same rule `codex verify` uses.
            let sparql = format!(
                "SELECT DISTINCT ?c WHERE {{ \
                 GRAPH <{iri}> {{ ?c a <https://metamind.dev/code#Capability> . \
                 ?m <https://metamind.dev/code#implementsCapability> ?c . \
                 FILTER NOT EXISTS {{ ?c <https://metamind.dev/code#hasTest> ?t }} \
                 FILTER NOT EXISTS {{ ?m <https://metamind.dev/code#generatedFrom> \"manifest\" }} }} \
                 }} ORDER BY ?c"
            );
            let rows = graph.sparql("code", &sparql).await?;
            Ok(MetaAnswer::new(
                query,
                &["capability"],
                rows.iter().map(|r| vec![cell(r, "c")]).collect(),
            ))
        }
        MetaQuery::CopiedFrom => {
            let graph = require_graph(ctx)?;
            let iri = code_graph();
            let sparql = format!(
                "SELECT ?m ?from ?revision WHERE {{ GRAPH <{iri}> {{ ?m <https://metamind.dev/code#copiedFrom> ?from . \
                 OPTIONAL {{ ?m <https://metamind.dev/code#copiedRevision> ?revision }} }} }} ORDER BY ?m"
            );
            let rows = graph.sparql("code", &sparql).await?;
            Ok(MetaAnswer::new(
                query,
                &["module", "copied_from", "revision"],
                rows.iter()
                    .map(|r| vec![cell(r, "m"), cell(r, "from"), cell(r, "revision")])
                    .collect(),
            ))
        }
        MetaQuery::DependencyCycles => Ok(MetaAnswer::new(
            query,
            &["cycle"],
            verify::dependency_cycles(ctx.report)
                .into_iter()
                .map(|cycle| vec![cycle.join(" -> ")])
                .collect(),
        )),
        MetaQuery::OrphanFiles => Ok(MetaAnswer::new(
            query,
            &["rel_path"],
            ctx.report
                .orphan_files
                .iter()
                .map(|p| vec![p.clone()])
                .collect(),
        )),
        MetaQuery::UnreferencedSymbols => {
            let referenced: std::collections::BTreeSet<&str> = ctx
                .report
                .references
                .iter()
                .map(|r| r.to_symbol.as_str())
                .collect();
            let rows: Vec<Vec<String>> = ctx
                .report
                .symbols
                .iter()
                .filter(|s| {
                    !s.is_test && s.is_interface && !referenced.contains(s.descriptor.as_str())
                })
                .map(|s| vec![s.descriptor.clone(), s.module_uri.clone()])
                .collect();
            Ok(MetaAnswer::new(query, &["descriptor", "module"], rows))
        }
        MetaQuery::VersionDrift => {
            let mut rows = Vec::new();
            if let Some(lock) = ctx.lock {
                for locked in &lock.module {
                    if let Some(m) = ctx
                        .report
                        .modules
                        .iter()
                        .find(|m| m.module_uri == locked.uri)
                    {
                        if m.content_hash != locked.content_hash && m.version == locked.version {
                            rows.push(vec![
                                m.module_uri.clone(),
                                locked.version.clone(),
                                m.content_hash.clone(),
                            ]);
                        }
                    }
                }
            }
            Ok(MetaAnswer::new(
                query,
                &["module", "version", "content_hash"],
                rows,
            ))
        }
        MetaQuery::Churn => {
            let sqlite = ctx.sqlite.ok_or_else(|| {
                MmError::Store("the Churn query needs the tabular store".to_string())
            })?;
            use mm_core::{Params, Tabular};
            let rows = sqlite
                .query_json(
                    "SELECT started_at, modules, files, symbols, changed_files, graph_hash \
                     FROM codex_run ORDER BY started_at, id",
                    Params::new(),
                )
                .await?;
            Ok(MetaAnswer::new(
                query,
                &[
                    "started_at",
                    "modules",
                    "files",
                    "symbols",
                    "changed_files",
                    "graph_hash",
                ],
                rows.iter()
                    .map(|r| {
                        vec![
                            cell(r, "started_at"),
                            cell(r, "modules"),
                            cell(r, "files"),
                            cell(r, "symbols"),
                            cell(r, "changed_files"),
                            cell(r, "graph_hash"),
                        ]
                    })
                    .collect(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn query_names_parse_case_insensitively() {
        assert_eq!(
            MetaQuery::parse("TotalModules").unwrap(),
            MetaQuery::TotalModules
        );
        assert_eq!(MetaQuery::parse("orbits").ok(), None);
        assert!(MetaQuery::parse("orbits").is_err());
        for q in MetaQuery::ALL {
            assert_eq!(MetaQuery::parse(q.name()).unwrap(), q);
        }
    }

    fn answer_shape() -> MetaAnswer {
        MetaAnswer::new(
            MetaQuery::TotalModules,
            &["modules"],
            vec![vec!["3".into()]],
        )
    }

    #[test]
    fn a_single_cell_answer_is_a_scalar() {
        assert_eq!(answer_shape().scalar(), Some("3"));
    }

    #[test]
    fn cells_render_numbers_objects_and_named_nodes() {
        let patched = serde_json::json!({
            "named": "https://example.com/x",
            "count": 13,
            "typed": { "value": "7", "datatype": "http://www.w3.org/2001/XMLSchema#integer" },
            "flag": true,
        });
        assert_eq!(cell(&patched, "named"), "https://example.com/x");
        assert_eq!(cell(&patched, "count"), "13");
        assert_eq!(cell(&patched, "typed"), "7");
        assert_eq!(cell(&patched, "flag"), "true");
        assert_eq!(cell(&patched, "missing"), "");
    }

    #[tokio::test]
    async fn total_modules_is_answered_by_sparql_over_the_code_graph() {
        let store = mm_store_graph::GraphStore::in_memory(Path::new("unused.ttl"))
            .await
            .unwrap();
        let report = crate::model::CodexReport {
            modules: vec![crate::model::ModuleRecord {
                rel_path: "crates/alpha".into(),
                module_uri: mm_core::codex::module_iri("crates/alpha").into_string(),
                name: "alpha".into(),
                version: "0.1.0".into(),
                kind: crate::model::ModuleKind::RustCrate,
                category: None,
                crate_name: Some("alpha".into()),
                owned_phase: 2,
                capability: "mm:CrateAlpha".into(),
                content_hash: "aa".repeat(32),
                copied_from: None,
                copied_revision: None,
                copied_license: None,
                origin: Some("metamind".into()),
                manifest_only: false,
                declared_uri: None,
                tbox_functions: vec![],
                depends_on: vec![],
            }],
            ..CodexReport::default()
        };
        store
            .handle()
            .insert_turtle("code", &crate::emit::turtle(&report))
            .await
            .unwrap();

        let ctx = MetaContext {
            report: &report,
            graph: Some(&store),
            sqlite: None,
            lock: None,
        };
        let total = answer(&ctx, MetaQuery::TotalModules).await.unwrap();
        assert_eq!(total.scalar(), Some("1"));

        let per_phase = answer(&ctx, MetaQuery::ModulesPerPhase).await.unwrap();
        assert_eq!(per_phase.rows.len(), 1);
        assert!(
            per_phase.rows[0][0].ends_with("/phase/2"),
            "{:?}",
            per_phase.rows
        );

        // The capability in this report has no test, so it is reported.
        let untested = answer(&ctx, MetaQuery::CapabilitiesWithoutTests)
            .await
            .unwrap();
        assert_eq!(untested.rows.len(), 1);
        store.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_graph_backed_query_without_a_graph_is_an_error() {
        let report = CodexReport::default();
        let ctx = MetaContext {
            report: &report,
            graph: None,
            sqlite: None,
            lock: None,
        };
        assert!(answer(&ctx, MetaQuery::TotalModules).await.is_err());
    }
}
