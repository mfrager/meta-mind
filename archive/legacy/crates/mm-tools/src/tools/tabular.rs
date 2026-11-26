//! `tabular.query`: read kernel tables with a read-only SQL statement.
//!
//! The kernel's tabular state is the projection of its event log, so a write through a
//! tool would be a second writer of history — the one thing the store's design refuses.
//! This tool therefore reads: the statement must be a single `SELECT` (or a `WITH` that
//! leads to one), and every table it names must be on the kernel's own list. A table
//! that is not on the list is refused even if it exists, because a tool that could read
//! any table would make the list decorative.
//!
//! `mm_core::Tabular::query_json` does the reading, so the tool shares the store's
//! mapping from rows to JSON objects rather than growing a second one.

use async_trait::async_trait;
use serde_json::json;

use crate::error::ToolError;
use crate::permissions::{PermissionAction, PermissionReq};
use crate::registry::{ExecCtx, Tool, ToolArgs, ToolOutcome};
use crate::sandbox::SandboxTier;
use crate::spec::{
    JsonSchema, Reversibility, SideEffectClass, ToolAnnotations, ToolName, ToolSpec,
};

/// The tables a tool may read.
///
/// Listed rather than derived from `sqlite_master`, so a table added by a later phase is
/// invisible here until someone decides it should be readable — which is the same
/// default-deny stance the rest of the phase takes.
pub const READABLE_TABLES: [&str; 12] = [
    "facts",
    "claims",
    "evidence",
    "assumptions",
    "decisions",
    "episodes",
    "programs",
    "library_entries",
    "tool_registry",
    "tool_calls",
    "action_ledger",
    "idempotency_keys",
];

/// The keywords that would change state.
const WRITE_KEYWORDS: [&str; 9] = [
    "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "REPLACE", "ATTACH", "PRAGMA",
];

/// Refuse a statement that is not a read of an allowed table.
///
/// The table scan is a token walk: it collects the word after each `FROM` and `JOIN`
/// and checks it against [`READABLE_TABLES`]. Subqueries are handled because they use the
/// same keywords, so `SELECT * FROM (SELECT * FROM claims)` sees `claims`.
pub fn check_query(statement: &str) -> Result<(), String> {
    let upper = statement.to_uppercase();
    if !upper.trim_start().starts_with("SELECT") && !upper.trim_start().starts_with("WITH") {
        return Err("only SELECT (or a WITH leading to one) is allowed".to_string());
    }
    for keyword in WRITE_KEYWORDS {
        if upper.contains(&format!("{keyword} ")) || upper.contains(&format!("{keyword}(")) {
            return Err(format!(
                "the statement contains the write keyword {keyword}"
            ));
        }
    }
    if upper.contains(';') && upper.trim().trim_end_matches(';').contains(';') {
        return Err("only one statement may be run at a time".to_string());
    }
    let tokens: Vec<&str> = upper.split_whitespace().collect();
    // A common table expression names a table that is not on the list, because it is not
    // a table: it is this statement's own intermediate result. `WITH x AS (...)` makes
    // `x` readable for the rest of the statement and for nothing else.
    let cte_names: Vec<&str> = tokens
        .windows(2)
        .filter(|window| window[1] == "AS")
        .map(|window| window[0].trim_matches(|c: char| c == '(' || c == ','))
        .collect();
    for (index, token) in tokens.iter().enumerate() {
        if *token == "FROM" || *token == "JOIN" {
            let Some(next) = tokens.get(index + 1) else {
                return Err("the statement ends with FROM".to_string());
            };
            let name = next
                .trim_matches(|c: char| c == '(' || c == ')' || c == ',')
                .trim_start_matches("MAIN.");
            if name.starts_with('(') || name == "SELECT" {
                continue;
            }
            // The token walk uppercases the statement, so the list is matched
            // case-insensitively: SQL table names are not case sensitive, and a caller
            // that writes `FROM tool_registry` or `FROM TOOL_REGISTRY` means the same
            // table.
            let readable = READABLE_TABLES.iter().any(|t| t.eq_ignore_ascii_case(name));
            if !readable && !cte_names.contains(&name) {
                return Err(format!(
                    "table {name:?} is not readable; the readable tables are {}",
                    READABLE_TABLES.join(", ")
                ));
            }
        }
    }
    Ok(())
}

/// `tabular.query`.
pub struct TabularQueryTool {
    spec: ToolSpec,
}

impl Default for TabularQueryTool {
    fn default() -> Self {
        TabularQueryTool::new()
    }
}

impl TabularQueryTool {
    /// The tool.
    pub fn new() -> Self {
        TabularQueryTool {
            spec: ToolSpec {
                name: ToolName::new("tabular.query")
                    .expect("tabular.query is a namespace.verb pair"),
                version: "0.1.0".to_string(),
                description: "Read kernel tables with a single read-only SQL statement."
                    .to_string(),
                input_schema: JsonSchema::object_with_strings(&["query"]),
                output_schema: JsonSchema::any_object(),
                permissions: vec![PermissionReq::new("sql:read:**", PermissionAction::Read)],
                reversibility: Reversibility::Reversible,
                side_effects: SideEffectClass::None,
                annotations: ToolAnnotations::pure(),
                module_uri: "https://metamind.dev/code/module/tools/tabular-query".to_string(),
                sandbox_tier: SandboxTier::WasmCaps,
            },
        }
    }
}

#[async_trait]
impl Tool for TabularQueryTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(&self, ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
        let query = ctx.arg_str("query")?;
        check_query(&query).map_err(|reason| ctx.bad_argument("query", reason))?;
        let rows = mm_core::Tabular::query_json(&ctx.sql, &query, Vec::new())
            .await
            .map_err(|e| ctx.backing(format!("query failed: {e}")))?;
        let output = json!({ "count": rows.len(), "rows": rows });
        let summary = format!("read {} rows", rows.len());
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
        let spec = TabularQueryTool::new().spec;
        let guard = CapabilityGuard::new(
            SandboxCapabilities::from_permissions(&spec.permissions, dir.path(), DEFAULT_MAX_BYTES),
            SandboxTier::WasmCaps,
            dir.path(),
        );
        ExecCtx {
            action_id: Ulid::from_parts(1_700_000_000_000, 9),
            trace_id: Ulid::from_parts(1_700_000_000_000, 10),
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
    async fn an_allowed_table_reads_back() {
        let tool = TabularQueryTool::new();
        let ctx = context(json!({ "query": "SELECT name, sandbox_tier FROM tool_registry" })).await;
        let outcome = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap();
        assert_eq!(outcome.output["count"], 0);
        assert!(outcome.output["rows"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_table_outside_the_list_is_refused() {
        let tool = TabularQueryTool::new();
        let ctx = context(json!({ "query": "SELECT * FROM sqlite_master" })).await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert!(error.to_string().contains("not readable"), "{error}");
    }

    #[tokio::test]
    async fn a_write_is_refused() {
        let tool = TabularQueryTool::new();
        let ctx = context(json!({ "query": "DELETE FROM tool_calls" })).await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert_eq!(error.code(), "tool.bad_arguments");
    }

    #[test]
    fn the_query_check_names_the_problem() {
        assert!(check_query("SELECT 1").is_ok());
        assert!(check_query("WITH x AS (SELECT id FROM claims) SELECT * FROM x").is_ok());
        assert!(check_query("UPDATE claims SET status = 'X'")
            .unwrap_err()
            .contains("only SELECT"));
        assert!(check_query("SELECT * FROM claims; DROP TABLE claims")
            .unwrap_err()
            .contains("DROP"));
        assert!(check_query("SELECT * FROM claims; SELECT * FROM evidence")
            .unwrap_err()
            .contains("one statement"));
        assert!(check_query("SELECT * FROM secrets")
            .unwrap_err()
            .contains("not readable"));
        assert!(check_query("SELECT * FROM FROM").is_err());
    }
}
