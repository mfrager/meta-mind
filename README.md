# Metamind — streamlined core

Metamind is a three-pass conversation core with one transparent event log. The
whole runtime is six Rust crates and one React workspace:

| Crate | Responsibility |
|---|---|
| `mm-core` | Config, session/turn IDs, hashing |
| `mm-prompt` | Prompt tree rendering and hashes |
| `mm-llm` | Provider client, request/response types, mock client |
| `mm-agentstream` | Event envelope, JSONL store, live bus, projections, LLM gateway |
| `mm-cme` | `TurnPipeline`: Interpret → Process → Verify, tools, budgets |
| `mm-serve` | `POST /api/turns`, sessions, event snapshot/SSE, health |

Everything else — the legacy kernel, controller, CME v3, data/RDF layer, agent
runtime, inspector, and their tests — is frozen in `archive/legacy/` and
excluded from the workspace. The annotated tag `legacy-freeze-2026-11-25` is the
last full-workspace reference:

```bash
git worktree add /tmp/metamind-legacy legacy-freeze-2026-11-25
```

## What a turn does

1. **Interpret** decides between a fast conversational reply and an action
   turn. An action turn registers the checks Verify must satisfy.
2. **Process** calls the model with the visible tool set, stages conversation
   state, and stops when the model returns a final reply.
3. **Verify** runs the registered checks, including the LLM judge. A rejection
   retries Process with the failure context; five rejections fail the turn with
   `VERIFY_EXHAUSTED`.

Only Verify approval commits staged state. Every model call goes through the
gateway, so no call can bypass the log.

## Agentstream

`data/core/agentstream/<session>.jsonl` is the only LLM/internal log. Each line
is one versioned event with a per-session `seq`, and `data` is the full payload:

- `llm.request` contains every message and every offered tool schema, not a
  preview.
- `prompt.rendered` records the strategy, hash, variables, and message count.
- `internal.action` records state load/stage/commit/discard and verification
  dispatch; `internal.error` records recovered failures.
- `state.updated` is written only when staged state commits.
- `llm.delta` is live-only and never persisted.

The snapshot route returns the same bytes the file holds:

```bash
curl -s 'http://127.0.0.1:8899/api/agentstream/sessions/<id>/events?from=1&limit=50'
curl -N 'http://127.0.0.1:8899/api/agentstream/sessions/<id>/stream?from=1'
```

The SSE frame is `id: <seq>`, `event: <type>`, `data: <record>`. A cursor below
the retained segment start receives a `resync` frame carrying the current page.

## Run it

```bash
# tests and guards
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
./scripts/check-core-only.sh

# server (config/server.toml, data/core/)
./start-mm-serve.sh

# console on http://127.0.0.1:8900 (proxies /api to 8899)
./start-console.sh
```

Provider credentials come from `OPENAI_BASE_URL` and `OPENAI_API_KEY`; they are
never in `config/llm.toml`. Without them the server still boots and health
answers, but a turn reports the provider error in `turn.failed`.

The HTTP surface is deliberately small:

| Route | Purpose |
|---|---|
| `POST /api/turns` | Validate text, create the session if absent, return `202 {sessionId, turnId, seq}`, then run the turn |
| `GET /api/sessions` | Session index, newest activity first |
| `GET /api/agentstream/sessions` | The same index |
| `GET /api/agentstream/sessions/{id}/events` | JSONL page from a cursor |
| `GET /api/agentstream/sessions/{id}/stream` | Replay from a cursor, then live tail |
| `GET /api/health` | Status, version, data root, session and turn counters |

There is no chat SSE, no `/api/llm-activity`, and no separate inspector feed.
The transcript and the log are two views of one stream.

## Frontend

The console is one workspace: a session picker, the transcript, the composer,
and the Agentstream dock. The transcript is derived from events
(`turn.started` → user, `pass.*`/`llm.*` → progress, `agent.message` →
assistant, `turn.failed` → error). The dock groups rows Turn → Pass → Call and
its expanded request shows **Prompt**, **Tools**, **Response**, and
**Internals** tabs, reading `record.data` directly.

```bash
cd frontend
npm ci
npm run typecheck
npm test
npm run build
npm run test:e2e   # one Playwright smoke test against e2e/mock-server.mjs
```

See [`frontend/README.md`](./frontend/README.md) for the frontend contract.

## Tests

Only the kept crates and the kept frontend are tested. The matrix covers the
event envelope and store, gateway full-prompt capture, the three passes and
retries, staged state commit/discard, tool scope, prompt fidelity, SSE byte
equality and resync, and the frontend event reducer, store, viewer, and stream
hook. `cargo test -p mm-serve export_agentstream_bindings` rewrites
`frontend/src/lib/wire/agentstream.ts`; CI fails if the checked-in file differs.
