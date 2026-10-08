//! The SQLite pool.
//!
//! One database file owns all tabular state. Every connection gets the same
//! pragmas — WAL, foreign keys on, a 5s busy timeout — set from the connect
//! options so they cannot drift from the code that opens the pool.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mm_core::MmError;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

/// How many connections the pool keeps.
const MAX_CONNECTIONS: u32 = 8;
/// How long a writer waits for another writer before reporting busy.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The tabular store.
#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
    path: PathBuf,
    /// Serializes audit-chain appends inside one process. Cross-process safety
    /// comes from `audit_log_prev_unique` plus a bounded retry.
    audit_lock: Arc<tokio::sync::Mutex<()>>,
}

impl SqliteStore {
    /// Open (creating if needed) the database at `path`.
    pub async fn open(path: &Path) -> Result<Self, MmError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .acquire_timeout(BUSY_TIMEOUT)
            .connect_with(options)
            .await
            .map_err(|e| MmError::Store(format!("cannot open {}: {e}", path.display())))?;
        Ok(SqliteStore {
            pool,
            path: path.to_path_buf(),
            audit_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// The in-process audit-chain lock.
    pub fn audit_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.audit_lock
    }

    /// The connection pool, for code that needs an explicit transaction.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Apply every migration in `migrations/`.
    ///
    /// Version numbers are phase numbers, so the history is the build order.
    pub async fn migrate(&self) -> Result<(), MmError> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| MmError::Store(format!("migration failed: {e}")))
    }

    /// The `journal_mode` the connection actually got.
    pub async fn journal_mode(&self) -> Result<String, MmError> {
        self.scalar_string("PRAGMA journal_mode").await
    }

    /// The SQLite library version.
    pub async fn sqlite_version(&self) -> Result<String, MmError> {
        self.scalar_string("SELECT sqlite_version()").await
    }

    /// The applied migration versions, ascending.
    pub async fn applied_migrations(&self) -> Result<Vec<i64>, MmError> {
        sqlx::query_scalar::<_, i64>("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| MmError::Store(e.to_string()))
    }

    /// A `SELECT count(*)` over a table. The table name is validated so it can
    /// never be attacker-controlled.
    pub async fn row_count(&self, table: &str) -> Result<i64, MmError> {
        validate_identifier(table)?;
        let sql = format!("SELECT count(*) FROM {table}");
        sqlx::query_scalar::<_, i64>(&sql)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| MmError::Store(format!("count({table}) failed: {e}")))
    }

    /// Every user table/view/trigger, with its DDL, ordered deterministically.
    ///
    /// Used by the migration round-trip test to prove that up → down → up yields
    /// a byte-identical schema.
    pub async fn schema_dump(&self) -> Result<String, MmError> {
        let rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT type, name, COALESCE(sql, '') FROM sqlite_master \
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MmError::Store(e.to_string()))?;
        let mut out = String::new();
        for (kind, name, sql) in rows {
            out.push_str(&format!("-- {kind} {name}\n{sql}\n"));
        }
        Ok(out)
    }

    /// Close the pool, waiting for in-flight work.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    async fn scalar_string(&self, sql: &str) -> Result<String, MmError> {
        sqlx::query_scalar::<_, String>(sql)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| MmError::Store(format!("{sql} failed: {e}")))
    }
}

/// Reject anything that is not a plain SQL identifier.
pub fn validate_identifier(name: &str) -> Result<(), MmError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.chars().next().is_some_and(|c| c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(MmError::Store(format!("invalid SQL identifier: {name:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn opens_with_wal_and_foreign_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("metamind.db"))
            .await
            .unwrap();
        assert_eq!(store.journal_mode().await.unwrap().to_lowercase(), "wal");
        let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(fk, 1, "foreign keys must be enforced on every connection");
        assert!(!store.sqlite_version().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn migrate_creates_the_kernel_schema() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("metamind.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        assert_eq!(store.applied_migrations().await.unwrap(), vec![1, 2, 3, 4, 5]);
        for table in ["events", "audit_log", "store_checkpoints", "facts"] {
            assert_eq!(
                store.row_count(table).await.unwrap(),
                0,
                "{table} must start empty"
            );
        }
        // Phase 2 adds the code-metadata mirror; it starts empty too.
        for table in ["module_index", "module_file", "symbol_index", "codex_run"] {
            assert_eq!(
                store.row_count(table).await.unwrap(),
                0,
                "{table} must start empty"
            );
        }
        // Phase 3 adds the LLM ledger and its satellites; they start empty too.
        for table in [
            "llm_calls",
            "llm_cache_index",
            "llm_semantic_cache",
            "llm_repair_attempts",
            "llm_routing",
            "llm_grammars",
        ] {
            assert_eq!(
                store.row_count(table).await.unwrap(),
                0,
                "{table} must start empty"
            );
        }
        // Phase 4 adds the persistent being substrate; it starts empty too.
        for table in [
            "identity",
            "invariants",
            "core_blocks",
            "personality_values",
            "personality_dispositions",
            "personality_context_modifiers",
            "affect_state",
            "affect_impulses",
            "motivations",
            "user_beliefs",
            "relationships",
            "relationship_events",
            "goals",
            "goal_transitions",
            "commitments",
            "commitment_transitions",
            "resource_accounts",
            "resource_ledger",
            "budget_policies",
        ] {
            assert_eq!(
                store.row_count(table).await.unwrap(),
                0,
                "{table} must start empty"
            );
        }
        // Phase 5 adds the memory organ; it starts empty too.
        for table in [
            "memories",
            "memory_cues",
            "memory_entities",
            "memory_links",
            "memory_access",
            "memory_consolidations",
            "memory_summaries",
            "memory_communities",
            "entity_edges",
            "mistakes",
        ] {
            assert_eq!(
                store.row_count(table).await.unwrap(),
                0,
                "{table} must start empty"
            );
        }
        // One row per phase: kernel (1), code index (2), LLM ledger (3), being (4),
        // memory (5).
        assert_eq!(store.row_count("schema_versions").await.unwrap(), 5);
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("metamind.db"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let first = store.schema_dump().await.unwrap();
        store.migrate().await.unwrap();
        assert_eq!(first, store.schema_dump().await.unwrap());
    }

    #[test]
    fn identifiers_are_validated() {
        assert!(validate_identifier("facts_2").is_ok());
        for bad in ["", "facts; DROP TABLE events", "2facts", "a-b"] {
            assert!(validate_identifier(bad).is_err(), "{bad} must be rejected");
        }
    }
}
