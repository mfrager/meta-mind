//! `mm-store-sqlite` — the tabular half of kernel state.
//!
//! SQLite owns every tabular projection; the event log is the source of truth and
//! these tables are folds over it. This crate also implements the kernel's
//! `Tabular` and `AuditWriter` traits, so callers never open a connection
//! themselves and never write to `audit_log` except through the chain writer.
#![forbid(unsafe_code)]

pub mod audit;
pub mod bitemporal;
pub mod pool;
pub mod tabular;

pub use audit::{is_unique_violation, write_audit_in_tx, AuditChainReport, AuditRow, GENESIS_PREV};
pub use bitemporal::{AsOf, NewFact, VISIBILITY_PREDICATE};
pub use pool::{validate_identifier, SqliteStore};
