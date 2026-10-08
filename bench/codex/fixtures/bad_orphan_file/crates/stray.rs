//! The planted defect of this fixture.
//!
//! This file sits directly under `crates/` and is not inside any workspace member
//! directory, so no module owns it. Every other file in this tree is clean, so
//! `orphan_file` is the only finding `codex verify` should report here.

/// Belongs to nobody.
pub fn stray() {}
