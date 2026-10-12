import type { HierarchyRectangularNode } from "d3-hierarchy";

import type { TreeNode } from "@/lib/trees/model.ts";

export type TreeTile = { id: string; label: string; x: number; y: number; width: number; height: number };

export function tilesFrom(root: HierarchyRectangularNode<TreeNode>): TreeTile[] {
  return root.descendants().map((node) => ({
    id: node.data.id,
    label: node.data.label,
    x: node.x0,
    y: node.y0,
    width: node.x1 - node.x0,
    height: node.y1 - node.y0,
  }));
}
