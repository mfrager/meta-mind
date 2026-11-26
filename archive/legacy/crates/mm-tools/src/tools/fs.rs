//! The filesystem tools: read, write, list, all through the capability guard.
//!
//! Three tools rather than one `fs` tool with a verb argument, because the verb is what
//! a grant and a policy rule name: a single tool would need `fs.write` and `fs.read` to
//! be told apart from its arguments, and a `tool_pattern` of `fs.*` would then cover
//! both. Splitting them makes the capability a property of the *tool*, which is what
//! the registry, the policy rules and the ledger all key on.
//!
//! Every path goes through [`ExecCtx::read_path`] or [`ExecCtx::write_path`]. There is
//! no `std::fs` call in this file that is not handed a path one of those returned, which
//! is what makes "undeclared access is a hard error" a property of the code rather than
//! of the reviewer's attention.
//!
//! # Writes are atomic
//!
//! [`FsWriteTool`] writes to a sibling temporary file and renames it over the target.
//! A direct write truncates first, so a crash mid-write leaves a half file — and a
//! rollback would then restore bytes the action never finished writing, which is worse
//! than leaving the original in place.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::json;

use crate::error::ToolError;
use crate::permissions::{PermissionAction, PermissionReq};
use crate::registry::{ExecCtx, Tool, ToolArgs, ToolOutcome};
use crate::sandbox::SandboxTier;
use crate::spec::{
    JsonSchema, Reversibility, SideEffectClass, ToolAnnotations, ToolName, ToolSpec,
};

/// The version every tool in this module is published at.
const VERSION: &str = "0.1.0";

/// The module IRI every tool in this file is implemented in.
fn module_uri(name: &str) -> String {
    format!("https://metamind.dev/code/module/tools/{name}")
}

fn tool_name(name: &str) -> ToolName {
    ToolName::new(name).expect("the names in this module are namespace.verb pairs")
}

fn read_permission() -> PermissionReq {
    PermissionReq::new("fs:read:data/sandbox/**", PermissionAction::Read)
}

fn write_permission() -> PermissionReq {
    PermissionReq::new("fs:write:data/sandbox/**", PermissionAction::Write)
}

/// `fs.read`.
pub struct FsReadTool {
    spec: ToolSpec,
}

impl Default for FsReadTool {
    fn default() -> Self {
        FsReadTool::new()
    }
}

impl FsReadTool {
    /// The tool.
    pub fn new() -> Self {
        FsReadTool {
            spec: ToolSpec {
                name: tool_name("fs.read"),
                version: VERSION.to_string(),
                description: "Read a file inside the sandbox and return its bytes, hash and \
                              text when it is UTF-8."
                    .to_string(),
                input_schema: JsonSchema::object_with_strings(&["path"]),
                output_schema: JsonSchema::any_object(),
                permissions: vec![read_permission()],
                reversibility: Reversibility::Reversible,
                side_effects: SideEffectClass::None,
                annotations: ToolAnnotations::pure(),
                module_uri: module_uri("fs-read"),
                sandbox_tier: SandboxTier::WasmCaps,
            },
        }
    }
}

#[async_trait]
impl Tool for FsReadTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(&self, ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
        let requested = ctx.arg_str("path")?;
        let resolved = ctx.read_path(&requested)?;
        let bytes = std::fs::read(&resolved)
            .map_err(|e| ctx.failed(format!("cannot read {}: {e}", resolved.display())))?;
        ctx.check_size(bytes.len() as u64)?;
        if let Some(cap) = ctx.opt_u64("max_bytes")? {
            if bytes.len() as u64 > cap {
                return Err(ctx.failed(format!(
                    "{} is {} bytes, above the requested ceiling of {cap}",
                    resolved.display(),
                    bytes.len()
                )));
            }
        }
        let hash = mm_core::content_hash(&bytes);
        let text = String::from_utf8(bytes.clone()).ok();
        let encoding = if text.is_some() { "utf8" } else { "binary" };
        let mut output = json!({
            "path": resolved.display().to_string(),
            "bytes": bytes.len(),
            "sha256": hash,
            "encoding": encoding,
        });
        if let Some(text) = &text {
            output["content"] = json!(text);
        }
        let summary = format!("read {} bytes from {}", bytes.len(), resolved.display());
        Ok(ctx.outcome(output, summary))
    }
}

/// `fs.write`.
pub struct FsWriteTool {
    spec: ToolSpec,
}

impl Default for FsWriteTool {
    fn default() -> Self {
        FsWriteTool::new()
    }
}

impl FsWriteTool {
    /// The tool.
    pub fn new() -> Self {
        FsWriteTool {
            spec: ToolSpec {
                name: tool_name("fs.write"),
                version: VERSION.to_string(),
                description: "Write text to a file inside the sandbox, atomically, and report \
                              what it wrote."
                    .to_string(),
                input_schema: JsonSchema::object_with_strings(&["path", "text"]),
                output_schema: JsonSchema::any_object(),
                permissions: vec![write_permission(), read_permission()],
                reversibility: Reversibility::Reversible,
                side_effects: SideEffectClass::Local,
                annotations: ToolAnnotations {
                    read_only: false,
                    destructive: false,
                    idempotent: true,
                    open_world: false,
                },
                module_uri: module_uri("fs-write"),
                sandbox_tier: SandboxTier::WasmCaps,
            },
        }
    }
}

#[async_trait]
impl Tool for FsWriteTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(&self, ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
        let requested = ctx.arg_str("path")?;
        let text = ctx.arg_str("text")?;
        let resolved = ctx.write_path(&requested)?;
        ctx.check_size(text.len() as u64)?;
        let created = !resolved.exists();
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ctx.failed(format!("cannot create {}: {e}", parent.display())))?;
        }
        let temporary = sibling_temporary(&resolved);
        std::fs::write(&temporary, text.as_bytes())
            .map_err(|e| ctx.failed(format!("cannot write {}: {e}", temporary.display())))?;
        std::fs::rename(&temporary, &resolved).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            ctx.failed(format!("cannot replace {}: {e}", resolved.display()))
        })?;
        let hash = mm_core::content_hash(text.as_bytes());
        let output = json!({
            "path": resolved.display().to_string(),
            "bytes": text.len(),
            "sha256": hash,
            "created": created,
        });
        let summary = format!("wrote {} bytes to {}", text.len(), resolved.display());
        Ok(ctx
            .outcome(output, summary)
            .touching(vec![resolved.display().to_string()]))
    }
}

/// `fs.list`.
pub struct FsListTool {
    spec: ToolSpec,
}

impl Default for FsListTool {
    fn default() -> Self {
        FsListTool::new()
    }
}

impl FsListTool {
    /// The tool.
    pub fn new() -> Self {
        FsListTool {
            spec: ToolSpec {
                name: tool_name("fs.list"),
                version: VERSION.to_string(),
                description: "List the entries of a directory inside the sandbox, sorted by name."
                    .to_string(),
                input_schema: JsonSchema::object(&["dir"]),
                output_schema: JsonSchema::any_object(),
                permissions: vec![read_permission()],
                reversibility: Reversibility::Reversible,
                side_effects: SideEffectClass::None,
                annotations: ToolAnnotations::pure(),
                module_uri: module_uri("fs-list"),
                sandbox_tier: SandboxTier::WasmCaps,
            },
        }
    }
}

#[async_trait]
impl Tool for FsListTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(&self, ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
        let requested = ctx.opt_str("dir").unwrap_or_else(|| ".".to_string());
        let resolved = ctx.read_path(&requested)?;
        let listing = std::fs::read_dir(&resolved)
            .map_err(|e| ctx.failed(format!("cannot list {}: {e}", resolved.display())))?;
        let mut entries: Vec<(String, bool, u64)> = Vec::new();
        for entry in listing {
            let entry =
                entry.map_err(|e| ctx.failed(format!("cannot read a directory entry: {e}")))?;
            let metadata = entry
                .metadata()
                .map_err(|e| ctx.failed(format!("cannot stat {}: {e}", entry.path().display())))?;
            entries.push((
                entry.file_name().to_string_lossy().to_string(),
                metadata.is_dir(),
                metadata.len(),
            ));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let items: Vec<serde_json::Value> = entries
            .iter()
            .map(|(name, is_dir, bytes)| json!({ "name": name, "is_dir": is_dir, "bytes": bytes }))
            .collect();
        let output = json!({
            "dir": resolved.display().to_string(),
            "count": items.len(),
            "entries": items,
        });
        let summary = format!("listed {} entries in {}", entries.len(), resolved.display());
        Ok(ctx.outcome(output, summary))
    }
}

/// The sibling temporary path an atomic write goes through.
fn sibling_temporary(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "mm".to_string());
    let mut temporary = target.to_path_buf();
    temporary.set_file_name(format!("{name}.mm-tmp"));
    temporary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ActionSpec;
    use crate::permissions::Principal;
    use crate::sandbox::{CapabilityGuard, SandboxCapabilities, DEFAULT_MAX_BYTES};
    use mm_core::{Timestamp, Ulid};

    /// A context over a throwaway store, an in-memory graph and a real sandbox root.
    async fn harness(tool: &dyn Tool, root: &Path, args: serde_json::Value) -> ExecCtx {
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
        let spec = tool.spec().clone();
        let guard = CapabilityGuard::new(
            SandboxCapabilities::from_permissions(&spec.permissions, root, DEFAULT_MAX_BYTES),
            spec.sandbox_tier,
            root,
        );
        ExecCtx {
            action_id: Ulid::from_parts(1_700_000_000_000, 3),
            trace_id: Ulid::from_parts(1_700_000_000_000, 4),
            action: ActionSpec::new(spec.name.clone(), args, Principal::system()),
            spec,
            guard,
            sql,
            graph: graph.handle().clone(),
            workdir: root.to_path_buf(),
            started_at: Timestamp::now(),
            confirmed: false,
        }
    }

    #[tokio::test]
    async fn read_returns_the_bytes_and_the_hash() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("note.txt"), "hello").unwrap();
        let tool = FsReadTool::new();
        let ctx = harness(
            &tool,
            &root,
            json!({ "path": root.join("note.txt").to_string_lossy() }),
        )
        .await;
        let outcome = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap();
        assert_eq!(outcome.output["bytes"], 5);
        assert_eq!(outcome.output["content"], "hello");
        assert_eq!(outcome.output["encoding"], "utf8");
        assert_eq!(outcome.output["sha256"], mm_core::content_hash(b"hello"));
    }

    #[tokio::test]
    async fn read_refuses_a_path_outside_the_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let tool = FsReadTool::new();
        let ctx = harness(&tool, &root, json!({ "path": secret.to_string_lossy() })).await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert!(error.is_sandbox_denial(), "{error}");
        assert!(
            matches!(error.code(), "tool.sandbox"),
            "the sandbox names the refusal: {error}"
        );
        assert!(error.to_string().contains("escape") || error.to_string().contains("denied"));
    }

    #[tokio::test]
    async fn write_replaces_atomically_and_reports_the_path_it_touched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("out.txt");
        std::fs::write(&target, "old").unwrap();
        let tool = FsWriteTool::new();
        let ctx = harness(
            &tool,
            &root,
            json!({ "path": target.to_string_lossy(), "text": "new" }),
        )
        .await;
        let outcome = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(outcome.output["created"], false);
        assert_eq!(outcome.output["bytes"], 3);
        assert_eq!(outcome.touched_paths.len(), 1);
        assert!(
            !root.join("out.txt.mm-tmp").exists(),
            "the temporary is gone"
        );
    }

    #[tokio::test]
    async fn write_refuses_a_path_outside_the_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let tool = FsWriteTool::new();
        let outside = dir.path().join("outside.txt");
        let ctx = harness(
            &tool,
            &root,
            json!({ "path": outside.to_string_lossy(), "text": "nope" }),
        )
        .await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert!(error.is_sandbox_denial(), "{error}");
        assert!(!outside.exists());
    }

    #[tokio::test]
    async fn write_refuses_a_payload_above_the_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let tool = FsWriteTool::new();
        let ctx = harness(
            &tool,
            &root,
            json!({
                "path": root.join("big.txt").to_string_lossy(),
                "text": "x".repeat(DEFAULT_MAX_BYTES as usize + 1),
            }),
        )
        .await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert!(error.to_string().contains("exceeds"), "{error}");
    }

    #[tokio::test]
    async fn list_is_sorted_and_reports_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("b.txt"), "b").unwrap();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        let tool = FsListTool::new();
        let ctx = harness(&tool, &root, json!({ "dir": root.to_string_lossy() })).await;
        let outcome = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap();
        assert_eq!(outcome.output["count"], 3);
        let names: Vec<String> = outcome.output["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "nested"]);
        assert_eq!(outcome.output["entries"][2]["is_dir"], true);
    }

    #[tokio::test]
    async fn a_missing_argument_is_a_bad_argument_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let tool = FsReadTool::new();
        let ctx = harness(&tool, &root, json!({})).await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert_eq!(error.code(), "tool.bad_arguments");
    }

    #[test]
    fn every_tool_declares_a_valid_spec() {
        for spec in [
            FsReadTool::new().spec,
            FsWriteTool::new().spec,
            FsListTool::new().spec,
        ] {
            spec.validate().unwrap();
            assert_eq!(spec.sandbox_tier, SandboxTier::WasmCaps);
        }
        assert!(FsReadTool::new().spec.is_pure());
        assert!(!FsWriteTool::new().spec.is_pure());
    }

    #[test]
    fn the_temporary_is_a_sibling() {
        let temporary = sibling_temporary(Path::new("/tmp/sandbox/out.txt"));
        assert_eq!(temporary, PathBuf::from("/tmp/sandbox/out.txt.mm-tmp"));
    }
}
