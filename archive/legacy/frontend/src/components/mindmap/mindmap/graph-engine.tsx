import { ReactFlow } from "@xyflow/react";
import "@xyflow/react/dist/style.css";

import { layoutRadial } from "@/lib/trees/layout.ts";
import type { TreeNode } from "@/lib/trees/model.ts";

export function GraphEngine({ nodes }: { nodes: TreeNode[] }) {
  const points = layoutRadial(nodes, 1);
  return (
    <div className="h-96" data-graph-engine>
      <ReactFlow
        nodes={points.map((point) => ({
          id: point.id,
          position: { x: point.x, y: point.y },
          data: { label: point.label },
        }))}
        edges={nodes.flatMap((node) =>
          node.parentId
            ? [{ id: `${node.parentId}-${node.id}`, source: node.parentId, target: node.id }]
            : [],
        )}
        fitView
        nodesDraggable={false}
        nodesConnectable={false}
        proOptions={{ hideAttribution: true }}
      />
    </div>
  );
}
