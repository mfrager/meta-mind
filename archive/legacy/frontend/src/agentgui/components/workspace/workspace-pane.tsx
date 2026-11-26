"use client";

import { LlmLogDock } from "@/components/chat/llm-log-dock.tsx";
import { ArtifactView } from "@/agentgui/components/workspace/artifact-view";
import { StateView } from "@/agentgui/components/workspace/state-view";
import { Badge } from "@/agentgui/components/ui/badge";
import { Button } from "@/agentgui/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/agentgui/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/agentgui/components/ui/tooltip";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn, downloadText, formatRelative } from "@/agentgui/lib/utils";
import type { ArtifactKind } from "@/agentgui/types/workspace";
import {
  ActivityIcon,
  BracesIcon,
  ChartColumnIcon,
  CheckIcon,
  Code2Icon,
  CopyIcon,
  DownloadIcon,
  EllipsisIcon,
  EyeIcon,
  FileTextIcon,
  FolderTreeIcon,
  ListChecksIcon,
  LoaderCircleIcon,
  Maximize2Icon,
  Minimize2Icon,
  PanelRightCloseIcon,
  ScrollTextIcon,
  TableIcon,
  TerminalIcon,
  XIcon,
} from "lucide-react";

const kindMeta: Record<
  ArtifactKind,
  { icon: typeof FileTextIcon; label: string }
> = {
  document: { icon: FileTextIcon, label: "Document" },
  code: { icon: Code2Icon, label: "Code" },
  json: { icon: BracesIcon, label: "JSON" },
  table: { icon: TableIcon, label: "Table" },
  chart: { icon: ChartColumnIcon, label: "Chart" },
  terminal: { icon: TerminalIcon, label: "Terminal" },
  plan: { icon: ListChecksIcon, label: "Plan" },
  files: { icon: FolderTreeIcon, label: "Files" },
  trace: { icon: ScrollTextIcon, label: "Log" },
  preview: { icon: EyeIcon, label: "Preview" },
};

const statusLabel = {
  ready: "ready",
  generating: "generating",
  synced: "synced",
  failed: "failed",
} as const;

export function WorkspacePane() {
  const artifacts = useWorkspaceStore((state) => state.artifacts);
  const openArtifactIds = useWorkspaceStore((state) => state.openArtifactIds);
  const activeArtifactId = useWorkspaceStore(
    (state) => state.activeArtifactId
  );
  const setActiveArtifact = useWorkspaceStore(
    (state) => state.setActiveArtifact
  );
  const closeArtifact = useWorkspaceStore((state) => state.closeArtifact);
  const paneLayout = useWorkspaceStore((state) => state.paneLayout);
  const setPaneLayout = useWorkspaceStore((state) => state.setPaneLayout);
  const activeSessionId = useWorkspaceStore((state) => state.activeSessionId);

  const tabs = openArtifactIds
    .map((id) => artifacts[id])
    .filter((artifact): artifact is NonNullable<typeof artifact> =>
      Boolean(artifact)
    );
  const active = activeArtifactId ? artifacts[activeArtifactId] : undefined;
  const meta = active ? kindMeta[active.kind] : undefined;
  const ActiveIcon = meta?.icon;
  const showingState = !active;

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface" data-agent-workspace-pane>
      <div className="pane-header flex h-11 shrink-0 items-stretch gap-1 px-2">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto py-1.5">
          <div
            className={cn(
              "relative flex h-full shrink-0 items-center gap-1.5 rounded-lg border px-2 text-[13px] transition-all",
              showingState
                ? "border-primary/20 bg-surface shadow-elev-1"
                : "border-transparent text-muted-foreground hover:bg-surface-muted/70 hover:text-foreground"
            )}
            data-tab="state"
          >
            <button
              className="flex min-w-0 items-center gap-1.5 py-1"
              onClick={() => setActiveArtifact(null)}
              type="button"
            >
              <ActivityIcon
                className={cn(
                  "size-3.5 shrink-0",
                  showingState ? "text-primary" : "text-muted-foreground"
                )}
              />
              <span className="truncate">State</span>
            </button>
            {showingState && (
              <span className="absolute inset-x-2 top-0 h-[1.5px] rounded-full bg-[linear-gradient(90deg,var(--primary),oklch(0.63_0.12_169))]" />
            )}
          </div>
          {tabs.map((tab) => {
            const Icon = kindMeta[tab.kind].icon;
            const isActive = tab.id === activeArtifactId;
            return (
              <div
                className={cn(
                  "group/tab relative flex h-full max-w-52 shrink-0 items-center gap-1.5 rounded-lg border px-2 text-[13px] transition-all",
                  isActive
                    ? "border-primary/20 bg-surface shadow-elev-1"
                    : "border-transparent text-muted-foreground hover:bg-surface-muted/70 hover:text-foreground"
                )}
                key={tab.id}
              >
                <button
                  className="flex min-w-0 items-center gap-1.5 py-1"
                  onClick={() => setActiveArtifact(tab.id)}
                  type="button"
                >
                  <Icon
                    className={cn(
                      "size-3.5 shrink-0",
                      isActive ? "text-primary" : "text-muted-foreground"
                    )}
                  />
                  <span className="truncate">{tab.title}</span>
                  {tab.dirty && (
                    <span
                      className="size-1.5 shrink-0 rounded-full bg-caution"
                      title="Unsaved changes"
                    />
                  )}
                  {tab.status === "generating" && (
                    <LoaderCircleIcon className="size-3 shrink-0 animate-spin text-primary" />
                  )}
                </button>
                <button
                  aria-label={`Close ${tab.title}`}
                  className={cn(
                    "flex size-4 shrink-0 items-center justify-center rounded text-muted-foreground transition-opacity hover:bg-surface-sunken hover:text-foreground",
                    isActive
                      ? "opacity-70 hover:opacity-100"
                      : "opacity-0 group-hover/tab:opacity-100"
                  )}
                  onClick={() => closeArtifact(tab.id)}
                  type="button"
                >
                  <XIcon className="size-3" />
                </button>
                {isActive && (
                  <span className="absolute inset-x-2 top-0 h-[1.5px] rounded-full bg-[linear-gradient(90deg,var(--primary),oklch(0.63_0.12_169))]" />
                )}
              </div>
            );
          })}

        </div>

        <div className="flex shrink-0 items-center gap-0.5 py-1.5">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="Copy artifact"
                disabled={!active}
                onClick={() =>
                  active && void navigator.clipboard?.writeText(active.content)
                }
                size="icon-xs"
                variant="ghost"
              >
                <CopyIcon className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Copy content</TooltipContent>
          </Tooltip>

          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="Download artifact"
                disabled={!active}
                onClick={() =>
                  active && downloadText(active.title, active.content)
                }
                size="icon-xs"
                variant="ghost"
              >
                <DownloadIcon className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Download</TooltipContent>
          </Tooltip>

          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="Toggle workspace focus"
                onClick={() =>
                  setPaneLayout(paneLayout === "workspace" ? "two" : "workspace")
                }
                size="icon-xs"
                variant="ghost"
              >
                {paneLayout === "workspace" ? (
                  <Minimize2Icon className="size-3.5" />
                ) : (
                  <Maximize2Icon className="size-3.5" />
                )}
              </Button>
            </TooltipTrigger>
            <TooltipContent>Toggle workspace focus · ⌘J</TooltipContent>
          </Tooltip>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label="Artifact actions"
                disabled={!active}
                size="icon-xs"
                variant="ghost"
              >
                <EllipsisIcon className="size-3.5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-48">
              <DropdownMenuItem
                onClick={() => active && closeArtifact(active.id)}
              >
                <XIcon className="size-3.5" /> Close tab
              </DropdownMenuItem>
              <DropdownMenuItem
                onClick={() => {
                  if (!active) return;
                  tabs
                    .filter((tab) => tab.id !== active.id)
                    .forEach((tab) => closeArtifact(tab.id));
                }}
              >
                <PanelRightCloseIcon className="size-3.5" /> Close other tabs
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onClick={() => setPaneLayout("workspace")}
              >
                <Maximize2Icon className="size-3.5" /> Focus workspace
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {active && meta && ActiveIcon ? (
        <>
          <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border/70 bg-surface-muted/40 px-3">
            <ActiveIcon className="size-3.5 text-muted-foreground" />
            <span className="text-[12.5px] font-medium">{active.title}</span>
            <Badge
              className="h-4 rounded px-1.5 text-[10.5px] font-normal text-muted-foreground"
              variant="secondary"
            >
              {meta.label}
            </Badge>
            <span className="font-mono text-[11px] text-muted-foreground/80">
              {statusLabel[active.status]}
              {active.updatedAt
                ? ` · ${formatRelative(active.updatedAt)}`
                : ""}
            </span>
            <div className="flex-1" />
            {active.dirty ? (
              <span className="flex items-center gap-1 text-[11px] text-caution">
                unsaved changes
              </span>
            ) : (
              <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
                <CheckIcon className="size-2.5 text-positive" />
                in sync
              </span>
            )}
          </div>

          <div className="relative min-h-0 flex-1 overflow-hidden">
            {tabs.map((tab) => (
              <div
                className={cn(
                  "absolute inset-0",
                  tab.id === active.id ? "block" : "hidden"
                )}
                key={tab.id}
              >
                <ArtifactView artifact={tab} />
              </div>
            ))}
          </div>
        </>
      ) : (
        <StateView sessionId={activeSessionId} />
      )}

      <LlmLogDock />
    </div>
  );
}
