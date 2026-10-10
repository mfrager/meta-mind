//! The async client over the installed `pi` binary.
//!
//! Pi is an *external process*, not copied source: this crate speaks its documented RPC
//! protocol and records the invocation, and the phase plan treats a Pi version change as
//! a versioned dependency rather than a vendored tree. Three decisions follow from that:
//!
//! * **The client is strict about the stream.** Every inbound record goes through
//!   [`PiEvent::parse`], so a record this build cannot represent fails the call that was
//!   waiting on it instead of being skipped. A blank line is not a record and is skipped;
//!   a non-empty line that is not JSON is a refusal.
//! * **The default working directory is the sandbox.** [`PiConfig::default_for`] points
//!   `cwd` at `<root>/data/sandbox` and the session directory at
//!   `<root>/data/sandbox/pi`, never at the repository root. A client that defaulted to
//!   the repository would make "Pi only edits inside a sandbox" a property of the caller
//!   rather than of the configuration.
//! * **The toolset is restricted and explicit.** [`PiConfig::default_tools`] is a small
//!   read/write set and nothing that runs a command: the sandbox builds and tests the
//!   candidate itself, so a shell inside the agent is capability the phase does not need.
//!
//! Logging is attached rather than required. [`PiClient::spawn`] takes only the
//! configuration, so a caller can drive a session without a kernel; [`PiClient::with_logger`]
//! adds the `pi.spawn`, `pi.command` and `pi.event` records. A logging failure is
//! deliberately not fatal here: the logger counts its own sink failures, and a session
//! must not die because a console line could not be written.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
// `Stdio` comes from `std`, not `tokio`: `tokio::process` takes the same `Stdio` values
// but does not re-export the type (only the child handles it owns).
use std::process::Stdio;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use mm_log::{codes, Level, LogRecord, Logger};

use crate::error::{PiError, Result};
use crate::framing::LineSplitter;
use crate::protocol::{PiCommand, PiEvent};
use crate::TARGET;

/// The read/write tools a session gets by default.
///
/// No shell and no test runner: `mm-selfeng` builds and tests the sandbox itself, so the
/// agent's job is to write the code, not to decide whether it passed.
pub const DEFAULT_TOOLS: [&str; 4] = ["read_file", "write_file", "edit_file", "list_dir"];

/// How the client reaches a Pi session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PiConfig {
    /// The binary to run.
    pub binary: PathBuf,
    /// The provider to route to.
    pub provider: String,
    /// The model to ask for.
    pub model: String,
    /// Where the session's own JSONL files are written.
    pub session_dir: PathBuf,
    /// The directory the agent edits in.
    pub cwd: PathBuf,
    /// The tools the session may use.
    pub tools: Vec<String>,
}

impl PiConfig {
    /// The default configuration for a repository root.
    ///
    /// The working directory and the session directory are both inside the sandbox.
    pub fn default_for(root: &Path) -> Self {
        let sandbox = root.join("data").join("sandbox");
        PiConfig {
            binary: PathBuf::from("pi"),
            provider: "recorded".to_string(),
            model: "pi-test".to_string(),
            session_dir: sandbox.join("pi"),
            cwd: sandbox,
            tools: PiConfig::default_tools(),
        }
    }

    /// The default toolset, as a list.
    pub fn default_tools() -> Vec<String> {
        DEFAULT_TOOLS.iter().map(|tool| tool.to_string()).collect()
    }

    /// The command line, in the order the protocol documents.
    pub fn argv(&self) -> Vec<String> {
        vec![
            "--mode".to_string(),
            "rpc".to_string(),
            "--provider".to_string(),
            self.provider.clone(),
            "--model".to_string(),
            self.model.clone(),
            "--session-dir".to_string(),
            self.session_dir.display().to_string(),
            "--tools".to_string(),
            self.tools.join(","),
        ]
    }
}

/// The async client.
pub struct PiClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    splitter: LineSplitter,
    pending: VecDeque<PiEvent>,
    config: PiConfig,
    sent: u64,
    logger: Option<Arc<Logger>>,
}

impl std::fmt::Debug for PiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PiClient")
            .field("pid", &self.pid())
            .field("provider", &self.config.provider)
            .field("model", &self.config.model)
            .field("sent", &self.sent)
            .finish_non_exhaustive()
    }
}

impl PiClient {
    /// Spawn a session.
    ///
    /// Both directories are created first. A session started with a working directory
    /// that does not exist fails at `exec` with a bare `ENOENT` naming the binary, which
    /// reads as "pi is not installed" and sends an operator after the wrong problem. The
    /// directories are inside the sandbox, so creating them is the one thing this makes
    /// possible and nothing else.
    pub async fn spawn(config: &PiConfig) -> Result<Self> {
        for dir in [&config.cwd, &config.session_dir] {
            std::fs::create_dir_all(dir).map_err(|e| PiError::Spawn {
                binary: config.binary.display().to_string(),
                detail: format!("cannot create {}: {e}", dir.display()),
            })?;
        }
        let mut child = Command::new(&config.binary)
            .args(config.argv())
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The session's stderr is not part of the protocol; a client that inherited it
            // would interleave agent diagnostics with the caller's own output.
            .stderr(Stdio::null())
            // A caller that drops the client must not leave an editing agent running.
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| PiError::Spawn {
                binary: config.binary.display().to_string(),
                detail: e.to_string(),
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or(PiError::Pipe { stream: "stdin" })?;
        let stdout = child
            .stdout
            .take()
            .ok_or(PiError::Pipe { stream: "stdout" })?;
        let client = PiClient {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            splitter: LineSplitter::new(),
            pending: VecDeque::new(),
            config: config.clone(),
            sent: 0,
            logger: None,
        };
        client
            .record(
                codes::PI_SPAWN,
                serde_json::json!({
                    "binary": config.binary.display().to_string(),
                    "provider": config.provider,
                    "model": config.model,
                    "session_dir": config.session_dir.display().to_string(),
                    "cwd": config.cwd.display().to_string(),
                    "tools": config.tools,
                    "pid": client.pid(),
                }),
            )
            .await;
        Ok(client)
    }

    /// Attach a logger, so the session's spawn, commands and records are recorded.
    pub fn with_logger(mut self, logger: Arc<Logger>) -> Self {
        self.logger = Some(logger);
        self
    }

    /// The configuration in use.
    pub fn config(&self) -> &PiConfig {
        &self.config
    }

    /// The operating-system process id, when it is still running.
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// How many commands have been sent.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// Write one command.
    pub async fn send(&mut self, command: PiCommand) -> Result<()> {
        let line = command.to_json_line();
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(PiError::from)?;
        self.stdin.write_all(b"\n").await.map_err(PiError::from)?;
        self.stdin.flush().await.map_err(PiError::from)?;
        self.sent += 1;
        self.record(
            codes::PI_COMMAND,
            serde_json::json!({
                "id": command.id(),
                "command": command.kind(),
                "seq": self.sent,
            }),
        )
        .await;
        Ok(())
    }

    /// Send a prompt.
    pub async fn prompt(&mut self, id: &str, message: &str) -> Result<()> {
        self.send(PiCommand::prompt(id, message)).await
    }

    /// Redirect the work in progress.
    pub async fn steer(&mut self, id: &str, message: &str) -> Result<()> {
        self.send(PiCommand::Steer {
            id: id.to_string(),
            message: message.to_string(),
        })
        .await
    }

    /// Queue a message for the end of the current turn.
    pub async fn follow_up(&mut self, id: &str, message: &str) -> Result<()> {
        self.send(PiCommand::FollowUp {
            id: id.to_string(),
            message: message.to_string(),
        })
        .await
    }

    /// Stop the current turn.
    pub async fn abort(&mut self, id: &str) -> Result<()> {
        self.send(PiCommand::Abort { id: id.to_string() }).await
    }

    /// Drop everything queued.
    pub async fn clear_queue(&mut self, id: &str) -> Result<()> {
        self.send(PiCommand::ClearQueue { id: id.to_string() })
            .await
    }

    /// Start a new session.
    pub async fn new_session(&mut self, id: &str) -> Result<()> {
        self.send(PiCommand::NewSession { id: id.to_string() })
            .await
    }

    /// The next record, or `None` at end of stream.
    pub async fn next_event(&mut self) -> Result<Option<PiEvent>> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }
            if let Some(reason) = self.splitter.reject_reason() {
                return Err(PiError::Protocol {
                    detail: reason.to_string(),
                });
            }
            let mut buffer = [0u8; 8192];
            let read = self.stdout.read(&mut buffer).await.map_err(PiError::from)?;
            if read == 0 {
                // End of stream. An unterminated final record is still a record.
                let Some(tail) = self.splitter.finish() else {
                    return Ok(None);
                };
                if tail.trim().is_empty() {
                    return Ok(None);
                }
                let event = PiEvent::parse(&tail)?;
                self.record_event(&event).await;
                return Ok(Some(event));
            }
            for line in self.splitter.push(&buffer[..read]) {
                // A blank line is not a record. A non-empty line that does not parse is a
                // refusal, raised by `parse` below.
                if line.trim().is_empty() {
                    continue;
                }
                let event = PiEvent::parse(&line)?;
                self.record_event(&event).await;
                self.pending.push_back(event);
            }
        }
    }

    /// Every record until the stream ends.
    ///
    /// The stream ends at end of stream *and* on an error: a stream cannot carry a
    /// `Result` per item without every caller matching on it, so a caller that needs the
    /// reason uses [`PiClient::next_event`]. The session's own records are the log; this
    /// iterator is for a caller driving a short exchange.
    pub fn events(&mut self) -> impl futures::Stream<Item = PiEvent> + '_ {
        futures::stream::unfold(self, |client| async move {
            match client.next_event().await {
                Ok(Some(event)) => Some((event, client)),
                _ => None,
            }
        })
    }

    /// Stop the session.
    ///
    /// A process that has already exited is not an error: the caller asked for it to be
    /// gone, and it is.
    pub async fn shutdown(&mut self) -> Result<()> {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
        Ok(())
    }

    /// Record one event, when a logger is attached.
    async fn record_event(&self, event: &PiEvent) {
        let fields = match event {
            PiEvent::Response {
                id,
                command,
                success,
            } => serde_json::json!({
                "kind": "response",
                "id": id,
                "command": command,
                "success": success,
            }),
            PiEvent::Agent { raw } => serde_json::json!({
                "kind": "agent",
                "type": raw.get("type"),
            }),
        };
        self.record(codes::PI_EVENT, fields).await;
    }

    /// Emit a record if a logger is attached.
    async fn record(&self, code: &str, fields: serde_json::Value) {
        let Some(logger) = &self.logger else {
            return;
        };
        let mut record = LogRecord::new(Level::Info, code, TARGET);
        record.fields = fields;
        let _ = logger.emit(record).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in agent: it reads commands and answers each with an agent record and a
    /// response naming the id and the command it saw.
    const FAKE_AGENT: &str = r#"#!/bin/sh
# Read one command per line from stdin, then answer it. The id and the type are pulled
# out with sed so the stand-in needs nothing but a POSIX shell.
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  cmd=$(printf '%s' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  printf '{"type":"agent","text":"working"}\n'
  printf '{"type":"response","id":"%s","command":"%s","success":true}\n' "$id" "$cmd"
done
"#;

    fn config(root: &Path) -> PiConfig {
        PiConfig::default_for(root)
    }

    #[test]
    fn the_default_configuration_edits_inside_the_sandbox_only() {
        let cfg = config(Path::new("/repo"));
        assert_eq!(cfg.binary, PathBuf::from("pi"));
        assert_eq!(cfg.cwd, PathBuf::from("/repo/data/sandbox"));
        assert_eq!(cfg.session_dir, PathBuf::from("/repo/data/sandbox/pi"));
        assert_ne!(cfg.cwd, PathBuf::from("/repo"), "never the repository root");
        for tool in DEFAULT_TOOLS {
            assert!(cfg.tools.iter().any(|t| t == tool), "{tool} is missing");
        }
        assert!(
            !cfg.tools.iter().any(|t| t == "run_command" || t == "shell"),
            "the agent gets no shell: the sandbox builds and tests the candidate"
        );
    }

    #[test]
    fn the_argv_is_the_documented_invocation() {
        let argv = config(Path::new("/repo")).argv();
        assert_eq!(argv[0], "--mode");
        assert_eq!(argv[1], "rpc");
        let session_dir = argv.iter().position(|a| a == "--session-dir").unwrap();
        assert_eq!(argv[session_dir + 1], "/repo/data/sandbox/pi");
        let tools = argv.iter().position(|a| a == "--tools").unwrap();
        assert!(argv[tools + 1].contains("write_file"));
        assert!(!argv.iter().any(|a| a == "-y" || a == "--dangerous"));
        assert_eq!(
            argv,
            config(Path::new("/repo")).argv(),
            "argv is deterministic"
        );
    }

    #[tokio::test]
    async fn a_missing_binary_is_a_named_spawn_error() {
        let cfg = PiConfig {
            binary: PathBuf::from("/nonexistent/pi-that-is-not-installed"),
            ..config(Path::new("/tmp"))
        };
        let error = PiClient::spawn(&cfg).await.unwrap_err();
        assert_eq!(error.code(), "spawn");
        assert!(!error.is_recording_fault(), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_prompt_is_answered_by_the_id_it_carried() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake_pi");
        std::fs::write(&script, FAKE_AGENT).unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let cfg = PiConfig {
            binary: script,
            ..config(dir.path())
        };
        let mut client = PiClient::spawn(&cfg).await.unwrap();
        assert!(client.pid().is_some(), "the session is running");

        client.prompt("c1", "scaffold the module").await.unwrap();
        let first = client.next_event().await.unwrap().unwrap();
        assert!(matches!(first, PiEvent::Agent { .. }), "{first:?}");
        let second = client.next_event().await.unwrap().unwrap();
        assert!(
            second.is_response_to("c1", "prompt"),
            "the response names the command it answers: {second:?}"
        );

        // A second command is correlated to its own id, not the first's.
        client.abort("c2").await.unwrap();
        let third = client.next_event().await.unwrap().unwrap();
        let fourth = client.next_event().await.unwrap().unwrap();
        assert!(matches!(third, PiEvent::Agent { .. }), "{third:?}");
        assert!(fourth.is_response_to("c2", "abort"), "{fourth:?}");
        assert!(
            !fourth.is_response_to("c1", "prompt"),
            "the second answer is not attributed to the first command"
        );
        assert_eq!(client.sent(), 2);

        client.shutdown().await.unwrap();
        assert!(client.pid().is_none(), "the session is gone");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_event_stream_yields_what_next_event_does() {
        use futures::StreamExt;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake_pi");
        std::fs::write(&script, FAKE_AGENT).unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let cfg = PiConfig {
            binary: script,
            ..config(dir.path())
        };
        let mut client = PiClient::spawn(&cfg).await.unwrap();
        client.prompt("s1", "hello").await.unwrap();
        let stream = client.events();
        futures::pin_mut!(stream);
        let first = stream.next().await.expect("an agent record");
        assert!(matches!(first, PiEvent::Agent { .. }));
        let second = stream.next().await.expect("a response");
        assert!(second.is_response_to("s1", "prompt"));
    }
}
