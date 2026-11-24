-- The model registry: an inspectable mirror of `config/llm.toml`.
--
-- Routing remains configuration-authoritative; `mm-serve` rewrites this table
-- from the loaded `LlmConfig` on boot. Keeping the registry in SQLite makes the
-- currently configured catalog queryable (and visible to the inspector) without
-- duplicating the routing decision in the database.
--
-- `synced_at` is when the boot snapshot was taken, not when the model changed.

PRAGMA journal_mode=WAL;

CREATE TABLE IF NOT EXISTS llm_models (
  name                          TEXT PRIMARY KEY,
  input_cost_micros_per_1k      INTEGER NOT NULL CHECK (input_cost_micros_per_1k >= 0),
  output_cost_micros_per_1k     INTEGER NOT NULL CHECK (output_cost_micros_per_1k >= 0),
  typical_latency_ms            INTEGER NOT NULL CHECK (typical_latency_ms >= 0),
  complexity_ceiling            TEXT NOT NULL
                                CHECK (complexity_ceiling IN ('low', 'medium', 'high', 'very_high')),
  mock                          INTEGER NOT NULL DEFAULT 0 CHECK (mock IN (0, 1)),
  synced_at                     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_llm_models_complexity
  ON llm_models (complexity_ceiling, input_cost_micros_per_1k);

INSERT INTO schema_versions (name, version, applied_at)
VALUES ('llm_models', 14, strftime('%Y-%m-%dT%H:%M:%f000000Z', 'now'))
ON CONFLICT(name) DO NOTHING;
