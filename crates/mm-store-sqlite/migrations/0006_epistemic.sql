-- Phase 6 — epistemic discipline, persisted.
--
-- The tables here hold *what the being thinks*: typed claims, the evidence that
-- backs them, observations, assumptions, predictions, contradictions, and the
-- dependency edges that make retraction dependency-directed. What the world
-- contains lives in the `/world` graph, not in these tables; the validation
-- barrier decides which written claim may also be mirrored there.
--
-- Three rules shape the schema:
--   * **Bi-temporal.** `valid_from`/`valid_until` is *world* time; `system_from`/
--     `system_to` is *system* time. Every table carries both, so "what did I
--     believe, and when did I believe it" is answerable and a replay can
--     reconstruct any past state by system time.
--   * **Nothing is destroyed.** A transition is a row in
--     `epistemic_transitions`; a superseded claim is closed with `system_to`
--     rather than deleted, so the prior value stays readable as provenance.
--   * **A contradiction is an object.** It is a row with both sides and their
--     evidence, never a silent edit to either claim.
--
-- Times are RFC3339; ids are ULIDs rendered as `TEXT(26)`. The migration number
-- is the phase number (parent plan §4): Phase 6 owns 0006.

PRAGMA journal_mode=WAL;

-- --------------------------------------------------------------------- claims --

CREATE TABLE claims (
  id            TEXT(26) PRIMARY KEY,                   -- ULID
  kind          TEXT NOT NULL,                          -- ClaimKind
  subject       TEXT NOT NULL,
  predicate     TEXT NOT NULL,
  object        TEXT NOT NULL,
  status        TEXT NOT NULL,                          -- EpistemicStatus, UPPERCASE
  confidence    REAL NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  valid_from    TEXT,
  valid_until   TEXT,
  source_ulid   TEXT,
  system_from   TEXT NOT NULL,
  system_to     TEXT,
  created_ulid  TEXT NOT NULL
);

-- ------------------------------------------------------------------- evidence --

CREATE TABLE evidence (
  id            TEXT(26) PRIMARY KEY,
  claim_id      TEXT(26) NOT NULL REFERENCES claims(id),
  kind          TEXT NOT NULL,                          -- EvidenceKind
  source_uri    TEXT,
  span_start    INTEGER,
  span_end      INTEGER,
  content_hash  TEXT NOT NULL,                          -- sha256 hex
  reliability   REAL NOT NULL,
  system_from   TEXT NOT NULL,
  system_to     TEXT,
  created_ulid  TEXT NOT NULL
);

-- --------------------------------------------------------------- observations --

CREATE TABLE observations (
  id             TEXT(26) PRIMARY KEY,
  claim_id       TEXT(26) NOT NULL REFERENCES claims(id),
  authoritative  INTEGER NOT NULL,
  observed_at    TEXT NOT NULL,
  source_ulid    TEXT NOT NULL,
  system_from    TEXT NOT NULL,
  system_to      TEXT,
  created_ulid   TEXT NOT NULL
);

-- --------------------------------------------------------------- assumptions --

CREATE TABLE assumptions (
  id                    TEXT(26) PRIMARY KEY,
  proposition_uri       TEXT NOT NULL,
  status                TEXT NOT NULL,
  confidence            REAL NOT NULL,
  consequence_if_false  TEXT NOT NULL,
  verification_cost     REAL NOT NULL,
  decision_dependence   REAL NOT NULL,
  valid_from            TEXT,
  valid_until           TEXT,
  system_from           TEXT NOT NULL,
  system_to             TEXT,
  created_ulid          TEXT NOT NULL
);

-- --------------------------------------------------------------- predictions --

CREATE TABLE predictions (
  id               TEXT(26) PRIMARY KEY,
  proposition_uri  TEXT NOT NULL,
  probability      REAL NOT NULL,
  horizon_secs     INTEGER NOT NULL,
  conditions_json  TEXT NOT NULL,
  system_from      TEXT NOT NULL,
  system_to        TEXT,
  created_ulid     TEXT NOT NULL,
  outcome_status   TEXT,
  outcome_value    TEXT,
  resolved_at      TEXT
);

-- ------------------------------------------------------------- contradictions --

CREATE TABLE contradictions (
  id            TEXT(26) PRIMARY KEY,
  claim_a       TEXT(26) NOT NULL,
  claim_b       TEXT(26) NOT NULL,
  reason        TEXT NOT NULL,
  evidence_a    TEXT,
  evidence_b    TEXT,
  status        TEXT NOT NULL,
  system_from   TEXT NOT NULL,
  system_to     TEXT,
  created_ulid  TEXT NOT NULL
);

-- -------------------------------------------------------------- dependencies --

CREATE TABLE dependencies (
  id            TEXT(26) PRIMARY KEY,
  consequent    TEXT(26) NOT NULL,
  antecedent    TEXT(26) NOT NULL,
  kind          TEXT NOT NULL,                          -- DepKind
  criticality   REAL NOT NULL,
  system_from   TEXT NOT NULL,
  system_to     TEXT,
  created_ulid  TEXT NOT NULL,
  UNIQUE(consequent, antecedent)
);

-- -------------------------------------------------------- epistemic_transitions --

CREATE TABLE epistemic_transitions (
  id             TEXT(26) PRIMARY KEY,
  subject        TEXT NOT NULL,
  from_status    TEXT,
  to_status      TEXT,
  reason         TEXT NOT NULL,
  evidence_ulid  TEXT,
  prov_activity  TEXT,
  system_from    TEXT NOT NULL,
  system_to      TEXT,
  created_ulid   TEXT NOT NULL
);

-- --------------------------------------------------------------------- indexes --

CREATE INDEX idx_claims_spo   ON claims(subject, predicate, object);
CREATE INDEX idx_deps_ante    ON dependencies(antecedent);
CREATE INDEX idx_claims_asof  ON claims(subject, predicate, system_from, valid_from);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('epistemic', 6, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
