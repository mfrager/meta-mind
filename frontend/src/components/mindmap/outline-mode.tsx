import type { TreeNode } from "@/lib/trees/model.ts";

export function OutlineMode({ nodes }: { nodes: TreeNode[] }) {
  const roots = nodes.filter((node) => node.parentId === null);
  return (
    <ul role="tree" aria-label="Map outline" data-map-outline>
      {roots.map((node) => (
        <Branch key={node.id} node={node} nodes={nodes} level={1} />
      ))}
    </ul>
  );
}

function Branch({ node, nodes, level }: { node: TreeNode; nodes: TreeNode[]; level: number }) {
  const children = nodes.filter((item) => item.parentId === node.id);
  return (
    <li role="treeitem" aria-level={level} aria-label={node.label} aria-expanded={children.length > 0 ? true : undefined}>
      <span>{node.label}</span>
      {children.length > 0 ? (
        <ul role="group">
          {children.map((child) => (
            <Branch key={child.id} node={child} nodes={nodes} level={level + 1} />
          ))}
        </ul>
      ) : null}
    </li>
  );
}
