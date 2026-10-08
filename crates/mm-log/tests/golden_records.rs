//! Golden `LogRecord` snapshots, one per Phase 1 event code.
//!
//! The record shape is an API: dashboards, `logs verify`, and later phases all
//! read it. A snapshot per code means an accidental field rename or a dropped
//! optional cannot slip through review unnoticed.
//!
//! The instants are fixed (`from_epoch_seconds`) and the ULIDs are literals, so
//! the snapshot depends only on the record shape.

use std::collections::BTreeMap;

use mm_core::Timestamp;
use mm_log::{codes, Level, LogRecord};

/// A syntactically valid ULID; the value is irrelevant, its shape is not.
const RECORD_ID: &str = "01h00000000000000000000000";
const TRACE_ID: &str = "01h00000000000000000000001";

fn record(index: usize, code: &str) -> LogRecord {
    LogRecord {
        record_id: RECORD_ID.to_string(),
        at: Timestamp::from_epoch_seconds(1_700_000_000 + index as u64),
        level: Level::Info,
        event_code: code.to_string(),
        target: "mm.kernel".to_string(),
        trace_id: Some(TRACE_ID.to_string()),
        span: Some("kernel.boot".to_string()),
        fields: serde_json::json!({ "code_index": index }),
        audit: false,
        redactions: 0,
    }
}

/// Every kernel code renders the same stable record shape.
#[test]
fn golden_log_records_per_code() {
    let golden: BTreeMap<&str, serde_json::Value> = codes::KERNEL_CODES
        .iter()
        .enumerate()
        .map(|(index, code)| {
            let value = serde_json::to_value(record(index, code)).unwrap();
            (*code, value)
        })
        .collect();
    assert_eq!(golden.len(), codes::KERNEL_CODES.len());
    insta::assert_json_snapshot!("golden_log_records_per_code", golden);
}

/// The optional fields really are optional: an absent `trace_id`/`span` and zero
/// redactions are omitted, not serialized as `null`.
#[test]
fn golden_log_record_without_optionals() {
    let mut minimal = record(0, codes::KERNEL_BOOT);
    minimal.trace_id = None;
    minimal.span = None;
    minimal.fields = serde_json::json!({});
    insta::assert_snapshot!("golden_log_record_without_optionals", minimal.to_json());
}

/// An audit record says so on the wire.
#[test]
fn golden_audit_record() {
    let mut audited = record(0, codes::EVENTLOG_COMMIT);
    audited.audit = true;
    audited.fields = serde_json::json!({ "id": RECORD_ID, "seq": 1 });
    insta::assert_snapshot!("golden_audit_record", audited.to_json());
}
