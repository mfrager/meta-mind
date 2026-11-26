//! The tabular side of the data root.
//!
//! RDF owns semantic state; SQLite owns the high-churn rows that do not benefit
//! from being triples (chat frames, audit rows, sequence watermarks). The pool
//! is small and dedicated to the data root; it does not replace the kernel's
//! `mm-store-sqlite` store.

use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::error::Result;

/// The database file under `sqlite/`.
pub const MAIN_DB: &str = "metamind.db";

/// A pool plus the schema version it is on.
#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
    path: PathBuf,
}

impl SqliteStore {
    /// Open (creating if necessary) `<dir>/metamind.db`.
    pub async fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(MAIN_DB);
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        let store = Self { pool, path };
        store.migrate().await?;
        Ok(store)
    }

    /// An in-memory database, for tests.
    pub async fn in_memory() -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        let store = Self {
            pool,
            path: PathBuf::from(":memory:"),
        };
        store.migrate().await?;
        Ok(store)
    }

    /// The pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Create the tables this layer owns. Idempotent.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS data_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chat_frames (
                frame_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                turn_id TEXT,
                created_at INTEGER NOT NULL,
                payload TEXT NOT NULL
             )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS chat_frames_session
             ON chat_frames (session_id, created_at)",
        )
        .execute(&self.pool)
        .await?;
        for statement in crate::data::outbox::DDL {
            sqlx::query(statement).execute(&self.pool).await?;
        }
        sqlx::query(
            "INSERT INTO data_meta (key, value) VALUES ('schema_version', '1')
             ON CONFLICT(key) DO NOTHING",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The stored schema version.
    pub async fn schema_version(&self) -> Result<String> {
        let row: (String,) =
            sqlx::query_as("SELECT value FROM data_meta WHERE key = 'schema_version'")
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    /// A cheap readiness probe.
    pub async fn is_ready(&self) -> Result<bool> {
        let row: (i64,) = sqlx::query_as("SELECT 1").fetch_one(&self.pool).await?;
        Ok(row.0 == 1)
    }
}
