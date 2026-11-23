-- Reverse of 0013_data_metamind.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.

DROP TRIGGER IF EXISTS data_outbox_tombstone_has_no_doc;
DROP INDEX IF EXISTS data_outbox_pending;
DROP INDEX IF EXISTS data_outbox_subject;
DROP TABLE IF EXISTS data_outbox;
DROP TABLE IF EXISTS data_graphs;
DELETE FROM schema_versions WHERE version = 13;
