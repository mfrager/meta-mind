//! Generate `frontend/src/lib/wire/agentstream.ts` from the Rust wire types.
//!
//! This is a contract test: a Rust type change that is not reflected in the
//! checked-in TypeScript file fails `cargo test` by rewriting it, and the diff
//! is reviewed with the change.

use std::path::Path;

use mm_serve::dto::{EventPage, HealthEnvelope, SessionList, StartTurnRequest, TurnAccepted};
use ts_rs::TS;

#[test]
fn export_agentstream_bindings() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/src/lib/wire");
    std::fs::create_dir_all(&out).unwrap();
    let cfg = ts_rs::Config::new()
        .with_out_dir(out)
        // The wire uses JSON numbers, not JavaScript bigint.
        .with_large_int("number");

    EventPage::export_all(&cfg).unwrap();
    TurnAccepted::export_all(&cfg).unwrap();
    SessionList::export_all(&cfg).unwrap();
    HealthEnvelope::export_all(&cfg).unwrap();
    StartTurnRequest::export_all(&cfg).unwrap();
}
