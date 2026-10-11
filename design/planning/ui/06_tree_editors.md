# 06 — Tree editors and viewers

> Parent: [`INDEX.md`](./INDEX.md) · Next: [`07_mind_map.md`](./07_mind_map.md)
> · Δ vs rust_symbolic: the reference has class accordions and a model outline; the Console ships a
> full **tree engine** (virtualized, editable, DnD, undo/redo, tree-table) and ten domain trees.

Trees are the Console's primary *navigation + editing* surface because the kernel's data is
hierarchical: ontology classes, goals and timescales, memory topics, module registry, policy genome,
decision/evidence chains, event provenance, change-set ops, the library, and budgets.

---

## 1. What "extremely rich, varied, and detailed" means here

The tree engine (`frontend/src/components/trees/`) is one codebase with pluggable adapters; each domain
tree declares its node schema, adornments, edit ops, and validation. Capabilities (all trees get all of
them unless explicitly disabled):

1. Virtualized rendering (20k nodes), lazy async children.
2. Expand/collapse with per-user persistence; expand-all/collapse/expand-to-path.
3. Multi-select (click, shift-range, ctrl-toggle) + tri-state checkbox mode for bulk ops.
4. Inline editing: rename (F2 / dbl-click), quick-edit fields in row chips, cell editing in table mode.
5. Drag & drop (dnd-kit) with drop indicators (above/inside/below), validation hooks (a drop that would
   violate the schema shows a red ghost + reason tooltip), and multi-node drags.
6. Cut / copy / paste (internal clipboard), duplicate, delete (with `AlertDialog` naming the object).
7. Undo/redo stack for all structural edits (⌘Z / ⌘⇧Z), with the batched transaction shown in a toast.
8. Search: fuzzy filter with auto-expand to matches, match count, highlight, next/prev (F3/⇧F3), and a
   "filter chips" mode (kind / status / epistemic).
9. Keyboard model per WAI-ARIA `treeview`/`treegrid` (§3.2).
10. Row adornments: icon chips, status chips, counts, sparklines, mini-bars, probability bars, avatars.
11. Context menus (right-click / ⌘-click) with domain actions + "Send to chat" + "Open in mind map".
12. Bookmarks/pins; "path bar" breadcrumb of the focused node with clickable ancestors.
13. Tree-table mode: columns (type-specific, sortable), frozen first column, cell renderers.
14. Alternate viewers over the same adapter: **outline**, **tree-table**, **sunburst/icicle**, **treemap**.
15. Export: JSON / CSV (flattened paths) / SVG (sunburst/treemap) / markdown outline.

---

## 2. Engine architecture

```
TreeAdapter (domain)  →  TreeCore (headless)  →  views: OutlineView | TreeTableView | SunburstView | TreemapView
   node schema             useTreeController
   load children           expansion · selection · editing
   edit ops                DnD · undo · search · keyboard
   validation              virtualizer (@tanstack/react-virtual)
   adornments              events → command bus (API mutations)
```

### 2.1 The adapter contract (normative)

```ts
interface TreeNode {
  id: string;                    // stable, path-safe
  parentId: string | null;
  kind: string;                  // 'class'|'property'|'goal'|'memory'|… (drives icon chip + tone)
  label: string;
  mono?: string;                 // right-aligned mono value (id, count, score)
  status?: DocStatus;            // chip in row
  adornments?: Adornment[];      // spark | bar | meter | badge | avatar-stack | probability
  childCount?: number;           // enables lazy expansion indicator
  capabilities: ('rename'|'move'|'delete'|'edit'|'add-child'|'add-sibling'|'check')[];
  payload?: unknown;             // domain record for the detail pane
}

interface TreeAdapter {
  roots(query: TreeQuery): AsyncIterable<TreeNode[]>;
  children(node: TreeNode): AsyncIterable<TreeNode[]>;
  schema: { nodeKinds: NodeKindSpec[]; editableFields: FieldSpec[]; validation: ZodSchema };
  ops: {
    rename(id, label, reason?): Promise<MutationResult>;
    move(ids, targetId, position: 'above'|'inside'|'below', reason?): Promise<MutationResult>;
    remove(ids, reason?): Promise<MutationResult>;
    add(parentId, kind, fields): Promise<MutationResult>;
    reorder?(ids, index): Promise<MutationResult>;
  };
  commands(node: TreeNode): CommandSpec[];   // domain actions surfaced in menus/toolbars
}
```

Every `MutationResult` returns the event-log provenance (`event_id`, `trace`) so rows can flash a green
"saved" tick and the footer can update the provenance strip — writes are visible, never silent.

### 2.2 `useTreeController` state

Pure, testable reducer over: `{ expanded: Set<id>, selection: RangeSel, focus: id|null, editing, clipboard, undoStack, redoStack, search, filters }`.
Unit tests drive the reducer directly (no DOM). Async loads are request-versioned (stale responses
dropped). Optimistic mutations apply locally then reconcile with the server snapshot; on failure the
edit is reverted and an error toast + row flash explains why (kernel `reason` verbatim).

---

## 3. Views

### 3.1 Outline view
Rows 28/32px: indent guide (1px `--border` lines, 12px per level), twisty (chevron, spins when loading),
icon chip, label, mono value, chips/adornments right-aligned, row hover shows the action cluster
(add-child, more). Focus row: 2px azure inset outline + `bg-accent/50`. Multi-select: 8% azure fill.

### 3.2 Tree-table view
First column = outline column (frozen); domain columns per tree (e.g. ontology: `label | property |
range | required | cardinality`). Sortable headers; inline cell editing for editable columns; column
visibility menu; this is the workhorse for auditing trees (goals with progress %, modules with versions,
policies with fitness).

### 3.3 Sunburst / icicle (zoomable)
`d3-hierarchy` `partition()`; arcs/rects colored by the **stable branch-hue rule** (§`07` §5.2); click
zooms (re-root), hover shows a `HoverCard` with the node's `KeyValueGrid`, breadcrumbs for the current
root. Icicle variant for wide screens. Export SVG/PNG.

### 3.4 Treemap
`d3-hierarchy` `treemap()` with squarify; tile label = label + mono measure; sizes by a selectable
measure (count, tokens, budget weight, debt score); click-through to the node doc. Used for memory
composition, debt breakdown, budget allocation.

---

## 4. Editing semantics (the write discipline)

| Edit | UI flow | Backend |
|---|---|---|
| Rename / field edit | Inline edit → optimistic → toast on failure | `adapter.ops.rename` → ledger write |
| Move / reparent | DnD or cut/paste → validation ghost → drop | `adapter.ops.move` (ledger write where the kernel allows; change set where it doesn't) |
| Delete | `AlertDialog` naming the object + reason field (required for non-ledger kinds) | `adapter.ops.remove` (reversible where kernel supports; `gc` never runs from tree edits) |
| Add child/sibling | Inline "ghost row" form (kind select + fields) | `adapter.ops.add` |
| Bulk ops | Multi-select → toolbar menu (Move to…, Tag, Export, Delete) | batched mutations, one undo entry |
| Production-affecting edits (module tree, policy genome) | Same as `05` §5.5: Save-as-change-set wizard | `changeset new` pipeline |

Destructive actions are **never one-click**: irreversible ops require typing the object id (matching the
kernel's own discipline). Tree edits always show what they did: a toast + a row tick + provenance strip.

---

## 5. The ten domain trees

| # | Tree | Adapter `kind`s | Columns (tree-table) | Adornments | Notable ops/commands |
|---|---|---|---|---|---|
| 5.1 | **Ontology** (`explore/ontology`) | class, property, shape, named-graph | label, range, required, cardinality, graph | SHACL status chips, instance counts | validate (`graph validate`), add class/property, edit shapes (change set), open instance table |
| 5.2 | **Being** (`explore/being`) | identity, invariant, goal, subgoal, commitment, relationship, belief, affect | title, timescale, progress, priority | progress bars, affect meter (rose), lock icon on invariants | `being goal add`, block/unblock, verify invariants; **invariants are read-only** (kernel rule) |
| 5.3 | **Memory** (`explore/memory`) | topic, episode, mistake, procedure, index entry | kind, strength, last-recalled, count | strength meter (teal), mistake→regression link | add, recall (op card), consolidate, forget (reversible), `memory eval` |
| 5.4 | **Epistemic** (`explore/epistemic`) | claim, evidence, assumption, dependency, contradiction | status, confidence, supports/refutes | epistemic badges, contradiction count (red) | ingest, promote-to-knowledge, invalidate, dependency view |
| 5.5 | **Library** (`explore/library`) | technique, policy, frame, skill, case | kind, applicability, uses | usage spark, applicability chips | import, `library applicable`, extract, register skill |
| 5.6 | **Modules** (`build/modules`) | module, version, capability, hook | version, status, loaded? | status chips (`promoted`/`promotion-pending`/`rolled-back`), load indicator | `module load/rollback`, view diff, hot-load animation |
| 5.7 | **Policy genome** (`build/policies`) | policy, generation, rule, authorization | generation, fitness, coverage | fitness sparkline per lineage | `policy history/fitness/evolve`, lineage expand |
| 5.8 | **Provenance** (drawer + doc) | event, span, actor, write | time, actor, hash | hash chain links, trace spans | `logs trace`, replay from here, export chain |
| 5.9 | **Change set** (doc-internal) | change-set, file, hunk, op | path, +/-, kind | diff colors, gate stage chips | open diff doc, comment pin, sandbox/promote ops |
| 5.10 | **Budgets** (`operate/budgets`) | budget, bucket, allocation, run | limit, used, remaining | `GaugeArc` mini, threshold chips | `budget show`, adjust (change set), audit |

Each tree ships: empty state with one action, a "Send tree to chat" (as a markdown outline attachment),
and "Open as mind map" (`07` §9).

---

## 6. Cross-tree features

- **Universal search across trees** (⌘K source + `SearchInput` in each tree): fuzzy over label + mono
  fields, grouped by tree, jumps + expands path.
- **Compare mode**: select two nodes (same kind) → `SplitDetail` diff of their editable fields + payload.
- **References panel**: for the focused node, a `ListCard` of inbound links (e.g. claims supporting a
  belief; modules implementing a capability) — the tree-of-record for "what touches this?".
- **Bookmarks**: any node pinnable to the sidebar navigator (`03` §2.1).

---

## 7. Performance and testing

- Virtualization: `@tanstack/react-virtual` with dynamic row heights (measured post-render); 20k nodes ≤
  50ms expand; search index built in a Web Worker for trees > 2k nodes.
- Sunburst/treemap layouts run in a Web Worker above 2k nodes; incremental updates on live trees.
- Tests: reducer unit tests for every adapter op (optimistic + revert paths), keyboard conformance suite
  (shared across trees), Playwright: rename → toast → reconcile, DnD move + undo, bulk delete dialog,
  search + F3 walk, tree-table sort/edit; axe checks on the treegrid.

---

## 8. Accessibility

`role="treegrid"` (table mode) / `role="tree"` (outline), `aria-expanded`, `aria-selected`,
`aria-level`, `aria-multiselectable`; full keyboard (§3.2 mapping): arrows navigate, `→/←`
expand/collapse, `Home/End`, `PageUp/Down`, `Enter` open, `Space` select, `F2` rename, `Delete` remove,
`Ctrl+X/C/V` clipboard, `*` expand-all-under-focus. Screen readers get a throttled level/label
announcement; drag & drop has keyboard equivalents (`Alt+↑/↓` move, `Alt+→/←` reparent).
