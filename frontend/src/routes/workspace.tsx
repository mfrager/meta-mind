import { useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import { PanelLeftClose } from "lucide-react";

import { ChatPane } from "@/components/chat/chat-pane.tsx";
import { DocBody } from "@/components/docs/viewers.tsx";
import { DocHeader } from "@/components/docs/doc-header.tsx";
import { StateBoard } from "@/components/state/state-board.tsx";
import { useLlmActivityStream } from "@/hooks/use-llm-activity-stream.ts";
import { useChatPending } from "@/lib/store/chat-pending.ts";
import { useSideStore } from "@/lib/store/side.ts";
import type { FocusDoc } from "@/lib/wire/focus.ts";
import { usePaneStore } from "@/lib/store/pane.ts";
import { WorkspaceSplit } from "@/components/shell/workspace-split.tsx";

export function Workspace() {
  useLlmActivityStream();
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
              <ChangeControl doc={doc} />
            </div>
            <DocBody doc={doc} editing={false} draft="" onDraft={() => undefined} />
          </div>
        ) : null}
      </div>
    </section>
  );
}

function ChangeControl({ doc }: { doc: FocusDoc }) {
  const [value, setValue] = useState("");
  const fixture = (doc.reason ?? "").toLowerCase().includes("fixture");
  const field = doc.id === "goals" ? "description" : doc.id === "memory" ? "content" : null;
  if (fixture || !field) {
    return (
      <button
        type="button"
        className="text-small"
        disabled
        aria-disabled="true"
        title={fixture ? (doc.reason ?? undefined) : undefined}
      >
        Ask Metamind to change this.
      </button>
    );
  }
  const label = field === "description" ? "Goal description" : "Memory content";
  const query = field === "description" ? "add a goal" : "remember this";
  return (
    <form
      className="flex items-center gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        const next = value.trim();
        if (!next) return;
        useChatPending.getState().queuePropose(query, {
          name: "focus.patch",
          input: { id: doc.id, patch: { op: "set_field", field, value: next } },
        });
        setValue("");
      }}
    >
      <input
        aria-label={label}
        value={value}
        onChange={(event) => setValue(event.target.value)}
        className="border-input h-7 w-40 rounded-md border px-2 text-small"
      />
      <button type="submit" className="text-small">
        Ask Metamind to change this.
      </button>
    </form>
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
