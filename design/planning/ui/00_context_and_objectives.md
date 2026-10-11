# 00 — Context, reference review, and objectives

> Parent: [`INDEX.md`](./INDEX.md) · Reference implementation reviewed: `~/Scripts/rust_symbolic/frontend`
> (chat section) · Target system: Metamind (`mm`), this repository.

---

## 1. The reference: rust_symbolic's chat web UI, reviewed in detail

The rust_symbolic frontend is a React 19 + Vite + TypeScript SPA styled with Tailwind CSS v4 and
shadcn/ui (style **`base-nova`**, Base UI primitives, lucide icons, Geist Variable font, oklch tokens,
`shadcn/tailwind.css`, `tw-animate-css`). Its chat section is the direct model for the Console's chat
pane, so it is worth knowing exactly what it does.

### 1.1 Anatomy (files reviewed)

| File | Lines | Role |
|---|---|---|
| `frontend/src/pages/chat-page.tsx` | 195 | The chat page: transcript reducer + two-pane layout (chat column + `ModelPanel` aside) |
| `frontend/src/components/chat/chat-message.tsx` | 110 | One message bubble: user/assistant/tool variants, phase label, streaming state, elapsed timer |
| `frontend/src/components/chat/tool-call-card.tsx` | 243 | Collapsible tool-call card with status icon, and smart Input/Output bodies (SEM-aware) |
| `frontend/src/components/chat/chat-input.tsx` | 41 | Textarea + Send, `Ctrl+Enter` to send, disabled while streaming |
| `frontend/src/components/chat/model-panel.tsx` | 156 | Right aside: session MODEL (Monaco read-only Turtle/RDF-XML with downloads) + EQUATIONS (KaTeX) |
| `frontend/src/components/chat/sem-viewer.tsx` | 56 | SEM s-expression viewer: structural indent, syntax highlight, copy button |
| `frontend/src/lib/chat-transcript.ts` | 242 | Pure reducer: ordered backend events → transcript items, sequence dedup, run-state machine |
| `frontend/src/lib/api.ts` (chat part) | ~280 | SSE client: `POST /api/chat`, resumable `GET /api/chat/{requestId}/stream?from=N`, session-model channel |
| `frontend/src/lib/types.ts` (chat part) | ~150 | `ChatStreamEvent`, `ChatTranscriptItem`, `ToolCallEvent`, `SessionModelSnapshot` |

### 1.2 The data flow (what to keep verbatim)

1. **One POST opens a run.** `POST /api/chat` with the query; the response header `X-Request-Id` (ULID)
   names the durable backend run; the body is an SSE stream of numbered frames.
2. **Ordered, resumable events.** Every frame carries a monotonic `sequence`. On connection loss the
   client reconnects with `GET /api/chat/{requestId}/stream?from=lastSeq+1` and replays exactly the missed
   frames. A `seenSequences` set makes replay idempotent in the reducer.
3. **A pure reducer owns the transcript.** `reduceChatEvent(state, event, sequence)` is a plain function;
   events (`llm_request_started`, `llm_response`, `llm_request_finished`, `tool_call`, `tool_result`,
   `final_response`, `completed`, `failed`, `reconnecting`) fold into `ChatTranscriptItem[]`. This makes
   live streaming and reconnect replay use one code path — the single best idea in the reference.
4. **Turn hygiene.** Assistant turns are created at `llm_request_started` (so "Processing…" shows
   immediately), filled by `llm_response`, terminated by `llm_request_finished`, and *hidden* if they
   produced no final text (tool-only turns disappear after they did their job). Machine-only responses are
   suppressed. Tool calls upsert by id (`tool_call` then `tool_result`).
5. **Run-state machine.** `processing → reconnecting → completed | failed | detached`, with honest UI copy
   for each ("Reconnecting to the backend…", "The connection is detached; the backend may still be
   processing this request.").
6. **A second, independent channel** (`GET /api/chat/{sessionId}/model?from=version`) feeds the side panel
   with versioned `SessionModelSnapshot`s (Turtle, RDF/XML, equations). Panel state is not entangled with
   the transcript.

### 1.3 Visual behavior of the chat (what to keep)

- User messages: right-aligned, `bg-primary text-primary-foreground`, `max-w-[85%]`, whitespace-pre-wrap.
- Assistant messages: left-aligned bordered `bg-card` bubbles, markdown rendered via
  remark/rehype + KaTeX + `rehype-sanitize` through `dangerouslySetInnerHTML` into `.markdown-viewer`.
- **Phase labels** on assistant turns (`Build / Verify / Repair / Solve / Final response`), with the
  repair phase getting an amber border/tint — phase is *visible information*, not just logging.
- **Tool call cards**: status icons (`CircleDashed` spinning / `CheckCircle2` green / `XCircle` red),
  mono name, right-aligned status label, chevron, collapsed by default; expansion shows `Input` and
  `Output` sections. Inputs render scalar fields as a `dl` grid, SEM payloads through `SemViewer`,
  `operations[]` arrays as action + highlighted target + payload; outputs render markdown, `summary`
  strings, model SEM, or pretty JSON. A `model_verify(correct=false)` is shown as "verification failed"
  in destructive styling — a successful tool call with a failed verdict is *not* a generic error.
- **Processing state**: pulsing dot + "Processing…" + live elapsed `m:ss` (mono, tabular-nums).
- **Empty state**: one quiet sentence in the center.

### 1.4 Weaknesses this plan deliberately fixes

| # | Weakness in the reference | Fix in this plan |
|---|---|---|
| W1 | Side panel is fixed to one content type (model + equations), `w-96`, no modes | General **Focus Document** workspace with a kind registry, viewer/editor modes, live status (`05`) |
| W2 | Chat input is a bare textarea + Send | Rich composer: slash commands, artifact mentions, **button grids**, **image chooser**, inline chat forms, suggestion chips, stop/regenerate (`08` §5) |
| W3 | ~20 shadcn primitives, ad-hoc styling, `rounded-xl` on bubbles and panels | 70+-component catalog with tokens, a `/dev/catalog` page, and a **radius policy enforced by lint** (`02` §5, `04`) |
| W4 | Almost no graphics (one Monaco box, one KaTeX list) | Charts, sparklines, gauges, budget meters, treemaps, sunburst, mind maps, run timelines, icon chips (`04` §4, `05` §7–8, `07`) |
| W5 | `dangerouslySetInnerHTML` markdown pipeline without a code-block renderer | Typed **content blocks** in the transcript (code, math, table, chart, tree, proof, …) with per-kind renderers (`08` §3) |
| W6 | No trees, no maps, no document editing | Tree engine + ten domain trees (`06`), mind-map viewer (`07`), document editors routed through change sets (`05` §5) |
| W7 | Single fixed pane layout, no resize/collapse/keyboard model | Resizable split, pane swap, fullscreen focus, ⌘K palette, full keymap (`03`) |
| W8 | Transcript is fire-and-forget: no search, export, branching, or input requests | Transcript search/export, branch/fork, structured **input requests** with pending/answered states (`08` §6–8) |

### 1.5 What is reused as-is (no improvement needed)

- The SSE contract shape (`X-Request-Id`, `?from=`, sequence dedup) — extended, not replaced (`09` §2–3).
- The reducer/store split (`chat-transcript.ts` pattern) and hidden-empty-turn hygiene (`08` §3.2).
- The `base-nova` shadcn style, lucide icons, Geist fonts, oklch tokens, sonner toasts, cmdk palette.
- Honest degraded-state copy. Metamind's own culture ("`Unavailable` *with a reason*") demands more of it,
  not less.

---

## 2. The target system: Metamind, and what a console must respect

Metamind is a **cognitive kernel that can modify itself under a gate**: 21 Rust crates (`crates/mm-*`), a
SQLite store (110 tables), an RDF store (named graphs, SHACL shapes in `ontology/`), an append-only
event log, and one operator CLI (`mm-cli`, 42 subcommands). The console is a second operator surface over
the same kernel — never a second source of truth.

The invariants that shape the UI (from `README.md` and `infodocs/`):

1. **The LLM proposes; the control plane structures; symbolic layers prove; deterministic code enforces.**
   The chat is an orchestrator UI over *bounded operations*, not a free-form agent playground. The UI must
   show which bounded operation ran, with what verdict — hence operation cards (`08` §4), not just prose.
2. **Nothing invents an answer.** Components return `Unavailable` with a reason. The UI has a first-class
   **unavailable/degraded chip** and *never* renders an empty placeholder where a reason exists.
3. **Production is written only by promotion.** Document editors write drafts and change sets; the
   "Save" path ends in `changeset new → sandbox run → promote` with a written reason (`05` §5.5).
4. **Everything is event-sourced and replayable** (`logs verify/tail/trace`, `replay`, `loop replay`).
   The console's log and timeline documents are views over the same streams (`05` §8).
5. **Budgets are hard** (four self-engineering budgets). Budget meters are persistent UI furniture
   (status bar + chat banners), because running out is normal operation, not an exception.
6. **Epistemic status is visible.** Claims carry observation/assumption/inference status; the UI colors
   them (`02` §3) and stamps them on chat output and documents.

### 2.1 The surface area the console must cover

The 42 CLI subcommands (see `infodocs/user-guide.md` §7) group naturally into console surfaces:

| CLI group | Console home |
|---|---|
| `doctor`, `replay`, `graph validate`, `logs *` | Status bar + Log/Replay documents (`05` §8) |
| `being *`, `memory *`, `epistemic *`, `library *`, `skill *`, `case *`, `frame *` | Tree editors + documents (`06`), chat ops (`08` §4) |
| `episode run`, `decide`, `firewall eval`, `compare check`, `risk analyze`, `calibration *`, `conformance`, `calibrate` | Chat ops with rich verdict/chart outputs (`08` §4.4–4.8) |
| `tool *`, `action *`, `verify run`, `mcp *` | Chat ops + Tools document + action ledger tree (`06` §5.9) |
| `meta *`, `regression run`, `changeset *`, `sandbox run`, `promote/reject`, `budget show`, `pi *`, `audit production-tree` | Self-engineering workspace docs (`05` §6.5–6.7), chat ops |
| `loop run/status/artifacts/replay/resume`, `self-model report`, `debt scan`, `gc`, `module load/rollback`, `design revise/history` | Run documents + charts + module tree (`05` §7, `06` §5.6) |
| `codex *`, `llm *` | Code-metadata and LLM-substrate documents (read-only + verify) |

---

## 3. Objective and scope

**Objective.** Build the MetaMind Web Console: a local-first, two-pane (plus left menu), chat-driven
operator console, templated on shadcn blocks and styled by the *Structured Color* design language, that
makes the kernel's full surface — being, memory, epistemics, library, judgment, tools, self-engineering,
autonomy — visible, inspectable, and operable, with a graphically rich document workspace on the left and
a rich chat log on the right.

**In scope:**

1. `crates/mm-serve` — an axum HTTP+SSE server embedding the kernel (read and bounded write paths),
   with the chat orchestrator and the focus/document services.
2. `frontend/` — the SPA: shell, design system, component library, document pane, tree editors,
   mind-map viewer, chat pane, command palette.
3. The wire protocol (`09`) and its generated TypeScript mirror.
4. Tests at every level and the CI jobs for them (`10` §6).

**Out of scope (explicit non-goals):**

- Multi-user, multi-tenant, or remote-access serving. The console binds `127.0.0.1` and ships no auth
  beyond a local session token (`09` §6). Hardening for remote use is a separate plan.
- Replacing the CLI. `mm-cli` stays the headless/scriptable surface; the console calls the same library
  entry points, and the CLI's `--json` output shapes are the wire-format baseline.
- Mobile authoring. Small screens get a read-only monitoring tier (`03` §8).
- Training/tuning UIs for the hosted LLM paths, and a visual ontology editor for OWL entailment
  (the tree editor edits *shapes and instances*, not reasoning rules).

---

## 4. UX principles (the ten rules every screen must pass)

1. **Artifact-centric.** The unit of work is the *document* (any focusable artifact). Chat helps you
   make and judge documents; documents are never buried inside chat-only rendering.
2. **Two-way focus.** Chat can open/refresh a document in the left pane; a document selection can be
   sent into chat. One `focus` protocol serves both (`05` §9).
3. **Honest state.** `Unavailable` with a reason, `degraded` with what still works, `detached` with what
   may still be happening. Never fake progress; never hide a verdict.
4. **Provenance on the surface.** Every claim, number, and verdict shows its epistemic badge and can open
   its lineage (event-log trace) in one click.
5. **Color is data.** The ten-hue taxonomy (`02` §3) assigns each hue one meaning. A screen that could be
   monochrome should be; color marks *state and kind*, not brand.
6. **Structure over decoration.** 1px borders, tinted surfaces, micro-labels. Small radii. No gratuitous
   shadow, gradient, or glass.
7. **Progressive disclosure.** Summaries first (chips, sparklines, meters), details on expand, raw JSON
   one click deeper. Every card is a funnel: status → summary → detail → raw.
8. **Keyboard-first.** ⌘K reaches every command and document; the two panes, the tree, and the composer
   are fully keyboardable; shortcuts are discoverable in tooltips.
9. **Deterministic surfaces for a deterministic system.** Given the same event stream the UI renders the
   same transcript and the same documents (golden transcript tests, `10` §5.3).
10. **Business-like, not over-done.** Rich graphics where they carry information (charts, trees, maps,
    meters); calm everywhere else. Delight comes from clarity and speed.

---

## 5. Research foundation (code-available) and locked decisions

Every idea below is borrowed from something whose source is available. Only ideas with code are used.

| Idea we borrow | Source (code) | What we take | Decision locked in this plan |
|---|---|---|---|
| Composable block-first UI kit | shadcn/ui + shadcn blocks — https://ui.shadcn.com, https://ui.shadcn.com/blocks | Block templates (sidebar, login-free app shells, cards, dashboards), `components.json` workflow, CVA variants | Frontend is shadcn `base-nova` (Base UI primitives) + Tailwind v4; blocks are page templates, never final styling |
| The proven in-house chat console | rust_symbolic `frontend/src/components/chat/*`, `lib/chat-transcript.ts`, `lib/api.ts` | SSE resume protocol, transcript reducer, turn hygiene, tool-call cards, SEM viewer, session panel channel | Adopted and extended: `08`, `09` |
| OKLCH design tokens as CSS variables | shadcn theme + Tailwind CSS v4 `@theme` — https://tailwindcss.com | CSS-first theming, `@theme inline`, semantic token names | All color via tokens; no literal hex in components (`02`) |
| Virtualized trees with editing + DnD | react-arborist — https://github.com/brimdata/react-arborist (reference), dnd-kit — https://github.com/clauderic/dnd-kit, TanStack Virtual — https://tanstack.com/virtual | Node model, drag validation hooks, virtual rows | Custom `TreeCore` on `@tanstack/react-virtual` + `@dnd-kit` for full editor semantics (`06` §2) |
| High-quality node graphs | React Flow (xyflow) — https://github.com/xyflow/xyflow | Custom node/edge rendering, minimap, controls, view transforms | Graph mode of the mind-map viewer and provenance maps (`07` §3) |
| Tidy/radial hierarchical layout | d3-hierarchy (tidy tree, cluster, pack, treemap, partition) — https://github.com/d3/d3-hierarchy | `tree()`, `cluster()`, `partition()`, `treemap()` layouts + `linkHorizontal` paths | Radial mind-map engine and tree viewers' sunburst/treemap modes (`07` §4, `06` §3.3) |
| Business charts as tokens | Recharts + shadcn `chart` (ChartContainer/ChartTooltip) — https://github.com/recharts/recharts | Categorical palette binding, tooltip/legend patterns, responsive containers | Primary chart library for the chart documents (`05` §7) |
| Text-as-diagram fallback | Mermaid (rendered, sanitized) — https://github.com/mermaid-js/mermaid | Sequence/flow rendering for chat `diagram` blocks | Optional block kind in chat, lazy-loaded (`08` §3.3) |
| Markdown + math + safe HTML | remark/rehype + KaTeX + rehype-sanitize (as in rust_symbolic) | The exact pipeline | Kept; extended with syntax-highlighted code blocks (`08` §3.3) |
| Command palette UX | cmdk — https://github.com/pacocoursey/cmdk | Command grouping, scoring, keyboard model | ⌘K palette over the 42 commands + documents (`03` §5) |
| Toast/sonner conventions | sonner — https://github.com/emilkowalski/sonner | Positions, promise toasts | Kept (`04` §3.8) |
| Editable code/diff surfaces | Monaco (`@monaco-editor/react`) — https://github.com/suren-atoyan/monaco-react | Read-only viewers and diff editor for Turtle/RDF/code documents | Kept for code docs only; forms are native components (`05` §6) |
| Rich, resizable panes | react-resizable-panels (or custom split) + `PanelGroup` semantics | Persisted sizes, collapse, min/max | 2-pane split per `03` §3 |
| Event-sourced replay determinism | The kernel's own `replay`/`loop replay` (`mm-eventlog`) | The rule that UI state is a fold over ordered events | Golden transcript + replay-equality tests (`10` §5.3) |
| GenAI telemetry naming | OpenTelemetry GenAI semconv (as used by `mm-log`) | Conventional span/attribute names for UI-originated calls | `mm-serve` logs UI actions with the same field names (`10` §6.6) |

**Stack versions (locked):** React 19, Vite 8, TypeScript ~6, Tailwind CSS 4, `@base-ui/react`,
`class-variance-authority`, `tailwind-merge`, lucide-react, TanStack Query 5, Zustand 5,
`@tanstack/react-virtual` 3, `@dnd-kit/*` 6, `@xyflow/react` 12, `d3-hierarchy` 3, Recharts 3 (or the
version pinned by the shadcn chart block), embla-carousel-react 8, KaTeX 0.16, Monaco 4.x, cmdk 1.x,
sonner 2.x. Exact pins live in `frontend/package.json` at M0 (`10` §2).

---

## 6. Personas and primary flows

| Persona | What they do in the Console | Primary surfaces |
|---|---|---|
| **Operator** | Boots the kernel, watches the loop run, catches budget/failure states, rolls back | Status bar, Run document (live log + stage ticker + charts), chat ops, alerts |
| **Researcher/author** | Explores being, memory, epistemics; builds goals; asks bounded questions | Trees (`06`), mind map (`07`), chat (`08`), claim/memory documents |
| **Self-engineering reviewer** | Reviews change sets, sandbox runs, benchmarks, promotion decisions | Diff documents, Proof cards, budget meters, `promote/reject` chat flow with reason form |
| **Auditor** | Replays a run, verifies logs, inspects provenance and production-tree integrity | Log/Replay documents, provenance tree, `audit production-tree` output |

**Flow A — "What is the system doing right now?"** Open Console → status bar shows kernel health and
active run → click the run chip → Run document focuses (live stage ticker + log + budget meters) →
ask chat "why is stage 6 slow?" → chat answers with an operation card (`meta analyze`) and focuses a
chart document of stage durations.

**Flow B — "Add a goal and see it on the map."** Chat: `/being goal add …` → inline **chat form** (title,
timescale, success criteria) → operation card shows the write + provenance → goal tree refreshes → user
clicks "Open in mind map" → left pane focuses the goal mind map, the new node animates in.

**Flow C — "Review and promote a change set."** Chat: `/changeset show cs_01H…` → Diff document focuses →
`/sandbox run` → operation card streams build/test results → verdict card + benchmark chart →
`promote` requires the **reason form** (button grid: Approve / Reject / Defer) → status chips flip
`promotion-pending → promoted`, module tree shows the new version hot-loaded.

**Flow D — "Audit last night's loop run."** Logs document (virtualized, filtered to `run_id`) → timeline
document of the ten stages → provenance tree of every write → `audit production-tree` verdict card →
export transcript + documents as an audit bundle.

---

## 7. Document map of this plan

- [`01_architecture.md`](./01_architecture.md) — the system behind the pixels.
- [`02_design_system.md`](./02_design_system.md) — the styling law (start here for look-and-feel).
- [`03_shell_and_layout.md`](./03_shell_and_layout.md) — the two panes and the furniture around them.
- [`04_component_library.md`](./04_component_library.md) — every reusable piece.
- [`05_document_pane.md`](./05_document_pane.md) — the left pane.
- [`06_tree_editors.md`](./06_tree_editors.md) — trees.
- [`07_mind_map.md`](./07_mind_map.md) — the mind-map viewer.
- [`08_chat_pane.md`](./08_chat_pane.md) — the right pane.
- [`09_api_and_streaming.md`](./09_api_and_streaming.md) — the wire.
- [`10_build_and_test.md`](./10_build_and_test.md) — how to build and prove it.
- [`11_risks_and_references.md`](./11_risks_and_references.md) — risks, references, open questions.
