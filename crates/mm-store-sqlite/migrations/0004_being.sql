-- Phase 4 — the persistent being substrate.
--
-- Three state classes with disjoint rules (plan §2): the PERSISTENT self lives
-- here (identity, invariants, core blocks, personality, relationships, goals,
-- commitments, accounts); GENERATED state is a per-episode value and is never
-- written here; VERIFIED state belongs to Phase 10 and is only ever observed, not
-- inferred from a plan.
--
-- Two shapes recur below and are worth stating once:
--   * `*_transitions` are append-only history. A terminal status is never
--     rewritten — a `BEFORE UPDATE` trigger aborts the attempt rather than
--     trusting the caller (plan §4.2).
--   * `resource_ledger` is the audit trail for `resource_accounts`. The account is
--     a projection of the ledger, and the two must reconcile exactly:
--     `sum(delta) == balance`.
--
-- The migration number is the phase number (parent plan §4): Phase 4 owns 0004.

-- ------------------------------------------------------------------ identity ---

CREATE TABLE identity (
    id               TEXT(26) PRIMARY KEY,  -- ULID
    created_ulid     TEXT(26) NOT NULL,
    self_description TEXT NOT NULL,
    lineage_json     TEXT NOT NULL DEFAULT '[]',
    current_version  TEXT NOT NULL,
    schema_version   INTEGER NOT NULL
);

CREATE TABLE invariants (
    id          TEXT(26) PRIMARY KEY,
    identity_id TEXT(26) NOT NULL REFERENCES identity(id),
    code        TEXT NOT NULL UNIQUE,
    assertion   TEXT NOT NULL,
    enforcement TEXT NOT NULL CHECK (enforcement IN ('deterministic', 'symbolic')),
    created_ulid TEXT(26) NOT NULL
);

-- ---------------------------------------------------------------- core blocks ---
-- Letta-style small, always-in-context blocks. `limit_chars` is enforced in code
-- and re-checked here, because an unbounded block is how a prompt silently grows
-- past the window it was designed for.
CREATE TABLE core_blocks (
    id           TEXT(26) PRIMARY KEY,
    identity_id  TEXT(26) NOT NULL REFERENCES identity(id),
    kind         TEXT NOT NULL CHECK (kind IN ('self', 'human', 'task')),
    label        TEXT NOT NULL,
    content      TEXT NOT NULL,
    limit_chars  INTEGER NOT NULL,
    system_from  TEXT NOT NULL,
    system_to    TEXT,
    updated_ulid TEXT(26) NOT NULL
);
CREATE INDEX idx_core_blocks_kind ON core_blocks(kind);

-- ---------------------------------------------------------------- personality ---
-- A small constitution the LLM elaborates, never an enumerated behavior matrix
-- (design §108): values with a weight, dispositions with a baseline, and
-- constraints that say what to avoid rather than how to act.
CREATE TABLE personality_values (
    value_key       TEXT PRIMARY KEY,
    weight          REAL NOT NULL,
    constraint_kind TEXT NOT NULL CHECK (constraint_kind IN ('avoid', 'prefer'))
);

CREATE TABLE personality_dispositions (
    key          TEXT PRIMARY KEY,
    baseline     REAL NOT NULL,
    value        REAL NOT NULL,
    confidence   REAL NOT NULL,
    updated_ulid TEXT(26) NOT NULL
);

CREATE TABLE personality_context_modifiers (
    context_key TEXT PRIMARY KEY,
    deltas_json TEXT NOT NULL
);

-- --------------------------------------------------------------------- affect ---
-- Affect is a control variable, never a fact. The two flags are CHECK-pinned to
-- 0 so a bug in a higher layer cannot promote an impulse into a belief.
CREATE TABLE affect_state (
    identity_id  TEXT(26) PRIMARY KEY REFERENCES identity(id),
    valence      REAL NOT NULL,
    arousal      REAL NOT NULL,
    engagement   REAL NOT NULL,
    warmth       REAL NOT NULL,
    caution      REAL NOT NULL,
    curiosity    REAL NOT NULL,
    energy       REAL NOT NULL,
    updated_ulid TEXT(26) NOT NULL
);

CREATE TABLE affect_impulses (
    id                     TEXT(26) PRIMARY KEY,
    emotion                TEXT NOT NULL,
    intensity              REAL NOT NULL,
    decay                  REAL NOT NULL,
    affects_internal_state INTEGER NOT NULL DEFAULT 0 CHECK (affects_internal_state = 0),
    affects_reasoning      INTEGER NOT NULL DEFAULT 0 CHECK (affects_reasoning = 0),
    created_ulid           TEXT(26) NOT NULL,
    expires_ulid           TEXT(26)
);

CREATE TABLE motivations (
    id           TEXT(26) PRIMARY KEY,
    drive        TEXT NOT NULL UNIQUE,
    strength     REAL NOT NULL,
    updated_ulid TEXT(26) NOT NULL
);

-- ---------------------------------------------------------------- user model ---
-- BDI beliefs about the user. `epistemic_status` is the lattice: a belief may only
-- reach a status its evidence supports, and `OBSERVED` requires an observation.
CREATE TABLE user_beliefs (
    id               TEXT(26) PRIMARY KEY,
    user_id          TEXT(26) NOT NULL,
    proposition      TEXT NOT NULL,
    proposition_hash TEXT NOT NULL,
    epistemic_status TEXT NOT NULL CHECK (epistemic_status IN ('OBSERVED', 'VERIFIED',
        'REPORTED', 'INFERRED', 'ASSUMED', 'HYPOTHETICAL', 'PREDICTED', 'SIMULATED',
        'FICTIONAL', 'UNKNOWN')),
    confidence       REAL NOT NULL,
    evidence_json    TEXT NOT NULL DEFAULT '[]',
    valid_from_ulid  TEXT(26),
    valid_until_ulid TEXT(26),
    created_ulid     TEXT(26) NOT NULL
);
CREATE INDEX idx_user_beliefs_user ON user_beliefs(user_id);
CREATE INDEX idx_user_beliefs_hash ON user_beliefs(proposition_hash);

-- ------------------------------------------------------------- relationships ---
-- Multi-dimensional and event-sourced: `relationships` is the projection,
-- `relationship_events` the deltas. Rebuilding the projection from the events must
-- reproduce the row exactly.
CREATE TABLE relationships (
    id              TEXT(26) PRIMARY KEY,
    user_id         TEXT(26) NOT NULL UNIQUE,
    familiarity     REAL NOT NULL,
    trust           REAL NOT NULL,
    reciprocity     REAL NOT NULL,
    openness        REAL NOT NULL,
    cooperation     REAL NOT NULL,
    reliance        REAL NOT NULL,
    recent_quality  REAL NOT NULL,
    unresolved_json TEXT NOT NULL DEFAULT '[]',
    updated_ulid    TEXT(26) NOT NULL
);

CREATE TABLE relationship_events (
    id              TEXT(26) PRIMARY KEY,
    relationship_id TEXT(26) NOT NULL REFERENCES relationships(id),
    kind            TEXT NOT NULL,
    delta_json      TEXT NOT NULL,
    ref_ulid        TEXT(26),
    created_ulid    TEXT(26) NOT NULL
);
CREATE INDEX idx_relationship_events_rel ON relationship_events(relationship_id);

-- ---------------------------------------------------------------------- goals ---
-- Desire (Goal) and intention (Commitment) are separate objects with separate
-- lifecycles, so a wish is never mistaken for a promise.
CREATE TABLE goals (
    id            TEXT(26) PRIMARY KEY,
    owner_id      TEXT(26) NOT NULL,
    description   TEXT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('active', 'pending', 'blocked',
        'fulfilled', 'abandoned', 'superseded')),
    priority      REAL NOT NULL,
    deadline_ulid TEXT(26),
    parent_id     TEXT(26) REFERENCES goals(id),
    evidence_json TEXT NOT NULL DEFAULT '[]',
    origin        TEXT NOT NULL CHECK (origin IN ('user', 'being', 'inferred')),
    created_ulid  TEXT(26) NOT NULL
);
CREATE INDEX idx_goals_status ON goals(status);

CREATE TABLE goal_transitions (
    id          TEXT(26) PRIMARY KEY,
    goal_id     TEXT(26) NOT NULL REFERENCES goals(id),
    from_status TEXT NOT NULL,
    to_status   TEXT NOT NULL,
    at_ulid     TEXT(26) NOT NULL,
    reason      TEXT NOT NULL
);
CREATE INDEX idx_goal_transitions_goal ON goal_transitions(goal_id);

CREATE TABLE commitments (
    id           TEXT(26) PRIMARY KEY,
    goal_id      TEXT(26) REFERENCES goals(id),
    made_to      TEXT(26) NOT NULL,
    description  TEXT NOT NULL,
    status       TEXT NOT NULL CHECK (status IN ('active', 'fulfilled', 'revoked',
        'superseded')),
    deadline_ulid TEXT(26),
    created_ulid TEXT(26) NOT NULL,
    terminal_ulid TEXT(26)
);
CREATE INDEX idx_commitments_status ON commitments(status);

CREATE TABLE commitment_transitions (
    id            TEXT(26) PRIMARY KEY,
    commitment_id TEXT(26) NOT NULL REFERENCES commitments(id),
    from_status   TEXT NOT NULL,
    to_status     TEXT NOT NULL,
    at_ulid       TEXT(26) NOT NULL,
    reason        TEXT NOT NULL
);
CREATE INDEX idx_commitment_transitions_c ON commitment_transitions(commitment_id);

-- A terminal status is history. The transition tables record how it was reached;
-- rewriting the terminal row itself would erase that history, so it is aborted.
CREATE TRIGGER goals_terminal_is_immutable
BEFORE UPDATE OF status ON goals
WHEN OLD.status IN ('fulfilled', 'abandoned', 'superseded') AND NEW.status <> OLD.status
BEGIN
    SELECT RAISE(ABORT, 'a terminal goal is never rewritten');
END;

CREATE TRIGGER commitments_terminal_is_immutable
BEFORE UPDATE OF status ON commitments
WHEN OLD.status IN ('fulfilled', 'revoked', 'superseded') AND NEW.status <> OLD.status
BEGIN
    SELECT RAISE(ABORT, 'a terminal commitment is never rewritten');
END;

-- ------------------------------------------------------------------ resources ---
-- Deterministic arithmetic only. The LLM proposes a spend; this ledger is what
-- decides whether it was allowed.
CREATE TABLE resource_accounts (
    kind         TEXT PRIMARY KEY,
    balance      REAL NOT NULL,
    unit         TEXT NOT NULL,
    updated_ulid TEXT(26) NOT NULL
);

CREATE TABLE resource_ledger (
    id            TEXT(26) PRIMARY KEY,
    kind          TEXT NOT NULL,
    delta         REAL NOT NULL,
    balance_after REAL NOT NULL,
    purpose       TEXT NOT NULL,
    ref_ulid      TEXT(26),
    created_ulid  TEXT(26) NOT NULL
);
CREATE INDEX idx_resource_ledger_kind ON resource_ledger(kind);

CREATE TABLE budget_policies (
    kind         TEXT PRIMARY KEY,
    period       TEXT NOT NULL,
    limit_amount REAL NOT NULL,
    hard         INTEGER NOT NULL DEFAULT 1
);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('being', 4, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'));
