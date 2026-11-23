import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { PlusIcon, RefreshCwIcon, Trash2Icon } from "lucide-react";
import { useDeferredValue, useState } from "react";

import { createConsoleSession, fetchSessionPage, resumeSession } from "@/lib/api/sessions.ts";
import type { SessionSummary } from "@/lib/wire/inspector.ts";
import { ago, errorMessage, shortId } from "@/lib/inspector/format.ts";
import { cn } from "@/lib/utils";

import { Chip, UnavailableRow } from "@/components/inspector/section.tsx";
import { DeleteSessionDialog } from "./delete-session-dialog.tsx";

function presenceTone(session: SessionSummary) {
  if (session.working) return "live" as const;
  if (session.presence === "present") return "ok" as const;
  if (session.presence === "away") return "warn" as const;
  return "neutral" as const;
}

function traceKinds(session: SessionSummary) {
  if (!session.traceKinds.length) return "no traces";
  return session.traceKinds.join(" + ");
}

function SessionRow({
  session,
  index,
  setSize,
  selected,
  onSelect,
  onResume,
  onDelete,
  resuming,
}: {
  session: SessionSummary;
  index: number;
  setSize: number;
  selected: boolean;
  onSelect: () => void;
  onResume: () => void;
  onDelete: () => void;
  resuming: boolean;
}) {
  return (
    <li
      aria-posinset={index + 1}
      aria-setsize={setSize}
      className={cn(
        "group/row border-border/60 flex items-start gap-1 rounded-sm border px-1.5 py-1.5",
        selected ? "bg-surface shadow-elev-1" : "hover:bg-muted/40",
      )}
      data-session-id={session.sessionId}
    >
      <span
        aria-hidden
        className={cn(
          "mt-1.5 size-1.5 shrink-0 rounded-full",
          session.working
            ? "bg-primary"
            : session.presence === "present"
              ? "bg-emerald-500"
              : session.presence === "away"
                ? "bg-amber-500"
                : "bg-muted-foreground/35",
        )}
      />
      <button className="min-w-0 flex-1 text-left" onClick={onSelect} type="button">
        <span className="block truncate text-[12px] font-medium">
          {session.topic ?? `${shortId(session.sessionId)} · untitled`}
        </span>
        <span className="mt-0.5 flex flex-wrap items-center gap-1 text-[10px] text-muted-foreground">
          <span className="font-mono">{shortId(session.sessionId)}</span>
          <span aria-hidden>·</span>
          <span>{session.lifecycle}</span>
          <span aria-hidden>·</span>
          <span>{session.presence}</span>
          <span aria-hidden>·</span>
          <span>{session.traceCount} traces</span>
          <span aria-hidden>·</span>
          <span>{ago(session.lastActivityAt)}</span>
        </span>
        <span className="mt-0.5 block truncate font-mono text-[9px] text-muted-foreground/80">
          {traceKinds(session)}
        </span>
      </button>
      <div className="flex shrink-0 flex-col items-end gap-0.5">
        <Chip tone={presenceTone(session)}>{session.working ? "working" : session.lifecycle}</Chip>
        <div className="flex items-center gap-0.5 opacity-0 transition-opacity group-hover/row:opacity-100 focus-within:opacity-100">
          {session.resumable || session.lifecycle === "ended" ? (
            <button
              className="text-[10px] text-primary disabled:opacity-50"
              disabled={resuming}
              onClick={onResume}
              type="button"
            >
              Resume
            </button>
          ) : null}
          <button
            aria-label={`Delete ${session.topic ?? session.sessionId}`}
            className="text-muted-foreground hover:text-destructive disabled:opacity-40"
            disabled={session.deleted}
            onClick={onDelete}
            type="button"
          >
            <Trash2Icon className="size-3" />
          </button>
        </div>
      </div>
    </li>
  );
}

export function SessionList({
  selectedId,
  onSelect,
}: {
  selectedId: string | null;
  onSelect: (sessionId: string) => void;
}) {
  const queryClient = useQueryClient();
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query.trim());
  const [status, setStatus] = useState("all");
  const [deleteTarget, setDeleteTarget] = useState<SessionSummary | null>(null);

  const sessions = useInfiniteQuery({
    queryKey: ["inspector", "sessions", deferredQuery, status],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam }) =>
      fetchSessionPage({
        cursor: pageParam,
        limit: 20,
        q: deferredQuery || undefined,
        status: status === "all" ? undefined : status,
      }),
    getNextPageParam: (last) => (last.hasMore ? last.nextCursor : undefined),
  });
  const rows = sessions.data?.pages.flatMap((page) => page.items) ?? [];

  const create = useMutation({
    mutationFn: () => createConsoleSession(),
    onSuccess: (result) => {
      void queryClient.invalidateQueries({ queryKey: ["inspector", "sessions"] });
      onSelect(result.sessionId);
    },
  });
  const resume = useMutation({
    mutationFn: resumeSession,
    onSuccess: (_result, sessionId) => {
      void queryClient.invalidateQueries({ queryKey: ["inspector", "sessions"] });
      onSelect(sessionId);
    },
  });

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-panel="sessions">
      <div className="border-border/70 flex items-center justify-between gap-1 border-b px-2 py-1.5">
        <span className="text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          Sessions
        </span>
        <div className="flex items-center gap-1">
          <button
            aria-label="Refresh sessions"
            className="text-muted-foreground hover:text-foreground grid size-5 place-items-center"
            onClick={() => void sessions.refetch()}
            type="button"
          >
            <RefreshCwIcon className="size-3" />
          </button>
          <button
            className="inline-flex items-center gap-1 text-[11px] text-primary disabled:opacity-50"
            disabled={create.isPending}
            onClick={() => create.mutate()}
            type="button"
          >
            <PlusIcon className="size-3" /> New
          </button>
        </div>
      </div>
      <div className="border-border/70 flex items-center gap-1 border-b px-2 py-1.5">
        <input
          aria-label="Search sessions"
          className="border-input bg-background h-6 min-w-0 flex-1 rounded-sm border px-2 text-[11px]"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search sessions…"
          value={query}
        />
        <select
          aria-label="Session status filter"
          className="border-input bg-background h-6 rounded-sm border px-1 text-[10px]"
          onChange={(event) => setStatus(event.target.value)}
          value={status}
        >
          <option value="all">all</option>
          <option value="active">active</option>
          <option value="ending">ending</option>
          <option value="ended">ended</option>
        </select>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
        {sessions.isError ? (
          <UnavailableRow section="sessions" reason={errorMessage(sessions.error)} />
        ) : null}
        {create.isError ? <UnavailableRow section="new session" reason={errorMessage(create.error)} /> : null}
        {resume.isError ? <UnavailableRow section="resume" reason={errorMessage(resume.error)} /> : null}
        {sessions.isPending ? <p className="text-[12px] text-muted-foreground">Loading sessions…</p> : null}
        {!sessions.isPending && rows.length === 0 ? (
          <p className="px-1 py-6 text-[12px] text-muted-foreground">
            {deferredQuery || status !== "all"
              ? "No sessions match this filter."
              : "No sessions yet. Create one to start a conversation."}
          </p>
        ) : null}
        <ul aria-label="Sessions" className="flex flex-col gap-1" role="list">
          {rows.map((session, index) => (
            <SessionRow
              index={index}
              key={session.sessionId}
              onDelete={() => setDeleteTarget(session)}
              onResume={() => resume.mutate(session.sessionId)}
              onSelect={() => onSelect(session.sessionId)}
              resuming={resume.isPending}
              selected={selectedId === session.sessionId}
              session={session}
              setSize={rows.length}
            />
          ))}
        </ul>
        {sessions.hasNextPage ? (
          <button
            className="mt-1 w-full text-[11px] text-primary disabled:opacity-50"
            disabled={sessions.isFetchingNextPage}
            onClick={() => void sessions.fetchNextPage()}
            type="button"
          >
            {sessions.isFetchingNextPage ? "Loading…" : "Load more sessions"}
          </button>
        ) : null}
        {rows.length ? (
          <p aria-live="polite" className="mt-1 text-center text-[10px] text-muted-foreground">
            {rows.length} session{rows.length === 1 ? "" : "s"} loaded
            {sessions.hasNextPage ? " · more available" : ""}
          </p>
        ) : null}
      </div>
      <DeleteSessionDialog
        onDeleted={(sessionId) => {
          void queryClient.invalidateQueries({ queryKey: ["inspector", "sessions"] });
          if (selectedId === sessionId) onSelect("");
        }}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
        session={deleteTarget}
      />
    </div>
  );
}
