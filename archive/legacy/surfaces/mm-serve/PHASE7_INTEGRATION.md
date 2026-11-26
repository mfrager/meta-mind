# Phase 7: `mm-serve` integration

Phase 7 puts the conversation engine behind the console's HTTP surface while
keeping the legacy controller available for comparison.

## Boot

`AppState::boot` now:

1. Opens the kernel's existing SQLite, event log and GraphStore exactly as before.
2. Calls `mm_data::initialize_data_layer` under `cfg.store.data_dir`.
3. Shares the resulting `MemorySystem`, `RdfEngine` and `AgentstreamStore` on
   `AppState`.
4. Creates one `CmeRuntime`; it starts a `SessionDriver` lazily for each chat
   session, so several conversations can run at once.

The existing GraphStore (`data/graph`) and the data layer's engine
(`data/cubecast/main`) are separate during the migration window. They do not
share a RocksDB instance, which keeps the legacy console routes working until
they are removed.

## Routes

| Route | Purpose |
|---|---|
| `POST /api/cme/chat` | Run one turn through CME, returning SSE frames |
| `GET /api/cme/sessions/{id}/snapshot` | Full session/turn/region snapshot |
| `GET /api/cme/sessions/{id}/events` | SSE stream of `sm_transition` events |
| `GET /api/cme/sessions/{id}/metrics` | Prometheus text for that session |
| `GET /debug/cme` | Server-rendered operator view of live CME sessions |
| `POST /api/chat` | Delegates to `POST /api/cme/chat` when CME is enabled |

The event route uses SSE, matching every other console stream, rather than a
separate WebSocket transport.

## Feature flags

Flags are read from the environment at `AppState::boot`. Tests call
`AppState::boot_with_flags` instead.

| Variable | Default | Effect |
|---|---|---|
| `MM_CME_ENABLED` | `false` | Send `/api/chat` through CME |
| `MM_CME_PROACTIVE` | `true` | Attach the six built-in proactive plugins |
| `MM_CME_VOICE_TIER` | unset | Pre-negotiate `tier1`, `tier2` or `tier3` |
| `MM_CME_MEMORY_COMPRESSION` | `true` | Compress completed turns |
| `MM_CME_PREFERENCE_LEARNING` | `true` | Reserved for the preference-learning adapter |
| `MM_CME_IMPROVEMENT` | `false` | Schedule the report/dashboard improvement loop |
| `MM_CME_AGENTSTREAM_LOGGING` | `true` | Log user and frame events to agentstream |

The default keeps the legacy controller on `/api/chat`; opting in is a single
environment variable and does not require a rebuild.

## Tools

CME uses the controller's catalog as its only tool inventory. Every tool is
advertised to the model with the same name, summary and JSON schema, and a
provider tool call runs through `controller::exec::run` after admission checks
its group, schema and read-only graph contract. The same firewall, loop-goal
and module-load guards run on both paths.

The executor alternates model calls and tool results, up to four model rounds
and eight tool steps per turn. Each result is appended to the model transcript,
so the model can answer from what the tool returned. The completed turn retains
its tool calls (`ToolCallEmission`) and the SSE route emits `tool_call` and
`tool_result` frames; a tool's `focus` result also emits the pane's `focus`
frame. Agentstream logging records the same tool events.

CME executes tools under the console's operator identity. The legacy controller
still owns the interactive Approve/Reject button; CME does not yet open that
gate, so a `Confirm::Always`/`Confirm::IfMutating` call runs once the model
proposes it. The tool's own admission, firewall and subsystem gates still apply.

## Memory logging

Each CME session is pre-registered in agentstream under the chat session id.
The driver's `MemoryBridge` logs the episodic turn and runs compression when
enabled. `CmeRuntime::log_turn_frames` writes the user message and each SSE
frame to agentstream so the conversation is queryable even when the UI never
reconnects.

## Migration

`mm_data::migrate_from_mm_serve(old_root, new_root)`:

- copies `metamind.db` to `sqlite/metamind.db`;
- copies the legacy `graph/` RocksDB store to `cubecast/main/` when the
  destination is empty;
- copies `users/`, `projects/` and `apps/` home directories;
- imports each old `chat-sessions.json` session into agentstream with its title.

It is idempotent and returns a `MigrationReport` with the copied/imported counts.
The destination root is opened with `initialize_data_layer` after the files are
in place.

## Verification

- `crates/mm-serve/tests/phase7.rs` covers CME chat frames, snapshot, metrics,
  debug page, memory/agentstream logging, the `/api/chat` flag, five isolated
  sessions, the full tool catalog plus a model→tool→model round, migration, and
  an ignored 25-session load smoke.
- Existing `mm-serve` routes and tests continue to pass with CME disabled.
- Run the load smoke with:
  `cargo test -p mm-serve --test phase7 -- --ignored`.

## Remaining production work

- `/api/chat` currently switches implementations; the legacy controller and
  `mm-store-graph` are not deleted.
- Canary rollout, release tagging, and a 100-concurrent-session benchmark are
  operational tasks, not code in this repository.
- CME returns the text plus one `block` frame per non-text chunk after the
  turn completes; token-by-token streaming and proactive frames are not yet
  mapped into the console's live frame protocol.
- The preference-learning flag is parsed but the console does not yet feed
  conversation signals into `PreferenceLearner`.
- CME tools share the controller's catalog, admission and execution, but the
  interactive Approve/Reject gate is still legacy-only; CME currently runs a
  proposed tool directly under the operator identity.
- Multi-device presence/broadcast/handoff exists in `mm-data`/`MemoryBridge`
  but is not mounted as console routes yet.
- Migration verifies by reopening and querying the imported session; it does not
  checksum every copied RocksDB file.
