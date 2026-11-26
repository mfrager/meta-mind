//! What `mm-pi` refuses, and why.
//!
//! Every refusal is typed, because the caller acts on the difference: a session at the
//! wrong version is a Pi upgrade to handle, a truncated file is an interrupted run, a
//! hole in the sequence means an ingest dropped a record it should have written, and a
//! missing binary is an installation problem. A single `Error(String)` would make
//! `pi run`, `pi ingest` and the ingester's own tests print the same thing.
//!
//! The distinction the phase leans on is that **nothing here is a warning**. A record
//! this crate cannot faithfully represent is a refusal, not a record silently dropped:
//! ingesting less than the agent wrote would make the session an unreliable witness to
//! what the self-improvement loop actually did.

use std::path::PathBuf;

/// Everything this crate can refuse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PiError {
    /// The Pi binary could not be started.
    #[error("cannot start {binary}: {detail}")]
    Spawn {
        /// The binary that was asked for.
        binary: String,
        /// What the operating system said.
        detail: String,
    },
    /// A pipe the client needs was not there after the spawn.
    #[error("the pi process did not open its {stream} pipe")]
    Pipe {
        /// Which pipe: `stdin`, `stdout` or `stderr`.
        stream: &'static str,
    },
    /// The session header names a version this build does not speak.
    #[error("pi session version {found} is not supported; this build speaks version {expected}")]
    UnsupportedVersion {
        /// The version the file declares.
        found: u32,
        /// The version this build speaks.
        expected: u32,
    },
    /// A stream record this build cannot represent.
    #[error("pi protocol: {detail}")]
    Protocol {
        /// What was wrong with the record.
        detail: String,
    },
    /// A session file that is not a session this build can read.
    #[error("{path}: {detail}")]
    Session {
        /// The file.
        path: PathBuf,
        /// What was wrong with it.
        detail: String,
    },
    /// The last record of a session file is cut in half.
    #[error("{path}: record {seq} is truncated: {detail}")]
    Truncated {
        /// The file.
        path: PathBuf,
        /// The 1-based position of the partial record.
        seq: u32,
        /// What the parser saw.
        detail: String,
    },
    /// A record's own sequence number disagrees with its position in the file.
    #[error("record at position {position} declares seq {found}; seq is 1..=n in file order")]
    Sequence {
        /// The 1-based position in the file.
        position: u32,
        /// The sequence number the record carries.
        found: u32,
    },
    /// A `parentId` names a record that is not earlier in the same file.
    #[error("record {id} names parent {parent}, which is not an earlier record in this file")]
    ParentChain {
        /// The record.
        id: String,
        /// The parent it named.
        parent: String,
    },
    /// Two records share an id.
    #[error("duplicate record id {id}")]
    DuplicateId {
        /// The id.
        id: String,
    },
    /// A record kind this build does not know.
    #[error("unknown record kind {kind:?}")]
    UnknownKind {
        /// The kind as written.
        kind: String,
    },
    /// An identifier that is not one this system could have produced.
    #[error("id: {0}")]
    Id(String),
    /// A read or write failed.
    #[error("i/o: {0}")]
    Io(String),
    /// A document was not the JSON it claimed to be.
    #[error("json: {0}")]
    Json(String),
    /// The tabular store refused a write.
    #[error("store: {0}")]
    Store(String),
    /// The graph store refused a write.
    #[error("graph: {0}")]
    Graph(String),
    /// The logger refused a record.
    #[error("log: {0}")]
    Log(String),
}

impl PiError {
    /// The stable code the caller records for this refusal.
    pub fn code(&self) -> &'static str {
        match self {
            PiError::Spawn { .. } => "spawn",
            PiError::Pipe { .. } => "pipe",
            PiError::UnsupportedVersion { .. } => "unsupported_version",
            PiError::Protocol { .. } => "protocol",
            PiError::Session { .. } => "session",
            PiError::Truncated { .. } => "truncated",
            PiError::Sequence { .. } => "sequence",
            PiError::ParentChain { .. } => "parent_chain",
            PiError::DuplicateId { .. } => "duplicate_id",
            PiError::UnknownKind { .. } => "unknown_kind",
            PiError::Id(_) => "id",
            PiError::Io(_) => "io",
            PiError::Json(_) => "json",
            PiError::Store(_) => "store",
            PiError::Graph(_) => "graph",
            PiError::Log(_) => "log",
        }
    }

    /// A session refusal about a specific file.
    pub fn session(path: &std::path::Path, detail: impl Into<String>) -> Self {
        PiError::Session {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }

    /// True when the recording itself is at fault rather than the environment, which is
    /// the difference between `pi ingest` failing on a bad file and `pi run` failing on a
    /// missing binary.
    pub fn is_recording_fault(&self) -> bool {
        matches!(
            self,
            PiError::UnsupportedVersion { .. }
                | PiError::Session { .. }
                | PiError::Truncated { .. }
                | PiError::Sequence { .. }
                | PiError::ParentChain { .. }
                | PiError::DuplicateId { .. }
                | PiError::UnknownKind { .. }
                | PiError::Protocol { .. }
        )
    }
}

impl From<std::io::Error> for PiError {
    fn from(e: std::io::Error) -> Self {
        PiError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for PiError {
    fn from(e: serde_json::Error) -> Self {
        PiError::Json(e.to_string())
    }
}

impl From<mm_core::MmError> for PiError {
    fn from(e: mm_core::MmError) -> Self {
        match e {
            mm_core::MmError::Store(message) => PiError::Store(message),
            mm_core::MmError::Graph(message) => PiError::Graph(message),
            other => PiError::Io(other.to_string()),
        }
    }
}

impl From<PiError> for mm_core::MmError {
    fn from(e: PiError) -> Self {
        match e {
            PiError::Store(message) => mm_core::MmError::Store(message),
            PiError::Graph(message) => mm_core::MmError::Graph(message),
            other => mm_core::MmError::Internal(format!("{} ({})", other, other.code())),
        }
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, PiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_has_a_distinct_code() {
        let cases = [
            PiError::Spawn {
                binary: "pi".into(),
                detail: "not found".into(),
            },
            PiError::Pipe { stream: "stdout" },
            PiError::UnsupportedVersion {
                found: 4,
                expected: 3,
            },
            PiError::Protocol {
                detail: "no type".into(),
            },
            PiError::session(std::path::Path::new("a.jsonl"), "bad header"),
            PiError::Truncated {
                path: PathBuf::from("a.jsonl"),
                seq: 3,
                detail: "unexpected end".into(),
            },
            PiError::Sequence {
                position: 2,
                found: 3,
            },
            PiError::ParentChain {
                id: "a".into(),
                parent: "b".into(),
            },
            PiError::DuplicateId { id: "a".into() },
            PiError::UnknownKind {
                kind: "telemetry".into(),
            },
            PiError::Id("nil".into()),
            PiError::Io("closed".into()),
            PiError::Json("eof".into()),
            PiError::Store("locked".into()),
            PiError::Graph("locked".into()),
            PiError::Log("no audit writer".into()),
        ];
        let codes: std::collections::BTreeSet<&str> = cases.iter().map(|c| c.code()).collect();
        assert_eq!(codes.len(), cases.len(), "codes must distinguish refusals");
    }

    #[test]
    fn a_recording_fault_is_distinguishable_from_an_environment_fault() {
        assert!(PiError::UnsupportedVersion {
            found: 2,
            expected: 3
        }
        .is_recording_fault());
        assert!(PiError::Truncated {
            path: PathBuf::from("a.jsonl"),
            seq: 1,
            detail: "x".into()
        }
        .is_recording_fault());
        assert!(!PiError::Spawn {
            binary: "pi".into(),
            detail: "x".into()
        }
        .is_recording_fault());
    }
}
