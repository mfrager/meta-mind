//! `graph.query`: read a named graph with a read-only SPARQL query.
//!
//! Two restrictions, both structural rather than advisory. The graph must be one the
//! kernel owns — [`mm_core::iri::is_named_graph`], the same list a migration has to
//! extend — and the query must *start* with a read form and contain no update keyword,
//! so the tool cannot be turned into a writer by a query that happens to begin with
//! `PREFIX`.
//!
//! The capability check is the executor's: this tool declares `graph:read:**`, and a
//! caller without a covering grant never reaches this code. What this module adds is the
//! check the capability cannot express — that the *content* of the query is a read.

use async_trait::async_trait;
use serde_json::json;

use crate::error::ToolError;
use crate::permissions::{PermissionAction, PermissionReq};
use crate::registry::{ExecCtx, Tool, ToolArgs, ToolOutcome};
use crate::sandbox::SandboxTier;
use crate::spec::{
    JsonSchema, Reversibility, SideEffectClass, ToolAnnotations, ToolName, ToolSpec,
};

/// The SPARQL keywords that would change the graph.
const MUTATION_KEYWORDS: [&str; 10] = [
    "INSERT", "DELETE", "DROP", "CLEAR", "LOAD", "CREATE", "ADD", "MOVE", "COPY", "WITH",
];

/// The forms a read query may open with.
const READ_FORMS: [&str; 5] = ["SELECT", "ASK", "CONSTRUCT", "DESCRIBE", "PREFIX"];

/// Refuse a query that could change the graph.
///
/// Uppercased before the scan, so `insert` is caught as surely as `INSERT`. The check is
/// deliberately coarse: it is a *refusal* of anything ambiguous, and a query the kernel
/// itself generates can be read by a person to see it is a read.
pub fn is_read_only(query: &str) -> Result<(), String> {
    let upper = query.to_uppercase();
    for keyword in MUTATION_KEYWORDS {
        if upper.contains(&format!("{keyword} ")) || upper.contains(&format!("{keyword}(")) {
            return Err(format!("the query contains the update keyword {keyword}"));
        }
    }
    upper
        .split_whitespace()
        .find(|token| !token.starts_with('#') && !token.starts_with("BASE"))
        .map(|first| {
            let first = first.trim_start_matches('(');
            if READ_FORMS.contains(&first) {
                Ok(())
            } else {
                Err(format!(
                    "the query begins with {first}, which is not a read form ({})",
                    READ_FORMS.join(", ")
                ))
            }
        })
        .unwrap_or_else(|| Err("the query is empty".to_string()))
}

/// `graph.query`.
pub struct GraphQueryTool {
    spec: ToolSpec,
}

impl Default for GraphQueryTool {
    fn default() -> Self {
        GraphQueryTool::new()
    }
}

impl GraphQueryTool {
    /// The tool.
    pub fn new() -> Self {
        GraphQueryTool {
            spec: ToolSpec {
                name: ToolName::new("graph.query").expect("graph.query is a namespace.verb pair"),
                version: "0.1.0".to_string(),
                description: "Run a read-only SPARQL query against one of the kernel's named \
                              graphs."
                    .to_string(),
                input_schema: JsonSchema::object_with_strings(&["query"]),
                output_schema: JsonSchema::any_object(),
                permissions: vec![PermissionReq::new("graph:read:**", PermissionAction::Read)],
                reversibility: Reversibility::Reversible,
                side_effects: SideEffectClass::None,
                annotations: ToolAnnotations::pure(),
                module_uri: "https://metamind.dev/code/module/tools/graph-query".to_string(),
                sandbox_tier: SandboxTier::WasmCaps,
            },
        }
    }
}

#[async_trait]
impl Tool for GraphQueryTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(&self, ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
        let query = ctx.arg_str("query")?;
        let graph = ctx.opt_str("graph").unwrap_or_else(|| "world".to_string());
        if !mm_core::iri::is_named_graph(&graph) {
            return Err(ctx.bad_argument(
                "graph",
                format!("{graph:?} is not one of the kernel's named graphs"),
            ));
        }
        is_read_only(&query).map_err(|reason| ctx.bad_argument("query", reason))?;
        let rows = ctx
            .graph
            .sparql(&graph, &query)
            .await
            .map_err(|e| ctx.backing(format!("sparql failed: {e}")))?;
        let output = json!({
            "graph": graph,
            "count": rows.len(),
            "rows": rows,
        });
        let summary = format!("{graph} returned {} rows", rows.len());
        Ok(ctx.outcome(output, summary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ActionSpec;
    use crate::permissions::Principal;
    use crate::sandbox::{CapabilityGuard, SandboxCapabilities, DEFAULT_MAX_BYTES};
    use mm_core::{Timestamp, Ulid};

    async fn context(args: serde_json::Value) -> ExecCtx {
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let sql = mm_store_sqlite::SqliteStore::open(&dir.path().join("mm.db"))
            .await
            .unwrap();
        sql.migrate().await.unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("epistemic_shapes.ttl");
        let graph = Box::leak(Box::new(
            mm_store_graph::GraphStore::in_memory(&shapes)
                .await
                .unwrap(),
        ));
        let spec = GraphQueryTool::new().spec;
        let guard = CapabilityGuard::new(
            SandboxCapabilities::from_permissions(&spec.permissions, dir.path(), DEFAULT_MAX_BYTES),
            SandboxTier::WasmCaps,
            dir.path(),
        );
        ExecCtx {
            action_id: Ulid::from_parts(1_700_000_000_000, 7),
            trace_id: Ulid::from_parts(1_700_000_000_000, 8),
            action: ActionSpec::new(spec.name.clone(), args, Principal::system()),
            spec,
            guard,
            sql,
            graph: graph.handle().clone(),
            workdir: dir.path().to_path_buf(),
            started_at: Timestamp::now(),
            confirmed: false,
        }
    }

    #[tokio::test]
    async fn a_select_over_a_named_graph_returns_its_rows() {
        let tool = GraphQueryTool::new();
        // The store refuses a query that does not name its graph, so the caller scopes
        // it the way the tool's own error message asks: `GRAPH <…> { … }`.
        let iri = mm_core::iri::graph("epistemic");
        let ctx = context(json!({
            "query": format!(
                "SELECT ?s ?p ?o WHERE {{ GRAPH <{iri}> {{ ?s ?p ?o }} }} LIMIT 5"
            ),
            "graph": "epistemic",
        }))
        .await;
        let outcome = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap();
        assert_eq!(outcome.output["graph"], "epistemic");
        assert_eq!(outcome.output["count"], 0, "an empty graph has no rows");
        assert_eq!(outcome.output["rows"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn an_unknown_graph_is_refused() {
        let tool = GraphQueryTool::new();
        let ctx =
            context(json!({ "query": "SELECT * WHERE { ?s ?p ?o }", "graph": "elsewhere" })).await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert_eq!(error.code(), "tool.bad_arguments");
        assert!(error.to_string().contains("named graphs"));
    }

    #[tokio::test]
    async fn an_update_is_refused() {
        let tool = GraphQueryTool::new();
        let ctx = context(json!({
            "query": "INSERT DATA { <https://x/s> <https://x/p> \"v\" }",
            "graph": "tools",
        }))
        .await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert_eq!(error.code(), "tool.bad_arguments");
        assert!(error.to_string().contains("INSERT"));
    }

    #[test]
    fn read_only_detection_is_case_insensitive_and_prefix_tolerant() {
        assert!(is_read_only("SELECT ?s WHERE { ?s ?p ?o }").is_ok());
        assert!(is_read_only(
            "PREFIX mm: <https://metamind.dev/ontology#>\nSELECT ?s WHERE { ?s ?p ?o }"
        )
        .is_ok());
        assert!(is_read_only("ASK { ?s ?p ?o }").is_ok());
        assert!(is_read_only("select ?s where { ?s ?p ?o }").is_ok());
        assert!(is_read_only("DELETE WHERE { ?s ?p ?o }").is_err());
        assert!(is_read_only("CLEAR GRAPH <https://metamind.dev/graph/tools>").is_err());
        assert!(is_read_only("").is_err());
    }
}
