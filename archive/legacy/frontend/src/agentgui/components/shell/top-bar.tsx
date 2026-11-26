"use client";

import { Button } from "@/agentgui/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/agentgui/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/agentgui/components/ui/tooltip";
import { useChatStore } from "@/agentgui/lib/store/chat-store";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn } from "@/agentgui/lib/utils";
import { useNavigate } from "react-router-dom";
import {
  ChevronRightIcon,
  Columns2Icon,
  EllipsisIcon,
  Maximize2Icon,
  MenuIcon,
  MessagesSquareIcon,
  PanelRightOpenIcon,
  PlusIcon,
  SparklesIcon,
  Trash2Icon,
} from "lucide-react";

import type { PaneLayout } from "@/agentgui/types/workspace";

const layoutOptions: {
  id: PaneLayout;
  label: string;
  hint: string;
  icon: typeof Columns2Icon;
}[] = [
  { id: "chat", label: "Chat only", hint: "", icon: MessagesSquareIcon },
  { id: "two", label: "Both panes", hint: "⌘\\", icon: Columns2Icon },
];

export function TopBar({
  onOpenSidebar,
  onOpenSettings,
}: {
  onOpenSidebar: () => void;
  onOpenSettings: () => void;
}) {
  const paneLayout = useWorkspaceStore((state) => state.paneLayout);
  const setPaneLayout = useWorkspaceStore((state) => state.setPaneLayout);
  const sessions = useWorkspaceStore((state) => state.sessions);
  const projects = useWorkspaceStore((state) => state.projects);
  const activeSessionId = useWorkspaceStore((state) => state.activeSessionId);
  const activeProjectId = useWorkspaceStore((state) => state.activeProjectId);
  const artifacts = useWorkspaceStore((state) => state.artifacts);
  const activeArtifactId = useWorkspaceStore(
    (state) => state.activeArtifactId
  );
  const createSession = useWorkspaceStore((state) => state.createSession);
  const clearSession = useChatStore((state) => state.clearSession);
  const navigate = useNavigate();
  const streaming = useChatStore(
    (state) => state.streamingSessions[activeSessionId] ?? false
  );

  const session = sessions.find((entry) => entry.id === activeSessionId);
  const project = projects.find((entry) => entry.id === activeProjectId);
  const artifact = activeArtifactId ? artifacts[activeArtifactId] : undefined;

  return (
    <header className="flex h-12 shrink-0 items-center gap-2 px-2 lg:px-3">
      <Button
        aria-label="Open navigation"
        className="lg:hidden"
        onClick={onOpenSidebar}
        size="icon-sm"
        variant="ghost"
      >
        <MenuIcon className="size-4" />
      </Button>

      <div className="hidden items-center gap-1.5 lg:flex">
        <span className="flex size-5 items-center justify-center rounded-md bg-surface text-muted-foreground shadow-elev-1">
          <SparklesIcon className="size-3" />
        </span>
        <span className="text-[13px] text-muted-foreground">
          {project?.name ?? "Workspace"}
        </span>
        <ChevronRightIcon className="size-3 text-muted-foreground/60" />
        <span className="max-w-56 truncate text-[13px] font-medium">
          {session?.title ?? "New session"}
        </span>
        {artifact && paneLayout !== "chat" && (
          <>
            <ChevronRightIcon className="size-3 text-muted-foreground/60" />
            <span className="max-w-44 truncate text-[13px] text-muted-foreground">
              {artifact.title}
            </span>
          </>
        )}
      </div>

      <div className="flex items-center gap-1.5 lg:hidden">
        <span className="max-w-40 truncate text-[14px] font-medium">
          {session?.title ?? "New session"}
        </span>
        {streaming && <span className="status-dot status-dot-busy" />}
      </div>

      <div className="flex-1" />

      {streaming && (
        <span className="hidden items-center gap-1.5 rounded-full border border-border bg-surface px-2 py-0.5 text-[12px] text-muted-foreground shadow-elev-1 md:flex">
          <span className="status-dot status-dot-busy" />
          Agent working
        </span>
      )}

      <div className="hidden items-center gap-0.5 rounded-lg border border-border bg-surface-muted/80 p-0.5 shadow-elev-1 sm:flex">
        {layoutOptions.map((option) => {
          const Icon = option.icon;
          const active = paneLayout === option.id;
          return (
            <Tooltip key={option.id}>
              <TooltipTrigger asChild>
                <button
                  aria-label={option.label}
                  aria-pressed={active}
                  className={cn(
                    "flex size-6 items-center justify-center rounded-[6px] transition-all",
                    active
                      ? "bg-surface text-foreground shadow-elev-1"
                      : "text-muted-foreground hover:text-foreground"
                  )}
                  onClick={() => setPaneLayout(option.id)}
                  type="button"
                >
                  <Icon className="size-3.5" />
                </button>
              </TooltipTrigger>
              <TooltipContent>
                {option.label}
                {option.hint ? ` · ${option.hint}` : ""}
              </TooltipContent>
            </Tooltip>
          );
        })}
      </div>

      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            aria-label="New session"
            onClick={() => createSession()}
            size="icon-sm"
            variant="ghost"
          >
            <PlusIcon className="size-4" />
          </Button>
        </TooltipTrigger>
        <TooltipContent>New session · ⌘N</TooltipContent>
      </Tooltip>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button aria-label="Workspace menu" size="icon-sm" variant="ghost">
            <EllipsisIcon className="size-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuLabel>Workspace</DropdownMenuLabel>
          <DropdownMenuItem onClick={() => setPaneLayout("two")}>
            <Columns2Icon className="size-3.5" /> Split view
            <DropdownMenuShortcut>⌘\</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuItem
            onClick={() =>
              setPaneLayout(paneLayout === "workspace" ? "two" : "workspace")
            }
          >
            <Maximize2Icon className="size-3.5" /> Toggle workspace focus
            <DropdownMenuShortcut>⌘J</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => navigate("/inspector")}>
            <PanelRightOpenIcon className="size-3.5" /> Open inspector
          </DropdownMenuItem>
          <DropdownMenuItem onClick={onOpenSettings}>
            <SparklesIcon className="size-3.5" /> Agent settings
          </DropdownMenuItem>
          <DropdownMenuItem
            onClick={() => clearSession(activeSessionId)}
            variant="destructive"
          >
            <Trash2Icon className="size-3.5" /> Clear conversation
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </header>
  );
}
