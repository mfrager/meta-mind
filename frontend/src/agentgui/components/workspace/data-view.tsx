"use client";

import { cn } from "@/agentgui/lib/utils";
import type { Artifact } from "@/agentgui/types/workspace";
import { BarChart3Icon } from "lucide-react";
import { useMemo } from "react";

function parseCsv(content: string): { header: string[]; rows: string[][] } {
  const lines = content.trim().split(/\r?\n/);
  const [headerLine, ...rest] = lines;
  const header = (headerLine ?? "").split(",").map((cell) => cell.trim());
  const rows = rest
    .filter((line) => line.trim().length > 0)
    .map((line) => line.split(",").map((cell) => cell.trim()));
  return { header, rows };
}

const isNumeric = (value: string) =>
  value !== "" && value !== "-" && !Number.isNaN(Number(value));

export function TableView({ artifact }: { artifact: Artifact }) {
  const { header, rows } = useMemo(
    () => parseCsv(artifact.content),
    [artifact.content]
  );

  return (
    <div className="h-full overflow-auto p-4">
      <div className="overflow-hidden rounded-xl border border-border bg-surface shadow-elev-1">
        <table className="w-full border-collapse text-[12px]">
          <thead>
            <tr className="bg-surface-muted/80">
              {header.map((cell) => (
                <th
                  className="border-b border-border px-3 py-2 text-left text-[10.5px] font-semibold tracking-wide text-muted-foreground uppercase"
                  key={cell}
                >
                  {cell.replaceAll("_", " ")}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, rowIndex) => (
              <tr
                className={cn(
                  "transition-colors hover:bg-surface-muted/70",
                  rowIndex % 2 === 1 && "bg-surface-muted/25"
                )}
                key={`${row[0]}-${rowIndex}`}
              >
                {row.map((cell, cellIndex) => (
                  <td
                    className={cn(
                      "border-b border-border/60 px-3 py-1.5",
                      isNumeric(cell) &&
                        "text-right font-mono tabular-nums text-foreground/85",
                      cellIndex === 0 && "font-medium"
                    )}
                    key={`${cell}-${cellIndex}`}
                  >
                    {cell}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="mt-2 font-mono text-[10px] text-muted-foreground">
        {rows.length} rows · {header.length} columns
      </p>
    </div>
  );
}

interface ChartSeries {
  unit?: string;
  series: { label: string; value: number; meta?: string }[];
}

export function ChartView({ artifact }: { artifact: Artifact }) {
  const chart = useMemo<ChartSeries>(() => {
    try {
      const parsed = JSON.parse(artifact.content) as ChartSeries;
      if (Array.isArray(parsed.series)) return parsed;
    } catch {
      /* fall through to the empty state */
    }
    return { series: [] };
  }, [artifact.content]);

  const max = Math.max(...chart.series.map((entry) => entry.value), 1);

  return (
    <div className="h-full overflow-auto p-5">
      <div className="mb-4 flex items-center gap-2">
        <span className="accent-soft flex size-6 items-center justify-center rounded-md border border-border/70 text-primary">
          <BarChart3Icon className="size-3.5" />
        </span>
        <div>
          <p className="text-[12.5px] font-medium">Rerank latency by stage</p>
          <p className="font-mono text-[10px] text-muted-foreground">
            {chart.unit ?? "ms"} · lower is better
          </p>
        </div>
      </div>

      <div className="space-y-3">
        {chart.series.map((entry, index) => (
          <div key={entry.label}>
            <div className="mb-1 flex items-baseline justify-between">
              <span className="text-[11.5px] font-medium">{entry.label}</span>
              <span className="font-mono text-[11px] text-muted-foreground">
                {entry.value} {chart.unit ?? ""}
                {entry.meta ? ` · ${entry.meta}` : ""}
              </span>
            </div>
            <div className="h-5 overflow-hidden rounded-md border border-border/60 bg-surface-sunken">
              <div
                className="flex h-full items-center justify-end rounded-r-sm pr-1.5 text-[9px] font-medium text-white/90 transition-[width] duration-300"
                style={{
                  width: `${Math.max(8, (entry.value / max) * 100)}%`,
                  backgroundImage:
                    "linear-gradient(90deg, var(--primary), color-mix(in oklab, oklch(0.632 0.119 169) 85%, white))",
                  opacity: 1 - index * 0.12,
                }}
              />
            </div>
          </div>
        ))}
      </div>

      {chart.series.length === 0 && (
        <p className="text-[12px] text-muted-foreground">
          No numeric series found in this artifact.
        </p>
      )}
    </div>
  );
}
