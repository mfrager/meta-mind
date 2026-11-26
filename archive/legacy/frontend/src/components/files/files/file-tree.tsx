import { useVirtualizer } from "@tanstack/react-virtual";
import {
  ChevronRightIcon,
  FileIcon,
  FileTextIcon,
  FolderIcon,
  Loader2Icon,
  RefreshCwIcon,
  ScrollTextIcon,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { fetchFileTree } from "@/lib/api/files.ts";
import type { FileTreeEntry, FileTreePage } from "@/lib/wire/inspector.ts";
import { errorMessage } from "@/lib/inspector/format.ts";
import { cn } from "@/lib/utils";

import { Chip, UnavailableRow } from "@/components/inspector/section.tsx";

type PageState = {
  entries: FileTreeEntry[];
  nextCursor: string | null;
  hasMore: boolean;
  loaded: boolean;
  loading: boolean;
  error: string | null;
};

type Row =
  | { kind: "root"; key: string; path: string; depth: number; label: string }
  | { kind: "entry"; key: string; path: string; depth: number; entry: FileTreeEntry }
  | { kind: "more"; key: string; path: string; depth: number; cursor: string }
  | { kind: "loading"; key: string; path: string; depth: number }
  | { kind: "notice"; key: string; path: string; depth: number; text: string };

const emptyPage: PageState = {
  entries: [],
  nextCursor: null,
  hasMore: false,
  loaded: false,
  loading: false,
  error: null,
};

function sessionIdFromPath(path: string): string | null {
  const match = /^\/sessions\/([^/]+)/.exec(path);
  return match?.[1] ?? null;
}

function iconFor(kind: FileTreeEntry["kind"], open: boolean) {
  if (kind === "dir") {
    return <FolderIcon className={cn("size-3.5 shrink-0", open ? "text-primary" : "text-muted-foreground")} />;
  }
  if (kind === "trace") return <FileTextIcon className="size-3.5 shrink-0 text-muted-foreground" />;
  if (kind === "log") return <ScrollTextIcon className="size-3.5 shrink-0 text-muted-foreground" />;
  return <FileIcon className="size-3.5 shrink-0 text-muted-foreground" />;
}

export function FileTree({
  onOpenTrace,
  onOpenLog,
  onOpenFile,
  onDeleteSession,
}: {
  onOpenTrace?: (tracePath: string) => void;
  onOpenLog?: (logPath: string) => void;
  onOpenFile?: (filePath: string) => void;
  onDeleteSession?: (sessionId: string) => void;
}) {
  const [pages, setPages] = useState<Record<string, PageState>>({});
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(["/sessions"]));
  const scrollRef = useRef<HTMLDivElement>(null);
  const loading = useRef<Set<string>>(new Set());

  const load = async (path: string, cursor?: string | null) => {
    const requestKey = `${path}:${cursor ?? ""}`;
    if (loading.current.has(requestKey)) return;
    loading.current.add(requestKey);
    setPages((current) => ({
      ...current,
      [path]: { ...(current[path] ?? emptyPage), loading: true, error: null },
    }));
    try {
      const page: FileTreePage = await fetchFileTree(path, cursor ?? null, 50);
      setPages((current) => {
        const previous = current[path] ?? emptyPage;
        return {
          ...current,
          [path]: {
            entries: cursor ? [...previous.entries, ...page.entries] : page.entries,
            nextCursor: page.nextCursor,
            hasMore: page.hasMore,
            loaded: true,
            loading: false,
            error: null,
          },
        };
      });
    } catch (error: unknown) {
      setPages((current) => ({
        ...current,
        [path]: {
          ...(current[path] ?? emptyPage),
          loading: false,
          error: errorMessage(error),
        },
      }));
    } finally {
      loading.current.delete(requestKey);
    }
  };

  useEffect(() => {
    void load("/sessions");
    // The tree root is loaded once for the panel; child pages load on expand.
  }, []);

  const toggle = (path: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
    if (!pages[path]?.loaded && !pages[path]?.loading) void load(path);
  };

  const rows: Row[] = [];
  rows.push({ kind: "root", key: "root:sessions", path: "/sessions", depth: 0, label: "sessions" });
  const visit = (path: string, depth: number) => {
    if (!expanded.has(path)) return;
    const page = pages[path] ?? emptyPage;
    if (page.loading && page.entries.length === 0) {
      rows.push({ kind: "loading", key: `loading:${path}`, path, depth });
    }
    if (page.error) {
      rows.push({ kind: "notice", key: `error:${path}`, path, depth, text: page.error });
    }
    for (const entry of page.entries) {
      rows.push({ kind: "entry", key: `entry:${entry.path}`, path: entry.path, depth, entry });
      if (entry.kind === "dir" && expanded.has(entry.path)) visit(entry.path, depth + 1);
    }
    if (page.hasMore && page.nextCursor) {
      rows.push({
        kind: "more",
        key: `more:${path}:${page.nextCursor}`,
        path,
        depth,
        cursor: page.nextCursor,
      });
    }
  };
  visit("/sessions", 1);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 24,
    overscan: 20,
    getItemKey: (index) => rows[index]?.key ?? index,
  });

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-panel="files">
      <div className="border-border/70 flex items-center gap-1 border-b px-2 py-1.5">
        <span className="flex-1 text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          Files
        </span>
        <Chip>{Object.values(pages).reduce((sum, page) => sum + page.entries.length, 0)} nodes</Chip>
        <button
          aria-label="Refresh files"
          className="text-muted-foreground hover:text-foreground grid size-5 place-items-center"
          onClick={() => {
            setPages({});
            void load("/sessions");
          }}
          type="button"
        >
          <RefreshCwIcon className="size-3" />
        </button>
      </div>
      {pages["/sessions"]?.error ? (
        <div className="p-2">
          <UnavailableRow section="files" reason={pages["/sessions"].error} />
        </div>
      ) : null}
      <div className="min-h-0 flex-1 overflow-auto px-1 py-1" ref={scrollRef}>
        <div className="relative w-full" style={{ height: `${virtualizer.getTotalSize()}px` }}>
          {virtualizer.getVirtualItems().map((item) => {
            const row = rows[item.index];
            if (!row) return null;
            return (
              <div
                className="absolute top-0 left-0 w-full"
                data-index={item.index}
                key={row.key}
                ref={virtualizer.measureElement}
                style={{ transform: `translateY(${item.start}px)` }}
              >
                <FileRow
                  expanded={expanded.has(row.path)}
                  onDeleteSession={onDeleteSession}
                  onOpenFile={onOpenFile}
                  onOpenLog={onOpenLog}
                  onOpenTrace={onOpenTrace}
                  onToggle={toggle}
                  pages={pages}
                  row={row}
                  onLoadMore={(path, cursor) => void load(path, cursor)}
                />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function FileRow({
  row,
  pages,
  expanded,
  onToggle,
  onLoadMore,
  onOpenTrace,
  onOpenLog,
  onOpenFile,
  onDeleteSession,
}: {
  row: Row;
  pages: Record<string, PageState>;
  expanded: boolean;
  onToggle: (path: string) => void;
  onLoadMore: (path: string, cursor: string) => void;
  onOpenTrace?: (tracePath: string) => void;
  onOpenLog?: (logPath: string) => void;
  onOpenFile?: (filePath: string) => void;
  onDeleteSession?: (sessionId: string) => void;
}) {
  const padding = 6 + row.depth * 12;
  if (row.kind === "notice") {
    return (
      <p className="line-clamp-2 text-[10px] text-red-700 dark:text-red-300" style={{ paddingLeft: padding }}>
        {row.text}
      </p>
    );
  }
  if (row.kind === "loading") {
    return (
      <p className="text-[10px] text-muted-foreground" style={{ paddingLeft: padding }}>
        <Loader2Icon className="mr-1 inline size-3 animate-spin" /> loading…
      </p>
    );
  }
  if (row.kind === "more") {
    return (
      <button
        className="text-[10px] text-primary hover:underline"
        onClick={() => onLoadMore(row.path, row.cursor)}
        style={{ paddingLeft: padding }}
        type="button"
      >
        Load more…
      </button>
    );
  }
  if (row.kind === "root") {
    return (
      <button
        className="flex w-full items-center gap-1 rounded-sm py-0.5 pr-1 text-left hover:bg-muted/50"
        onClick={() => onToggle(row.path)}
        style={{ paddingLeft: padding }}
        type="button"
      >
        <ChevronRightIcon
          className={cn("size-3 shrink-0 text-muted-foreground transition-transform", expanded && "rotate-90")}
        />
        <FolderIcon className="size-3.5 shrink-0 text-primary" />
        <span className="truncate font-mono text-[11px]">{row.label}/</span>
        <Chip>{pages[row.path]?.entries.length ?? 0}</Chip>
      </button>
    );
  }

  const { entry } = row;
  const isDir = entry.kind === "dir";
  const sessionId = sessionIdFromPath(entry.path);
  const leafClick = () => {
    if (entry.kind === "trace") onOpenTrace?.(entry.path);
    else if (entry.kind === "log") onOpenLog?.(entry.path);
    else if (entry.kind === "file") onOpenFile?.(entry.path);
  };
  return (
    <div
      className="group/file flex items-center gap-1 rounded-sm pr-1 hover:bg-muted/50"
      data-file-kind={entry.kind}
      data-file-path={entry.path}
      style={{ paddingLeft: padding }}
    >
      <button
        className="flex min-w-0 flex-1 items-center gap-1 py-0.5 text-left"
        onClick={() => (isDir ? onToggle(entry.path) : leafClick())}
        type="button"
      >
        {isDir ? (
          <ChevronRightIcon
            className={cn("size-3 shrink-0 text-muted-foreground transition-transform", expanded && "rotate-90")}
          />
        ) : (
          <span className="w-3 shrink-0" />
        )}
        {iconFor(entry.kind, expanded)}
        <span
          className={cn("min-w-0 flex-1 truncate", isDir ? "font-mono text-[11px]" : "text-[11px]")}
          title={entry.path}
        >
          {entry.name}
          {isDir ? "/" : ""}
        </span>
        {entry.kind === "trace" && typeof entry.eventCount === "number" ? (
          <span className="shrink-0 font-mono text-[9px] text-muted-foreground">
            {entry.eventCount} ev
          </span>
        ) : null}
        {isDir && pages[entry.path]?.hasMore ? (
          <span className="shrink-0 font-mono text-[9px] text-muted-foreground">more</span>
        ) : null}
      </button>
      {sessionId && entry.path === `/sessions/${sessionId}` && onDeleteSession ? (
        <button
          aria-label={`Delete session ${sessionId}`}
          className="text-muted-foreground opacity-0 group-hover/file:opacity-100 hover:text-destructive"
          onClick={() => onDeleteSession(sessionId)}
          type="button"
        >
          ×
        </button>
      ) : null}
    </div>
  );
}
