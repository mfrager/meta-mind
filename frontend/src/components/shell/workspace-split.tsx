import { useEffect, useState, type ReactNode } from "react";
import { Group, Panel, Separator, type Layout } from "react-resizable-panels";

import { usePaneStore } from "@/lib/store/pane.ts";
import { useSideStore } from "@/lib/store/side.ts";
import { useViewportTier } from "@/lib/use-tier.ts";
import { cn } from "@/lib/utils.ts";

const MONITOR_BANNER = "The console's editing surfaces need a wider window.";

export function WorkspaceSplit({
  document,
  chat,
}: {
  document: ReactNode;
  chat: ReactNode;
}) {
  const tier = useViewportTier();
  const swapped = usePaneStore((state) => state.swapped);
  const documentCollapsed = usePaneStore((state) => state.documentCollapsed);
  const chatCollapsed = false;
  const sideLabel = useSideStore((state) => state.label);
  const fullscreen = usePaneStore((state) => state.fullscreen);
  const activePane = usePaneStore((state) => state.activePane);
  const documentSize = usePaneStore((state) => state.documentSize);
  const chatSize = usePaneStore((state) => state.chatSize);
  const layoutEpoch = usePaneStore((state) => state.layoutEpoch);
  const setActivePane = usePaneStore((state) => state.setActivePane);
  const expand = usePaneStore((state) => state.expand);
  const toggleFullscreen = usePaneStore((state) => state.toggleFullscreen);

  if (fullscreen) {
    const showDocument = activePane === "document";
    return (
      <div className="flex h-full min-h-0 flex-col" data-fullscreen="true" data-split="fullscreen">
        <button
          type="button"
          className="border-border text-small hover:bg-muted h-8 shrink-0 border-b px-3 text-left"
          onClick={() => toggleFullscreen()}
        >
          Restore {showDocument ? "chat" : "document"}
        </button>
        <div className={showDocument ? "min-h-0 flex-1" : "hidden"}>{document}</div>
        <div className={showDocument ? "hidden" : "min-h-0 flex-1"}>{chat}</div>
      </div>
    );
  }

  if (tier === "narrow" || tier === "monitor") {
    return (
      <div className="flex h-full min-h-0 flex-col" data-split="switch" data-tier={tier}>
        {tier === "monitor" ? (
          <p
            data-monitor-banner
            className="border-border bg-amber-100 text-small text-amber-900 dark:bg-amber-950 dark:text-amber-100 shrink-0 border-b px-3 py-2"
          >
            {MONITOR_BANNER}
          </p>
        ) : null}
        <div role="tablist" aria-label="Workspace pane" className="border-border flex shrink-0 gap-1 border-b p-1">
          <PaneTab selected={activePane === "chat"} onSelect={() => setActivePane("chat")} label="Chat" />
          <PaneTab
            selected={activePane === "document"}
            onSelect={() => setActivePane("document")}
            label={sideLabel}
          />
        </div>
        <div className={activePane === "document" ? "min-h-0 flex-1" : "hidden"}>{document}</div>
        <div className={activePane === "chat" ? "min-h-0 flex-1" : "hidden"}>{chat}</div>
      </div>
    );
  }

  const orientation = tier === "medium" ? "vertical" : "horizontal";
  const order = swapped ? (["chat", "document"] as const) : (["document", "chat"] as const);
  const visible = order.filter((id) => (id === "document" ? !documentCollapsed : !chatCollapsed));
  const both = visible.length === 2;

  const leading = order[0];
  const trailing = order[1];

  return (
    <div
      className={cn("flex h-full min-h-0", orientation === "vertical" ? "flex-col" : "flex-row")}
      data-split={orientation}
      data-tier={tier}
      data-swapped={swapped ? "true" : "false"}
      data-pane-order={order.join(",")}
    >
      <PaneStrip
        pane={leading}
        orientation={orientation}
        collapsed={leading === "document" ? documentCollapsed : chatCollapsed}
        onRestore={() => expand(leading)}
      />
      {visible.length === 0 ? (
        <p className="text-small text-muted-foreground grid flex-1 place-items-center">Both panes are collapsed.</p>
      ) : (
        <SplitGroup
          key={`${orientation}:${visible.join(",")}:${swapped ? "swap" : "rest"}:${layoutEpoch}`}
          orientation={orientation}
          both={both}
          visible={visible}
          documentSize={documentSize}
          chatSize={chatSize}
          document={document}
          chat={chat}
        />
      )}
      <PaneStrip
        pane={trailing}
        orientation={orientation}
        collapsed={trailing === "document" ? documentCollapsed : chatCollapsed}
        onRestore={() => expand(trailing)}
      />
    </div>
  );
}

function PaneTab({
  selected,
  onSelect,
  label,
}: {
  selected: boolean;
  onSelect: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={selected}
      className={cn(
        "text-small h-[var(--control-h)] rounded-md px-3",
        selected ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-muted",
      )}
      onClick={onSelect}
    >
      {label}
    </button>
  );
}

function PaneStrip({
  pane,
  orientation,
  collapsed,
  onRestore,
}: {
  pane: "document" | "chat";
  orientation: "horizontal" | "vertical";
  collapsed: boolean;
  onRestore: () => void;
}) {
  const sideLabel = useSideStore((state) => state.label);
  if (!collapsed) return null;
  const label = pane === "document" ? sideLabel : "Chat";
  const vertical = orientation === "horizontal";
  return (
    <button
      type="button"
      data-pane-strip={pane}
      aria-label={`Restore ${label}`}
      className={cn(
        "border-border text-muted-foreground hover:bg-muted hover:text-foreground shrink-0",
        vertical ? "w-8 border-x" : "h-8 border-y",
      )}
      onClick={onRestore}
    >
      <span className={cn("micro-label", vertical && "vertical-label")}>{label}</span>
    </button>
  );
}

function SplitGroup({
  orientation,
  both,
  visible,
  documentSize,
  chatSize,
  document,
  chat,
}: {
  orientation: "horizontal" | "vertical";
  both: boolean;
  visible: readonly ("document" | "chat")[];
  documentSize: number;
  chatSize: number;
  document: ReactNode;
  chat: ReactNode;
}) {
  const [resizing, setResizing] = useState(false);

  useEffect(() => {
    if (!resizing) return;
    const stop = () => setResizing(false);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
    return () => {
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
    };
  }, [resizing]);

  const first = visible[0];
  const second = visible[1];

  return (
    <Group
      id="mm-workspace"
      orientation={orientation}
      className="min-h-0 min-w-0 flex-1"
      data-resizing={resizing ? "true" : "false"}
      defaultLayout={both ? { document: documentSize, chat: chatSize } : undefined}
      onLayoutChanged={(layout, meta) => rememberSizes(layout, meta.requestedLayout, meta.isUserInteraction)}
    >
      {first ? (
        <WorkspacePanel id={first} both={both} documentSize={documentSize} chatSize={chatSize}>
          {first === "document" ? document : chat}
        </WorkspacePanel>
      ) : null}
      {both ? (
        <Separator
          id="mm-separator"
          className="pane-separator"
          aria-label="Resize panes"
          onPointerDown={() => setResizing(true)}
        />
      ) : null}
      {second ? (
        <WorkspacePanel id={second} both={both} documentSize={documentSize} chatSize={chatSize}>
          {second === "document" ? document : chat}
        </WorkspacePanel>
      ) : null}
    </Group>
  );
}

function WorkspacePanel({
  id,
  both,
  documentSize,
  chatSize,
  children,
}: {
  id: "document" | "chat";
  both: boolean;
  documentSize: number;
  chatSize: number;
  children: ReactNode;
}) {
  const size = id === "document" ? documentSize : chatSize;
  return (
    <Panel
      id={id}
      className="h-full min-h-0"
      defaultSize={both ? `${size}%` : "100%"}
      minSize={both ? "25%" : "0"}
      maxSize={both ? "75%" : "100%"}
    >
      {children}
    </Panel>
  );
}

function rememberSizes(layout: Layout, requested: Layout | undefined, user: boolean) {
  if (!user) return;
  const source = requested ?? layout;
  const documentSize = source.document;
  const chatSize = source.chat;
  if (typeof documentSize !== "number" || typeof chatSize !== "number") return;
  const nextDocument = Math.round(documentSize * 10) / 10;
  const nextChat = Math.round(chatSize * 10) / 10;
  const current = usePaneStore.getState();
  if (current.documentSize === nextDocument && current.chatSize === nextChat) return;
  current.setSizes(nextDocument, nextChat);
}
