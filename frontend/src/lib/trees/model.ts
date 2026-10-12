export const TREE_NAMES = [
  "ontology",
  "being",
  "memory",
  "epistemic",
  "library",
  "modules",
  "policies",
  "provenance",
  "changesets",
  "budgets",
] as const;

export type TreeName = (typeof TREE_NAMES)[number];

export type NodeCapability = "rename" | "move" | "delete" | "add-child";

export type TreeNode = {
  id: string;
  parentId: string | null;
  kind: string;
  label: string;
  mono?: string;
  capabilities: NodeCapability[];
};

export type TreeSnapshot = {
  name: string;
  title: string;
  fixture: boolean;
  reason?: string | null;
  nodes: TreeNode[];
};

export type MovePosition = "above" | "inside" | "below";

export type TreeOp =
  | { op: "rename"; id: string; label: string }
  | { op: "move"; id: string; targetId: string; position: MovePosition }
  | { op: "remove"; id: string }
  | { op: "add"; id: string; parentId: string | null; kind: string; label: string; mono?: string };

export type Edit = { undo: TreeOp[]; redo: TreeOp[] };

export type TreeState = {
  nodes: TreeNode[];
  expanded: string[];
  selection: string[];
  focusId: string | null;
  editingId: string | null;
  undo: Edit[];
  redo: Edit[];
  query: string;
  pendingDeleteId: string | null;
};

export type TreeAction =
  | { type: "load"; nodes: TreeNode[] }
  | { type: "key"; key: string; alt?: boolean; meta?: boolean; shift?: boolean }
  | { type: "rename"; id: string; label: string }
  | { type: "move"; id: string; targetId: string; position: MovePosition }
  | { type: "remove"; id: string }
  | { type: "add"; parentId: string; kind: string; label: string }
  | { type: "undo" }
  | { type: "redo" }
  | { type: "toggle"; id: string }
  | { type: "focus"; id: string }
  | { type: "search"; query: string }
  | { type: "edit"; id: string }
  | { type: "cancel-edit" }
  | { type: "ask-remove"; id: string }
  | { type: "cancel-remove" }
  | { type: "revert"; state: TreeState };

export const initialTreeState: TreeState = {
  nodes: [],
  expanded: [],
  selection: [],
  focusId: null,
  editingId: null,
  undo: [],
  redo: [],
  query: "",
  pendingDeleteId: null,
};

export type FlatRow = { node: TreeNode; depth: number };

export function heapTree(count: number): TreeNode[] {
  const nodes: TreeNode[] = [];
  for (let index = 0; index < count; index += 1) {
    nodes.push({
      id: `n${index}`,
      parentId: index === 0 ? null : `n${(index - 1) >> 1}`,
      kind: "node",
      label: `Node ${index}`,
      capabilities: ["rename", "move", "delete", "add-child"],
    });
  }
  return nodes;
}

export function visibleRows(nodes: TreeNode[], expanded: ReadonlySet<string>, query = ""): FlatRow[] {
  const byParent = new Map<string | null, TreeNode[]>();
  for (const node of nodes) {
    const list = byParent.get(node.parentId) ?? [];
    list.push(node);
    byParent.set(node.parentId, list);
  }
  const needle = query.trim().toLowerCase();
  const rows: FlatRow[] = [];
  const walk = (parentId: string | null, depth: number) => {
    for (const node of byParent.get(parentId) ?? []) {
      const children = byParent.get(node.id) ?? [];
      const self = !needle || node.label.toLowerCase().includes(needle);
      const child = needle ? branchMatches(node.id, needle, byParent) : false;
      if (needle && !self && !child) continue;
      rows.push({ node, depth });
      if (expanded.has(node.id) || (needle && child)) walk(node.id, depth + 1);
      void children;
    }
  };
  walk(null, 0);
  return rows;
}

function branchMatches(id: string, needle: string, byParent: Map<string | null, TreeNode[]>): boolean {
  for (const child of byParent.get(id) ?? []) {
    if (child.label.toLowerCase().includes(needle) || branchMatches(child.id, needle, byParent)) return true;
  }
  return false;
}

export function treeMapRef(tree: string, nodeId: string): string {
  return `@map:${tree}#${nodeId}`;
}

export function parseTreeMapRef(value: string): { tree: string; nodeId: string } | null {
  const match = /^@map:([a-z]+)#([A-Za-z0-9_-]+)$/.exec(value.trim());
  if (!match) return null;
  return { tree: match[1] ?? "", nodeId: match[2] ?? "" };
}

/** Tree node, map node, and chat reference are the same id. */
export function roundTripRef(tree: TreeSnapshot, nodeId: string): TreeNode | null {
  const parsed = parseTreeMapRef(treeMapRef(tree.name, nodeId));
  if (!parsed || parsed.tree !== tree.name) return null;
  return tree.nodes.find((node) => node.id === parsed.nodeId) ?? null;
}

export function reduceTree(state: TreeState, action: TreeAction): TreeState {
  switch (action.type) {
    case "load": {
      const root = action.nodes.find((node) => node.parentId === null);
      return {
        ...initialTreeState,
        nodes: action.nodes,
        expanded: root ? [root.id] : [],
        focusId: root?.id ?? null,
      };
    }
    case "revert":
      return action.state;
    case "focus":
      return { ...state, focusId: action.id, selection: [action.id] };
    case "toggle":
      return {
        ...state,
        expanded: state.expanded.includes(action.id)
          ? state.expanded.filter((id) => id !== action.id)
          : [...state.expanded, action.id],
      };
    case "search":
      return { ...state, query: action.query };
    case "edit":
      return { ...state, editingId: action.id, focusId: action.id };
    case "cancel-edit":
      return { ...state, editingId: null };
    case "ask-remove":
      return { ...state, pendingDeleteId: action.id };
    case "cancel-remove":
      return { ...state, pendingDeleteId: null };
    case "key":
      return applyKey(state, action);
    case "rename":
    case "move":
    case "remove":
    case "add":
      return applyEdit(state, action);
    case "undo": {
      const edit = state.undo.at(-1);
      if (!edit) return state;
      return {
        ...state,
        nodes: edit.undo.reduce(applyOp, state.nodes),
        undo: state.undo.slice(0, -1),
        redo: [...state.redo, edit],
        editingId: null,
      };
    }
    case "redo": {
      const edit = state.redo.at(-1);
      if (!edit) return state;
      return {
        ...state,
        nodes: edit.redo.reduce(applyOp, state.nodes),
        redo: state.redo.slice(0, -1),
        undo: [...state.undo, edit],
      };
    }
    default:
      return state;
  }
}

export function postFor(state: TreeState, action: TreeAction): TreeOp[] | null {
  if (action.type === "undo") return state.undo.at(-1)?.undo ?? null;
  if (action.type === "redo") return state.redo.at(-1)?.redo ?? null;
  if (action.type === "rename" || action.type === "move" || action.type === "remove" || action.type === "add" || action.type === "key") {
    const next = reduceTree(state, action);
    if (next.undo.length < state.undo.length) return state.undo.at(-1)?.undo ?? null;
    if (next.undo.length > state.undo.length) return next.undo.at(-1)?.redo ?? null;
  }
  return null;
}

function applyKey(state: TreeState, action: Extract<TreeAction, { type: "key" }>): TreeState {
  if (state.editingId) {
    if (action.key === "Escape") return { ...state, editingId: null };
    return state;
  }
  if (action.meta && action.key.toLowerCase() === "z") {
    return reduceTree(state, { type: action.shift ? "redo" : "undo" });
  }
  const rows = visibleRows(state.nodes, new Set(state.expanded), state.query);
  const index = Math.max(0, rows.findIndex((row) => row.node.id === state.focusId));
  const current = rows[index];
  if (!current) return state;
  if (action.key === "ArrowDown") {
    const next = rows[Math.min(rows.length - 1, index + 1)];
    return next ? { ...state, focusId: next.node.id, selection: [next.node.id] } : state;
  }
  if (action.key === "ArrowUp") {
    const next = rows[Math.max(0, index - 1)];
    return next ? { ...state, focusId: next.node.id, selection: [next.node.id] } : state;
  }
  if (action.key === "Home") {
    const first = rows[0];
    return first ? { ...state, focusId: first.node.id, selection: [first.node.id] } : state;
  }
  if (action.key === "End") {
    const last = rows.at(-1);
    return last ? { ...state, focusId: last.node.id, selection: [last.node.id] } : state;
  }
  if (action.key === "ArrowRight") {
    if (!state.expanded.includes(current.node.id)) return reduceTree(state, { type: "toggle", id: current.node.id });
    const child = state.nodes.find((node) => node.parentId === current.node.id);
    return child ? { ...state, focusId: child.id, selection: [child.id] } : state;
  }
  if (action.key === "ArrowLeft") {
    if (state.expanded.includes(current.node.id) && state.nodes.some((node) => node.parentId === current.node.id)) {
      return reduceTree(state, { type: "toggle", id: current.node.id });
    }
    return current.node.parentId ? { ...state, focusId: current.node.parentId, selection: [current.node.parentId] } : state;
  }
  if (action.key === "F2") return { ...state, editingId: current.node.id };
  if (action.key === "Delete") return { ...state, pendingDeleteId: current.node.id };
  if (action.key === "*") {
    const ids = descendants(state.nodes, current.node.id).map((node) => node.id);
    return { ...state, expanded: [...new Set([...state.expanded, ...ids])] };
  }
  if (action.alt && action.key === "ArrowUp") {
    const previous = rows[index - 1];
    if (!previous || previous.node.id === current.node.parentId) return state;
    return applyEdit(state, { type: "move", id: current.node.id, targetId: previous.node.id, position: "above" });
  }
  if (action.alt && action.key === "ArrowDown") {
    const next = rows[index + 1];
    if (!next) return state;
    return applyEdit(state, { type: "move", id: current.node.id, targetId: next.node.id, position: "below" });
  }
  return state;
}

function applyEdit(state: TreeState, action: TreeAction): TreeState {
  const redo = opFrom(state, action);
  if (!redo) return state;
  const undo = inverse(state.nodes, redo);
  const nodes = redo.reduce(applyOp, state.nodes);
  return {
    ...state,
    nodes,
    undo: [...state.undo, { undo, redo }],
    redo: [],
    editingId: null,
    pendingDeleteId: null,
  };
}

function opFrom(state: TreeState, action: TreeAction): TreeOp[] | null {
  if (action.type === "rename") return [{ op: "rename", id: action.id, label: action.label }];
  if (action.type === "move") return [{ op: "move", id: action.id, targetId: action.targetId, position: action.position }];
  if (action.type === "remove") return [{ op: "remove", id: action.id }];
  if (action.type === "add") {
    return [{ op: "add", id: `new-${state.nodes.length}`, parentId: action.parentId, kind: action.kind, label: action.label }];
  }
  return null;
}

function inverse(nodes: TreeNode[], ops: TreeOp[]): TreeOp[] {
  const undone: TreeOp[] = [];
  for (const op of ops) {
    if (op.op === "rename") {
      const previous = nodes.find((node) => node.id === op.id);
      if (previous) undone.push({ op: "rename", id: op.id, label: previous.label });
    } else if (op.op === "remove") {
      for (const node of descendants(nodes, op.id)) {
        undone.push({
          op: "add",
          id: node.id,
          parentId: node.parentId,
          kind: node.kind,
          label: node.label,
          mono: node.mono,
        });
      }
    } else if (op.op === "add") {
      undone.push({ op: "remove", id: op.id });
    } else if (op.op === "move") {
      const node = nodes.find((item) => item.id === op.id);
      const next = siblingAfter(nodes, op.id);
      if (node?.parentId && next) undone.push({ op: "move", id: op.id, targetId: next, position: "above" });
      else if (node?.parentId) undone.push({ op: "move", id: op.id, targetId: node.parentId, position: "inside" });
    }
  }
  return undone.reverse();
}

function siblingAfter(nodes: TreeNode[], id: string): string | null {
  const node = nodes.find((item) => item.id === id);
  if (!node) return null;
  const siblings = nodes.filter((item) => item.parentId === node.parentId);
  const index = siblings.findIndex((item) => item.id === id);
  return siblings[index + 1]?.id ?? null;
}

export function applyOp(nodes: TreeNode[], op: TreeOp): TreeNode[] {
  if (op.op === "rename") {
    return nodes.map((node) => (node.id === op.id ? { ...node, label: op.label } : node));
  }
  if (op.op === "remove") {
    const drop = new Set(descendants(nodes, op.id).map((node) => node.id));
    return nodes.filter((node) => !drop.has(node.id));
  }
  if (op.op === "add") {
    if (nodes.some((node) => node.id === op.id)) return nodes;
    return [
      ...nodes,
      {
        id: op.id,
        parentId: op.parentId,
        kind: op.kind,
        label: op.label,
        mono: op.mono,
        capabilities: ["rename", "move", "delete"],
      },
    ];
  }
  if (isDescendant(nodes, op.id, op.targetId) || op.id === op.targetId) return nodes;
  const moving = nodes.find((node) => node.id === op.id);
  const target = nodes.find((node) => node.id === op.targetId);
  if (!moving || !target || moving.parentId === null) return nodes;
  const parentId = op.position === "inside" ? target.id : target.parentId;
  const rest = nodes.filter((node) => node.id !== op.id);
  const moved = { ...moving, parentId };
  if (op.position === "inside") return [...rest, moved];
  const index = rest.findIndex((node) => node.id === op.targetId);
  const at = op.position === "below" ? index + 1 : index;
  rest.splice(at, 0, moved);
  return rest;
}

function descendants(nodes: TreeNode[], id: string): TreeNode[] {
  const out: TreeNode[] = [];
  const walk = (current: string) => {
    const node = nodes.find((item) => item.id === current);
    if (!node) return;
    out.push(node);
    for (const child of nodes) {
      if (child.parentId === current) walk(child.id);
    }
  };
  walk(id);
  return out;
}

function isDescendant(nodes: TreeNode[], ancestor: string, id: string): boolean {
  let current = nodes.find((node) => node.id === id)?.parentId ?? null;
  while (current) {
    if (current === ancestor) return true;
    current = nodes.find((node) => node.id === current)?.parentId ?? null;
  }
  return false;
}
