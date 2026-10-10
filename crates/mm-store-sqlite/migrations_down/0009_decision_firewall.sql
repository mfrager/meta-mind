-- Reverse of 0009_decision_firewall.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- The six tables have no foreign keys between them (a firewall run names an episode
-- ULID as a correlation, not as a containment), so the drop order is simply the
-- reverse of the create order. Indexes fall with their tables.

DROP TABLE IF EXISTS factuality_checks;
DROP TABLE IF EXISTS conformal_thresholds;
DROP TABLE IF EXISTS calibration;
DROP TABLE IF EXISTS firewall_runs;
DROP TABLE IF EXISTS comparisons;
DROP TABLE IF EXISTS decisions;
DELETE FROM schema_versions WHERE version = 9;
DELETE FROM _sqlx_migrations WHERE version = 9;
