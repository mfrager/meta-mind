# 09 — API and streaming: the wire behind the panes

> Parent: [`INDEX.md`](./INDEX.md) · Next: [`10_build_and_test.md`](./10_build_and_test.md)
> · Δ vs rust_symbolic: the reference protocol (`POST /api/chat` + `?from=` resume + session model
> channel) is adopted wholesale and extended with `focus`, `input_request`/`input_answer`, and domain
> routes for every kernel surface.

---

## 1. Conventions

- Base: `http://127.0.0.1:8787/api`; all JSON; all ULIDs **lowercase** hex; all timestamps epoch ms.
- Every response body is either the payload or `ApiError`:
  ```ts
  interface ApiError { code: string; message: string; reason?: string;      // Unavailable-with-reason
                       correlationId: string; retryable: boolean }
  ```
  `code` values: `unavailable`, `invalid_argument`, `not_found`, `conflict`, `budget_exceeded`,
  `firewall_blocked`, `locked` (exec lock), `internal`. The UI maps codes to chip/banner styles (`02`)
  and **always shows `reason` verbatim** when present.
- Pagination: cursor-based `?cursor=&limit=`, envelope `{ items, nextCursor?, total? }`.
- Idempotency: mutating POSTs accept `Idempotency-Key` (ULID); retries return the first result.
- Wire version: `GET /api/health → { ok, wireVersion: 1, kernel, dataDir, eventHead, budgets }`.

---

## 2. Route table

### 2.1 Kernel and health

| Route | Method | Returns |
|---|---|---|
| `/api/health` | GET | health envelope (status bar) |
| `/api/doctor` | GET | doctor report (all stores open, ontology loaded, writable) |
| `/api/exec` | POST | whitelisted CLI passthrough `{ argv } → { stdout_json, exit, provenance }` (`01` §5.4) |
| `/api/ui-errors` | POST | client error sink (best-effort) |

### 2.2 Chat (§3)

| Route | Method | Notes |
|---|---|---|
| `/api/chat` | POST | open a run; `X-Request-Id` header; body is the SSE stream |
| `/api/chat/{requestId}/stream` | GET | SSE resume `?from=<seq>` |
| `/api/chat/{requestId}` | GET | run summary (state, counts, timings) |
| `/api/chat/sessions` | GET/POST | session list / create |
| `/api/chat/sessions/{id}` | GET/PATCH/DELETE | transcript page (`?cursor=`), rename, archive |
| `/api/chat/sessions/{id}/branch` | POST | fork from a turn |
| `/api/chat/sessions/{id}/input-answer` | POST | input request answers (also streamed) |

### 2.3 Focus channel and documents (§3)

| Route | Method | Notes |
|---|---|---|
| `/api/focus/{docId}` | GET | one document snapshot |
| `/api/focus/{sessionId}/stream` | GET | SSE versioned snapshot stream `?from=<version>` (live docs) |
| `/api/focus` | POST | create a document (save draft / from chat) |
| `/api/focus/{docId}/draft` | PUT | autosave draft |
| `/api/focus/{docId}/history` | GET | version list |
| `/api/focus/{docId}/export` | GET | `?format=md|json|csv|png|svg|ttl|pdf` |
| `/api/focus/{docId}/provenance` | GET | lineage events for the provenance strip |

### 2.4 Domain reads and ledger writes (each maps to CLI `--json` shapes)

| Group | Routes (representative) |
|---|---|
| being | `GET /api/being`, `GET /api/being/invariants`, `POST /api/being/goals`, `PATCH /api/being/goals/{id}`, `POST /api/being/verify` |
| memory | `GET /api/memory/tree`, `POST /api/memory`, `POST /api/memory/recall`, `POST /api/memory/consolidate`, `POST /api/memory/forget`, `GET /api/memory/stats` |
| epistemic | `GET /api/epistemic/claims`, `POST /api/epistemic/ingest`, `GET /api/epistemic/contradictions`, `GET /api/epistemic/dependency/{id}` |
| library | `GET /api/library/{techniques,policies,frames,skills,cases}`, `POST /api/library/import`, `POST /api/library/applicable` |
| runs | `GET /api/runs`, `GET /api/runs/{id}`, `POST /api/runs` (episode/loop/sandbox), `POST /api/runs/{id}/{pause,resume,replay}` |
| decisions/firewall | `POST /api/decide`, `POST /api/firewall/eval`, `POST /api/risk/analyze`, `POST /api/calibration/{fit,conformal}` |
| tools/actions | `GET /api/tools`, `POST /api/tools/{id}/run`, `GET /api/actions`, `POST /api/actions/{id}/rollback` |
| selfeng | `GET/POST /api/changesets`, `POST /api/changesets/{id}/sandbox`, `POST /api/changesets/{id}/{promote,reject}`, `GET /api/budgets`, `GET /api/modules`, `POST /api/modules/{id}/{load,rollback}` |
| logs | `GET /api/logs/streams`, `GET /api/logs/{stream}/page`, `GET /api/logs/{stream}/tail` (SSE), `GET /api/logs/trace/{id}` |
| metrics | `GET /api/metrics/{self-model,budgets,calibration,firewall,risk,memory,llm,throughput,fitness,debt}?range=` (chart payloads) |
| trees | `GET /api/trees/{ontology,being,memory,epistemic,library,modules,policies,provenance,changesets,budgets}?cursor=` + `POST /api/trees/{name}/ops` (rename/move/add/remove batches) |
| maps | `GET /api/maps/{source}?root=&depth=` → `MindMapSpec` |

**Ledger writes** (memory/being/epistemic/tools) return `{ result, provenance: { eventId, traceId } }`;
**production-affecting writes** return 409 `conflict` with `reason: "use the change-set flow"` unless
issued through `/api/changesets` — the promotion rule is enforced *by the API*, not just the UI.

---

## 3. Streaming protocols

### 3.1 Chat run stream

```
POST /api/chat  { sessionId, query, refs[], attachments[], blocks? }
→ 200  X-Request-Id: 01h8…        (ULID)
→ text/event-stream:

id: 1
event: run
data: {"type":"run","requestId":"01h8…","sessionId":"01h8…","query":"…"}

id: 2
event: llm_request_started
data: {"type":"llm_request_started","turn_id":"01h8…","phase":"plan","turn_number":1,"visible":true}

id: 3
event: tool_call
data: {"type":"tool_call","id":"01h8…","turn_id":"01h8…","phase":"act","name":"memory_add","input":{…}}

…
id: 41
event: completed
data: {"type":"completed","subsystems":["memory","decision"]}
```

- **Resume**: `GET /api/chat/{requestId}/stream?from=<lastSeq+1>` replays exactly the missed frames
  (and `Last-Event-ID` is honored). The client dedups by `id` (sequence) in the reducer — idempotent
  replay, identical to the reference's `seenSequences` discipline.
- **Heartbeats**: comment frame `: ping` every 15s; if no frame in 45s the client shows `stream-stale`
  and auto-reconnects with backoff (0.5s, 1s, 2s, 5s cap) — notice row per attempt (the reference's
  `reconnecting` event, with a countdown).
- **Terminal events**: `completed` | `failed` (with `reason`). EOF without terminal event ⇒ client marks
  `detached` and keeps a Resync affordance (reference behavior).

### 3.2 Chat frame taxonomy (superset of the reference's `ChatStreamEvent`)

```ts
type ChatFrame =
  | { type: 'started'; query: string }
  | { type: 'run'; requestId: string; sessionId: string; query: string }
  | { type: 'reconnecting'; attempt: number }
  | { type: 'llm_request_started'; turn_id: string; phase: Phase; subsystem?: string;
      turn_number: number; visible: boolean }
  | { type: 'llm_response'; turn_id: string; phase: Phase; text: string;
      visible: boolean; is_final: boolean }
  | { type: 'llm_request_finished'; turn_id: string; outcome: 'response'|'error'|'cancelled';
      error?: string }
  | { type: 'tool_call'; id: string; turn_id: string; phase: Phase; name: string; input: unknown }
  | { type: 'tool_result'; id: string; turn_id: string; name: string; success: boolean;
      output: unknown; durationMs: number }
  | { type: 'block'; turn_id: string; block: ContentBlock }          // Δ: rich blocks
  | { type: 'input_request'; id: string; request: InputRequest }     // Δ
  | { type: 'input_answer'; id: string; answer: InputAnswer; at: number } // Δ
  | { type: 'focus'; doc: FocusDoc }                                // Δ: open/refresh left pane
  | { type: 'status'; docId?: string; status: DocStatus; reason?: string } // Δ
  | { type: 'progress'; label: string; value: number; max: number }  // Δ
  | { type: 'final_response'; response_id: string; text: string; blocks?: ContentBlock[] }
  | { type: 'completed'; subsystems: string[] }
  | { type: 'failed'; error: string; reason?: string };
```

Frames carrying `turn_id` upsert turn items exactly like the reference reducer; `block` frames append
to the turn's `blocks[]`; `focus` frames feed the left pane synchronously (a chat answer can *show* the
document in the same stream that describes it).

### 3.3 Focus channel

`GET /api/focus/{sessionId}/stream?from=<version>` — one SSE stream per session carrying
`{ type:'focus', doc }` frames for live documents (monotonic `version` per doc), plus `status`,
`stage_tick`, `budget_update` frames. Semantics mirror rust_symbolic's `/api/chat/{sessionId}/model`
(versioned snapshots, `?from=` resume) generalized to any document.

### 3.4 Log tails

`GET /api/logs/{stream}/tail?from=<seq>&filter=<json>` — SSE of log rows (same frame id discipline),
pausable by simply disconnecting (rows re-page through the cursor API).

---

## 4. Wire types and fixtures

- Rust DTOs (`crates/mm-serve/src/dto/*`) are the source of truth; `ts-rs` exports
  `frontend/src/lib/wire/*.ts` via `cargo test -p mm-serve export_types -- --ignored` (CI enforces
  no-diff). `schemars` additionally emits `docs/wire/schema.json` for external consumers.
- Golden fixtures shared by both sides (`10` §5.4): `tests/fixtures/{chat_run_*.jsonl, focus_*.json,
  errors.json}` — the Rust side asserts serialization matches byte-for-byte; the frontend replays
  `chat_run_*.jsonl` through the reducer and snapshot-tests the rendered transcript.
- Compatibility rule: additive fields only within `wireVersion: 1`; the frontend ignores unknown fields
  (forward compat) and refuses older servers (§1).

---

## 5. Command parity surface

For every CLI subcommand there is exactly one of: (a) a bespoke route + rich UI, or (b) the `/api/exec`
passthrough with an arg form + generic renderers. The parity matrix lives in
`crates/mm-serve/tests/parity.rs` and asserts coverage of all 42 subcommands; the catalog page shows
which tier each command is at (bespoke/passthrough) so the gap is always visible.

---

## 6. Security posture (local-first)

- Bind `127.0.0.1` only; non-loopback bind requires `--i-know-what-im-doing` and prints a warning banner
  in the UI (out of scope to harden — `00` §3).
- Session token: generated at boot, set as `HttpOnly; SameSite=Strict` cookie when `session_token = true`;
  Vite dev proxy forwards it; SSE and REST both require it (except `/api/health`).
- CSP: `default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'` (KaTeX/Monaco
  require inline styles); no remote fonts (Geist bundled via `@fontsource`, as the reference does).
- Uploads: size/type validated server-side; stored under `data/artifacts/ui/`; never executed.
- The `/api/exec` allowlist rejects unknown argv and any shell metacharacter; `sandbox run` is the only
  route allowed to spawn processes, and only with the kernel's own isolation helpers.
- No telemetry leaves the machine; UI logs stay under `data/logs/ui/` and rotate with the kernel's
  logging contract (`infodocs` §4.3).

---

## 7. Error and degradation matrix (UI expectations)

| Code | UI treatment |
|---|---|
| `unavailable` | slate `unavailable` chip + verbatim reason + "what still works" hints (`02` §3.2) |
| `budget_exceeded` | amber banner + `budget show` meters inline + queued-retry affordance |
| `firewall_blocked` | red verdict card + reasons + `Compare`/`Override?` (never silent) |
| `locked` | read-only banner ("another console or CLI holds the write lock") |
| `conflict` (production write) | points to the change-set wizard |
| `invalid_argument` | inline form errors bound to the offending fields |
| `internal` | `ErrorState` with correlation id + "Copy details" + "Open logs" |
| network/`detached` | transcript system row with countdown + Resync (`08` §3.2) |
