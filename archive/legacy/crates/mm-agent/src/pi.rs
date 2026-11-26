//! Pi backend using the external `pi-codes` SDK as its protocol glue.
//!
//! `pi-codes` owns the typed RPC commands, the CLI argv builder, and the
//! event/message vocabulary. This module supplies the process transport so the
//! configured binary remains injectable, then maps one completed Pi run onto the
//! same `AgentTurnResult` contract as Hermes.

use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use pi_codes::{
    ContentBlock, Mode, PiCliBuilder, PiEvent, PiMessage, RpcCommand, RpcResponse, Usage,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

use crate::{
    AgentArtifact, AgentBackend, AgentError, AgentRequest, AgentRunStatus, AgentToolSummary,
    AgentTurnResult, AgentUsage, Result,
};

/// Environment variable for the Pi binary.
pub const BINARY_ENV: &str = "MM_AGENT_PI_BINARY";
/// Environment variable for the Pi provider.
pub const PROVIDER_ENV: &str = "MM_AGENT_PI_PROVIDER";
/// Environment variable for the Pi model.
pub const MODEL_ENV: &str = "MM_AGENT_PI_MODEL";
/// Environment variable for a Pi turn timeout.
pub const TIMEOUT_ENV: &str = "MM_AGENT_PI_TIMEOUT_MS";

/// The read/write tool set advertised to Pi.
///
/// These are Pi's own built-in names. There is deliberately no `bash` (or any
/// other command runner) in the list.
pub const DEFAULT_TOOLS: [&str; 4] = ["read", "write", "edit", "ls"];

/// Resolved Pi settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiSettings {
    /// Pi executable.
    pub binary: PathBuf,
    /// Provider name.
    pub provider: String,
    /// Model name.
    pub model: String,
    /// Per-turn timeout.
    pub timeout_ms: u64,
}

impl PiSettings {
    /// Resolve from the environment, preserving the existing Pi defaults.
    pub fn from_env() -> Self {
        Self {
            binary: std::env::var(BINARY_ENV)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("pi")),
            provider: std::env::var(PROVIDER_ENV)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "recorded".into()),
            model: std::env::var(MODEL_ENV)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "pi-test".into()),
            timeout_ms: std::env::var(TIMEOUT_ENV)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(120_000),
        }
    }
}

/// A Pi availability snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiProbe {
    /// Whether the binary exists and is executable.
    pub available: bool,
    /// Redacted reason when unavailable.
    pub reason: Option<String>,
    /// The resolved binary.
    pub binary: PathBuf,
}

/// The Pi backend.
#[derive(Debug)]
pub struct PiBackend {
    settings: PiSettings,
    probe: Mutex<PiProbe>,
}

impl PiBackend {
    /// Build for explicit settings.
    pub fn new(settings: PiSettings) -> Self {
        let probe = probe_binary(&settings.binary);
        Self {
            settings,
            probe: Mutex::new(probe),
        }
    }

    /// Resolve from the environment.
    pub fn from_env() -> Self {
        Self::new(PiSettings::from_env())
    }

    /// Re-check the binary.
    pub fn refresh(&self) -> PiProbe {
        let probe = probe_binary(&self.settings.binary);
        *self.probe.lock().unwrap_or_else(|error| error.into_inner()) = probe.clone();
        probe
    }

    /// The current probe.
    pub fn probe(&self) -> PiProbe {
        self.probe
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// The configured tools. There is deliberately no shell.
    pub fn tools(&self) -> Vec<String> {
        DEFAULT_TOOLS
            .iter()
            .map(|tool| (*tool).to_string())
            .collect()
    }

    /// The configured per-turn timeout.
    pub fn timeout_ms(&self) -> u64 {
        self.settings.timeout_ms
    }

    /// The configured model.
    pub fn model(&self) -> &str {
        &self.settings.model
    }

    /// The `pi-codes` CLI builder, with the workspace and tool policy applied.
    fn cli(&self, request: &AgentRequest) -> PiCliBuilder {
        PiCliBuilder::new()
            .mode(Mode::Rpc)
            .provider(self.settings.provider.clone())
            .model(self.settings.model.clone())
            .session_dir(request.workspace.join(".pi").join(&request.run_id))
            .tools(DEFAULT_TOOLS.join(","))
            .no_mcp(true)
            .extra_args(["--no-extensions"])
            .working_directory(request.workspace.clone())
    }

    fn argv(&self, request: &AgentRequest) -> Vec<OsString> {
        self.cli(request).assembled_args()
    }
}

#[async_trait]
impl AgentBackend for PiBackend {
    async fn run_turn(&self, request: &AgentRequest) -> Result<AgentTurnResult> {
        let probe = if self.probe().available {
            self.probe()
        } else {
            self.refresh()
        };
        if !probe.available {
            return Err(AgentError::Unavailable(format!(
                "{} ({})",
                probe.binary.display(),
                probe.reason.unwrap_or_else(|| "Pi is unavailable".into())
            )));
        }

        let session_dir = request.workspace.join(".pi").join(&request.run_id);
        std::fs::create_dir_all(&session_dir).map_err(|error| {
            AgentError::Other(format!(
                "cannot create Pi session directory {}: {error}",
                session_dir.display()
            ))
        })?;

        let argv = self.argv(request);
        let mut client = PiTransport::spawn(&probe.binary, &argv, &request.workspace).await?;
        let timeout = Duration::from_millis(self.settings.timeout_ms.max(1));
        let started = Instant::now();
        let command_id = format!("{}-{}", request.run_id, mm_data::rdf::now_ms());
        let prompt = prompt_for(request);
        let response = match tokio::time::timeout(
            timeout,
            client.request(RpcCommand::Prompt {
                id: Some(command_id),
                message: prompt,
                images: None,
                streaming_behavior: None,
            }),
        )
        .await
        {
            Err(_) => {
                client.shutdown().await;
                return Err(AgentError::Timeout(self.settings.timeout_ms));
            }
            Ok(Err(error)) => {
                client.shutdown().await;
                return Err(error);
            }
            Ok(Ok(response)) => response,
        };

        if !response.success {
            let detail = response
                .error
                .unwrap_or_else(|| "Pi rejected the prompt".into());
            client.shutdown().await;
            return Err(AgentError::Rejected(detail));
        }
        if matches!(
            response.disposition(),
            Some(pi_codes::InputDisposition::Handled)
        ) {
            client.shutdown().await;
            return Err(AgentError::Rejected(
                "Pi handled the prompt without starting an agent turn".into(),
            ));
        }

        let messages = loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                client.shutdown().await;
                return Err(AgentError::Timeout(self.settings.timeout_ms));
            }
            let event = match tokio::time::timeout(remaining, client.next_event()).await {
                Err(_) => {
                    client.shutdown().await;
                    return Err(AgentError::Timeout(self.settings.timeout_ms));
                }
                Ok(Err(error)) => {
                    client.shutdown().await;
                    return Err(error);
                }
                Ok(Ok(Some(event))) => event,
                Ok(Ok(None)) => {
                    client.shutdown().await;
                    return Err(AgentError::Rejected(
                        "Pi ended before it completed the turn".into(),
                    ));
                }
            };
            if let PiEvent::AgentEnd { messages } = event {
                break messages;
            }
        };
        client.shutdown().await;

        let mapped = map_agent_end(messages)?;
        Ok(AgentTurnResult {
            run_id: request.run_id.clone(),
            turn: 0,
            status: AgentRunStatus::Done,
            workspace: request.workspace.clone(),
            message: mapped.message.clone(),
            summary: mapped.message,
            summary_detail: None,
            tool_calls: mapped.tool_calls,
            artifacts: mapped.artifacts,
            next: None,
            error: None,
            usage: mapped.usage,
            streamed: true,
            model: mapped.model.or_else(|| Some(self.settings.model.clone())),
            raw_ref: None,
        })
    }
}

/// A small process transport for `pi --mode rpc`.
///
/// The wire types and argv come from `pi-codes`; the transport stays here
/// because the SDK's async client resolves only the literal `pi` binary from
/// `PATH`, while this backend must honor `MM_AGENT_PI_BINARY`.
struct PiTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    pending: VecDeque<PiEvent>,
}

impl PiTransport {
    async fn spawn(binary: &Path, argv: &[OsString], cwd: &Path) -> Result<Self> {
        let mut child = tokio::process::Command::new(binary)
            .args(argv)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| AgentError::Unavailable(format!("{}: {error}", binary.display())))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AgentError::Other("Pi stdin was not piped".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AgentError::Other("Pi stdout was not piped".into()))?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            pending: VecDeque::new(),
        })
    }

    async fn request(&mut self, command: RpcCommand) -> Result<RpcResponse> {
        let id = command_id(&command)?;
        let line = command
            .to_line()
            .map_err(|error| AgentError::Rejected(format!("Pi command: {error}")))?;
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|error| AgentError::Other(format!("Pi write: {error}")))?;
        self.stdin
            .flush()
            .await
            .map_err(|error| AgentError::Other(format!("Pi write: {error}")))?;

        loop {
            let value = self.next_value().await?;
            if value.get("type").and_then(Value::as_str) == Some("response") {
                let response: RpcResponse = serde_json::from_value(value)
                    .map_err(|error| AgentError::Rejected(format!("Pi response: {error}")))?;
                if response.id.is_none() || response.id.as_deref() == Some(id.as_str()) {
                    return Ok(response);
                }
                continue;
            }
            self.pending.push_back(
                PiEvent::from_value(value)
                    .map_err(|error| AgentError::Rejected(format!("Pi event: {error}")))?,
            );
        }
    }

    async fn next_event(&mut self) -> Result<Option<PiEvent>> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(Some(event));
        }
        loop {
            let Some(line) = self
                .stdout
                .next_line()
                .await
                .map_err(|error| AgentError::Other(format!("Pi read: {error}")))?
            else {
                return Ok(None);
            };
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line)
                .map_err(|error| AgentError::Rejected(format!("Pi protocol: {error}")))?;
            return Ok(Some(PiEvent::from_value(value).map_err(|error| {
                AgentError::Rejected(format!("Pi event: {error}"))
            })?));
        }
    }

    async fn next_value(&mut self) -> Result<Value> {
        loop {
            let Some(line) = self
                .stdout
                .next_line()
                .await
                .map_err(|error| AgentError::Other(format!("Pi read: {error}")))?
            else {
                return Err(AgentError::Rejected(
                    "Pi ended before it answered the prompt".into(),
                ));
            };
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            return serde_json::from_str(line)
                .map_err(|error| AgentError::Rejected(format!("Pi protocol: {error}")));
        }
    }

    async fn shutdown(mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

fn command_id(command: &RpcCommand) -> Result<String> {
    serde_json::to_value(command)
        .ok()
        .and_then(|value| value.get("id").and_then(Value::as_str).map(str::to_string))
        .ok_or_else(|| AgentError::Rejected("Pi command has no correlation id".into()))
}

struct MappedTurn {
    message: String,
    tool_calls: Vec<AgentToolSummary>,
    artifacts: Vec<AgentArtifact>,
    usage: AgentUsage,
    model: Option<String>,
}

fn map_agent_end(messages: Vec<PiMessage>) -> Result<MappedTurn> {
    let mut texts = Vec::new();
    let mut tool_calls = Vec::new();
    let mut artifacts = Vec::new();
    let mut usage = AgentUsage::default();
    let mut model = None;
    let mut seen_calls = HashSet::new();
    let mut seen_artifacts = HashSet::new();

    for message in messages {
        let PiMessage::Assistant {
            content,
            usage: message_usage,
            model: message_model,
            stop_reason,
            ..
        } = message
        else {
            continue;
        };

        if stop_reason == "error" || stop_reason == "aborted" {
            return Err(AgentError::Rejected(format!(
                "Pi stopped with {stop_reason}"
            )));
        }
        if !message_model.is_empty() {
            model = Some(message_model);
        }
        if let Some(message_usage) = message_usage {
            add_usage(&mut usage, &message_usage);
        }

        for block in content {
            match block {
                ContentBlock::Text { text } => {
                    let text = text.trim();
                    if !text.is_empty() {
                        texts.push(text.to_string());
                    }
                }
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                    ..
                } => {
                    let key = if id.is_empty() {
                        format!("{name}:{}", tool_calls.len())
                    } else {
                        id
                    };
                    if !seen_calls.insert(key) {
                        continue;
                    }
                    let path = arguments
                        .get("path")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    tool_calls.push(AgentToolSummary {
                        name: name.clone(),
                        arguments: path
                            .clone()
                            .or_else(|| (!arguments.is_null()).then(|| arguments.to_string())),
                    });
                    if matches!(name.as_str(), "write" | "edit") {
                        if let Some(path) = path {
                            if seen_artifacts.insert(path.clone()) {
                                artifacts.push(AgentArtifact {
                                    kind: "file".into(),
                                    id: path.clone(),
                                    label: Some(format!("{name} {path}")),
                                });
                            }
                        }
                    }
                }
                ContentBlock::Thinking { .. } | ContentBlock::Image { .. } => {}
            }
        }
    }

    Ok(MappedTurn {
        message: texts.join("\n").trim().to_string(),
        tool_calls,
        artifacts,
        usage,
        model,
    })
}

fn add_usage(total: &mut AgentUsage, usage: &Usage) {
    total.prompt_tokens = total
        .prompt_tokens
        .saturating_add(usage.input.max(0.0).round() as u64);
    total.completion_tokens = total
        .completion_tokens
        .saturating_add(usage.output.max(0.0).round() as u64);
}

fn prompt_for(request: &AgentRequest) -> String {
    let mut prompt = String::new();
    if let Some(system) = &request.context {
        if !system.trim().is_empty() {
            prompt.push_str(system.trim());
            prompt.push_str("\n\n");
        }
    }
    if request.messages.len() > 1 {
        prompt.push_str("Conversation so far:\n");
        for message in &request.messages {
            let role = if message.role == "assistant" {
                "ASSISTANT"
            } else {
                "USER"
            };
            prompt.push_str(role);
            prompt.push_str(": ");
            prompt.push_str(&message.content);
            prompt.push('\n');
        }
        prompt.push('\n');
    }
    prompt.push_str("Current instruction:\n");
    prompt.push_str(&request.input);
    prompt
}

fn probe_binary(binary: &Path) -> PiProbe {
    let resolved = if binary.components().count() > 1 {
        Some(binary.to_path_buf())
    } else {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(binary))
                .find(|candidate| is_executable(candidate))
        })
    };
    let Some(resolved) = resolved else {
        return PiProbe {
            available: false,
            reason: Some("Pi binary is not on PATH".into()),
            binary: binary.to_path_buf(),
        };
    };
    if !is_executable(&resolved) {
        return PiProbe {
            available: false,
            reason: Some("Pi binary is not executable".into()),
            binary: resolved,
        };
    }
    PiProbe {
        available: true,
        reason: None,
        binary: resolved,
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pi_toolset_has_no_shell() {
        let backend = PiBackend::new(PiSettings {
            binary: PathBuf::from("pi"),
            provider: "recorded".into(),
            model: "pi-test".into(),
            timeout_ms: 1_000,
        });
        assert_eq!(backend.tools(), vec!["read", "write", "edit", "ls"]);
        assert!(!backend.tools().iter().any(|tool| tool == "bash"));
    }

    #[test]
    fn the_cli_uses_pi_codes_and_advertises_no_shell() {
        let backend = PiBackend::new(PiSettings {
            binary: PathBuf::from("pi"),
            provider: "recorded".into(),
            model: "pi-test".into(),
            timeout_ms: 1_000,
        });
        let request = AgentRequest {
            run_id: "run".into(),
            session_id: "session".into(),
            user_id: "user".into(),
            input: "scaffold".into(),
            kind: crate::AgentKind::Pi,
            mode: crate::AgentTurnMode::New,
            workspace: PathBuf::from("/tmp/workspace"),
            context: None,
            messages: Vec::new(),
            timeout_ms: 1_000,
        };
        let argv: Vec<String> = backend
            .argv(&request)
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&argv[0..2], ["--mode", "rpc"]);
        let tools = argv.iter().position(|arg| arg == "--tools").unwrap();
        assert_eq!(argv[tools + 1], "read,write,edit,ls");
        assert!(argv.iter().any(|arg| arg == "--no-mcp"));
        assert!(argv.iter().any(|arg| arg == "--no-extensions"));
        assert!(!argv.iter().any(|arg| arg == "bash"));
    }

    #[test]
    fn a_missing_binary_is_unavailable() {
        let backend = PiBackend::new(PiSettings {
            binary: PathBuf::from("/nonexistent/pi"),
            provider: "recorded".into(),
            model: "pi-test".into(),
            timeout_ms: 1_000,
        });
        assert!(!backend.probe().available);
    }

    #[test]
    fn an_agent_end_maps_messages_tools_and_usage() {
        let messages: Vec<PiMessage> = serde_json::from_value(serde_json::json!([
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Writing the module."},
                    {
                        "type": "toolCall",
                        "id": "call-1",
                        "name": "write",
                        "arguments": {"path": "src/lib.rs", "content": ""}
                    }
                ],
                "api": "openai-responses",
                "provider": "openai",
                "model": "gpt-4.1-mini",
                "usage": {"input": 100, "output": 20},
                "stopReason": "toolUse"
            },
            {
                "role": "assistant",
                "content": [{"type": "text", "text": "Module scaffolded."}],
                "api": "openai-responses",
                "provider": "openai",
                "model": "gpt-4.1-mini",
                "usage": {"input": 40, "output": 10},
                "stopReason": "stop"
            }
        ]))
        .unwrap();
        let mapped = map_agent_end(messages).unwrap();
        assert_eq!(mapped.message, "Writing the module.\nModule scaffolded.");
        assert_eq!(mapped.tool_calls.len(), 1);
        assert_eq!(mapped.tool_calls[0].name, "write");
        assert_eq!(
            mapped.tool_calls[0].arguments.as_deref(),
            Some("src/lib.rs")
        );
        assert_eq!(mapped.artifacts[0].id, "src/lib.rs");
        assert_eq!(mapped.usage.prompt_tokens, 140);
        assert_eq!(mapped.usage.completion_tokens, 30);
        assert_eq!(mapped.model.as_deref(), Some("gpt-4.1-mini"));
    }
}
