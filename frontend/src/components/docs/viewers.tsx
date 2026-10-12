import { lazy, Suspense, useState, type ReactNode } from "react";

import type { ChartSeries } from "@/components/charts/export.ts";
import { LogDocument } from "@/components/docs/log-doc.tsx";
import { TreeCore } from "@/components/trees/tree-core.tsx";
import { DataTable, GaugeArc, StageTicker } from "@/components/kit/data.tsx";
import { ProofCard } from "@/components/kit/cards.tsx";
import { IconChip } from "@/components/kit/icon-chip.tsx";
import { Textarea } from "@/components/ui/controls.tsx";
import { fetchDoc } from "@/lib/api/focus.ts";
import { useStageTicks } from "@/lib/live/stages.ts";
import { useDocuments } from "@/lib/store/documents.ts";
import type { DocKind, FocusDoc } from "@/lib/wire/focus.ts";

const ChartRecipe = lazy(() => import("@/components/charts/recipes.tsx").then((mod) => ({ default: mod.ChartRecipe })));
const MapView = lazy(() => import("@/components/mindmap/map-view.tsx").then((mod) => ({ default: mod.MapView })));

function PiecePending() {
  return <p className="text-small text-muted-foreground">Loading…</p>;
}

function asRecord(snapshot: unknown): Record<string, unknown> {
  return snapshot && typeof snapshot === "object" ? (snapshot as Record<string, unknown>) : {};
}

function textOf(value: unknown) {
  return typeof value === "string" ? value : "";
}

export function DocBody({ doc, editing, draft, onDraft }: { doc: FocusDoc; editing: boolean; draft: string; onDraft: (value: string) => void }) {
  const reason = doc.reason ?? "";
  const announced =
    doc.status === "unavailable" || doc.status === "degraded" || doc.status === "failed" || doc.status === "blocked";
  return (
    <div className="flex flex-col gap-3 p-4" data-doc-kind={doc.kind} data-doc-status={doc.status}>
      {reason && (announced || doc.status === "saved") ? <p data-doc-reason className="text-body">{reason}</p> : null}
      <KindBody doc={doc} editing={editing} draft={draft} onDraft={onDraft} />
    </div>
  );
}

function KindBody({ doc, editing, draft, onDraft }: { doc: FocusDoc; editing: boolean; draft: string; onDraft: (value: string) => void }) {
  const snapshot = asRecord(doc.snapshot);
  switch (doc.kind) {
    case "prose":
      return editing ? (
        <Textarea aria-label="Markdown" value={draft} onChange={(event) => onDraft(event.target.value)} className="min-h-48 font-mono" />
      ) : (
        <article className="text-body max-w-prose whitespace-pre-wrap">{textOf(snapshot.markdown)}</article>
      );
    case "code":
      return editing && snapshot.language !== "json" ? (
        <Textarea aria-label="Source" value={draft} onChange={(event) => onDraft(event.target.value)} className="min-h-48 font-mono" />
      ) : (
        <pre className="bg-muted overflow-auto rounded-xs p-3 font-mono text-value" data-code-language={textOf(snapshot.language)}>
          {textOf(snapshot.text)}
        </pre>
      );
    case "chart":
      return (
        <div className="flex flex-col gap-2">
          <p className="text-small text-muted-foreground">{textOf(snapshot.note)}</p>
          <Suspense fallback={<PiecePending />}>
            <ChartRecipe recipe={textOf(snapshot.recipe)} series={(snapshot.series as ChartSeries[]) ?? []} />
          </Suspense>
        </div>
      );
    case "log":
      return <LogDocument count={Number(snapshot.count ?? 0)} />;
    case "table":
      return <DataTable rows={(snapshot.rows as { id: string; name: string; state: string }[]) ?? []} />;
    case "diff":
      return <DiffView before={textOf(snapshot.before)} after={textOf(snapshot.after)} />;
    case "proof":
      return (
        <div>
          <ProofCard />
          <ol className="mt-2 list-decimal pl-5 text-small">
            {((snapshot.steps as string[]) ?? []).map((step) => (
              <li key={step}>{step}</li>
            ))}
          </ol>
          <p className="text-small mt-2">{textOf(snapshot.goal)}</p>
        </div>
      );
    case "form":
      return editing ? (
        <Textarea aria-label="Title" value={draft} onChange={(event) => onDraft(event.target.value)} />
      ) : (
        <p className="text-body">
          {textOf(snapshot.title)} · {textOf(snapshot.owner)}
        </p>
      );
    case "timeline":
      return <Timeline events={(snapshot.events as { at: string; label: string; kind: "run" | "log" }[]) ?? []} />;
    case "run":
      return <RunBody doc={doc} stages={(snapshot.stages as { name: string; state: "pending" | "done" | "active" | "failed" }[]) ?? []} />;
    case "budget":
      return <BudgetBody rows={(snapshot.rows as { kind: string; period: string; limit: number; spent: number }[]) ?? []} />;
    case "tree":
      return typeof snapshot.tree === "string" ? <TreeCore name={snapshot.tree} /> : null;
    case "mindmap":
      return typeof snapshot.map === "string" ? (
        <Suspense fallback={<PiecePending />}>
          <MapView source={snapshot.map} />
        </Suspense>
      ) : null;
    default:
      return <Unsupported kind={doc.kind} />;
  }
}

function RunBody({
  doc,
  stages,
}: {
  doc: FocusDoc;
  stages: { name: string; state: "pending" | "done" | "active" | "failed" }[];
}) {
  const ticks = useStageTicks(doc.id);
  const open = useDocuments((state) => state.open);
  const [trace, setTrace] = useState(false);
  const reason = ticks.at(-1)?.reason ?? doc.reason;
  return (
    <div className="space-y-3" data-run-doc>
      <StageTicker stages={stages} />
      {reason ? <p data-stage-reason>{reason}</p> : null}
      <div className="flex gap-2">
        <button type="button" className="rounded-md border border-border px-2 py-1 text-sm" onClick={() => setTrace((open) => !open)}>
          {trace ? "Stages" : "Trace map"}
        </button>
        <button
          type="button"
          className="rounded-md border border-border px-2 py-1 text-sm"
          onClick={() => void fetchDoc("01jdoctime0000000000000000").then(open)}
        >
          Open boot timeline
        </button>
      </div>
      {trace ? <p data-trace-reason>no trace is recorded</p> : null}
    </div>
  );
}

function DiffView({ before, after }: { before: string; after: string }) {
  return (
    <div className="grid gap-2 md:grid-cols-2">
      <pre className="bg-red-50 rounded-xs p-2 font-mono text-value">{before}</pre>
      <pre className="bg-green-50 rounded-xs p-2 font-mono text-value">{after}</pre>
    </div>
  );
}

function Timeline({ events }: { events: { at: string; label: string; kind: "run" | "log" }[] }) {
  return (
    <ol className="flex flex-col gap-2">
      {events.map((event) => (
        <li key={`${event.at}-${event.label}`} className="flex items-center gap-2">
          <span className="font-mono text-value w-8">{event.at}</span>
          <IconChip kind={event.kind} size={12} />
          <span className="text-small">{event.label}</span>
        </li>
      ))}
    </ol>
  );
}

function BudgetBody({ rows }: { rows: { kind: string; period: string; limit: number; spent: number }[] }) {
  if (rows.length === 0) return null;
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      {rows.map((row) => (
        <div key={`${row.kind}-${row.period}`} className="border-border rounded-xs border p-3">
          <p className="micro-label">{row.kind}</p>
          <GaugeArc value={row.limit === 0 ? 0 : Math.round((row.spent / row.limit) * 100)} label={row.kind} />
          <p className="font-mono text-value mt-1">
            {row.spent} / {row.limit}
          </p>
        </div>
      ))}
    </div>
  );
}

function Unsupported({ kind }: { kind: DocKind }) {
  return <p className="text-small text-muted-foreground">{kind}</p>;
}

export function rawSnapshot(doc: FocusDoc): ReactNode {
  return <pre className="bg-muted overflow-auto rounded-xs p-3 font-mono text-value">{JSON.stringify(doc.snapshot, null, 2)}</pre>;
}
