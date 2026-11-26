-- Reverse of 0006_epistemic.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- Tables are dropped child-first: `evidence` and `observations` carry a foreign
-- key to `claims`, so they go before it. Indexes fall with their tables, and
-- `epistemic_transitions` references nothing, so it goes last.
DROP TABLE IF EXISTS epistemic_transitions;
DROP TABLE IF EXISTS dependencies;
DROP TABLE IF EXISTS contradictions;
DROP TABLE IF EXISTS predictions;
DROP TABLE IF EXISTS assumptions;
DROP TABLE IF EXISTS observations;
DROP TABLE IF EXISTS evidence;
DROP TABLE IF EXISTS claims;
DELETE FROM schema_versions WHERE version = 6;
DELETE FROM _sqlx_migrations WHERE version = 6;
