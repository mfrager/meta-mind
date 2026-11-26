import type { TreeNode } from "@/lib/trees/model.ts";

export type MapPoint = {
  id: string;
  parentId: string | null;
  label: string;
  depth: number;
  x: number;
  y: number;
  lod: "full" | "label" | "dot";
  hue: BranchHue;
};

export const BRANCH_HUES = ["cyan", "azure", "violet", "teal", "indigo", "rose", "green", "amber"] as const;
export type BranchHue = (typeof BRANCH_HUES)[number];

export function branchHue(id: string): BranchHue {
  let hash = 0;
  for (const char of id) hash = (hash * 33 + char.charCodeAt(0)) >>> 0;
  return BRANCH_HUES[hash % BRANCH_HUES.length] ?? "cyan";
}

export function lodFor(depth: number, zoom: number): MapPoint["lod"] {
  if (zoom < 0.5) return depth <= 2 ? "label" : "dot";
  if (zoom <= 1) return "label";
  return "full";
}

/** Radial tidy layout. Above 2,000 nodes and below 0.5× zoom, only depth ≤ 2 stays visible. */
export function layoutRadial(nodes: TreeNode[], zoom: number): MapPoint[] {
  if (nodes.length === 0) return [];
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const children = new Map<string, TreeNode[]>();
  let root: TreeNode | null = null;
  for (const node of nodes) {
    if (!node.parentId || !byId.has(node.parentId)) {
      if (!root) root = node;
      continue;
    }
    const list = children.get(node.parentId) ?? [];
    list.push(node);
    children.set(node.parentId, list);
  }
  if (!root) return [];
  const leaves = new Map<string, number>();
  const countLeaves = (id: string): number => {
    const kids = children.get(id) ?? [];
    const total = kids.length === 0 ? 1 : kids.reduce((sum, child) => sum + countLeaves(child.id), 0);
    leaves.set(id, total);
    return total;
  };
  countLeaves(root.id);
  const points: MapPoint[] = [];
  const walk = (node: TreeNode, depth: number, start: number, end: number) => {
    const angle = (start + end) / 2;
    const radius = depth === 0 ? 0 : 80 + depth * 70;
    points.push({
      id: node.id,
      parentId: node.parentId,
      label: node.label,
      depth,
      x: Math.cos(angle - Math.PI / 2) * radius,
      y: Math.sin(angle - Math.PI / 2) * radius,
      lod: lodFor(depth, zoom),
      hue: branchHue(depth === 0 ? node.id : (ancestorHue(node, byId) ?? node.id)),
    });
    const kids = children.get(node.id) ?? [];
    const width = end - start;
    const total = Math.max(1, leaves.get(node.id) ?? 1);
    let cursor = start;
    for (const child of kids) {
      const share = width * ((leaves.get(child.id) ?? 1) / total);
      walk(child, depth + 1, cursor, cursor + share);
      cursor += share;
    }
  };
  walk(root, 0, 0, Math.PI * 2);
  return points;
}

function ancestorHue(node: TreeNode, byId: Map<string, TreeNode>): string | null {
  let current: TreeNode | undefined = node;
  let parent = current.parentId ? byId.get(current.parentId) : undefined;
  while (parent?.parentId) {
    current = parent;
    parent = current.parentId ? byId.get(current.parentId) : undefined;
  }
  return parent ? current?.id ?? parent.id : current?.id ?? null;
}

export function pointsForZoom(nodes: TreeNode[], zoom: number): MapPoint[] {
  const laid = layoutRadial(nodes, zoom);
  if (laid.length > 2000 && zoom < 0.5) return laid.filter((point) => point.depth <= 2);
  if (laid.length > 5000) return laid.filter((point) => point.depth <= 3);
  return laid;
}
