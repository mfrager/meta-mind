//! The Pi RPC protocol: the commands, the events, and the session header.
//!
//! Two decisions are worth stating, because they are the ones a reader will question.
//!
//! **A command's rendering is deterministic.** Every command renders through a JSON
//! object with a fixed field set, and `serde_json`'s default map is a `BTreeMap`, so the
//! keys come out sorted. Two equal commands therefore produce the same bytes, which is
//! what lets a recorded session be replayed and compared rather than merely re-run. The
//! rendering is a `Display`-style `to_string` on the value rather than a fallible
//! `serde_json::to_string`, because there is no input for which it can fail.
//!
//! **Every inbound record is typed or refused.** [`PiEvent::parse`] accepts a response
//! and an agent record and *refuses* everything else by name. The alternative — treating
//! an unrecognised record as an opaque agent record — is how a stream silently loses the
//! one record that mattered: a `response` whose shape changed would be filed as agent
//! noise, and the caller waiting for its id would hang. The phase's rule is that a
//! session is an unreliable witness only when something is missing, so missing-ness is a
//! refusal.
//!
//! The session header is checked for its **version**, not parsed loosely: a `version: 4`
//! file is a different format, and reading it as version 3 would produce a plausible
//! chain of records with the wrong meaning.

use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{PiError, Result};
use mm_core::{id::parse_ulid, iri, Timestamp, Ulid};

/// The only session-format version this build speaks.
pub const SUPPORTED_SESSION_VERSION: u32 = 3;

/// A command sent to a Pi session.
///
/// Each carries the id its response will name, which is what keeps a stream of
/// concurrent prompts attributable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PiCommand {
    /// Send an instruction.
    Prompt {
        /// The correlation id.
        id: String,
        /// The instruction.
        message: String,
        /// The streaming behaviour, when the caller needs one.
        streaming_behavior: Option<String>,
    },
    /// Redirect the work in progress.
    Steer {
        /// The correlation id.
        id: String,
        /// The redirection.
        message: String,
    },
    /// Queue a message for when the current turn ends.
    FollowUp {
        /// The correlation id.
        id: String,
        /// The message.
        message: String,
    },
    /// Stop the current turn.
    Abort {
        /// The correlation id.
        id: String,
    },
    /// Drop everything queued.
    ClearQueue {
        /// The correlation id.
        id: String,
    },
    /// Start a new session.
    NewSession {
        /// The correlation id.
        id: String,
    },
}

impl PiCommand {
    /// A prompt with no streaming override.
    pub fn prompt(id: impl Into<String>, message: impl Into<String>) -> Self {
        PiCommand::Prompt {
            id: id.into(),
            message: message.into(),
            streaming_behavior: None,
        }
    }

    /// The correlation id.
    pub fn id(&self) -> &str {
        match self {
            PiCommand::Prompt { id, .. }
            | PiCommand::Steer { id, .. }
            | PiCommand::FollowUp { id, .. }
            | PiCommand::Abort { id }
            | PiCommand::ClearQueue { id }
            | PiCommand::NewSession { id } => id,
        }
    }

    /// The wire name, which is also the `command` a response echoes back.
    pub fn kind(&self) -> &'static str {
        match self {
            PiCommand::Prompt { .. } => "prompt",
            PiCommand::Steer { .. } => "steer",
            PiCommand::FollowUp { .. } => "follow_up",
            PiCommand::Abort { .. } => "abort",
            PiCommand::ClearQueue { .. } => "clear_queue",
            PiCommand::NewSession { .. } => "new_session",
        }
    }

    /// One JSON line, without the terminator.
    pub fn to_json_line(&self) -> String {
        let value = match self {
            PiCommand::Prompt {
                id,
                message,
                streaming_behavior,
            } => match streaming_behavior {
                Some(behavior) => serde_json::json!({
                    "type": "prompt",
                    "id": id,
                    "message": message,
                    "streamingBehavior": behavior,
                }),
                None => serde_json::json!({
                    "type": "prompt",
                    "id": id,
                    "message": message,
                }),
            },
            PiCommand::Steer { id, message } => serde_json::json!({
                "type": "steer",
                "id": id,
                "message": message,
            }),
            PiCommand::FollowUp { id, message } => serde_json::json!({
                "type": "follow_up",
                "id": id,
                "message": message,
            }),
            PiCommand::Abort { id } => serde_json::json!({ "type": "abort", "id": id }),
            PiCommand::ClearQueue { id } => {
                serde_json::json!({ "type": "clear_queue", "id": id })
            }
            PiCommand::NewSession { id } => {
                serde_json::json!({ "type": "new_session", "id": id })
            }
        };
        value.to_string()
    }
}

/// A record read from a Pi session's output.
#[derive(Clone, Debug, PartialEq)]
pub enum PiEvent {
    /// The session answered a command.
    Response {
        /// The id of the command it answers, when the response names one.
        id: Option<String>,
        /// The command name the session echoed.
        command: String,
        /// Whether the session reports it succeeded.
        success: bool,
    },
    /// Anything the session streamed while working.
    Agent {
        /// The record as it arrived.
        raw: serde_json::Value,
    },
}

impl PiEvent {
    /// Parse one record. Everything unrecognised is a refusal.
    pub fn parse(line: &str) -> Result<PiEvent> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Err(PiError::Protocol {
                detail: "an empty record is not a record".to_string(),
            });
        }
        let value: serde_json::Value =
            serde_json::from_str(trimmed).map_err(|e| PiError::Protocol {
                detail: format!("not JSON: {e}"),
            })?;
        let kind = value
            .get("type")
            .and_then(|t| t.as_str())
            .ok_or_else(|| PiError::Protocol {
                detail: "the record has no `type`".to_string(),
            })?;
        match kind {
            "response" => {
                let command = value
                    .get("command")
                    .and_then(|c| c.as_str())
                    .ok_or_else(|| PiError::Protocol {
                        detail: "a response names no `command`".to_string(),
                    })?;
                let id = value.get("id").and_then(|i| i.as_str()).map(str::to_string);
                // A response that does not say it succeeded did not.
                let success = value
                    .get("success")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                Ok(PiEvent::Response {
                    id,
                    command: command.to_string(),
                    success,
                })
            }
            "agent" => Ok(PiEvent::Agent { raw: value }),
            other => Err(PiError::Protocol {
                detail: format!("unknown record type {other:?}"),
            }),
        }
    }

    /// Parse a whole document of records.
    pub fn parse_all(text: &str) -> Result<Vec<PiEvent>> {
        crate::framing::iter_lines(text)
            .iter()
            .map(|line| PiEvent::parse(line))
            .collect()
    }

    /// The id this record names, when it names one.
    pub fn id(&self) -> Option<&str> {
        match self {
            PiEvent::Response { id, .. } => id.as_deref(),
            PiEvent::Agent { .. } => None,
        }
    }

    /// True when this is the response to a named command.
    pub fn is_response_to(&self, id: &str, command: &str) -> bool {
        matches!(
            self,
            PiEvent::Response {
                id: Some(event_id),
                command: event_command,
                ..
            } if event_id == id && event_command == command
        )
    }

    /// True when this is a response, whatever it answers.
    pub fn is_response(&self) -> bool {
        matches!(self, PiEvent::Response { .. })
    }
}

/// The first record of a session file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PiSessionHeader {
    /// The format version the file declares.
    pub version: u32,
    /// The session's own ULID, which is also the prefix of its records' ids.
    pub session_id: Ulid,
    /// The model the session ran.
    pub model: String,
    /// The directory the agent edited in.
    pub cwd: PathBuf,
    /// When the session started.
    pub started_at: Timestamp,
}

/// The header as it appears on the wire.
#[derive(Debug, Deserialize)]
struct WireHeader {
    version: u32,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(rename = "startedAt")]
    started_at: String,
}

impl PiSessionHeader {
    /// Parse the header line and check its version.
    ///
    /// The version is checked here rather than by the caller so no code path can build
    /// a header it does not understand.
    pub fn parse(line: &str) -> Result<PiSessionHeader> {
        let wire: WireHeader = serde_json::from_str(line.trim()).map_err(|e| PiError::Session {
            path: PathBuf::new(),
            detail: format!("the first line is not a session header: {e}"),
        })?;
        if wire.version != SUPPORTED_SESSION_VERSION {
            return Err(PiError::UnsupportedVersion {
                found: wire.version,
                expected: SUPPORTED_SESSION_VERSION,
            });
        }
        let session_id = parse_ulid(&wire.session_id)
            .map_err(|e| PiError::Id(format!("sessionId {:?}: {e}", wire.session_id)))?;
        let started_at =
            Timestamp::from_rfc3339(&wire.started_at).map_err(|e| PiError::Session {
                path: PathBuf::new(),
                detail: format!("startedAt: {e}"),
            })?;
        Ok(PiSessionHeader {
            version: wire.version,
            session_id,
            model: wire.model.unwrap_or_default(),
            cwd: wire.cwd.unwrap_or_default(),
            started_at,
        })
    }

    /// Refuse a header this build cannot faithfully read.
    pub fn check(&self) -> Result<()> {
        if self.version != SUPPORTED_SESSION_VERSION {
            return Err(PiError::UnsupportedVersion {
                found: self.version,
                expected: SUPPORTED_SESSION_VERSION,
            });
        }
        Ok(())
    }

    /// The session's instance IRI, which is the node the `/code` mirror writes.
    pub fn session_iri(&self) -> String {
        iri::data(&self.session_id).into_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_renders_one_sorted_line_with_its_id() {
        let commands = vec![
            PiCommand::prompt("c1", "scaffold the module"),
            PiCommand::Steer {
                id: "c2".into(),
                message: "stop".into(),
            },
            PiCommand::FollowUp {
                id: "c3".into(),
                message: "also write the test".into(),
            },
            PiCommand::Abort { id: "c4".into() },
            PiCommand::ClearQueue { id: "c5".into() },
            PiCommand::NewSession { id: "c6".into() },
        ];
        for command in &commands {
            let line = command.to_json_line();
            assert!(!line.contains('\n'), "a command is one line: {line}");
            let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(parsed["type"], command.kind());
            assert_eq!(parsed["id"], command.id());
            assert_eq!(line, command.to_json_line(), "rendering is deterministic");
        }
        assert_eq!(
            PiCommand::prompt("c1", "x").to_json_line(),
            "{\"id\":\"c1\",\"message\":\"x\",\"type\":\"prompt\"}"
        );
    }

    #[test]
    fn a_streaming_behavior_is_carried_only_when_set() {
        let plain = PiCommand::prompt("c1", "x").to_json_line();
        assert!(!plain.contains("streamingBehavior"));
        let streaming = PiCommand::Prompt {
            id: "c1".into(),
            message: "x".into(),
            streaming_behavior: Some("none".into()),
        }
        .to_json_line();
        assert!(streaming.contains("streamingBehavior"));
    }

    #[test]
    fn a_response_needs_its_command_and_defaults_to_failure() {
        let event =
            PiEvent::parse(r#"{"type":"response","id":"c1","command":"prompt","success":true}"#)
                .unwrap();
        assert_eq!(event.id(), Some("c1"));
        assert!(event.is_response_to("c1", "prompt"));
        assert!(!event.is_response_to("c2", "prompt"));
        assert!(event.is_response());

        let silent = PiEvent::parse(r#"{"type":"response","command":"abort"}"#).unwrap();
        assert_eq!(
            silent,
            PiEvent::Response {
                id: None,
                command: "abort".into(),
                success: false
            },
            "a response that does not claim success did not succeed"
        );

        assert!(PiEvent::parse(r#"{"type":"response","id":"c1"}"#).is_err());
    }

    #[test]
    fn an_unknown_record_type_is_refused_not_filed_as_noise() {
        let error = PiEvent::parse(r#"{"type":"telemetry","load":0.5}"#).unwrap_err();
        assert!(error.to_string().contains("telemetry"), "{error}");
        assert_eq!(error.code(), "protocol");
        assert!(PiEvent::parse("").is_err());
        assert!(PiEvent::parse("{").is_err());
        assert!(PiEvent::parse(r#"{"load":0.5}"#).is_err());
    }

    #[test]
    fn an_agent_record_keeps_its_payload() {
        let event = PiEvent::parse(r#"{"type":"agent","text":"thinking"}"#).unwrap();
        match event {
            PiEvent::Agent { raw } => assert_eq!(raw["text"], "thinking"),
            other => panic!("expected an agent record, got {other:?}"),
        }
    }

    #[test]
    fn parse_all_handles_crlf_and_a_trailing_partial_only_when_told() {
        let events = PiEvent::parse_all(
            "{\"type\":\"agent\",\"text\":\"a\"}\r\n{\"type\":\"agent\",\"text\":\"b\"}\n",
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        // An unterminated final record is still a record.
        let events = PiEvent::parse_all("{\"type\":\"agent\",\"text\":\"a\"}").unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn a_header_is_version_checked_at_parse_time() {
        let header = PiSessionHeader::parse(
            r#"{"version":3,"sessionId":"01h0000000000000000000f101","model":"pi-test","cwd":"/tmp/x","startedAt":"2026-10-08T00:00:00.000000000Z"}"#,
        )
        .unwrap();
        assert_eq!(header.version, 3);
        assert_eq!(header.model, "pi-test");
        assert_eq!(header.cwd, PathBuf::from("/tmp/x"));
        // 2026-10-08T00:00:00Z.
        assert_eq!(
            header.started_at,
            Timestamp::from_epoch_seconds(1_791_417_600)
        );
        header.check().unwrap();
        assert!(header
            .session_iri()
            .starts_with("https://metamind.dev/data/"));

        let error = PiSessionHeader::parse(
            r#"{"version":4,"sessionId":"01h0000000000000000000f101","startedAt":"2026-10-08T00:00:00.000000000Z"}"#,
        )
        .unwrap_err();
        assert_eq!(error.code(), "unsupported_version");
        assert!(error.is_recording_fault());
    }

    #[test]
    fn a_header_with_a_bad_id_or_instant_is_refused() {
        assert!(PiSessionHeader::parse(
            r#"{"version":3,"sessionId":"not-a-ulid","startedAt":"2026-10-08T00:00:00.000000000Z"}"#
        )
        .is_err());
        assert!(PiSessionHeader::parse(
            r#"{"version":3,"sessionId":"01h0000000000000000000f101","startedAt":"yesterday"}"#
        )
        .is_err());
    }
}
