-- Reverse of 0012_loop.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- Triggers fall with their table, so the explicit DROPs are only for readability.
-- The order is reverse creation with `divergence_metrics` before
-- `self_model_reports`, since the former references the latter.

DROP TABLE IF EXISTS divergence_metrics;
DROP TRIGGER IF EXISTS gc_actions_protected_no_update;
DROP TRIGGER IF EXISTS gc_actions_protected_no_delete;
DROP TABLE IF EXISTS gc_actions;
DROP TABLE IF EXISTS debt_findings;
DROP TABLE IF EXISTS self_model_reports;
DROP TABLE IF EXISTS loop_iterations;
DROP TABLE IF EXISTS loop_runs;
DROP TABLE IF EXISTS module_loads;
DROP TABLE IF EXISTS design_revisions;
DROP TABLE IF EXISTS timescales;
DELETE FROM schema_versions WHERE version = 12;
DELETE FROM _sqlx_migrations WHERE version = 12;
