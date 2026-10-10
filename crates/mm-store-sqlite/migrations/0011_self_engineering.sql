-- Phase 11 — self-engineering, indexed.
--
-- The tables here hold the records of the improvement loop: what the being
-- predicted and what actually happened, how well its confidence matched reality,
-- what it diagnosed when a trigger fired, which typed change it proposed, who
-- decided and why, the regression test a mistake became, the immutable journal of
-- every accepted change, the four budgets, and the Pi sessions the code came from.
--
--   * `prediction_ledger`   — one row per consequential prediction. Append-only:
--                             a prediction is a claim staked *before* the outcome,
--                             and a ledger that can be edited after the fact
--                             measures nothing.
--   * `prediction_outcomes` — the resolution, once. Primary keyed by the
--                             prediction, so a second resolution is impossible
--                             rather than merely discouraged.
--   * `calibration_runs`    — one row per scoring pass: Brier, log loss, ECE,
--                             coverage, selective risk, and the baseline it beat.
--   * `meta_analyses`       — one row per diagnosis: the trigger, the episode, the
--                             error classes and the recurrence/impact scores.
--   * `change_sets`         — a typed proposal with its rollback plan and its
--                             status through the sandbox pipeline.
--   * `promotions`          — the gate's decision and *the written reason*. A
--                             rejection with no reason is not a decision.
--   * `regression_tests`    — the mistake→test compiler's output, with both sides
--                             of the fail-before/pass-after assertion.
--   * `evolution_journal`   — the append-only lineage: every promoted or rejected
--                             change, with the self version it produced.
--   * `budgets`             — limit and spend per kind and period. A debit that
--                             would cross the limit is refused whole; the row is
--                             the audit of what was actually spent.
--   * `pi_sessions`/`pi_events` — the ingested Pi JSONL stream, gapless by `seq`.
--
-- Five rules shape the schema:
--   * **Immutability is a database property, not a convention.** The same
--     `BEFORE UPDATE`/`BEFORE DELETE` triggers that protect `audit_log` and
--     `action_ledger` guard `prediction_ledger`, `prediction_outcomes` and
--     `evolution_journal`. Calibration computed over a ledger that a later writer
--     may edit is not evidence; the trigger is what makes the earlier rows
--     immutable even for a writer that never goes through `mm-metaanalysis`.
--   * **A resolution happens once.** `prediction_outcomes` is keyed by the
--     prediction, so "resolve it twice, keeping the better score" cannot be
--     expressed as SQL.
--   * **Every decision carries a reason.** `promotions.reason` is `NOT NULL`, and
--     `meta_analyses.diagnosis` likewise, because the phase's gate asks for a
--     rejection *with a written reason* and a nullable column would let that
--     requirement decay into an empty string.
--   * **Budget is enforced by arithmetic on stored rows.** `spent_amount` never
--     exceeds `limit_amount`; the check constraint states that here and the
--     ledger's debit method enforces it before writing.
--   * **Pi events are gapless per session.** `UNIQUE (session_ulid, seq)` makes a
--     dropped event a write failure rather than a gap nobody notices.
--
-- Two naming deviations from the plan's §4.2 sketch, recorded here for the same
-- reason 0009 and 0010 record theirs:
--
--   * The plan calls the ledger `predictions`, which migration 0006 already owns
--     with a different, *claim*-scoped shape (`proposition_uri`, `outcome_status`,
--     `outcome_value`, bi-temporal columns). A Phase 11 prediction is an
--     operational bet whose calibration is scored, keyed by a calibration *class*
--     rather than by a claim; reusing the name would either force a schema change
--     to Phase 6's table or silently mix two meanings in one column set. The
--     ledger is therefore `prediction_ledger`, and the resolution is
--     `prediction_outcomes` rather than an `outcome_status` column on it.
--   * `prediction_ledger` gains `class`, because calibration, `threshold_for` and
--     `calibration_runs.subject` are all per-class, and a class derived at read
--     time from `conditions_json` would make a stored run unreproducible.
--
-- Times are RFC3339; ids are ULIDs rendered as `TEXT(26)`. The migration number is
-- the phase number (parent plan §4): Phase 11 owns 0011.

PRAGMA journal_mode=WAL;

-- --------------------------------------------------------- prediction ledger ---

CREATE TABLE prediction_ledger (
  id               TEXT(26) PRIMARY KEY,                 -- ULID
  class            TEXT NOT NULL,                        -- the calibration subject
  proposition      TEXT NOT NULL,
  probability      REAL NOT NULL CHECK (probability BETWEEN 0 AND 1),
  horizon_seconds  INTEGER NOT NULL CHECK (horizon_seconds > 0),
  conditions_json  TEXT NOT NULL,
  episode_ulid     TEXT(26),
  created_at       TEXT NOT NULL
);

CREATE INDEX idx_prediction_ledger_class ON prediction_ledger(class, created_at);

-- Append-only, enforced by the database (same shape as the `audit_log` guards in
-- 0001_kernel.sql). A prediction that can be rewritten after its outcome is known
-- is a prediction that cannot be scored.
CREATE TRIGGER prediction_ledger_no_update
BEFORE UPDATE ON prediction_ledger
BEGIN
  SELECT RAISE(ABORT, 'prediction_ledger is append-only');
END;

CREATE TRIGGER prediction_ledger_no_delete
BEFORE DELETE ON prediction_ledger
BEGIN
  SELECT RAISE(ABORT, 'prediction_ledger is append-only');
END;

-- ------------------------------------------------------------ prediction outcomes

CREATE TABLE prediction_outcomes (
  prediction_ulid TEXT(26) PRIMARY KEY REFERENCES prediction_ledger(id),
  observed        INTEGER NOT NULL CHECK (observed IN (0,1)),
  resolved_at     TEXT NOT NULL,
  evidence_ulid   TEXT(26) NOT NULL                      -- Phase 6 Evidence ULID
);

-- A resolution is immutable for the same reason the prediction is: re-resolving
-- until the calibration looks good is the failure mode this table exists to
-- prevent. The primary key stops a second row; the trigger stops an edit of the
-- first.
CREATE TRIGGER prediction_outcomes_no_update
BEFORE UPDATE ON prediction_outcomes
BEGIN
  SELECT RAISE(ABORT, 'prediction_outcomes is append-only');
END;

-- -------------------------------------------------------------- calibration ----

CREATE TABLE calibration_runs (
  id              TEXT(26) PRIMARY KEY,                  -- ULID
  subject         TEXT NOT NULL,                         -- the class that was scored
  n               INTEGER NOT NULL CHECK (n >= 0),
  brier           REAL NOT NULL,
  log_loss        REAL NOT NULL,
  ece             REAL NOT NULL,
  coverage        REAL,
  selective_risk  REAL,
  baseline_brier  REAL,
  created_at      TEXT NOT NULL
);

CREATE INDEX idx_calibration_runs_subject ON calibration_runs(subject, created_at);

-- --------------------------------------------------------------- meta-analysis -

CREATE TABLE meta_analyses (
  id                  TEXT(26) PRIMARY KEY,               -- ULID
  trigger             TEXT NOT NULL,                      -- Trigger, snake_case
  episode_ulid        TEXT(26),
  diagnosis           TEXT NOT NULL,                      -- the written explanation
  error_classes_json  TEXT NOT NULL,                      -- [ErrorClass]
  recurrence          REAL NOT NULL CHECK (recurrence BETWEEN 0 AND 1),
  impact              REAL NOT NULL CHECK (impact BETWEEN 0 AND 1),
  created_at          TEXT NOT NULL
);

CREATE INDEX idx_meta_analyses_trigger ON meta_analyses(trigger, created_at);

-- ---------------------------------------------------------------- change sets --

CREATE TABLE change_sets (
  id              TEXT(26) PRIMARY KEY,                   -- ULID
  reason          TEXT NOT NULL,
  hypothesis      TEXT NOT NULL,
  payload_json    TEXT NOT NULL,                          -- the typed ChangeSet body
  rollback_json   TEXT NOT NULL,                          -- a change set always has one
  status          TEXT NOT NULL CHECK (status IN
                    ('draft','sandboxed','built','tested','benchmarked','shadowed','promoted','rejected')),
  created_at      TEXT NOT NULL
);

CREATE INDEX idx_change_sets_status ON change_sets(status, created_at);

-- ----------------------------------------------------------------- promotions --

CREATE TABLE promotions (
  id              TEXT(26) PRIMARY KEY,                   -- ULID
  changeset_ulid  TEXT(26) NOT NULL REFERENCES change_sets(id),
  decision        TEXT NOT NULL CHECK (decision IN ('promote','reject')),
  reason          TEXT NOT NULL,                          -- the written reason, never empty
  evidence_json   TEXT NOT NULL,
  created_at      TEXT NOT NULL
);

CREATE INDEX idx_promotions_changeset ON promotions(changeset_ulid);

-- ------------------------------------------------------------ regression tests --

CREATE TABLE regression_tests (
  id                 TEXT(26) PRIMARY KEY,                -- ULID
  mistake_ulid       TEXT(26),
  path               TEXT NOT NULL UNIQUE,                -- bench/regression/<case>
  fails_before_ulid  TEXT(26),
  passes_after_ulid  TEXT(26),
  created_at         TEXT NOT NULL
);

-- ------------------------------------------------------------ evolution journal -

CREATE TABLE evolution_journal (
  id              TEXT(26) PRIMARY KEY,                   -- ULID
  date            TEXT NOT NULL,
  reason          TEXT NOT NULL,
  evidence_json   TEXT NOT NULL,
  hypothesis      TEXT NOT NULL,
  changeset_ulid  TEXT(26),
  outcome         TEXT NOT NULL,
  decision        TEXT NOT NULL,
  self_version    TEXT NOT NULL
);

CREATE INDEX idx_evolution_journal_version ON evolution_journal(self_version, date);

-- The lineage is the self's history: appending is the only operation, and later
-- phases read it to answer "which version was I, and why".
CREATE TRIGGER evolution_journal_no_update
BEFORE UPDATE ON evolution_journal
BEGIN
  SELECT RAISE(ABORT, 'evolution_journal is append-only');
END;

CREATE TRIGGER evolution_journal_no_delete
BEFORE DELETE ON evolution_journal
BEGIN
  SELECT RAISE(ABORT, 'evolution_journal is append-only');
END;

-- -------------------------------------------------------------------- budgets --

CREATE TABLE budgets (
  id            TEXT(26) PRIMARY KEY,                     -- ULID
  kind          TEXT NOT NULL CHECK (kind IN
                  ('meta_analysis','improvement','evolution','metacognitive')),
  period        TEXT NOT NULL,                            -- operator-chosen, e.g. `cycle-1`
  limit_amount  REAL NOT NULL CHECK (limit_amount >= 0),
  spent_amount  REAL NOT NULL DEFAULT 0 CHECK (spent_amount >= 0),
  UNIQUE (kind, period)
);

-- ---------------------------------------------------------------- pi sessions --

CREATE TABLE pi_sessions (
  id              TEXT(26) PRIMARY KEY,                   -- ULID
  session_file    TEXT NOT NULL,                          -- data/sandbox/pi/<file>.jsonl
  changeset_ulid  TEXT(26),
  model           TEXT,
  cwd             TEXT,
  started_at      TEXT NOT NULL,
  ended_at        TEXT,
  event_count     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE pi_events (
  id            TEXT(26) PRIMARY KEY,                     -- ULID
  session_ulid  TEXT(26) NOT NULL REFERENCES pi_sessions(id),
  seq           INTEGER NOT NULL,                         -- gapless within a session
  kind          TEXT NOT NULL,                            -- the record's type
  payload_json  TEXT NOT NULL,
  UNIQUE (session_ulid, seq)
);

CREATE INDEX idx_pi_events_session ON pi_events(session_ulid, seq);

-- ------------------------------------------------------------------ seed data --
--
-- The four budgets of the first period. Seeded rather than created on first use so
-- a fresh kernel has a hard cap from its first command — "the evolution budget is
-- exhausted" must be a fact of the shipped data, not of a code path that may not
-- have run yet. The period is a name the operator chooses; a new period is a new
-- set of rows, and the old ones stay as the audit of what was spent.

INSERT INTO budgets (id, kind, period, limit_amount, spent_amount) VALUES
  ('01h0000000000000000000b110', 'meta_analysis', 'cycle-1', 200.0, 0.0),
  ('01h0000000000000000000b111', 'improvement',   'cycle-1',  50.0, 0.0),
  ('01h0000000000000000000b112', 'evolution',     'cycle-1',  20.0, 0.0),
  ('01h0000000000000000000b113', 'metacognitive', 'cycle-1', 500.0, 0.0);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('self_engineering', 11, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
