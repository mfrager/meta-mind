# 01 — Architecture

> Parent: [`INDEX.md`](./INDEX.md) · Next: [`02_design_system.md`](./02_design_system.md)
> · Δ vs rust_symbolic: the reference backend lives in rust_symbolic's `math-service`; here the console
> gets its own `mm-serve` crate embedding the Metamind kernel, and the frontend keeps the reference's
> reducer-first streaming shape.

---

## 1. System diagram

```
┌────────────────────────────────────────────────────────────────────────────────┐
│ browser — the Console SPA (frontend/, Vite + React 19 + Tailwind v4)           │
│                                                                                │
│  ┌──────────┐   ┌───────────────────────────────┐   ┌───────────────────────┐  │
│  │ AppSidebar│   │ DocumentPane (focus workspace) │   │ ChatPane (transcript) │  │
│  │  (menu)  │   │  registry: doc viewers/editors │   │  reducer + blocks     │  │
│  └──────────┘   └──────────────┬────────────────┘   └──────────┬────────────┘  │
│        StatusBar · TopBar · CommandPalette (⌘K)                │               │
└───────────────────────────────────────┬───────────────────────┬───────────────┘
                                        │ REST (JSON)           │ SSE (resumable)
                                        ▼                       ▼
┌────────────────────────────────────────────────────────────────────────────────┐
│ crates/mm-serve (axum, 127.0.0.1:8787)                                         │
│                                                                                │
│  http layer: routes · dto (ts-rs generated) · sse codec · session token        │
│  services:   chat orchestrator · focus service · domain services · exec        │
│                     │                   │                   │                  │
│                     ▼                   ▼                   ▼                  │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │ the kernel (existing crates, embedded — one process, one writer)         │  │
│  │ mm-core · mm-log · mm-store-sqlite · mm-store-graph · mm-eventlog        │  │
│  │ mm-llm · mm-being · mm-memory · mm-epistemic · mm-library · mm-metacog   │  │
│  │ mm-decision · mm-firewall · mm-tools · mm-metaanalysis · mm-mistakes      │  │
│  │ mm-selfeng · mm-pi · mm-runtime                                          │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
│                                        │                                       │
│                    data/metamind.db · data/graph (RocksDB) · data/logs          │
└────────────────────────────────────────────────────────────────────────────────┘
```

**Key principle:** `mm-serve` is *another operator surface* over the same kernel the CLI drives. It calls
the same library entry points `mm-cli` calls; it is not a rewrite of domain logic and it is not a
wrapper that shells out to `mm` per request (except the guarded `exec` passthrough, §5.4).

---

## 2. Workspace layout (exact paths)

```
Cargo.toml                                  # + members: "crates/mm-serve"
crates/mm-serve/
├── Cargo.toml                              # axum, tokio, tower-http, serde, schemars, ts-rs, futures-util
├── src/
│   ├── main.rs                             # arg parse (--config, --data-dir, --bind, --port), kernel boot, doctor preflight
│   ├── state.rs                            # AppState: Arc<Kernel> (stores, services), session token, subscriber bus
│   ├── error.rs                            # AppError → { code, message, reason } (Unavailable-with-reason preserved)
│   ├── dto/                                # wire types, one module per domain, #[derive(TS)] for export
│   │   ├── mod.rs · chat.rs · focus.rs · being.rs · memory.rs · epistemic.rs
│   │   ├── library.rs · runs.rs · tools.rs · changesets.rs · budgets.rs · logs.rs
│   ├── routes/
│   │   ├── mod.rs                          # router assembly, CORS=localhost, CSP, request logging
│   │   ├── health.rs · chat.rs · focus.rs · domain.rs · logs.rs · exec.rs
│   ├── chat/
│   │   ├── mod.rs                          # ChatOrchestrator: user query → bounded operations → frames
│   │   ├── planner.rs                      # intent → typed operation sequence (deterministic first)
│   │   ├── ops.rs                          # op executors: memory_add, decide, firewall_eval, tool_run, …
│   │   ├── frames.rs                       # ChatFrame taxonomy (mirrors frontend/src/lib/wire/chat.ts)
│   │   └── bus.rs                          # broadcast channel per request/session (subscribe/tail)
│   ├── focus/
│   │   ├── mod.rs                          # FocusService: doc registry, snapshots, version counters
│   │   └── docs.rs                         # doc-kind projections (run, log, chart, tree, diff, proof, …)
│   └── sse.rs                              # frame codec: id: <seq>, event:, data: JSON; heartbeat; resume
├── tests/
│   ├── chat_stream.rs                      # ordered frames, resume from N, idempotent replay
│   ├── focus_channel.rs                    # versioned snapshots, ?from= semantics
│   └── wire_compat.rs                      # golden JSON fixtures vs frontend/src/lib/wire/fixtures/
└── README.md                               # run instructions, route table (generated from 09)

frontend/                                   # package name: metamind-console
├── package.json · tsconfig.json · vite.config.ts · components.json · index.css
├── public/
├── src/
│   ├── main.tsx · App.tsx                  # router + providers (Query, Theme, Toast, DnD, XYFlow)
│   ├── routes/                             # one file per route (lazy-loaded)
│   ├── components/
│   │   ├── ui/                             # shadcn primitives (generated)
│   │   ├── blocks/                         # shadcn blocks adapted (shell, cards, dashboards)
│   │   ├── kit/                            # custom rich components (04 §4)
│   │   ├── trees/ · mindmap/ · chat/ · docs/ · charts/
│   ├── lib/
│   │   ├── wire/                           # GENERATED TS from ts-rs (do not edit)
│   │   ├── api/                            # typed fetch + SSE clients per domain
│   │   ├── store/                          # Zustand: panes, focus, drafts, prefs
│   │   ├── transcript/                     # reducer (pure) + selectors + golden tests
│   │   └── format/                         # numbers, durations, ULID prettify, bytes
│   ├── hooks/                              # useTreeController, useSse, useFocusDoc, useHotkeys, …
│   └── styles/                             # tokens consumed by index.css (design 02)
└── e2e/                                    # Playwright specs + fixtures
```

---

## 3. Frontend rendering and routing

### 3.1 Rendering model

SPA with **React 19**, **Vite 8**, no SSR. Rationale: the console is a local, authenticated-by-localhost
operator tool; SSR buys nothing and would complicate the single-writer kernel process. Server-driven
data arrives over REST + SSE; all rendering is client-side.

- **Route-level code splitting** with `React.lazy` + `Suspense`; heavy viewers (Monaco, Mermaid,
  mind-map engine, Recharts bundle) load on first use. Budgets in `10` §6.5.
- **Error boundaries** at pane level (`ErrorBoundary` per pane, modeled on rust_symbolic's), never
  blanking the whole shell; boundary errors are posted to the UI log (§8).

### 3.2 Routes

| Route | Screen | Notes |
|---|---|---|
| `/` | Home / overview | Health cards, recent runs, suggested actions (shadcn dashboard block) |
| `/chat/:sessionId?` | **The 2-pane workspace** (default screen) | Left = focus doc, right = chat; `sessionId` optional |
| `/focus/:docId` | Workspace with a pinned document | Deep link from chat links, exports, bookmarks |
| `/explore/{being,memory,epistemic,library}` | Tree workspaces | Tree + detail document (`06`) |
| `/build/{changesets,modules,pi}` | Self-engineering workspaces | Diff docs, promotion flows |
| `/operate/{runs,logs,tools,budgets,doctor}` | Operations workspaces | Live docs, meters |
| `/map/:mapId?` | Full-bleed mind map | Can be focused into the left pane too (`07` §9) |
| `/dev/catalog` | Component catalog | Every component with states/variants (`04` §1) |
| `/settings` | Console + kernel config surfaces | Read config; writes via change-set flow only |

The workspace route is the product. All others are document kinds opened *into* the workspace's left
pane when the user prefers (route → focus-doc translation table lives in
`frontend/src/lib/store/focus.ts`).

### 3.3 URL is state

`/chat/:sessionId?focus=<docId>&pane=docs|chat&mode=view|edit` — pane sizes, active tab of the doc, and
transcript scroll anchors are **not** in the URL (they are preferences); focus doc, session, and mode
**are** (they are shareable, bookmarkable state).

---

## 4. State management

| Layer | Tool | Owns |
|---|---|---|
| Server state | **TanStack Query 5** | Domain lists, documents, snapshots; keyed queries; mutations with optimistic update + invalidate |
| Live streams | **useSse hook** → pure reducers | Chat transcript, log tails, run progress; reducer functions in `lib/transcript/` and `lib/live/` are pure and unit-tested |
| UI state | **Zustand 5** (slices) | `usePaneStore` (split sizes, collapsed, swap), `useFocusStore` (open docs, tabs, dirty drafts), `usePrefsStore` (theme, density, keymap hints), `useChatStore` (composer drafts, pending input requests) |
| URL | react-router 7 | session, focus doc, mode (§3.3) |
| Drafts | `useFocusStore` + IndexedDB persist | Editor drafts survive reload; autosave to server draft endpoint every 5s (debounced) |

**Rules:**

1. Reducers are pure functions of `(state, frame)`; all SSE frames pass through one `applyFrame`
   middleware so logging, sequence dedup, and devtools see one stream (rust_symbolic's
   `reduceChatEvent` generalized).
2. Query keys are hierarchical and stable: `['memory','tree',filters]`, `['focus','doc',docId,version]`.
3. No component fetches directly; hooks in `lib/api/` return typed clients generated against `09`.
4. Transcripts and logs cap memory at 2,000 rendered items; older items virtualize to IndexedDB and
   rehydrate on scroll-up (transcript search spans the whole session).

---

## 5. The kernel access discipline

### 5.1 Single writer, one process

`mm-store-graph` is RocksDB-backed and `mm-store-sqlite` is SQLite — the kernel's stores are
**single-writer**. Rules:

1. `mm-serve` acquires a lock file `data/console.lock` (pid + start time) at boot and refuses to start
   if another `mm-serve` or a store-locking CLI command holds it. `doctor` preflight runs before serving.
2. While the console runs, read-write CLI commands against the same data dir print a pointer to the
   console or require `--force`. Read-only CLI (`--json` reads) stays allowed.
3. The console exposes an `exec` passthrough (§5.4) so the CLI surface remains reachable *through* the
   single writer when needed.

### 5.2 Services, not routes, call the kernel

Route handlers are thin: parse → call a service → map to DTO. Services in `crates/mm-serve/src/chat/`
and `.../focus/` call the same public functions `mm-cli` calls (e.g. `mm_memory::…`,
`mm_decision::…`, `mm_runtime::…`). Anything the CLI can do must be reachable from a service within
one milestone; drift is caught by the parity test (`10` §5.5) that runs the CLI `--json` and the service
DTO against the same fixtures and compares.

### 5.3 Write paths honor the promotion rule

| Write class | Examples | Console behavior |
|---|---|---|
| **Ledger writes** (always allowed) | `memory add`, `epistemic ingest`, `being goal add`, tool runs | Direct service call; every write returns its event-log provenance for the UI chip |
| **Draft writes** (console-local) | Document edits, composer drafts, map annotations | Stored as drafts; never touch production trees |
| **Promoted writes** (gated) | Module code, design revisions, production tree | Only through `changeset new → sandbox run → promote`; the UI's Save-as-change-set flow (`05` §5.5) is the only door |

### 5.4 `exec` passthrough

`POST /api/exec` accepts a whitelisted CLI argv (the 42 subcommands, no shell strings), runs it through
the same in-process library (not `Command::spawn`, except for `sandbox run` which needs the worktree),
and returns `--json` output as `{ stdout_json, exit, provenance }`. The command palette uses this for any
command without a bespoke rich UI yet — so **the palette is complete on day one** even before every op
card exists.

---

## 6. Streaming transport (summary; full protocol in `09`)

- **Chat runs:** `POST /api/chat` → `X-Request-Id` (ULID) + SSE body; resumable
  `GET /api/chat/{requestId}/stream?from=<seq>`; heartbeats every 15s; terminal `completed | failed`.
- **Focus channel:** `GET /api/focus/{sessionId}/stream?from=<version>` — versioned snapshots for the
  left pane's live documents (the generalization of rust_symbolic's `/api/chat/{sessionId}/model`).
- **Log tails:** `GET /api/logs/{stream}/tail?from=<seq>&filter=…`.
- **Run progress:** frames on the focus channel (`stage_tick`, `budget_update`).

All SSE responses are `text/event-stream`, `Cache-Control: no-cache`, `X-Accel-Buffering: no`, and each
frame is `id: <seq>\nevent: <type>\ndata: <json>\n\n` so `Last-Event-ID` works natively.

---

## 7. Wire types: one source of truth

Rust DTOs in `crates/mm-serve/src/dto/` carry `#[derive(TS)]` (`ts-rs`) and `#[derive(JsonSchema)]`
(`schemars`). A `cargo test -p mm-serve export_types` writes `frontend/src/lib/wire/*.ts`, checked in,
and CI fails if regeneration produces a diff (**generated files are never hand-edited**).

- Golden fixtures: `crates/mm-serve/tests/fixtures/*.json` ↔ `frontend/src/lib/wire/fixtures/*.json`
  (same files), consumed by both `wire_compat.rs` (Rust round-trip) and vitest snapshot tests.
- ULIDs serialize lowercase everywhere (repo convention from `infodocs/open-gaps.md` §G6 housekeeping —
  `serde_ulid` with lowercase hex; the frontend's `format/ulid.ts` adds grouping for display only).
- Versioning: every DTO carries `v: 1` today; additive changes only. Breaking changes bump and the
  frontend refuses to run against an older `mm-serve` (`/api/health` reports `wire_version`).

---

## 8. Observability from the UI itself

- Every chat run, op execution, and document save emits a structured JSON-line entry into
  `data/logs/ui/{yyyy-mm-dd}.jsonl` with the same field names `mm-log` uses (GenAI semconv for LLM
  calls: `gen_ai.request.model`, `gen_ai.usage.input_tokens`, …).
- Client-side errors (ErrorBoundary, SSE failures) post to `POST /api/ui-errors` (best-effort, queued).
- The status bar surfaces SSE connection state and last frame age; a stalled stream shows a chip within
  5s (`stream-stale`).

---

## 9. Config surface

`config/metamind.toml` gains a `[ui]` table (all optional; defaults shown):

```toml
[ui]
bind = "127.0.0.1"        # remote binding is a documented non-goal
port = 8787
theme = "light"           # light | dark | system
density = "comfortable"   # comfortable | compact
open_browser = true       # open the console on startup
session_token = true      # issue a localhost session token (cookie, SameSite=Strict)
max_upload_mb = 20        # image chooser + attachments
```

Environment overrides follow the repo's existing `MM_*` convention (`MM_UI_PORT`, …) and are read via the
kernel's config override helper (the same one `infodocs` flags as the correct pattern).

---

## 10. Performance budgets (enforced in `10` §6.5)

| Target | Budget |
|---|---|
| First contentful paint (workspace) | ≤ 1.5s on localhost, ≤ 250KB gzip initial JS (route chunks split) |
| Chat frame → visible | ≤ 16ms per frame (reducer + memoized rows); 1,000-frame transcript scrolls at 60fps (virtualized) |
| Log document | 1,000,000 rows virtualized, filter latency ≤ 100ms per keystroke (indexed in a Web Worker) |
| Tree document | 20,000 nodes virtualized; expand ≤ 50ms |
| Mind map | 5,000 nodes at ≥ 55fps pan/zoom with LOD (`07` §8) |
| SSE reconnect | Resume within 2s, no duplicate transcript items (sequence dedup) |
