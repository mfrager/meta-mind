//! `pi_editor.open`: the PI editor's spec and permission profile, without its client.
//!
//! The phase plan puts the PI editor implementation out of scope ("Phase 10 ships only its
//! spec + permission profile; Phase 11 wires the RPC client") and §5.8 asks for it as a
//! **stub**. A stub is written as a tool that refuses rather than as a tool that is
//! missing, for the same reason [`crate::registry::PersistedTool`] exists: an operator
//! who reads `mm-cli tool list` should see what is installed, and a call should say why
//! it cannot proceed.
//!
//! What the stub does carry is the part that is a *decision* rather than an
//! implementation: the tier it needs, the capabilities it asks for, and its
//! reversibility. It is [`Reversibility::Irreversible`] because editing a live editor
//! buffer has no snapshot that restores it, and it runs in [`SandboxTier::MicroVm`]
//! because it drives another process over a socket. Both are visible in
//! `mm-cli tool describe pi_editor.open`, so the profile a Phase 11 implementation must
//! honour is already recorded and already enforced: a call today is refused for its tier
//! *and* for its irreversibility, in that order, whichever the machine can decide first.

use async_trait::async_trait;

use crate::error::ToolError;
use crate::permissions::{PermissionAction, PermissionReq};
use crate::registry::{ExecCtx, Tool, ToolArgs, ToolOutcome};
use crate::sandbox::SandboxTier;
use crate::spec::{
    JsonSchema, Reversibility, SideEffectClass, ToolAnnotations, ToolName, ToolSpec,
};

/// `pi_editor.open`.
pub struct PiEditorTool {
    spec: ToolSpec,
}

impl Default for PiEditorTool {
    fn default() -> Self {
        PiEditorTool::new()
    }
}

impl PiEditorTool {
    /// The tool.
    pub fn new() -> Self {
        PiEditorTool {
            spec: ToolSpec {
                name: ToolName::new("pi_editor.open")
                    .expect("pi_editor.open is a namespace.verb pair"),
                version: "0.1.0".to_string(),
                description: "Open a path in the PI editor. Phase 10 ships the spec and the \
                              permission profile; the RPC client arrives in Phase 11."
                    .to_string(),
                input_schema: JsonSchema::object_with_strings(&["path"]),
                output_schema: JsonSchema::any_object(),
                permissions: vec![
                    PermissionReq::new("fs:read:data/sandbox/**", PermissionAction::Read),
                    PermissionReq::new("process:execute:pi", PermissionAction::Execute),
                ],
                reversibility: Reversibility::Irreversible,
                side_effects: SideEffectClass::External,
                annotations: ToolAnnotations {
                    read_only: false,
                    destructive: true,
                    idempotent: false,
                    open_world: true,
                },
                module_uri: "https://metamind.dev/code/module/tools/pi-editor".to_string(),
                sandbox_tier: SandboxTier::MicroVm,
            },
        }
    }
}

#[async_trait]
impl Tool for PiEditorTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(&self, ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
        // The path is still resolved through the guard before the refusal, so a caller
        // that asked for something outside the sandbox is told *that* rather than being
        // told the editor is unimplemented. The more specific refusal is the useful one.
        let requested = ctx.arg_str("path")?;
        let _resolved = ctx.read_path(&requested)?;
        Err(ctx.unsupported(
            "Phase 11 wires the PI editor's RPC client; this build ships the tool's spec and \
             its permission profile only, and runs it in a microVM tier this build cannot \
             provide",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ActionSpec;
    use crate::permissions::Principal;
    use crate::sandbox::{CapabilityGuard, SandboxCapabilities, DEFAULT_MAX_BYTES};
    use mm_core::{Timestamp, Ulid};

    async fn context(root: &std::path::Path, args: serde_json::Value) -> ExecCtx {
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
        let spec = PiEditorTool::new().spec;
        let guard = CapabilityGuard::new(
            SandboxCapabilities::from_permissions(&spec.permissions, root, DEFAULT_MAX_BYTES),
            spec.sandbox_tier,
            root,
        );
        ExecCtx {
            action_id: Ulid::from_parts(1_700_000_000_000, 15),
            trace_id: Ulid::from_parts(1_700_000_000_000, 16),
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
    async fn the_stub_refuses_and_says_where_the_implementation_is() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("note.txt");
        std::fs::write(&target, "x").unwrap();
        let tool = PiEditorTool::new();
        let ctx = context(
            &root,
            serde_json::json!({ "path": target.to_string_lossy() }),
        )
        .await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert_eq!(error.code(), "tool.unsupported");
        assert!(error.to_string().contains("Phase 11"));
    }

    #[tokio::test]
    async fn the_stub_still_refuses_a_path_outside_the_sandbox_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let outside = dir.path().join("secret.txt");
        std::fs::write(&outside, "s").unwrap();
        let tool = PiEditorTool::new();
        let ctx = context(
            &root,
            serde_json::json!({ "path": outside.to_string_lossy() }),
        )
        .await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert!(error.is_sandbox_denial(), "{error}");
    }

    #[test]
    fn the_profile_is_the_one_the_plan_promises() {
        let spec = PiEditorTool::new().spec;
        spec.validate().unwrap();
        assert_eq!(spec.sandbox_tier, SandboxTier::MicroVm);
        assert_eq!(spec.reversibility, Reversibility::Irreversible);
        assert_eq!(
            crate::sandbox::tier_available(spec.sandbox_tier)
                .unwrap_err()
                .code(),
            "sandbox.tier_unavailable"
        );
    }
}
