"use client";

import { ChevronRightIcon, FolderIcon, Loader2Icon, RefreshCwIcon, ScrollTextIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { openTraceInWorkspace } from "@/agentgui/lib/artifacts";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn } from "@/agentgui/lib/utils";
import { fetchFileTree } from "@/lib/api/files";
import type { FileTreeEntry } from "@/lib/wire/inspector";

const ROOT = "/sessions";
const PAGE_SIZE = 50;

type PageState = {
  entries: FileTreeEntry[];
  nextCursor: string | null;
  hasMore: boolean;
  loading: boolean;
  error: string | null;
};

const emptyPage: PageState = {
  entries: [],
  nextCursor: null,
  hasMore: false,
  loading: false,
  error: null,
};

function tracePath(sessionId: string): string {
  return `${ROOT}/${sessionId}/traces`;
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function Row({
  depth,
  expandable,
  open,
  icon,
  label,
  detail,
  title,
  onClick,
}: {
  depth: number;
  expandable?: boolean;
  open?: boolean;
  icon: ReactNode;
  label: string;
  detail?: ReactNode;
  title?: string;
  onClick: () => void;
}) {
  return (
    <button
      className="group/row flex w-full items-center gap-1.5 rounded-md py-1 pr-1.5 text-left transition-colors hover:bg-sidebar-accent/60"
      onClick={onClick}
      style={{ paddingLeft: `${6 + depth * 12}px` }}
      title={title ?? label}
      type="button"
    >
      {expandable ? (
        <ChevronRightIcon
          className={cn(
            "size-3 shrink-0 text-muted-foreground transition-transform",
            open && "rotate-90"
          )}
        />
      ) : (
        <span className="w-3 shrink-0" />
      )}
      {icon}
      <span
        className={cn(
          "min-w-0 flex-1 truncate",
          expandable ? "font-mono text-[12.5px]" : "text-[12.5px]"
        )}
      >
        {label}
        {expandable ? "/" : ""}
      </span>
      {detail}
    </button>
  );
}

export function FilesNav() {
  const query = useWorkspaceStore((state) => state.searchQuery);
  const selectSession = useWorkspaceStore((state) => state.selectSession);
  const [sessions, setSessions] = useState<PageState>(emptyPage);
  const [traces, setTraces] = useState<Record<string, PageState>>({});
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set([ROOT]));
  const inFlight = useRef<Set<string>>(new Set());

  const loadSessions = useCallback(async (cursor?: string | null) => {
    const key = `${ROOT}:${cursor ?? ""}`;
    if (inFlight.current.has(key)) return;
    inFlight.current.add(key);
    setSessions((current) => ({ ...current, loading: true, error: null }));
    try {
      const page = await fetchFileTree(ROOT, cursor ?? null, PAGE_SIZE);
      setSessions((current) => ({
        entries: cursor ? [...current.entries, ...page.entries] : page.entries,
        nextCursor: page.nextCursor,
        hasMore: page.hasMore,
        loading: false,
        error: null,
      }));
    } catch (error: unknown) {
      setSessions((current) => ({ ...current, loading: false, error: messageOf(error) }));
    } finally {
      inFlight.current.delete(key);
    }
  }, []);

  const loadTraces = useCallback(async (sessionId: string, cursor?: string | null) => {
    const path = tracePath(sessionId);
    const key = `${path}:${cursor ?? ""}`;
    if (inFlight.current.has(key)) return;
    inFlight.current.add(key);
    setTraces((current) => ({
      ...current,
      [sessionId]: { ...(current[sessionId] ?? emptyPage), loading: true, error: null },
    }));
    try {
      const page = await fetchFileTree(path, cursor ?? null, PAGE_SIZE);
      setTraces((current) => {
        const previous = current[sessionId] ?? emptyPage;
        return {
          ...current,
          [sessionId]: {
            entries: cursor ? [...previous.entries, ...page.entries] : page.entries,
            nextCursor: page.nextCursor,
            hasMore: page.hasMore,
            loading: false,
            error: null,
          },
        };
      });
    } catch (error: unknown) {
      setTraces((current) => ({
        ...current,
        [sessionId]: {
          ...(current[sessionId] ?? emptyPage),
          loading: false,
          error: messageOf(error),
        },
      }));
    } finally {
      inFlight.current.delete(key);
    }
  }, []);

  useEffect(() => {
    void loadSessions();
  }, [loadSessions]);

  const toggleRoot = () => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(ROOT)) next.delete(ROOT);
      else next.add(ROOT);
      return next;
    });
  };

  const toggleSession = (sessionId: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(sessionId)) next.delete(sessionId);
      else next.add(sessionId);
      return next;
    });
    const page = traces[sessionId];
    if (!page || (!page.entries.length && !page.loading && !page.error)) {
      void loadTraces(sessionId);
    }
  };

  const openTrace = (sessionId: string, entry: FileTreeEntry) => {
    selectSession(sessionId);
    openTraceInWorkspace({
      sessionId,
      traceId: entry.name,
      agentstreamId: entry.agentstreamId,
    });
  };

  const needle = query.trim().toLowerCase();
  const shownSessions = needle
    ? sessions.entries.filter((entry) => entry.name.toLowerCase().includes(needle))
    : sessions.entries;

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-panel="session-files">
      <div className="flex items-center justify-between px-2 pb-2">
        <span className="text-[11px] font-medium tracking-wider text-muted-foreground uppercase">
          Sessions & logs
        </span>
        <div className="flex items-center gap-1.5">
          <span className="font-mono text-[11px] text-muted-foreground/70">
            {sessions.entries.length}
          </span>
          <button
            aria-label="Refresh session logs"
            className="text-muted-foreground hover:text-foreground"
            onClick={() => {
              setSessions(emptyPage);
              setTraces({});
              void loadSessions();
            }}
            type="button"
          >
            <RefreshCwIcon className="size-3" />
          </button>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-1 pb-2">
        <Row
          depth={0}
          expandable
          icon={<FolderIcon className="size-3.5 shrink-0 text-primary" />}
          label="sessions"
          onClick={toggleRoot}
          open={expanded.has(ROOT)}
          detail={
            <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
              {sessions.entries.length}
            </span>
          }
        />

        {expanded.has(ROOT) ? (
          <>
            {sessions.loading && sessions.entries.length === 0 ? (
              <p className="px-2 py-2 text-[12px] text-muted-foreground">
                <Loader2Icon className="mr-1 inline size-3 animate-spin" /> loading sessions…
              </p>
            ) : null}
            {sessions.error ? (
              <p className="px-2 py-2 text-[11px] text-red-700 dark:text-red-300">{sessions.error}</p>
            ) : null}
            {!sessions.loading && !sessions.error && shownSessions.length === 0 ? (
              <p className="px-2 py-2 text-[12px] text-muted-foreground">
                {needle ? `No sessions match “${query}”.` : "No sessions yet."}
              </p>
            ) : null}

            {shownSessions.map((session) => {
              const sessionId = session.name;
              const page = traces[sessionId];
              const open = expanded.has(sessionId);
              return (
                <div key={session.path}>
                  <Row
                    depth={1}
                    detail={
                      page?.entries.length ? (
                        <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                          {page.entries.length}
                          {page.hasMore ? "+" : ""}
                        </span>
                      ) : null
                    }
                    expandable
                    icon={<FolderIcon className="size-3.5 shrink-0 text-muted-foreground" />}
                    label={sessionId}
                    onClick={() => toggleSession(sessionId)}
                    open={open}
                    title={sessionId}
                  />
                  {open ? (
                    <>
                      {page?.loading && page.entries.length === 0 ? (
                        <p className="px-2 py-1 text-[11px] text-muted-foreground" style={{ paddingLeft: 30 }}>
                          <Loader2Icon className="mr-1 inline size-3 animate-spin" /> loading logs…
                        </p>
                      ) : null}
                      {page?.error ? (
                        <p
                          className="px-2 py-1 text-[11px] text-red-700 dark:text-red-300"
                          style={{ paddingLeft: 30 }}
                        >
                          {page.error}
                        </p>
                      ) : null}
                      {page && !page.loading && !page.error && page.entries.length === 0 ? (
                        <p
                          className="px-2 py-1 text-[11px] text-muted-foreground"
                          style={{ paddingLeft: 30 }}
                        >
                          No agentstream logs for this session.
                        </p>
                      ) : null}
                      {page?.entries.map((entry) => (
                        <Row
                          depth={2}
                          icon={<ScrollTextIcon className="size-3.5 shrink-0 text-muted-foreground" />}
                          key={entry.path}
                          label={entry.name}
                          onClick={() => openTrace(sessionId, entry)}
                          detail={
                            typeof entry.eventCount === "number" ? (
                              <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                                {entry.eventCount} ev
                              </span>
                            ) : null
                          }
                        />
                      ))}
                      {page?.hasMore && page.nextCursor ? (
                        <button
                          className="py-1 text-[11px] text-primary hover:underline"
                          onClick={() => void loadTraces(sessionId, page.nextCursor)}
                          style={{ paddingLeft: 30 }}
                          type="button"
                        >
                          Load more logs…
                        </button>
                      ) : null}
                    </>
                  ) : null}
                </div>
              );
            })}

            {sessions.hasMore && sessions.nextCursor ? (
              <button
                className="py-1.5 text-[11px] text-primary hover:underline"
                onClick={() => void loadSessions(sessions.nextCursor)}
                style={{ paddingLeft: 18 }}
                type="button"
              >
                Load more sessions…
              </button>
            ) : null}
          </>
        ) : null}
      </div>
    </div>
  );
}
