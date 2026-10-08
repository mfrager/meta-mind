//! Bitemporal reads.
//!
//! Two axes, never conflated:
//!
//! * **system time** — *when the kernel knew* the fact: the append-only event
//!   sequence at which the row was recorded. Using the sequence rather than a wall
//!   clock is what makes a replay reproduce the same answers.
//! * **valid time** — *when the fact is about* in the domain, an RFC3339 instant.
//!
//! A row is visible at `as_of(system, valid)` iff it was recorded at or before
//! `system` and its valid interval covers `valid`.

use serde::{Deserialize, Serialize};

use mm_core::store::{Param, Params, Tabular};
use mm_core::{MmError, Timestamp, Ulid};

use crate::pool::{validate_identifier, SqliteStore};

/// A point on both time axes. `None` means "now" on that axis: every committed
/// row for system time, the current instant for valid time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsOf {
    /// The event-log sequence to read as of.
    pub system: Option<u64>,
    /// The domain instant to read as of.
    pub valid: Option<Timestamp>,
}

/// The visibility predicate. Four `?` placeholders, in the order [`AsOf::bind`]
/// supplies them.
pub const VISIBILITY_PREDICATE: &str = "system_from <= ? AND (system_to IS NULL OR system_to > ?) \
     AND valid_from <= ? AND (valid_to IS NULL OR valid_to > ?)";

impl Default for AsOf {
    fn default() -> Self {
        AsOf::now()
    }
}

impl AsOf {
    /// The current state on both axes.
    pub fn now() -> Self {
        AsOf {
            system: None,
            valid: None,
        }
    }

    /// An explicit point on both axes.
    pub fn at(system: u64, valid: Timestamp) -> Self {
        AsOf {
            system: Some(system),
            valid: Some(valid),
        }
    }

    /// A point on the system axis only.
    pub fn system(system: u64) -> Self {
        AsOf {
            system: Some(system),
            valid: None,
        }
    }

    /// A point on the valid axis only.
    pub fn valid(valid: Timestamp) -> Self {
        AsOf {
            system: None,
            valid: Some(valid),
        }
    }

    /// Whether both axes mean "now".
    pub fn is_now(&self) -> bool {
        self.system.is_none() && self.valid.is_none()
    }

    /// The system boundary actually compared against.
    pub fn effective_system(&self) -> i64 {
        self.system
            .map_or(i64::MAX, |s| i64::try_from(s).unwrap_or(i64::MAX))
    }

    /// The valid boundary actually compared against.
    pub fn effective_valid(&self) -> Timestamp {
        self.valid.unwrap_or_else(Timestamp::now)
    }

    /// The predicate to interpose after `WHERE`.
    pub fn predicate() -> &'static str {
        VISIBILITY_PREDICATE
    }

    /// The four bind parameters the predicate expects.
    pub fn bind(&self) -> Params {
        let system = Param::Int(self.effective_system());
        let valid = Param::Text(self.effective_valid().to_rfc3339());
        vec![system.clone(), system, valid.clone(), valid]
    }
}

/// A fact to record.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFact {
    /// The fact's identity.
    pub id: Ulid,
    /// Subject IRI.
    pub subject: String,
    /// Predicate IRI.
    pub predicate: String,
    /// Object IRI or literal.
    pub object: String,
    /// The event sequence at which this fact became known.
    pub system_from: i64,
    /// When the fact starts being true in the domain.
    pub valid_from: Timestamp,
    /// When it stops, if it has.
    pub valid_to: Option<Timestamp>,
    /// Provenance IRI or note.
    pub provenance: Option<String>,
    /// The correlation id of the operation that recorded it.
    pub trace_id: Option<Ulid>,
}

impl SqliteStore {
    /// Read a bitemporal table as of a point on both axes.
    pub async fn query_bitemporal(
        &self,
        table: &str,
        as_of: AsOf,
    ) -> Result<Vec<serde_json::Value>, MmError> {
        validate_identifier(table)?;
        let sql = format!("SELECT * FROM {table} WHERE {}", AsOf::predicate());
        Tabular::query_json(self, &sql, as_of.bind()).await
    }

    /// Record a fact.
    pub async fn insert_fact(&self, fact: &NewFact) -> Result<(), MmError> {
        Tabular::execute(
            self,
            "INSERT INTO facts (id, subject, predicate, object, system_from, system_to, \
             valid_from, valid_to, provenance, trace_id) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&fact.id)),
                Param::Text(fact.subject.clone()),
                Param::Text(fact.predicate.clone()),
                Param::Text(fact.object.clone()),
                Param::Int(fact.system_from),
                Param::Text(fact.valid_from.to_rfc3339()),
                Param::opt_text(fact.valid_to.map(|t| t.to_rfc3339())),
                Param::opt_text(fact.provenance.clone()),
                Param::opt_text(fact.trace_id.map(|t| mm_core::ulid_string(&t))),
            ],
        )
        .await?;
        Ok(())
    }

    /// Close the validity of a fact on both axes.
    pub async fn close_fact(
        &self,
        id: &Ulid,
        system_to: i64,
        valid_to: Timestamp,
    ) -> Result<u64, MmError> {
        Tabular::execute(
            self,
            "UPDATE facts SET system_to = ?, valid_to = ? WHERE id = ?",
            vec![
                Param::Int(system_to),
                Param::Text(valid_to.to_rfc3339()),
                Param::Text(mm_core::ulid_string(id)),
            ],
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::UlidFactory;

    fn ts(s: &str) -> Timestamp {
        Timestamp::from_rfc3339(s).unwrap()
    }

    async fn seeded_store() -> (tempfile::TempDir, SqliteStore, Vec<Ulid>) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("t.db")).await.unwrap();
        store.migrate().await.unwrap();
        let ids = UlidFactory::new();
        let o1 = ids.next();
        let o2 = ids.next();
        // o1 is known from sequence 1 and true from January.
        let f1 = NewFact {
            id: ids.next(),
            subject: "mmd:s".into(),
            predicate: "mm:value".into(),
            object: "o1".into(),
            system_from: 1,
            valid_from: ts("2024-01-01T00:00:00.000000000Z"),
            valid_to: None,
            provenance: None,
            trace_id: None,
        };
        // o2 is not known until sequence 5 and not true until June.
        let f2 = NewFact {
            id: ids.next(),
            subject: "mmd:s".into(),
            predicate: "mm:value".into(),
            object: "o2".into(),
            system_from: 5,
            valid_from: ts("2024-06-01T00:00:00.000000000Z"),
            valid_to: None,
            provenance: None,
            trace_id: None,
        };
        store.insert_fact(&f1).await.unwrap();
        store.insert_fact(&f2).await.unwrap();
        assert_eq!(store.row_count("facts").await.unwrap(), 2);
        (dir, store, vec![o1, o2])
    }

    async fn objects(store: &SqliteStore, as_of: AsOf) -> Vec<String> {
        let mut got: Vec<String> = store
            .query_bitemporal("facts", as_of)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r["object"].as_str().unwrap().to_string())
            .collect();
        got.sort();
        got
    }

    #[tokio::test]
    async fn point_in_time_reads_return_exactly_the_rows_valid_then() {
        let (_d, store, _o) = seeded_store().await;

        // As known at seq 1, in March: only o1.
        assert_eq!(
            objects(&store, AsOf::at(1, ts("2024-03-01T00:00:00.000000000Z"))).await,
            vec!["o1"]
        );
        // As known at seq 5, still in March: o2 exists but is not yet valid.
        assert_eq!(
            objects(&store, AsOf::at(5, ts("2024-03-01T00:00:00.000000000Z"))).await,
            vec!["o1"]
        );
        // As known at seq 5, in July: both.
        assert_eq!(
            objects(&store, AsOf::at(5, ts("2024-07-01T00:00:00.000000000Z"))).await,
            vec!["o1", "o2"]
        );
        // As known at seq 4, in July: o2 was not known yet.
        assert_eq!(
            objects(&store, AsOf::at(4, ts("2024-07-01T00:00:00.000000000Z"))).await,
            vec!["o1"]
        );
        // Reads on one axis only.
        assert_eq!(objects(&store, AsOf::system(1)).await, vec!["o1"]);
        assert_eq!(
            objects(&store, AsOf::valid(ts("2024-03-01T00:00:00.000000000Z"))).await,
            vec!["o1"]
        );
        assert_eq!(objects(&store, AsOf::now()).await, vec!["o1", "o2"]);
    }

    #[tokio::test]
    async fn closing_a_fact_hides_it_from_later_reads_but_not_earlier_ones() {
        let (_d, store, _o) = seeded_store().await;
        let id = Ulid::from_string(
            store
                .query_bitemporal("facts", AsOf::now())
                .await
                .unwrap()
                .iter()
                .find(|r| r["object"] == "o1")
                .unwrap()["id"]
                .as_str()
                .unwrap(),
        )
        .unwrap();

        let closed = store
            .close_fact(&id, 6, ts("2024-06-01T00:00:00.000000000Z"))
            .await
            .unwrap();
        assert_eq!(closed, 1);

        // At seq 4 / March, the fact is still visible: it had not been superseded.
        assert_eq!(
            objects(&store, AsOf::at(4, ts("2024-03-01T00:00:00.000000000Z"))).await,
            vec!["o1"]
        );
        // At seq 6 / July, it is gone and only o2 remains.
        assert_eq!(
            objects(&store, AsOf::at(6, ts("2024-07-01T00:00:00.000000000Z"))).await,
            vec!["o2"]
        );
    }

    #[tokio::test]
    async fn the_trigger_refuses_a_second_current_fact_for_the_same_triple() {
        let (_d, store, _o) = seeded_store().await;
        let dup = NewFact {
            id: UlidFactory::new().next(),
            subject: "mmd:s".into(),
            predicate: "mm:value".into(),
            object: "o1".into(),
            system_from: 9,
            valid_from: ts("2025-01-01T00:00:00.000000000Z"),
            valid_to: None,
            provenance: None,
            trace_id: None,
        };
        let err = store.insert_fact(&dup).await.unwrap_err();
        assert!(
            err.to_string().contains("already current"),
            "expected the overlap trigger to fire, got {err}"
        );
    }

    #[tokio::test]
    async fn table_names_are_validated_before_they_reach_sql() {
        let (_d, store, _o) = seeded_store().await;
        let err = store
            .query_bitemporal("facts; DROP TABLE events", AsOf::now())
            .await
            .unwrap_err();
        assert!(matches!(err, MmError::Store(_)));
        // The table is still there.
        assert_eq!(store.row_count("facts").await.unwrap(), 2);
    }
}
