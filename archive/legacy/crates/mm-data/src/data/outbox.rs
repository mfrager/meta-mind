//! The durable outbox between an RDF commit and a Tantivy document.
//!
//! A row is written **before** the RDF transaction, with `seq` equal to the
//! entity's `dataRevision`. That ordering is what closes the crash window: a
//! crash after the RDF commit but before indexing leaves a pending row, and the
//! worker checks the RDF revision before applying it. A row whose RDF write did
//! not land is discarded instead of resurrecting an entity.

use sqlx::SqlitePool;

use super::DataError;

/// A pending index operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    /// The `dataRevision` that produced it.
    pub seq: u64,
    /// `upsert` or `delete`.
    pub op: String,
    /// Entity IRI.
    pub subject: String,
    /// Authoritative graph IRI.
    pub graph: String,
    /// The document to index, for an upsert.
    pub doc_json: Option<String>,
    /// Failed attempts so far.
    pub attempts: u32,
}

impl OutboxEntry {
    /// Whether this row is a delete/tombstone.
    pub fn is_delete(&self) -> bool {
        self.op == "delete"
    }
}

/// The outbox over the data root's SQLite pool.
#[derive(Debug, Clone)]
pub struct DataOutbox {
    pool: SqlitePool,
}

impl DataOutbox {
    /// Open an outbox over `pool`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create the table when it is absent. Idempotent.
    pub async fn migrate(&self) -> Result<(), DataError> {
        for statement in DDL {
            sqlx::query(statement).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Write a pending row. `seq` is the entity's `dataRevision`.
    #[allow(clippy::too_many_arguments)]
    pub async fn enqueue(
        &self,
        seq: u64,
        op: &str,
        subject: &str,
        graph: &str,
        doc_json: Option<&str>,
        idempotency_key: Option<&str>,
        now: i64,
    ) -> Result<(), DataError> {
        sqlx::query(
            "INSERT INTO data_outbox
                (seq, op, subject, graph, doc_json, idempotency_key, attempts, next_attempt_at, created_at, done)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, ?7, 0)
             ON CONFLICT(seq) DO UPDATE SET
                op = excluded.op,
                subject = excluded.subject,
                graph = excluded.graph,
                doc_json = excluded.doc_json,
                attempts = 0,
                next_attempt_at = 0,
                done = 0",
        )
        .bind(seq as i64)
        .bind(op)
        .bind(subject)
        .bind(graph)
        .bind(doc_json)
        .bind(idempotency_key)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Rows that are due, oldest first.
    pub async fn pending(&self, now: i64, limit: i64) -> Result<Vec<OutboxEntry>, DataError> {
        let rows: Vec<(i64, String, String, String, Option<String>, i64)> = sqlx::query_as(
            "SELECT seq, op, subject, graph, doc_json, attempts
             FROM data_outbox
             WHERE done = 0 AND next_attempt_at <= ?1
             ORDER BY seq ASC
             LIMIT ?2",
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(seq, op, subject, graph, doc_json, attempts)| OutboxEntry {
                    seq: seq.max(0) as u64,
                    op,
                    subject,
                    graph,
                    doc_json,
                    attempts: attempts.max(0) as u32,
                },
            )
            .collect())
    }

    /// Mark a row as applied.
    pub async fn mark_done(&self, seq: u64) -> Result<(), DataError> {
        sqlx::query("UPDATE data_outbox SET done = 1 WHERE seq = ?1")
            .bind(seq as i64)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Record a failure and schedule a retry with exponential backoff.
    pub async fn mark_failed(
        &self,
        seq: u64,
        attempts: u32,
        next_attempt_at: i64,
    ) -> Result<(), DataError> {
        sqlx::query("UPDATE data_outbox SET attempts = ?2, next_attempt_at = ?3 WHERE seq = ?1")
            .bind(seq as i64)
            .bind(i64::from(attempts))
            .bind(next_attempt_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Discard a row whose RDF write never landed.
    pub async fn discard(&self, seq: u64) -> Result<(), DataError> {
        self.mark_done(seq).await
    }

    /// The highest `dataRevision` the outbox has seen.
    pub async fn max_seq(&self) -> Result<u64, DataError> {
        let value: Option<i64> = sqlx::query_scalar("SELECT MAX(seq) FROM data_outbox")
            .fetch_one(&self.pool)
            .await?;
        Ok(value.unwrap_or(0).max(0) as u64)
    }

    /// Pending rows, for status and health.
    pub async fn pending_count(&self) -> Result<u64, DataError> {
        let value: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM data_outbox WHERE done = 0")
            .fetch_one(&self.pool)
            .await?;
        Ok(value.max(0) as u64)
    }
}

/// The outbox DDL. The same table is created by migration 0013 for the kernel
/// database; this crate owns the live copy under `<data-root>/sqlite/`.
pub const DDL: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS data_outbox (
        seq INTEGER PRIMARY KEY,
        op TEXT NOT NULL CHECK (op IN ('upsert','delete')),
        subject TEXT NOT NULL,
        graph TEXT NOT NULL,
        doc_json TEXT,
        idempotency_key TEXT UNIQUE,
        attempts INTEGER NOT NULL DEFAULT 0,
        next_attempt_at INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        done INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0,1))
     )",
    "CREATE INDEX IF NOT EXISTS data_outbox_pending
        ON data_outbox (done, next_attempt_at, seq)",
    "CREATE INDEX IF NOT EXISTS data_outbox_subject
        ON data_outbox (subject)",
];
