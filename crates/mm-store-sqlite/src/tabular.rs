//! The `Tabular` implementation.
//!
//! Bind parameters are typed and positional; result rows become JSON objects
//! keyed by column name. Nothing here formats SQL, so a caller can always see the
//! statement it is running.

use async_trait::async_trait;
use mm_core::store::{Param, Params, Tabular};
use mm_core::MmError;
use serde_json::{Map, Value};
use sqlx::query::Query;
use sqlx::sqlite::SqliteArguments;
use sqlx::{Column, Row, Sqlite, ValueRef};

use crate::pool::SqliteStore;

#[async_trait]
impl Tabular for SqliteStore {
    async fn execute(&self, sql: &str, args: Params) -> Result<u64, MmError> {
        let query = bind_all(sqlx::query(sql), args);
        let result = query
            .execute(self.pool())
            .await
            .map_err(|e| MmError::Store(format!("{e} (statement: {sql})")))?;
        Ok(result.rows_affected())
    }

    async fn query_json(&self, sql: &str, args: Params) -> Result<Vec<Value>, MmError> {
        let query = bind_all(sqlx::query(sql), args);
        let rows = query
            .fetch_all(self.pool())
            .await
            .map_err(|e| MmError::Store(format!("{e} (statement: {sql})")))?;
        Ok(rows.iter().map(row_to_json).collect())
    }
}

fn bind_all<'q>(
    mut query: Query<'q, Sqlite, SqliteArguments<'q>>,
    args: Params,
) -> Query<'q, Sqlite, SqliteArguments<'q>> {
    for arg in args {
        query = match arg {
            Param::Null => query.bind(Option::<String>::None),
            Param::Int(v) => query.bind(v),
            Param::Real(v) => query.bind(v),
            Param::Text(v) => query.bind(v),
            Param::Blob(v) => query.bind(v),
        };
    }
    query
}

fn row_to_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    let mut map = Map::with_capacity(row.columns().len());
    for (i, column) in row.columns().iter().enumerate() {
        map.insert(column.name().to_string(), value_at(row, i));
    }
    Value::Object(map)
}

/// Map one cell to JSON by declared SQLite type.
///
/// `sqlx` type-checks on read, so the first conversion that succeeds is the
/// column's actual type; a NULL fails all of them and becomes JSON `null`.
fn value_at(row: &sqlx::sqlite::SqliteRow, index: usize) -> Value {
    let is_null = row
        .try_get_raw(index)
        .map(|raw| raw.is_null())
        .unwrap_or(true);
    if is_null {
        return Value::Null;
    }
    if let Ok(v) = row.try_get::<i64, _>(index) {
        return Value::from(v);
    }
    if let Ok(v) = row.try_get::<f64, _>(index) {
        return Value::from(v);
    }
    if let Ok(v) = row.try_get::<String, _>(index) {
        return Value::from(v);
    }
    if let Ok(v) = row.try_get::<Vec<u8>, _>(index) {
        return Value::from(v.iter().map(|b| format!("{b:02x}")).collect::<String>());
    }
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::store::Tabular;

    async fn store() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("t.db")).await.unwrap();
        store.migrate().await.unwrap();
        (dir, store)
    }

    #[tokio::test]
    async fn typed_binds_and_json_rows_round_trip() {
        let (_dir, store) = store().await;
        let affected = store
            .execute(
                "INSERT INTO schema_versions (name, version, applied_at) VALUES (?, ?, ?)",
                vec![
                    Param::Text("probe".into()),
                    Param::Int(7),
                    Param::Text("2024-01-01T00:00:00.000000000Z".into()),
                ],
            )
            .await
            .unwrap();
        assert_eq!(affected, 1);

        let rows = store
            .execute_query(
                "SELECT name, version, applied_at FROM schema_versions WHERE name = ?",
                vec![Param::Text("probe".into())],
            )
            .await;
        assert_eq!(rows[0]["name"], Value::from("probe"));
        assert_eq!(rows[0]["version"], Value::from(7));
        assert!(rows[0]["applied_at"].is_string());
    }

    #[tokio::test]
    async fn nulls_reals_blobs_and_negative_numbers_map_faithfully() {
        let (_dir, store) = store().await;
        store
            .execute(
                "INSERT INTO facts (id, subject, predicate, object, system_from, system_to, valid_from, valid_to, provenance, trace_id) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                vec![
                    Param::Text("01h0000000000000000000001".into()),
                    Param::Text("s".into()),
                    Param::Text("p".into()),
                    Param::Text("o".into()),
                    Param::Int(-5),
                    Param::Null,
                    Param::Text("2024-01-01T00:00:00.000000000Z".into()),
                    Param::Null,
                    Param::Blob(vec![0xde, 0xad]),
                    Param::Null,
                ],
            )
            .await
            .unwrap();
        let rows = store
            .execute_query(
                "SELECT system_from, system_to, provenance, trace_id FROM facts",
                vec![],
            )
            .await;
        assert_eq!(rows[0]["system_from"], Value::from(-5));
        assert_eq!(rows[0]["system_to"], Value::Null);
        assert_eq!(rows[0]["trace_id"], Value::Null);
        assert_eq!(rows[0]["provenance"], Value::from("dead"));
    }

    #[tokio::test]
    async fn a_failing_statement_reports_the_sql_and_does_not_panic() {
        let (_dir, store) = store().await;
        let err = store
            .execute("SELECT * FROM no_such_table", vec![])
            .await
            .unwrap_err();
        assert!(matches!(err, MmError::Store(_)));
        assert!(err.to_string().contains("no_such_table"));
    }

    impl SqliteStore {
        async fn execute_query(&self, sql: &str, args: Params) -> Vec<Value> {
            Tabular::query_json(self, sql, args).await.unwrap()
        }
    }
}
