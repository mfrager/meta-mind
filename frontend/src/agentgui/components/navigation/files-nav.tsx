"use client";

import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { openFileInWorkspace } from "@/agentgui/lib/artifacts";
import { cn, formatBytes } from "@/agentgui/lib/utils";
import type { FileEntry } from "@/agentgui/types/workspace";
import {
  ChevronRightIcon,
  FileCode2Icon,
  FileJsonIcon,
  FileTextIcon,
  FolderIcon,
  TableIcon,
} from "lucide-react";
import { useMemo } from "react";

function FileGlyph({ file }: { file: FileEntry }) {
  if (file.kind === "folder") {
    return <FolderIcon className="size-3.5 shrink-0 text-muted-foreground" />;
  }
  if (file.language === "json") {
    return <FileJsonIcon className="size-3.5 shrink-0 text-muted-foreground" />;
  }
  if (file.language === "csv") {
    return <TableIcon className="size-3.5 shrink-0 text-muted-foreground" />;
  }
  if (["ts", "typescript", "tsx", "js", "rust", "python"].includes(file.language ?? "")) {
    return <FileCode2Icon className="size-3.5 shrink-0 text-muted-foreground" />;
  }
  return <FileTextIcon className="size-3.5 shrink-0 text-muted-foreground" />;
}

function FileNode({ file, depth }: { file: FileEntry; depth: number }) {
  const {
    expandedGroups,
    toggleGroup,
    activeFileId,
    selectFile,
  } = useWorkspaceStore();
  const open = expandedGroups[file.id] ?? false;

  if (file.kind === "folder") {
    return (
      <div>
        <button
          className="flex w-full items-center gap-1.5 rounded-md px-1.5 py-1 text-left transition-colors hover:bg-sidebar-accent/60"
          onClick={() => toggleGroup(file.id)}
          style={{ paddingLeft: `${6 + depth * 12}px` }}
          type="button"
        >
          <ChevronRightIcon
            className={cn(
              "size-3 shrink-0 text-muted-foreground transition-transform",
              open && "rotate-90"
            )}
          />
          <FolderIcon className="size-3.5 shrink-0 text-muted-foreground" />
          <span className="truncate text-[13px] font-medium text-foreground/85">
            {file.name}
          </span>
          <span className="ml-auto font-mono text-[11px] text-muted-foreground/70">
            {file.children?.length ?? 0}
          </span>
        </button>
        {open &&
          file.children?.map((child) => (
            <FileNode depth={depth + 1} file={child} key={child.id} />
          ))}
      </div>
    );
  }

  return (
    <button
      className={cn(
        "group/file flex w-full items-center gap-1.5 rounded-md py-1 pr-1.5 text-left transition-colors",
        activeFileId === file.id
          ? "active-nav-item"
          : "hover:bg-sidebar-accent/60"
      )}
      onClick={() => {
        selectFile(file.id);
        openFileInWorkspace(file);
      }}
      style={{ paddingLeft: `${6 + depth * 12 + 14}px` }}
      type="button"
    >
      <FileGlyph file={file} />
      <span className="truncate text-[13px] text-foreground/85">{file.name}</span>
      <span className="ml-auto shrink-0 font-mono text-[11px] text-muted-foreground/60 opacity-0 transition-opacity group-hover/file:opacity-100">
        {file.size ? formatBytes(file.size) : ""}
      </span>
    </button>
  );
}

export function FilesNav() {
  const files = useWorkspaceStore((state) => state.files);
  const query = useWorkspaceStore((state) => state.searchQuery);

  const filtered = useMemo(() => {
    if (!query) return files;
    const needle = query.toLowerCase();
    const filter = (entries: FileEntry[]): FileEntry[] =>
      entries.flatMap((entry) => {
        if (entry.kind === "file") {
          return entry.name.toLowerCase().includes(needle) ? [entry] : [];
        }
        const children = filter(entry.children ?? []);
        return children.length
          ? [{ ...entry, children }]
          : entry.name.toLowerCase().includes(needle)
            ? [entry]
            : [];
      });
    return filter(files);
  }, [files, query]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-between px-2 pb-2">
        <span className="text-[11px] font-medium tracking-wider text-muted-foreground uppercase">
          Workspace files
        </span>
        <span className="font-mono text-[11px] text-muted-foreground/70">
          {files.reduce((count, entry) => count + (entry.children?.length ?? 0), 0)}
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-1 pb-2">
        {filtered.map((file) => (
          <FileNode depth={0} file={file} key={file.id} />
        ))}
        {filtered.length === 0 && (
          <p className="px-2 py-6 text-[13px] text-muted-foreground">
            {query ? `Nothing matches “${query}”.` : "No workspace files yet."}
          </p>
        )}
      </div>
    </div>
  );
}
