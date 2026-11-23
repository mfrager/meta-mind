"use client";

import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDownIcon, Loader2Icon, RefreshCwIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import type { Artifact } from "@/agentgui/types/workspace";
import { cn, formatBytes } from "@/agentgui/lib/utils";
import { TraceEventRow } from "@/components/inspector/trace-event-row";
import { useTraceStream } from "@/hooks/use-trace-stream";
import { fetchTraces } from "@/lib/api/sessions";

function clock(at: number): string {
  return new Date(at).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function TraceLogView({ artifact }: { artifact: Artifact }) {
  const sessionId = artifact.sessionId ?? "";
  const traceId = artifact.traceId ?? "";
  const traces = useQuery({
    queryKey: ["agentgui", "session-traces", sessionId],
    queryFn: () => fetchTraces(sessionId, { limit: 100 }),
    enabled: Boolean(sessionId),
  });
  const meta = traces.data?.items.find((trace) => trace.traceId === traceId);
  const agentstreamId = artifact.agentstreamId ?? meta?.agentstreamId ?? null;
  const trace = useTraceStream(agentstreamId, Boolean(meta && !meta.endedAt));
  const events = trace.events;

  const [raw, setRaw] = useState(false);
  const [follow, setFollow] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: events.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 54,
    overscan: 12,
    getItemKey: (index) => events[index]?.event_id ?? index,
  });
  const virtualizerRef = useRef(virtualizer);
  virtualizerRef.current = virtualizer;

  useEffect(() => {
    if (!follow || events.length === 0) return;
    virtualizerRef.current.scrollToIndex(events.length - 1, { align: "end" });
  }, [follow, events.length, traceId]);

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-view="trace-log">
      <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border/70 bg-surface-muted/40 px-3 py-1.5">
        <span className="truncate font-mono text-[10.5px] text-muted-foreground" title={traceId}>
          {traceId}
        </span>
        {meta ? (
          <span className="font-mono text-[10px] text-muted-foreground">
            · {meta.kind} · {meta.eventCount} events · {formatBytes(meta.sizeBytes)} · started{" "}
            {clock(meta.startedAt)}
          </span>
        ) : null}
        <span
          className={cn(
            "rounded border px-1 font-mono text-[9.5px]",
            meta && !meta.endedAt
              ? "border-primary/25 text-primary"
              : "border-border/70 text-muted-foreground"
          )}
        >
          {meta ? (meta.endedAt ? "ended" : "live") : "…"}
        </span>
        <div className="ml-auto flex items-center gap-2">
          <label className="flex items-center gap-1 text-[10px] text-muted-foreground">
            <input
              checked={follow}
              onChange={(event) => setFollow(event.target.checked)}
              type="checkbox"
            />
            follow
          </label>
          <label className="flex items-center gap-1 text-[10px] text-muted-foreground">
            <input checked={raw} onChange={(event) => setRaw(event.target.checked)} type="checkbox" />
            raw
          </label>
          <button
            aria-label="Reload trace"
            className="text-muted-foreground hover:text-foreground"
            onClick={() => void traces.refetch()}
            type="button"
          >
            <RefreshCwIcon className={cn("size-3", traces.isFetching && "animate-spin")} />
          </button>
        </div>
      </div>

      {traces.isError ? (
        <p className="p-3 text-[12px] text-red-700 dark:text-red-300">
          {traces.error instanceof Error ? traces.error.message : "The trace list did not load."}
        </p>
      ) : null}
      {trace.isError ? (
        <p className="p-3 text-[12px] text-red-700 dark:text-red-300">
          {trace.error instanceof Error ? trace.error.message : "The trace events did not load."}
        </p>
      ) : null}
      {!traces.isPending && !agentstreamId ? (
        <p className="p-3 text-[12px] text-muted-foreground">
          This session does not expose an agentstream id for the selected log.
        </p>
      ) : null}
      {agentstreamId && !trace.isPending && events.length === 0 ? (
        <div className="grid flex-1 place-items-center p-6 text-center">
          <div>
            <Loader2Icon className="mx-auto size-4 animate-spin text-muted-foreground" />
            <p className="mt-2 text-[12px] text-muted-foreground">
              {meta?.endedAt
                ? "This log has no events."
                : "Waiting for the first event on this log."}
            </p>
          </div>
        </div>
      ) : null}

      <div
        className="min-h-0 flex-1 overflow-auto"
        onScroll={(event) => {
          const element = event.currentTarget;
          const atBottom = element.scrollHeight - element.scrollTop - element.clientHeight < 48;
          if (atBottom !== follow) setFollow(atBottom);
        }}
        ref={scrollRef}
      >
        <div className="relative w-full" style={{ height: `${virtualizer.getTotalSize()}px` }}>
          {virtualizer.getVirtualItems().map((item) => {
            const event = events[item.index];
            if (!event) return null;
            return (
              <div
                className="absolute top-0 left-0 w-full"
                data-index={item.index}
                key={event.event_id}
                ref={virtualizer.measureElement}
                style={{ transform: `translateY(${item.start}px)` }}
              >
                <TraceEventRow event={event} raw={raw} />
              </div>
            );
          })}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-2 border-t border-border/70 px-3 py-1 font-mono text-[10px] text-muted-foreground">
        <span>{events.length} events</span>
        <span>·</span>
        <span>{trace.connection}</span>
        {!follow ? (
          <button
            className="ml-auto inline-flex items-center gap-1 text-primary"
            onClick={() => setFollow(true)}
            type="button"
          >
            <ArrowDownIcon className="size-3" /> Latest
          </button>
        ) : null}
      </div>
    </div>
  );
}
