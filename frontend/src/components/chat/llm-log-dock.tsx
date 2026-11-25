import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { ArrowDown, ChevronDown, ChevronUp, Copy, Pause, Play, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button.tsx";
import {
  filterGroups,
  groupRows,
  useLlmLogStore,
  type LlmLogConnection,
  type LlmLogFilter,
} from "@/lib/store/llm-log.ts";
import {
  clampLlmLogHeight,
  LLM_LOG_MAX_HEIGHT,
  LLM_LOG_MIN_HEIGHT,
  usePrefsStore,
  type LlmLogTab,
} from "@/lib/store/prefs.ts";
import { useViewportTier } from "@/lib/use-tier.ts";
import { textLog } from "./llm-log-format.ts";
import { CallGroupRow } from "./llm-log-row.tsx";
import { LlmLogText } from "./llm-log-text.tsx";

const TABS: Array<{ value: LlmLogTab; label: string }> = [
  { value: "log", label: "Log" },
  { value: "text", label: "Text" },
];

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
  const tab = usePrefsStore((state) => state.llmLogTab);
  const setHeight = usePrefsStore((state) => state.setLlmLogHeight);
  const setTab = usePrefsStore((state) => state.setLlmLogTab);
  const toggleCollapsed = usePrefsStore((state) => state.toggleLlmLogCollapsed);
  const tier = useViewportTier();
  const compact = tier === "narrow" || tier === "monitor";

  const groups = useMemo(() => groupRows(rows), [rows]);
  const visibleGroups = useMemo(() => filterGroups(groups, filter), [groups, filter]);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());
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
  }, [visibleGroups, collapsed, tab]);

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

  const toggleRow = (key: string) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  const copyTextLog = () => {
    // The copy payload is the rows the browser already holds; nothing is fetched.
    void navigator.clipboard?.writeText(textLog(visibleGroups));
  };

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
  const filteredEmpty = !empty && visibleGroups.length === 0;

  return (
    <section
      ref={dock}
      data-llm-log-dock
      data-connection={connection}
      data-filter={filter}
      data-tab={tab}
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
            {visibleGroups.length} calls / {rows.length} events
          </span>
        ) : null}
        <span role="status" aria-live="polite" className="ml-auto shrink-0 text-[11px]">
          <ConnectionLabel connection={connection} error={error} />
        </span>
        <div
          role="tablist"
          aria-label="LLM activity view"
          className={`shrink-0 items-center gap-2 ${collapsed ? "hidden" : "flex"}`}
        >
          {TABS.map((item) => (
            <button
              key={item.value}
              type="button"
              role="tab"
              aria-selected={tab === item.value}
              onClick={() => {
                setTab(item.value);
                pinned.current = true;
                setAtBottom(true);
              }}
              className={`h-8 border-b-2 px-1 text-[11px] ${
                tab === item.value
                  ? "border-azure-500 text-foreground"
                  : "text-muted-foreground border-transparent"
              }`}
            >
              {item.label}
            </button>
          ))}
        </div>
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
        {tab === "text" ? (
          <Button
            variant="ghost"
            size="xs"
            className={`shrink-0 ${collapsed ? "hidden" : ""}`}
            aria-label="Copy text log"
            title="Copy the text log"
            disabled={visibleGroups.length === 0}
            onClick={copyTextLog}
          >
            <Copy className="size-3" aria-hidden />
            {!compact ? "Copy" : null}
          </Button>
        ) : null}
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
                aria-label={tab === "text" ? "LLM activity text log" : "LLM activity log"}
                className="h-full overflow-auto font-mono text-[11px]"
                onScroll={(event) => {
                  const element = event.currentTarget;
                  const bottom = element.scrollHeight - element.scrollTop - element.clientHeight < 24;
                  pinned.current = bottom;
                  setAtBottom(bottom);
                }}
              >
                {tab === "text" ? (
                  <LlmLogText groups={visibleGroups} />
                ) : (
                  <ul role="tree" aria-label="LLM activity calls">
                    {visibleGroups.map((group) => (
                      <CallGroupRow
                        key={group.callId}
                        group={group}
                        level={1}
                        expanded={expanded}
                        onToggle={toggleRow}
                      />
                    ))}
                  </ul>
                )}
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
