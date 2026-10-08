-- Reverse of 0001_kernel.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
DROP TRIGGER IF EXISTS audit_log_no_update;
DROP TRIGGER IF EXISTS audit_log_no_delete;
DROP TRIGGER IF EXISTS facts_no_overlapping_system_time;
DROP TABLE IF EXISTS facts;
DROP TABLE IF EXISTS store_checkpoints;
DROP TABLE IF EXISTS audit_log;
DROP TABLE IF EXISTS events;
DROP TABLE IF EXISTS schema_versions;
DELETE FROM _sqlx_migrations;
