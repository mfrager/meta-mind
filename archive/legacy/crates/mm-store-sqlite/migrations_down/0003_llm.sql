-- Reverse of 0003_llm.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- Dropped in reverse dependency order: `llm_repair_attempts` references
-- `llm_calls`, so it must go first. SQLite drops a table's indexes with it, so
-- the explicit index creations need no matching drops.
DROP TABLE IF EXISTS llm_repair_attempts;
DROP TABLE IF EXISTS llm_cache_index;
DROP TABLE IF EXISTS llm_semantic_cache;
DROP TABLE IF EXISTS llm_routing;
DROP TABLE IF EXISTS llm_grammars;
DROP TABLE IF EXISTS llm_calls;
DELETE FROM schema_versions WHERE version = 3;
DELETE FROM _sqlx_migrations WHERE version = 3;
