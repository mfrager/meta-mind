import type { ButtonHTMLAttributes, ReactNode } from "react";
import { PanelLeftClose } from "lucide-react";

import { ChatPane } from "@/components/chat/chat-pane.tsx";
import { DocBody } from "@/components/docs/viewers.tsx";
import { DocHeader } from "@/components/docs/doc-header.tsx";
import { StateBoard } from "@/components/state/state-board.tsx";
import { useSideStore } from "@/lib/store/side.ts";
import { usePaneStore } from "@/lib/store/pane.ts";
import { WorkspaceSplit } from "@/components/shell/workspace-split.tsx";

export function Workspace() {
  return (
    <div className="h-full min-h-0">
      <WorkspaceSplit document={<SidePane />} chat={<ChatPane />} />
    </div>
  );
}

export function SidePane() {
  const mode = useSideStore((state) => state.mode);
  const doc = useSideStore((state) => state.doc);
  const error = useSideStore((state) => state.error);
  const label = useSideStore((state) => state.label);
  const showState = useSideStore((state) => state.showState);
  const toggleCollapsed = usePaneStore((state) => state.toggleCollapsed);
  const title = mode === "document" && doc ? doc.title : "State";

  return (
    <section data-side-pane data-pane="document" aria-label={title} className="bg-background flex h-full min-h-0 flex-col">
      <header className="border-border flex h-8 shrink-0 items-center gap-2 border-b px-2">
        <h2 className="text-h3 min-w-0 flex-1 truncate">{title}</h2>
        {mode === "document" ? (
          <button type="button" className="text-small hover:bg-muted h-6 rounded-md px-2" onClick={() => showState()}>
            Back to state
          </button>
        ) : null}
        <IconAction label={`Collapse ${label}`} data-collapse="document" onClick={() => toggleCollapsed("document")}>
          <PanelLeftClose className="size-3.5" />
        </IconAction>
      </header>
      <div data-pane-body className="pane-scroll min-h-0 flex-1 overflow-auto">
        {mode === "state" ? <StateBoard /> : null}
        {mode === "document" && error ? <p className="text-body p-4">{error}</p> : null}
        {mode === "document" && doc ? (
          <div className="flex flex-col">
            <div className="border-border flex items-start justify-between gap-2 border-b px-3 py-2">
              <DocHeader doc={doc} />
              <button type="button" className="text-small" disabled aria-disabled="true">
                Ask Metamind to change this.
              </button>
            </div>
            <DocBody doc={doc} editing={false} draft="" onDraft={() => undefined} />
          </div>
        ) : null}
      </div>
    </section>
  );
}

function IconAction({
  label,
  onClick,
  children,
  ...rest
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className="text-muted-foreground hover:bg-muted hover:text-foreground grid size-6 place-items-center rounded-md"
      onClick={onClick}
      {...rest}
    >
      {children}
    </button>
  );
}
