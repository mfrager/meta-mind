"use client";

import { Button } from "@/agentgui/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/agentgui/components/ui/dropdown-menu";
import { findFileByName } from "@/agentgui/lib/store/workspace-store";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { openFileInWorkspace } from "@/agentgui/lib/artifacts";
import { cn, formatRelative } from "@/agentgui/lib/utils";
import type { Project } from "@/agentgui/types/workspace";
import {
  ChevronRightIcon,
  DatabaseIcon,
  EllipsisIcon,
  FileTextIcon,
  FolderGit2Icon,
  FolderPlusIcon,
  LayersIcon,
  MessagesSquareIcon,
  SparklesIcon,
} from "lucide-react";
import { useMemo, useState } from "react";

const accentClass: Record<Project["accent"], string> = {
  blue: "from-primary/85 to-primary/55",
  teal: "from-primary/80 to-[oklch(0.63_0.12_190)]",
  slate: "from-muted-foreground/60 to-muted-foreground/35",
  green: "from-[oklch(0.6_0.13_168)] to-[oklch(0.66_0.1_195)]",
};

const resourceIcon = {
  document: FileTextIcon,
  repository: FolderGit2Icon,
  dataset: DatabaseIcon,
  artifact: LayersIcon,
};

function ProjectRow({ project }: { project: Project }) {
  const { expandedGroups, toggleGroup, activeProjectId, selectProject } =
    useWorkspaceStore();
  const createSession = useWorkspaceStore((state) => state.createSession);
  const open = expandedGroups[project.id] ?? false;
  const active = project.id === activeProjectId;
  const [hovered, setHovered] = useState(false);

  return (
    <div className="mb-1">
      <div
        className={cn(
          "group/row flex items-start gap-2 rounded-lg px-2 py-1.5 transition-colors",
          active
            ? "active-nav-item"
            : "hover:bg-sidebar-accent/60 border border-transparent"
        )}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
      >
        <button
          className="flex min-w-0 flex-1 items-start gap-2 text-left"
          onClick={() => {
            selectProject(project.id);
            toggleGroup(project.id);
          }}
          type="button"
        >
          <span
            className={cn(
              "mt-0.5 size-2 shrink-0 rounded-[3px] bg-gradient-to-br",
              accentClass[project.accent]
            )}
          />
          <span className="min-w-0 flex-1">
            <span className="flex items-center gap-1">
              <span className="truncate text-[14px] font-medium leading-tight">
                {project.name}
              </span>
              <ChevronRightIcon
                className={cn(
                  "size-3 shrink-0 text-muted-foreground transition-transform",
                  open && "rotate-90"
                )}
              />
            </span>
            <span className="mt-0.5 flex items-center gap-1.5">
              <span className="truncate text-[12px] text-muted-foreground">
                {project.resources.length} resources
              </span>
              <span className="font-mono text-[11px] text-muted-foreground/70">
                {formatRelative(project.updatedAt)}
              </span>
            </span>
          </span>
        </button>

        {(hovered || active) && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label="Project actions"
                className="size-5 rounded-md"
                size="icon-xs"
                variant="ghost"
              >
                <EllipsisIcon className="size-3" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-48">
              <DropdownMenuItem onClick={() => createSession("New session", project.id)}>
                <MessagesSquareIcon className="size-3.5" /> New session here
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => toggleGroup(project.id)}>
                <LayersIcon className="size-3.5" /> Toggle resources
              </DropdownMenuItem>
              <DropdownMenuItem disabled>
                <FolderPlusIcon className="size-3.5" /> Add resource
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>

      {open && (
        <div className="mt-1 mb-1.5 ml-4 space-y-0.5 border-l border-sidebar-border pl-2">
          <p className="px-1 pb-1 text-[12px] leading-snug text-muted-foreground">
            {project.description}
          </p>
          {project.resources.map((resource) => {
            const Icon = resourceIcon[resource.kind];
            return (
              <button
                className="flex w-full items-center gap-1.5 rounded-md px-1.5 py-1 text-left transition-colors hover:bg-sidebar-accent/70"
                key={resource.id}
                onClick={() => {
                  const file = findFileByName(resource.label);
                  if (file) {
                    openFileInWorkspace(file);
                    return;
                  }
                  useWorkspaceStore.getState().openArtifact({
                    id: `resource:${resource.id}`,
                    title: resource.label,
                    kind: resource.kind === "dataset" ? "json" : "document",
                    content: `# ${resource.label}\n\nProject: ${project.name}\n\n${project.description}`,
                    origin: "file",
                    status: "synced",
                  });
                }}
                type="button"
              >
                <Icon className="size-3 shrink-0 text-muted-foreground" />
                <span className="truncate text-[13px] text-foreground/80">
                  {resource.label}
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

export function ProjectsNav() {
  const projects = useWorkspaceStore((state) => state.projects);
  const query = useWorkspaceStore((state) => state.searchQuery);
  const createSession = useWorkspaceStore((state) => state.createSession);

  const filtered = useMemo(() => {
    if (!query) return projects;
    const needle = query.toLowerCase();
    return projects.filter(
      (project) =>
        project.name.toLowerCase().includes(needle) ||
        project.description.toLowerCase().includes(needle) ||
        project.resources.some((resource) =>
          resource.label.toLowerCase().includes(needle)
        )
    );
  }, [projects, query]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-between px-2 pb-2">
        <span className="text-[11px] font-medium tracking-wider text-muted-foreground uppercase">
          Projects
        </span>
        <Button
          className="h-6 gap-1 px-1.5 text-[12px]"
          onClick={() => {
            createSession("New session");
          }}
          size="sm"
          variant="ghost"
        >
          <SparklesIcon className="size-3" /> Session
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-1">
        {filtered.map((project) => (
          <ProjectRow key={project.id} project={project} />
        ))}
        {filtered.length === 0 && (
          <p className="px-2 py-6 text-[13px] text-muted-foreground">
            {query ? `No projects match “${query}”.` : "No projects yet."}
          </p>
        )}
      </div>
    </div>
  );
}
