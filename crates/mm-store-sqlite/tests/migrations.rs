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
    include_str!("../migrations_down/0012_loop.sql"),
    "\n",
    include_str!("../migrations_down/0011_self_engineering.sql"),
    "\n",
    include_str!("../migrations_down/0010_tools.sql"),
    "\n",
    include_str!("../migrations_down/0009_decision_firewall.sql"),
    "\n",
    include_str!("../migrations_down/0008_metacog.sql"),
    "\n",
    include_str!("../migrations_down/0007_library.sql"),
    "\n",
    include_str!("../migrations_down/0006_epistemic.sql"),
    "\n",
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
    assert!(after_up.contains("CREATE TABLE claims"));
    assert!(after_up.contains("CREATE TABLE evidence"));
    assert!(after_up.contains("CREATE TABLE observations"));
    assert!(after_up.contains("CREATE TABLE assumptions"));
    assert!(after_up.contains("CREATE TABLE predictions"));
    assert!(after_up.contains("CREATE TABLE contradictions"));
    assert!(after_up.contains("CREATE TABLE dependencies"));
    assert!(after_up.contains("CREATE TABLE epistemic_transitions"));
    assert!(after_up.contains("CREATE INDEX idx_claims_spo"));
    assert!(after_up.contains("CREATE INDEX idx_deps_ante"));
    assert!(after_up.contains("CREATE INDEX idx_claims_asof"));
    assert!(after_up.contains("CREATE TABLE library_entries"));
    assert!(after_up.contains("CREATE TABLE skills"));
    assert!(after_up.contains("CREATE TABLE workflows"));
    assert!(after_up.contains("CREATE TABLE cases"));
    assert!(after_up.contains("CREATE TABLE case_map"));
    assert!(after_up.contains("CREATE TABLE insights"));
    assert!(after_up.contains("CREATE TABLE policies"));
    assert!(after_up.contains("CREATE TABLE policy_versions"));
    assert!(after_up.contains("CREATE TABLE policy_fitness"));
    assert!(after_up.contains("CREATE TABLE policy_population"));
    assert!(after_up.contains("CREATE TABLE frame_instances"));
    assert!(after_up.contains("CREATE TABLE applicability_runs"));
    assert!(after_up.contains("CREATE INDEX idx_library_kind"));
    assert!(after_up.contains("CREATE TABLE episodes"));
    assert!(after_up.contains("CREATE TABLE programs"));
    assert!(after_up.contains("CREATE TABLE program_traces"));
    assert!(after_up.contains("CREATE INDEX idx_programs_episode"));
    assert!(after_up.contains("CREATE INDEX idx_traces_program_seq"));
    assert!(after_up.contains("CREATE TABLE decisions"));
    assert!(after_up.contains("CREATE TABLE comparisons"));
    assert!(after_up.contains("CREATE TABLE firewall_runs"));
    assert!(after_up.contains("CREATE TABLE calibration"));
    assert!(after_up.contains("CREATE TABLE conformal_thresholds"));
    assert!(after_up.contains("CREATE TABLE factuality_checks"));
    assert!(after_up.contains("CREATE INDEX idx_decisions_episode"));
    assert!(after_up.contains("CREATE INDEX idx_decisions_core"));
    assert!(after_up.contains("CREATE INDEX idx_firewall_runs_episode"));
    assert!(after_up.contains("CREATE INDEX idx_calibration_class"));
    assert!(after_up.contains("CREATE INDEX idx_conformal_class"));
    assert!(after_up.contains("CREATE INDEX idx_factuality_claim"));
    assert!(after_up.contains("CREATE TABLE tool_registry"));
    assert!(after_up.contains("CREATE TABLE policy_sets"));
    assert!(after_up.contains("CREATE TABLE permission_grants"));
    assert!(after_up.contains("CREATE TABLE tool_calls"));
    assert!(after_up.contains("CREATE TABLE idempotency_keys"));
    assert!(after_up.contains("CREATE TABLE action_ledger"));
    assert!(after_up.contains("CREATE TABLE observed_payloads"));
    assert!(after_up.contains("CREATE TABLE rollback_snapshots"));
    assert!(after_up.contains("CREATE TRIGGER action_ledger_no_update"));
    assert!(after_up.contains("CREATE TRIGGER action_ledger_no_delete"));
    assert!(after_up.contains("CREATE INDEX idx_grants_principal"));
    assert!(after_up.contains("CREATE INDEX idx_tool_calls_tool"));
    assert!(after_up.contains("CREATE INDEX idx_tool_calls_trace"));
    assert!(after_up.contains("CREATE INDEX idx_ledger_action"));
    assert!(after_up.contains("CREATE INDEX idx_observed_action"));
    assert!(after_up.contains("CREATE INDEX idx_snapshots_action"));
    assert!(after_up.contains("CREATE TABLE prediction_ledger"));
    assert!(after_up.contains("CREATE TABLE prediction_outcomes"));
    assert!(after_up.contains("CREATE TABLE calibration_runs"));
    assert!(after_up.contains("CREATE TABLE meta_analyses"));
    assert!(after_up.contains("CREATE TABLE change_sets"));
    assert!(after_up.contains("CREATE TABLE promotions"));
    assert!(after_up.contains("CREATE TABLE regression_tests"));
    assert!(after_up.contains("CREATE TABLE evolution_journal"));
    assert!(after_up.contains("CREATE TABLE budgets"));
    assert!(after_up.contains("CREATE TABLE pi_sessions"));
    assert!(after_up.contains("CREATE TABLE pi_events"));
    assert!(after_up.contains("CREATE TRIGGER prediction_ledger_no_update"));
    assert!(after_up.contains("CREATE TRIGGER prediction_outcomes_no_update"));
    assert!(after_up.contains("CREATE TRIGGER evolution_journal_no_delete"));
    assert!(after_up.contains("CREATE INDEX idx_pi_events_session"));
    assert!(after_up.contains("CREATE TABLE loop_runs"));
    assert!(after_up.contains("CREATE TABLE loop_iterations"));
    assert!(after_up.contains("CREATE TABLE self_model_reports"));
    assert!(after_up.contains("CREATE TABLE divergence_metrics"));
    assert!(after_up.contains("CREATE TABLE debt_findings"));
    assert!(after_up.contains("CREATE TABLE gc_actions"));
    assert!(after_up.contains("CREATE TABLE module_loads"));
    assert!(after_up.contains("CREATE TABLE design_revisions"));
    assert!(after_up.contains("CREATE TABLE timescales"));
    assert!(after_up.contains("CREATE TRIGGER gc_actions_protected_no_update"));
    assert!(after_up.contains("CREATE TRIGGER gc_actions_protected_no_delete"));
    assert!(after_up.contains("CREATE INDEX idx_loop_runs_status"));
    assert!(after_up.contains("CREATE INDEX idx_divergence_report"));
    assert!(after_up.contains("CREATE INDEX idx_debt_findings_kind"));
    assert!(after_up.contains("CREATE INDEX idx_gc_actions_finding"));
    assert!(after_up.contains("CREATE INDEX idx_module_loads_uri"));
    assert!(after_up.contains("CREATE INDEX idx_design_revisions_changeset"));
    assert_eq!(
        store.applied_migrations().await.unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    );

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
    assert_eq!(
        store.applied_migrations().await.unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    );
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
