//! `process.exec`: run a command, under a command-scoped capability.
//!
//! Spawning a process is the loudest capability in the kernel, so it is guarded twice
//! and in two different directions. [`ExecCtx::check_subprocess`] answers "may this
//! caller run anything at all", and [`ExecCtx::check_command`] answers "is *this*
//! command the one it was granted" by matching the command against the pattern the
//! authorizing grant carried. A grant on `cargo` therefore cannot run `rm`, which is
//! the difference between a process capability and a shell.
//!
//! # No shell
//!
//! The command is split on whitespace and executed directly, never through `sh -c`.
//! That removes quoting, redirection, pipes and substitution in one decision: a shell
//! would turn *any* granted pattern into arbitrary execution, because `cargo build`
//! followed by `; rm -rf /` still starts with `cargo`. A caller that needs a pipeline
//! can ask for it explicitly in a later phase, where it can be granted explicitly too.
//!
//! # Timeout
//!
//! Every run has a deadline, defaulting to [`DEFAULT_TIMEOUT_MS`]. A tool call that
//! never returns is not an outcome the ledger can record, and without a deadline an
//! intermittent hang would hold a budget dimension open forever.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use crate::error::ToolError;
use crate::permissions::{PermissionAction, PermissionReq};
use crate::registry::{ExecCtx, Tool, ToolArgs, ToolOutcome};
use crate::sandbox::SandboxTier;
use crate::spec::{
    JsonSchema, Reversibility, SideEffectClass, ToolAnnotations, ToolName, ToolSpec,
};

/// How long a command may run before it is killed.
pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// The longest deadline a caller may ask for. Ten minutes: long enough for a workspace
/// build, short enough that a stuck command is a failure rather than an outage.
pub const MAX_TIMEOUT_MS: u64 = 600_000;

/// `process.exec`.
pub struct ProcessExecTool {
    spec: ToolSpec,
}

impl Default for ProcessExecTool {
    fn default() -> Self {
        ProcessExecTool::new()
    }
}

impl ProcessExecTool {
    /// The tool.
    pub fn new() -> Self {
        ProcessExecTool {
            spec: ToolSpec {
                name: ToolName::new("process.exec").expect("process.exec is a namespace.verb pair"),
                version: "0.1.0".to_string(),
                description: "Run a command directly, without a shell, under a command-scoped \
                              capability and a deadline."
                    .to_string(),
                input_schema: JsonSchema::object_with_strings(&["cmd"]),
                output_schema: JsonSchema::any_object(),
                // The tool declares the commands it is written to run; the grant that
                // authorizes the call may name a narrower pattern, and the narrower one
                // is what the sandbox enforces.
                permissions: vec![PermissionReq::new(
                    "process:execute:cargo",
                    PermissionAction::Execute,
                )],
                reversibility: Reversibility::Compensatable,
                side_effects: SideEffectClass::External,
                annotations: ToolAnnotations {
                    read_only: false,
                    destructive: true,
                    idempotent: false,
                    open_world: true,
                },
                module_uri: "https://metamind.dev/code/module/tools/process-exec".to_string(),
                sandbox_tier: SandboxTier::WasmCaps,
            },
        }
    }
}

/// Split a command into a program and its arguments, without a shell.
///
/// Quoted segments are honoured — `"a b"` is one argument — because a path with a space
/// is ordinary, but nothing is expanded: a `$`, a `*` or a `|` is passed through as the
/// literal character it is.
pub fn split_command(command: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quoted: Option<char> = None;
    for character in command.trim().chars() {
        match quoted {
            Some(quote) => {
                if character == quote {
                    quoted = None;
                } else {
                    current.push(character);
                }
            }
            None => match character {
                '"' | '\'' => quoted = Some(character),
                c if c.is_whitespace() => {
                    if !current.is_empty() {
                        words.push(std::mem::take(&mut current));
                    }
                }
                c => current.push(c),
            },
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

#[async_trait]
impl Tool for ProcessExecTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(&self, ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
        let command = ctx.arg_str("cmd")?;
        ctx.check_subprocess()?;
        ctx.check_command(&command)?;

        let words = split_command(&command);
        let Some((program, arguments)) = words.split_first() else {
            return Err(ctx.bad_argument("cmd", "it is empty"));
        };
        let timeout_ms = ctx
            .opt_u64("timeout_ms")?
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let mut invocation = tokio::process::Command::new(program);
        invocation.args(arguments);
        if let Some(cwd) = ctx.opt_str("cwd") {
            let resolved = ctx.read_path(&cwd)?;
            invocation.current_dir(resolved);
        }
        let output = tokio::time::timeout(Duration::from_millis(timeout_ms), invocation.output())
            .await
            .map_err(|_| ctx.failed(format!("{program} exceeded its {timeout_ms}ms deadline")))?
            .map_err(|e| ctx.failed(format!("cannot run {program}: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        ctx.check_size((stdout.len() + stderr.len()) as u64)?;
        let exit_code = output.status.code().unwrap_or(-1);
        let output_json = json!({
            "program": program,
            "args": arguments,
            "exit_code": exit_code,
            "success": output.status.success(),
            "stdout": stdout,
            "stderr": stderr,
            "stdout_sha256": mm_core::content_hash(stdout.as_bytes()),
        });
        let summary = format!("{command} exited with {exit_code}");
        Ok(ctx.outcome(output_json, summary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ActionSpec;
    use crate::permissions::Principal;
    use crate::sandbox::{CapabilityGuard, SandboxCapabilities, DEFAULT_MAX_BYTES};
    use mm_core::{Timestamp, Ulid};

    async fn context(permissions: Vec<PermissionReq>, args: serde_json::Value) -> ExecCtx {
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
        let spec = ProcessExecTool::new().spec;
        let guard = CapabilityGuard::new(
            SandboxCapabilities::from_permissions(&permissions, dir.path(), DEFAULT_MAX_BYTES),
            SandboxTier::WasmCaps,
            dir.path(),
        );
        ExecCtx {
            action_id: Ulid::from_parts(1_700_000_000_000, 5),
            trace_id: Ulid::from_parts(1_700_000_000_000, 6),
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

    fn execute(pattern: &str) -> PermissionReq {
        PermissionReq::new(
            format!("process:execute:{pattern}"),
            PermissionAction::Execute,
        )
    }

    #[tokio::test]
    async fn a_granted_command_runs() {
        let tool = ProcessExecTool::new();
        let ctx = context(vec![execute("sh")], json!({ "cmd": "sh -c 'echo hello'" })).await;
        let outcome = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap();
        assert_eq!(outcome.output["success"], true);
        assert_eq!(outcome.output["exit_code"], 0);
        assert!(outcome.output["stdout"].as_str().unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn a_command_outside_the_granted_pattern_is_refused() {
        let tool = ProcessExecTool::new();
        // Granted `echo`, asked for `rm`: the pattern is the restriction.
        let ctx = context(
            vec![execute("echo")],
            json!({ "cmd": "rm -rf /tmp/whatever" }),
        )
        .await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert!(error.is_sandbox_denial(), "{error}");
        assert!(error.to_string().contains("subprocess"));
    }

    #[tokio::test]
    async fn a_pattern_covers_whole_words_only() {
        assert!(crate::sandbox::command_pattern_covers(
            "cargo",
            "cargo test -p mm-tools"
        ));
        assert!(crate::sandbox::command_pattern_covers(
            "cargo test",
            "cargo test -p mm-tools"
        ));
        assert!(crate::sandbox::command_pattern_covers(
            "*",
            "anything at all"
        ));
        assert!(!crate::sandbox::command_pattern_covers(
            "cargo",
            "cargo-evil"
        ));
        assert!(!crate::sandbox::command_pattern_covers("cargo", "rm -rf /"));
    }

    #[tokio::test]
    async fn a_guard_with_no_process_capability_refuses_everything() {
        let tool = ProcessExecTool::new();
        let ctx = context(Vec::new(), json!({ "cmd": "echo hi" })).await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert!(error.to_string().contains("subprocess"), "{error}");
    }

    #[test]
    fn a_command_splits_without_a_shell() {
        assert_eq!(
            split_command("cargo test -p mm-tools"),
            vec![
                "cargo".to_string(),
                "test".to_string(),
                "-p".to_string(),
                "mm-tools".to_string()
            ]
        );
        assert_eq!(
            split_command("echo \"a b\" 'c d'"),
            vec!["echo".to_string(), "a b".to_string(), "c d".to_string()]
        );
        assert_eq!(
            split_command("echo $HOME | wc"),
            vec![
                "echo".to_string(),
                "$HOME".to_string(),
                "|".to_string(),
                "wc".to_string()
            ],
            "nothing is expanded and no pipe is created"
        );
        assert!(split_command("   ").is_empty());
    }

    #[test]
    fn the_spec_is_valid_and_names_its_module() {
        let spec = ProcessExecTool::new().spec;
        spec.validate().unwrap();
        assert_eq!(spec.sandbox_tier, SandboxTier::WasmCaps);
        assert_eq!(spec.side_effects, SideEffectClass::External);
        assert_eq!(spec.reversibility, Reversibility::Compensatable);
    }
}
