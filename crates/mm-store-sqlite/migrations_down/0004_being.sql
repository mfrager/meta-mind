-- Reverse of 0004_being.sql, used only by the migration round-trip test.
-- Kept outside `migrations/` because sqlx applies every `.sql` file it finds
-- there, and a down script must never run as a forward migration.
--
-- Dropped in reverse dependency order: `commitment_transitions` references
-- `commitments`, `relationship_events` references `relationships`, and both
-- `goal_transitions` and `commitments` reference `goals`, so the dependants go
-- first. SQLite drops a table's indexes and triggers with it, so those need no
-- explicit drops here.
DROP TRIGGER IF EXISTS goals_terminal_is_immutable;
DROP TRIGGER IF EXISTS commitments_terminal_is_immutable;
DROP TABLE IF EXISTS commitment_transitions;
DROP TABLE IF EXISTS commitments;
DROP TABLE IF EXISTS goal_transitions;
DROP TABLE IF EXISTS goals;
DROP TABLE IF EXISTS relationship_events;
DROP TABLE IF EXISTS relationships;
DROP TABLE IF EXISTS user_beliefs;
DROP TABLE IF EXISTS affect_impulses;
DROP TABLE IF EXISTS affect_state;
DROP TABLE IF EXISTS motivations;
DROP TABLE IF EXISTS personality_context_modifiers;
DROP TABLE IF EXISTS personality_dispositions;
DROP TABLE IF EXISTS personality_values;
DROP TABLE IF EXISTS core_blocks;
DROP TABLE IF EXISTS invariants;
DROP TABLE IF EXISTS identity;
DROP TABLE IF EXISTS resource_ledger;
DROP TABLE IF EXISTS resource_accounts;
DROP TABLE IF EXISTS budget_policies;
DELETE FROM schema_versions WHERE version = 4;
DELETE FROM _sqlx_migrations WHERE version = 4;
