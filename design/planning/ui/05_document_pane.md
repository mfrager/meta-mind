# 05 — The document pane (left): current document status, viewer & editor modes

> Parent: [`INDEX.md`](./INDEX.md) · Next: [`06_tree_editors.md`](./06_tree_editors.md)
> · Δ vs rust_symbolic: the reference's aside is a fixed model+equations panel; the Console's left pane
> is a general **focus workspace** — any artifact the user (or chat) focuses opens here as a document
> with a live status header, in viewer **or** editor mode.

**Requirement this doc implements:** *"The left-pane will show the current document status, whether a
document, a chart, a log, whatever the user is focused on in a document viewer or editor mode."*

---

## 1. Anatomy of the pane

```
┌─ DocumentPane ──────────────────────────────────────────────────┐
│ ┌ DocHeader ──────────────────────────────────────────────────┐ │
│ │[IconChip] Title (H1)          [chips: kind·status·epistemic]│ │
│ │ subtitle: mono id · source · updated 4m ago                 │ │
│ │ [View|Edit|Diff|Raw] segmented    [Actions ▾] [⋯] [⤢] [×]  │ │
│ ├─────────────────────────────────────────────────────────────┤ │
│ │                                                             │ │
│ │   DocumentBody — renderer for the document kind:            │ │
│ │   Prose / Chart dashboard / Log stream / Tree / Mind map /   │ │
│ │   Table / Diff / Proof / Code / Form / Timeline              │ │
│ │                                                             │ │
│ ├ DocFooter ──────────────────────────────────────────────────┤ │
│ │ ● saved 2m ago · 0 unsaved fields · v7 · [Save draft]       │ │
│ │ [Export ▾] [Focus in chat] [History] [Provenance]            │ │
│ └─────────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────┘
```

- The **kind color** paints the pane header's 3px left edge (`02` §3.1) and the icon chip.
- The **status chip** is the authoritative "current document status" at all times — including while a
  live document streams (`live` + last-frame age) or an editor has unsaved fields (`draft`, count).
- Multiple documents open as **pane tabs** (`03` §3); the pane shows one at a time.

---

## 2. The Focus Document model

```ts
// frontend/src/lib/wire/focus.ts (generated from crates/mm-serve/src/dto/focus.rs)
type DocKind =
  | 'prose' | 'chart' | 'log' | 'table' | 'tree' | 'mindmap' | 'diff'
  | 'proof' | 'code' | 'form' | 'timeline' | 'run' | 'decision' | 'budget';

type DocStatus =
  | 'draft' | 'saved' | 'live' | 'stale' | 'running' | 'verified' | 'failed'
  | 'blocked' | 'unavailable' | 'degraded' | 'promotion-pending' | 'promoted' | 'rolled-back';

interface FocusDoc {
  v: 1;
  id: string;                  // ULID, lowercase
  kind: DocKind;
  title: string;
  subtitle?: string;           // mono value line (ids, paths, counts)
  status: DocStatus;
  reason?: string;             // required when status is unavailable/degraded/blocked/failed
  epistemic?: 'observation'|'assumption'|'inference'|'unverified'|'contradicted';
  version: number;             // increments per snapshot on the focus channel
  dirty?: { fields: number; savedAt?: number };   // editor state (client-owned)
  source: { kind: 'chat'|'nav'|'palette'|'link'|'stream'; refId?: string };
  capabilities: DocCapability[];  // what this doc supports: 'edit'|'export'|'history'|'provenance'|'focus-chat'|'refresh'|'live'
  snapshot: unknown;           // kind-specific payload (one of the *DocSnapshot types below)
  provenance?: ProvenanceStrip;   // event count, trace id, created/updated actors
}
```

The **focus channel** (`GET /api/focus/{sessionId}/stream?from=<version>`, `09` §3) delivers
`{ docId, version, snapshot }` frames for live documents; static documents arrive via
`GET /api/focus/{docId}`. The client keeps `doc.version` monotonic and ignores stale frames (the same
dedup discipline as the transcript).

### 2.1 Registry and renderers

`frontend/src/components/docs/registry.ts` maps `DocKind → { Viewer, Editor?, DefaultActions, IconChipKind, Tone }`.
Adding a kind = one registry entry + renderer components + one catalog row. The registry drives pane
tabs, deep links (`/focus/:docId`), and the "Open with…" menu.

---

## 3. Viewer / editor modes

| Mode | Meaning | Who enables it |
|---|---|---|
| **View** | Read-only rendering of the snapshot + status + provenance | Default for every doc |
| **Edit** | Form/editor surface bound to a **draft** (client + server draft store) | Only when `capabilities` includes `edit` and the doc isn't `live` |
| **Diff** | Side-by-side or unified diff of draft vs last-saved (or version A vs B) | Any doc with a draft or ≥2 versions in history |
| **Raw** | The underlying machine form: JSON / Turtle / SEM / log lines in `CodeCard` | Every doc (auditor affordance) |

Mechanics:

- `⌘E` toggles View/Edit; toggling with unsaved edits prompts (Save draft / Discard / Cancel).
- Editors are **per-kind** (§6); shared pieces are the field kit (`04` §4.7) and `RecordForm`.
- **Autosave**: debounce 5s → `PUT /api/focus/{id}/draft`; footer chip shows `saving… / saved / offline`.
- **Save is not "write production"**: see §5.5 — production-affecting saves become change sets.
- **History drawer**: version list (mono v#, time, actor, source chip), each viewable as a read-only
  doc instance; Diff against current or any pair.

---

## 4. Status semantics (the "current document status" contract)

| Status | Chip (`02` §3.2) | Extra chrome | Typical docs |
|---|---|---|---|
| `draft` | slate pencil | footer `N unsaved fields` | edited prose/form |
| `saved` | slate check | footer `saved 2m ago` | all |
| `live` | cyan pulsing | header right shows `last frame 0.4s ago`, body auto-updates | log, chart, run |
| `stale` | amber hourglass | corner ribbon + `Refresh` action (source changed since snapshot) | any |
| `running` | violet spinner | `StageTicker` in header | run, sandbox, loop |
| `verified` | green shield | `ProvenanceStrip` shows verify trace | proof, gate |
| `failed` / `blocked` | red octagon | `reason` banner + `Open logs` | verdict docs |
| `unavailable` | slate plug-off | **reason string required**, shown as `InlineNote` in body | components without runtime/provider |
| `degraded` | amber triangle | `reason` + "what still works" list | partial features |
| `promotion-pending` | amber hourglass | action `Review gate` | change set docs |
| `promoted` | green up-circle | lineage link to promotion event | modules, designs |
| `rolled-back` | red undo | lineage link to rollback event | modules, actions |

Status changes animate the chip (120ms crossfade) and **announce politely** (`aria-live`), and are
recorded to the UI log. The status is also mirrored on the pane tab (dot) and in the sidebar badges.

---

## 5. Document kinds and their surfaces

### 5.1 `prose` — documents and notes
Markdown (remark/rehype + KaTeX + sanitize pipeline from the reference), edit mode with split
preview and a formatting toolbar; YAML-ish front matter rendered as `KeyValueGrid`; epistemic badges on
paragraphs carrying claims; export md/pdf(export via print stylesheet).

### 5.2 `form` — structured records
`RecordForm` over kernel records (being profile, goal, claim, memory entry, tool descriptor). Header
status shows validation state; Save flow per §5.5. Live validation uses zod mirrors of kernel schemas.

### 5.3 `code` — SEM, Turtle/RDF, JSON, Rust
Monaco with language packs (turtle/xml/json/rust/sem), read-only by default, gutter line numbers,
`SEM` gets structural indent + highlight (the `SemViewer` behavior, promoted to a document mode with
copy/download). Edit mode for module docs routes through change sets.

### 5.4 `diff` — change sets and version pairs
Split/unified toggle, word-diff highlighting (amber/red/green per `02`), file tree left rail (tree
engine, `06`), per-hunk comment pins stored as draft annotations, "Send to chat" on any selection.
Status mirrors the change set's gate state (`draft → sandbox-verified → promotion-pending → promoted /
rejected / rolled-back`).

### 5.5 Save-as-change-set (the promotion rule in the UI)
When an edit touches **production-affecting** objects (modules, design docs, policy genome, production
tree), the Save button becomes **"Save as change set…"** and opens `WizardForm`: (1) summary + typed
kind (`ChangeSetKind`), (2) affected files/objects preview as a diff, (3) reason field (required,
min 10 chars), then `changeset new` is issued and the doc status flips to `draft (cs_01H…)`. Sandbox →
gate → promote continue from the change set doc or the chat ops (`08` §4.7). Ledger writes (memory,
goals, claims) stay one-click Save — they are append-only events by design.

### 5.6 `run` — a loop/episode/sandbox run
Composition doc: header `StageTicker` (10 stages), body tabs `Summary | Timeline | Log | Artifacts | Budget`,
live status (`running`/`verified`/`failed`), per-stage duration bar chart (cyan→violet), artifact tiles
(open as new doc tabs), and controls (Pause/Resume/Replay via ops). The flagship "graphically rich"
document; every stage row is a `FeedCard` timeline entry with mini-metrics.

### 5.7 `chart` — dashboards and single charts
- **Dashboard mode**: responsive card grid (1/2/3 columns) of `ChartCard`s with a layout editor
  (drag to reorder, resize handles), range selector (1h/24h/7d/30d/all + custom), refresh cadence for
  live dashboards (5s/30s/off).
- **Single-chart mode**: large chart + series toggles + annotations + threshold editing + data table
  fallback + export menu (PNG/SVG/CSV).
- **The MetaMind chart catalog** (each is a registered chart recipe in `components/charts/`):

| Recipe | Type | Series / marks | Source |
|---|---|---|---|
| `self_model_divergence` | line + bands | Actual / Model / Ideal over runs | `self-model report` |
| `budget_burn` | stacked area + threshold bands | 4 budgets over runs | `budget show` history |
| `calibration_curve` | line + diagonal ref | predicted vs observed, Brier inset | `calibration fit` |
| `firewall_signals` | grouped bar | 8 signals, precision/recall overlays | `firewall eval` |
| `risk_octagon` | radar | 8 risk measures vs reference | `risk analyze` |
| `memory_growth` | area + event pins | memories, episodes, mistakes over time | memory stats |
| `llm_cache` | line + gauge tile | hit rate, tokens, spend | `llm stats` |
| `loop_throughput` | bar + line | stage durations per run | run events |
| `policy_fitness` | multi-line | fitness vector dims per policy version | `policy fitness` |
| `debt_trend` | stacked bar | debt kinds over scans | `debt scan` |

### 5.8 `log` — the log document (virtualized stream)
- **VirtualTable** over the log streams (`system`, `ui`, `trace`), 1M rows, severity coloring
  (trace slate / info cyan / warn amber / error red), structured fields expandable as `KeyValueGrid`.
- Toolbar: stream select, severity pills, regex search with match count + jump, filter chips
  (run_id, trace_id, component, span), `Follow tail` toggle (auto-pause on scroll-up, "N new lines" pill
  to resume), bookmarks, export selection.
- Row click → detail drawer with full JSON, GenAI semconv fields grouped, and `Open trace` (`logs trace`
  equivalent: a `timeline` doc of the trace).
- The live tail is a focus-channel subscription (`09` §3); offline chunks page in via cursor API.

### 5.9 `table`, `timeline`, `decision`, `budget`, `proof`, `mindmap`, `tree`
- `table`: the generic `DataTable` document over any queryable domain set.
- `timeline`: vertical event timeline (time gutter, icon chips, durations as inline bars) used for
  traces, replay views, and audit walks; zoom levels (run / stage / event).
- `decision`: a `decide` record — question, core used, verdict card with probability bar(s), evidence
  list with epistemic badges, alternatives considered; export as prose.
- `budget`: the four-budget instrument: 4 `GaugeArc`s + burn chart + ledger table + "what is left" list.
- `proof`: `ProofCard` composition with step navigation and copy-as-latex.
- `tree` / `mindmap`: thin documents embedding `06`/`07` viewers with their own toolbars (full-screen
  affordance to `/map/:id`).

---

## 6. Live documents and performance

- Live docs subscribe to the focus channel with `?from=lastVersion`; frames apply via a pure
  `applyDocFrame` reducer; the body only re-renders changed regions (chart data slices, log append).
- Chart live cadence batches frames at 1Hz (charts), logs render append-only with virtualization.
- Screenshot/exports of live docs capture the current frame + a mono caption line (time, version, source).

---

## 7. Provenance strip

Every document footer carries a `ProvenanceStrip`: `events 42 · trace 01H8… · created by loop_run ·
updated 4m ago · [View lineage]` — clicking opens the **provenance tree** (`06` §5.8) focused on this
doc's write chain. This is the UI face of the kernel's event-sourcing: every artifact can always answer
"where did you come from?".

---

## 8. Export

`Export ▾` menu per kind: md / json / csv / png / svg / ttl / pdf(print) — generated client-side where
possible (json/csv/png/svg), server-side for large ones (`GET /api/focus/{id}/export?format=`).
Exports embed a mono caption (doc id, version, time, console version) for audit bundles; `Export all
open tabs` builds a zip (audit bundle flow, `00` §6 Flow D).

---

## 9. Chat ⇄ focus protocol

| Direction | Trigger | Effect |
|---|---|---|
| Chat → doc | Op result carries `focus: { docId }` / user clicks an op card "Open" | Left pane opens/refreshes the doc (new tab or replace per preference), status chip animates |
| Chat → doc | User clicks a chart/table/tree **block** in a message ("Open in pane") | Same, with source chip `from chat` |
| Doc → chat | `Focus in chat` action / select text → `Ask about this` context menu | Composer gains `@doc:…#selection` reference chip; send includes the excerpt |
| Doc → chat | Any doc's `Run command…` action | Sends `/command` to chat and focuses the chat pane |
| Any | Drag doc tab / tree node onto composer | Inserts `@ref` chip |

The composer's `@`-autocomplete lists open documents, pinned docs, and recent artifacts; references
render as `KindBadge` chips with title + id, and the referenced snapshot is attached server-side at send
time (so the kernel sees exactly what the user saw).

---

## 10. Accessibility & keyboard for the pane

- Pane is a `main` landmark; header/foot are `role="banner"/"contentinfo"` within it.
- Mode switch is a `segmented` radiogroup; tabs are `role="tablist"` with arrow-key movement.
- Every status change announces ("Document status: verified"). Live docs announce *transitions* only.
- Charts expose a "View as table" toggle (also auto for screen readers); logs use `role="log"` with
  throttled announcements when follow-tail is on.
- Full keymap: `⌘E` mode, `⌘S` save draft, `⌘G` quick-switch, `⌘W` close tab, `⌘⇧F` fullscreen.
