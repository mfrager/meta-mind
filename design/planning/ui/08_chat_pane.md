# 08 — The chat pane (right): a rich chat log

> Parent: [`INDEX.md`](./INDEX.md) · Next: [`09_api_and_streaming.md`](./09_api_and_streaming.md)
> · Δ vs rust_symbolic: the reference transcript (user/assistant/tool bubbles + collapsible tool cards)
> is the skeleton; the Console adds **typed content blocks**, per-operation card renderers, **structured
> input requests (button grids, image chooser, forms)**, phase rails, and transcript tooling.

---

## 1. Anatomy

```
┌─ ChatPane ───────────────────────────────────────────────────────┐
│ ┌ ChatHeader ─────────────────────────────────────────────────┐  │
│ │ [IconChip] Session title (editable)   [● running 12:04]     │  │
│ │ provider chip: local·qwen… · purpose chip · wire v1         │  │
│ │ [Search transcript] [Branch] [Export ▾] [Clear] [⋯] [⤢]    │  │
│ ├─────────────────────────────────────────────────────────────┤  │
│ │  Transcript (virtualized, grouped by turn)                  │  │
│ │   ┌ user bubble (azure, right) ──────────────┐              │  │
│ │   │ …plus attachment chips + @refs           │              │  │
│ │   └──────────────────────────────────────────┘              │  │
│ │   ┌ turn (phase-railed) ──────────────────────────────────┐ │  │
│ │   │ ▌PLAN  violet rail · "Planning…" · 0:07 · stop        │ │  │
│ │   │  narration text (markdown blocks)                      │ │  │
│ │   │  ┌ op card: decide ─────────────────────────────────┐  │ │  │
│ │   │  │ ◐ question · ▸ input · ▾ output · verdict bar    │  │ │  │
│ │   │  └──────────────────────────────────────────────────┘  │ │  │
│ │   │ ▌VERIFY green rail · verdict card · proof card         │ │  │
│ │   └────────────────────────────────────────────────────────┘ │  │
│ │   ┌ assistant final (bubble, prose + rich blocks) ┐          │  │
│ │   ┌ input request: ButtonGrid  [Approve][Reject][Defer] ┐   │  │
│ ├─────────────────────────────────────────────────────────────┤  │
│ │ ┌ Composer ───────────────────────────────────────────────┐ │  │
│ │ │ @ref chips · attachment chips · image thumbnails         │ │  │
│ │ │ textarea (autogrow) — "/" commands · "@" refs · "Ask…"   │ │  │
│ │ │ [ / ] [ @ ] [ 🖼 ] [ 📎 ] [⌘↵]   suggestions chips  [Send]│ │  │
│ │ └──────────────────────────────────────────────────────────┘ │  │
│ └─────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

---

## 2. Transcript model

### 2.1 Items (wire + client)

```ts
type TranscriptItem =
  | { kind: 'user'; id: string; content: string; blocks?: ContentBlock[];
      attachments?: Attachment[]; refs?: DocRef[]; timestamp: number }
  | { kind: 'turn'; id: string; phase: Phase; subsystem?: string;      // an assistant working turn
      blocks: ContentBlock[]; state: 'processing'|'complete'|'error';
      startedAt: number; finishedAt?: number; visible: boolean;
      isFinal?: boolean; error?: string; ops: OpCall[] }
  | { kind: 'op'; call: OpCall }                                       // flat op card (tool-only turns)
  | { kind: 'input_request'; id: string; request: InputRequest;         // §5
      answered?: InputAnswer; expiresAt?: number }
  | { kind: 'system'; id: string; level: 'info'|'warn'|'error';
      message: string; reason?: string; timestamp: number }
  | { kind: 'divider'; id: string; label: string };                     // date/run/session dividers
```

This is a strict superset of rust_symbolic's `ChatTranscriptItem` (user / assistant_turn / tool_call /
error) — the reducer keeps the reference's behaviors: sequence dedup, hidden empty turns, machine-only
suppression, run-state machine (`processing | reconnecting | completed | failed | detached`) — extended
with `input_request` lifecycle (§5.8) and per-turn `ops` grouping.

### 2.2 Turn hygiene (kept from the reference)

Turns appear at `llm_request_started`; tool-only turns with no final text **collapse after completion**
into their op cards (the empty narration bubble disappears); machine-only responses never show;
`final_response` renders as the assistant bubble; `failed` yields a `system` error item with the kernel
`reason`. Improvement Δ: collapsed turns leave a **one-line phase summary** (e.g. `✓ VERIFY · 3 ops ·
2.1s`) that expands back on click — nothing silently vanishes.

### 2.3 Content blocks (the richness layer)

```ts
type ContentBlock =
  | { type: 'text'; markdown: string }                    // remark/rehype+KaTeX+sanitize
  | { type: 'code'; language: string; code: string; caption?: string }
  | { type: 'sem'; sem: string }                          // SemViewer: indent + highlight + copy
  | { type: 'math'; latex: string; display: boolean }
  | { type: 'table'; columns: ColumnSpec[]; rows: Row[]; footnote?: string }
  | { type: 'chart'; recipe: string; data: unknown; range?: string }   // §4.6 recipes
  | { type: 'json'; value: unknown; open?: boolean }      // JsonViewer (collapsible tree)
  | { type: 'tree'; adapter: string; root: unknown; depth?: number }   // embeds 06 outline viewer
  | { type: 'mindmap'; spec: MindMapSpec; depth?: number }// embeds 07 mini-map
  | { type: 'diff'; hunks: DiffHunk[]; mode: 'split'|'unified' }
  | { type: 'proof'; steps: ProofStep[]; qed: boolean }
  | { type: 'verdict'; verdict: 'pass'|'fail'|'blocked'|'degraded'|'unavailable';
      reason: string; evidence?: Evidence[] }
  | { type: 'meters'; meters: MeterSpec[] }               // budgets/gauges row
  | { type: 'timeline'; events: TimelineEvent[] }         // run stages / traces
  | { type: 'images'; images: ImageRef[]; layout: 'grid'|'carousel' }
  | { type: 'buttons'; requestId: string; grid: ButtonGridSpec }  // §5.3 (rendered from input_request)
  | { type: 'form'; requestId: string; schema: FormSchema }        // §5.5
  | { type: 'citation'; refs: DocRef[]; note?: string }   // epistemic citations
  | { type: 'progress'; label: string; value: number; max: number }
  | { type: 'diagram'; mermaid: string };                 // lazy-loaded Mermaid
```

Blocks are **sanitized, pure data**; each has a renderer in `components/chat/blocks/` and a catalog
entry (`04` §5). Every block has hover actions: Copy · Open in pane · Send to chat · Export.

---

## 3. Turn lifecycle and phase rails

### 3.1 Phase mapping (Metamind loop vocabulary)

| Phase | Rail color | Micro-label | What streams in this phase |
|---|---|---|---|
| `plan` | violet | PLAN | intent parsing, chosen operation sequence |
| `act` | indigo | ACT | op cards executing (tools, writes) |
| `verify` | green | VERIFY | proofs, verifications, gate results |
| `repair` | amber | REPAIR | repair loops (LLM structured-output repair, sandbox fixes) |
| `final` | azure | FINAL | the final response bubble |

The rail is a 3px vertical bar on the turn container's left edge, segmented when phases change within
one turn (a turn can be PLAN→ACT→VERIFY). Each segment has a hover tooltip with duration. This is the
direct descendant of rust_symbolic's phase labels, made structural.

### 3.2 Streaming affordances
- Narration text streams token-wise into its `text` block; blocks *materialize* in order (skeleton at
  first frame).
- Elapsed `m:ss` mono timer per turn (reference behavior), plus per-op durations on cards.
- **Stop** button (sends cancel; the kernel's bounded cancel semantics), **Regenerate** (new attempt),
  **Branch** (fork the session from this turn into a new session id).
- Reconnecting: amber `system` row "Reconnecting to the backend… attempt 2" (auto-retry countdown);
  detached: `system` row with "Resync" action that replays from `lastSeq+1`.

---

## 4. Operation cards (the chat workhorse)

Every kernel operation renders as an `OpCard` — the generalization of rust_symbolic's `ToolCallCard`,
with a **renderer registry per operation**.

### 4.1 Anatomy
Header: status icon (spinning dashed / green check / red ×) · op name (mono) · purpose chip · subsystem
chip · right-aligned status + duration · chevron. Body (expandable): `Input` (KeyValueGrid + specialized
widgets) · `Output` (specialized renderer + raw JSON in `Raw` tab) · footer: provenance strip
(`event 01H8… · [View lineage]`) + actions (Open in pane, Copy, Send to chat).

Special case kept from the reference: an op whose invocation succeeded but whose **verdict failed**
(e.g. `model_verify correct=false`) renders header red with label "verification failed" — a *failed
verdict* is not the same as a *failed call*.

### 4.2 The op renderer registry (excerpt — one entry per CLI capability)

| Op | Input widgets | Output renderer |
|---|---|---|
| `memory_add` / `epistemic ingest` | RecordForm summary | receipt chip + provenance link |
| `memory recall` | query + k slider | `ListCard` of hits with score meters + epistemic badges |
| `decide` | question + core select (`rules|local|hosted`) | **verdict card**: probability bar(s), options table, evidence citations, core chip |
| `firewall eval` | corpus select + threshold fields | verdict card + 8 **signal chips** (green/amber/red) + precision/recall meters |
| `risk analyze` | target select | **risk octagon** radar (chart recipe) + top-3 measures table |
| `tool run` | tool select + args form | status + output (`CodeBlock`/files) + rollback action |
| `action rollback` | action id | before/after hash chips + byte-identical ✓ chip |
| `verify run` | obligation select | **ProofCard** (steps, QED chip) |
| `episode run` | episode spec | timeline block (stages) + value/budget meters |
| `sandbox run` | change-set id | progress (build/test/audit) + diff summary + log links |
| `promote` / `reject` | reason form (required) | verdict card + lineage chain (changeset→version→module) |
| `budget show` | — | 4 `GaugeArc`s + remaining list + burn sparkline |
| `loop run` | goal select | **StageTicker** + live timeline + artifacts tiles + final self-model chart |
| `self-model report` | — | Actual/Model/Ideal line chart + divergence table |
| `calibration fit`/`conformal` | corpus select | calibration curve + Brier table + threshold chips |
| `logs tail/trace` | stream + filter | mini log rows (Open as log doc) / trace timeline |
| `audit production-tree` | — | verdict card + tree diff |
| `module load/rollback` | module select | status chips + loaded-version ticker |
| `codex scan/verify/diff` | scope select | drift table + verdict card |
| `pi run` | task spec | progress + artifact tiles |

Unregistered ops fall back to the generic renderer (KeyValueGrid input + `json` output) — so the chat is
complete before the registry is.

### 4.3 Op grouping and collapse
Ops within a turn nest under the phase rail; > 4 ops of one kind auto-collapse into a group card
(`flush` accordion) with counts. Completed groups show a one-line summary ("✓ 6 memory writes · 210ms").

### 4.4 Rich outputs ("outputs of all kinds")
The chat renders, end-to-end: prose (markdown+math), code (highlighted, copy, wrap toggle), SEM
(structural indent + highlight), JSON (collapsible tree), tables (sortable inline), charts (all §`05`
§5.7 recipes, hover tooltips, "Open in pane"), meters/gauges, timelines, proofs, verdicts, diffs
(split/unified inline), trees (outline embeds), mind maps (mini embeds), images (grid/carousel +
lightbox), diagrams (Mermaid), citations, progress bars, and mixed compositions in one message.

### 4.5 Provenance and epistemic stamps (Metamind signature)
Every output block that states a fact carries an **epistemic badge** (`02` §3.3) and, where the kernel
recorded it, a lineage link. Assistant prose gets margin chips on claim sentences (detected via the
kernel's structured results — the UI never *invents* a badge: no provenance ⇒ no badge, and the badge
tooltip says "no recorded provenance").

---

## 5. The rich input set

### 5.1 Composer
- Autogrowing textarea (min 56px, max 40vh), `⌘↵` sends, `Enter` newline, draft persisted per session.
- **Slash commands**: typing `/` opens the command menu (cmdk) — the 42 CLI commands + chat-native
  commands (`/map`, `/focus`, `/export`, `/clear`, `/branch`, `/help`). Picking one with args opens its
  `CommandForm` inline **above the textarea** (fields replace the arg string; Send runs it).
- **`@` references**: documents, tree nodes, map nodes, transcript selections, files — inserted as
  `KindBadge` chips; the attached snapshot is pinned at send time (`05` §9).
- **Attachment chips**: files (text/json/csv/images) with size + type validation (`max_upload_mb` from
  config); text previews inline before send.
- Toolbar: `/` commands · `@` refs · **image chooser** (§5.4) · file attach · template menu (guided
  tour prompts) · mic is absent (non-goal) · Send/Stop (Stop while running).
- **Suggestion chips** (below the composer after each final response): 2–5 system-provided next steps
  ("Open the run map", "Why did stage 6 slow down?", "Promote cs_01H…") — clicking sends the chip as a
  user message. Chips come from the kernel's suggestions field, not client invention.

### 5.2 Button grids — `ButtonGridSpec` (a first-class input)

Requested explicitly: *inputs like button grids*. A `buttons` block renders an actionable grid inside
the transcript; the kernel (or UI) posts an `input_request` of kind `buttons`:

```ts
interface ButtonGridSpec {
  columns: 1|2|3|4;
  variant: 'confirm'|'options'|'multi'|'scale'|'nav';
  prompt?: string;                     // shown above the grid (markdown)
  options: { label: string; value: string; icon?: IconChipKind; tone?: Tone;
             description?: string; disabled?: boolean; shortcut?: string }[];
  multi?: { min: number; max: number };// for variant 'multi'
  scale?: { min: number; max: number; minLabel?: string; maxLabel?: number };
  default?: string | string[];
  timeoutMs?: number;                  // default action on timeout (recorded as such)
  allowOther?: boolean;                // adds a final "Other…" text button
}
```

Variants and looks:

| Variant | Layout | Use |
|---|---|---|
| `confirm` | 2 columns, one positive (green outline), one negative (red outline), optional third neutral | Approve/Reject/Defer, Run/Cancel, Promote/Reject |
| `options` | 1–4 columns of equal-weight buttons (icon chip + label + description line) | "Which core?" (`rules|local|hosted`), artifact choices, next-step pickers |
| `multi` | checkbox-style toggle buttons with counter + "Done" | select datasets, pick modules, choose signals to inspect |
| `scale` | N equal segments (radio-row look) with min/max labels | confidence, severity, usefulness ratings (feeds calibration labels!) |
| `nav` | 3 columns Back / Next / Skip with key hints | wizard-style guided flows in chat |

Behavior: keyboard grid (arrows + Enter), pending state until answered, answered state collapses to a
**answer echo** ("✓ Promote — with reason, 14:22") and the grid becomes read-only; expired grids render
dimmed with the default that fired; every answer is a recorded input event (audit trail) and can be
re-opened from the transcript.

### 5.3 Ratings and signatures
`scale` grids double as **label capture** for `calibrate` flows (outcome scoring) — answering writes the
label through the op pipeline. A `confirm` grid with `variant:'confirm'` + `danger:true` paints red
outlines and requires a reason field for irreversible actions (`gc --apply`, rollback).

### 5.4 Image chooser — `ImageChooser` (a first-class input)

Requested explicitly: *inputs like image choosers*. Used standalone in the composer and as `ImageField`
in forms:

```ts
interface ImageChooserSpec {
  sources: ('upload'|'paste'|'drag'|'recent'|'artifacts')[];
  max: number;                    // default 6
  maxMb: number;                  // from [ui].max_upload_mb
  accept: ('png'|'jpeg'|'webp'|'svg')[];
  captions: boolean;              // per-image caption inputs
  annotate?: boolean;             // rectangle/arrow/mask annotation overlay
}
```

- **Sources**: file picker (multi), drag-and-drop onto the pane (drop zone outline in azure),
  paste from clipboard (⌘V anywhere in the pane), **Recent** (last 20 images sent), **From artifacts**
  (chart PNG exports, screenshots, run captures — browsable lightbox picker with kind chips).
- **Thumbnails**: 72px, 2px radius, kind corner badge, per-image caption input, remove ×, drag to
  reorder, click opens the lightbox with annotate tools (rect/arrow/blur-mask, stored as SVG overlay —
  originals untouched).
- **Upload state**: per-thumb progress ring; validation errors inline (too large, wrong type, too many).
- **Sending**: images attach to the user message (chips + thumbnail strip) and flow to the kernel as
  multimodal parts; if the configured provider cannot take images, the composer shows an amber
  `degraded` note *before* sending ("this route records images as attachments; the model will see
  captions only") — honest degradation (`00` §4.3).
- Chat images render as `images` blocks (grid/carousel) with lightbox, zoom, and "Open as document".

### 5.5 Chat forms — `form` blocks
Parameterized commands and record creation can post a `form` input request (schema = the same
`CommandForm`/`RecordForm` field kit). Validation is inline; Send submits the structured payload as the
answer event. Example: `/being goal add` → form (title, timescale, success criteria, priority slider) →
answer → op card `being.goal_add` → goal tree refresh + focus offer.

### 5.6 Text and voice of the input surface
Placeholder: "Ask the kernel — or press / for commands…". Hint row under the textarea:
`⌘↵ send · / commands · @ attach context · ⇧↵ newline`. When a run is active the Send becomes **Stop**;
composer remains editable (queued messages post after completion, shown as queued chips).

### 5.7 Input request lifecycle (server-driven)
```
kernel → { type: 'input_request', id, kind, spec, expiresAt? }
UI     → renders buttons/form block (pending)
user   → answer event { type: 'input_answer', id, answer, at }  (also echoed locally)
kernel → continues with the answer (recorded in the event log)
```
Pending requests pin to the **bottom of the transcript** (a "1 response needed" pill) if scrolled away;
expired ones show `default fired` or `timed out` with the reason. Answers are part of the audit trail.

### 5.8 Rich output ↔ input pairing examples
- `decide` → verdict card + `buttons` grid "Accept verdict? [Accept][Challenge][Re-run with core…]".
- `sandbox run` fails → verdict card + `buttons` "Retry / Open log / Send to repair" + optional `form`
  (reason).
- `budget show` → meters + `scale` grid "How constrained should the next loop be?".
- `risk analyze` → octagon + `multi` grid "Which signals should the firewall weight higher?".

---

## 6. Transcript tooling

- **Search** (header): across user/assistant/ops/answers with match highlighting and `F3` walk;
  results peek the item in a right drawer.
- **Export**: markdown (prose + code, op summaries), JSON (full wire items incl. blocks and answers),
  and "audit bundle" (md + referenced docs zipped).
- **Branch/fork**: any turn → new session with shared prefix (server-side copy of the event range).
- **Session list** (in the sidebar Work section): title, last activity, run status dot, item count;
  rename/pin/delete (delete archives, never destroys — the event log is immutable).
- **Date/run dividers** and turn group headers keep long sessions scannable.

---

## 7. Performance

- Transcript virtualizes at the item level (blocks lazy-render on expand for > 200 items);
  1,000-item sessions scroll at 60fps; streaming updates mutate one item in the store (memoized rows).
- Markdown renders in chunks (streaming parser keeps a rolling block index); KaTeX renders per-block.
- Images lazy-load with `loading="lazy"` + blur placeholders; lightbox uses the shared overlay layer.

---

## 8. Accessibility

- Transcript is `role="log"` with throttled `aria-live="polite"` for final text and state transitions.
- Op cards are `role="group"` with `aria-expanded`; status is icon + text (never color alone).
- Button grids are `role="radiogroup"`/`group` with roving tabindex and arrow keys; forms have proper
  labels/errors; image thumbs have alt text (captions) and the chooser announces upload state.
- The composer exposes its hints via `aria-describedby`; Send/Stop announce state changes.
