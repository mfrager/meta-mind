//! `mm-codex` — the code-metadata scanner.
//!
//! Turns the workspace into a queryable RDF artifact: a deterministic walk finds
//! modules and files, a parser and a Rust analyzer find the definitions and the
//! interfaces, and the result is emitted as canonical Turtle into the `/code`
//! named graph and mirrored into SQLite for fast queries. `codex verify` then
//! checks the rules a graph alone cannot express.
//!
//! Nothing here reads runtime state: the `/code` graph is separate from every
//! runtime graph by construction, so meta-analysis can never confuse a fact about
//! the code with a fact about the being.
#![forbid(unsafe_code)]

pub mod drift;
pub mod emit;
pub mod graph_view;
pub mod hash;
pub mod lock;
pub mod manifest;
pub mod meta;
pub mod model;
pub mod parse;
pub mod registry;
pub mod rust_analyze;
pub mod scan;
pub mod symbol;
pub mod verify;

pub use drift::{detect as detect_drift, InterfaceDrift};
pub use graph_view::{mermaid, MermaidView, ViewManifest, ViewOptions};
pub use lock::{diff, CodexLock, LockDiff, LockedModule};
pub use meta::{answer as answer_meta, MetaAnswer, MetaContext, MetaQuery};
pub use model::{
    CapabilityRecord, CodexReport, DepKind, DependencyEdge, FileRecord, ModuleKind, ModuleRecord,
    ReferenceRecord, SymbolKind, SymbolRecord,
};
pub use scan::{scan, PreviousState, ScanOutput};
pub use verify::{dependency_cycles, verify, VerifyFailure};
