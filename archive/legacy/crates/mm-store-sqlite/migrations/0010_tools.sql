-- Phase 10 — tool execution, verification, and the external world.
--
-- Eight tables, and each one answers a question the pipeline
-- (resolve → authorize → tier → snapshot → invoke → observe → ledger) actually
-- asks:
--
--   * `tool_registry`      — what is installed, with which contract, in which tier.
--   * `policy_sets`        — the versioned, reviewable rules a decision was made
--                            against. Data, not code, so the exact rule set that
--                            permitted a call is recoverable after the fact.
--   * `permission_grants`  — who may exercise which capability through which tool.
--   * `tool_calls`         — one row per attempt: the decision, the tier, the status,
--                            the idempotency key and the trace it belongs to.
--   * `idempotency_keys`   — the exactly-once guard. `in_flight` is a claim; `done`
--                            carries the recorded outcome a retry replays.
--   * `action_ledger`      — the append-only, hash-chained journal. The chain is the
--                            source of truth for "this happened", and `verify_chain`
--                            is what makes tampering detectable rather than merely
--                            discouraged.
--   * `observed_payloads`  — one execution-sourced observation per action, with the
--                            Phase 6 evidence id it stands on.
--   * `rollback_snapshots` — what a reversible action may be restored to, and the
--                            hash that proves the restore was byte-identical.
--
-- Six rules shape the schema:
--
--   * **The ledger is append-only in the database, not just in the code.** Triggers
--     refuse UPDATE and DELETE on `action_ledger`, the same way 0001 protects
--     `audit_log`. A hash chain that a later script may edit is a hash chain that
--     proves nothing; the trigger is what makes the earlier rows immutable even for
--     a writer that never goes through `mm-tools`.
--   * **A grant names a tool.** `permission_grants.tool_pattern` is `NOT NULL`, so a
--     grant is a statement about a principal acting *through a tool*. A row here
--     cannot be read as "may write anywhere": it must also match the tool the call
--     resolved to, which is what stops a grant written for `fs.*` from authorizing
--     `process.exec`.
--   * **Default deny is visible as absence.** No `process` and no `net` grant is
--     seeded below. That absence is the whole of the reason `process.exec` and
--     `http.fetch` are refused on a fresh kernel: not a rule that says no, but the
--     lack of a permit, which is the model the plan asks for.
--   * **The policy seed contains a real precedence example.** The baseline set has
--     both a `permit` for `data/sandbox/**` and a `forbid` for
--     `data/sandbox/forbidden/**`, so "explicit forbid beats a permit" is a property
--     of the shipped data and not only of a unit test.
--   * **Bi-temporal, like every other phase.** `tool_registry` carries
--     `valid_from`/`valid_to` (contract time) and `system_from`/`system_to` (system
--     time), so "which version of this tool's contract was active when that call
--     ran" is answerable.
--   * **Nothing that can be undone is stored without the bytes to undo it with.**
--     `rollback_snapshots.content` is a `BLOB`. The plan's sketch had only `ref` and
--     `content_hash`, which can *detect* a difference but cannot restore anything —
--     and "restore to `content_hash`" is the requirement. The hash stays, and is what
--     the restore is checked against.
--
-- Two naming deviations from the plan's §4.3 sketch, both recorded here for the same
-- reason 0009 records its own:
--
--   * The plan calls the observation table `observations`. Migration 0006 already
--     owns `observations` with a different, *claim*-scoped shape
--     (`claim_id`, `authoritative`, `source_ulid`). An action-sourced observation is
--     not a claim-scoped one, and reusing the name would either force a schema change
--     to Phase 6's table or silently mix two meanings in one column set. The table is
--     therefore `observed_payloads`.
--   * `rollback_snapshots` gains `content`, as explained above.
--
-- Times are RFC3339; ids are ULIDs rendered as `TEXT(26)`. The migration number is
-- the phase number (parent plan §4): Phase 10 owns 0010.

PRAGMA journal_mode=WAL;

-- --------------------------------------------------------------- tool registry --

CREATE TABLE tool_registry (
  id              TEXT(26) PRIMARY KEY,                       -- ULID
  name            TEXT NOT NULL UNIQUE,                       -- ToolName, `namespace.verb`
  version         TEXT NOT NULL,
  module_uri      TEXT NOT NULL,
  spec_json       TEXT NOT NULL,                              -- the whole ToolSpec
  sandbox_tier    TEXT NOT NULL CHECK (sandbox_tier IN ('WasmCaps','Gvisor','MicroVm')),
  reversibility   TEXT NOT NULL CHECK (reversibility IN
                    ('reversible','compensatable','irreversible')),
  active          INTEGER NOT NULL DEFAULT 1,
  registered_ulid TEXT NOT NULL,
  system_from     TEXT NOT NULL,
  system_to       TEXT,
  valid_from      TEXT NOT NULL,
  valid_to        TEXT
);

-- ------------------------------------------------------------------ policy sets --

CREATE TABLE policy_sets (
  id            TEXT(26) PRIMARY KEY,                         -- ULID
  name          TEXT NOT NULL,
  version       INTEGER NOT NULL,
  rules_json    TEXT NOT NULL,                                -- [PolicyRule]
  created_ulid  TEXT(26) NOT NULL,
  system_from   TEXT NOT NULL,
  UNIQUE(name, version)
);

-- --------------------------------------------------------------------- grants ---

CREATE TABLE permission_grants (
  id             TEXT(26) PRIMARY KEY,                        -- ULID
  principal      TEXT NOT NULL,                               -- being ULID | user ULID | system | *
  scope          TEXT NOT NULL,                               -- kind:verb:pattern
  tool_pattern   TEXT NOT NULL,                               -- `namespace.*` or an exact name
  policy_set_id  TEXT REFERENCES policy_sets(id),
  granted_ulid   TEXT(26) NOT NULL,
  granted_by     TEXT NOT NULL,
  granted_at     TEXT NOT NULL,
  expires_at     TEXT,
  revoked_at     TEXT
);

CREATE INDEX idx_grants_principal ON permission_grants(principal, tool_pattern);

-- ------------------------------------------------------------------ tool calls --

CREATE TABLE tool_calls (
  id                  TEXT(26) PRIMARY KEY,                   -- ULID
  tool_name           TEXT NOT NULL,
  args_hash           TEXT NOT NULL,                          -- sha256 of canonical args
  args_json           TEXT NOT NULL,                          -- redacted by mm-log
  permission_decision TEXT NOT NULL CHECK (permission_decision IN ('allow','deny','escalate')),
  sandbox_tier        TEXT NOT NULL,
  status              TEXT NOT NULL CHECK (status IN ('ok','error','denied','timeout','deduplicated')),
  idempotency_key     TEXT,
  trace_id            TEXT(26) NOT NULL,
  started_at          TEXT NOT NULL,
  ended_at            TEXT,
  latency_ms          INTEGER
);

CREATE INDEX idx_tool_calls_tool ON tool_calls(tool_name, started_at);
CREATE INDEX idx_tool_calls_trace ON tool_calls(trace_id);

-- -------------------------------------------------------------- idempotency keys -

CREATE TABLE idempotency_keys (
  key         TEXT PRIMARY KEY,
  action_id   TEXT(26) NOT NULL,
  state       TEXT NOT NULL CHECK (state IN ('in_flight','done')),
  result_json TEXT,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

-- --------------------------------------------------------------- action ledger --

CREATE TABLE action_ledger (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  id           TEXT(26) NOT NULL UNIQUE,
  action_id    TEXT(26) NOT NULL,
  event        TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  prev_hash    TEXT NOT NULL,                                 -- genesis = 64 zeros
  entry_hash   TEXT NOT NULL,                                 -- sha256(prev || id || event || payload)
  at           TEXT NOT NULL
);

CREATE INDEX idx_ledger_action ON action_ledger(action_id);

-- Append-only, enforced by the database. Same shape as the `audit_log` guards in
-- 0001_kernel.sql: an out-of-band UPDATE or DELETE aborts rather than quietly
-- rewriting history that `verify_chain` would then report as intact.
CREATE TRIGGER action_ledger_no_update
BEFORE UPDATE ON action_ledger
BEGIN
  SELECT RAISE(ABORT, 'action_ledger is append-only');
END;

CREATE TRIGGER action_ledger_no_delete
BEFORE DELETE ON action_ledger
BEGIN
  SELECT RAISE(ABORT, 'action_ledger is append-only');
END;

-- ----------------------------------------------------------- observed payloads --

CREATE TABLE observed_payloads (
  id           TEXT(26) PRIMARY KEY,                          -- ULID
  action_id    TEXT(26) NOT NULL,
  source       TEXT NOT NULL,                                 -- kind:endpoint
  payload_json TEXT NOT NULL,
  evidence_id  TEXT(26) NOT NULL,                             -- Phase 6 Evidence ULID
  observed_at  TEXT NOT NULL,
  graph_iri    TEXT NOT NULL                                  -- the /tools node
);

CREATE INDEX idx_observed_action ON observed_payloads(action_id);

-- -------------------------------------------------------- rollback snapshots ----

CREATE TABLE rollback_snapshots (
  id           TEXT(26) PRIMARY KEY,                          -- ULID
  action_id    TEXT(26) NOT NULL,
  kind         TEXT NOT NULL CHECK (kind IN ('fs_path','sql_row','graph_quad','none')),
  ref          TEXT NOT NULL,                                 -- what was snapshotted
  content      BLOB,                                          -- the bytes, for a byte-identical restore
  content_hash TEXT NOT NULL,                                 -- sha256 hex, the restore is checked against it
  created_at   TEXT NOT NULL
);

CREATE INDEX idx_snapshots_action ON rollback_snapshots(action_id);

-- ------------------------------------------------------------------ seed data ---
--
-- The baseline policy set and the capabilities the kernel principal holds. Both are
-- *data*: `mm-cli policy list` prints them, and `mm-cli policy check` evaluates the
-- same rows the executor does. `system_from` is a fixed instant rather than `now()`
-- so a fresh database has byte-identical seed rows on every machine, which is what
-- makes a replayed episode's decisions reproducible.

INSERT INTO policy_sets (id, name, version, rules_json, created_ulid, system_from)
VALUES (
  '01h0000000000000000000p010',
  'baseline',
  1,
  '[{"effect":"permit","principal":"*","action":"fs.write","resource":"data/sandbox/**"},{"effect":"forbid","principal":"*","action":"fs.write","resource":"data/sandbox/forbidden/**"}]',
  '01h0000000000000000000p011',
  '2026-10-08T00:00:00.000000000Z'
);

INSERT INTO permission_grants
  (id, principal, scope, tool_pattern, policy_set_id, granted_ulid, granted_by, granted_at, expires_at, revoked_at)
VALUES
  ('01h0000000000000000000g001', 'system', 'fs:read:data/sandbox/**',  'fs.*',
   '01h0000000000000000000p010', '01h0000000000000000000p011', 'operator',
   '2026-10-08T00:00:00.000000000Z', NULL, NULL),
  ('01h0000000000000000000g002', 'system', 'fs:write:data/sandbox/**', 'fs.*',
   '01h0000000000000000000p010', '01h0000000000000000000p011', 'operator',
   '2026-10-08T00:00:00.000000000Z', NULL, NULL),
  ('01h0000000000000000000g003', 'system', 'graph:read:**',            'graph.*',
   '01h0000000000000000000p010', '01h0000000000000000000p011', 'operator',
   '2026-10-08T00:00:00.000000000Z', NULL, NULL),
  -- The one host `http.fetch` declares, granted exactly as the tool declares it. Without
  -- it the network case is refused for holding no net capability at all, and the *host*
  -- check — the reason an allowlist exists — could never be the thing that refused a
  -- foreign URL. The two rows above are the same shape: the declaration, granted as
  -- written.
  ('01h0000000000000000000g004', 'system', 'net:connect:api.metamind.dev', 'http.*',
   '01h0000000000000000000p010', '01h0000000000000000000p011', 'operator',
   '2026-10-08T00:00:00.000000000Z', NULL, NULL);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('tools', 10, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
