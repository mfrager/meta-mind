//! `mm-eventlog` — the append-only, bitemporal event log.
//!
//! The log is the source of truth: SQLite tables and RDF graphs are projections
//! that can be rebuilt from it. Writes follow `append` → (`apply`) → `commit`,
//! each audited state change and its audit record landing in one transaction, so
//! a crash mid-write leaves a `provisional` event that the next replay aborts
//! rather than a half-applied fact.
#![forbid(unsafe_code)]

pub mod log;
pub mod record;
pub mod replay;

pub use log::{EventLog, DEFAULT_CHECKPOINT, TARGET};
pub use record::{EventRecord, EventStatus};
pub use replay::{fold_hashes, fold_records, CountingApplier, EventApplier, StateSnapshot};
