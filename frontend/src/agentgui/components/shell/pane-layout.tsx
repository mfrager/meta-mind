"use client";

import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/agentgui/components/ui/resizable";
import { useIsDesktop } from "@/agentgui/hooks/use-media-query";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn } from "@/agentgui/lib/utils";
import type { ReactNode } from "react";

interface PaneLayoutProps {
  chat: ReactNode;
  workspace: ReactNode;
}

export function PaneLayout({ chat, workspace }: PaneLayoutProps) {
  const isDesktop = useIsDesktop();
  const paneLayout = useWorkspaceStore((state) => state.paneLayout);
  const splitRatio = useWorkspaceStore((state) => state.splitRatio);
  const setSplitRatio = useWorkspaceStore((state) => state.setSplitRatio);

  const showChat = paneLayout !== "workspace";
  const showWorkspace = paneLayout !== "chat";

  if (!isDesktop) {
    return (
      <div className="flex h-full min-h-0">
        {paneLayout === "workspace" ? (
          <div className="app-panel hairline-top min-w-0 flex-1 overflow-hidden">
            {workspace}
          </div>
        ) : (
          <div className="chat-panel app-panel hairline-top min-w-0 flex-1 overflow-hidden">
            {chat}
          </div>
        )}
      </div>
    );
  }

  if (paneLayout !== "two" || !showChat || !showWorkspace) {
    return (
      <div className="flex h-full min-h-0">
        <div
          className={cn(
            "app-panel hairline-top min-w-0 flex-1 overflow-hidden",
            paneLayout === "chat" && "chat-panel"
          )}
        >
          {paneLayout === "chat" ? chat : workspace}
        </div>
      </div>
    );
  }

  return (
    <ResizablePanelGroup
      className="h-full min-h-0"
      id="agent-workspace"
      onLayoutChanged={(layout) => {
        const next = layout.chat;
        if (typeof next === "number") setSplitRatio(next);
      }}
      orientation="horizontal"
    >
      <ResizablePanel
        className="min-w-0"
        defaultSize={`${splitRatio}%`}
        id="chat"
        minSize="26%"
      >
        <div className="chat-panel app-panel hairline-top h-full overflow-hidden rounded-r-none">
          {chat}
        </div>
      </ResizablePanel>

      <ResizableHandle
        className={cn(
          "w-1 bg-transparent",
          "after:w-0.5 after:bg-foreground/20 hover:after:bg-primary/60 data-[resize-handle-active]:after:bg-primary/70"
        )}
      />

      <ResizablePanel
        className="min-w-0"
        defaultSize={`${100 - splitRatio}%`}
        id="workspace"
        minSize="26%"
      >
        <div className="app-panel hairline-top h-full overflow-hidden rounded-l-none">
          {workspace}
        </div>
      </ResizablePanel>
    </ResizablePanelGroup>
  );
}
