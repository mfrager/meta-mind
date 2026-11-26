import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Line, LineChart, ResponsiveContainer } from "recharts";

import { Badge } from "@/components/ui/display.tsx";
import { Button } from "@/components/ui/actions.tsx";
import { CardShell, type CardState } from "@/components/kit/cards.tsx";
import { cn } from "@/lib/utils.ts";

export function StatusChip({
  tone,
  label,
  reason,
}: {
  tone: "ok" | "degraded" | "unavailable" | "error";
  label: string;
  reason?: string;
}) {
  const styles = {
    ok: "border-green-200 bg-green-100 text-green-800",
    degraded: "border-amber-200 bg-amber-100 text-amber-900",
    unavailable: "border-slate-200 bg-slate-100 text-slate-700",
    error: "border-red-200 bg-red-100 text-red-800",
  };
  return (
    <span data-token={`status-chip ${tone}`} title={reason} className={cn("micro-label inline-flex h-[22px] items-center rounded-pill border px-2", styles[tone])}>
      {label}
    </span>
  );
}

export function Delta({ value }: { value: number }) {
  const tone = value > 0 ? "text-green-700" : value < 0 ? "text-red-700" : "text-slate-600";
  const mark = value > 0 ? "▲" : value < 0 ? "▼" : "·";
  return (
    <span data-token="delta" className={cn("rounded-pill border border-border px-1 font-mono text-value", tone)}>
      {mark} {Math.abs(value).toFixed(1)}%
    </span>
  );
}

export function ProgressBar({ value, label, indeterminate = false }: { value?: number; label: string; indeterminate?: boolean }) {
  return (
    <div data-token="progress-bar" className="flex items-center gap-2">
      <div
        className="bg-muted h-1.5 flex-1 overflow-hidden rounded-pill"
        role="progressbar"
        aria-label={label}
        aria-valuenow={indeterminate ? undefined : value}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div className={cn("bg-azure-500 h-full", indeterminate && "w-1/3 motion-safe:animate-pulse")} style={indeterminate ? undefined : { width: `${value ?? 0}%` }} />
      </div>
      {indeterminate ? null : <span className="font-mono text-value">{value}%</span>}
    </div>
  );
}

export function StageTicker({ stages }: { stages: { name: string; state: "done" | "active" | "failed" | "pending" }[] }) {
  const color = { done: "bg-green-500", active: "bg-violet-500 mm-pulse", failed: "bg-red-500", pending: "bg-muted" };
  return (
    <div data-token="stage-ticker" className="flex gap-0.5" role="img" aria-label={stages.map((stage) => `${stage.name} ${stage.state}`).join(", ")}>
      {stages.map((stage) => (
        <span key={stage.name} title={stage.name} className={cn("h-2 w-3", color[stage.state])} />
      ))}
    </div>
  );
}

export function Sparkline({ values, label }: { values: number[]; label: string }) {
  const min = Math.min(...values);
  const max = Math.max(...values);
  const width = 80;
  const height = 24;
  const points = values
    .map((value, index) => {
      const x = (index / Math.max(values.length - 1, 1)) * width;
      const y = height - ((value - min) / (max - min || 1)) * (height - 4) - 2;
      return `${x},${y}`;
    })
    .join(" ");
  const last = points.split(" ").at(-1)?.split(",") ?? ["0", "0"];
  return (
    <svg data-token="sparkline" width={width} height={height} role="img" aria-label={label}>
      <polyline fill="none" stroke="var(--color-cyan-500)" strokeWidth="1" points={points} />
      <circle cx={last[0]} cy={last[1]} r="1.5" fill="var(--color-cyan-500)" />
    </svg>
  );
}

export function GaugeArc({ value, label }: { value: number; label: string }) {
  const start = Math.PI * 0.75;
  const end = start + (Math.PI * 1.5 * value) / 100;
  const arc = (angle: number) => `${16 + Math.cos(angle) * 12},${16 + Math.sin(angle) * 12}`;
  return (
    <svg data-token="gauge" width="32" height="32" role="img" aria-label={`${label} ${value}%`}>
      <path d={`M ${arc(start)} A 12 12 0 1 1 ${arc(start + Math.PI * 1.5)}`} fill="none" stroke="var(--muted)" strokeWidth="4" />
      <path d={`M ${arc(start)} A 12 12 0 ${value > 50 ? 1 : 0} 1 ${arc(end)}`} fill="none" stroke="var(--color-azure-500)" strokeWidth="4" />
      <text x="16" y="18" textAnchor="middle" className="fill-foreground font-mono text-[8px]">
        {value}
      </text>
    </svg>
  );
}

const CHART = [
  { x: "1", y: 2 },
  { x: "2", y: 4 },
  { x: "3", y: 3 },
];

export function TableCard({ state = "default" }: { state?: CardState }) {
  return (
    <CardShell title="Services" eyebrow="Records" kind="log" state={state}>
      <DataTable
        rows={[
          { id: "1", name: "sqlite", state: "ok" },
          { id: "2", name: "graph", state: "unavailable" },
        ]}
      />
    </CardShell>
  );
}

export function ChartCard({ state = "default" }: { state?: CardState }) {
  return (
    <CardShell title="Events" eyebrow="Chart" kind="chart" state={state}>
      {state === "empty" ? null : (
        <div className="h-24" role="img" aria-label="Events rose, then eased">
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={CHART}>
              <Line dataKey="y" stroke="var(--color-cyan-500)" dot={false} strokeWidth={1} />
            </LineChart>
          </ResponsiveContainer>
        </div>
      )}
    </CardShell>
  );
}

export type TableRow = { id: string; name: string; state: string };

export function DataTable({ rows, empty = false }: { rows: TableRow[]; empty?: boolean }) {
  return (
    <div data-token="data-table">
      <div className="mb-2 flex items-center gap-2">
        <input aria-label="Search rows" className="border-input h-7 rounded-md border px-2 text-small" placeholder="Search" />
        <Button size="sm" variant="outline">
          Columns
        </Button>
      </div>
      <table className="w-full border-collapse text-left">
        <thead className="bg-muted sticky top-0">
          <tr>
            <th className="micro-label p-1">
              <input type="checkbox" aria-label="Select all rows" />
            </th>
            <th className="micro-label p-1">Name</th>
            <th className="micro-label p-1">State</th>
          </tr>
        </thead>
        <tbody>
          {empty ? (
            <tr>
              <td colSpan={3} className="text-small text-muted-foreground p-2">
                No rows match.
              </td>
            </tr>
          ) : (
            rows.map((row) => (
              <tr key={row.id} className="border-border border-t">
                <td className="p-1">
                  <input type="checkbox" aria-label={`Select ${row.name}`} />
                </td>
                <td className="p-1 font-mono text-value">{row.name}</td>
                <td className="p-1">
                  <Badge tone={row.state === "ok" ? "green" : "amber"}>{row.state}</Badge>
                </td>
              </tr>
            ))
          )}
        </tbody>
      </table>
      <p className="text-small text-muted-foreground mt-1">{empty ? "0 rows" : `${rows.length} rows`}</p>
    </div>
  );
}

export function VirtualTable({ rows, label = "Virtual rows" }: { rows: TableRow[]; label?: string }) {
  const parent = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({ count: rows.length, getScrollElement: () => parent.current, estimateSize: () => 28 });
  return (
    <div ref={parent} data-token="virtual-table" role="region" tabIndex={0} aria-label={label} className="pane-scroll h-28 overflow-auto">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((item) => (
          <div key={item.key} className="absolute right-0 left-0 flex h-7 items-center px-2 font-mono text-value" style={{ transform: `translateY(${item.start}px)` }}>
            {rows[item.index]?.name}
          </div>
        ))}
      </div>
    </div>
  );
}

export function SectionHeader({ label, action }: { label: string; action?: string }) {
  return (
    <div data-token="section-header" className="flex items-center gap-2">
      <span className="micro-label">{label}</span>
      <span className="bg-border h-px flex-1" />
      {action ? (
        <Button size="xs" variant="ghost">
          {action}
        </Button>
      ) : null}
    </div>
  );
}

export function KeyValueGrid({ pairs }: { pairs: [string, string][] }) {
  return (
    <dl data-token="key-value" className="grid grid-cols-2 gap-x-3 gap-y-1">
      {pairs.map(([key, value]) => (
        <div key={key} className="contents">
          <dt className="micro-label text-muted-foreground">{key}</dt>
          <dd className="text-right font-mono text-value">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function Toolbar({ label }: { label: string }) {
  return (
    <div role="toolbar" aria-label={label} data-token="toolbar" className="flex items-center gap-1">
      <Button size="sm" variant="outline">
        Filter
      </Button>
      <span className="bg-border h-4 w-px" role="separator" />
      <Button size="sm" variant="ghost">
        Export
      </Button>
    </div>
  );
}

export function SplitDetail() {
  return (
    <div data-token="split-detail" className="grid min-h-20 grid-cols-[8rem_1fr] border border-border">
      <ul className="border-border border-r" aria-label="Records">
        <li>
          <button type="button" className="bg-accent w-full px-2 py-1 text-left text-small">
            doctor
          </button>
        </li>
      </ul>
      <div className="p-2 text-small">Store check passed.</div>
    </div>
  );
}

export function LoadingState({ label = "Loading the document" }: { label?: string }) {
  return (
    <div data-token="loading-state" role="status" aria-busy="true" aria-label={label} className="flex flex-col gap-2">
      <div className="bg-muted h-3 w-1/3 rounded-xs" />
      <div className="bg-muted h-3 w-full rounded-xs" />
      <div className="bg-muted h-3 w-2/3 rounded-xs" />
    </div>
  );
}
