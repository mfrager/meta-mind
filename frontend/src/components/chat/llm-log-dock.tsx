import { Fragment, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { ArrowDown, ChevronDown, ChevronUp, Pause, Play, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button.tsx";
import {
  filterRows,
  useLlmLogStore,
  type LlmLogConnection,
  type LlmLogFilter,
  type LlmLogRow,
  type RequestRow,
} from "@/lib/store/llm-log.ts";
import { clampLlmLogHeight, LLM_LOG_MAX_HEIGHT, LLM_LOG_MIN_HEIGHT, usePrefsStore } from "@/lib/store/prefs.ts";
import { useViewportTier } from "@/lib/use-tier.ts";

const KIND_LABEL: Record<LlmLogRow["kind"], string> = {
  request: "REQ",
  delta: "STR",
  response: "RSP",
  error: "ERR",
};

const KIND_CLASS: Record<LlmLogRow["kind"], string> = {
  request: "text-azure-600",
  delta: "text-muted-foreground",
  response: "text-emerald-600",
  error: "text-red-600",
};

function clock(at: number) {
  return new Date(at).toLocaleTimeString("en-GB", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function requestSummary(row: RequestRow) {
  return row.streamText || row.preview || "(empty prompt)";
}

function rowSummary(row: LlmLogRow) {
  switch (row.kind) {
    case "request":
      return requestSummary(row);
    case "delta":
      return row.text;
    case "response":
      return row.text || "(empty reply)";
    case "error":
      return row.error;
  }
}

function rowMeta(row: LlmLogRow) {
  switch (row.kind) {
    case "request": {
      const parts = [row.source, row.purpose, row.model, `${row.messageCount} msgs`, `${row.toolCount} tools`];
      if (row.streamText) {
        parts.push(`${row.streamChars} chars${row.streamTruncated ? " (truncated)" : ""}`);
      }
      return parts.filter(Boolean).join(" · ");
    }
    case "delta":
      return `${row.chars} chars${row.truncated ? " (truncated)" : ""}`;
    case "response":
      return [
        row.model,
        row.status,
        row.cached ? "cached" : null,
        `${row.tokensIn}/${row.tokensOut} tok`,
        `${row.latencyMs} ms`,
        row.toolCalls ? `${row.toolCalls} tool calls` : null,
      ]
        .filter(Boolean)
        .join(" · ");
    case "error":
      return `${row.status} · ${row.latencyMs} ms` + (row.partialChars ? ` · ${row.partialChars} chars` : "");
  }
}

function detailRows(row: LlmLogRow): Array<[string, string]> {
  const rows: Array<[string, string]> = [
    ["seq", String(row.seq)],
    ["call", row.callId],
  ];
  const add = (label: string, value: string | number | boolean | null | undefined) => {
    if (value === null || value === undefined || value === "") return;
    rows.push([label, typeof value === "boolean" ? (value ? "yes" : "no") : String(value)]);
  };

  switch (row.kind) {
    case "request":
      add("trace", row.traceId);
      add("session", row.sessionId);
      add("run", row.runId);
      add("pass", row.pass);
      add("round", row.round);
      add("source", row.source);
      add("purpose", row.purpose);
      add("provider", row.provider);
      add("model", row.model);
      add("requested", row.requested);
      add("streamed", row.streamed);
      add("messages", row.messageCount);
      add("tools", row.toolCount);
      add("prompt", row.preview);
      break;
    case "delta":
      add("chars", row.chars);
      add("truncated", row.truncated);
      add("text", row.text);
      break;
    case "response":
      add("model", row.model);
      add("status", row.status);
      add("cached", row.cached);
      add("tokens in", row.tokensIn);
      add("tokens out", row.tokensOut);
      add("latency", `${row.latencyMs} ms`);
      add("tool calls", row.toolCalls);
      add("reply", row.text);
      break;
    case "error":
      add("status", row.status);
      add("latency", `${row.latencyMs} ms`);
      add("partial chars", row.partialChars);
      add("error", row.error);
      break;
  }
  return rows;
}

function ConnectionLabel({ connection, error }: { connection: LlmLogConnection; error: string | null }) {
  if (connection === "error") {
    return (
      <span className="inline-block max-w-40 truncate align-bottom text-red-600" title={error ?? undefined}>
        {error ?? "error"}
      </span>
    );
  }
  const label =
    connection === "live"
      ? "live"
      : connection === "reconnecting"
        ? "reconnecting"
        : connection === "loading"
          ? "loading"
          : connection === "idle"
            ? "idle"
            : "connecting";
  const dot =
    connection === "live"
      ? "bg-emerald-500"
      : connection === "reconnecting" || connection === "connecting"
        ? "bg-amber-500"
        : "bg-muted-foreground/50";
  return (
    <span className="text-muted-foreground inline-flex items-center gap-1">
      <span aria-hidden className={`size-1.5 rounded-pill ${dot}`} />
      {label}
    </span>
  );
}

export function LlmLogDock() {
  const rows = useLlmLogStore((state) => state.rows);
  const connection = useLlmLogStore((state) => state.connection);
  const error = useLlmLogStore((state) => state.error);
  const paused = useLlmLogStore((state) => state.paused);
  const pausedCount = useLlmLogStore((state) => state.pausedCount);
  const filter = useLlmLogStore((state) => state.filter);
  const setFilter = useLlmLogStore((state) => state.setFilter);
  const setPaused = useLlmLogStore((state) => state.setPaused);
  const clear = useLlmLogStore((state) => state.clear);

  const collapsed = usePrefsStore((state) => state.llmLogCollapsed);
  const height = usePrefsStore((state) => state.llmLogHeight);
  const setHeight = usePrefsStore((state) => state.setLlmLogHeight);
  const toggleCollapsed = usePrefsStore((state) => state.toggleLlmLogCollapsed);
  const tier = useViewportTier();
  const compact = tier === "narrow" || tier === "monitor";

  const visibleRows = useMemo(() => filterRows(rows, filter), [rows, filter]);
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(() => new Set());
  const [atBottom, setAtBottom] = useState(true);
  const [draftHeight, setDraftHeight] = useState<number | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const dock = useRef<HTMLElement>(null);
  const drag = useRef({
    active: false,
    startY: 0,
    startHeight: 0,
    nextHeight: 0,
  });
  const pinned = useRef(true);

  useEffect(() => {
    const element = viewport.current;
    if (!element || collapsed || !pinned.current) return;
    element.scrollTop = element.scrollHeight;
    setAtBottom(true);
  }, [visibleRows, collapsed]);

  useEffect(() => {
    const clampToPane = () => {
      const parentHeight = dock.current?.parentElement?.clientHeight ?? 0;
      if (parentHeight <= 0) return;
      const max = Math.max(LLM_LOG_MIN_HEIGHT, Math.round(parentHeight * 0.4));
      if (usePrefsStore.getState().llmLogHeight > max) setHeight(max);
    };
    clampToPane();
    window.addEventListener("resize", clampToPane);
    return () => window.removeEventListener("resize", clampToPane);
  }, [setHeight]);

  const jumpToLatest = () => {
    const element = viewport.current;
    if (!element) return;
    pinned.current = true;
    setAtBottom(true);
    element.scrollTop = element.scrollHeight;
  };

  const toggleRow = (seq: number) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(seq)) next.delete(seq);
      else next.add(seq);
      return next;
    });

  const startResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const start = draftHeight ?? height;
    drag.current = {
      active: true,
      startY: event.clientY,
      startHeight: start,
      nextHeight: start,
    };
    setDraftHeight(start);
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // Pointer capture is unavailable in some test environments.
    }
  };

  const moveResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!drag.current.active) return;
    const parentHeight = dock.current?.parentElement?.clientHeight ?? window.innerHeight;
    const max = Math.max(LLM_LOG_MIN_HEIGHT, Math.round(parentHeight * 0.4));
    const next = Math.min(
      max,
      Math.max(LLM_LOG_MIN_HEIGHT, drag.current.startHeight + drag.current.startY - event.clientY),
    );
    drag.current.nextHeight = next;
    setDraftHeight(next);
  };

  const endResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!drag.current.active) return;
    drag.current.active = false;
    setHeight(clampLlmLogHeight(drag.current.nextHeight));
    setDraftHeight(null);
    try {
      event.currentTarget.releasePointerCapture(event.pointerId);
    } catch {
      // Capture may already have been released.
    }
  };

  const shownHeight = draftHeight ?? height;
  const empty = rows.length === 0;
  const filteredEmpty = !empty && visibleRows.length === 0;

  return (
    <section
      ref={dock}
      data-llm-log-dock
      data-connection={connection}
      data-filter={filter}
      data-paused={paused ? "true" : "false"}
      data-collapsed={collapsed ? "true" : "false"}
      aria-label="LLM activity"
      style={{ height: collapsed ? 32 : shownHeight }}
      className={`border-border bg-card relative flex shrink-0 flex-col border-t ${
        draftHeight === null ? "transition-[height] duration-150 motion-reduce:transition-none" : ""
      }`}
    >
      {!collapsed ? (
        <div
          role="separator"
          tabIndex={0}
          aria-orientation="horizontal"
          aria-label="Resize LLM activity"
          aria-valuemin={LLM_LOG_MIN_HEIGHT}
          aria-valuemax={LLM_LOG_MAX_HEIGHT}
          aria-valuenow={shownHeight}
          data-llm-log-resize
          className="group bg-border/60 hover:bg-primary/40 focus-visible:bg-primary/60 flex h-1.5 shrink-0 cursor-row-resize touch-none items-center justify-center outline-none transition-colors"
          onPointerDown={startResize}
          onPointerMove={moveResize}
          onPointerUp={endResize}
          onPointerCancel={endResize}
          onKeyDown={(event) => {
            if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
            event.preventDefault();
            setHeight(height + (event.key === "ArrowUp" ? 16 : -16));
          }}
        >
          <span
            aria-hidden
            className="bg-foreground/20 group-hover:bg-primary/70 h-0.5 w-8 rounded-full transition-colors"
          />
        </div>
      ) : null}
      <header
        className="border-border flex h-8 shrink-0 items-center gap-1.5 border-b px-2"
        onDoubleClick={(event) => {
          const target = event.target;
          if (target instanceof HTMLElement && target.closest("button, select, input")) return;
          toggleCollapsed();
        }}
      >
        <h2 className="text-small shrink-0 font-medium">LLM activity</h2>
        {!compact ? (
          <span className="text-muted-foreground shrink-0 font-mono text-[11px]">
            {visibleRows.length}/{rows.length}
          </span>
        ) : null}
        <span role="status" aria-live="polite" className="ml-auto shrink-0 text-[11px]">
          <ConnectionLabel connection={connection} error={error} />
        </span>
        <select
          aria-label="Filter activity"
          value={filter}
          onChange={(event) => {
            setFilter(event.target.value as LlmLogFilter);
            pinned.current = true;
            setAtBottom(true);
          }}
          className={`border-border bg-card h-6 max-w-24 shrink-0 rounded-md border px-1 text-[11px] ${
            collapsed ? "hidden" : ""
          }`}
        >
          <option value="all">All</option>
          <option value="requests">Requests</option>
          <option value="streams">Streams</option>
          <option value="responses">Responses</option>
          <option value="errors">Errors</option>
        </select>
        <Button
          variant="ghost"
          size="xs"
          className={`shrink-0 ${collapsed ? "hidden" : ""}`}
          aria-label={paused ? `Resume${pausedCount ? ` (${pausedCount})` : ""}` : "Pause"}
          title={paused ? "Resume the live feed" : "Pause the live feed"}
          onClick={() => setPaused(!paused)}
        >
          {paused ? <Play className="size-3" aria-hidden /> : <Pause className="size-3" aria-hidden />}
          {!compact ? (paused ? `Resume${pausedCount ? ` (${pausedCount})` : ""}` : "Pause") : null}
        </Button>
        <Button
          variant="ghost"
          size="xs"
          className={`shrink-0 ${collapsed ? "hidden" : ""}`}
          aria-label="Clear activity"
          title="Clear the visible activity"
          disabled={empty}
          onClick={clear}
        >
          <Trash2 className="size-3" aria-hidden />
          {!compact ? "Clear" : null}
        </Button>
        <Button
          variant="ghost"
          size="xs"
          className="shrink-0"
          aria-label={collapsed ? "Expand LLM activity" : "Collapse LLM activity"}
          aria-expanded={!collapsed}
          aria-controls="llm-log-body"
          onClick={toggleCollapsed}
        >
          {collapsed ? <ChevronUp className="size-3" aria-hidden /> : <ChevronDown className="size-3" aria-hidden />}
        </Button>
      </header>
      {!collapsed ? (
        <div id="llm-log-body" className="relative min-h-0 flex-1">
          {connection === "loading" && empty ? (
            <p role="status" className="text-muted-foreground p-3 text-xs">
              Loading recent activity…
            </p>
          ) : connection === "error" && empty ? (
            <p role="alert" className="text-red-600 p-3 text-xs">
              {error ?? "The LLM activity feed is unavailable."}
            </p>
          ) : (connection === "reconnecting" || connection === "connecting") && empty ? (
            <p role="status" className="text-muted-foreground p-3 text-xs">
              {connection === "reconnecting"
                ? "Reconnecting to the activity feed…"
                : "Connecting to the activity feed…"}
            </p>
          ) : empty ? (
            <p role="status" className="text-muted-foreground p-3 text-xs">
              No model activity yet.
            </p>
          ) : filteredEmpty ? (
            <p role="status" className="text-muted-foreground p-3 text-xs">
              No activity matches this filter.
            </p>
          ) : (
            <>
              <div
                ref={viewport}
                role="log"
                aria-live="off"
                className="h-full overflow-auto font-mono text-[11px]"
                onScroll={(event) => {
                  const element = event.currentTarget;
                  const bottom = element.scrollHeight - element.scrollTop - element.clientHeight < 24;
                  pinned.current = bottom;
                  setAtBottom(bottom);
                }}
              >
                <ul>
                  {visibleRows.map((row) => {
                    const open = expanded.has(row.seq);
                    const details = open ? detailRows(row) : [];
                    return (
                      <li
                        key={`${row.kind}-${row.seq}`}
                        data-kind={row.kind}
                        data-llm-log-row
                        data-call-id={row.callId}
                        className="border-border/60 border-b last:border-b-0"
                      >
                        <button
                          type="button"
                          aria-expanded={open}
                          onClick={() => toggleRow(row.seq)}
                          className="hover:bg-muted/50 flex w-full min-w-0 items-baseline gap-2 px-2 py-1 text-left"
                        >
                          <span className="text-muted-foreground shrink-0 tabular-nums">{clock(row.at)}</span>
                          <span className={`w-7 shrink-0 font-semibold ${KIND_CLASS[row.kind]}`}>
                            {KIND_LABEL[row.kind]}
                          </span>
                          <span className="min-w-0 flex-1">
                            <span className="block truncate" title={rowSummary(row)}>
                              {rowSummary(row)}
                            </span>
                            <span className="text-muted-foreground block truncate" title={rowMeta(row)}>
                              {rowMeta(row)}
                            </span>
                          </span>
                          <span className="text-muted-foreground shrink-0">
                            {open ? (
                              <ChevronUp className="size-3" aria-hidden />
                            ) : (
                              <ChevronDown className="size-3" aria-hidden />
                            )}
                          </span>
                        </button>
                        {open ? (
                          <dl className="text-muted-foreground grid grid-cols-[5rem_1fr] gap-x-2 gap-y-0.5 px-2 pb-2 pl-11">
                            {details.map(([label, value]) => (
                              <Fragment key={label}>
                                <dt className="shrink-0">{label}</dt>
                                <dd className="text-foreground min-w-0 break-words whitespace-pre-wrap">{value}</dd>
                              </Fragment>
                            ))}
                          </dl>
                        ) : null}
                      </li>
                    );
                  })}
                </ul>
              </div>
              {!atBottom ? (
                <Button
                  variant="outline"
                  size="xs"
                  className="absolute right-2 bottom-2 shadow-none"
                  onClick={jumpToLatest}
                >
                  <ArrowDown className="size-3" aria-hidden />
                  Jump to latest
                </Button>
              ) : null}
            </>
          )}
        </div>
      ) : null}
    </section>
  );
}
