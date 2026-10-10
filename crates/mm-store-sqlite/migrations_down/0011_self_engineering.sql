-- Reverse of 0011_self_engineering.sql, used only by the migration round-trip
-- test. Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- `prediction_outcomes.prediction_ulid` references `prediction_ledger(id)`, and
-- `promotions.changeset_ulid` references `change_sets(id)`, and
-- `pi_events.session_ulid` references `pi_sessions(id)`, so each child table goes
-- before its parent. Triggers and indexes fall with their tables.

DROP TABLE IF EXISTS pi_events;
DROP TABLE IF EXISTS pi_sessions;
DROP TABLE IF EXISTS budgets;
DROP TRIGGER IF EXISTS evolution_journal_no_update;
DROP TRIGGER IF EXISTS evolution_journal_no_delete;
DROP TABLE IF EXISTS evolution_journal;
DROP TABLE IF EXISTS regression_tests;
DROP TABLE IF EXISTS promotions;
DROP TABLE IF EXISTS change_sets;
DROP TABLE IF EXISTS meta_analyses;
DROP TABLE IF EXISTS calibration_runs;
DROP TRIGGER IF EXISTS prediction_outcomes_no_update;
DROP TABLE IF EXISTS prediction_outcomes;
DROP TRIGGER IF EXISTS prediction_ledger_no_update;
DROP TRIGGER IF EXISTS prediction_ledger_no_delete;
DROP TABLE IF EXISTS prediction_ledger;
DELETE FROM schema_versions WHERE version = 11;
DELETE FROM _sqlx_migrations WHERE version = 11;
