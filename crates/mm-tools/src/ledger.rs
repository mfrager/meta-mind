//! The append-only, hash-chained action ledger.
//!
//! The ledger is the journal Phase 10's fourth invariant depends on: an effect that
//! happened once is recorded once, and a record cannot be edited or removed without
//! the chain saying so. Three properties carry that, and each is a mechanism rather
//! than a convention:
//!
//! * **Append-only by construction.** There is no `update` and no `delete` in this
//!   module, and migration `0010_tools.sql` installs `action_ledger_no_update` /
//!   `action_ledger_no_delete` triggers, so the guarantee survives a caller that
//!   reaches past this module into SQL.
//! * **A hash chain.** Every entry's [`LedgerEntry::entry_hash`] covers its
//!   `prev_hash`, its `id`, its `event` and its canonical payload, so editing any of
//!   those four changes the hash and every later entry's link breaks with it.
//! * **One writer at a time.** [`append`] takes the store's audit lock and reads the
//!   head, computes and inserts inside **one transaction**, so two concurrent appends
//!   cannot both chain onto the same predecessor.
//!
//! # What `verify_chain` detects, and why each shape has its own error
//!
//! Four tamper shapes are named, because "the ledger is broken" is not actionable:
//!
//! | Shape | Error | What it means |
//! |---|---|---|
//! | a mutated payload | [`LedgerError::HashMismatch`] | a row's content no longer hashes to its stored hash |
//! | a deleted row | [`LedgerError::Gap`] | the sequence is not gapless |
//! | a spliced-in row | [`LedgerError::ChainBroken`] | a row's `prev_hash` does not name its predecessor's hash |
//! | a rewritten first row | [`LedgerError::BadGenesis`] | the chain does not start at [`GENESIS_HASH`] |
//!
//! A chain can only be *checked* forwards, so [`verify_chain`] reads every entry and
//! reports the first defect it finds, at the lowest sequence that is wrong — the
//! earliest point at which the history is no longer trustworthy.

use mm_core::{Timestamp, Ulid};
use mm_store_sqlite::SqliteStore;

use crate::error::LedgerError;

/// The `prev_hash` the first entry names: 64 zeros.
///
/// A fixed, impossible hash rather than `NULL`, so "the first entry" is a property of
/// the *content* and not of the row's position: a chain whose first row was dropped
/// fails [`LedgerError::BadGenesis`] rather than looking like a fresh ledger.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// True when `hash` is the genesis hash.
pub fn is_genesis(hash: &str) -> bool {
    hash == GENESIS_HASH
}

/// The lifecycle events this phase writes.
///
/// A closed set, and [`append`] refuses anything else. The ledger is read by
/// `mm-cli action ledger`, by `logs verify` and by a later replay; an event code
/// invented at a call site would make all three show a row they cannot interpret,
/// which is why the list is here rather than in each caller.
pub const LEDGER_EVENTS: [&str; 8] = [
    "action.proposed",
    "permission.check",
    "sandbox.start",
    "sandbox.deny",
    "invoke.end",
    "observation.record",
    "rollback.start",
    "rollback.end",
];

/// True when `event` is one of [`LEDGER_EVENTS`].
pub fn is_known_event(event: &str) -> bool {
    LEDGER_EVENTS.contains(&event)
}

/// One entry in the chain.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LedgerEntry {
    /// The gapless sequence number, from 1.
    pub seq: i64,
    /// The entry's own ULID.
    pub id: Ulid,
    /// The action this entry belongs to.
    pub action_id: Ulid,
    /// The lifecycle event.
    pub event: String,
    /// The payload, as written.
    pub payload: serde_json::Value,
    /// The hash of the entry before it, or [`GENESIS_HASH`].
    pub prev_hash: String,
    /// This entry's hash.
    pub entry_hash: String,
    /// When it was written.
    pub at: Timestamp,
}

impl LedgerEntry {
    /// The exact string this entry's hash covers.
    ///
    /// `id` and `event` rather than the sequence number, so an entry's hash is a
    /// statement about *what happened* and not about where in the file it landed; the
    /// sequence is the chain's, and [`verify_chain`] checks it separately.
    pub fn canonical(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            mm_core::ulid_string(&self.id),
            self.event,
            crate::canonical_json(&self.payload)
        )
    }

    /// The hash this entry's stored fields produce.
    pub fn recompute(&self) -> String {
        entry_hash(
            &self.prev_hash,
            &mm_core::ulid_string(&self.id),
            &self.event,
            &crate::canonical_json(&self.payload),
        )
    }

    /// True when the stored hash matches the entry's own content.
    pub fn is_intact(&self) -> bool {
        self.recompute() == self.entry_hash
    }
}

/// Where an entry landed.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LedgerRef {
    /// The sequence number it was written at.
    pub seq: i64,
    /// Its ULID.
    pub id: Ulid,
    /// Its hash, which the next entry will name.
    pub entry_hash: String,
}

/// The hash of one chain link.
///
/// `sha256(prev_hash || id || event || payload_json)`, concatenated with no separator.
/// That is unambiguous without one because the first field is always exactly 64
/// lowercase hex characters, the second exactly 26 Crockford characters, and the event
/// code contains no JSON, so no two different `(id, event, payload)` triples can
/// produce the same concatenation — the boundary between the fields is derivable
/// rather than guessed.
pub fn entry_hash(prev_hash: &str, id: &str, event: &str, payload_json: &str) -> String {
    let mut joined =
        String::with_capacity(prev_hash.len() + id.len() + event.len() + payload_json.len());
    joined.push_str(prev_hash);
    joined.push_str(id);
    joined.push_str(event);
    joined.push_str(payload_json);
    mm_core::content_hash(joined.as_bytes())
}

/// Append an entry, timestamped now.
pub async fn append(
    sql: &SqliteStore,
    action_id: Ulid,
    event: &str,
    payload: &serde_json::Value,
) -> Result<LedgerRef, LedgerError> {
    append_at(sql, action_id, event, payload, Timestamp::now()).await
}

/// Append an entry with an explicit instant.
///
/// The instant is a parameter because the ledger is compared byte for byte across a
/// replay: a caller that re-runs a recorded action must be able to reproduce the same
/// row, and a hard-coded `now()` would make that impossible in principle.
pub async fn append_at(
    sql: &SqliteStore,
    action_id: Ulid,
    event: &str,
    payload: &serde_json::Value,
    at: Timestamp,
) -> Result<LedgerRef, LedgerError> {
    if !is_known_event(event) {
        return Err(LedgerError::Store(format!(
            "{event:?} is not a lifecycle event this ledger records; \
             known events are {}",
            LEDGER_EVENTS.join(", ")
        )));
    }

    // One writer at a time: the head is read, the entry is derived from it and the row
    // is inserted inside the same critical section, so two appends cannot both name the
    // same predecessor.
    let _guard = sql.audit_lock().lock().await;
    let mut tx = sql.pool().begin().await.map_err(store_err)?;

    let last: Option<(i64, String, String, String, String, String)> = sqlx::query_as(
        "SELECT seq, id, event, payload_json, prev_hash, entry_hash FROM action_ledger \
         ORDER BY seq DESC LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(store_err)?;

    let (seq, prev_hash) = match last {
        Some((last_seq, last_id, last_event, last_payload, last_prev, stored_hash)) => {
            // Refuse to extend a chain whose head has already been edited: chaining onto
            // a hash that does not match its own row would launder the tampering into
            // every entry written afterwards.
            let recomputed = entry_hash(&last_prev, &last_id, &last_event, &last_payload);
            if recomputed != stored_hash {
                return Err(LedgerError::HashMismatch {
                    seq: last_seq,
                    stored: stored_hash,
                    computed: recomputed,
                });
            }
            (last_seq + 1, stored_hash)
        }
        None => (1, GENESIS_HASH.to_string()),
    };

    let payload_json = crate::canonical_json(payload);
    let id = entry_id(&mm_core::ulid_string(&action_id), event, &payload_json, seq);
    let id_text = mm_core::ulid_string(&id);
    let hash = entry_hash(&prev_hash, &id_text, event, &payload_json);

    sqlx::query(
        "INSERT INTO action_ledger (seq, id, action_id, event, payload_json, prev_hash, \
         entry_hash, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(seq)
    .bind(&id_text)
    .bind(mm_core::ulid_string(&action_id))
    .bind(event)
    .bind(&payload_json)
    .bind(&prev_hash)
    .bind(&hash)
    .bind(at.to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(store_err)?;
    tx.commit().await.map_err(store_err)?;

    Ok(LedgerRef {
        seq,
        id,
        entry_hash: hash,
    })
}

/// Every entry for one action, in sequence order.
pub async fn entries_for(
    sql: &SqliteStore,
    action_id: &Ulid,
) -> Result<Vec<LedgerEntry>, LedgerError> {
    let rows = fetch(
        sql,
        "WHERE action_id = ?",
        Some(mm_core::ulid_string(action_id)),
    )
    .await?;
    Ok(rows)
}

/// The newest `limit` entries, oldest first.
pub async fn tail(sql: &SqliteStore, limit: i64) -> Result<Vec<LedgerEntry>, LedgerError> {
    let mut rows: Vec<LedgerEntry> = sqlx::query_as::<_, Row>(
        "SELECT seq, id, action_id, event, payload_json, prev_hash, entry_hash, at \
         FROM action_ledger ORDER BY seq DESC LIMIT ?",
    )
    .bind(limit.max(0))
    .fetch_all(sql.pool())
    .await
    .map_err(store_err)?
    .into_iter()
    .map(Row::into_entry)
    .collect::<Result<Vec<_>, _>>()?;
    rows.reverse();
    Ok(rows)
}

/// The sequence and hash of the last entry, or `(0, GENESIS_HASH)` for an empty chain.
pub async fn head(sql: &SqliteStore) -> Result<(i64, String), LedgerError> {
    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT seq, entry_hash FROM action_ledger ORDER BY seq DESC LIMIT 1")
            .fetch_optional(sql.pool())
            .await
            .map_err(store_err)?;
    Ok(row.unwrap_or((0, GENESIS_HASH.to_string())))
}

/// Verify the whole chain, returning how many entries were checked.
///
/// The check is `append`'s, applied to every row: the first row must name
/// [`GENESIS_HASH`], each row's content must hash to its stored hash, and each row
/// must name its predecessor's hash.
pub async fn verify_chain(sql: &SqliteStore) -> Result<i64, LedgerError> {
    let entries = fetch(sql, "", None).await?;
    recompute_chain(&entries)?;
    Ok(entries.len() as i64)
}

/// The same check over entries already in hand, without a store.
///
/// Shared with the tests and with a replay that has read the ledger into memory, so
/// there is one definition of "intact" rather than two that can drift.
pub fn recompute_chain(entries: &[LedgerEntry]) -> Result<(), LedgerError> {
    let mut previous: Option<&LedgerEntry> = None;
    for (index, entry) in entries.iter().enumerate() {
        let expected_seq = index as i64 + 1;
        if entry.seq != expected_seq {
            return Err(LedgerError::Gap {
                expected: expected_seq,
                found: entry.seq,
            });
        }
        match previous {
            None => {
                if !is_genesis(&entry.prev_hash) {
                    return Err(LedgerError::BadGenesis {
                        found: entry.prev_hash.clone(),
                    });
                }
            }
            Some(previous) => {
                if entry.prev_hash != previous.entry_hash {
                    return Err(LedgerError::ChainBroken {
                        seq: entry.seq,
                        named: entry.prev_hash.clone(),
                        head: previous.entry_hash.clone(),
                    });
                }
            }
        }
        if !entry.is_intact() {
            return Err(LedgerError::HashMismatch {
                seq: entry.seq,
                stored: entry.entry_hash.clone(),
                computed: entry.recompute(),
            });
        }
        previous = Some(entry);
    }
    Ok(())
}

/// Verify the chain on open, so a tampered ledger is reported at startup rather than
/// at the first append.
pub async fn ensure_chain(sql: &SqliteStore) -> Result<i64, LedgerError> {
    verify_chain(sql).await
}

/// A deterministic entry id.
///
/// Derived from the action, the event, the canonical payload and the sequence, so a
/// replay of the same recorded call produces the same row — and so the id cannot
/// collide with itself when the same event legitimately recurs for one action (a
/// `permission.check` per capability, for instance), because the sequence differs.
fn entry_id(action_id: &str, event: &str, payload_json: &str, seq: i64) -> Ulid {
    let digest = mm_core::content_hash(
        format!("{action_id}\u{1f}{event}\u{1f}{payload_json}\u{1f}{seq}").as_bytes(),
    );
    let bytes = digest.as_bytes();
    let mut parts = [0u8; 16];
    for (index, slot) in parts.iter_mut().enumerate() {
        *slot = (hex_nibble(bytes[index * 2]) << 4) | hex_nibble(bytes[index * 2 + 1]);
    }
    parts[0] &= 0b0000_0111;
    Ulid::from_bytes(parts)
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

/// A typed row, so the `query_as` decoding lives in one place.
#[derive(sqlx::FromRow)]
struct Row {
    seq: i64,
    id: String,
    action_id: String,
    event: String,
    payload_json: String,
    prev_hash: String,
    entry_hash: String,
    at: String,
}

impl Row {
    fn into_entry(self) -> Result<LedgerEntry, LedgerError> {
        Ok(LedgerEntry {
            seq: self.seq,
            id: parse_ulid(&self.id)?,
            action_id: parse_ulid(&self.action_id)?,
            event: self.event,
            payload: serde_json::from_str(&self.payload_json).map_err(|e| {
                LedgerError::Store(format!("entry {} has invalid payload JSON: {e}", self.seq))
            })?,
            prev_hash: self.prev_hash,
            entry_hash: self.entry_hash,
            at: Timestamp::from_rfc3339(&self.at).map_err(|e| {
                LedgerError::Store(format!("entry {} has an invalid timestamp: {e}", self.seq))
            })?,
        })
    }
}

/// Read entries, optionally filtered.
async fn fetch(
    sql: &SqliteStore,
    filter: &str,
    bind: Option<String>,
) -> Result<Vec<LedgerEntry>, LedgerError> {
    let sql_text = format!(
        "SELECT seq, id, action_id, event, payload_json, prev_hash, entry_hash, at \
         FROM action_ledger {filter} ORDER BY seq"
    );
    let mut query = sqlx::query_as::<_, Row>(&sql_text);
    if let Some(bind) = bind {
        query = query.bind(bind);
    }
    query
        .fetch_all(sql.pool())
        .await
        .map_err(store_err)?
        .into_iter()
        .map(Row::into_entry)
        .collect()
}

fn parse_ulid(text: &str) -> Result<Ulid, LedgerError> {
    mm_core::id::parse_ulid(text).map_err(|e| LedgerError::Store(format!("bad ULID {text:?}: {e}")))
}

fn store_err(e: sqlx::Error) -> LedgerError {
    LedgerError::Store(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Executor;

    /// An open, migrated store in a temporary directory.
    ///
    /// The directory is leaked on purpose: the store holds a pool over the file, and a
    /// test that dropped the directory first would fail for the wrong reason.
    async fn store() -> SqliteStore {
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let sql = SqliteStore::open(&dir.path().join("ledger.db"))
            .await
            .unwrap();
        sql.migrate().await.unwrap();
        sql
    }

    fn action(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    /// Drop the append-only triggers on `action_ledger`, whatever they are named.
    ///
    /// The tamper tests have to *reach* the rows to prove the hash chain catches an
    /// edit, and migration `0010_tools.sql` refuses `UPDATE` and `DELETE` on this
    /// table outright. Disarming the triggers is therefore the honest way to test the
    /// second line of defence: the first is the trigger, and this is what the ledger
    /// still catches when something has already got past it. The names are read from
    /// `sqlite_master` so the test does not depend on what the migration named them.
    async fn disarm_triggers(sql: &SqliteStore) {
        let names: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'trigger' AND tbl_name = \
             'action_ledger'",
        )
        .fetch_all(sql.pool())
        .await
        .unwrap();
        for name in names {
            let drop = format!("DROP TRIGGER IF EXISTS {name}");
            sql.pool().execute(drop.as_str()).await.unwrap();
        }
    }

    fn at(seconds: u64) -> Timestamp {
        Timestamp::from_epoch_seconds(seconds)
    }

    #[tokio::test]
    async fn an_appended_entry_is_readable_and_starts_the_chain_at_genesis() {
        let sql = store().await;
        let reference = append_at(
            &sql,
            action(1),
            "action.proposed",
            &serde_json::json!({ "tool": "fs.read" }),
            at(1_700_000_000),
        )
        .await
        .unwrap();
        assert_eq!(reference.seq, 1);

        let entries = entries_for(&sql, &action(1)).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].prev_hash, GENESIS_HASH);
        assert_eq!(entries[0].entry_hash, reference.entry_hash);
        assert!(entries[0].is_intact());
        assert_eq!(entries[0].payload["tool"], "fs.read");
        assert_eq!(entries[0].at, at(1_700_000_000));
    }

    #[tokio::test]
    async fn each_entry_names_the_hash_of_the_one_before_it() {
        let sql = store().await;
        let first = append_at(
            &sql,
            action(1),
            "action.proposed",
            &serde_json::json!({}),
            at(1),
        )
        .await
        .unwrap();
        let second = append_at(&sql, action(1), "invoke.end", &serde_json::json!({}), at(2))
            .await
            .unwrap();
        assert_eq!(second.seq, first.seq + 1);
        let entries = entries_for(&sql, &action(1)).await.unwrap();
        assert_eq!(entries[1].prev_hash, first.entry_hash);
        assert_eq!(head(&sql).await.unwrap(), (2, second.entry_hash));
    }

    #[tokio::test]
    async fn an_empty_ledger_heads_at_genesis() {
        let sql = store().await;
        assert_eq!(head(&sql).await.unwrap().0, 0);
        assert!(is_genesis(&head(&sql).await.unwrap().1));
        assert_eq!(verify_chain(&sql).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_chain_of_eight_verifies() {
        let sql = store().await;
        for n in 0..8 {
            append_at(
                &sql,
                action(1),
                LEDGER_EVENTS[n % LEDGER_EVENTS.len()],
                &serde_json::json!({ "n": n }),
                at(1_700_000_000 + n as u64),
            )
            .await
            .unwrap();
        }
        assert_eq!(verify_chain(&sql).await.unwrap(), 8);
        assert_eq!(tail(&sql, 3).await.unwrap().len(), 3);
        assert_eq!(tail(&sql, 100).await.unwrap().len(), 8);
    }

    #[tokio::test]
    async fn a_mutated_payload_is_detected() {
        let sql = store().await;
        append_at(
            &sql,
            action(1),
            "action.proposed",
            &serde_json::json!({ "n": 1 }),
            at(1),
        )
        .await
        .unwrap();
        append_at(
            &sql,
            action(1),
            "invoke.end",
            &serde_json::json!({ "n": 2 }),
            at(2),
        )
        .await
        .unwrap();
        disarm_triggers(&sql).await;
        sql.pool()
            .execute("UPDATE action_ledger SET payload_json = '{\"n\":99}' WHERE seq = 1")
            .await
            .unwrap();
        let error = verify_chain(&sql).await.unwrap_err();
        assert_eq!(error.code(), "ledger.hash_mismatch");
        assert!(matches!(error, LedgerError::HashMismatch { seq: 1, .. }));
        assert!(error.to_string().contains("hashes to"));
    }

    #[tokio::test]
    async fn editing_a_row_hash_too_does_not_hide_the_edit() {
        let sql = store().await;
        append_at(
            &sql,
            action(1),
            "action.proposed",
            &serde_json::json!({ "n": 1 }),
            at(1),
        )
        .await
        .unwrap();
        append_at(
            &sql,
            action(1),
            "invoke.end",
            &serde_json::json!({ "n": 2 }),
            at(2),
        )
        .await
        .unwrap();
        // A thorough attacker rewrites the row *and* its hash, so only the link to the
        // next entry betrays it.
        disarm_triggers(&sql).await;
        sql.pool()
            .execute("UPDATE action_ledger SET payload_json = '{\"n\":99}' WHERE seq = 1")
            .await
            .unwrap();
        let entries = entries_for(&sql, &action(1)).await.unwrap();
        let forged = entry_hash(
            &entries[0].prev_hash,
            &mm_core::ulid_string(&entries[0].id),
            &entries[0].event,
            &crate::canonical_json(&serde_json::json!({ "n": 99 })),
        );
        sqlx::query("UPDATE action_ledger SET entry_hash = ? WHERE seq = 1")
            .bind(&forged)
            .execute(sql.pool())
            .await
            .unwrap();
        let error = verify_chain(&sql).await.unwrap_err();
        assert_eq!(error.code(), "ledger.chain_broken");
        assert!(matches!(error, LedgerError::ChainBroken { seq: 2, .. }));
    }

    #[tokio::test]
    async fn a_deleted_row_is_detected_as_a_gap() {
        let sql = store().await;
        for n in 0..4 {
            append_at(
                &sql,
                action(1),
                "invoke.end",
                &serde_json::json!({ "n": n }),
                at(n as u64 + 1),
            )
            .await
            .unwrap();
        }
        disarm_triggers(&sql).await;
        sql.pool()
            .execute("DELETE FROM action_ledger WHERE seq = 3")
            .await
            .unwrap();
        let error = verify_chain(&sql).await.unwrap_err();
        assert_eq!(error.code(), "ledger.gap");
        assert!(matches!(
            error,
            LedgerError::Gap {
                expected: 3,
                found: 4
            }
        ));
    }

    #[tokio::test]
    async fn a_spliced_in_self_consistent_row_is_detected() {
        let sql = store().await;
        append_at(
            &sql,
            action(1),
            "action.proposed",
            &serde_json::json!({}),
            at(1),
        )
        .await
        .unwrap();
        // A row that hashes correctly over *its own* content but names the genesis hash
        // as its predecessor: the shape an inserted entry takes when the attacker has
        // no way to reach the following rows.
        disarm_triggers(&sql).await;
        let id = mm_core::ulid_string(&action(9));
        let payload_json = "{}".to_string();
        let hash = entry_hash(GENESIS_HASH, &id, "invoke.end", &payload_json);
        sqlx::query(
            "INSERT INTO action_ledger (seq, id, action_id, event, payload_json, prev_hash, \
             entry_hash, at) VALUES (2, ?, ?, 'invoke.end', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&id)
        .bind(&payload_json)
        .bind(GENESIS_HASH)
        .bind(&hash)
        .bind(at(2).to_rfc3339())
        .execute(sql.pool())
        .await
        .unwrap();
        let error = verify_chain(&sql).await.unwrap_err();
        assert_eq!(error.code(), "ledger.chain_broken");
        assert!(matches!(error, LedgerError::ChainBroken { seq: 2, .. }));
    }

    #[tokio::test]
    async fn a_rewritten_first_row_is_detected_as_bad_genesis() {
        let sql = store().await;
        append_at(
            &sql,
            action(1),
            "action.proposed",
            &serde_json::json!({}),
            at(1),
        )
        .await
        .unwrap();
        append_at(&sql, action(1), "invoke.end", &serde_json::json!({}), at(2))
            .await
            .unwrap();
        disarm_triggers(&sql).await;
        let entries = entries_for(&sql, &action(1)).await.unwrap();
        let forged_prev = "f".repeat(64);
        let forged_hash = entry_hash(
            &forged_prev,
            &mm_core::ulid_string(&entries[0].id),
            &entries[0].event,
            &crate::canonical_json(&entries[0].payload),
        );
        sqlx::query("UPDATE action_ledger SET prev_hash = ?, entry_hash = ? WHERE seq = 1")
            .bind(&forged_prev)
            .bind(&forged_hash)
            .execute(sql.pool())
            .await
            .unwrap();
        let error = verify_chain(&sql).await.unwrap_err();
        assert_eq!(error.code(), "ledger.bad_genesis");
        assert!(matches!(error, LedgerError::BadGenesis { .. }));
    }

    #[tokio::test]
    async fn append_refuses_to_extend_a_tampered_head() {
        let sql = store().await;
        append_at(
            &sql,
            action(1),
            "action.proposed",
            &serde_json::json!({ "n": 1 }),
            at(1),
        )
        .await
        .unwrap();
        disarm_triggers(&sql).await;
        sql.pool()
            .execute("UPDATE action_ledger SET payload_json = '{\"n\":99}' WHERE seq = 1")
            .await
            .unwrap();
        let error = append_at(&sql, action(1), "invoke.end", &serde_json::json!({}), at(2))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ledger.hash_mismatch");
    }

    #[tokio::test]
    async fn an_unknown_event_is_refused() {
        let sql = store().await;
        let error = append(
            &sql,
            action(1),
            "something.invented",
            &serde_json::json!({}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), "ledger.store");
        assert!(error.to_string().contains("not a lifecycle event"));
        assert!(is_known_event("invoke.end"));
        assert!(!is_known_event("invoke.started"));
        assert_eq!(LEDGER_EVENTS.len(), 8);
    }

    #[tokio::test]
    async fn entries_are_filtered_by_action() {
        let sql = store().await;
        append_at(
            &sql,
            action(1),
            "action.proposed",
            &serde_json::json!({}),
            at(1),
        )
        .await
        .unwrap();
        append_at(
            &sql,
            action(2),
            "action.proposed",
            &serde_json::json!({}),
            at(2),
        )
        .await
        .unwrap();
        append_at(&sql, action(1), "invoke.end", &serde_json::json!({}), at(3))
            .await
            .unwrap();
        assert_eq!(entries_for(&sql, &action(1)).await.unwrap().len(), 2);
        assert_eq!(entries_for(&sql, &action(2)).await.unwrap().len(), 1);
        assert!(entries_for(&sql, &action(3)).await.unwrap().is_empty());
        assert_eq!(tail(&sql, 10).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn two_concurrent_appends_land_on_distinct_sequences() {
        let sql = store().await;
        let one = sql.clone();
        let two = sql.clone();
        let (a, b) = tokio::join!(
            async move {
                append_at(
                    &one,
                    action(1),
                    "invoke.end",
                    &serde_json::json!({ "a": 1 }),
                    at(1),
                )
                .await
            },
            async move {
                append_at(
                    &two,
                    action(1),
                    "invoke.end",
                    &serde_json::json!({ "b": 2 }),
                    at(2),
                )
                .await
            },
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a.seq, b.seq);
        assert_eq!(vec![a.seq.min(b.seq), a.seq.max(b.seq)], vec![1, 2]);
        assert_eq!(verify_chain(&sql).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn the_same_recorded_call_appends_the_same_row() {
        let payload = serde_json::json!({ "status": "ok", "latency_ms": 3 });
        let one = store().await;
        let two = store().await;
        let a = append_at(&one, action(1), "invoke.end", &payload, at(1_700_000_000))
            .await
            .unwrap();
        let b = append_at(&two, action(1), "invoke.end", &payload, at(1_700_000_000))
            .await
            .unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.entry_hash, b.entry_hash);
        assert_eq!(a.seq, b.seq);
    }

    #[test]
    fn the_hash_is_deterministic_and_covers_every_field() {
        let baseline = entry_hash(
            GENESIS_HASH,
            "01h00000000000000000000001",
            "invoke.end",
            "{}",
        );
        assert_eq!(
            baseline,
            entry_hash(
                GENESIS_HASH,
                "01h00000000000000000000001",
                "invoke.end",
                "{}"
            )
        );
        assert_ne!(
            baseline,
            entry_hash(
                "a".repeat(64).as_str(),
                "01h00000000000000000000001",
                "invoke.end",
                "{}"
            )
        );
        assert_ne!(
            baseline,
            entry_hash(
                GENESIS_HASH,
                "01h00000000000000000000002",
                "invoke.end",
                "{}"
            )
        );
        assert_ne!(
            baseline,
            entry_hash(
                GENESIS_HASH,
                "01h00000000000000000000001",
                "action.proposed",
                "{}"
            )
        );
        assert_ne!(
            baseline,
            entry_hash(
                GENESIS_HASH,
                "01h00000000000000000000001",
                "invoke.end",
                "{\"n\":1}"
            )
        );
        assert_eq!(baseline.len(), 64);
        assert!(is_genesis(GENESIS_HASH));
        assert!(!is_genesis(&baseline));
    }

    #[test]
    fn the_canonical_rendering_ignores_payload_key_order() {
        let mut entry = LedgerEntry {
            seq: 1,
            id: action(1),
            action_id: action(2),
            event: "invoke.end".into(),
            payload: serde_json::json!({ "a": 1, "b": 2 }),
            prev_hash: GENESIS_HASH.into(),
            entry_hash: String::new(),
            at: at(1),
        };
        let canonical = entry.canonical();
        entry.payload = serde_json::json!({ "b": 2, "a": 1 });
        assert_eq!(canonical, entry.canonical());
    }

    #[test]
    fn recompute_chain_reports_the_first_defect() {
        let entry = |seq: i64, prev: &str, hash: &str| LedgerEntry {
            seq,
            id: action(seq as u128),
            action_id: action(1),
            event: "invoke.end".into(),
            payload: serde_json::json!({}),
            prev_hash: prev.to_string(),
            entry_hash: hash.to_string(),
            at: at(1),
        };
        let first = entry(
            1,
            GENESIS_HASH,
            &entry_hash(
                GENESIS_HASH,
                &mm_core::ulid_string(&action(1)),
                "invoke.end",
                "{}",
            ),
        );
        assert!(recompute_chain(std::slice::from_ref(&first)).is_ok());
        let wrong_seq = entry(5, GENESIS_HASH, "");
        assert!(matches!(
            recompute_chain(&[wrong_seq]).unwrap_err(),
            LedgerError::Gap { .. }
        ));
        let bad_start = entry(1, "abc", "");
        assert!(matches!(
            recompute_chain(&[bad_start]).unwrap_err(),
            LedgerError::BadGenesis { .. }
        ));
    }

    #[tokio::test]
    async fn an_entry_round_trips_through_json() {
        let sql = store().await;
        append_at(
            &sql,
            action(1),
            "observation.record",
            &serde_json::json!({ "n": 1 }),
            at(7),
        )
        .await
        .unwrap();
        let entry = entries_for(&sql, &action(1)).await.unwrap().remove(0);
        let text = serde_json::to_string(&entry).unwrap();
        let back: LedgerEntry = serde_json::from_str(&text).unwrap();
        assert_eq!(entry, back);
        assert!(back.is_intact());
    }

    #[tokio::test]
    async fn ensure_chain_agrees_with_verify_chain() {
        let sql = store().await;
        append_at(
            &sql,
            action(1),
            "action.proposed",
            &serde_json::json!({}),
            at(1),
        )
        .await
        .unwrap();
        assert_eq!(ensure_chain(&sql).await.unwrap(), 1);
        disarm_triggers(&sql).await;
        sql.pool()
            .execute("UPDATE action_ledger SET event = 'invoke.end' WHERE seq = 1")
            .await
            .unwrap();
        assert!(ensure_chain(&sql).await.is_err());
    }
}
