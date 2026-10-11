# 03 — Shell and layout: the menu, the two panes, and the furniture

> Parent: [`INDEX.md`](./INDEX.md) · Next: [`04_component_library.md`](./04_component_library.md)
> · Δ vs rust_symbolic: the reference has a fixed sidebar + outlet + status bar; the Console keeps that
> skeleton but adds a resizable, swappable **two-pane workspace** and a full command surface.

---

## 1. The three-zone shell

```
┌────────────────────────────────────────────────────────────────────────────────────┐
│ AppSidebar │            TopBar (breadcrumbs · run status · ⌘K · theme)              │
│  (menu,    ├───────────────────────────────────────────────────────────────────────┤
│  not a     │ ┌─── DocumentPane (left) ────┐ │ ┌─── ChatPane (right) ─────────────┐ │
│  pane)     │ │ DocHeader: icon·title·chips │ │ │ ChatHeader: session·provider·ops │ │
│            │ │ Mode: View|Edit|Diff|Raw   │ │ ├────────────────────────────────────┤ │
│  Work      │ ├────────────────────────────┤ │ │  Transcript (virtualized)         │ │
│  Explore   │ │                            │ │ │  · user bubbles                   │ │
│  Build     │ │  Document body:            │ │ │  · phase-railed turns             │ │
│  Operate   │ │  viewer / editor / chart / │s│ │  · op cards                       │ │
│  Admin     │ │  log / tree / mind map /   │e│ │  · rich output blocks             │ │
│            │ │  diff / proof / table      │p│ │  · input requests (button grids)  │ │
│            │ │                            │a│ ├────────────────────────────────────┤ │
│            │ ├────────────────────────────┤r│ │ Composer: / cmds · @refs · images │ │
│            │ │ DocFooter: status·save·exp │a│ │ [Send / Stop]  suggestions        │ │
│            │ └────────────────────────────┘t│ └────────────────────────────────────┘ │
│            │                               o│                                     │
├────────────┴───────────────────────────────r┴─────────────────────────────────────┤
│ StatusBar: ● healthy · event head · budgets ▮▮▮ · cache 74% · run stage 6/10 · 14:22 │
└────────────────────────────────────────────────────────────────────────────────────┘
```

**The 2-pane system excludes the left menu** as specified: the workspace region right of the sidebar is
split into exactly two panes — **Document pane (left)** and **Chat pane (right)**. The menu, top bar, and
status bar are shell furniture and do not count as panes.

---

## 2. Zone specs

### 2.1 AppSidebar (the left menu)

- Width 248px expanded / 56px icon rail; collapsible via the sidebar grip or `⌘B`; state persisted.
- Built on the shadcn `sidebar` block (rust_symbolic already ships `ui/sidebar.tsx`; we keep its
  primitives and replace the content model).
- **Sections** (micro-labels, uppercase): 

| Section | Items (icon chip per item) | Badges |
|---|---|---|
| **Work** | Overview · Workspace (chat) · Pinned documents · Drafts | draft count, pinned count |
| **Explore** | Being · Memory · Epistemics · Library · Ontology | memory count, contradiction count (amber) |
| **Build** | Change sets · Modules · Pi authoring · Regressions | pending promotions (amber), failing regressions (red) |
| **Operate** | Runs · Logs · Tools · Budgets · Doctor | active run (pulsing violet), budget warnings (amber) |
| **Admin** | Settings · Catalog (`/dev/catalog`) · About | wire version |

- Footer: kernel health chip (from `/api/health`), session token status, theme toggle, density toggle.
- Nav rows: 28px (comfortable 32px), icon chip 20px + label + right-aligned count badge; active row gets
  `bg-accent` + 2px azure left rail (`border-l-2`), **not** a rounded highlight.
- The bottom two sections can be hidden by preference; the tree workspaces (`06`) can pin any tree to
  the sidebar as a lightweight navigator (last 5 nodes + search).

### 2.2 TopBar (44px)

Left: breadcrumb of the focused document (`Work / Runs / loop_2026-10-10T14:22`), each crumb clickable.
Center: global search + ⌘K button ("Search or run a command… ⌘K").
Right: run-status pill (when a loop/episode runs: pulsing dot + stage name + elapsed), SSE connection
chip, notifications menu (toasts digest), theme + density + user menu (local operator).

### 2.3 StatusBar (24px, mono values)

Left→right: `● kernel healthy` (or amber `degraded: <reason>` / red) · `events 01H8…9K2` (event-log head,
click → logs doc) · `budgets` mini bars (4 segments, amber < 30%, red < 10%, click → budgets doc) ·
`llm cache 74%` · `run stage 6/10 (benchmark)` with a 10-segment stage ticker · `stream ✓ 0.4s ago` ·
clock. All chips are clickable and focus the relevant document. The status bar is the *persistent
instrumentation strip*; it never carries navigation.

---

## 3. The two-pane split

- `PanelGroup` (react-resizable-panels) with **direction horizontal**, gutter 4px (visual: 1px line +
  hover widen to 3px azure). Default ratio **55 / 45** (document / chat); user-draggable 25–75%.
- Sizes persist per workspace route in `usePaneStore` (localStorage), with `Reset layout` in the View menu.
- **Collapse**: each pane collapses to a 32px vertical strip showing its icon chip + micro-label rotated
  90° (vertical text), not to zero — the collapsed strip is clickable to restore.
- **Swap** (`⌘\`): exchanges the panes (chat left / document right) for chat-centric or doc-centric work.
- **Fullscreen focus** (`⌘⇧F` or the expand icon in either header): the active pane fills the workspace;
  the other pane remains alive (SSE stays connected) and its header shows a restore chip.
- **Pane tabs**: each pane hosts a tab strip *inside its header* when more than one document or more than
  one chat session is open (document tabs: icon chip + title + dirty dot + close ×; chat tabs: session
  name + run dot). Middle-click closes; drag reorders; overflow becomes a dropdown list.
- **Drop targets**: dragging a tree node / chat artifact chip onto a pane header focuses it there;
  dropping onto the chat composer inserts a reference (`@doc:…`); dropping onto the document pane opens
  a picker (Open / Replace / New tab).

---

## 4. Workspace lifecycle states

| State | Document pane | Chat pane |
|---|---|---|
| Fresh session | Empty state: hero art + "Focus a document or ask the kernel" + 4 starter cards | Empty state: capability grid (`08` §2.2) |
| Working | Live doc (log/chart/run) with `live` chip + streaming body | Transcript streaming, op cards filling |
| Detached | Doc freezes at last snapshot + `stale` chip + Retry/Reconnect | Detached banner (rust_symbolic copy, improved: auto-retry countdown + Resync button) |
| Read-only (exec lock) | Docs open read-only; editors show "console holds the write lock" banner | Writes blocked at composer with reason chip |

---

## 5. Command palette (⌘K)

Built on cmdk (as rust_symbolic's `command-palette.tsx`), extended into the primary power-user surface.

- **Sources merged with scoring**: commands (42 CLI subcommands surfaced as typed actions with arg
  forms), documents (open + recent + pinned), navigation (routes), chat sessions, and "Ask the kernel…"
  (free text → chat).
- Each command row: icon chip · title · micro-label category · keybinding hint · `--json` preview in a
  right-side peek when highlighted.
- Commands that need arguments open **inline arg forms** in the palette (`04` §4.15) — e.g.
  `firewall eval` shows corpus/threshold fields — then execute via the op pipeline and stream results
  into chat as an op card + focused document.
- Palette states: empty (recent + suggested), no-match ("No match — ask the kernel instead" → sends to
  chat), long-running (progress rows with spinner + elapsed, ESC detaches but does not cancel).
- Theming: E2 shadow, 6px radius, 2px card interior; group micro-labels; fuzzy highlight in azure.

---

## 6. Keyboard map

| Key | Action |
|---|---|
| `⌘K` / `Ctrl+K` | Command palette |
| `⌘L` | Focus composer |
| `⌘1` / `⌘2` | Focus document pane / chat pane |
| `⌘\` | Swap panes |
| `⌘B` | Toggle sidebar |
| `⌘⇧F` | Fullscreen active pane |
| `⌘E` | Toggle View/Edit mode of focused document |
| `⌘S` | Save draft (document) / send (composer, with Enter) |
| `⌘/` | Shortcut cheatsheet dialog (searchable) |
| `⌘G` | Document quick-switcher (MRU list) |
| `⌘D` | Duplicate document to new tab |
| `⌘W` | Close tab |
| `Esc` | Close overlay → clear selection → stop streaming (twice) |
| `j/k`, `Enter`, `Space` | Tree: navigate / open / select (`06` §3.2) |
| `f` | Mind map: zoom-to-fit (`07` §7) |
| `?` | Help (same as `⌘/` when not in an input) |

All bindings are contextual (never fire while typing in an input except `Esc` and `⌘K`), discoverable in
tooltips and the cheatsheet dialog, and rebindable in Settings (stored in `usePrefsStore`).

---

## 7. Focus and overlay management

- Landmarks: `<nav>` sidebar, `<header>` top bar, two `<main>` panes (`aria-label`ed "Document pane" /
  "Chat pane"), `<contentinfo>` status bar.
- Overlays (dialog, palette, drawer) use the E2 layer, trap focus, restore to the invoker, and pause the
  transcript's `aria-live` while open.
- The status bar and live docs use polite live regions only for state *transitions* (not ticks).

---

## 8. Responsive tiers

| Width | Behavior |
|---|---|
| ≥ 1280px | Full two-pane workspace + sidebar |
| 1024–1279px | Panes stack vertically (document above, chat below) with a tab switcher; sidebar → icon rail |
| 768–1023px | Single pane with `Docs | Chat` segmented switch; sidebar in a sheet |
| < 768px | **Monitoring tier**: read-only run/status/log views, chat transcript + text composer; editors and trees hidden with an explanatory banner ("The console's editing surfaces need a wider window") |

Mobile is deliberately a monitoring tier (non-goal in `00` §3): the shell remains the same tokens and
components at smaller sizes; no separate mobile design language.

---

## 9. Pane furniture details (the small print)

- **Gutters** show a 1px `--border` line; on hover a 3px azure line; while dragging, a 2px azure line +
  `data-resizing` freezes child re-layout (children render at a fixed size until release → no jitter).
- **Pane headers** are 32px: icon chip · title (H3) · status chips · spacer · icon actions (mode switch,
  tabs menu, more menu) · collapse · expand. The header's left 3px edge carries the document kind color
  (§3.1 recipe) so the pane's content kind is visible at a glance.
- **Scrollbars** are thin (6px), `--muted-foreground` at 40% thumb, always visible in log/transcript
  panes (they are data surfaces), auto-hiding elsewhere.
- **Empty pane** art: 200px monochrome isometric grid illustration at 8% opacity + one sentence + one
  primary action + two quiet links.
