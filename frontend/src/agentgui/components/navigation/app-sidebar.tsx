"use client";

import { Button } from "@/agentgui/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/agentgui/components/ui/dropdown-menu";
import { Input } from "@/agentgui/components/ui/input";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/agentgui/components/ui/tooltip";
import { useServerHealth } from "@/agentgui/hooks/use-server-health";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn } from "@/agentgui/lib/utils";
import type { SidebarTab } from "@/agentgui/types/workspace";
import {
  FilesIcon,
  FolderKanbanIcon,
  MessagesSquareIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  PlusIcon,
  SearchIcon,
  Settings2Icon,
  SparklesIcon,
  UserRoundIcon,
} from "lucide-react";

import { FilesNav } from "./files-nav";
import { ProjectsNav } from "./projects-nav";
import { SessionsNav } from "./sessions-nav";

const tabs: { id: SidebarTab; label: string; icon: typeof MessagesSquareIcon }[] =
  [
    { id: "sessions", label: "Sessions", icon: MessagesSquareIcon },
    { id: "projects", label: "Projects", icon: FolderKanbanIcon },
    { id: "files", label: "Files", icon: FilesIcon },
  ];

function ConnectionBadge({ compact }: { compact: boolean }) {
  const { health, checking } = useServerHealth();
  const transport = useWorkspaceStore((state) => state.transport);
  const setTransport = useWorkspaceStore((state) => state.setTransport);

  const connected = transport === "mock" || Boolean(health);
  const label = checking
    ? "Checking backend…"
    : transport === "mock"
      ? "Mock responses"
      : health
        ? health.demo
          ? "Demo backend"
          : "Metamind online"
        : "Backend offline";

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          className={cn(
            "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-sidebar-accent/60",
            compact && "justify-center px-0"
          )}
          type="button"
        >
          <span
            className={cn(
              "size-1.5 shrink-0 rounded-full",
              connected ? "bg-positive" : "bg-caution"
            )}
            style={
              connected
                ? { boxShadow: "0 0 0 3px color-mix(in oklab, var(--positive) 16%, transparent)" }
                : undefined
            }
          />
          {!compact && (
            <>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[12.5px] leading-tight font-medium">
                  {label}
                </span>
                <span className="block truncate font-mono text-[11px] text-muted-foreground">
                  {transport === "mock"
                    ? "local simulation"
                    : health
                      ? `${health.backend} · ${health.model}`
                      : "start the rust server"}
                </span>
              </span>
              <Settings2Icon className="size-3.5 shrink-0 text-muted-foreground" />
            </>
          )}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-56" side="top">
        <DropdownMenuLabel>Agent transport</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => setTransport(value as "server" | "mock")}
          value={transport}
        >
          <DropdownMenuRadioItem value="server">
            Rust server (SSE)
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="mock">
            Local mock stream
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <div className="px-2 py-1.5 font-mono text-[11px] text-muted-foreground">
          {health
            ? `${health.status} · v${health.version}`
            : "No health signal from /api/health"}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function SidebarContent({ compact }: { compact: boolean }) {
  const sidebarTab = useWorkspaceStore((state) => state.sidebarTab);
  const setSidebarTab = useWorkspaceStore((state) => state.setSidebarTab);
  const searchQuery = useWorkspaceStore((state) => state.searchQuery);
  const setSearchQuery = useWorkspaceStore((state) => state.setSearchQuery);
  const createSession = useWorkspaceStore((state) => state.createSession);
  const toggleSidebarCompact = useWorkspaceStore(
    (state) => state.toggleSidebarCompact
  );
  const setMobileSidebarOpen = useWorkspaceStore(
    (state) => state.setMobileSidebarOpen
  );

  if (compact) {
    return (
      <div className="flex h-full flex-col items-center gap-1 py-3">
        <div className="accent-gradient mb-1 flex size-8 items-center justify-center rounded-[10px] text-white shadow-elev-2">
          <SparklesIcon className="size-4" />
        </div>

        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label="Expand sidebar"
              onClick={toggleSidebarCompact}
              size="icon-sm"
              variant="ghost"
            >
              <PanelLeftOpenIcon className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">Expand sidebar · ⌘B</TooltipContent>
        </Tooltip>

        <div className="my-1 h-px w-6 bg-sidebar-border" />

        {tabs.map((tab) => {
          const Icon = tab.icon;
          return (
            <Tooltip key={tab.id}>
              <TooltipTrigger asChild>
                <button
                  aria-label={tab.label}
                  className={cn(
                    "flex size-8 items-center justify-center rounded-lg transition-colors",
                    sidebarTab === tab.id
                      ? "active-nav-item text-foreground"
                      : "text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground"
                  )}
                  onClick={() => setSidebarTab(tab.id)}
                  type="button"
                >
                  <Icon className="size-4" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">{tab.label}</TooltipContent>
            </Tooltip>
          );
        })}

        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label="New session"
              className="mt-1"
              onClick={() => createSession()}
              size="icon-sm"
              variant="ghost"
            >
              <PlusIcon className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">New session · ⌘N</TooltipContent>
        </Tooltip>

        <div className="mt-auto flex flex-col items-center gap-2">
          <ConnectionBadge compact />
          <button
            aria-label="Account"
            className="flex size-7 items-center justify-center rounded-full bg-sidebar-accent text-muted-foreground"
            type="button"
          >
            <UserRoundIcon className="size-3.5" />
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-2 px-3 py-3">
        <div className="accent-gradient flex size-8 items-center justify-center rounded-[10px] text-white shadow-elev-2">
          <SparklesIcon className="size-4" />
        </div>
        <div className="min-w-0 flex-1">
          <p className="truncate text-[14px] font-semibold tracking-tight">
            Metamind
          </p>
          <p className="truncate font-mono text-[11px] text-muted-foreground">
            metamind · local kernel
          </p>
        </div>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label="Collapse sidebar"
              onClick={toggleSidebarCompact}
              size="icon-xs"
              variant="ghost"
            >
              <PanelLeftCloseIcon className="size-3.5" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Collapse sidebar · ⌘B</TooltipContent>
        </Tooltip>
      </div>

      <div className="px-2">
        <div className="flex items-center gap-0.5 rounded-lg border border-sidebar-border bg-sidebar-accent/40 p-0.5">
          {tabs.map((tab) => {
            const Icon = tab.icon;
            const active = sidebarTab === tab.id;
            return (
              <button
                className={cn(
                  "flex flex-1 items-center justify-center gap-1.5 rounded-[7px] py-1.5 text-[13px] font-medium transition-all",
                  active
                    ? "bg-surface text-foreground shadow-elev-1"
                    : "text-muted-foreground hover:text-foreground"
                )}
                key={tab.id}
                onClick={() => setSidebarTab(tab.id)}
                type="button"
              >
                <Icon className="size-3.5" />
                {tab.label}
              </button>
            );
          })}
        </div>
      </div>

      <div className="px-2 py-2">
        <div className="relative">
          <SearchIcon className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            className="h-8 border-sidebar-border bg-surface/70 pl-7 text-[13px] shadow-none"
            onChange={(event) => setSearchQuery(event.target.value)}
            placeholder="Search sessions, projects, files…"
            value={searchQuery}
          />
          <span className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 rounded border border-border bg-surface px-1 font-mono text-[10px] text-muted-foreground">
            ⌘K
          </span>
        </div>
      </div>

      <div className="flex min-h-0 flex-1 flex-col">
        {sidebarTab === "sessions" && <SessionsNav compact={false} />}
        {sidebarTab === "projects" && <ProjectsNav />}
        {sidebarTab === "files" && <FilesNav />}
      </div>

      <div className="border-t border-sidebar-border px-2 py-2">
        <ConnectionBadge compact={false} />
        <button
          className="mt-0.5 flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-sidebar-accent/60"
          onClick={() => {
            setMobileSidebarOpen(false);
            setSearchQuery("");
          }}
          type="button"
        >
          <span className="flex size-6 items-center justify-center rounded-full bg-sidebar-accent text-muted-foreground">
            <UserRoundIcon className="size-3.5" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[13px] font-medium leading-tight">
              Workspace owner
            </span>
            <span className="block truncate text-[11px] text-muted-foreground">
              Local development
            </span>
          </span>
        </button>
      </div>
    </div>
  );
}

export function AppSidebar() {
  const compact = useWorkspaceStore((state) => state.sidebarCompact);

  return (
    <aside
      className={cn(
        "sidebar-shell hidden shrink-0 flex-col transition-[width] duration-200 lg:flex",
        compact ? "w-[60px]" : "w-[276px]"
      )}
    >
      <SidebarContent compact={compact} />
    </aside>
  );
}
