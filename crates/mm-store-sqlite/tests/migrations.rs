//! Migration round trip.
//!
//! The gate requires that `up → down → up` leaves an *identical* schema, not
//! merely a working one. The down script lives in `migrations_down/` because sqlx
//! applies every `.sql` file it finds under `migrations/` as a forward migration.

use mm_store_sqlite::SqliteStore;
use sqlx::Executor;

/// Every migration's down script, newest first, so the round trip unwinds the
/// schema in reverse dependency order. `migrate()` applies them all, so a down
/// script that reversed only the newest would leave the kernel tables behind.
const DOWN_SCRIPT: &str = concat!(
    include_str!("../migrations_down/0005_memory.sql"),
    "\n",
    include_str!("../migrations_down/0004_being.sql"),
    "\n",
    include_str!("../migrations_down/0003_llm.sql"),
    "\n",
    include_str!("../migrations_down/0002_codex.sql"),
    "\n",
    include_str!("../migrations_down/0001_kernel.sql"),
);

#[tokio::test]
async fn up_down_up_is_byte_identical_and_leaves_no_user_tables() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("metamind.db");

    let store = SqliteStore::open(&db).await.unwrap();
    store.migrate().await.unwrap();
    let after_up = store.schema_dump().await.unwrap();
    assert!(after_up.contains("CREATE TABLE events"));
    assert!(after_up.contains("CREATE TABLE audit_log"));
    assert!(after_up.contains("CREATE TABLE facts"));
    assert!(after_up.contains("CREATE TRIGGER audit_log_no_update"));
    assert!(after_up.contains("CREATE TABLE module_index"));
    assert!(after_up.contains("CREATE TABLE symbol_index"));
    assert!(after_up.contains("CREATE TABLE llm_calls"));
    assert!(after_up.contains("CREATE TABLE llm_grammars"));
    assert!(after_up.contains("CREATE TABLE identity"));
    assert!(after_up.contains("CREATE TABLE goals"));
    assert!(after_up.contains("CREATE TRIGGER goals_terminal_is_immutable"));
    assert!(after_up.contains("CREATE TRIGGER commitments_terminal_is_immutable"));
    assert!(after_up.contains("CREATE TABLE memories"));
    assert!(after_up.contains("CREATE TABLE entity_edges"));
    assert!(after_up.contains("CREATE TABLE mistakes"));
    assert!(after_up.contains("CREATE VIRTUAL TABLE memory_fts"));
    assert_eq!(store.applied_migrations().await.unwrap(), vec![1, 2, 3, 4, 5]);

    // Down: drop every kernel object and clear the migration ledger.
    store
        .pool()
        .execute(DOWN_SCRIPT)
        .await
        .expect("down script must run");

    // sqlx keeps its own migrations ledger; it is infrastructure, not a user
    // table, so it is excluded from the "no user objects remain" check.
    let objects: Vec<(String, String)> = sqlx::query_as(
        "SELECT type, name FROM sqlite_master \
         WHERE name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'",
    )
    .fetch_all(store.pool())
    .await
    .unwrap();
    assert!(
        objects.is_empty(),
        "down must leave no user objects, found {objects:?}"
    );
    assert!(store.applied_migrations().await.unwrap().is_empty());

    // Up again: identical DDL, not merely equivalent.
    store.migrate().await.unwrap();
    let after_second_up = store.schema_dump().await.unwrap();
    assert_eq!(
        after_up, after_second_up,
        "up -> down -> up must produce an identical schema"
    );
    assert_eq!(store.applied_migrations().await.unwrap(), vec![1, 2, 3, 4, 5]);
    store.close().await;
}

#[tokio::test]
async fn migration_versions_are_phase_numbered_and_unique() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("m.db")).await.unwrap();
    store.migrate().await.unwrap();
    let versions = store.applied_migrations().await.unwrap();
    let expected: Vec<i64> = (1..=versions.len() as i64).collect();
    assert_eq!(versions, expected, "phase N must own migration N");
}
