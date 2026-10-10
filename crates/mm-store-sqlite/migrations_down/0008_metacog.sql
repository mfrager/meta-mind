-- Reverse of 0008_metacog.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- `program_traces` and `programs` both reference `episodes`, so the drop order is
-- the reverse of the create order and the children go first. Indexes fall with
-- their tables.

DROP TABLE IF EXISTS program_traces;
DROP TABLE IF EXISTS programs;
DROP TABLE IF EXISTS episodes;
DELETE FROM schema_versions WHERE version = 8;
DELETE FROM _sqlx_migrations WHERE version = 8;
