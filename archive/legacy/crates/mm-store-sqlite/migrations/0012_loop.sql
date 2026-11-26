-- Phase 12 — autonomy: the closed developmental loop, projected.
--
-- Every table here is a *projection of the event log*, never an authority. The run
-- itself is the sequence of committed events the loop appends (one per stage), and
-- these rows are what a reader folds those events into: which stages ran, what each
-- one produced, what the self-model measured, what debt was found, what was collected,
-- which module versions were hot-loaded, and which design revisions a promoted change
-- set produced. The rule the parent plan states — "no authoritative mutable state" —
-- is why `loop_runs` carries `digest` instead of a status a stage merely overwrites:
-- the digest is the fold of the run's own events, so a row whose digest does not
-- recompute from the log is a row that has drifted from the source of truth.
--
--   * `loop_runs`         — one row per run: the goal, whether it was novel, the
--                           `(code_version, config_hash)` pair replay is defined
--                           against, the folded `digest`, and the terminal status.
--   * `loop_iterations`   — one row per `(run_id, idx)`. The UNIQUE constraint is the
--                           idempotency key: a resumed stage writes the same row again
--                           rather than a second one, so a crash cannot double-apply a
--                           stage's side effects.
--   * `self_model_reports`— one numeric measurement of Actual/Model/Ideal divergence.
--   * `divergence_metrics`— one row per dimension, so a report is a vector and not a
--                           single number that hides which axis moved.
--   * `debt_findings`     — what the scanner found, with the evidence that showed it.
--                           A finding with no evidence is not a finding.
--   * `gc_actions`        — what the collector proposed or applied, and the two flags
--                           that make the phase's central guard checkable in SQL:
--                           `reversible` and `protects_ledger`.
--   * `module_loads`      — the hot-load history. Prior versions stay rows, because
--                           "the previous version kept serving" is only checkable if
--                           it is still recorded.
--   * `design_revisions`  — a design document's revision number and the change set
--                           that produced it. UNIQUE `(doc_uri, revision)` makes a
--                           re-used revision number a write failure.
--   * `timescales`        — the five scheduler clocks and the last tick each recorded.
--
-- Three rules shape the schema, each mirroring an earlier phase:
--   * **The developmental ledger is immutable.** `gc_actions` carries
--     `protects_ledger`, and a `BEFORE UPDATE`/`BEFORE DELETE` trigger refuses to
--     touch a row that asserts it. The guard the collector applies in code is thus
--     also a property of the database: an action that named a protected subject
--     cannot be erased after the fact, so the refusal stays visible.
--   * **A stage is idempotent.** `UNIQUE (run_id, idx)` plus `INSERT ... ON CONFLICT
--     DO UPDATE` on the outcome is what makes resume safe; a stage that ran twice
--     writes one row, not two.
--   * **A decision is announced and reasoned.** `module_loads` and
--     `design_revisions` both record what happened; a rollback is a new row with
--     `status = 'rolled_back'`, not a deletion of the load it undid.
--
-- Two deviations from the plan's §4.2 sketch, recorded here so the diff is not
-- discovered later:
--   * `loop_runs` gains `digest`, because "replay reproduces the run byte-identically"
--     needs a byte to compare against, and computing it from the log and *also*
--     storing it is what makes the comparison a check rather than a tautology.
--   * `gc_actions` gains the `protects_ledger` CHECK and the append-only triggers; the
--     plan states the property in prose, and the phase's own rule is that a promise
--     the database can keep, the database keeps.
--
-- Times are RFC3339; ids are ULIDs rendered as `TEXT(26)`. The migration number is the
-- phase number (parent plan §4): Phase 12 owns 0012.

PRAGMA journal_mode=WAL;

-- ------------------------------------------------------------------- loop runs --

CREATE TABLE loop_runs (
  id            TEXT(26) PRIMARY KEY,                    -- ULID
  goal_ulid     TEXT(26) NOT NULL,
  goal_text     TEXT NOT NULL,
  novel         INTEGER NOT NULL DEFAULT 0 CHECK (novel IN (0, 1)),
  status        TEXT NOT NULL CHECK (status IN ('running','completed','failed','aborted')),
  config_hash   TEXT NOT NULL,
  code_version  TEXT NOT NULL,
  digest        TEXT NOT NULL,                           -- fold of the run's events
  started_at    TEXT NOT NULL,
  ended_at      TEXT
);

CREATE INDEX idx_loop_runs_status ON loop_runs(status, started_at);

-- -------------------------------------------------------------- loop iterations --

CREATE TABLE loop_iterations (
  id          TEXT(26) PRIMARY KEY,                      -- ULID
  run_id      TEXT(26) NOT NULL REFERENCES loop_runs(id),
  idx         INTEGER NOT NULL CHECK (idx >= 0),
  stage       TEXT NOT NULL,
  outcome     TEXT NOT NULL,                             -- 'ok' or a refusal code
  detail      TEXT NOT NULL,
  started_at  TEXT NOT NULL,
  ended_at    TEXT,
  UNIQUE (run_id, idx)
);

-- -------------------------------------------------------------- self-model reports --

CREATE TABLE self_model_reports (
  id            TEXT(26) PRIMARY KEY,                    -- ULID
  run_id        TEXT(26) REFERENCES loop_runs(id),
  actual_model  REAL NOT NULL CHECK (actual_model BETWEEN 0 AND 1),
  actual_ideal  REAL NOT NULL CHECK (actual_ideal BETWEEN 0 AND 1),
  model_ideal   REAL NOT NULL CHECK (model_ideal BETWEEN 0 AND 1),
  created_at    TEXT NOT NULL
);

CREATE TABLE divergence_metrics (
  id         TEXT(26) PRIMARY KEY,                       -- ULID
  report_id  TEXT(26) NOT NULL REFERENCES self_model_reports(id),
  dimension  TEXT NOT NULL,
  value      REAL NOT NULL CHECK (value >= 0),
  UNIQUE (report_id, dimension)
);

CREATE INDEX idx_divergence_report ON divergence_metrics(report_id);

-- ---------------------------------------------------------------- debt findings --

CREATE TABLE debt_findings (
  id             TEXT(26) PRIMARY KEY,                   -- ULID
  run_id         TEXT(26),
  kind           TEXT NOT NULL CHECK (kind IN
                   ('unused_capability','duplicate_policy','conflicting_policy','stale_memory',
                    'unused_schema','expensive_workflow','obsolete_technique','complexity')),
  subject_uri    TEXT NOT NULL,
  severity       REAL NOT NULL CHECK (severity BETWEEN 0 AND 1),
  evidence_json  TEXT NOT NULL,                          -- non-empty: a finding stands on evidence
  created_at     TEXT NOT NULL
);

CREATE INDEX idx_debt_findings_kind ON debt_findings(kind, severity);

-- ------------------------------------------------------------------- gc actions --

CREATE TABLE gc_actions (
  id              TEXT(26) PRIMARY KEY,                  -- ULID
  finding_id      TEXT(26) REFERENCES debt_findings(id),
  action          TEXT NOT NULL CHECK (action IN
                    ('temp_reasoning','context','data','policy','prompt','skill','code',
                     'architecture','model')),
  subject_uri     TEXT NOT NULL,
  reversible      INTEGER NOT NULL CHECK (reversible IN (0, 1)),
  ledger_impact   TEXT NOT NULL CHECK (ledger_impact IN ('none','metadata','row')),
  protects_ledger INTEGER NOT NULL DEFAULT 0 CHECK (protects_ledger IN (0, 1)),
  applied         INTEGER NOT NULL DEFAULT 0 CHECK (applied IN (0, 1)),
  created_at      TEXT NOT NULL
);

CREATE INDEX idx_gc_actions_finding ON gc_actions(finding_id);

-- The immutable developmental ledger, as a database property. A row that asserts it
-- protects the ledger may not be rewritten or deleted: the refusal has to survive the
-- process that recorded it, or "the ledger was never touched" is a claim about this
-- build's code rather than about the data.
CREATE TRIGGER gc_actions_protected_no_update
BEFORE UPDATE ON gc_actions
WHEN OLD.protects_ledger = 1
BEGIN
  SELECT RAISE(ABORT, 'gc_actions for a protected ledger subject is append-only');
END;

CREATE TRIGGER gc_actions_protected_no_delete
BEFORE DELETE ON gc_actions
WHEN OLD.protects_ledger = 1
BEGIN
  SELECT RAISE(ABORT, 'gc_actions for a protected ledger subject is append-only');
END;

-- ----------------------------------------------------------------- module loads --

CREATE TABLE module_loads (
  id           TEXT(26) PRIMARY KEY,                     -- ULID
  module_uri   TEXT NOT NULL,
  version      TEXT NOT NULL,
  status       TEXT NOT NULL CHECK (status IN ('loaded','rolled_back','failed')),
  functions    INTEGER NOT NULL DEFAULT 0,
  latency_ms   INTEGER NOT NULL DEFAULT 0,
  loaded_at    TEXT NOT NULL
);

CREATE INDEX idx_module_loads_uri ON module_loads(module_uri, loaded_at);

-- ------------------------------------------------------------- design revisions --

CREATE TABLE design_revisions (
  id             TEXT(26) PRIMARY KEY,                   -- ULID
  doc_uri        TEXT NOT NULL,
  doc_path       TEXT NOT NULL,
  revision       INTEGER NOT NULL CHECK (revision >= 1),
  change_set_id  TEXT(26) NOT NULL,
  promoted       INTEGER NOT NULL DEFAULT 0 CHECK (promoted IN (0, 1)),
  created_at     TEXT NOT NULL,
  UNIQUE (doc_uri, revision)
);

CREATE INDEX idx_design_revisions_changeset ON design_revisions(change_set_id);

-- ------------------------------------------------------------------ timescales --

CREATE TABLE timescales (
  name           TEXT PRIMARY KEY,
  kind           TEXT NOT NULL CHECK (kind IN
                   ('action','episode','project','goal','identity')),
  tick_ms        INTEGER NOT NULL CHECK (tick_ms > 0),
  handler        TEXT NOT NULL,
  last_tick_ulid TEXT(26),
  last_tick_at   TEXT
);

-- The five clocks of §31, seeded rather than created on first use: "Identity never
-- ticks during a run" has to be a fact of the shipped data, not of a code path that
-- may not have run. Identity's tick is the longest, and the scheduler refuses to
-- fire it while a loop run is open (recorded in the handler name).
INSERT INTO timescales (name, kind, tick_ms, handler) VALUES
  ('action',   'action',         1000, 'loop.action_tick'),
  ('episode',  'episode',       60000, 'loop.episode_tick'),
  ('project',  'project',     3600000, 'loop.project_tick'),
  ('goal',     'goal',       86400000, 'loop.goal_tick'),
  ('identity', 'identity', 31536000000, 'identity.freeze');

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('autonomy', 12, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
