-- Reverse of 0007_library.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- Nothing here has a foreign key (the library cross-references by IRI, because a
-- versioned entry must be insertable without the entry it mentions having a row
-- yet), so the drop order is simply the reverse of the create order. Indexes fall
-- with their tables.

DROP TABLE IF EXISTS applicability_runs;
DROP TABLE IF EXISTS frame_instances;
DROP TABLE IF EXISTS policy_population;
DROP TABLE IF EXISTS policy_fitness;
DROP TABLE IF EXISTS policy_versions;
DROP TABLE IF EXISTS policies;
DROP TABLE IF EXISTS insights;
DROP TABLE IF EXISTS case_map;
DROP TABLE IF EXISTS cases;
DROP TABLE IF EXISTS workflows;
DROP TABLE IF EXISTS skills;
DROP TABLE IF EXISTS library_entries;
DELETE FROM schema_versions WHERE version = 7;
DELETE FROM _sqlx_migrations WHERE version = 7;
