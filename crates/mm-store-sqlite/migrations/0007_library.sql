-- Phase 7 — the cognitive library, indexed.
--
-- The `/library` graph is the authority for *what an entry says*; these tables are
-- the index the operator surface queries — kind, slug, version, activation score,
-- verification state, fitness. A projection, not a second authority: every write
-- goes through `mm-library`, which validates the Turtle first and writes the graph
-- and these rows in the same operation.
--
-- Four rules shape the schema:
--   * **Immutable versions.** There is no UPDATE path for an entry's body or a
--     policy's behaviour. A new version is a new row; `parent_version` records
--     what it descends from. `version_iri` and `(iri, version)` are unique, so a
--     re-insert is refused rather than overwriting.
--   * **One entry per content.** `content_hash` is indexed so a duplicate body is
--     detected by hash, not by title.
--   * **Verification is a state, not a flag.** A skill is `draft` until its test
--     passes; `verified_at` records when it last did, and a later failure moves it
--     to `failing` without deleting the record.
--   * **Fitness appends.** `policy_fitness` is one row per (policy, version) that
--     is updated in place only through `policy.fitness.update`, which appends an
--     audit record first.
--
-- Times are RFC3339; ids are ULIDs rendered as `TEXT(26)`. The migration number is
-- the phase number (parent plan §4): Phase 7 owns 0007.

PRAGMA journal_mode=WAL;

-- ----------------------------------------------------------------- entries ----

CREATE TABLE library_entries (
  id               TEXT(26) PRIMARY KEY,                       -- ULID
  iri              TEXT NOT NULL,                              -- head IRI
  version_iri      TEXT NOT NULL UNIQUE,                        -- iri@version
  kind             TEXT NOT NULL CHECK (kind IN ('Doctrine','Principle','Heuristic',
                                                 'Technique','Pattern','Case','AntiPattern',
                                                 'Skill','Policy','Frame','FrameInstance',
                                                 'Evaluation','Insight','Workflow')),
  slug             TEXT NOT NULL,
  version          INTEGER NOT NULL DEFAULT 1,
  title            TEXT NOT NULL,
  body_ttl         TEXT NOT NULL,                              -- canonical Turtle
  content_hash     TEXT NOT NULL,                              -- sha256 of body_ttl
  activation_score REAL NOT NULL DEFAULT 0.0,                  -- ACT-R-style blend
  embedding_ref    TEXT,
  system_from      TEXT NOT NULL,
  system_to        TEXT,
  created_ulid     TEXT NOT NULL,
  UNIQUE (iri, version)
);
CREATE INDEX idx_library_kind      ON library_entries(kind, slug);
CREATE INDEX idx_library_hash      ON library_entries(content_hash);
CREATE INDEX idx_library_activation ON library_entries(activation_score);

-- ------------------------------------------------------------------ skills ----

CREATE TABLE skills (
  id            TEXT(26) PRIMARY KEY,
  iri           TEXT NOT NULL,
  name          TEXT NOT NULL,
  description   TEXT NOT NULL,
  impl_ref      TEXT NOT NULL,                                 -- content-addressed artefact
  entrypoint    TEXT NOT NULL,
  signature     TEXT NOT NULL,
  tests_ref     TEXT NOT NULL,
  verification  TEXT NOT NULL CHECK (verification IN ('draft','verified','failing')),
  verified_at   TEXT,
  embedding_ref TEXT,
  version       INTEGER NOT NULL DEFAULT 1,
  created_ulid  TEXT NOT NULL,
  UNIQUE (iri, version)
);

-- --------------------------------------------------------------- workflows ----

CREATE TABLE workflows (
  id            TEXT(26) PRIMARY KEY,
  iri           TEXT NOT NULL,
  name          TEXT NOT NULL,
  steps_json    TEXT NOT NULL,                                 -- induced from a trajectory
  induced_from  TEXT NOT NULL,                                 -- trajectory ULID or ref
  version       INTEGER NOT NULL DEFAULT 1,
  created_ulid  TEXT NOT NULL,
  UNIQUE (iri, version)
);

-- ------------------------------------------------------------------- cases ----

CREATE TABLE cases (
  id                TEXT(26) PRIMARY KEY,
  iri               TEXT UNIQUE NOT NULL,
  kind              TEXT NOT NULL CHECK (kind IN ('success','failure','unusual','edge')),
  problem_ttl       TEXT NOT NULL,
  solution_ttl      TEXT NOT NULL,
  outcome_ttl       TEXT NOT NULL,
  outcome_quality   REAL NOT NULL CHECK (outcome_quality  BETWEEN 0.0 AND 1.0),
  transferability   REAL NOT NULL CHECK (transferability  BETWEEN 0.0 AND 1.0),
  evidence_quality  REAL NOT NULL CHECK (evidence_quality BETWEEN 0.0 AND 1.0),
  embedding_ref     TEXT,
  created_ulid      TEXT NOT NULL
);

-- Structure-mapping correspondences between two cases. Capped per case by the
-- writer: a map that relates everything to everything ranks nothing.
CREATE TABLE case_map (
  id           TEXT(26) PRIMARY KEY,
  case_iri     TEXT NOT NULL,
  source_elem  TEXT NOT NULL,
  target_elem  TEXT NOT NULL,
  relation     TEXT NOT NULL,
  score        REAL NOT NULL,
  created_ulid TEXT NOT NULL
);
CREATE INDEX idx_case_map_case ON case_map(case_iri);

-- ---------------------------------------------------------------- insights ----

CREATE TABLE insights (
  id           TEXT(26) PRIMARY KEY,
  iri          TEXT UNIQUE NOT NULL,
  text         TEXT NOT NULL,
  kind         TEXT NOT NULL CHECK (kind IN ('success_rule','failure_lesson','workflow','preference')),
  evidence     TEXT NOT NULL,                                  -- JSON array of trajectory ids
  upvotes      INTEGER NOT NULL DEFAULT 0,
  downvotes    INTEGER NOT NULL DEFAULT 0,
  created_ulid TEXT NOT NULL
);

-- ---------------------------------------------------------------- policies ----

CREATE TABLE policies (
  id            TEXT(26) PRIMARY KEY,
  iri           TEXT UNIQUE NOT NULL,
  name          TEXT NOT NULL,
  head_version  INTEGER NOT NULL,
  created_ulid  TEXT NOT NULL
);

CREATE TABLE policy_versions (
  id              TEXT(26) PRIMARY KEY,
  policy_iri      TEXT NOT NULL,
  version         INTEGER NOT NULL,
  parent_version  INTEGER,                                     -- NULL only for version 1
  scope_ttl       TEXT NOT NULL,
  activation_ttl  TEXT NOT NULL,
  behavior_ttl    TEXT NOT NULL,                               -- typed DSL fragment
  evidence_ttl    TEXT NOT NULL,
  confidence      REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
  content_hash    TEXT NOT NULL,
  generation      INTEGER NOT NULL DEFAULT 0,
  created_ulid    TEXT NOT NULL,
  UNIQUE (policy_iri, version)
);

CREATE TABLE policy_fitness (
  id            TEXT(26) PRIMARY KEY,
  policy_iri    TEXT NOT NULL,
  version       INTEGER NOT NULL,
  trials        INTEGER NOT NULL,
  successes     INTEGER NOT NULL,
  mean_utility  REAL,
  updated_ulid  TEXT NOT NULL,
  UNIQUE (policy_iri, version)
);

CREATE TABLE policy_population (
  id           TEXT(26) PRIMARY KEY,
  generation   INTEGER NOT NULL,
  policy_iri   TEXT NOT NULL,
  version      INTEGER NOT NULL,
  parent_a     TEXT,
  parent_b     TEXT,
  mutation     TEXT,
  fitness      REAL,
  retained     INTEGER NOT NULL DEFAULT 0,
  created_ulid TEXT NOT NULL
);
CREATE INDEX idx_population_generation ON policy_population(generation);

-- ------------------------------------------------------------------ frames ----

CREATE TABLE frame_instances (
  id           TEXT(26) PRIMARY KEY,
  frame_iri    TEXT NOT NULL,
  episode_ulid TEXT NOT NULL,
  parents_json TEXT NOT NULL,
  slots_json   TEXT NOT NULL,
  missing_json TEXT NOT NULL,
  created_ulid TEXT NOT NULL
);
CREATE INDEX idx_frame_instances_episode ON frame_instances(episode_ulid);

-- ------------------------------------------------------------ applicability ---

CREATE TABLE applicability_runs (
  id           TEXT(26) PRIMARY KEY,
  episode_ulid TEXT NOT NULL,
  state_hash   TEXT NOT NULL,
  ranked_json  TEXT NOT NULL,
  created_ulid TEXT NOT NULL
);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('library', 7, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
