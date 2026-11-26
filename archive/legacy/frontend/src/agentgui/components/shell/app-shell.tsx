"use client";

import { ChatPane } from "@/agentgui/components/chat/chat-pane";
import { useLlmActivityStream } from "@/hooks/use-llm-activity-stream.ts";
import { AppSidebar, SidebarContent } from "@/agentgui/components/navigation/app-sidebar";
import { AgentSettingsDialog } from "@/agentgui/components/shell/agent-settings-dialog";
import { PaneLayout } from "@/agentgui/components/shell/pane-layout";
import { TopBar } from "@/agentgui/components/shell/top-bar";
import {
  Sheet,
  SheetContent,
  SheetTitle,
} from "@/agentgui/components/ui/sheet";
import { useKeyboardShortcuts } from "@/agentgui/hooks/use-keyboard-shortcuts";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { WorkspacePane } from "@/agentgui/components/workspace/workspace-pane";
import { useEffect, useState } from "react";
import { Outlet, useLocation } from "react-router-dom";

export function AppShell() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const mobileSidebarOpen = useWorkspaceStore(
    (state) => state.mobileSidebarOpen
  );
  const setMobileSidebarOpen = useWorkspaceStore(
    (state) => state.setMobileSidebarOpen
  );
  const activeSessionId = useWorkspaceStore((state) => state.activeSessionId);
  const sessions = useWorkspaceStore((state) => state.sessions);
  const hydrateSessions = useWorkspaceStore((state) => state.hydrateSessions);
  const paneLayout = useWorkspaceStore((state) => state.paneLayout);
  const setPaneLayout = useWorkspaceStore((state) => state.setPaneLayout);
  const { pathname } = useLocation();
  const isWorkspace = pathname === "/" || pathname.startsWith("/chat");
  useLlmActivityStream();

  useEffect(() => {
    let cancelled = false;
    void hydrateSessions().finally(() => {
      if (cancelled) return;
      const state = useWorkspaceStore.getState();
      if (!state.activeSessionId) state.createSession("New session");
    });
    return () => {
      cancelled = true;
    };
  }, [hydrateSessions]);

  useKeyboardShortcuts({
    onFocusComposer: () => {
      if (paneLayout === "workspace") setPaneLayout("two");
      requestAnimationFrame(() => {
        document
          .querySelector<HTMLTextAreaElement>("#chat-composer textarea")
          ?.focus();
      });
    },
  });

  const sessionExists = sessions.some(
    (session) => session.id === activeSessionId
  );

  return (
    <div className="flex h-dvh w-full overflow-hidden">
      <AppSidebar />

      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar
          onOpenSettings={() => setSettingsOpen(true)}
          onOpenSidebar={() => setMobileSidebarOpen(true)}
        />

        <main className="min-h-0 flex-1 px-2 pb-2 lg:px-3 lg:pb-3">
          {!isWorkspace ? (
            <div className="app-panel h-full overflow-hidden">
              <Outlet />
            </div>
          ) : sessionExists ? (
            <PaneLayout
              chat={<ChatPane sessionId={activeSessionId} />}
              workspace={<WorkspacePane />}
            />
          ) : (
            <div className="app-panel flex h-full items-center justify-center">
              <div className="text-center">
                <p className="text-[13px] font-medium">No session selected</p>
                <p className="mt-1 text-[12px] text-muted-foreground">
                  Create a session to start talking to the agent.
                </p>
              </div>
            </div>
          )}
        </main>
      </div>

      <Sheet
        onOpenChange={setMobileSidebarOpen}
        open={mobileSidebarOpen}
      >
        <SheetContent
          className="w-[286px] border-r border-sidebar-border bg-sidebar p-0"
          side="left"
        >
          <SheetTitle className="sr-only">Navigation</SheetTitle>
          <SidebarContent compact={false} />
        </SheetContent>
      </Sheet>

      <AgentSettingsDialog
        onOpenChange={setSettingsOpen}
        open={settingsOpen}
      />
    </div>
  );
}
