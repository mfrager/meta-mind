//! The append-only event log.
//!
//! This is the source of truth. SQLite and Oxigraph are projections of it, so the
//! log is the only thing that has to be correct for a rebuild to be possible. The
//! write path is `append` → (`apply`) → `commit`, with the audited record for a
//! commit written **inside the same transaction**, so a crash can never leave a
//! committed state change without its audit entry.

use std::sync::Arc;

use mm_core::store::NewEvent;
use mm_core::{AuditRecord, MmError, Timestamp, Ulid};
use mm_log::{codes, Level, LogRecord, Logger};
use mm_store_sqlite::{write_audit_in_tx, AsOf, SqliteStore};
use sqlx::Row;

use crate::record::{EventRecord, EventStatus};
use crate::replay::{fold_records, EventApplier, StateSnapshot};

/// The log's subsystem name in every record it emits.
pub const TARGET: &str = "mm.eventlog";

/// The checkpoint name used for the projection's own progress.
pub const DEFAULT_CHECKPOINT: &str = "projection";

/// The append-only event log.
pub struct EventLog {
    store: SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<mm_core::UlidFactory>,
}

impl std::fmt::Debug for EventLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventLog")
            .field("db", &self.store.path())
            .finish_non_exhaustive()
    }
}

impl EventLog {
    /// Build a log over an open store.
    pub fn new(store: SqliteStore, logger: Arc<Logger>, ids: Arc<mm_core::UlidFactory>) -> Self {
        EventLog { store, logger, ids }
    }

    /// The underlying store.
    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// Append an event, timestamped now.
    pub async fn append(&self, event: NewEvent) -> Result<Ulid, MmError> {
        self.append_at(event, Timestamp::now()).await
    }

    /// Append an event with an explicit creation time.
    ///
    /// The event's content hash covers `created_at`, so re-appending the same
    /// payload *and* the same instant is deduplicated: the second call returns
    /// the identifier of the first. That is what makes a retried append safe
    /// after a crash.
    pub async fn append_at(&self, event: NewEvent, created_at: Timestamp) -> Result<Ulid, MmError> {
        let payload = event.canonical_payload()?;
        let kind = event.kind.as_str();
        let created = created_at.to_rfc3339();
        let hash = mm_core::hash_fields(&[kind, &payload, &created]);
        let correlation = event.correlation;

        let _guard = self.store.audit_lock().lock().await;
        let mut tx = self.store.pool().begin().await.map_err(store_err)?;

        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM events WHERE hash = ?")
            .bind(&hash)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_err)?;
        if let Some(existing) = existing {
            tx.commit().await.map_err(store_err)?;
            return mm_core::id::parse_ulid(&existing);
        }

        let seq: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) + 1 FROM events")
            .fetch_one(&mut *tx)
            .await
            .map_err(store_err)?;
        let id = self.ids.next();
        sqlx::query(
            "INSERT INTO events (seq, id, kind, payload, status, correlation, system_from, \
             created_at, hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(seq)
        .bind(mm_core::ulid_string(&id))
        .bind(kind)
        .bind(&payload)
        .bind(EventStatus::Provisional.as_str())
        .bind(correlation.map(|c| mm_core::ulid_string(&c)))
        .bind(&created)
        .bind(&created)
        .bind(&hash)
        .execute(&mut *tx)
        .await
        .map_err(store_err)?;
        tx.commit().await.map_err(store_err)?;

        self.logger
            .emit(
                LogRecord::new(Level::Debug, codes::EVENTLOG_APPEND, TARGET)
                    .with_field("id", mm_core::ulid_string(&id))
                    .with_field("kind", kind)
                    .with_field("seq", seq)
                    .with_field("correlation", correlation.map(|c| mm_core::ulid_string(&c))),
            )
            .await?;
        Ok(id)
    }

    /// Commit an appended event and write its audited record atomically.
    pub async fn commit(&self, id: &Ulid) -> Result<(), MmError> {
        let id_text = mm_core::ulid_string(id);
        let now = Timestamp::now();

        let _guard = self.store.audit_lock().lock().await;
        let mut tx = self.store.pool().begin().await.map_err(store_err)?;

        let row: Option<(i64, String, String, String)> =
            sqlx::query_as("SELECT seq, kind, status, hash FROM events WHERE id = ?")
                .bind(&id_text)
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_err)?;
        let Some((seq, kind, status, hash)) = row else {
            return Err(MmError::Event(format!("no event {id}")));
        };
        if status != EventStatus::Provisional.as_str() {
            return Err(MmError::Event(format!(
                "event {id} is {status}, not provisional; refusing to commit it"
            )));
        }

        let affected = sqlx::query(
            "UPDATE events SET status = 'committed', system_from = ? \
             WHERE id = ? AND status = 'provisional'",
        )
        .bind(now.to_rfc3339())
        .bind(&id_text)
        .execute(&mut *tx)
        .await
        .map_err(store_err)?
        .rows_affected();
        if affected != 1 {
            return Err(MmError::Event(format!(
                "event {id} changed state under the transaction"
            )));
        }

        let audit = AuditRecord {
            record_id: *id,
            event_code: codes::EVENTLOG_COMMIT.to_string(),
            level: "info".to_string(),
            target: TARGET.to_string(),
            trace_id: None,
            payload: serde_json::json!({
                "id": id_text,
                "seq": seq,
                "kind": kind,
                "event_hash": hash,
            }),
            at: now,
        };
        write_audit_in_tx(&mut tx, &audit).await?;
        tx.commit().await.map_err(store_err)?;

        self.logger
            .emit(
                LogRecord::new(Level::Info, codes::EVENTLOG_COMMIT, TARGET)
                    .with_field("id", id_text)
                    .with_field("seq", seq)
                    .with_field("kind", kind),
            )
            .await?;
        Ok(())
    }

    /// Append, apply to a projection, then commit.
    ///
    /// If the projection rejects the event, the event is aborted and the error is
    /// returned: a failed apply never leaves a committed event behind.
    pub async fn append_and_apply<A: EventApplier + ?Sized>(
        &self,
        event: NewEvent,
        applier: &mut A,
    ) -> Result<Ulid, MmError> {
        let id = self.append(event).await?;
        let record = self
            .record(&id)
            .await?
            .ok_or_else(|| MmError::Event(format!("event {id} vanished after append")))?;
        if let Err(e) = applier.apply(&record) {
            self.abort(&id, "projection rejected the event").await?;
            return Err(e);
        }
        self.commit(&id).await?;
        Ok(id)
    }

    /// Abort one provisional event.
    pub async fn abort(&self, id: &Ulid, reason: &str) -> Result<(), MmError> {
        let id_text = mm_core::ulid_string(id);
        let now = Timestamp::now();
        let _guard = self.store.audit_lock().lock().await;
        let mut tx = self.store.pool().begin().await.map_err(store_err)?;

        let row: Option<(i64, String)> =
            sqlx::query_as("SELECT seq, kind FROM events WHERE id = ?")
                .bind(&id_text)
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_err)?;
        let Some((seq, kind)) = row else {
            return Err(MmError::Event(format!("no event {id}")));
        };
        let affected = sqlx::query(
            "UPDATE events SET status = 'aborted' WHERE id = ? AND status = 'provisional'",
        )
        .bind(&id_text)
        .execute(&mut *tx)
        .await
        .map_err(store_err)?
        .rows_affected();
        if affected != 1 {
            return Err(MmError::Event(format!("event {id} is not provisional")));
        }
        let audit = AuditRecord {
            record_id: *id,
            event_code: codes::EVENTLOG_ABORT.to_string(),
            level: "warn".to_string(),
            target: TARGET.to_string(),
            trace_id: None,
            payload: serde_json::json!({"id": id_text, "seq": seq, "kind": kind, "reason": reason}),
            at: now,
        };
        write_audit_in_tx(&mut tx, &audit).await?;
        tx.commit().await.map_err(store_err)?;

        self.logger
            .emit(
                LogRecord::new(Level::Warn, codes::EVENTLOG_ABORT, TARGET)
                    .with_field("id", id_text)
                    .with_field("seq", seq)
                    .with_field("reason", reason),
            )
            .await?;
        Ok(())
    }

    /// Abort every event left `provisional` by an interrupted run.
    ///
    /// Called at the start of every replay. Idempotent: a second call finds
    /// nothing to abort, which is what makes "exactly once" hold.
    pub async fn abort_provisional(&self) -> Result<Vec<Ulid>, MmError> {
        let pending = self.records_with_status(EventStatus::Provisional).await?;
        let mut aborted = Vec::with_capacity(pending.len());
        for record in pending {
            self.abort(&record.id, "interrupted before commit").await?;
            aborted.push(record.id);
        }
        Ok(aborted)
    }

    /// Replay committed events in `seq` order into an applier.
    pub async fn replay(
        &self,
        as_of: AsOf,
        applier: &mut dyn EventApplier,
    ) -> Result<StateSnapshot, MmError> {
        let trace = self.ids.next();
        let until = as_of.effective_system();
        self.logger
            .emit(
                LogRecord::new(Level::Info, codes::EVENTLOG_REPLAY_START, TARGET)
                    .with_trace(trace)
                    .with_field("from_seq", 1)
                    .with_field("until_seq", if as_of.system.is_some() { until } else { -1 })
                    .with_field("source", "eventlog"),
            )
            .await?;

        let aborted = self.abort_provisional().await?;
        let records = self.committed_records(1, until).await?;
        for record in &records {
            applier.apply(record)?;
            self.logger
                .emit(
                    LogRecord::new(Level::Debug, codes::EVENTLOG_APPLY, TARGET)
                        .with_trace(trace)
                        .with_field("id", mm_core::ulid_string(&record.id))
                        .with_field("seq", record.seq),
                )
                .await?;
        }

        let log_hash = fold_records(&records);
        let last_seq = records.last().map_or(0, |r| r.seq);
        // A checkpoint describes the state *at its own sequence*. Comparing it to
        // a replay that stops anywhere else would be meaningless (the checkpoint
        // event itself is part of the log), so `match` is `None` when the two are
        // not describing the same point in the log.
        let checkpoint = self.checkpoint_of(DEFAULT_CHECKPOINT).await?;
        let (expected, matches) = match &checkpoint {
            Some((seq, hash)) if *seq == last_seq => (Some(hash.clone()), Some(hash == &log_hash)),
            Some((_, hash)) => (Some(hash.clone()), None),
            None => (None, None),
        };
        let snapshot = StateSnapshot {
            applied: records.len() as u64,
            last_seq,
            log_hash: log_hash.clone(),
            applier_hash: applier.state_hash(),
            aborted,
        };

        self.logger
            .emit(
                LogRecord::new(Level::Info, codes::EVENTLOG_REPLAY_END, TARGET)
                    .with_trace(trace)
                    .with_field("events_applied", snapshot.applied)
                    .with_field("state_hash", &log_hash)
                    .with_field("expected_hash", expected.clone())
                    .with_field("match", matches),
            )
            .await?;
        Ok(snapshot)
    }

    /// One event by identifier.
    pub async fn record(&self, id: &Ulid) -> Result<Option<EventRecord>, MmError> {
        let row = sqlx::query("SELECT * FROM events WHERE id = ?")
            .bind(mm_core::ulid_string(id))
            .fetch_optional(self.store.pool())
            .await
            .map_err(store_err)?;
        row.as_ref().map(EventRecord::from_row).transpose()
    }

    /// Every event, in `seq` order, whatever its status.
    pub async fn all_records(&self) -> Result<Vec<EventRecord>, MmError> {
        let rows = sqlx::query("SELECT * FROM events ORDER BY seq")
            .fetch_all(self.store.pool())
            .await
            .map_err(store_err)?;
        rows.iter().map(EventRecord::from_row).collect()
    }

    /// Committed events with `from <= seq <= until`.
    pub async fn committed_records(
        &self,
        from: i64,
        until: i64,
    ) -> Result<Vec<EventRecord>, MmError> {
        let rows = sqlx::query(
            "SELECT * FROM events WHERE status = 'committed' AND seq >= ? AND seq <= ? ORDER BY seq",
        )
        .bind(from)
        .bind(until)
        .fetch_all(self.store.pool())
        .await
        .map_err(store_err)?;
        rows.iter().map(EventRecord::from_row).collect()
    }

    /// Events with a given status.
    pub async fn records_with_status(
        &self,
        status: EventStatus,
    ) -> Result<Vec<EventRecord>, MmError> {
        let rows = sqlx::query("SELECT * FROM events WHERE status = ? ORDER BY seq")
            .bind(status.as_str())
            .fetch_all(self.store.pool())
            .await
            .map_err(store_err)?;
        rows.iter().map(EventRecord::from_row).collect()
    }

    /// The highest sequence committed so far, or 0.
    pub async fn head_seq(&self) -> Result<i64, MmError> {
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MAX(seq) FROM events WHERE status = 'committed'",
        )
        .fetch_one(self.store.pool())
        .await
        .map(|v| v.unwrap_or(0))
        .map_err(store_err)
    }

    /// `(provisional, committed, aborted)` counts.
    pub async fn status_counts(&self) -> Result<(i64, i64, i64), MmError> {
        let rows = sqlx::query("SELECT status, count(*) AS n FROM events GROUP BY status")
            .fetch_all(self.store.pool())
            .await
            .map_err(store_err)?;
        let (mut p, mut c, mut a) = (0i64, 0i64, 0i64);
        for row in rows {
            let status: String = row.try_get("status").map_err(store_err)?;
            let n: i64 = row.try_get("n").map_err(store_err)?;
            match status.as_str() {
                "provisional" => p = n,
                "committed" => c = n,
                "aborted" => a = n,
                _ => {}
            }
        }
        Ok((p, c, a))
    }

    /// Record a checkpoint of a projection's progress.
    ///
    /// Stored as an event so a checkpoint is part of the log's own history, and
    /// mirrored into `store_checkpoints` so a reader can find it in one query.
    pub async fn checkpoint(&self, name: &str, seq: i64, state_hash: &str) -> Result<(), MmError> {
        let created = Timestamp::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO store_checkpoints (name, seq, state_hash, created_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(name) DO UPDATE SET seq = excluded.seq, state_hash = excluded.state_hash, \
             created_at = excluded.created_at",
        )
        .bind(name)
        .bind(seq)
        .bind(state_hash)
        .bind(&created)
        .execute(self.store.pool())
        .await
        .map_err(store_err)?;

        let payload = serde_json::json!({"name": name, "seq": seq, "state_hash": state_hash});
        let _ = self
            .logger
            .emit(
                LogRecord::new(Level::Info, codes::EVENTLOG_COMMIT, TARGET)
                    .with_field("checkpoint", name)
                    .with_field("seq", seq)
                    .with_field("state_hash", state_hash),
            )
            .await;
        let event = NewEvent {
            kind: mm_core::EventKind::Checkpoint,
            payload,
            correlation: None,
        };
        let id = self.append(event).await?;
        self.commit(&id).await?;
        Ok(())
    }

    /// The stored checkpoint `(seq, state_hash)`.
    pub async fn checkpoint_of(&self, name: &str) -> Result<Option<(i64, String)>, MmError> {
        let row: Option<(i64, String)> =
            sqlx::query_as("SELECT seq, state_hash FROM store_checkpoints WHERE name = ?")
                .bind(name)
                .fetch_optional(self.store.pool())
                .await
                .map_err(store_err)?;
        Ok(row)
    }
}

fn store_err(e: sqlx::Error) -> MmError {
    MmError::Store(e.to_string())
}
