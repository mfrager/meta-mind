import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import { Button } from "@/components/ui/actions.tsx";
import { CallViewer } from "@/components/agentstream/CallViewer.tsx";
import {
  filterRecords,
  groupRecords,
  useAgentstreamStore,
  type Connection,
  type DockFilter,
} from "@/lib/store/agentstream.ts";
import { clampLlmLogHeight, LLM_LOG_MIN_HEIGHT, usePrefsStore } from "@/lib/store/prefs.ts";
import { callIdOf, clock, copyText, dataOf, isErrorEvent, passOf, recordSummary } from "@/lib/transcript/model.ts";
import type { Event } from "@/lib/wire/agentstream.ts";

const FILTERS: Array<{ id: DockFilter; label: string }> = [
  { id: "all", label: "All" },
  { id: "turns", label: "Turns" },
  { id: "passes", label: "Passes" },
  { id: "calls", label: "Calls" },
  { id: "tools", label: "Tools" },
  { id: "verification", label: "Verification" },
  { id: "internals", label: "Internals" },
  { id: "errors", label: "Errors" },
];

export function AgentstreamDock() {
  const records = useAgentstreamStore((state) => state.records);
  const deltas = useAgentstreamStore((state) => state.deltas);
  const lastSeq = useAgentstreamStore((state) => state.lastSeq);
  const connection = useAgentstreamStore((state) => state.connection);
  const error = useAgentstreamStore((state) => state.error);
  const paused = useAgentstreamStore((state) => state.paused);
  const pending = useAgentstreamStore((state) => state.pending.length);
  const filter = useAgentstreamStore((state) => state.filter);
  const setFilter = useAgentstreamStore((state) => state.setFilter);
  const setPaused = useAgentstreamStore((state) => state.setPaused);
  const clear = useAgentstreamStore((state) => state.clear);

  const collapsed = usePrefsStore((state) => state.llmLogCollapsed);
  const height = usePrefsStore((state) => state.llmLogHeight);
  const setHeight = usePrefsStore((state) => state.setLlmLogHeight);
  const toggleCollapsed = usePrefsStore((state) => state.toggleLlmLogCollapsed);

  const [expanded, setExpanded] = useState<ReadonlySet<number>>(() => new Set());
  const [resizing, setResizing] = useState(false);
  const drag = useRef({ startY: 0, startHeight: 0 });
  const viewport = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);

  const filtered = useMemo(() => filterRecords(records, filter), [records, filter]);
  const groups = useMemo(() => groupRecords(filtered), [filtered]);

  useEffect(() => {
    const element = viewport.current;
    if (!element || collapsed || paused || !pinned.current) return;
    element.scrollTop = element.scrollHeight;
  }, [filtered, collapsed, paused]);

  useEffect(() => {
    if (!resizing) return;
    const move = (event: PointerEvent) => {
      const next = drag.current.startHeight + drag.current.startY - event.clientY;
      setHeight(clampLlmLogHeight(next));
    };
    const stop = () => setResizing(false);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
    };
  }, [resizing, setHeight]);

  const startResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    drag.current = { startY: event.clientY, startHeight: height };
    setResizing(true);
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // Pointer capture is unavailable in some test environments.
    }
  };

  const toggle = (seq: number) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(seq)) next.delete(seq);
      else next.add(seq);
      return next;
    });

  return (
    <section
      data-agentstream-dock
      aria-label="Agentstream"
      className="border-border bg-card flex shrink-0 flex-col border-t"
      style={{ height: collapsed ? 32 : height }}
    >
      <header className="flex h-8 shrink-0 items-center gap-2 px-2">
        <span className="text-small font-medium">Agentstream</span>
        <ConnectionLabel connection={connection} error={error} />
        <span className="text-muted-foreground font-mono text-xs">
          seq {lastSeq} · {filtered.length} rows
        </span>
        <div className="ml-auto flex items-center gap-1">
          {paused ? <span className="text-small text-amber-700">{pending} paused</span> : null}
          <select
            aria-label="Log filter"
            className="border-input bg-background text-small h-6 rounded-md border px-1"
            value={filter}
            onChange={(event) => setFilter(event.target.value as DockFilter)}
          >
            {FILTERS.map((entry) => (
              <option key={entry.id} value={entry.id}>
                {entry.label}
              </option>
            ))}
          </select>
          <Button type="button" variant="outline" onClick={() => setPaused(!paused)}>
            {paused ? "Resume" : "Pause"}
          </Button>
          <Button type="button" variant="outline" onClick={() => void navigator.clipboard?.writeText(copyText(filtered))}>
            Copy
          </Button>
          <Button type="button" variant="outline" disabled={records.length === 0} onClick={clear}>
            Clear
          </Button>
          <Button type="button" variant="outline" onClick={toggleCollapsed}>
            {collapsed ? "Expand" : "Collapse"}
          </Button>
        </div>
      </header>

      {collapsed ? null : (
        <>
          <div
            role="separator"
            aria-label="Resize agentstream"
            aria-orientation="horizontal"
            aria-valuenow={height}
            aria-valuemin={LLM_LOG_MIN_HEIGHT}
            tabIndex={0}
            data-dock-resize
            className="border-border hover:bg-accent h-1 shrink-0 cursor-row-resize border-t"
            onPointerDown={startResize}
            onKeyDown={(event) => {
              if (event.key === "ArrowUp") {
                event.preventDefault();
                setHeight(height + 24);
              }
              if (event.key === "ArrowDown") {
                event.preventDefault();
                setHeight(height - 24);
              }
            }}
          />
          <div
            ref={viewport}
            data-dock-rows
            className="min-h-0 flex-1 overflow-auto px-2 pb-1"
            onScroll={(event) => {
              const element = event.currentTarget;
              pinned.current = element.scrollHeight - element.scrollTop - element.clientHeight <= 24;
            }}
          >
            {groups.length === 0 ? (
              <p className="text-muted-foreground p-2 text-xs">No records yet.</p>
            ) : (
              groups.map((turn) => (
                <div key={turn.turnId} data-dock-turn={turn.turnId} className="border-border mb-1 border-b pb-1">
                  <p className="text-muted-foreground font-mono text-xs">
                    turn {turn.turnId === "session" ? "session" : turn.turnId.slice(0, 10)}
                  </p>
                  {turn.passes.map((pass) => (
                    <div key={pass.pass || "none"} data-dock-pass={pass.pass || "none"}>
                      {pass.pass ? <p className="text-muted-foreground pl-2 font-mono text-xs">{pass.pass}</p> : null}
                      {pass.rows.map((event) => (
                        <RecordRow
                          key={event.seq}
                          event={event}
                          records={records}
                          deltas={deltas}
                          open={expanded.has(event.seq)}
                          onToggle={() => toggle(event.seq)}
                        />
                      ))}
                    </div>
                  ))}
                </div>
              ))
            )}
          </div>
        </>
      )}
    </section>
  );
}

function ConnectionLabel({ connection, error }: { connection: Connection; error: string | null }) {
  const label =
    connection === "live"
      ? "live"
      : connection === "reconnecting"
        ? "reconnecting"
        : connection === "connecting"
          ? "connecting"
          : connection === "error"
            ? (error ?? "error")
            : "idle";
  const tone =
    connection === "live"
      ? "bg-emerald-500"
      : connection === "reconnecting" || connection === "connecting"
        ? "bg-amber-500"
        : connection === "error"
          ? "bg-red-500"
          : "bg-muted-foreground/50";
  return (
    <span data-connection={connection} className="text-muted-foreground inline-flex items-center gap-1 text-xs">
      <span aria-hidden className={`size-1.5 rounded-pill ${tone}`} />
      <span className="max-w-40 truncate">{label}</span>
    </span>
  );
}

function RecordRow({
  event,
  records,
  deltas,
  open,
  onToggle,
}: {
  event: Event;
  records: Event[];
  deltas: Record<string, string>;
  open: boolean;
  onToggle: () => void;
}) {
  const callId = callIdOf(event);
  const streamed = callId ? deltas[callId] : undefined;
  const hasResponse = callId ? records.some((entry) => entry.type === "llm.response" && callIdOf(entry) === callId) : false;
  const error = isErrorEvent(event);
  const isRequest = event.type === "llm.request";

  return (
    <div data-dock-row={event.seq} data-event-type={event.type}>
      <button
        type="button"
        className="hover:bg-muted flex w-full items-start gap-2 rounded-xs px-1 py-0.5 text-left font-mono text-xs"
        aria-expanded={open}
        onClick={onToggle}
      >
        <span className="text-muted-foreground w-8 shrink-0 text-right">{event.seq}</span>
        <span className="text-muted-foreground w-16 shrink-0">{clock(event.at)}</span>
        <span className={`w-40 shrink-0 ${error ? "text-red-700" : ""}`}>{event.type}</span>
        <span className="min-w-0 flex-1 truncate">{recordSummary(event)}</span>
        {passOf(event) ? <span className="text-muted-foreground">{passOf(event)}</span> : null}
      </button>
      {isRequest && streamed && !hasResponse ? (
        <pre className="text-muted-foreground ml-10 max-h-20 overflow-hidden whitespace-pre-wrap break-words font-mono text-xs">
          {streamed}
        </pre>
      ) : null}
      {open ? (
        isRequest ? (
          <div className="ml-8">
            <CallViewer request={event} records={records} />
          </div>
        ) : (
          <pre className="border-border ml-8 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-xs border p-2 font-mono text-xs">
            {JSON.stringify(dataOf(event), null, 2)}
          </pre>
        )
      ) : null}
    </div>
  );
}
