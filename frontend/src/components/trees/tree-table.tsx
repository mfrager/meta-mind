import { useMemo, useState } from "react";

import type { FlatRow } from "@/lib/trees/model.ts";

export function TreeTableView({ rows }: { rows: FlatRow[] }) {
  const [sort, setSort] = useState<"label" | "kind">("label");
  const ordered = useMemo(() => {
    return [...rows].sort((a, b) => a.node[sort].localeCompare(b.node[sort]));
  }, [rows, sort]);
  return (
    <div className="pane-scroll h-96 overflow-auto" data-tree-table>
      <table className="w-full text-small">
        <thead>
          <tr className="text-left">
            <th className="bg-background sticky left-0">
              <button type="button" onClick={() => setSort("label")}>Label</button>
            </th>
            <th>
              <button type="button" onClick={() => setSort("kind")}>Kind</button>
            </th>
            <th>Value</th>
          </tr>
        </thead>
        <tbody>
          {ordered.map((row) => (
            <tr key={row.node.id}>
              <td className="bg-background sticky left-0" style={{ paddingLeft: row.depth * 12 }}>
                {row.node.label}
              </td>
              <td>{row.node.kind}</td>
              <td className="font-mono text-value">{row.node.mono ?? ""}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
