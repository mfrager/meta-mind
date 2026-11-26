-- Reverse of 0014_llm_models.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.

DROP INDEX IF EXISTS idx_llm_models_complexity;
DROP TABLE IF EXISTS llm_models;
DELETE FROM schema_versions WHERE version = 14;
