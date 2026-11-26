-- Phase 2 — code-metadata index.
--
-- The `/code` RDF graph is the authority for what the codebase is; these tables
-- are a mirror that makes `codex meta`, churn, and incremental scans fast. Every
-- row is derived, so the tables can be dropped and rebuilt by `codex scan`.
--
-- The migration number is the phase number (parent plan §4): Phase 2 owns 0002.

-- ---------------------------------------------------------------- modules ----
CREATE TABLE module_index (
    id              TEXT(26) PRIMARY KEY,
    module_uri      TEXT NOT NULL UNIQUE,   -- stable path-derived module IRI
    rel_path        TEXT NOT NULL UNIQUE,
    version         TEXT NOT NULL,
    category        TEXT,
    crate_name      TEXT,
    plugin_type     TEXT,                   -- rust_crate | nexus_plugin
    owned_phase     INTEGER NOT NULL,
    content_hash    TEXT NOT NULL,          -- canonical hash of the module's file set
    copied_from     TEXT,                   -- set only for copied-in code
    first_seen_ulid TEXT(26) NOT NULL,
    last_seen_ulid  TEXT(26) NOT NULL
);

-- ------------------------------------------------------------------ files ----
CREATE TABLE module_file (
    id           TEXT(26) PRIMARY KEY,
    module_uri   TEXT NOT NULL REFERENCES module_index(module_uri),
    rel_path     TEXT NOT NULL UNIQUE,       -- a file belongs to exactly one module
    content_hash TEXT NOT NULL,              -- sha256 of the bytes
    swhid        TEXT,
    lang         TEXT NOT NULL
);

CREATE INDEX module_file_module ON module_file(module_uri);

-- ---------------------------------------------------------------- symbols ----
CREATE TABLE symbol_index (
    id           TEXT(26) PRIMARY KEY,
    symbol_uri   TEXT NOT NULL UNIQUE,
    descriptor   TEXT NOT NULL,              -- SCIP-style, scope+item+kind
    kind         TEXT NOT NULL,              -- interface | trait | fn | type | const | module
    module_uri   TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    byte_start   INTEGER NOT NULL,
    byte_end     INTEGER NOT NULL,
    is_public    INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    UNIQUE(descriptor, module_uri)
);

CREATE INDEX symbol_index_module ON symbol_index(module_uri);
CREATE INDEX symbol_index_file ON symbol_index(file_path);

-- ------------------------------------------------------------- references ----
CREATE TABLE symbol_reference (
    from_symbol  TEXT NOT NULL,
    to_symbol    TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    byte_start   INTEGER NOT NULL,
    PRIMARY KEY(from_symbol, to_symbol, file_path, byte_start)
);

-- ------------------------------------------------------------ capabilities ---
CREATE TABLE capability_index (
    id         TEXT(26) PRIMARY KEY,
    capability TEXT NOT NULL,
    module_uri TEXT NOT NULL,
    test_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE(capability, module_uri)
);

-- ------------------------------------------------------------ dependencies ---
CREATE TABLE module_dep (
    from_uri TEXT NOT NULL,
    to_uri   TEXT NOT NULL,
    kind     TEXT NOT NULL,                  -- crate | module | reference
    PRIMARY KEY(from_uri, to_uri, kind)
);

CREATE INDEX module_dep_to ON module_dep(to_uri);

-- ------------------------------------------------------------------ runs -----
CREATE TABLE codex_run (
    id            TEXT(26) PRIMARY KEY,      -- the scan's ULID, shared with mm-log
    started_at    TEXT NOT NULL,
    modules       INTEGER NOT NULL,
    files         INTEGER NOT NULL,
    symbols       INTEGER NOT NULL,
    violations    INTEGER NOT NULL,
    graph_hash    TEXT NOT NULL,
    changed_files INTEGER NOT NULL
);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('codex', 2, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
