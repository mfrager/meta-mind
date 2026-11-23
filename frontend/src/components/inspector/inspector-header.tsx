import { RefreshCwIcon, XIcon } from "lucide-react";

import type { StatusView } from "@/lib/wire/inspector.ts";
import type { SessionStatusState } from "@/lib/store/status.ts";
import type { InspectorTab } from "@/lib/store/inspector.ts";
import { inspectorTabs } from "@/lib/store/inspector.ts";
import { ago, clockAt } from "@/lib/inspector/format.ts";
import { useNow } from "@/hooks/use-now.ts";
import { cn } from "@/lib/utils";

import { Chip } from "./section.tsx";

function liveLabel(state: SessionStatusState) {
  if (state.stale) return { label: "STALE", tone: "warn" as const };
  if (state.connection === "live") return { label: "LIVE", tone: "live" as const };
  if (state.connection === "reconnecting") return { label: "RECONNECTING", tone: "warn" as const };
  if (state.connection === "error") return { label: "OFFLINE", tone: "danger" as const };
  return { label: "LOADING", tone: "neutral" as const };
}

export function InspectorHeader({
  state,
  view,
  tab,
  onTab,
  onRefresh,
  onClose,
}: {
  state: SessionStatusState;
  view: StatusView | null;
  tab: InspectorTab;
  onTab: (tab: InspectorTab) => void;
  onRefresh: () => void;
  onClose: () => void;
}) {
  const now = useNow(5_000);
  const live = liveLabel(state);
  return (
    <header className="border-border/70 shrink-0 border-b">
      <div className="flex h-9 items-center gap-1.5 px-2">
        <Chip tone={live.tone} title={state.error ?? undefined}>
          {live.label}
        </Chip>
        <span className="font-mono text-[10px] text-muted-foreground" data-revision={view?.revision}>
          view {view?.revision ?? "—"} · state {view?.stateRevision ?? "—"} ·{" "}
          {view ? (state.lastFrameAt ? ago(state.lastFrameAt, now) : clockAt(view.generatedAt)) : "waiting"}
        </span>
        <div className="flex-1" />
        <button
          aria-label="Refresh inspector status"
          className="text-muted-foreground hover:bg-muted hover:text-foreground grid size-6 place-items-center rounded-sm"
          onClick={onRefresh}
          title="Refresh now"
          type="button"
        >
          <RefreshCwIcon className="size-3" />
        </button>
        <button
          aria-label="Close inspector"
          className="text-muted-foreground hover:bg-muted hover:text-foreground grid size-6 place-items-center rounded-sm"
          onClick={onClose}
          title="Close inspector · ⌘⇧I"
          type="button"
        >
          <XIcon className="size-3" />
        </button>
      </div>
      <div aria-label="Inspector tabs" className="flex gap-0.5 px-2 pb-1" role="tablist">
        {inspectorTabs.map((entry, index) => (
          <button
            aria-selected={tab === entry.id}
            className={cn(
              "h-6 rounded-sm px-2 text-[11px] font-medium text-muted-foreground transition-colors",
              tab === entry.id ? "bg-surface text-foreground shadow-elev-1" : "hover:text-foreground",
            )}
            key={entry.id}
            onClick={() => onTab(entry.id)}
            onKeyDown={(event) => {
              if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
              event.preventDefault();
              const delta = event.key === "ArrowRight" ? 1 : -1;
              const next = inspectorTabs[(index + delta + inspectorTabs.length) % inspectorTabs.length];
              onTab(next.id);
              const tablist = event.currentTarget.parentElement;
              const buttons = tablist?.querySelectorAll<HTMLButtonElement>("[role=tab]");
              buttons?.[inspectorTabs.indexOf(next)]?.focus();
            }}
            role="tab"
            tabIndex={tab === entry.id ? 0 : -1}
            type="button"
          >
            {entry.label}
          </button>
        ))}
      </div>
      {state.error ? (
        <p className="px-3 pb-1 text-[10px] text-red-700 dark:text-red-300">{state.error}</p>
      ) : null}
    </header>
  );
}
