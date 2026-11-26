import type { TreeNode } from "@/lib/trees/model.ts";
import type { MapPoint } from "@/lib/trees/layout.ts";

export function outlineMarkdown(nodes: TreeNode[]): string {
  const lines: string[] = [];
  const walk = (parentId: string | null, depth: number) => {
    for (const node of nodes.filter((item) => item.parentId === parentId)) {
      lines.push(`${"  ".repeat(depth)}- ${node.label}`);
      walk(node.id, depth + 1);
    }
  };
  walk(null, 0);
  return `${lines.join("\n")}\n`;
}

export function mapSvg(points: MapPoint[]): string {
  const marks = points
    .map((point) => {
      if (point.lod === "dot") {
        return `<circle data-map-node="${point.id}" cx="${point.x.toFixed(1)}" cy="${point.y.toFixed(1)}" r="3" />`;
      }
      return `<text data-map-node="${point.id}" x="${point.x.toFixed(1)}" y="${point.y.toFixed(1)}">${escapeXml(point.label)}</text>`;
    })
    .join("");
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-800 -800 1600 1600">${marks}</svg>`;
}

function escapeXml(value: string) {
  return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}
