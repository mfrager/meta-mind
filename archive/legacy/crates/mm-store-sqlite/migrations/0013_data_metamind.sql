-- Phase 3 — the data substrate, tabular side.
--
-- RDF is authority for every managed entity; this migration records the named
-- graphs that Phase 3 adds and the durable outbox that bridges an RDF commit to
-- the Tantivy document. The live outbox lives in the data root's own
-- `sqlite/metamind.db` (owned by `mm-data`); this copy exists in the kernel
-- database for the inspector and CLI, with the same shape so a row can be read
-- by either.
--
--   * `data_graphs`   — the graph registry: alias, IRI, and the phase that
--                       introduced it. A new graph requires a row here, which is
--                       what makes "no fact is authoritative in two graphs"
--                       checkable at boot rather than by memory.
--   * `data_outbox`   — one row per (dataRevision, subject) index operation.
--                       `seq` is the entity's dataRevision, so ordering the
--                       queue is ordering the writes. `done = 0` is the retry
--                       set; the worker checks the RDF revision before applying
--                       a row, so a row whose RDF write never landed is
--                       discarded instead of resurrecting an entity.
--
-- Times are Unix milliseconds here because the outbox is a queue, not a ledger:
-- its ordering is `seq`, not a wall clock.

PRAGMA journal_mode=WAL;

-- ------------------------------------------------------------- graph registry --

CREATE TABLE IF NOT EXISTS data_graphs (
  alias       TEXT PRIMARY KEY,
  iri         TEXT NOT NULL UNIQUE,
  phase       INTEGER NOT NULL CHECK (phase >= 1),
  description TEXT NOT NULL
);

INSERT INTO data_graphs (alias, iri, phase, description) VALUES
  ('schema',  'https://metamind.dev/graph/schema',  3, 'schema catalog and namespace registry'),
  ('plan',    'https://metamind.dev/graph/plan',    3, 'plans, plan versions, and thoughts'),
  ('action',  'https://metamind.dev/graph/action',  3, 'scheduled actions and receipts'),
  ('session', 'https://metamind.dev/graph/session', 3, 'sessions, traces, work, frames'),
  ('log',     'https://metamind.dev/graph/log',     3, 'RDF log entries and retention')
ON CONFLICT(alias) DO NOTHING;

-- -------------------------------------------------------------------- outbox --

CREATE TABLE IF NOT EXISTS data_outbox (
  seq             INTEGER PRIMARY KEY,            -- the entity's dataRevision
  op              TEXT NOT NULL CHECK (op IN ('upsert','delete')),
  subject         TEXT NOT NULL,
  graph           TEXT NOT NULL,
  doc_json        TEXT,
  idempotency_key TEXT UNIQUE,
  attempts        INTEGER NOT NULL DEFAULT 0,
  next_attempt_at INTEGER NOT NULL DEFAULT 0,
  created_at      INTEGER NOT NULL,
  done            INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0,1))
);

CREATE INDEX IF NOT EXISTS data_outbox_pending
  ON data_outbox (done, next_attempt_at, seq);

CREATE INDEX IF NOT EXISTS data_outbox_subject
  ON data_outbox (subject);

-- A tombstone row never carries a document: there is nothing to index.
CREATE TRIGGER IF NOT EXISTS data_outbox_tombstone_has_no_doc
BEFORE INSERT ON data_outbox
WHEN NEW.op = 'delete' AND NEW.doc_json IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'a delete outbox row must not carry a document');
END;

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('data_metamind', 13, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
