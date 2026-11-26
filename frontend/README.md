# Metamind console

One workspace over the retained routes: session picker, transcript, composer,
and the Agentstream dock. There is no second chat shell and no separate chat
SSE. Everything the UI shows comes from the agentstream.

## Transport

- `lib/api/turns.ts` — `startTurn({ sessionId, text })` posts `/api/turns` and
  receives `202 { sessionId, turnId, seq }`.
- `lib/api/agentstream.ts` — session list, event snapshot, SSE URL, health.
- `hooks/use-agentstream.ts` — initial snapshot, `EventSource` from
  `lastSeq + 1`, dedupe by `seq`, reconnect with backoff, and a snapshot refill
  when the live stream jumps a cursor or sends `resync`.
- `lib/store/agentstream.ts` — bounded persisted rows, `lastSeq`, live deltas by
  call id, connection state, filters, pause/clear, and Turn → Pass grouping.

A persisted record is a log row. A live `llm.delta` updates the open call row
without adding one.

## Transcript and dock

`lib/transcript/reducer.ts` derives the transcript from events:
`turn.started` → user item, `pass.*`/`llm.*` → pass progress with streamed text,
`agent.message` → assistant item, `turn.completed` → verified, `turn.failed` →
error item. A duplicate `seq` is a no-op.

`components/agentstream/AgentstreamDock.tsx` groups rows Turn → Pass → Call,
filters (all/turns/passes/calls/tools/verification/internals/errors), pauses,
clears, collapses, resizes by drag or keyboard, and copies the visible rows as
text.

`components/agentstream/CallViewer.tsx` expands one `llm.request` with four
tabs:

- **Prompt** — every message with role and complete content, assistant tool
  calls, and tool results.
- **Tools** — every offered tool's name, description, and complete
  `parameters` JSON, plus which tools the response called.
- **Response** — full text, finish reason, usage, latency, attempt.
- **Internals** — `prompt.rendered` variables/hash, `state.updated` before and
  after, `verification.result`, and `internal.action`.

It reads `record.data` directly; nothing is reconstructed or re-truncated.

## Run and test

```bash
npm ci
npm run dev        # 0.0.0.0:8900, proxies /api to 127.0.0.1:8899
npm run typecheck
npm test
npm run build
npm run test:e2e   # Playwright against e2e/mock-server.mjs
```

The Playwright smoke test starts a deterministic mock server, sends one action
turn, and checks the transcript, the full process prompt, all four offered tool
schemas, the judge result, and the same rows after reload.
