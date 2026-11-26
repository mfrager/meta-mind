-- Phase 5 — the persistent memory organ.
--
-- One store, three views over it (plan §2): `core` is the bounded always-in-
-- context set, `recall` is everything hybrid search can reach, and `archival` is
-- low-retention state kept for provenance. Tiers are a column, not a second
-- table, because a memory that moved between tiers must not become a different
-- memory.
--
-- Three rules shape the schema:
--   * **Bi-temporal** (Graphiti). `valid_from`/`valid_until` is *world* time;
--     `recorded_at` is *system* time. They are independent, so "what did I know,
--     and when did I know it" is answerable.
--   * **Append-only, never destructive.** Forgetting archives; it never deletes.
--     `protected` marks the developmental ledger Phase 4 forbids rewriting.
--   * **Provenance is a first-class column.** A memory without a recorded
--     provenance node cannot be written at all.
--
-- Times are integer nanoseconds (Phase 1 `Timestamp`). The DDL in the plan writes
-- `references events(ulid)`; the kernel's event table keys on `id`, and Phase 5
-- writes memories through the store without necessarily appending an event first,
-- so `recorded_ulid` is a plain ULID column here rather than a foreign key.
--
-- The migration number is the phase number (parent plan §4): Phase 5 owns 0005.

PRAGMA journal_mode=WAL;

-- -------------------------------------------------------------------- memories --

CREATE TABLE memories (
  id            TEXT(26) PRIMARY KEY,                   -- ULID
  kind          TEXT NOT NULL CHECK (kind IN
                  ('episodic','semantic','procedural','working','autobiographical',
                   'relational','prediction','mistake','near_miss','developmental')),
  tier          TEXT NOT NULL DEFAULT 'recall' CHECK (tier IN ('core','recall','archival')),
  content       TEXT NOT NULL,
  source_ulid   TEXT(26),
  confidence    REAL NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  importance    REAL NOT NULL CHECK (importance BETWEEN 0 AND 1),
  utility       REAL,                                  -- last computed
  retention     REAL,                                  -- last computed retention_score
  valid_from    INTEGER NOT NULL,                      -- world time, ns
  valid_until   INTEGER,                               -- NULL = open
  recorded_at   INTEGER NOT NULL,                      -- system time, ns
  recorded_ulid TEXT(26) NOT NULL,                     -- the operation that recorded it
  provenance    TEXT(26) NOT NULL,                     -- PROV node ULID
  protected     INTEGER NOT NULL DEFAULT 0 CHECK (protected IN (0, 1)),
  status        TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','archived')),
  content_hash  TEXT NOT NULL                          -- sha256(canonical content)
);

-- --------------------------------------------------------------- cues & entities --

CREATE TABLE memory_cues (
  id        TEXT(26) PRIMARY KEY,
  memory_id TEXT(26) NOT NULL REFERENCES memories(id),
  cue       TEXT NOT NULL,
  cue_kind  TEXT NOT NULL CHECK (cue_kind IN ('keyword','embedding','entity'))
);
CREATE INDEX idx_memory_cues_memory ON memory_cues(memory_id);

CREATE TABLE memory_entities (
  id          TEXT(26) PRIMARY KEY,
  memory_id   TEXT(26) NOT NULL REFERENCES memories(id),
  entity_ulid TEXT(26) NOT NULL,
  role        TEXT
);
CREATE INDEX idx_memory_entities_memory ON memory_entities(memory_id);
CREATE INDEX idx_memory_entities_entity ON memory_entities(entity_ulid);

-- --------------------------------------------------------------------- links ----

-- A-MEM's dynamic links: written on write, reinforced on access. `relation` is a
-- closed vocabulary so a link always says *how* two memories are related.
CREATE TABLE memory_links (
  id           TEXT(26) PRIMARY KEY,
  from_id      TEXT(26) NOT NULL,
  to_id        TEXT(26) NOT NULL,
  relation     TEXT NOT NULL,
  weight       REAL NOT NULL,
  created_ulid TEXT(26) NOT NULL
);
CREATE INDEX idx_memory_links_from ON memory_links(from_id, relation);
CREATE INDEX idx_memory_links_to ON memory_links(to_id, relation);

-- -------------------------------------------------------------------- access ----

CREATE TABLE memory_access (
  id          TEXT(26) PRIMARY KEY,
  memory_id   TEXT(26) NOT NULL REFERENCES memories(id),
  accessed_at INTEGER NOT NULL,
  query_hash  TEXT NOT NULL,
  score       REAL NOT NULL,
  trace_id    TEXT(26) NOT NULL
);
CREATE INDEX idx_memory_access_memory ON memory_access(memory_id);

-- ------------------------------------------------------- consolidation & trees --

CREATE TABLE memory_consolidations (
  id           TEXT(26) PRIMARY KEY,
  source_ids   TEXT NOT NULL,                          -- JSON array of ULIDs
  target_id    TEXT(26) NOT NULL,
  method       TEXT NOT NULL,
  created_ulid TEXT(26) NOT NULL
);
CREATE INDEX idx_memory_consolidations_target ON memory_consolidations(target_id);

-- The RAPTOR summary tree. A node's `parent_id` is the next level up, and the
-- root is the node with no parent.
CREATE TABLE memory_summaries (
  id           TEXT(26) PRIMARY KEY,
  memory_id    TEXT(26) NOT NULL,
  parent_id    TEXT(26),
  level        INTEGER NOT NULL,
  member_ids   TEXT NOT NULL,                          -- JSON array of ULIDs
  created_ulid TEXT(26) NOT NULL
);
CREATE INDEX idx_memory_summaries_memory ON memory_summaries(memory_id);
CREATE INDEX idx_memory_summaries_parent ON memory_summaries(parent_id);

CREATE TABLE memory_communities (
  id           TEXT(26) PRIMARY KEY,
  memory_id    TEXT(26) NOT NULL,
  label        TEXT NOT NULL,
  member_ids   TEXT NOT NULL,                          -- JSON array of ULIDs
  modularity   REAL,
  created_ulid TEXT(26) NOT NULL
);

-- --------------------------------------------------------------- entity graph ----

-- Phase 5 OWNS this table (INDEX.md D4): Phase 6 references it and must not
-- duplicate it. Bi-temporal, so an edge that stopped holding is closed rather
-- than deleted.
CREATE TABLE entity_edges (
  id          TEXT(26) PRIMARY KEY,
  from_entity TEXT(26) NOT NULL,
  to_entity   TEXT(26) NOT NULL,
  relation    TEXT NOT NULL,
  weight      REAL NOT NULL,
  valid_from  INTEGER NOT NULL,
  valid_until INTEGER
);

-- ------------------------------------------------------------------- mistakes ----

-- Phase 11's `regression_tests.mistake_ulid` targets this table's `id` column
-- (INDEX.md D5).
CREATE TABLE mistakes (
  id                  TEXT(26) PRIMARY KEY,
  memory_id           TEXT(26) NOT NULL,
  situation_ulid      TEXT(26),
  decision_ulid       TEXT(26),
  outcome_ulid        TEXT(26),
  failure_mode        TEXT NOT NULL,
  root_cause_ulid     TEXT(26),
  missed_signal       TEXT NOT NULL DEFAULT '[]',      -- JSON array
  corrective_rule_ulid TEXT(26),
  recurrence_risk     REAL NOT NULL CHECK (recurrence_risk BETWEEN 0 AND 1),
  created_ulid        TEXT(26) NOT NULL
);
CREATE INDEX idx_mistakes_memory ON mistakes(memory_id);

-- ------------------------------------------------------------------ fts index ----

-- Lexical retrieval is primary (plan §9): embeddings only rank and expand. The
-- external-content FTS5 table keeps `memories.content` searchable without a
-- second copy, and the triggers keep it in step with every write.
CREATE VIRTUAL TABLE memory_fts USING fts5(content, content='memories', content_rowid='rowid');

CREATE TRIGGER memories_fts_insert AFTER INSERT ON memories BEGIN
  INSERT INTO memory_fts(rowid, content) VALUES (new.rowid, new.content);
END;

CREATE TRIGGER memories_fts_delete AFTER DELETE ON memories BEGIN
  INSERT INTO memory_fts(memory_fts, rowid, content) VALUES ('delete', old.rowid, old.content);
END;

CREATE TRIGGER memories_fts_update AFTER UPDATE ON memories BEGIN
  INSERT INTO memory_fts(memory_fts, rowid, content) VALUES ('delete', old.rowid, old.content);
  INSERT INTO memory_fts(rowid, content) VALUES (new.rowid, new.content);
END;

-- --------------------------------------------------------------------- indexes ---

CREATE INDEX idx_memories_kind   ON memories(kind, status);
CREATE INDEX idx_memories_tier   ON memories(tier, status);
CREATE INDEX idx_memories_valid  ON memories(valid_from, valid_until);
CREATE INDEX idx_memories_hash   ON memories(content_hash);
CREATE INDEX idx_entity_edges    ON entity_edges(from_entity, to_entity);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('memory', 5, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
