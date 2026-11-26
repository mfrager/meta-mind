-- Phase 8 — the metacognitive controller, indexed.
--
-- An episode is a bounded deliberation: a provisional model, a computed tier, the
-- program compiled from it, and the trace of what the controller actually spent.
-- Three tables, and each one answers a different question an operator asks:
--
--   * `episodes`      — what was being decided, and how it ended.
--   * `programs`      — which typed graph of operations was compiled, immutable per
--                       `(episode, version)`, with the hash of the DAG it lowers to.
--   * `program_traces`— every operation the controller *considered*, in order, with
--                       the score it was ranked by and whether it ran.
--
-- Four rules shape the schema:
--   * **Trace, not just result.** A skipped operation is a row too (`selected = 0`),
--     because "the controller chose not to" is as much of the deliberation as what
--     it did, and a replay has to reproduce both.
--   * **Immutable programs.** `UNIQUE (episode_id, version)` and no UPDATE path: a
--     recompiled program is a new version, and the old one stays byte-identical.
--   * **Bounded budgets are recorded as spent, not as limits.** `budget_json` holds
--     the limits; the spent side lives in the episode's status row and the traces,
--     so a budget audit reads what happened rather than recomputing it.
--   * **Bi-temporal like every other projection.** `system_from`/`system_to` and
--     `valid_from`/`valid_to` are separate columns.
--
-- Times are RFC3339; ids are ULIDs rendered as `TEXT(26)`. The migration number is
-- the phase number (parent plan §4): Phase 8 owns 0008.

PRAGMA journal_mode=WAL;

-- ---------------------------------------------------------------- episodes ----

CREATE TABLE episodes (
  id            TEXT(26) PRIMARY KEY,                       -- ULID
  goal          TEXT NOT NULL,
  tier          INTEGER NOT NULL CHECK (tier BETWEEN 0 AND 5),
  stakes        REAL NOT NULL CHECK (stakes BETWEEN 0.0 AND 1.0),
  uncertainty   REAL NOT NULL CHECK (uncertainty BETWEEN 0.0 AND 1.0),
  reversibility REAL NOT NULL CHECK (reversibility BETWEEN 0.0 AND 1.0),
  status        TEXT NOT NULL CHECK (status IN ('open','programmed','stopped','closed')),
  budget_json   TEXT NOT NULL,
  trace_id      TEXT(26) NOT NULL,
  created_ulid  TEXT(26) NOT NULL,
  system_from   TEXT NOT NULL,
  system_to     TEXT,
  valid_from    TEXT NOT NULL,
  valid_to      TEXT
);

-- ---------------------------------------------------------------- programs ----

CREATE TABLE programs (
  id                   TEXT(26) PRIMARY KEY,                -- ULID
  episode_id           TEXT(26) NOT NULL REFERENCES episodes(id),
  version              INTEGER NOT NULL,
  tier                 INTEGER NOT NULL CHECK (tier BETWEEN 0 AND 5),
  program_json         TEXT NOT NULL,                       -- canonical serde
  dag_hash             TEXT NOT NULL,                       -- sha256 hex
  operation_value_json TEXT NOT NULL,
  created_ulid         TEXT(26) NOT NULL,
  UNIQUE (episode_id, version)
);

-- ----------------------------------------------------------------- traces -----

CREATE TABLE program_traces (
  id                 TEXT(26) PRIMARY KEY,                  -- ULID
  program_id         TEXT(26) NOT NULL REFERENCES programs(id),
  seq                INTEGER NOT NULL,
  node_id            INTEGER NOT NULL,
  op                 TEXT NOT NULL,
  op_class           TEXT NOT NULL,
  selected           INTEGER NOT NULL CHECK (selected IN (0,1)),
  value_json         TEXT NOT NULL,                         -- {eer, importance, p_change, cost, score}
  outcome            TEXT NOT NULL CHECK (outcome IN ('executed','skipped','failed','stopped')),
  budget_after_json  TEXT NOT NULL,
  stopping_reason    TEXT,
  at_ulid            TEXT(26) NOT NULL,
  UNIQUE (program_id, seq)
);

CREATE INDEX idx_programs_episode ON programs(episode_id);
CREATE INDEX idx_traces_program_seq ON program_traces(program_id, seq);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('metacog', 8, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
