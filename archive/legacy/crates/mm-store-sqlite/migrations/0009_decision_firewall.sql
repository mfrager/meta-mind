-- Phase 9 — bounded decisions, comparisons, firewall runs, calibration.
--
-- Six tables, and each one answers a question a later phase actually asks:
--
--   * `decisions`            — one bounded question, the answer, the features it was
--                              made from, which core answered, and (once the
--                              episode ends) the outcome it was graded against.
--   * `comparisons`          — one comparison check: what was compared, the verdict,
--                              the violations, and the normalized values when the
--                              verdict was `valid`.
--   * `firewall_runs`        — one evaluation of one episode: the input, the
--                              outcome, the reason codes, the decisions consulted
--                              and the prohibition that short-circuited it.
--   * `calibration`          — one fitted temperature per decision class.
--   * `conformal_thresholds` — one split-conformal `q_hat` per decision class.
--   * `factuality_checks`    — one claim-level support score per claim and check.
--
-- Five rules shape the schema:
--
--   * **Features are mandatory.** `decisions.features_json` is `NOT NULL` and every
--     row carries it, because Phase 11 fits a calibrator on exactly this column and
--     a decision recorded without its features cannot be calibrated later — the
--     gate counts nulls in it for that reason.
--   * **Reason codes are mandatory.** `firewall_runs.reason_codes_json` is `NOT
--     NULL` too: a run with no reasons is a run nothing can explain, and the
--     aggregation always writes at least `signal.nominal`.
--   * **Confidence is bounded in the schema as well as in the type.** The CHECK is
--     not redundant with `DecisionAnswer::new`: a direct `INSERT` from a migration
--     or a repair script does not go through the type, and the constraint is what
--     makes the column's meaning true of the table rather than of one writer.
--   * **Calibration and conformal rows are versioned by `created_ulid`, not
--     updated.** A refit is a new row, so a decision can name the threshold that
--     admitted it and an operator can see when the threshold changed. That is also
--     why the plan asks for a per-class index on the newest row.
--   * **No foreign keys into `episodes`.** A firewall run may be recorded for an
--     episode the tabular `episodes` row has not been written for — `mm-cli firewall
--     eval` grades a corpus of inputs against a template episode, none of which are
--     deliberations the controller opened. A foreign key would turn that into a
--     failed insert rather than a recorded fact, and the ULID is a correlation, not
--     a containment.
--
-- The plan (§4.7) spells the index names `decisions_episode` and so on; they are
-- written here with the `idx_` prefix the other phase migrations use, so one naming
-- convention holds across the schema.
--
-- Times are RFC3339; ids are ULIDs rendered as `TEXT(26)`. The migration number is
-- the phase number (parent plan §4): Phase 9 owns 0009.

PRAGMA journal_mode=WAL;

-- --------------------------------------------------------------- decisions ----

CREATE TABLE decisions (
  id                   TEXT(26) PRIMARY KEY,                  -- ULID
  question_kind        TEXT NOT NULL CHECK (question_kind IN ('choice','score','yes_no')),
  question_json        TEXT NOT NULL,
  answer_json          TEXT NOT NULL,
  confidence           REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
  calibrated_confidence REAL CHECK (calibrated_confidence IS NULL
                                    OR (calibrated_confidence >= 0.0 AND calibrated_confidence <= 1.0)),
  features_json        TEXT NOT NULL,
  core_impl            TEXT NOT NULL CHECK (core_impl IN ('rules','local','hosted')),
  latency_ms           INTEGER NOT NULL CHECK (latency_ms >= 0),
  cost                 REAL NOT NULL DEFAULT 0.0 CHECK (cost >= 0.0),
  created_ulid         TEXT(26) NOT NULL,
  episode_ulid         TEXT(26),
  outcome_json         TEXT,
  outcome_ulid         TEXT(26)
);

CREATE INDEX idx_decisions_episode ON decisions(episode_ulid);
CREATE INDEX idx_decisions_core ON decisions(core_impl, question_kind);

-- ------------------------------------------------------------- comparisons ----

CREATE TABLE comparisons (
  id              TEXT(26) PRIMARY KEY,                       -- ULID
  objective       TEXT NOT NULL,
  contract_json   TEXT NOT NULL,
  verdict         TEXT NOT NULL CHECK (verdict IN ('valid','non_comparable','rejected')),
  violations_json TEXT NOT NULL,
  normalized_json TEXT,
  created_ulid    TEXT(26) NOT NULL
);

-- ---------------------------------------------------------- firewall runs -----

CREATE TABLE firewall_runs (
  id                  TEXT(26) PRIMARY KEY,                   -- ULID
  episode_ulid        TEXT(26) NOT NULL,
  inputs_json         TEXT NOT NULL,
  outcome             TEXT NOT NULL CHECK (outcome IN
                        ('PROCEED','PROCEED_WITH_CAUTION','VERIFY_FIRST','ASK_USER',
                         'REPLAN','HUMAN_REVIEW','REJECT')),
  reason_codes_json   TEXT NOT NULL,
  decision_ulids      TEXT NOT NULL,
  hard_prohibition    TEXT,
  created_ulid        TEXT(26) NOT NULL
);

CREATE INDEX idx_firewall_runs_episode ON firewall_runs(episode_ulid);

-- ------------------------------------------------------------ calibration -----

CREATE TABLE calibration (
  id             TEXT(26) PRIMARY KEY,                        -- ULID
  decision_class TEXT NOT NULL,
  temperature    REAL NOT NULL CHECK (temperature > 0.0),
  ece            REAL NOT NULL CHECK (ece >= 0.0 AND ece <= 1.0),
  brier          REAL NOT NULL CHECK (brier >= 0.0 AND brier <= 1.0),
  n              INTEGER NOT NULL CHECK (n >= 0),
  created_ulid   TEXT(26) NOT NULL
);

CREATE INDEX idx_calibration_class ON calibration(decision_class, created_ulid);

-- ------------------------------------------------------ conformal thresholds ---

CREATE TABLE conformal_thresholds (
  id             TEXT(26) PRIMARY KEY,                        -- ULID
  decision_class TEXT NOT NULL,
  target_coverage REAL NOT NULL CHECK (target_coverage > 0.0 AND target_coverage < 1.0),
  q_hat          REAL NOT NULL CHECK (q_hat >= 0.0 AND q_hat <= 1.0),
  n              INTEGER NOT NULL CHECK (n >= 0),
  created_ulid   TEXT(26) NOT NULL
);

CREATE INDEX idx_conformal_class ON conformal_thresholds(decision_class, created_ulid);

-- -------------------------------------------------------- factuality checks ---

CREATE TABLE factuality_checks (
  id            TEXT(26) PRIMARY KEY,                         -- ULID
  claim_ulid    TEXT(26) NOT NULL,
  check_kind    TEXT NOT NULL CHECK (check_kind IN
                  ('self_consistency','atomic_precision','search_augmented')),
  support       REAL NOT NULL CHECK (support >= 0.0 AND support <= 1.0),
  atoms_json    TEXT,
  created_ulid  TEXT(26) NOT NULL
);

CREATE INDEX idx_factuality_claim ON factuality_checks(claim_ulid);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('decision_firewall', 9, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
