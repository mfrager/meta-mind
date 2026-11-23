import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDownIcon, BanIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { cancelAgentRun, eventRole, fetchAgentRuns } from "@/lib/api/agentstream.ts";
import { fetchTraces } from "@/lib/api/sessions.ts";
import { useTraceStream } from "@/hooks/use-trace-stream.ts";
import type { AgentstreamEvent, SessionTrace } from "@/lib/wire/inspector.ts";
import { clockAt, errorMessage, shortId } from "@/lib/inspector/format.ts";
import { cn } from "@/lib/utils";

import { Chip, UnavailableRow } from "./section.tsx";
import { TraceEventRow } from "./trace-event-row.tsx";

type Filter = "all" | "messages" | "tools" | "summaries" | "errors";

type Row =
  | { kind: "turn"; key: string; turnId: string; count: number; start: number; end: number }
  | { kind: "event"; key: string; event: AgentstreamEvent };

function isError(event: AgentstreamEvent) {
  if (event.payload && typeof event.payload === "object" && !Array.isArray(event.payload)) {
    const payload = event.payload as Record<string, unknown>;
    if (payload.ok === false) return true;
    if (typeof payload.error === "string" && payload.error) return true;
    if (["failed", "error", "denied"].includes(String(payload.status ?? ""))) return true;
  }
  return event.event_type === "agent_status" && /fail|error/i.test(JSON.stringify(event.payload));
}

function matchesFilter(event: AgentstreamEvent, filter: Filter) {
  switch (filter) {
    case "all":
      return true;
    case "messages":
      return ["user_message", "agent_message", "agent_turn", "barge_in", "proactive"].includes(event.event_type);
    case "tools":
      return event.event_type === "tool_call" || event.event_type === "tool_result";
    case "summaries":
      return event.event_type === "agent_turn_summary";
    case "errors":
      return isError(event);
  }
}

function buildRows(events: AgentstreamEvent[], filter: Filter, turn: string): Row[] {
  const rows: Row[] = [];
  let currentTurn: string | null = null;
  let bucket: AgentstreamEvent[] = [];
  const flush = () => {
    if (!currentTurn || bucket.length === 0) return;
    rows.push({
      kind: "turn",
      key: `turn:${currentTurn}`,
      turnId: currentTurn,
      count: bucket.length,
      start: bucket[0].timestamp,
      end: bucket[bucket.length - 1].timestamp,
    });
    for (const event of bucket) rows.push({ kind: "event", key: event.event_id, event });
    bucket = [];
  };
  for (const event of events) {
    if (!matchesFilter(event, filter)) continue;
    if (turn !== "all" && (event.turn_id ?? "unassigned") !== turn) continue;
    const id = event.turn_id ?? "unassigned";
    if (id !== currentTurn) {
      flush();
      currentTurn = id;
    }
    bucket.push(event);
  }
  flush();
  return rows;
}

function TraceTurnHeader({ row }: { row: Extract<Row, { kind: "turn" }> }) {
  return (
    <div className="border-border/70 bg-surface/80 flex items-center gap-2 border-b px-2 py-1">
      <span className="text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
        Turn {row.turnId}
      </span>
      <span className="font-mono text-[10px] text-muted-foreground">{row.count} events</span>
      <span className="ml-auto font-mono text-[10px] text-muted-foreground">
        {clockAt(row.start)} · {Math.max(0, row.end - row.start)}ms
      </span>
    </div>
  );
}

function traceLabel(trace: SessionTrace): string {
  const kind = trace.kind === "main" ? "CM" : trace.kind;
  return `${kind} · ${shortId(trace.traceId)} · ${trace.eventCount} events`;
}

export function TraceViewer({
  sessionId,
  selectedTraceId,
  onSelectTrace,
}: {
  sessionId: string;
  selectedTraceId: string | null;
  onSelectTrace: (traceId: string | null) => void;
}) {
  const queryClient = useQueryClient();
  const traces = useQuery({
    queryKey: ["inspector", "traces", sessionId],
    queryFn: () => fetchTraces(sessionId, { limit: 100 }),
  });
  const items = traces.data?.items ?? [];
  const selected =
    items.find((trace) => trace.traceId === selectedTraceId) ??
    items.find((trace) => trace.kind === "main") ??
    items[0] ??
    null;
  const trace = useTraceStream(selected?.agentstreamId ?? null, Boolean(selected && !selected.endedAt));
  const events = trace.events;

  const [filter, setFilter] = useState<Filter>("all");
  const [turn, setTurn] = useState("all");
  const [raw, setRaw] = useState(false);
  const [follow, setFollow] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setTurn("all");
  }, [selected?.traceId]);

  const turns = useMemo(() => {
    const ids: string[] = [];
    for (const event of events) {
      const id = event.turn_id ?? "unassigned";
      if (!ids.includes(id)) ids.push(id);
    }
    return ids;
  }, [events]);

  const rows = useMemo(() => buildRows(events, filter, turn), [events, filter, turn]);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => (rows[index]?.kind === "turn" ? 26 : 54),
    overscan: 12,
    getItemKey: (index) => rows[index]?.key ?? index,
  });
  const virtualizerRef = useRef(virtualizer);
  virtualizerRef.current = virtualizer;

  useEffect(() => {
    if (!follow || rows.length === 0) return;
    virtualizerRef.current.scrollToIndex(rows.length - 1, { align: "end" });
  }, [follow, rows.length, selected?.traceId]);

  const agents = useQuery({
    queryKey: ["inspector", "agents", sessionId],
    queryFn: () => fetchAgentRuns(sessionId),
  });
  const matchingRun = selected
    ? agents.data?.items.find(
        (run) =>
          run.agentstream_id === selected.agentstreamId ||
          run.run_id === selected.traceId ||
          selected.agentstreamId === `agent/${run.run_id}`,
      )
    : null;
  const cancel = useMutation({
    mutationFn: cancelAgentRun,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["inspector", "agents", sessionId] });
      void queryClient.invalidateQueries({ queryKey: ["inspector", "traces", sessionId] });
      void queryClient.invalidateQueries({ queryKey: ["inspector", "trace"] });
    },
  });

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-panel="trace">
      <div className="border-border/70 flex flex-col gap-1 border-b px-2 py-1.5">
        <div className="flex items-center gap-1.5">
          <select
            aria-label="Trace session"
            className="border-input bg-background h-6 min-w-0 flex-1 rounded-sm border px-1 font-mono text-[10px]"
            onChange={(event) => onSelectTrace(event.target.value || null)}
            value={selected?.traceId ?? ""}
          >
            {items.length === 0 ? <option value="">No traces</option> : null}
            {items.map((item) => (
              <option key={item.traceId} value={item.traceId}>
                {traceLabel(item)}
              </option>
            ))}
          </select>
          {selected ? (
            <Chip tone={selected.endedAt ? "neutral" : trace.connection === "live" ? "live" : "warn"}>
              {selected.endedAt ? "ended" : trace.connection === "live" ? "live" : "connecting"}
            </Chip>
          ) : null}
          {matchingRun && !["done", "failed", "cancelled"].includes(matchingRun.status) ? (
            <button
              className="text-muted-foreground hover:text-destructive inline-flex items-center gap-1 text-[10px] disabled:opacity-50"
              disabled={cancel.isPending}
              onClick={() => cancel.mutate(matchingRun.run_id)}
              type="button"
            >
              <BanIcon className="size-3" /> Cancel
            </button>
          ) : null}
        </div>
        {selected ? (
          <div className="flex flex-wrap items-center gap-1 font-mono text-[10px] text-muted-foreground">
            <span>{selected.kind}</span>
            <span>·</span>
            <span>{selected.eventCount} events</span>
            <span>·</span>
            <span>started {clockAt(selected.startedAt)}</span>
            {selected.endedAt ? (
              <>
                <span>·</span>
                <span>ended {clockAt(selected.endedAt)}</span>
              </>
            ) : null}
          </div>
        ) : null}
        <div className="flex flex-wrap items-center gap-1">
          <select
            aria-label="Trace event filter"
            className="border-input bg-background h-6 rounded-sm border px-1 text-[10px]"
            onChange={(event) => setFilter(event.target.value as Filter)}
            value={filter}
          >
            <option value="all">all</option>
            <option value="messages">messages</option>
            <option value="tools">tools</option>
            <option value="summaries">summaries</option>
            <option value="errors">errors</option>
          </select>
          <select
            aria-label="Trace turn filter"
            className="border-input bg-background h-6 rounded-sm border px-1 text-[10px]"
            onChange={(event) => setTurn(event.target.value)}
            value={turn}
          >
            <option value="all">all turns</option>
            {turns.map((id) => (
              <option key={id} value={id}>
                turn {id}
              </option>
            ))}
          </select>
          <label className="flex items-center gap-1 text-[10px] text-muted-foreground">
            <input checked={follow} onChange={(event) => setFollow(event.target.checked)} type="checkbox" />
            follow
          </label>
          <label className="flex items-center gap-1 text-[10px] text-muted-foreground">
            <input checked={raw} onChange={(event) => setRaw(event.target.checked)} type="checkbox" />
            raw
          </label>
          {!follow && rows.length ? (
            <button
              className="ml-auto inline-flex items-center gap-1 text-[10px] text-primary"
              onClick={() => setFollow(true)}
              type="button"
            >
              <ArrowDownIcon className="size-3" /> Jump to live
            </button>
          ) : null}
        </div>
      </div>

      <div
        className="min-h-0 flex-1 overflow-auto"
        onScroll={(event) => {
          const element = event.currentTarget;
          const atBottom = element.scrollHeight - element.scrollTop - element.clientHeight < 48;
          if (atBottom !== follow) setFollow(atBottom);
        }}
        ref={scrollRef}
      >
        {traces.isError ? (
          <div className="p-2">
            <UnavailableRow section="trace" reason={errorMessage(traces.error)} />
          </div>
        ) : null}
        {trace.isError ? (
          <div className="p-2">
            <UnavailableRow section="trace events" reason={errorMessage(trace.error)} />
          </div>
        ) : null}
        {!traces.isPending && items.length === 0 ? (
          <p className="p-3 text-[12px] text-muted-foreground">
            This session has no traces yet. The main trace appears when the conversation starts.
          </p>
        ) : null}
        {selected && !trace.isPending && events.length === 0 ? (
          <p className="p-3 text-[12px] text-muted-foreground">This trace has no events.</p>
        ) : null}
        {selected && events.length > 0 && rows.length === 0 ? (
          <p className="p-3 text-[12px] text-muted-foreground">No events match this filter.</p>
        ) : null}
        <div className="relative w-full" style={{ height: `${virtualizer.getTotalSize()}px` }}>
          {virtualizer.getVirtualItems().map((item) => {
            const row = rows[item.index];
            if (!row) return null;
            return (
              <div
                className={cn("absolute top-0 left-0 w-full", row.kind === "turn" && "z-[1]")}
                data-index={item.index}
                key={row.key}
                ref={virtualizer.measureElement}
                style={{ transform: `translateY(${item.start}px)` }}
              >
                {row.kind === "turn" ? <TraceTurnHeader row={row} /> : <TraceEventRow event={row.event} raw={raw} />}
              </div>
            );
          })}
        </div>
        {cancel.isError ? (
          <div className="p-2">
            <UnavailableRow section="cancel" reason={errorMessage(cancel.error)} />
          </div>
        ) : null}
      </div>
      {selected && events.length ? (
        <div className="border-border/70 flex shrink-0 items-center gap-2 border-t px-2 py-1 font-mono text-[10px] text-muted-foreground">
          <span>{rows.length} rows</span>
          <span>·</span>
          <span>{events.filter((event) => eventRole(event) === "tool").length} tool events</span>
          <span className="ml-auto">{selected.traceId}</span>
        </div>
      ) : null}
    </div>
  );
}
