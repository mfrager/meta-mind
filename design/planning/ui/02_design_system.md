# 02 — Design system: *Structured Color*

> Parent: [`INDEX.md`](./INDEX.md) · Next: [`03_shell_and_layout.md`](./03_shell_and_layout.md)
> · This document is **normative**. Components that violate tokens, the radius policy, or the color
> taxonomy do not ship. The look: **colorful, business-like, professional, not over-done** — and
> **corner radii are deliberately restrained**.

---

## 1. Design language in one paragraph

Structure is drawn with **1px borders and tinted surfaces**, not shadows and not large radii. **Color is
semantic**: ten hues, each with exactly one meaning, applied to state chips, phase rails, icon chips,
chart series, and 3px accent edges — everything else is neutral. Type does the remaining "designed" work:
Geist for UI, Geist Mono for every value that is a value, and 10px uppercase micro-labels over data
blocks. The result should read like a well-made financial terminal or an aerospace console: dense with
real information, calm in the neutrals, precise in the accents, fast everywhere.

---

## 2. Tokens

All styling is Tailwind CSS v4 with CSS variables in `frontend/src/index.css` (`@theme inline` mapping,
`@custom-variant dark`, oklch values — the same mechanism rust_symbolic's `index.css` uses, extended).

### 2.1 Neutral ramp (cool gray, slight blue cast)

| Token | Light | Dark | Use |
|---|---|---|---|
| `--background` | `oklch(0.985 0.004 255)` | `oklch(0.16 0.008 255)` | App canvas |
| `--foreground` | `oklch(0.21 0.012 255)` | `oklch(0.96 0.004 255)` | Body text |
| `--card` | `oklch(1 0 0)` | `oklch(0.20 0.008 255)` | Surfaces (flat white, not translucent) |
| `--muted` | `oklch(0.955 0.005 255)` | `oklch(0.26 0.008 255)` | Subtle fills, table stripes |
| `--muted-foreground` | `oklch(0.48 0.012 255)` | `oklch(0.68 0.01 255)` | Secondary text |
| `--border` | `oklch(0.895 0.008 255)` | `oklch(1 0 0 / 11%)` | The primary structuring device |
| `--border-strong` | `oklch(0.80 0.012 255)` | `oklch(1 0 0 / 20%)` | Panel edges, input borders |
| `--input` | `oklch(0.895 0.008 255)` | `oklch(1 0 0 / 15%)` | Control borders |
| `--ring` | `oklch(0.58 0.16 258)` | `oklch(0.70 0.15 258)` | Focus rings (azure) |

`--card` is **opaque** (no glass). Translucency is reserved for overlays (popovers, dialogs) at 92% with
`backdrop-blur-sm` only.

### 2.2 Radius scale (see §5 for the policy)

| Token | Value | Use |
|---|---|---|
| `--radius-xs` | `2px` | Cards, panels, code blocks, table cells' containers, chips' squares |
| `--radius-sm` | `3px` | Small controls (icon buttons, tabs underline-none segments) |
| `--radius-md` | `4px` | Buttons, inputs, selects, menus, tooltips, popovers |
| `--radius-lg` | `6px` | Dialogs, drawers, large popovers |
| `--radius-bubble` | `8px` | Chat bubbles **only** |
| `--radius-pill` | `999px` | Badges, status chips, toggles, avatars **only** |

Tailwind mapping: `rounded-xs`, `rounded-sm`, `rounded-md`, `rounded-lg` cover 99% of usage.
`rounded-xl/2xl/3xl` are **removed from the theme** (§5.3).

### 2.3 Spacing, sizing, density

- 4px base grid: `--spacing` = 4px; paddings only in multiples (4/8/12/16/20/24/32).
- Two densities, switched by `data-density` on `<html>`: **comfortable** (row height 36px, control 32px,
  card padding 16px) and **compact** (row 28px, control 26px, card padding 12px). Tokens:
  `--row-h`, `--control-h`, `--card-pad`, `--gutter` (pane gap: 12px).
- Sidebar width 248px expanded / 56px icon rail. Top bar 44px. Status bar 24px. Composer min 56px.

### 2.4 Elevation (three levels, no more)

| Level | Recipe | Use |
|---|---|---|
| E0 | `border` only | Cards, panels, sections — the default |
| E1 | `shadow-[0_1px_2px_oklch(0.2_0.01_255/0.06)]` | Menus, popovers, tooltips, floating toolbars |
| E2 | `shadow-[0_8px_24px_oklch(0.2_0.01_255/0.12)]` | Dialogs, drawers, command palette, drag ghosts |

### 2.5 Motion

| Token | Value | Use |
|---|---|---|
| `--t-fast` | 120ms | Hover, chips, toggles |
| `--t-base` | 180ms | Accordion, tabs, dropdowns |
| `--t-slow` | 240ms | Drawer, dialog, pane resize settle |
| `--ease-out` | `cubic-bezier(0.16, 1, 0.3, 1)` | Entrances |
| `--ease-in-out` | `cubic-bezier(0.65, 0, 0.35, 1)` | Moves, resizes |

Rules: motion is **transform/opacity only**; `prefers-reduced-motion: reduce` zeroes all durations and
disables pulsing (replaced by a static dot + text). No parallax, no bouncing, no looping decorative
animation. The only permitted loops are *state* signals: processing pulse (1.6s), live-log dot (2s),
progress indeterminate bar.

---

## 3. Color taxonomy — the ten hues (the heart of the system)

Each hue has one semantic meaning across the whole app: chips, rails, icons, charts, tree branches, mind
map branches. Never use a hue for decoration; never use one hue's meaning with another hue's token.

| # | Hue | Name | Core (light) | Tint bg / border / text (light) | Means (use it for) |
|---|---|---|---|---|---|
| 1 | Azure | **primary** | `oklch(0.58 0.16 258)` | `azure-50/azure-200/azure-700` | Primary actions, selection, focus, links, user chat bubbles |
| 2 | Teal | **knowledge** | `oklch(0.62 0.11 190)` | teal ramp | Memory, learning, consolidation, procedures |
| 3 | Violet | **llm** | `oklch(0.56 0.16 300)` | violet ramp | LLM turns, prompts, tokens, cache, provider chips |
| 4 | Amber | **caution** | `oklch(0.76 0.14 72)` | amber ramp | Repair phase, assumptions, warnings, budgets < 30%, stale |
| 5 | Red | **destructive** | `oklch(0.58 0.19 27)` | red ramp | Failures, denials, firewall blocks, rollbacks, errors |
| 6 | Green | **verified** | `oklch(0.62 0.15 150)` | green ramp | Verified, promoted, passing, healthy, success |
| 7 | Cyan | **data** | `oklch(0.68 0.11 215)` | cyan ramp | Charts/series identity, datasets, metrics, logs info |
| 8 | Indigo | **tools** | `oklch(0.54 0.15 275)` | indigo ramp | Tool executions, actions, sandbox, MCP |
| 9 | Rose | **being** | `oklch(0.62 0.16 10)` | rose ramp | Identity, goals, affect, relationships, commitments |
| 10 | Slate | **neutral** | ramp of `--muted-foreground` | slate ramp | Structure, unknown, disabled, raw machine output |

Each hue ships a **10-step ramp** generated in oklch (steps 50/100/200/300/400/500/600/700/800/900),
declared in `@theme` as `--color-{hue}-{step}`. Dark mode keeps the *same hues* (identical hue angle),
raising lightness ~0.12 and dropping chroma ~15%.

### 3.1 Tinted surface recipe (the "graphically rich" workhorse)

For any hue `H` and element E: container `bg-H-50 border-H-200` (dark: `bg-H-950/40 border-H-800`),
micro-label `text-H-700` (dark: `H-300`), icon chip `bg-H-100 text-H-700`, left rail `border-l-[3px] border-H-500`.
That is the *only* sanctioned way to add color to a surface. **No gradients on surfaces.** Gradients are
allowed in exactly two places: chart area fills (10% → 0 alpha, vertical) and the empty-state hero art.

### 3.2 Status chip language (state before color)

Chips are pills (`rounded-pill`), 18px tall, 10px uppercase text, 600 weight, 0.04em tracking, with a
leading 6px dot or 12px icon. The **shape + icon always encodes state**, color reinforces it (a11y rule:
never color-only):

| Status | Icon | Hue | Where |
|---|---|---|---|
| `draft` | pencil-dot | slate | documents, change sets |
| `saved` | check | slate | documents |
| `live` | pulsing dot | cyan | log/chart docs, running streams |
| `queued` | clock | slate | operations |
| `running` | spinner | violet | LLM turns, sandbox runs |
| `verified` / `passing` | shield-check | green | proofs, tests, gates |
| `assumption` | dashed circle | amber | epistemic badges |
| `observation` | eye | cyan | epistemic badges |
| `inference` | branch | violet | epistemic badges |
| `unavailable` | plug-off + reason tooltip | slate | components with no runtime/provider |
| `degraded` | triangle-exclamation | amber | partial capability |
| `blocked` / `failed` | octagon-x | red | firewall blocks, failures |
| `repair` | wrench | amber | repair phase |
| `promoted` | arrow-up-circle | green | production writes |
| `promotion-pending` | hourglass | amber | change sets in gate |
| `rolled-back` | undo | red | modules/actions |
| `stale` | hourglass-half | amber | docs whose source changed |

### 3.3 Epistemic badges

A special chip family (used on chat claims, tables, document headers): `OBSERVATION` (cyan),
`ASSUMPTION` (amber), `INFERENCE` (violet), `UNVERIFIED` (slate), `CONTRADICTED` (red). Every assistant
factual statement in chat carries one (`08` §4.10); hovering shows evidence count and a "View lineage"
link that opens the provenance tree focused.

---

## 4. Typography

| Role | Family | Size / line | Weight / tracking | Use |
|---|---|---|---|---|
| Display | Geist Variable | 28/34 | 600 / -0.02em | Page titles (rare) |
| H1 | Geist | 20/26 | 600 / -0.01em | Document titles |
| H2 | Geist | 16/22 | 600 | Section headers, card titles |
| H3 | Geist | 13/18 | 600 | Sub-sections, table groups |
| Body | Geist | 13/19 | 400 | Default UI text (13px is the console baseline) |
| Small | Geist | 11/16 | 400 | Secondary copy, table meta |
| **Micro-label** | Geist | 10/12 | 600 / 0.06em uppercase | Labels over data blocks, column heads, phase names |
| Value | Geist Mono | 12/16 | 450 / tabular-nums | IDs, numbers, times, durations, counts |
| Code | Geist Mono | 12/18 | 400 | Code blocks, SEM, JSON, Turtle |
| Prose (docs/chat) | Geist | 14/22 | 400 | Markdown body text |

Rules: numbers are **always** mono + `tabular-nums`; ULIDs render as `01H8…9K2` (6+3 grouping in mono,
tooltip has full); timestamps show relative ("4m ago") with absolute in tooltip; ISO only in mono value
cells. Headings never use the accent hues (structure is neutral; color is for state).

---

## 5. Radius policy — the anti-roundness law

> **Do not overuse rounded corners.** This section is a hard rule, not guidance.

### 5.1 The rule

| Element class | Radius | Example |
|---|---|---|
| Surfaces: cards, panels, sidebars, sections, tables, code blocks, screenshots | **2px** (`rounded-xs`) | The default for anything that "holds" content |
| Controls: buttons, inputs, selects, menus, tooltips, popovers | **4px** (`rounded-md`) | Interactive affordances |
| Overlays: dialogs, drawers, command palette | **6px** (`rounded-lg`) | Floating layers |
| Chat bubbles | **8px** (`rounded-bubble`) | The only soft shape in the stream |
| Pills: badges, status chips, toggles, avatars, progress bars | **999px** (`rounded-pill`) | Only for chips/controls explicitly listed |

### 5.2 Forbidden

- `rounded-xl`, `rounded-2xl`, `rounded-3xl`, `rounded-full` **except** the pill allowlist above
  (avatars, badges, chips, toggle knobs, circular icon buttons, progress bars, dots).
- Nesting radii for style ("rounded card inside rounded card"). Inner elements step *down* one notch
  (card 2px → inner block 2px, never 6px inside 2px).
- Radii on images/screenshots (2px max), tables (square cells, 2px container), dividers, or rails.

### 5.3 Enforcement (concrete)

1. `frontend/src/index.css` `@theme` **omits** `--radius-xl/2xl/3xl/4xl`; `tailwind.config`-equivalent
   theme only defines `--radius-xs/sm/md/lg/bubble/pill`. Unknown radius utilities fail the build.
2. ESLint rule `no-restricted-syntax` on `className` string literals matching
   `rounded-(xl|2xl|3xl|4xl|full)` (allowlist file `frontend/.radius-allowlist.json` for pills/avatars).
3. The catalog page (`/dev/catalog`) renders a **Radius gallery** showing every allowed shape; visual
   diff tests (`10` §6.4) catch accidental additions.

### 5.4 The one-line summary

**Square-ish surfaces (2px), rounded controls (4px), one soft bubble (8px), pills only for chips.**

---

## 6. Charts and data graphics

- **Categorical series order** (fixed, every chart): cyan → azure → violet → teal → indigo → rose →
  green → amber → red → slate. Series colors are `H-500` light / `H-400` dark.
- **Grid**: horizontal gridlines only, `--border` at 60% opacity; axis lines `--border-strong`; no vertical
  gridlines except point-reference guides (dashed `--muted-foreground` 40%).
- **Legends**: micro-labels below-left; interactive toggles use pill chips with the series color dot.
- **Tooltips**: E1 shadow, 2px radius, mono values, micro-label keys, and a mini swatch column.
- **Thresholds**: budget/gauge charts draw threshold bands as 8% tinted fills (amber-500 for warn,
  red-500 for critical) with dashed edge lines and a micro-label ("30% floor").
- **Sparklines**: 1px stroke, no axes, 20% area fill; end-point dot in the series color; used inside
  tree rows, stat tiles, and chat op cards.
- **Gauges**: 270° arc, 8px stroke, `--muted` track, value in mono 16px centered, status color of §3.2.
- **Empty chart states**: axis ghost + micro-label ("no data in range"), never a blank box.

---

## 7. Iconography and icon chips

- lucide-react, 16px default (12px in chips/rows, 20px in nav), stroke 1.75, `currentColor`.
- **Icon chip** (the signature element): 24×24 (`rounded-xs`) tinted square `bg-H-100` with a 14px
  `text-H-700` icon — used on document headers, tree nodes, nav items, op cards, chat blocks. This is the
  main "graphically rich" device: every kind in the system gets a recognizable tinted square.
- Kind icon map (excerpt; full map in `04` §4.2): document=file-text (slate), chart=chart-line (cyan),
  log=scroll-text (indigo), tree=network (teal), mind-map=git-fork (rose), diff=git-compare (amber),
  proof=shield-check (green), run=activity (violet), memory=brain (teal), claim=scale (amber),
  goal=flag (rose), module=box (indigo), budget=gauge (amber).

---

## 8. Dark mode

- Class-based `.dark` variant (as rust_symbolic), driven by `next-themes`-style provider with
  `system | light | dark` and persisted preference.
- Same hue angles; lightness raised, chroma reduced (§3). Borders become white-alpha (§2.1).
- Charts get the dark series ramp automatically via `--chart-series-*` tokens; screenshot and Monaco
  documents switch to their dark themes; images get `brightness-95` in dark to avoid glare.
- The status bar and sidebar keep slightly *different* lightness from the canvas (sidebar 2% darker in
  dark, 1% lighter in light) so the three-zone shell stays legible without borders.

---

## 9. Accessibility (WCAG 2.2 AA as the floor)

- Text contrast ≥ 4.5:1 (≥ 3:1 for 10px bold micro-labels used on tinted fills — all tint recipes in
  §3.1 are pre-verified for AA; a unit test computes contrast for every `H-step` pair in the ramp).
- Focus: 2px `--ring` outline with 2px offset (`outline-none ring-2 ring-ring ring-offset-2`), visible
  in both themes; `:focus-visible` only.
- Every status is icon + text (+ color). Every chart has a table fallback (`05` §7.6) and an
  `aria-label` summary. Trees implement the WAI-ARIA `treegrid`/`treeview` patterns (`06` §3.2).
- Keyboard map (`03` §6) never traps focus; dialogs restore focus; the composer announces streaming
  state via `aria-live="polite"` (throttled to 1/s).
- `prefers-reduced-motion`, `prefers-contrast: more` (borders strengthen to `--border-strong`, tints
  flatten to pure surfaces).

---

## 10. Writing and microcopy (the tone of the product)

- Sentence case everywhere except micro-labels (uppercase).
- Verbs for buttons ("Run sandbox", "Promote change set"), never nouns ("Sandbox", "Promotion").
- State copy is specific: "Unavailable — no sandbox runtime installed (tier 2 requires Docker)" not
  "Something went wrong". The UI quotes the kernel's own `reason` strings verbatim where provided.
- Chat placeholder: "Ask the kernel — or press / for commands…". Empty states teach (one sentence + one
  action + one quiet illustration at 8% opacity).
