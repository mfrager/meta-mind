//! `mm-pi` — the Pi code-agent client.
//!
//! Phase 11's rule is that **Pi is the only code editor**: the self-engineering loop
//! never writes source itself, it asks a Pi session to, inside a sandbox, and then judges
//! the result by evidence. This crate is the whole of that dependency — the wire protocol,
//! the client, and the ingester that turns a recorded session into kernel records.
//!
//! The modules, in the order a session flows through them:
//!
//! * [`framing`] — LF-only JSONL framing. Split on `\n` and nothing else; strip one
//!   trailing `\r`. A Unicode line separator inside a payload must not split a record,
//!   and a multi-byte character split across two reads must not be corrupted.
//! * [`protocol`] — the commands, the events, and the `version: 3` session header. An
//!   unknown record type is a hard error, never a record that is quietly dropped.
//! * [`rpc`] — the async client over the installed `pi` binary, id-correlated, with a
//!   restricted toolset.
//! * [`session`] — the recorded stream as a validated `parentId` chain with a gapless
//!   sequence.
//! * [`ingest`] — the chain written to `pi_sessions`/`pi_events` and mirrored into the
//!   `/code` graph, satisfying the `mmc:PiSessionShape`, `mmc:EditShape` and
//!   `mmc:ToolCallShape` shapes.
//!
//! Four invariants hold across the crate, and each is a test somewhere:
//!
//! 1. **A record is never silently dropped.** Invalid JSON, an unknown type, a wrong
//!    version, a truncated file and a hole in the sequence are all refusals. Ingesting
//!    less than the agent wrote would make the session an unreliable witness.
//! 2. **Ids correlate.** Every command carries an id and the response that answers it
//!    carries the same one, so a stream of concurrent prompts stays attributable.
//! 3. **The mirror is deterministic.** `canonical_turtle` renders the same records to the
//!    same bytes, so two ingestions of one file are comparable rather than merely
//!    equivalent.
//! 4. **Nothing is written outside the sandbox.** The client's working directory and the
//!    installer's are the caller's to choose; this crate never defaults to the repository
//!    root as the place an agent edits.
#![forbid(unsafe_code)]

pub mod error;
pub mod framing;
pub mod ingest;
pub mod protocol;
pub mod rpc;
pub mod session;

pub use error::{PiError, Result};
pub use framing::{iter_lines, LineSplitter, MAX_BUFFERED_BYTES};
pub use ingest::{ingest_session, ingest_session_for, PiEdit, PiSessionGraph, PiToolCall};
pub use protocol::{PiCommand, PiEvent, PiSessionHeader, SUPPORTED_SESSION_VERSION};
pub use rpc::{PiClient, PiConfig};
pub use session::{parse_session, parse_session_file, ParsedSession, PiRecord, PiRecordKind};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.pi";
