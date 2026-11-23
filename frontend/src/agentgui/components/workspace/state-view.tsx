"use client";

import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn, formatRelative } from "@/agentgui/lib/utils";
import { fetchStateBoard } from "@/lib/api/inspector";
import { fetchConversation } from "@/lib/api/sessions";
import type { StatusValue } from "@/lib/wire/inspector";

function Value({ value }: { value: StatusValue }) {
  if (value === null || value === undefined) {
    return <span className="text-muted-foreground">—</span>;
  }
  if (typeof value === "boolean") {
    return <span className="font-mono">{value ? "true" : "false"}</span>;
  }
  if (typeof value === "number" || typeof value === "string") {
    return <span className="whitespace-pre-wrap break-words">{String(value)}</span>;
  }
  if (Array.isArray(value)) {
    if (value.length === 0) return <span className="text-muted-foreground">[]</span>;
    return (
      <span className="break-words">
        {value.map((item, index) => (
          <span key={index}>
            {index > 0 ? " · " : ""}
            <Value value={item} />
          </span>
        ))}
      </span>
    );
  }
  return (
    <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap rounded-sm border border-border/70 bg-surface-muted/40 p-1.5 font-mono text-[10.5px]">
      {JSON.stringify(value, null, 2)}
    </pre>
  );
}

function Card({
  title,
  meta,
  children,
}: {
  title: string;
  meta?: string;
  children: ReactNode;
}) {
  return (
    <section className="overflow-hidden rounded-xl border border-border bg-surface shadow-elev-1">
      <header className="flex items-baseline gap-2 border-b border-border/70 bg-surface-muted/40 px-3 py-2">
        <h2 className="text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          {title}
        </h2>
        {meta ? (
          <span className="ml-auto font-mono text-[10px] text-muted-foreground">{meta}</span>
        ) : null}
      </header>
      <div className="flex flex-col gap-2 p-3">{children}</div>
    </section>
  );
}

function ConversationCard({ sessionId }: { sessionId: string }) {
  const conversation = useQuery({
    queryKey: ["agentgui", "conversation", sessionId],
    queryFn: () => fetchConversation(sessionId),
    enabled: Boolean(sessionId),
  });
  const state = conversation.data;
  const entries = state ? Object.entries(state.status) : [];

  return (
    <Card
      title="Conversation"
      meta={state ? `rev ${state.revision} · ${formatRelative(new Date(state.updatedAt).toISOString())}` : undefined}
    >
      {!sessionId ? (
        <p className="text-[12px] text-muted-foreground">
          No session is selected, so there is no conversation state yet.
        </p>
      ) : null}
      {sessionId && conversation.isPending ? (
        <p className="text-[12px] text-muted-foreground">Loading conversation state…</p>
      ) : null}
      {conversation.isError ? (
        <p className="text-[12px] text-red-700 dark:text-red-300">
          {conversation.error instanceof Error
            ? conversation.error.message
            : "The conversation state did not load."}
        </p>
      ) : null}
      {state ? (
        <>
          <div className="grid grid-cols-[72px_minmax(0,1fr)] gap-x-2 gap-y-1 text-[12px]">
            <span className="text-muted-foreground">topic</span>
            <span className={cn("break-words", !state.topic && "text-muted-foreground")}>
              {state.topic ?? "not set"}
            </span>
            <span className="text-muted-foreground">depth</span>
            <span className="font-mono">{state.depth}</span>
            <span className="text-muted-foreground">updated by</span>
            <span className="font-mono">{state.updatedBy}</span>
          </div>
          {entries.length > 0 ? (
            <div className="mt-1 flex flex-col gap-1 border-t border-border/60 pt-2">
              {entries.map(([key, value]) => (
                <div className="grid grid-cols-[104px_minmax(0,1fr)] gap-2 text-[12px]" key={key}>
                  <span className="truncate font-mono text-[11px] text-muted-foreground" title={key}>
                    {key}
                  </span>
                  <Value value={value} />
                </div>
              ))}
            </div>
          ) : (
            <p className="text-[12px] text-muted-foreground">No working variables are recorded.</p>
          )}
        </>
      ) : null}
    </Card>
  );
}

function BeingCard() {
  const board = useQuery({ queryKey: ["agentgui", "being"], queryFn: fetchStateBoard });
  const being = board.data?.being;

  return (
    <Card title="Being" meta={being?.ok ? `version ${being.version}` : undefined}>
      {board.isPending ? (
        <p className="text-[12px] text-muted-foreground">Loading being state…</p>
      ) : null}
      {board.isError ? (
        <p className="text-[12px] text-red-700 dark:text-red-300">
          {board.error instanceof Error ? board.error.message : "The being state did not load."}
        </p>
      ) : null}
      {board.data && !being ? (
        <p className="text-[12px] text-muted-foreground">The kernel reports no being section.</p>
      ) : null}
      {being && !being.ok ? (
        <p className="text-[12px] text-red-700 dark:text-red-300">{being.reason}</p>
      ) : null}
      {being?.ok ? (
        <>
          <p className="text-[12px] leading-relaxed">{being.self_description}</p>
          <p className="text-[11px] text-muted-foreground">
            {being.goal_count} goals · voice warmth {being.voice.warmth.toFixed(2)} · directness{" "}
            {being.voice.directness.toFixed(2)} · curiosity {being.voice.curiosity.toFixed(2)} ·
            playfulness {being.voice.playfulness.toFixed(2)} · formality{" "}
            {being.voice.formality.toFixed(2)}
          </p>
          {being.invariants.length > 0 ? (
            <div className="flex flex-wrap gap-1">
              {being.invariants.map((invariant) => (
                <span
                  className="rounded border border-border/70 bg-surface-muted/60 px-1.5 py-0.5 text-[10.5px] text-muted-foreground"
                  key={invariant}
                >
                  {invariant}
                </span>
              ))}
            </div>
          ) : null}
        </>
      ) : null}
    </Card>
  );
}

export function StateView({ sessionId }: { sessionId: string }) {
  const artifacts = useWorkspaceStore((state) => state.artifacts);
  const openArtifact = useWorkspaceStore((state) => state.openArtifact);
  const recent = Object.values(artifacts)
    .filter((artifact) => artifact.kind !== "files")
    .sort((a, b) => (b.updatedAt ?? "").localeCompare(a.updatedAt ?? ""))
    .slice(0, 6);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-3" data-view="state">
      <div className="mx-auto flex w-full max-w-2xl flex-col gap-3">
        <ConversationCard sessionId={sessionId} />
        <BeingCard />
        {recent.length > 0 ? (
          <section className="flex flex-wrap items-center gap-1.5 px-1">
            <span className="text-[10.5px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
              Artifacts
            </span>
            {recent.map((artifact) => (
              <button
                className="rounded-md border border-border/70 bg-surface px-2 py-0.5 text-[11.5px] text-muted-foreground transition-colors hover:border-primary/25 hover:text-foreground"
                key={artifact.id}
                onClick={() => openArtifact(artifact, { activate: true })}
                type="button"
              >
                {artifact.title}
              </button>
            ))}
          </section>
        ) : null}
      </div>
    </div>
  );
}
