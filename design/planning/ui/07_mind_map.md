# 07 — The mind-map viewer

> Parent: [`INDEX.md`](./INDEX.md) · Next: [`08_chat_pane.md`](./08_chat_pane.md)
> · Δ vs rust_symbolic: the reference has no map at all; the Console ships a **high-quality, dual-engine
> mind-map viewer** — a custom radial tidy engine for thinking/teaching, and a React Flow graph engine
> for structure/audit — over the same adapters as the tree editors.

**Purpose.** Give the operator a *spatial* view of the being's identity and goals, the memory topic
space, the library, the ontology, run stage structure, decision option spaces, and change-set impact —
and let chat build and manipulate maps conversationally ("map the goals", "show me what touches this
claim").

---

## 1. The two engines

| Engine | Built on | Shape | Use |
|---|---|---|---|
| **Radial mind map** (default) | Custom SVG + `d3-hierarchy` (radial tidy tree) | Root center, branches outward, curved links | Being & goals, memory topics, library, teaching/summary maps, chat-built maps |
| **Graph view** | `@xyflow/react` (React Flow) + `d3-force` seeding | Free node-link, minimap, orthogonal/bezier edges | Provenance chains, ontology relations, decision evidence, impact maps |

A segmented switch flips engines **preserving camera and selection** (nodes map 1:1 via adapter ids).
A third read-only **outline** mode (nested lists) is the accessible fallback and the export format.

---

## 2. Data: `MindMapSpec`

```ts
interface MindMapSpec {
  id: string;
  title: string;
  rootId: string;
  nodes: MapNode[];             // { id, parentId, label, kind, tone?, icon?, badge?, status?,
                                //   note?, weight?, side?: 'left'|'right' (radial hint), payload? }
  edges?: MapEdge[];            // extra non-hierarchical relations (graph engine only)
                                // { id, from, to, relation: 'supports'|'refutes'|'depends'|'derived'|'related',
                                //   style?: 'solid'|'dashed' }
  layout: { engine: 'radial'|'graph'; spread: number; balance: boolean; seed?: number };
  theme?: 'auto'|'light'|'dark'|'contrast';
}
```

**Adapters** produce `MindMapSpec` from: `being` (identity → goals → subgoals), `memory` (topics →
episodes/mistakes/procedures), `library` (techniques/policies/frames/skills), `ontology` (classes →
properties), `run` (stages → artifacts → ops), `decision` (question → options → evidence),
`changeset` (change set → files → impacted modules), and **chat** (streamed node creation).

---

## 3. Radial engine (the quality bar)

### 3.1 Layout
- `d3-hierarchy.tree()` with `nodeSize([radialStep, ringGap])`, radial projection:
  angle ∝ subtree leaf count (weight-aware: `weight` scales leaf cost), radius ∝ depth
  (`ringGap = 120 * pow(0.82, depth)` compression for deep maps).
- **Side balancing**: roots split left/right for topic-style maps (`side` hint), or full 360° for
  single-topic maps; toggle in the layout toolbar.
- Re-layout is incremental: nodes keep polar coordinates and animate to new targets (`--t-slow`,
  transform-only), so streaming/edits never jump the whole map.
- Label collision: per-ring angular separation clamp + horizontal flip past the 90° meridian +
  leader-line offset for tight nodes.

### 3.2 Node language (graphically rich, token-bound)
- Root: 2px-radius card, tone-filled (`bg-{tone}-500`, white text), title (H2) + mono count.
- Branch nodes: tinted pill-free cards (`bg-{tone}-50 border-{tone}-200`, 2px radius) with icon chip,
  label (13px), optional mono value; status chips only when not `saved`/neutral.
- Leaves: compact rows (icon chip + label), 1px border.
- Edges: 2px cubic-bezier curves, **stroke = parent branch tone**, 70% opacity, 3px at hover;
  relation edges (graph) use dashed strokes with arrowheads and relation micro-labels.
- **Stable branch-hue rule**: sibling branches take hues from the categorical order
  (cyan→azure→violet→teal→indigo→rose→green→amber) by *stable hash of node id* — colors do not reshuffle
  between sessions or re-layouts; a branch's subtree inherits its hue with decreasing chroma by depth.
- Optional "pop" treatments kept in reserve (used sparingly per `04` §6): 10% tinted glow on the active
  path, animated dashed stroke while a run streams into the map, tiny sparkline badges on metric nodes.

### 3.3 Interaction model
- **Navigate**: drag to pan, wheel to zoom (cursor-anchored), pinch on touch, `f` zoom-to-fit,
  `0` reset, `+/−` step zoom, click node → select (2px azure outline + dim others to 60%),
  double-click → open payload doc in the left pane.
- **Inspect**: hover → `HoverCard` with `KeyValueGrid` + chips + "Open document"; idle hover previews a
  mini-branch (3 levels) in a 240px popover.
- **Focus+context**: click the ring edge or `Enter` on a node → **re-root** at that node (breadcrumb
  shows the path with clickable ancestors); `Esc` walks up one level; "Pin as second root" for compare.
- **Edit mode** (toggle in toolbar, or from chat): add child (`Tab`), add sibling (`Enter`), rename
  (inline input, `F2`), delete (`Del` with confirm), drag to reparent (subtree ghost + drop line),
  drop *external* items (tree nodes, chat `@refs`, files) to create linked nodes. All edits go through
  the same mutation/undo discipline as trees (`06` §4) and can require the change-set wizard.
- **Search** (`⌘F` within map): fuzzy with match ring animation, `F3` next, auto-pan to match,
  "dim non-matches" mode.
- **Selection tools**: rubber-band select (graph engine), subtree select, `Copy as outline`,
  `Send subtree to chat`, `Export selection`.
- **Live mode**: when sourced from a running op (e.g. chat mapping while the kernel works), new nodes
  arrive as `focus` frames and animate in along their edge; the map auto-pans to keep the growth point
  visible; a `live` chip + "N nodes added" pill appears.

### 3.4 Level-of-detail rendering
- Zoom buckets: `< 0.5×` renders labels only for depth ≤ 2 and dot-glyphs for the rest;
  `0.5–1×` all labels at 11px; `> 1×` full adornments (chips, values). LOD switches per *node*, not per
  map, so partially zoomed maps look right.
- Text rendered as SVG `text` with `paint-order: stroke` halo (background-colored 3px stroke) — readable
  over edges without plates.
- Above 2,000 visible nodes: offscreen culling (viewport + 20% margin) and edge batching into a single
  `<path>` per hue; above 5,000 nodes the toolbar offers "simplify" (collapse to depth 3 + density
  rings).

---

## 4. Graph engine (React Flow)

- Custom node types: `record-node` (icon chip + title + 2-line excerpt + chips), `cluster-node`
  (grouping hull with tinted 4% fill + micro-label), `pin-node` (annotated marker).
- Edge types: `relation-edge` (dashed/solid + arrow + relation micro-label), `chain-edge` (hash-chain
  style with chevrons for provenance), `impact-edge` (thickness ∝ impact score).
- Layout: ELK-style layered for DAGs (via `dagre`/`elkjs` — pick `elkjs` for quality, worker-threaded),
  `d3-force` for organic exploration; "re-layout" and "tidy" toolbar buttons.
- Built-in: minimap (tone-colored), controls (token-styled, 4px radius), background grid (dots at 8%
  opacity), edge focus dimming, multi-select + group drag, node pinning, screenshot export.

---

## 5. Themes and styling

- The map obeys `02` tokens exactly (light/dark/system; `contrast` theme flattens tints to white/black
  plates with thicker strokes).
- Toolbar (floating, E1, 4px radius): engine switch (segmented), layout menu (spread, balance, side,
  re-layout), depth stepper, filter chips (hide by kind/status), search, edit-mode toggle, theme menu,
  export menu, fullscreen (`⌘⇧F` in pane; `/map/:id` for full-bleed).
- Legend: bottom-left, collapsible, showing branch hue → domain meaning and edge styles.

---

## 6. Chat ⇄ map integration (the "similar process", improved)

- Chat op blocks can carry a `mindmap` block (`08` §3.3) rendering a live mini-map (max depth 2,
  click-through to focus the full map in the left pane).
- Chat commands: `/map <topic>`, `/map from <selection>`, `/map expand <node>`, `/map prune <node>` —
  each returns a map block and focuses the doc.
- While the kernel runs (loop/episode), an optional "trace map" mode grows the map stage-by-stage:
  stage → ops → artifacts, with the active stage pulsing violet. This is the flagship rich view of the
  autonomous loop (`00` §6 Flow A).
- Any map selection → `Send to chat` inserts `@map:…#node` chips; the kernel receives the subtree as
  structured context.

---

## 7. Keyboard and accessibility

- Full keyboard: arrows move between nodes (spatially aware: left/right along rings, up/down across
  siblings), `Tab/Shift+Tab` depth walk, `Enter` open doc, `F2` rename, `f` fit, `e` edit mode,
  `⌘F` search. A visible focus ring (2px azure, offset) marks the focused node.
- **Screen-reader mirror**: a visually-hidden `role="tree"` nested list mirrors the map (same order as
  the outline mode), with `aria-describedby` linking each node to its chips/values; map regions announce
  zoom level and node counts on change only.
- Reduced motion: layout jumps instead of animating; streaming pulse replaced by static dot + counter.
- Every map has an **outline fallback** tab and a "View as table" export — no information exists only
  spatially.

---

## 8. Performance budgets

| Target | Budget |
|---|---|
| Layout 5,000 nodes (radial) | ≤ 250ms in a Web Worker; incremental re-layout ≤ 30ms per batch |
| Pan/zoom 5,000 nodes | ≥ 55fps with LOD + culling (transform-only updates) |
| First render (500 nodes) | ≤ 150ms after data load |
| Export PNG 4K | ≤ 1s (SVG serialize + rasterize in worker) |
| Memory | ≤ 80MB steady for 10k-node map (nodes pooled, text cached) |

---

## 9. Export and embed

- SVG / PNG (2× retina) / JSON (`MindMapSpec`) / **markdown outline** (markmap-compatible) / CSV
  (id, parent, label, kind, path).
- Embed: maps are documents (`kind: 'mindmap'`), so they live in pane tabs, pin to dashboards, and embed
  in `prose` docs via an embed directive (`::map{id}`) rendered read-only at depth 2 with a "Open full
  map" affordance.
