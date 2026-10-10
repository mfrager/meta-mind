//! The MCP bridge: `tools/list` and `tools/call` over JSON-RPC 2.0.
//!
//! MCP's own contract is small — a tool list, a call, a JSON-RPC envelope — and the bridge
//! keeps it that small deliberately. What MCP leaves to prose, this kernel decides
//! elsewhere: a call here goes through [`Executor`], so an MCP client reaches exactly the
//! same authorization, sandbox, idempotency and ledger path the CLI does. There is one
//! door, and the bridge is on the outside of it.
//!
//! # Two properties worth naming
//!
//! * **The principal is explicit.** An MCP request carries a principal in its `params`
//!   (a field this kernel adds to the call), and a request without one is refused. MCP
//!   itself has no notion of a caller identity, and a bridge that defaulted to the system
//!   principal would hand every connected client the kernel's own rights — the
//!   confused-deputy failure the phase's risk table names.
//! * **A refusal is a result, not a protocol error.** A denied call returns JSON-RPC
//!   `result` with `isError: true` and the denial reason, because the protocol call
//!   succeeded: the tool declined. Only a malformed envelope, an unknown method or an
//!   unknown tool is a JSON-RPC `error`.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::action::{ActionResult, ActionSpec};
use crate::error::McpError;
use crate::executor::Executor;
use crate::idempotency::IdempotencyKey;
use crate::permissions::Principal;
use crate::spec::ToolSpec;

/// The JSON-RPC version this bridge speaks.
pub const JSONRPC_VERSION: &str = "2.0";

/// The MCP protocol version the bridge reports.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// A JSON-RPC error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcErrorCode {
    /// The envelope is not a JSON-RPC 2.0 request.
    InvalidRequest = -32600,
    /// The method is not one the bridge serves.
    MethodNotFound = -32601,
    /// The params are not the shape the method takes.
    InvalidParams = -32602,
    /// The tool is not registered.
    UnknownTool = -32603,
}

/// The bridge.
#[derive(Debug)]
pub struct McpBridge {
    executor: Arc<Executor>,
    server: String,
}

impl McpBridge {
    /// A bridge over an executor, named as `server` in the records it writes.
    pub fn new(executor: Arc<Executor>, server: impl Into<String>) -> Self {
        McpBridge {
            executor,
            server: server.into(),
        }
    }

    /// The name this bridge reports itself as.
    pub fn server(&self) -> &str {
        &self.server
    }

    /// The `tools/list` result: every registered spec, in MCP's shape.
    ///
    /// The MCP fields come from the spec's own schema, and the kernel's extra declarations
    /// travel alongside them under `annotations`, so a client that only understands MCP
    /// still sees a valid tool description and a client that understands this kernel can
    /// see the reversibility and the required tier without a second call.
    pub fn tools_list(&self) -> Value {
        let tools: Vec<Value> = self
            .executor
            .registry()
            .list()
            .iter()
            .map(spec_to_mcp)
            .collect();
        json!({ "tools": tools })
    }

    /// Handle one JSON-RPC message.
    pub async fn handle(&self, request: &Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let reply =
            |result: Value| json!({ "jsonrpc": JSONRPC_VERSION, "id": id, "result": result });
        let fail = |code: RpcErrorCode, message: String| {
            json!({
                "jsonrpc": JSONRPC_VERSION,
                "id": id,
                "error": { "code": code as i32, "message": message },
            })
        };

        if request.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
            return fail(
                RpcErrorCode::InvalidRequest,
                "not a JSON-RPC 2.0 message".to_string(),
            );
        }
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            return fail(
                RpcErrorCode::InvalidRequest,
                "the message has no method".to_string(),
            );
        };
        match method {
            "tools/list" => reply(self.tools_list()),
            "tools/call" => {
                let Some(params) = request.get("params") else {
                    return fail(
                        RpcErrorCode::InvalidParams,
                        "tools/call needs params".to_string(),
                    );
                };
                let Some(name) = params.get("name").and_then(Value::as_str) else {
                    return fail(
                        RpcErrorCode::InvalidParams,
                        "tools/call needs a tool name".to_string(),
                    );
                };
                let Some(principal) = params.get("principal").and_then(Value::as_str) else {
                    return fail(
                        RpcErrorCode::InvalidParams,
                        "tools/call needs a principal: this bridge will not act as the kernel \
                         on a client's behalf"
                            .to_string(),
                    );
                };
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let key = params
                    .get("idempotencyKey")
                    .and_then(Value::as_str)
                    .map(|text| IdempotencyKey(text.to_string()));
                match self
                    .call_tool(name, args, Principal(principal.to_string()), key)
                    .await
                {
                    Ok(result) => reply(json!({
                        "content": [{ "type": "text", "text": crate::canonical_json(&serde_json::to_value(&result).unwrap_or(Value::Null)) }],
                        "isError": result.status != crate::action::ActionStatus::Ok,
                    })),
                    Err(error) => fail(RpcErrorCode::UnknownTool, error.to_string()),
                }
            }
            other => fail(
                RpcErrorCode::MethodNotFound,
                format!("unknown MCP method {other:?}"),
            ),
        }
    }

    /// Call a tool, through the executor.
    pub async fn call_tool(
        &self,
        name: &str,
        args: Value,
        principal: Principal,
        key: Option<IdempotencyKey>,
    ) -> Result<ActionResult, McpError> {
        let tool = crate::spec::ToolName::new(name)
            .map_err(|e| McpError::UnknownTool(format!("{name:?}: {e}")))?;
        if !self.executor.registry().contains(&tool) {
            return Err(McpError::UnknownTool(name.to_string()));
        }
        let mut action = ActionSpec::new(tool, args, principal);
        action.rationale = Some(format!("mcp:{}", self.server));
        self.executor
            .execute_with_key(action, key)
            .await
            .map_err(|e| McpError::Refused(e.to_string()))
    }

    /// Serve `tools/list` and `tools/call` on stdin/stdout, one JSON message per line.
    ///
    /// A transport failure ends the loop rather than panicking: the bridge owns no state,
    /// so a client that disconnects has left nothing behind.
    pub async fn serve_stdio(&self) -> Result<(), McpError> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let stdin = tokio::io::stdin();
        let mut lines = BufReader::new(stdin).lines();
        let mut stdout = tokio::io::stdout();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?
        {
            if line.trim().is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<Value>(&line) {
                Ok(request) => self.handle(&request).await,
                Err(error) => json!({
                    "jsonrpc": JSONRPC_VERSION,
                    "id": Value::Null,
                    "error": {
                        "code": RpcErrorCode::InvalidRequest as i32,
                        "message": format!("not JSON: {error}"),
                    },
                }),
            };
            let mut text = serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string());
            text.push('\n');
            stdout
                .write_all(text.as_bytes())
                .await
                .map_err(|e| McpError::Transport(e.to_string()))?;
            stdout
                .flush()
                .await
                .map_err(|e| McpError::Transport(e.to_string()))?;
        }
        Ok(())
    }

    /// Register an external MCP server's tool behind the same trait and the same policy.
    ///
    /// The spec is a copy of an external tool's *description*, and nothing about it is
    /// believed: it declares its capabilities like any other tool, it is authorized like
    /// any other tool, and its calls are recorded like any other tool's. What this build
    /// does not ship is the client that would speak the external protocol, so a call is
    /// refused with a transport error naming the server.
    pub fn ingest_remote(&self, spec: ToolSpec, client: McpClient) -> Result<(), McpError> {
        let spec = crate::spec::ToolSpec::checked(spec)
            .map_err(|e| McpError::BadRequest(format!("the remote spec is invalid: {e}")))?;
        self.executor
            .registry()
            .register(Arc::new(RemoteTool { spec, client }))
            .map_err(|e| McpError::BadRequest(e.to_string()))
    }
}

/// A client for an external MCP server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpClient {
    /// The server's endpoint.
    pub endpoint: String,
}

impl McpClient {
    /// A client for an endpoint.
    pub fn new(endpoint: impl Into<String>) -> Self {
        McpClient {
            endpoint: endpoint.into(),
        }
    }
}

/// An ingested external tool.
#[derive(Debug)]
pub struct RemoteTool {
    spec: ToolSpec,
    client: McpClient,
}

impl RemoteTool {
    /// The server it would talk to.
    pub fn client(&self) -> &McpClient {
        &self.client
    }
}

#[async_trait::async_trait]
impl crate::registry::Tool for RemoteTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(
        &self,
        ctx: &crate::registry::ExecCtx,
        _args: crate::registry::ToolArgs,
    ) -> Result<crate::registry::ToolOutcome, crate::error::ToolError> {
        // The capability checks still run first: an ingested tool with no grant fails as a
        // capability refusal, which is the more useful answer, and the transport refusal
        // only appears for a call that was authorized.
        ctx.check_subprocess()?;
        Err(crate::error::ToolError::Backing {
            tool: self.spec.name.clone(),
            reason: format!(
                "this build ships no MCP client, so {} cannot be reached",
                self.client.endpoint
            ),
        })
    }
}

/// Render one spec as an MCP tool description.
pub fn spec_to_mcp(spec: &ToolSpec) -> Value {
    json!({
        "name": spec.name.as_str(),
        "description": spec.description,
        "inputSchema": spec.input_schema.0,
        "outputSchema": spec.output_schema.0,
        "annotations": {
            "readOnlyHint": spec.annotations.read_only,
            "destructiveHint": spec.annotations.destructive,
            "idempotentHint": spec.annotations.idempotent,
            "openWorldHint": spec.annotations.open_world,
            "reversibility": spec.reversibility.as_str(),
            "sideEffectClass": spec.side_effects.as_str(),
            "sandboxTier": spec.sandbox_tier.as_str(),
            "version": spec.version,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger;
    use crate::permissions::{Grant, PermissionEngine, Principal, TablePermissionEngine};
    use crate::policy::{PolicyRule, PolicySet};
    use crate::tools;
    use mm_core::{Timestamp, UlidFactory};
    use mm_store_sqlite::SqliteStore;

    async fn bridge(
        root: &std::path::Path,
        sql: &SqliteStore,
        graph: mm_store_graph::GraphHandle,
    ) -> McpBridge {
        let registry = tools::default_registry().unwrap();
        let grants = vec![
            Grant::new(
                "01h0000000000000000000g900",
                Principal::system(),
                "fs:read:data/sandbox/**",
                "fs.*",
                "operator",
            ),
            Grant::new(
                "01h0000000000000000000g901",
                Principal::system(),
                "fs:write:data/sandbox/**",
                "fs.*",
                "operator",
            ),
        ];
        let sets = vec![PolicySet::new("01h0000000000000000000p900", "baseline", 1)
            .with_rule(PolicyRule::permit("*", "fs.*", "data/sandbox/**"))];
        let engine: Arc<dyn PermissionEngine> =
            Arc::new(TablePermissionEngine::new(grants, sets).pinned_at(Timestamp::now()));
        let executor = Executor::new(
            registry,
            engine,
            sql.clone(),
            graph,
            Arc::new(UlidFactory::new()),
        )
        .with_sandbox_root(root);
        McpBridge::new(Arc::new(executor), "test-server")
    }

    struct Fixture {
        dir: tempfile::TempDir,
        bridge: McpBridge,
        sql: SqliteStore,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("note.txt"), "hello").unwrap();
        let sql = SqliteStore::open(&dir.path().join("mm.db")).await.unwrap();
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
        let bridge = bridge(&root, &sql, graph.handle().clone()).await;
        Fixture { dir, bridge, sql }
    }

    #[tokio::test]
    async fn tools_list_is_mcp_shaped() {
        let fixture = fixture().await;
        let response = fixture
            .bridge
            .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
            .await;
        let tools = response["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|tool| tool["name"] == "fs.read"));
        let read = tools.iter().find(|tool| tool["name"] == "fs.read").unwrap();
        assert!(read["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "path"));
        assert_eq!(read["annotations"]["sandboxTier"], "WasmCaps");
    }

    #[tokio::test]
    async fn a_call_runs_through_the_executor_and_is_recorded() {
        let fixture = fixture().await;
        let path = fixture.dir.path().join("sandbox").join("note.txt");
        let response = fixture
            .bridge
            .handle(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "fs.read",
                    "principal": "system",
                    "arguments": { "path": path.to_string_lossy() },
                },
            }))
            .await;
        assert_eq!(response["result"]["isError"], false, "{response}");
        assert!(ledger::verify_chain(&fixture.sql).await.unwrap() > 0);
    }

    #[tokio::test]
    async fn a_call_without_a_principal_is_refused() {
        let fixture = fixture().await;
        let response = fixture
            .bridge
            .handle(&json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": { "name": "fs.read", "arguments": {} },
            }))
            .await;
        assert_eq!(
            response["error"]["code"],
            RpcErrorCode::InvalidParams as i32
        );
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("principal"));
    }

    #[tokio::test]
    async fn a_denied_call_is_a_result_with_is_error() {
        let fixture = fixture().await;
        let response = fixture
            .bridge
            .handle(&json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "process.exec",
                    "principal": "system",
                    "arguments": { "cmd": "cargo test" },
                },
            }))
            .await;
        assert_eq!(response["result"]["isError"], true, "{response}");
    }

    #[tokio::test]
    async fn an_unknown_method_and_an_unknown_tool_are_errors() {
        let fixture = fixture().await;
        let unknown_method = fixture
            .bridge
            .handle(&json!({ "jsonrpc": "2.0", "id": 5, "method": "resources/list" }))
            .await;
        assert_eq!(
            unknown_method["error"]["code"],
            RpcErrorCode::MethodNotFound as i32
        );

        let unknown_tool = fixture
            .bridge
            .handle(&json!({
                "jsonrpc": "2.0",
                "id": 6,
                "method": "tools/call",
                "params": { "name": "no.such", "principal": "system" },
            }))
            .await;
        assert_eq!(
            unknown_tool["error"]["code"],
            RpcErrorCode::UnknownTool as i32
        );

        let not_rpc = fixture
            .bridge
            .handle(&json!({ "method": "tools/list" }))
            .await;
        assert_eq!(
            not_rpc["error"]["code"],
            RpcErrorCode::InvalidRequest as i32
        );
    }

    #[tokio::test]
    async fn an_ingested_remote_tool_is_registered_behind_the_same_policy() {
        let fixture = fixture().await;
        let remote_spec = {
            use crate::registry::Tool as _;
            let mut spec = crate::tools::FsReadTool::new().spec().clone();
            spec.name = crate::spec::ToolName::new("remote.read").unwrap();
            spec.module_uri = "https://metamind.dev/code/module/tools/remote-read".into();
            spec.version = "9.9.9".into();
            // Keep the capability it declares, so the same grants apply to it.
            spec
        };
        fixture
            .bridge
            .ingest_remote(remote_spec, McpClient::new("https://elsewhere.example/mcp"))
            .unwrap();
        assert!(fixture
            .bridge
            .executor
            .registry()
            .contains(&crate::spec::ToolName::new("remote.read").unwrap()));

        // An ingested tool is authorized like any other: the seeded grants name `fs.*`,
        // and `remote.read` is not in that namespace, so the call is denied by the
        // permission engine before the missing transport is ever reached.
        let response = fixture
            .bridge
            .handle(&json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": { "name": "remote.read", "principal": "system", "arguments": { "path": "x" } },
            }))
            .await;
        assert_eq!(response["result"]["isError"], true, "{response}");
    }
}
