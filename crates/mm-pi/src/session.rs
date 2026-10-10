//! A recorded session: a header, a `parentId` chain, and a gapless sequence.
//!
//! The strictness here is the point. A session record is evidence about what the agent
//! did, so this parser refuses anything it cannot represent exactly:
//!
//! * **The first line is a `version: 3` header.** A different version is a different
//!   format; reading it as version 3 would produce a plausible chain with the wrong
//!   meaning.
//! * **`seq` is 1..=n in file order.** A record that carries its own `seq` must agree
//!   with its position. A hole in the sequence is the shape a dropped record takes, and
//!   a hole nobody notices is worse than a refusal.
//! * **A `parentId` names an earlier record in the same file.** The chain is what makes
//!   the order meaningful; a dangling parent is either a corrupted file or a record from
//!   a session this one continues, and neither can be resolved by guessing.
//! * **A truncated final record is a refusal.** The last line failing to parse is what an
//!   interrupted run looks like, and it is reported as [`crate::PiError::Truncated`]
//!   rather than dropped, because "the agent stopped mid-record" is a fact about the run.
//! * **`session_end` is last, and there is at most one.** Records after the end cannot
//!   exist, so one that appears there means the file is not what it claims to be.
//!
//! The `raw` lines are kept beside the parsed records: the ingest writes the bytes the
//! agent actually sent rather than a re-rendering of them, which is what makes `pi_events`
//! an archive instead of a summary.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{PiError, Result};
use crate::framing::iter_lines;
use crate::protocol::PiSessionHeader;
use mm_core::{id::parse_ulid, Timestamp, Ulid};

/// What a record is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PiRecordKind {
    /// A message to or from the model.
    Message,
    /// A tool invocation the session made.
    ToolCall,
    /// A file the session changed.
    Edit,
    /// The result of a turn.
    Result,
    /// The end of the session.
    SessionEnd,
}

/// Every kind, in the order the plan lists them.
pub const RECORD_KINDS: [PiRecordKind; 5] = [
    PiRecordKind::Message,
    PiRecordKind::ToolCall,
    PiRecordKind::Edit,
    PiRecordKind::Result,
    PiRecordKind::SessionEnd,
];

impl PiRecordKind {
    /// Every kind.
    pub const ALL: [PiRecordKind; 5] = RECORD_KINDS;

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            PiRecordKind::Message => "message",
            PiRecordKind::ToolCall => "tool_call",
            PiRecordKind::Edit => "edit",
            PiRecordKind::Result => "result",
            PiRecordKind::SessionEnd => "session_end",
        }
    }

    /// Parse a wire name. An unknown name is `None`, never a default.
    pub fn parse(text: &str) -> Option<Self> {
        RECORD_KINDS.into_iter().find(|kind| kind.as_str() == text)
    }

    /// True when the record changes the tree.
    pub fn is_edit(self) -> bool {
        matches!(self, PiRecordKind::Edit)
    }

    /// True when the record invokes a tool.
    pub fn is_tool_call(self) -> bool {
        matches!(self, PiRecordKind::ToolCall)
    }
}

impl std::fmt::Display for PiRecordKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One record of a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PiRecord {
    /// The 1-based position in the file, assigned by this parser.
    pub seq: u32,
    /// The record's own id.
    pub id: String,
    /// The id of the record it follows in the chain.
    pub parent_id: Option<String>,
    /// When it happened.
    pub at: Timestamp,
    /// What it is.
    pub kind: PiRecordKind,
    /// The tool, for a tool call.
    pub tool: Option<String>,
    /// The path, for an edit.
    pub path: Option<PathBuf>,
    /// The record's text.
    pub text: String,
}

impl PiRecord {
    /// A canonical rendering: sorted keys, escaped, so two equal records render alike.
    pub fn canonical(&self) -> String {
        serde_json::json!({
            "seq": self.seq,
            "id": self.id,
            "parent_id": self.parent_id,
            "at": self.at.to_rfc3339(),
            "kind": self.kind.as_str(),
            "tool": self.tool,
            "path": self.path.as_ref().map(|p| p.display().to_string()),
            "text": self.text,
        })
        .to_string()
    }
}

/// One edit a session made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PiEdit {
    /// The record's position in the stream.
    pub seq: u32,
    /// The file the edit touched.
    pub path: PathBuf,
    /// The record's kind, as written: `edit`.
    pub kind: String,
}

/// One tool a session invoked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PiToolCall {
    /// The record's position in the stream.
    pub seq: u32,
    /// The tool's name.
    pub tool: String,
}

/// A parsed session, before it reaches any store.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedSession {
    /// The header.
    pub header: PiSessionHeader,
    /// The records, in file order, with `seq` 1..=n.
    pub records: Vec<PiRecord>,
    /// The raw line each record came from, aligned with `records`.
    pub raw: Vec<String>,
    /// When the session ended, from the `session_end` record.
    pub ended_at: Option<Timestamp>,
}

/// A record as it appears on the wire.
#[derive(Debug, Deserialize)]
struct WireRecord {
    id: String,
    #[serde(rename = "parentId", default)]
    parent_id: Option<String>,
    at: String,
    kind: String,
    #[serde(default)]
    seq: Option<u32>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    text: Option<String>,
}

impl ParsedSession {
    /// How many records the session holds.
    pub fn event_count(&self) -> u32 {
        self.records.len() as u32
    }

    /// Every edit, in stream order.
    pub fn edits(&self) -> Vec<PiEdit> {
        self.records
            .iter()
            .filter(|record| record.kind.is_edit())
            .filter_map(|record| {
                record.path.as_ref().map(|path| PiEdit {
                    seq: record.seq,
                    path: path.clone(),
                    kind: record.kind.as_str().to_string(),
                })
            })
            .collect()
    }

    /// Every tool call, in stream order.
    pub fn tool_calls(&self) -> Vec<PiToolCall> {
        self.records
            .iter()
            .filter(|record| record.kind.is_tool_call())
            .filter_map(|record| {
                record.tool.as_ref().map(|tool| PiToolCall {
                    seq: record.seq,
                    tool: tool.clone(),
                })
            })
            .collect()
    }

    /// The record ids in order: the chain, as ids.
    pub fn parent_chain(&self) -> Vec<String> {
        self.records
            .iter()
            .map(|record| record.id.clone())
            .collect()
    }

    /// The session this one continues, when its first record names a parent.
    ///
    /// A session's first record normally has no parent. When it does, the parent is a
    /// record from a *previous* session and its ULID-shaped id names that session: the
    /// prototype's ids are ULIDs, so the first 26 characters are the session and the rest
    /// is the position. Anything that does not parse as a ULID yields `None` rather than
    /// a wrong session.
    pub fn parent_session(&self) -> Option<Ulid> {
        let first = self.records.first()?;
        let parent = first.parent_id.as_ref()?;
        let head: String = parent.chars().take(mm_core::ULID_LEN).collect();
        let id = parse_ulid(&head).ok()?;
        if id.is_nil() {
            None
        } else {
            Some(id)
        }
    }

    /// A canonical rendering of the whole session.
    pub fn canonical(&self) -> String {
        let records: Vec<String> = self.records.iter().map(PiRecord::canonical).collect();
        format!(
            "session={}\nmodel={}\ncwd={}\nstarted={}\nended={}\n{}",
            mm_core::ulid_string(&self.header.session_id),
            self.header.model,
            self.header.cwd.display(),
            self.header.started_at.to_rfc3339(),
            self.ended_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_else(|| "-".to_string()),
            records.join("\n")
        )
    }
}

/// Parse a session document.
pub fn parse_session(text: &str) -> Result<ParsedSession> {
    let lines = iter_lines(text);
    let Some((header_line, rest)) = lines.split_first() else {
        return Err(PiError::session(Path::new(""), "the session file is empty"));
    };
    if header_line.trim().is_empty() {
        return Err(PiError::session(
            Path::new(""),
            "the session file starts with a blank line",
        ));
    }
    let header = PiSessionHeader::parse(header_line)?;

    let mut records: Vec<PiRecord> = Vec::new();
    let mut raw_lines: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut ended_at: Option<Timestamp> = None;

    for (index, line) in rest.iter().enumerate() {
        let position = (index + 1) as u32;
        let is_last = index + 1 == rest.len();
        let wire: WireRecord = serde_json::from_str(line).map_err(|e| {
            // The last line failing is what an interrupted run looks like; a middle line
            // failing is a corrupt file. Both are refusals, and they are different ones.
            if is_last {
                PiError::Truncated {
                    path: PathBuf::new(),
                    seq: position,
                    detail: e.to_string(),
                }
            } else {
                PiError::session(
                    Path::new(""),
                    format!("record {position} is not a record: {e}"),
                )
            }
        })?;
        if let Some(declared) = wire.seq {
            if declared != position {
                return Err(PiError::Sequence {
                    position,
                    found: declared,
                });
            }
        }
        if !seen.insert(wire.id.clone()) {
            return Err(PiError::DuplicateId { id: wire.id });
        }
        if let Some(parent) = &wire.parent_id {
            let known = records.iter().any(|record| record.id == *parent);
            if !known {
                return Err(PiError::ParentChain {
                    id: wire.id,
                    parent: parent.clone(),
                });
            }
        }
        let kind = PiRecordKind::parse(&wire.kind).ok_or_else(|| PiError::UnknownKind {
            kind: wire.kind.clone(),
        })?;
        let at = Timestamp::from_rfc3339(&wire.at).map_err(|e| {
            PiError::session(
                Path::new(""),
                format!("record {position} has an unreadable `at`: {e}"),
            )
        })?;
        if kind == PiRecordKind::SessionEnd {
            if !is_last {
                return Err(PiError::session(
                    Path::new(""),
                    format!(
                        "record {position} ends the session but {position} is not the last record"
                    ),
                ));
            }
            ended_at = Some(at);
        }
        records.push(PiRecord {
            seq: position,
            id: wire.id,
            parent_id: wire.parent_id,
            at,
            kind,
            tool: wire.tool,
            path: wire.path,
            text: wire.text.unwrap_or_default(),
        });
        raw_lines.push(line.clone());
    }

    Ok(ParsedSession {
        header,
        records,
        raw: raw_lines,
        ended_at,
    })
}

/// Parse a session file, filling the path into any refusal that lacks one.
pub fn parse_session_file(path: &Path) -> Result<ParsedSession> {
    let text = std::fs::read_to_string(path).map_err(|e| PiError::session(path, format!("{e}")))?;
    parse_session(&text).map_err(|e| fill_path(e, path))
}

/// Put the file's path into a refusal that was built without one.
fn fill_path(error: PiError, path: &Path) -> PiError {
    match error {
        PiError::Session {
            path: empty,
            detail,
        } if empty.as_os_str().is_empty() => PiError::session(path, detail),
        PiError::Truncated {
            path: empty,
            seq,
            detail,
        } if empty.as_os_str().is_empty() => PiError::Truncated {
            path: path.to_path_buf(),
            seq,
            detail,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = r#"{"version":3,"sessionId":"01h0000000000000000000f100","model":"pi-test","cwd":"data/sandbox/01h0000000000000000000f100","startedAt":"2026-10-08T00:00:00.000000000Z"}"#;

    fn record(seq: u32, parent: Option<&str>, kind: &str, extra: &str) -> String {
        let parent = match parent {
            Some(id) => format!("\"{id}\""),
            None => "null".to_string(),
        };
        format!(
            r#"{{"id":"01h0000000000000000000f10{seq}","parentId":{parent},"at":"2026-10-08T00:00:0{seq}.000000000Z","kind":"{kind}"{extra}}}"#,
            seq = seq
        )
    }

    fn session() -> String {
        let mut text = String::new();
        text.push_str(HEADER);
        text.push('\n');
        text.push_str(&record(1, None, "message", r#","text":"read the skill""#));
        text.push('\n');
        text.push_str(&record(
            2,
            Some("01h0000000000000000000f101"),
            "tool_call",
            r#","tool":"read_file","text":"read""#,
        ));
        text.push('\n');
        text.push_str(&record(
            3,
            Some("01h0000000000000000000f102"),
            "edit",
            r#","path":"modules/cognition/calibration/plugin.toml""#,
        ));
        text.push('\n');
        text.push_str(&record(
            4,
            Some("01h0000000000000000000f103"),
            "session_end",
            r#","text":"done""#,
        ));
        text.push('\n');
        text
    }

    #[test]
    fn a_well_formed_session_parses_into_a_chain() {
        let parsed = parse_session(&session()).unwrap();
        assert_eq!(parsed.header.version, 3);
        assert_eq!(parsed.event_count(), 4);
        assert_eq!(
            parsed.parent_chain(),
            vec![
                "01h0000000000000000000f101".to_string(),
                "01h0000000000000000000f102".to_string(),
                "01h0000000000000000000f103".to_string(),
                "01h0000000000000000000f104".to_string()
            ]
        );
        assert_eq!(parsed.edits().len(), 1);
        assert_eq!(parsed.edits()[0].seq, 3);
        assert_eq!(parsed.tool_calls().len(), 1);
        assert_eq!(parsed.tool_calls()[0].tool, "read_file");
        assert_eq!(parsed.records[0].seq, 1);
        assert_eq!(
            parsed.records[1].parent_id.as_deref(),
            Some("01h0000000000000000000f101")
        );
        assert!(parsed.ended_at.is_some());
        assert_eq!(
            parsed.parent_session(),
            None,
            "the first record has no parent"
        );
        assert_eq!(parsed.raw.len(), 4);
        assert_eq!(parsed, parse_session(&session()).unwrap());
        assert_eq!(
            parsed.canonical(),
            parse_session(&session()).unwrap().canonical()
        );
    }

    #[test]
    fn crlf_and_a_missing_final_newline_are_accepted() {
        let text = session().replace('\n', "\r\n");
        let parsed = parse_session(&text).unwrap();
        assert_eq!(parsed.event_count(), 4);

        let trimmed = session();
        let parsed = parse_session(trimmed.trim_end_matches('\n')).unwrap();
        assert_eq!(parsed.event_count(), 4);
    }

    #[test]
    fn an_empty_file_or_a_missing_header_is_refused() {
        assert!(parse_session("").is_err());
        assert!(parse_session("\n").is_err());
        assert!(parse_session(r#"{"version":3}"#).is_err());
    }

    #[test]
    fn a_wrong_version_is_refused_before_anything_is_read() {
        let text = session().replace("\"version\":3", "\"version\":4");
        let error = parse_session(&text).unwrap_err();
        assert_eq!(error.code(), "unsupported_version");
    }

    #[test]
    fn a_truncated_final_record_is_refused_as_truncation() {
        let text = session();
        let cut = &text[..text.len() - 12];
        let error = parse_session(cut).unwrap_err();
        assert_eq!(error.code(), "truncated", "{error}");
        assert!(error.is_recording_fault());
    }

    #[test]
    fn a_hole_or_a_renumbering_is_refused() {
        // 1, 2, 4: a hole where 3 should be.
        let text = session().replace(
            r#""id":"01h0000000000000000000f103""#,
            r#""seq":9,"id":"01h0000000000000000000f103""#,
        );
        let error = parse_session(&text).unwrap_err();
        assert_eq!(error.code(), "sequence", "{error}");
        assert!(error.to_string().contains("position 3"));
    }

    #[test]
    fn a_dangling_parent_is_refused() {
        let text = session().replace(
            r#""parentId":"01h0000000000000000000f103""#,
            r#""parentId":"01h0000000000000000000ffff""#,
        );
        let error = parse_session(&text).unwrap_err();
        assert_eq!(error.code(), "parent_chain", "{error}");
    }

    #[test]
    fn a_duplicate_id_is_refused() {
        let text = session().replace(
            r#""id":"01h0000000000000000000f102""#,
            r#""id":"01h0000000000000000000f101""#,
        );
        let error = parse_session(&text).unwrap_err();
        assert_eq!(error.code(), "duplicate_id");
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        let text = session().replace("\"kind\":\"result\"", "\"kind\":\"telemetry\"");
        let text = if text.contains("telemetry") {
            text
        } else {
            session().replace("\"kind\":\"tool_call\"", "\"kind\":\"telemetry\"")
        };
        let error = parse_session(&text).unwrap_err();
        assert_eq!(error.code(), "unknown_kind", "{error}");
    }

    #[test]
    fn a_session_end_that_is_not_last_is_refused() {
        let text = session().replace(r#""kind":"tool_call""#, r#""kind":"session_end""#);
        let error = parse_session(&text).unwrap_err();
        assert_eq!(error.code(), "session");
        assert!(error.to_string().contains("ends the session"));
    }

    #[test]
    fn every_kind_round_trips_through_its_wire_name() {
        for kind in PiRecordKind::ALL {
            assert_eq!(PiRecordKind::parse(kind.as_str()), Some(kind));
            assert_eq!(kind.to_string(), kind.as_str());
        }
        assert_eq!(PiRecordKind::parse("stream"), None);
        assert!(PiRecordKind::Edit.is_edit());
        assert!(!PiRecordKind::Message.is_edit());
        assert!(PiRecordKind::ToolCall.is_tool_call());
    }
}
