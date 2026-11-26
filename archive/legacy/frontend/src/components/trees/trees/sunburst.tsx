import { useMemo } from "react";
import { partition, stratify } from "d3-hierarchy";

import type { TreeNode } from "@/lib/trees/model.ts";
import { tilesFrom, type TreeTile } from "@/components/trees/tiles.ts";

export function SunburstView({ nodes }: { nodes: TreeNode[] }) {
  const tiles = useMemo(() => {
    const root = hierarchyOf(nodes);
    if (!root) return [];
    root.sum(() => 1);
    return tilesFrom(partition<TreeNode>().size([640, 240])(root));
  }, [nodes]);
  return <TileSvg tiles={tiles} label="Sunburst" marker="data-sunburst" fill="fill-cyan-100 stroke-cyan-700" />;
}

export function hierarchyOf(nodes: TreeNode[]) {
  if (nodes.length === 0) return null;
  return stratify<TreeNode>()
    .id((node) => node.id)
    .parentId((node) => node.parentId)(nodes);
}

export function TileSvg({
  tiles,
  label,
  marker,
  fill,
}: {
  tiles: TreeTile[];
  label: string;
  marker: string;
  fill: string;
}) {
  return (
    <svg viewBox="0 0 640 240" className="h-60 w-full" {...{ [marker]: "" }} role="img" aria-label={label}>
      {tiles.map((tile) => (
        <g key={tile.id}>
          <rect x={tile.x} y={tile.y} width={Math.max(0, tile.width)} height={Math.max(0, tile.height)} className={fill} />
          {tile.width > 48 ? (
            <text x={tile.x + 4} y={tile.y + 16} className="fill-foreground text-value">
              {tile.label}
            </text>
          ) : null}
        </g>
      ))}
    </svg>
  );
}
