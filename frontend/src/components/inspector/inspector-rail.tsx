import { useMutation } from "@tanstack/react-query";
import { PanelRightOpenIcon } from "lucide-react";
import type { PointerEvent as ReactPointerEvent } from "react";

import { cancelAction, fetchStatus } from "@/lib/api/inspector.ts";
import { useMediaQuery } from "@/agentgui/hooks/use-media-query.ts";
import { useStatusStream } from "@/hooks/use-status-stream.ts";
import { useInspectorStore } from "@/lib/store/inspector.ts";
import { useStatusStore } from "@/lib/store/status.ts";
import { errorMessage } from "@/lib/inspector/format.ts";
import { cn } from "@/lib/utils";

import { ActionsPanel } from "./actions-panel.tsx";
import { InspectorHeader } from "./inspector-header.tsx";
import { KnowledgePanel } from "./knowledge-panel.tsx";
import { PlanPanel } from "./plan-panel.tsx";
import { StatusPanel } from "./status-panel.tsx";
import { UnavailableRow } from "./section.tsx";
import { TraceViewer } from "./trace-viewer.tsx";

export type InspectorNavigation = "files" | "knowledge" | "plan";

function EmptyRail({ message }: { message: string }) {
  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <p className="max-w-56 text-center text-[12px] text-muted-foreground">{message}</p>
    </div>
  );
}

export function InspectorRail({
  sessionId,
  onNavigate,
}: {
  sessionId: string | null;
  onNavigate?: (target: InspectorNavigation) => void;
}) {
  const { open, width, tab, selectedTrace, setOpen, setTab, setWidth, setSelectedTrace } =
    useInspectorStore();
  const state = useStatusStream(sessionId);
  const isWide = useMediaQuery("(min-width: 1440px)");
  const isDesktop = useMediaQuery("(min-width: 1024px)");

  const cancel = useMutation({
    mutationFn: cancelAction,
    onSuccess: () => {
      if (sessionId) {
        void fetchStatus(sessionId).then((view) => useStatusStore.getState().setView(sessionId, view));
      }
    },
  });

  const refresh = () => {
    if (!sessionId) return;
    useStatusStore.getState().begin(sessionId);
    void fetchStatus(sessionId)
      .then((view) => useStatusStore.getState().setView(sessionId, view))
      .catch((error: unknown) =>
        useStatusStore.getState().setConnection(sessionId, "error", errorMessage(error)),
      );
  };

  const startResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = width;
    const move = (moveEvent: PointerEvent) => {
      setWidth(startWidth + (startX - moveEvent.clientX));
    };
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
  };

  const body = (
    <>
      <InspectorHeader
        onClose={() => setOpen(false)}
        onRefresh={refresh}
        onTab={setTab}
        state={state}
        tab={tab}
        view={state.view}
      />
      {!sessionId ? (
        <EmptyRail message="Select or create a session to inspect its status, knowledge, plans and traces." />
      ) : null}
      {sessionId && tab === "status" ? (
        state.view ? (
          <StatusPanel
            onCancelAction={(id) => cancel.mutate(id)}
            onOpenFiles={() => onNavigate?.("files")}
            onOpenKnowledge={() => onNavigate?.("knowledge")}
            onOpenPlan={() => onNavigate?.("plan")}
            onOpenTrace={(traceId) => {
              setSelectedTrace(traceId);
              setTab("trace");
            }}
            view={state.view}
          />
        ) : state.connection === "error" ? (
          <div className="p-2">
            <UnavailableRow section="status" reason={state.error ?? undefined} />
          </div>
        ) : (
          <EmptyRail message="Reading the status snapshot…" />
        )
      ) : null}
      {sessionId && tab === "knowledge" ? (
        <KnowledgePanel
          onOpenTrace={(traceId) => {
            setSelectedTrace(traceId);
            setTab("trace");
          }}
        />
      ) : null}
      {sessionId && tab === "plan" ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden border-b border-border/70">
            <PlanPanel sessionId={sessionId} />
          </div>
          <div className="flex max-h-[42%] min-h-0 flex-col overflow-hidden">
            <ActionsPanel sessionId={sessionId} />
          </div>
        </div>
      ) : null}
      {sessionId && tab === "trace" ? (
        <TraceViewer
          onSelectTrace={setSelectedTrace}
          selectedTraceId={selectedTrace}
          sessionId={sessionId}
        />
      ) : null}
      {cancel.isError ? (
        <p className="px-3 py-1 text-[10px] text-red-700 dark:text-red-300">
          cancel failed: {errorMessage(cancel.error)}
        </p>
      ) : null}
    </>
  );

  if (!open) {
    return (
      <aside className="border-border/70 bg-background flex w-10 shrink-0 flex-col items-center border-l py-2" aria-label="Inspector">
        <button
          aria-label="Open inspector"
          className="text-muted-foreground hover:bg-muted hover:text-foreground grid size-7 place-items-center rounded-sm"
          onClick={() => setOpen(true)}
          title="Open inspector · ⌘⇧I"
          type="button"
        >
          <PanelRightOpenIcon className="size-4" />
        </button>
      </aside>
    );
  }

  if (isWide) {
    return (
      <aside
        aria-label="Inspector"
        className="border-border/70 bg-background relative flex shrink-0 flex-col border-l"
        data-revision={state.view?.revision}
        data-state-revision={state.view?.stateRevision}
        style={{ width }}
      >
        <div
          aria-hidden
          className="hover:bg-primary/40 absolute inset-y-0 left-0 z-20 w-1 cursor-col-resize"
          onPointerDown={startResize}
        />
        {body}
      </aside>
    );
  }

  return (
    <>
      <aside className="border-border/70 bg-background flex w-10 shrink-0 flex-col items-center border-l py-2" aria-label="Inspector">
        <button
          aria-label="Open inspector"
          className="text-muted-foreground hover:bg-muted hover:text-foreground grid size-7 place-items-center rounded-sm"
          onClick={() => setOpen(true)}
          type="button"
        >
          <PanelRightOpenIcon className="size-4" />
        </button>
      </aside>
      <div className="fixed inset-0 z-40 bg-black/25" onClick={() => setOpen(false)} role="presentation" />
      <aside
        aria-label="Inspector"
        className={cn(
          "bg-background fixed inset-y-0 right-0 z-50 flex w-[min(94vw,380px)] flex-col border-l shadow-lg",
          isDesktop ? "w-[380px]" : "",
        )}
        data-revision={state.view?.revision}
        data-state-revision={state.view?.stateRevision}
      >
        {body}
      </aside>
    </>
  );
}
