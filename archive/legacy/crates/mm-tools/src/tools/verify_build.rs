//! `verify.run`: answer a verification obligation and return its proof.
//!
//! The tool is thin on purpose: it parses two strings into an
//! [`Obligation`](crate::verify::Obligation), hands it to
//! [`run_obligation`](crate::verify::run_obligation), and reports the
//! [`Proof`](crate::verify::Proof). Every decision about what "verified" means lives in
//! `crate::verify`, so the CLI and a tool call cannot disagree about it.
//!
//! A refuted obligation is **not** an error. The tool ran, and the thing it checked
//! failed: `ToolError` is for the call failing, and a build that does not compile is a
//! successful call that returned `refuted`. The exit code a caller sees is the CLI's
//! decision to map `refuted` onto a non-zero status, which is where that belongs.

use async_trait::async_trait;
use serde_json::json;

use crate::error::ToolError;
use crate::permissions::{PermissionAction, PermissionReq};
use crate::registry::{ExecCtx, Tool, ToolArgs, ToolOutcome};
use crate::sandbox::SandboxTier;
use crate::spec::{
    JsonSchema, Reversibility, SideEffectClass, ToolAnnotations, ToolName, ToolSpec,
};
use crate::verify::{obligation_for, run_obligation, ObligationKind, OBLIGATION_KINDS};

/// The tolerance a residual check uses when the caller does not name one. A nanounit: a
/// residual larger than this is a real disagreement rather than floating-point noise.
pub const DEFAULT_TOLERANCE: f64 = 1e-9;

/// `verify.run`.
pub struct VerifyRunTool {
    spec: ToolSpec,
}

impl Default for VerifyRunTool {
    fn default() -> Self {
        VerifyRunTool::new()
    }
}

impl VerifyRunTool {
    /// The tool.
    pub fn new() -> Self {
        VerifyRunTool {
            spec: ToolSpec {
                name: ToolName::new("verify.run").expect("verify.run is a namespace.verb pair"),
                version: "0.1.0".to_string(),
                description: "Answer a verification obligation — a build, a test run, a lint \
                              pass, a symbolic check or a residual check — and return its proof."
                    .to_string(),
                input_schema: JsonSchema::object_with_strings(&["obligation", "subject"]),
                output_schema: JsonSchema::any_object(),
                permissions: vec![
                    // The command-backed obligations run cargo; the file-backed ones read a
                    // subject out of the sandbox.
                    PermissionReq::new("process:execute:cargo", PermissionAction::Execute),
                    PermissionReq::new("fs:read:data/sandbox/**", PermissionAction::Read),
                ],
                reversibility: Reversibility::Reversible,
                side_effects: SideEffectClass::Local,
                annotations: ToolAnnotations {
                    read_only: false,
                    destructive: false,
                    idempotent: true,
                    open_world: false,
                },
                module_uri: "https://metamind.dev/code/module/tools/verify-build".to_string(),
                sandbox_tier: SandboxTier::WasmCaps,
            },
        }
    }
}

/// The kinds a caller may name, for `mm-cli tool describe`.
pub fn obligation_names() -> Vec<&'static str> {
    OBLIGATION_KINDS
        .into_iter()
        .map(ObligationKind::as_str)
        .collect()
}

#[async_trait]
impl Tool for VerifyRunTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(&self, ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
        let kind = ctx.arg_str("obligation")?;
        let subject = ctx.arg_str("subject")?;
        let Some(obligation) = obligation_for(&kind, &subject) else {
            return Err(ctx.bad_argument(
                "obligation",
                format!(
                    "{kind:?} is not an obligation; the kinds are {}",
                    obligation_names().join(", ")
                ),
            ));
        };
        let tolerance = ctx
            .args()
            .get("tolerance")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(DEFAULT_TOLERANCE);
        let proof = run_obligation(ctx, &obligation, tolerance).await?;
        let output = json!({
            "obligation": proof.obligation.canonical(),
            "verdict": proof.verdict.as_str(),
            "detail": proof.detail,
            "counterexample": proof.counterexample,
            "artifact_hash": proof.artifact_hash,
            "proof_hash": proof.hash(),
        });
        let summary = format!(
            "{} was {} ({})",
            proof.obligation.canonical(),
            proof.verdict.as_str(),
            proof.detail
        );
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
        let spec = VerifyRunTool::new().spec;
        let guard = CapabilityGuard::new(
            SandboxCapabilities::from_permissions(&spec.permissions, root, DEFAULT_MAX_BYTES),
            SandboxTier::WasmCaps,
            root,
        );
        ExecCtx {
            action_id: Ulid::from_parts(1_700_000_000_000, 13),
            trace_id: Ulid::from_parts(1_700_000_000_000, 14),
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
    async fn a_seeded_inconsistency_is_refuted_with_a_counterexample() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let subject = root.join("seed.logic");
        std::fs::write(&subject, "p(a)\nnot p(a)\n").unwrap();
        let tool = VerifyRunTool::new();
        let ctx = context(
            &root,
            json!({ "obligation": "symbolic", "subject": subject.to_string_lossy() }),
        )
        .await;
        let outcome = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap();
        assert_eq!(outcome.output["verdict"], "refuted");
        assert!(outcome.output["counterexample"].is_string());
        assert!(outcome.output["proof_hash"].is_string());
    }

    #[tokio::test]
    async fn a_consistent_subject_is_inconclusive_rather_than_proven() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let subject = root.join("ok.logic");
        std::fs::write(&subject, "p(a)\nq(b)\n").unwrap();
        let tool = VerifyRunTool::new();
        let ctx = context(
            &root,
            json!({ "obligation": "symbolic", "subject": subject.to_string_lossy() }),
        )
        .await;
        let outcome = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap();
        assert_eq!(outcome.output["verdict"], "inconclusive");
    }

    #[tokio::test]
    async fn a_command_obligation_is_refused_without_a_process_capability() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let tool = VerifyRunTool::new();
        // The declared capabilities are filed under the sandbox root; the guard here is
        // built from an empty list, which is what a caller with no grant gets.
        let ctx = context(
            &root,
            json!({ "obligation": "cargo-build", "subject": "mm-tools" }),
        )
        .await;
        let mut no_process = ctx.clone();
        no_process.guard = CapabilityGuard::new(
            SandboxCapabilities::from_permissions(
                &[PermissionReq::new(
                    "fs:read:data/sandbox/**",
                    PermissionAction::Read,
                )],
                &root,
                DEFAULT_MAX_BYTES,
            ),
            SandboxTier::WasmCaps,
            &root,
        );
        let error = tool
            .invoke(&no_process, ToolArgs::empty())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("subprocess"), "{error}");
    }

    #[tokio::test]
    async fn an_unknown_obligation_is_a_bad_argument() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sandbox");
        std::fs::create_dir_all(&root).unwrap();
        let tool = VerifyRunTool::new();
        let ctx = context(&root, json!({ "obligation": "guess", "subject": "x" })).await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert_eq!(error.code(), "tool.bad_arguments");
        assert!(error.to_string().contains("symbolic"));
    }

    #[test]
    fn the_kinds_are_all_listed() {
        assert_eq!(obligation_names().len(), 5);
        assert!(obligation_names().contains(&"math-residual"));
        VerifyRunTool::new().spec.validate().unwrap();
    }
}
