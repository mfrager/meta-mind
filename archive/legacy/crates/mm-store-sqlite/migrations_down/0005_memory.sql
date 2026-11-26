-- Reverse of 0005_memory.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- `DROP TABLE memory_fts` also drops the FTS5 shadow tables (`memory_fts_data`,
-- `memory_fts_idx`, …), so they need no explicit drop. The triggers go first
-- because they reference `memories`, and the tables that carry a foreign key to
-- `memories` go before it.
DROP TRIGGER IF EXISTS memories_fts_insert;
DROP TRIGGER IF EXISTS memories_fts_delete;
DROP TRIGGER IF EXISTS memories_fts_update;
DROP TABLE IF EXISTS memory_fts;
DROP TABLE IF EXISTS memory_access;
DROP TABLE IF EXISTS memory_entities;
DROP TABLE IF EXISTS memory_cues;
DROP TABLE IF EXISTS memory_summaries;
DROP TABLE IF EXISTS memory_communities;
DROP TABLE IF EXISTS memory_consolidations;
DROP TABLE IF EXISTS memory_links;
DROP TABLE IF EXISTS mistakes;
DROP TABLE IF EXISTS entity_edges;
DROP TABLE IF EXISTS memories;
DELETE FROM schema_versions WHERE version = 5;
DELETE FROM _sqlx_migrations WHERE version = 5;
