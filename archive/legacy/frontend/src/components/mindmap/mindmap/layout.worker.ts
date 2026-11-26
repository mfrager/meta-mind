import { layoutRadial, pointsForZoom } from "@/lib/trees/layout.ts";
import type { TreeNode } from "@/lib/trees/model.ts";

export type LayoutRequest = { nodes: TreeNode[]; zoom: number };

self.onmessage = (event: MessageEvent<LayoutRequest>) => {
  const { nodes, zoom } = event.data;
  const points = nodes.length > 2000 ? pointsForZoom(nodes, zoom) : layoutRadial(nodes, zoom);
  self.postMessage(points);
};
