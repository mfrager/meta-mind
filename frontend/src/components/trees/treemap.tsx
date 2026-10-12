import { useMemo } from "react";
import { treemap } from "d3-hierarchy";

import { hierarchyOf, TileSvg } from "@/components/trees/sunburst.tsx";
import { tilesFrom } from "@/components/trees/tiles.ts";
import type { TreeNode } from "@/lib/trees/model.ts";

export function TreemapView({ nodes }: { nodes: TreeNode[] }) {
  const tiles = useMemo(() => {
    const root = hierarchyOf(nodes);
    if (!root) return [];
    root.sum(() => 1);
    return tilesFrom(treemap<TreeNode>().size([640, 240]).padding(2)(root));
  }, [nodes]);
  return <TileSvg tiles={tiles} label="Treemap" marker="data-treemap" fill="fill-teal-50 stroke-teal-700" />;
}
