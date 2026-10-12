import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import type { FlatRow } from "@/lib/trees/model.ts";

export function OutlineView({
  rows,
  nodes,
  focusId,
  expanded,
  onFocus,
  onToggle,
}: {
  rows: FlatRow[];
  nodes: { id: string; parentId: string | null }[];
  focusId: string | null;
  expanded: ReadonlySet<string>;
  onFocus: (id: string) => void;
  onToggle: (id: string) => void;
}) {
  const parent = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parent.current,
    estimateSize: () => 28,
    overscan: 8,
    initialRect: { width: 640, height: 384 },
  });
  return (
    <div
      ref={parent}
      role="tree"
      aria-label="Tree"
      data-tree-scroll
      data-tree-count={rows.length}
      className="pane-scroll h-96 overflow-auto"
    >
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          if (!row) return null;
          const hasChildren = nodes.some((node) => node.parentId === row.node.id);
          return (
            <div
              key={row.node.id}
              role="treeitem"
              aria-level={row.depth + 1}
              aria-selected={row.node.id === focusId}
              aria-expanded={hasChildren ? expanded.has(row.node.id) : undefined}
              data-tree-row
              data-tree-index={item.index}
              data-tree-id={row.node.id}
              className="absolute right-0 left-0 flex h-7 items-center gap-2 pr-2"
              style={{ transform: `translateY(${item.start}px)`, paddingLeft: 8 + row.depth * 12 }}
              onClick={() => onFocus(row.node.id)}
            >
              <button
                type="button"
                aria-label={expanded.has(row.node.id) ? `Collapse ${row.node.label}` : `Expand ${row.node.label}`}
                className="text-muted-foreground grid size-4 place-items-center"
                onClick={(event) => {
                  event.stopPropagation();
                  onToggle(row.node.id);
                }}
              >
                {expanded.has(row.node.id) ? "▾" : "▸"}
              </button>
              <span className="truncate text-small">{row.node.label}</span>
              {row.node.mono ? <span className="text-muted-foreground ml-auto font-mono text-value">{row.node.mono}</span> : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
