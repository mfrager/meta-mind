-- Phase 3 — LLM call substrate.
--
-- `llm_calls` is the ledger: a call that is not in it did not happen, and a row
-- that is in it must be complete enough to audit. The remaining tables are the
-- substrates that surround a call — the exact and semantic cache indexes, the
-- repair passes a rejected structured call spent, the routing decisions, and the
-- compiled grammars keyed by schema hash.
--
-- The migration number is the phase number (parent plan §4): Phase 3 owns 0003.

-- ----------------------------------------------------------------- ledger ----
CREATE TABLE llm_calls (
    id            TEXT(26) PRIMARY KEY,     -- ULID
    trace_id      TEXT(26),
    purpose       TEXT NOT NULL,            -- interpret|plan|extract|critique|summarize|code_review|classify|diagnose
    provider      TEXT NOT NULL,
    model         TEXT NOT NULL,
    prompt_hash   TEXT NOT NULL,            -- sha256 over the canonical request
    schema_id     TEXT,
    decoder       TEXT NOT NULL,            -- llguidance|xgrammar|provider_native|none
    schema_ok     INTEGER NOT NULL DEFAULT 1,
    routed_from   TEXT,                     -- the model asked for, when routing changed it
    route_reason  TEXT,
    cached        INTEGER NOT NULL DEFAULT 0,
    cache_layer   TEXT,                     -- exact|semantic|none
    tokens_in     INTEGER NOT NULL DEFAULT 0,
    tokens_out    INTEGER NOT NULL DEFAULT 0,
    cost_micros   INTEGER NOT NULL DEFAULT 0,
    latency_ms    INTEGER NOT NULL DEFAULT 0,
    status        TEXT NOT NULL,            -- ok|schema_rejected|provider_error|timeout|replay
    error_kind    TEXT,
    created_ulid  TEXT(26) NOT NULL,
    created_at    TEXT NOT NULL
);

CREATE INDEX idx_llm_calls_purpose ON llm_calls(purpose);
CREATE INDEX idx_llm_calls_hash    ON llm_calls(prompt_hash);
CREATE INDEX idx_llm_calls_trace   ON llm_calls(trace_id);

-- ---------------------------------------------------------------- caches -----
-- The body lives in a file (a response is a document, not a column); the index
-- records where it is and the hash that proves it was not altered in place.
CREATE TABLE llm_cache_index (
    prompt_hash   TEXT PRIMARY KEY,
    model         TEXT NOT NULL,
    schema_id     TEXT,
    response_path TEXT NOT NULL,
    response_sha  TEXT NOT NULL,
    created_ulid  TEXT(26) NOT NULL
);

CREATE TABLE llm_semantic_cache (
    id            TEXT(26) PRIMARY KEY,
    prompt_hash   TEXT NOT NULL,
    embedding     BLOB NOT NULL,
    response_path TEXT NOT NULL,
    purpose       TEXT NOT NULL,
    similarity    REAL NOT NULL,
    created_ulid  TEXT(26) NOT NULL
);

-- ---------------------------------------------------------------- repairs ----
-- One row per bounded repair pass, so a rejected call can be told from a call
-- that was never checked.
CREATE TABLE llm_repair_attempts (
    id         TEXT(26) PRIMARY KEY,
    call_id    TEXT(26) NOT NULL REFERENCES llm_calls(id),
    attempt_no INTEGER NOT NULL,
    error_kind TEXT NOT NULL,
    accepted   INTEGER NOT NULL DEFAULT 0
);

-- --------------------------------------------------------------- routing -----
CREATE TABLE llm_routing (
    id           TEXT(26) PRIMARY KEY,
    call_id      TEXT(26),
    requested    TEXT,
    selected     TEXT NOT NULL,
    reason       TEXT NOT NULL,
    cost_micros  INTEGER NOT NULL DEFAULT 0,
    created_ulid TEXT(26) NOT NULL
);

-- -------------------------------------------------------------- grammars -----
-- A schema maps 1:1 to a compiled grammar, cached by schema hash *and* decoder,
-- so a schema edit invalidates the cache by hash rather than by trust.
CREATE TABLE llm_grammars (
    schema_id    TEXT PRIMARY KEY,
    schema_sha   TEXT NOT NULL,
    grammar      TEXT NOT NULL,
    decoder      TEXT NOT NULL,
    created_ulid TEXT(26) NOT NULL
);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('llm', 3, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
