# 10 — Build sequence, testing, and the pass gate

> Parent: [`INDEX.md`](./INDEX.md) · Next: [`11_risks_and_references.md`](./11_risks_and_references.md)
> · Conventions follow this repo's build plans: deliverables at **exact paths**, per-milestone
> acceptance criteria, and a final **pass gate** of runnable commands.

---

## 1. Milestone map

| Milestone | Theme | Depends on | Est. effort |
|---|---|---|---|
| M0 | Scaffold: `mm-serve` skeleton + Vite/shadcn app + tokens | — | 2–3 days |
| M1 | Shell: menu, top bar, status bar, 2-pane split, palette | M0 | 2–3 days |
| M2 | Component library + `/dev/catalog` | M0 (parallel M1) | 4–6 days |
| M3 | Document pane: focus model, prose/code/table/log/chart docs | M1, M2 | 4–6 days |
| M4 | Tree engine + domain trees + mind-map viewer | M3 | 5–7 days |
| M5 | Chat pane: transcript, op cards, blocks, inputs | M2, M3 | 5–7 days |
| M6 | Integration: full op registry, focus⇄chat, runs/live docs | M4, M5 | 3–5 days |
| M7 | Polish: a11y pass, perf pass, docs, CI hardening | M6 | 2–3 days |

Milestones M2/M4 can proceed in parallel tracks against fixture data; M5 needs M3's focus protocol.

---

## 2. M0 — scaffold (exact deliverables)

```
Cargo.toml                                     # workspace += crates/mm-serve
crates/mm-serve/Cargo.toml
crates/mm-serve/src/{main,state,error,dto/mod,routes/mod,sse}.rs   # boots, /api/health, SSE codec
crates/mm-serve/tests/wire_compat.rs
frontend/package.json                          # pins per 00 §5
frontend/components.json                       # style: base-nova, aliases per reference
frontend/src/index.css                         # tokens: 02 §2–3 (all hues, radius scale, motion)
frontend/src/{main.tsx,App.tsx}
frontend/src/lib/wire/{index,chat,focus}.ts    # generated
frontend/src/lib/wire/fixtures/.gitkeep
frontend/src/routes/{workspace,catalog}.tsx    # placeholder pages
.github/workflows/ci.yml                       # += frontend + mm-serve jobs (§6.7)
```

Acceptance: `cargo build -p mm-serve` serves `/api/health`; `npm run dev` renders the token gallery;
`cargo test -p mm-serve export_types` regenerates `lib/wire` with no diff; CI green.

## 3. M1 — shell

Deliverables: `components/blocks/app-shell.tsx` (sidebar block), `components/shell/{topbar,statusbar,
command-palette,keyboard-map}.tsx`, `lib/store/{pane,focus,prefs}.ts`, routes wired, resizable split
(`resizable`), persisted pane sizes, keyboard map live (`⌘K`, `⌘1/2`, `⌘\`, `⌘B`, `⌘E`…).
Acceptance: pane resize/swap/collapse persists across reload; palette lists 42 commands (passthrough
tier acceptable); status bar renders live health from `/api/health`; responsive tiers verified at the
four widths (`03` §8).

## 4. M2 — component library

Deliverables: all `components/ui/*` primitives (§`04` §2), `components/kit/*` (§`04` §4 — every
component), `components/blocks/{dashboard-*,cards-*,table,settings}` adaptations, `/dev/catalog` with
variant×state matrices + radius gallery + token inspector, `styles/` recipes.
Acceptance: catalog renders every component in every state; axe passes on each; radius lint active
(`02` §5.3); unit tests per component (render + states + a11y); visual baseline screenshots committed.

## 5. M3 — document pane

Deliverables: `components/docs/{registry,doc-shell,doc-header,doc-footer,history-drawer}.ts`,
viewers/editors for `prose|code|table|json|form|diff|proof`, `log` (VirtualTable), `chart` (10 recipes
in `components/charts/`), `timeline`, `run` composition; `lib/api/focus.ts` + focus-channel hook;
save-as-change-set wizard (`wizard-form`); export menu; provenance strip + lineage drawer.
Acceptance: chat-frame `focus` opens/refreshes a doc (fixture-driven); log doc renders 1M-row fixture at
60fps scroll; chart doc exports PNG/CSV; edit→draft→wizard→`POST /api/changesets` round-trips against
the real kernel; `unavailable`/`degraded` docs show verbatim reasons.

## 6. M4 — trees + mind map

Deliverables: `components/trees/{tree-core,outline,tree-table,sunburst,treemap,use-tree-controller}`,
ten adapters (`adapters/{ontology,being,memory,epistemic,library,modules,policies,provenance,
changesets,budgets}.ts`), mutation/undo pipeline + `POST /api/trees/{name}/ops`;
`components/mindmap/{radial-engine,graph-engine,toolbar,legend,outline-mode}`, map adapters, `/map/:id`
route, export (svg/png/json/md), Web Worker layouts.
Acceptance: 20k-node fixture tree scrolls at 60fps; rename/move/delete round-trip with undo/redo;
keyboard conformance suite passes; radial map renders 5k nodes at ≥ 55fps with LOD; screen-reader
outline mirror works (axe + manual spot check); tree→map→chat reference round-trips.

## 7. M5 — chat pane

Deliverables: `lib/transcript/{reducer,selectors}` (superset reducer + golden fixtures),
`components/chat/{chat-pane,chat-header,composer,turn,phase-rail,op-card,op-registry,blocks/*,
button-grid,image-chooser,chat-form,input-request,suggestion-chips}`, slash menu + `@` refs,
`mm-serve` chat orchestrator (`src/chat/*`) with ≥ 20 ops wired + generic fallback, `/api/chat`
streams, input-request lifecycle.
Acceptance: replaying `chat_run_*.jsonl` renders the golden transcript (byte-equal DOM snapshot);
reconnect test drops frames and asserts no duplicates and correct resume; button-grid answer flows to
the kernel and echoes; image chooser validates + attaches + degrades honestly without provider support;
op cards render specialized outputs for `decide`, `firewall eval`, `budget show`, `verify run`,
`sandbox run`; `⌘↵`/Stop/Regenerate/Branch all work.

## 8. M6 — integration

Deliverables: full op registry parity (§`09` §5 — all 42 subcommands bespoke or passthrough with
forms), run documents with live `stage_tick` + trace map mode, cross-pane flows (`05` §9) end-to-end,
session persistence/branching, notifications digest.
Acceptance: Flow A–D from `00` §6 run end-to-end against the real kernel on a fresh data dir;
parity test green; the promotion rule enforced by API (409 path tested).

## 9. M7 — polish

Deliverables: a11y remediation list closed (axe + manual keyboard sweep), perf budgets measured and
documented (`01` §10), reduced-motion and contrast themes verified, `frontend/README.md` (dev guide) +
`infodocs/ui.md` (operator guide section), print stylesheets for export, production build (bundle
report under budget).
Acceptance: all pass-gate commands (§12) green on a clean machine from the README alone.

---

## 10. Testing strategy

### 10.1 Unit (vitest + RTL)
- Reducers (transcript, live docs, tree controller) — pure tests with frame fixtures, including replay
  idempotency and stale-response drops.
- Components — catalog-driven state matrix renders; CVA variants snapshot.
- `format/` utilities, ULID display, duration math, contrast-token checks (every `H-step` pair AA).

### 10.2 Integration (Rust, `crates/mm-serve/tests/`)
- `chat_stream.rs`: ordered frames, `?from=` resume, terminal states, cancel.
- `focus_channel.rs`: version monotonicity, snapshot diffs, `?from=` semantics.
- `wire_compat.rs`: golden fixtures byte-equal both directions.
- `parity.rs`: 42 subcommand coverage (bespoke route or exec allowlist).
- `promotion_rule.rs`: production write via direct route ⇒ 409 with reason.

### 10.3 Deterministic replay tests (the repo's culture)
- `frontend`: `npm run test:replay` folds each `chat_run_*.jsonl` through the reducer and diffs a
  committed DOM snapshot — live rendering and replay rendering must be identical (`00` §4.9).
- `mm-serve`: a recorded orchestrator session replays with **zero provider calls** (network guard armed,
  like `llm replay`), asserting identical frame sequences.

### 10.4 E2E (Playwright, `frontend/e2e/`)
Scenarios (each a spec with fixture kernel data): shell + palette + pane ops; chat flow with button-grid
answer; image chooser upload/paste; document focus from chat; tree rename + undo; mind-map search +
re-root; log follow-tail + filter; change-set wizard → promote reason form; detached/reconnect banner
(toxiproxy-style fault injection against the dev server); responsive tiers; dark mode smoke.
Visual regression: screenshot the catalog + the four showcase screens (workspace light/dark, run doc,
goal map) with a strict-diff threshold on token-level pixels.

### 10.5 Performance (`frontend/e2e/perf.spec.ts` + `scripts/perf/*.mjs`)
Budgets from `01` §10 asserted as tests (bundle size, frame timing, virtualized scroll FPS, map layout
times) with a small representative fixture set committed under `frontend/e2e/fixtures/perf/`.
Failures fail CI (no soft budgets).

### 10.6 Accessibility
axe on every catalog entry + every route; keyboard-only Playwright pass per surface; manual checklist
for screen readers (VoiceOver/NVDA smoke on chat, tree, map) recorded in `infodocs/ui.md`.

### 10.7 CI (`.github/workflows/ci.yml` additions)
```yaml
# frontend job
- run: npm ci
- run: npm run typecheck        # tsc -b --noEmit
- run: npm run lint             # eslint + radius lint
- run: npm run test             # vitest (unit + replay + contrast)
- run: npm run build            # tsc -b && vite build (bundle budget check)
- run: npx playwright install --with-deps chromium
- run: npm run test:e2e         # Playwright (incl. visual + perf)
# backend job
- run: cargo build -p mm-serve
- run: cargo test -p mm-serve
- run: cargo test -p mm-serve --test parity
- run: cargo test -p mm-serve export_types -- --ignored && git diff --exit-code frontend/src/lib/wire
```
The UI jobs sit alongside the existing workspace jobs and gate the same way (`09` §Phase gates in CI).

---

## 11. Logging & observability deliverables

- `mm-serve` request logging (JSON lines, `mm-log` field names) + per-run trace ids surfaced in the UI
  provenance strips.
- `data/logs/ui/*.jsonl` client logs (§`01` §8) with rotation; `doctor` gains a `ui` section reporting
  the console's own health (SSE bus size, draft store, upload dir).

---

## 12. Pass gate (the whole plan, runnable)

```bash
# 0. Clean machine, from repo root — the README walkthrough must suffice.
cargo build --workspace && cargo test --workspace          # kernel still green
cargo test -p mm-serve && cargo test -p mm-serve --test parity

# 1. Wire is single-sourced.
cargo test -p mm-serve export_types -- --ignored && git diff --exit-code frontend/src/lib/wire

# 2. Frontend quality bar.
cd frontend
npm ci && npm run typecheck && npm run lint && npm run test && npm run build

# 3. Determinism: replay folds to golden transcripts.
npm run test:replay

# 4. E2E + visual + perf + a11y.
npm run test:e2e

# 5. The four flows from 00 §6 against a fresh kernel (scripted, offline providers).
./scripts/ui_flows.sh --offline        # Flow A–D end to end; exits non-zero on any assertion

# 6. Radius law spot check.
npm run lint:radius                    # must report 0 violations outside .radius-allowlist.json
```

Gate notes:
- `ui_flows.sh` drives `mm-serve` + the built SPA through the ops API and asserts on the *rendered*
  transcript goldens and document statuses — the UI's own invariants ("nothing invents an answer",
  "nothing writes production outside promotion", "Unavailable with reason") are asserted as tests here.
- Any milestone merges only when its acceptance list passes; the final gate is the block above.
