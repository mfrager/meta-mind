-- Phase 1 — kernel schema.
--
-- Connection-level pragmas (journal_mode = WAL, foreign_keys = ON, busy_timeout)
-- are set on every connection by `mm-store-sqlite::pool` rather than here:
-- `PRAGMA journal_mode` cannot run inside the transaction sqlx wraps a migration
-- in. Setting them per-connection also means they cannot drift from the config.

-- ---------------------------------------------------------------- event log --
CREATE TABLE events (
    seq          INTEGER PRIMARY KEY,           -- gapless, assigned by the log writer
    id           TEXT(26) NOT NULL UNIQUE,      -- lowercase Crockford ULID
    kind         TEXT NOT NULL,
    payload      TEXT NOT NULL,                 -- canonical JSON
    status       TEXT NOT NULL CHECK (status IN ('provisional','committed','aborted')),
    correlation  TEXT(26),                      -- trace_id ULID, shared with mm-log
    system_from  TEXT NOT NULL,                 -- commit instant, RFC3339 (system time)
    created_at   TEXT NOT NULL,
    hash         TEXT NOT NULL                  -- sha256(kind|payload|created_at)
);

CREATE INDEX events_status_seq ON events(status, seq);
CREATE UNIQUE INDEX events_hash ON events(hash); -- idempotent append

-- ------------------------------------------------------------------- audit ---
-- Append-only. `prev_hash` is NOT NULL with the empty string as the genesis
-- sentinel, so `audit_log_prev_unique` makes a fork in the chain impossible
-- rather than merely detectable, and permits exactly one genesis record.
CREATE TABLE audit_log (
    seq        INTEGER PRIMARY KEY,             -- gapless, assigned by the writer
    record_id  TEXT(26) NOT NULL UNIQUE,
    event_code TEXT NOT NULL,
    level      TEXT NOT NULL,
    target     TEXT NOT NULL,
    trace_id   TEXT(26),
    payload    TEXT NOT NULL,                   -- canonical JSON
    prev_hash  TEXT NOT NULL,
    hash       TEXT NOT NULL UNIQUE,            -- chained sha256
    at         TEXT NOT NULL                    -- the record's own instant, in the hash
);
CREATE UNIQUE INDEX audit_log_prev_unique ON audit_log(prev_hash);

CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;

-- ---------------------------------------------------------- replay snapshots --
-- The event-sourcing snapshot pattern: the last applied event sequence and the
-- hash of the projected state at that point.
CREATE TABLE store_checkpoints (
    name       TEXT PRIMARY KEY,
    seq        INTEGER NOT NULL,
    state_hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- -------------------------------------------------------------- bitemporal ---
-- The XTDB model, with one clarification: *system* time is the event-log
-- sequence at which the fact became known (deterministic and replayable), and
-- *valid* time is the domain instant the fact is about (RFC3339). A row is
-- visible at `as_of(system, valid)` iff it was recorded at or before `system`
-- and its valid interval covers `valid`.
--
-- Later phases clone this shape for their own domain tables; `facts` exists in
-- Phase 1 so the as-of contract is exercised and gated from the start.
CREATE TABLE facts (
    id          TEXT(26) PRIMARY KEY,
    subject     TEXT NOT NULL,
    predicate   TEXT NOT NULL,
    object      TEXT NOT NULL,
    system_from INTEGER NOT NULL,
    system_to   INTEGER,
    valid_from  TEXT NOT NULL,
    valid_to    TEXT,
    provenance  TEXT,
    trace_id    TEXT(26)
);
CREATE INDEX facts_asof ON facts(subject, predicate, system_from, valid_from);
CREATE INDEX facts_valid ON facts(valid_from, valid_to);

CREATE TRIGGER facts_no_overlapping_system_time
BEFORE INSERT ON facts
WHEN EXISTS (
    SELECT 1 FROM facts f
    WHERE f.subject = NEW.subject
      AND f.predicate = NEW.predicate
      AND f.object = NEW.object
      AND f.system_to IS NULL
)
BEGIN
    SELECT RAISE(ABORT, 'fact is already current: close it with system_to first');
END;

-- ------------------------------------------------------- forward scaffolding --
-- Kernel bookkeeping for migrations that later phases extend. No phase-2+ domain
-- table is created here.
CREATE TABLE schema_versions (
    name       TEXT PRIMARY KEY,
    version    INTEGER NOT NULL,
    applied_at TEXT NOT NULL
);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('kernel', 1, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
