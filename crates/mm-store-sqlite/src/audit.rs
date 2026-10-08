//! The immutable audit chain.
//!
//! Every record carries a hash over its own contents plus its predecessor's
//! hash, and `audit_log_prev_unique` makes two rows sharing a predecessor
//! impossible. A fork is therefore rejected by the database rather than merely
//! detected later, and a rewrite of any row is detectable from the head of the
//! chain alone.
//!
//! Sequence numbers are assigned by the writer (not `AUTOINCREMENT`) so that a
//! rolled-back transaction leaves no hole: the chain is gapless by construction.

use async_trait::async_trait;
use mm_core::{AuditRecord, AuditWriter, MmError};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, SqliteConnection, Transaction};

use crate::pool::SqliteStore;

/// The predecessor hash of the first record. Stored rather than `NULL` so the
/// unique index applies to the genesis row too.
pub const GENESIS_PREV: &str = "";

/// How many times a lost race for the chain head is retried.
const MAX_CHAIN_ATTEMPTS: u32 = 8;

/// One row of the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRow {
    /// Position in the chain, from 1.
    pub seq: i64,
    /// The record's identifier.
    pub record_id: String,
    /// The log event code.
    pub event_code: String,
    /// Severity.
    pub level: String,
    /// Emitting subsystem.
    pub target: String,
    /// Correlation identifier.
    pub trace_id: Option<String>,
    /// Canonical JSON body.
    pub payload: String,
    /// The predecessor's hash (empty at the genesis).
    pub prev_hash: String,
    /// This record's chain hash.
    pub hash: String,
    /// The record's instant, part of the hash.
    pub at: String,
}

fn row_to_audit(row: &SqliteRow) -> Result<AuditRow, MmError> {
    Ok(AuditRow {
        seq: row.try_get("seq").map_err(map_sqlx)?,
        record_id: row.try_get("record_id").map_err(map_sqlx)?,
        event_code: row.try_get("event_code").map_err(map_sqlx)?,
        level: row.try_get("level").map_err(map_sqlx)?,
        target: row.try_get("target").map_err(map_sqlx)?,
        trace_id: row.try_get("trace_id").map_err(map_sqlx)?,
        payload: row.try_get("payload").map_err(map_sqlx)?,
        prev_hash: row.try_get("prev_hash").map_err(map_sqlx)?,
        hash: row.try_get("hash").map_err(map_sqlx)?,
        at: row.try_get("at").map_err(map_sqlx)?,
    })
}

/// The result of walking the whole chain.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AuditChainReport {
    /// Records examined.
    pub rows: u64,
    /// Whether the first row is the unique genesis.
    pub genesis_ok: bool,
    /// Whether `seq` runs 1..n with no holes.
    pub gapless: bool,
    /// The first missing sequence number, if any.
    pub first_gap_seq: Option<i64>,
    /// Whether every `prev_hash` equals its predecessor's `hash`.
    pub chain_linked: bool,
    /// The first row whose back-pointer does not match, if any.
    pub first_broken_seq: Option<i64>,
    /// How many rows were re-hashed from their contents.
    pub hashes_verified: u64,
    /// The first row whose stored hash does not match its contents, if any.
    pub hash_mismatch_seq: Option<i64>,
}

impl AuditChainReport {
    /// True when the chain is intact, gapless, and every hash reproduces.
    pub fn is_ok(&self) -> bool {
        self.genesis_ok && self.gapless && self.chain_linked && self.hash_mismatch_seq.is_none()
    }

    /// One line describing the first problem found.
    pub fn summary(&self) -> String {
        if self.is_ok() {
            return format!("{} records, chain intact", self.rows);
        }
        if !self.genesis_ok {
            return format!(
                "genesis record is missing or duplicated ({} rows)",
                self.rows
            );
        }
        if let Some(seq) = self.first_gap_seq {
            return format!("gap in audit sequence at {seq}");
        }
        if let Some(seq) = self.first_broken_seq {
            return format!("audit chain broken at seq {seq}");
        }
        match self.hash_mismatch_seq {
            Some(seq) => format!("audit record {seq} does not match its own hash"),
            None => "audit chain check failed".to_string(),
        }
    }
}

/// True when `e` is a uniqueness violation, used to detect a lost race for the
/// chain head.
pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(db) => db.is_unique_violation(),
        _ => false,
    }
}

/// True when `e` is SQLite's transient "the other writer got there first" family:
/// `SQLITE_BUSY` (5) or `SQLITE_LOCKED` (6), including their extended forms such as
/// `SQLITE_BUSY_SNAPSHOT` (517).
///
/// These are the errors a *retry* fixes, as opposed to a uniqueness violation which
/// means the chain head moved. The extended code is the primary code in its low
/// byte, which is why the test is `% 256`.
fn is_transient_lock(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(db) => {
            if let Some(code) = db.code() {
                if let Ok(value) = code.parse::<i64>() {
                    return matches!(value % 256, 5 | 6);
                }
            }
            let message = db.message();
            message.contains("database is locked") || message.contains("database table is locked")
        }
        _ => false,
    }
}

fn map_sqlx(e: sqlx::Error) -> MmError {
    MmError::Store(e.to_string())
}

/// Append a record to the chain inside a caller-owned transaction.
///
/// Callers that share a transaction with the mutation the record describes get
/// atomicity: either both land or neither does. The chain head is read inside
/// the transaction, so the new record extends whatever the transaction saw.
pub async fn write_audit_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    record: &AuditRecord,
) -> Result<(), MmError> {
    insert_audit(&mut *tx, record).await.map_err(map_sqlx)
}

async fn insert_audit(
    conn: &mut SqliteConnection,
    record: &AuditRecord,
) -> Result<(), sqlx::Error> {
    let head: Option<(i64, String)> =
        sqlx::query_as("SELECT seq, hash FROM audit_log ORDER BY seq DESC LIMIT 1")
            .fetch_optional(&mut *conn)
            .await?;
    let (prev_seq, prev_hash) = head.unwrap_or((0, GENESIS_PREV.to_string()));
    let seq = prev_seq + 1;
    let hash = record.chain_hash(Some(&prev_hash));
    sqlx::query(
        "INSERT INTO audit_log (seq, record_id, event_code, level, target, trace_id, payload, \
         prev_hash, hash, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(seq)
    .bind(mm_core::ulid_string(&record.record_id))
    .bind(&record.event_code)
    .bind(&record.level)
    .bind(&record.target)
    .bind(record.trace_id.map(|t| mm_core::ulid_string(&t)))
    .bind(record.canonical_payload())
    .bind(&prev_hash)
    .bind(&hash)
    .bind(record.at.to_rfc3339())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

impl SqliteStore {
    /// Every audit row, in chain order.
    pub async fn audit_rows(&self) -> Result<Vec<AuditRow>, MmError> {
        let rows = sqlx::query("SELECT * FROM audit_log ORDER BY seq")
            .fetch_all(self.pool())
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(row_to_audit).collect()
    }

    /// How many audit records exist.
    pub async fn audit_count(&self) -> Result<i64, MmError> {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_log")
            .fetch_one(self.pool())
            .await
            .map_err(map_sqlx)
    }

    /// The chain head as `(seq, hash)`.
    pub async fn audit_head(&self) -> Result<Option<(i64, String)>, MmError> {
        sqlx::query_as::<_, (i64, String)>(
            "SELECT seq, hash FROM audit_log ORDER BY seq DESC LIMIT 1",
        )
        .fetch_optional(self.pool())
        .await
        .map_err(map_sqlx)
    }

    /// Walk the chain and verify it end to end.
    ///
    /// This is the check `mm-cli logs verify` gates on: genesis uniqueness,
    /// gaplessness, back-pointer linkage, and re-hashing every record from the
    /// stored contents with the same function the writer used.
    pub async fn audit_chain_report(&self) -> Result<AuditChainReport, MmError> {
        let rows = self.audit_rows().await?;
        let mut report = AuditChainReport {
            rows: rows.len() as u64,
            genesis_ok: true,
            gapless: true,
            chain_linked: true,
            ..AuditChainReport::default()
        };
        let mut expected_seq: i64 = 1;
        let mut prev_hash: Option<String> = None;

        for row in &rows {
            if row.seq != expected_seq {
                report.gapless = false;
                report.first_gap_seq.get_or_insert(row.seq);
            }
            expected_seq = row.seq + 1;

            match &prev_hash {
                None => {
                    if row.prev_hash != GENESIS_PREV {
                        report.genesis_ok = false;
                    }
                }
                Some(prev) => {
                    if row.prev_hash != *prev {
                        report.chain_linked = false;
                        report.first_broken_seq.get_or_insert(row.seq);
                    }
                }
            }

            if let Some(recomputed) = recompute_hash(row) {
                report.hashes_verified += 1;
                if recomputed != row.hash {
                    report.hash_mismatch_seq.get_or_insert(row.seq);
                }
            } else {
                report.hash_mismatch_seq.get_or_insert(row.seq);
            }

            prev_hash = Some(row.hash.clone());
        }
        Ok(report)
    }
}

/// Rebuild the record from its stored columns and recompute its chain hash,
/// using the writer's own definition.
fn recompute_hash(row: &AuditRow) -> Option<String> {
    let record = AuditRecord {
        record_id: mm_core::id::parse_ulid(&row.record_id).ok()?,
        event_code: row.event_code.clone(),
        level: row.level.clone(),
        target: row.target.clone(),
        trace_id: row
            .trace_id
            .as_deref()
            .and_then(|t| mm_core::id::parse_ulid(t).ok()),
        payload: serde_json::from_str(&row.payload).ok()?,
        at: mm_core::Timestamp::from_rfc3339(&row.at).ok()?,
    };
    Some(record.chain_hash(Some(&row.prev_hash)))
}

/// The generic writer: its own transaction, its own serialization.
#[async_trait]
impl AuditWriter for SqliteStore {
    async fn write_audit(&self, record: &AuditRecord) -> Result<(), MmError> {
        let _guard = self.audit_lock().lock().await;
        let mut last_error: Option<MmError> = None;
        for attempt in 0..MAX_CHAIN_ATTEMPTS {
            let mut conn = self.pool().acquire().await.map_err(map_sqlx)?;
            // `BEGIN IMMEDIATE` takes the write lock *before* the chain head is read.
            // A deferred transaction reads a snapshot first and then discovers it can
            // no longer be upgraded once any other writer commits —
            // `SQLITE_BUSY_SNAPSHOT`, which no busy timeout can wait out. Taking the
            // lock up front makes contention a wait instead of a failure, which is
            // what lets an LLM call's ledger row and the audit record that describes
            // it be written by concurrent callers at all.
            if let Err(e) = sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await {
                return Err(map_sqlx(e));
            }
            match insert_audit(&mut conn, record).await {
                Ok(()) => {
                    sqlx::query("COMMIT")
                        .execute(&mut *conn)
                        .await
                        .map_err(map_sqlx)?;
                    return Ok(());
                }
                Err(e) if is_unique_violation(&e) || is_transient_lock(&e) => {
                    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                    last_error = Some(MmError::Store(format!("audit chain contention: {e}")));
                    if attempt + 1 == MAX_CHAIN_ATTEMPTS {
                        break;
                    }
                }
                Err(e) => {
                    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                    return Err(map_sqlx(e));
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            MmError::Internal("audit chain stayed contended across every attempt".into())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::{Timestamp, UlidFactory};

    async fn store() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = SqliteStore::open(&dir.path().join("t.db")).await.unwrap();
        s.migrate().await.unwrap();
        (dir, s)
    }

    fn record(ids: &UlidFactory, code: &str, payload: serde_json::Value) -> AuditRecord {
        let mut r = AuditRecord::new(code, "info", "mm.test", None, payload);
        r.record_id = ids.next();
        r.at = Timestamp::from_rfc3339("2024-01-01T00:00:00.000000000Z").unwrap();
        r
    }

    #[tokio::test]
    async fn chain_is_gapless_linked_and_verifiable() {
        let (_d, s) = store().await;
        let ids = UlidFactory::new();
        for i in 0..5 {
            let rec = record(&ids, "kernel.boot", serde_json::json!({"i": i}));
            AuditWriter::write_audit(&s, &rec).await.unwrap();
        }
        let report = s.audit_chain_report().await.unwrap();
        assert!(report.is_ok(), "{report:?}");
        assert_eq!(report.rows, 5);
        assert_eq!(report.hashes_verified, 5);
        assert_eq!(report.first_gap_seq, None);
        assert!(report.summary().contains("chain intact"));

        let head = s.audit_head().await.unwrap().unwrap();
        assert_eq!(head.0, 5);
    }

    #[tokio::test]
    async fn an_empty_chain_is_ok() {
        let (_d, s) = store().await;
        let report = s.audit_chain_report().await.unwrap();
        assert!(report.is_ok());
        assert_eq!(report.rows, 0);
        assert!(s.audit_head().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rewriting_a_record_is_detected_by_rehashing() {
        let (_d, s) = store().await;
        let ids = UlidFactory::new();
        for i in 0..3 {
            let rec = record(&ids, "kernel.boot", serde_json::json!({"i": i}));
            AuditWriter::write_audit(&s, &rec).await.unwrap();
        }
        assert!(s.audit_chain_report().await.unwrap().is_ok());

        // UPDATE is refused outright by the trigger.
        let update = sqlx::query("UPDATE audit_log SET payload = '{}' WHERE seq = 2")
            .execute(s.pool())
            .await;
        assert!(update.is_err(), "audit_log must reject UPDATE");

        // And a rewrite performed *outside* the trigger (as an attacker with
        // filesystem access would do) is still caught by re-hashing.
        sqlx::raw_sql("DROP TRIGGER audit_log_no_update")
            .execute(s.pool())
            .await
            .unwrap();
        sqlx::query("UPDATE audit_log SET payload = '{\"i\":99}' WHERE seq = 2")
            .execute(s.pool())
            .await
            .unwrap();
        let report = s.audit_chain_report().await.unwrap();
        assert!(!report.is_ok());
        assert_eq!(report.hash_mismatch_seq, Some(2));
        assert!(report.summary().contains("does not match its own hash"));
    }

    #[tokio::test]
    async fn deleting_a_record_is_detected_as_a_gap_and_a_break() {
        let (_d, s) = store().await;
        let ids = UlidFactory::new();
        for i in 0..4 {
            let rec = record(&ids, "kernel.boot", serde_json::json!({"i": i}));
            AuditWriter::write_audit(&s, &rec).await.unwrap();
        }
        sqlx::raw_sql("DROP TRIGGER audit_log_no_delete")
            .execute(s.pool())
            .await
            .unwrap();
        sqlx::query("DELETE FROM audit_log WHERE seq = 2")
            .execute(s.pool())
            .await
            .unwrap();

        let report = s.audit_chain_report().await.unwrap();
        assert!(!report.is_ok());
        assert_eq!(report.first_gap_seq, Some(3));
        assert_eq!(report.first_broken_seq, Some(3));
        assert!(report.summary().contains("gap in audit sequence"));
    }

    #[tokio::test]
    async fn delete_is_refused_by_trigger_while_installed() {
        let (_d, s) = store().await;
        let ids = UlidFactory::new();
        AuditWriter::write_audit(&s, &record(&ids, "kernel.boot", serde_json::json!({})))
            .await
            .unwrap();
        let err = sqlx::query("DELETE FROM audit_log WHERE seq = 1")
            .execute(s.pool())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("append-only"));
    }

    #[tokio::test]
    async fn a_forked_chain_is_impossible() {
        let (_d, s) = store().await;
        let ids = UlidFactory::new();
        let rec = record(&ids, "kernel.boot", serde_json::json!({}));
        AuditWriter::write_audit(&s, &rec).await.unwrap();

        // Forge a second record claiming the same predecessor (the genesis).
        let forged = AuditRecord {
            record_id: ids.next(),
            event_code: "kernel.boot".into(),
            level: "info".into(),
            target: "mm.test".into(),
            trace_id: None,
            payload: serde_json::json!({"forged": true}),
            at: Timestamp::from_rfc3339("2024-01-01T00:00:00.000000000Z").unwrap(),
        };
        let mut tx = s.pool().begin().await.unwrap();
        let direct = sqlx::query(
            "INSERT INTO audit_log (seq, record_id, event_code, level, target, trace_id, payload, \
             prev_hash, hash, at) VALUES (2, ?, 'kernel.boot', 'info', 'mm.test', NULL, '{}', ?, ?, ?)",
        )
        .bind(mm_core::ulid_string(&forged.record_id))
        .bind(GENESIS_PREV)
        .bind(forged.chain_hash(Some(GENESIS_PREV)))
        .bind(forged.at.to_rfc3339())
        .execute(&mut *tx)
        .await;
        assert!(
            direct.is_err(),
            "a second genesis must be rejected by the unique index"
        );
        let _ = tx.rollback().await;
    }

    #[tokio::test]
    async fn concurrent_writers_produce_one_unforked_chain() {
        let (_d, s) = store().await;
        let mut handles = Vec::new();
        for t in 0..8 {
            let s = s.clone();
            handles.push(tokio::spawn(async move {
                let ids = UlidFactory::new();
                for i in 0..5 {
                    let rec = record(&ids, "kernel.boot", serde_json::json!({"t": t, "i": i}));
                    AuditWriter::write_audit(&s, &rec).await.unwrap();
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(s.audit_count().await.unwrap(), 40);
        let report = s.audit_chain_report().await.unwrap();
        assert!(report.is_ok(), "{report:?}");
    }
}
