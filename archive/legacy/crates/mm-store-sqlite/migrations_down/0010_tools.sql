-- Reverse of 0010_tools.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- `permission_grants.policy_set_id` references `policy_sets(id)`, so the grants go
-- before the sets; everything else has no foreign key between the new tables and
-- falls in reverse creation order. Triggers and indexes fall with their tables.

DROP TABLE IF EXISTS rollback_snapshots;
DROP TABLE IF EXISTS observed_payloads;
DROP TRIGGER IF EXISTS action_ledger_no_update;
DROP TRIGGER IF EXISTS action_ledger_no_delete;
DROP TABLE IF EXISTS action_ledger;
DROP TABLE IF EXISTS idempotency_keys;
DROP TABLE IF EXISTS tool_calls;
DROP TABLE IF EXISTS permission_grants;
DROP TABLE IF EXISTS policy_sets;
DROP TABLE IF EXISTS tool_registry;
DELETE FROM schema_versions WHERE version = 10;
DELETE FROM _sqlx_migrations WHERE version = 10;
