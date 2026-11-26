-- Reverse of 0002_codex.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- Dropped in reverse dependency order: `module_file` references `module_index`,
-- so it must go first.
DROP TABLE IF EXISTS codex_run;
DROP TABLE IF EXISTS module_dep;
DROP TABLE IF EXISTS capability_index;
DROP TABLE IF EXISTS symbol_reference;
DROP TABLE IF EXISTS symbol_index;
DROP TABLE IF EXISTS module_file;
DROP TABLE IF EXISTS module_index;
DELETE FROM schema_versions WHERE version = 2;
DELETE FROM _sqlx_migrations WHERE version = 2;
