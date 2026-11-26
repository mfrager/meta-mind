"use client";

import { cn, formatBytes, formatTimestamp } from "@/agentgui/lib/utils";
import type { Artifact, FileEntry } from "@/agentgui/types/workspace";
import {
  ChevronRightIcon,
  FileCode2Icon,
  FileJsonIcon,
  FileTextIcon,
  FolderIcon,
  TableIcon,
} from "lucide-react";
import { useMemo, useState } from "react";

function Row({
  file,
  depth,
  expanded,
  onToggle,
}: {
  file: FileEntry;
  depth: number;
  expanded: boolean;
  onToggle: () => void;
}) {
  const isFolder = file.kind === "folder";
  const isCode = ["ts", "typescript", "tsx", "js", "rust", "python"].includes(
    file.language ?? ""
  );
  const Icon = isFolder
    ? FolderIcon
    : file.language === "json"
      ? FileJsonIcon
      : file.language === "csv"
        ? TableIcon
        : isCode
          ? FileCode2Icon
          : FileTextIcon;

  return (
    <>
      <button
        className={cn(
          "group flex w-full items-center gap-2 rounded-md py-1.5 pr-2 text-left transition-colors hover:bg-surface-muted",
          isFolder && "font-medium"
        )}
        onClick={isFolder ? onToggle : undefined}
        style={{ paddingLeft: `${10 + depth * 14}px` }}
        type="button"
      >
        {isFolder ? (
          <ChevronRightIcon
            className={cn(
              "size-3 shrink-0 text-muted-foreground transition-transform",
              expanded && "rotate-90"
            )}
          />
        ) : (
          <span className="w-3 shrink-0" />
        )}
        <Icon className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-[12px]">{file.name}</span>
        {file.size !== undefined && (
          <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
            {formatBytes(file.size)}
          </span>
        )}
        {file.updatedAt && (
          <span className="w-16 shrink-0 text-right font-mono text-[10px] text-muted-foreground/60">
            {formatTimestamp(file.updatedAt)}
          </span>
        )}
      </button>
      {isFolder &&
        expanded &&
        file.children?.map((child) => (
          <TreeRow depth={depth + 1} file={child} key={child.id} />
        ))}
    </>
  );
}

function TreeRow({ file, depth }: { file: FileEntry; depth: number }) {
  const [expanded, setExpanded] = useState(depth === 0);
  return (
    <Row
      depth={depth}
      expanded={expanded}
      file={file}
      onToggle={() => setExpanded((value) => !value)}
    />
  );
}

export function FilesView({ artifact }: { artifact: Artifact }) {
  const files = useMemo<FileEntry[]>(() => {
    try {
      const parsed = JSON.parse(artifact.content) as FileEntry[];
      if (Array.isArray(parsed)) return parsed;
    } catch {
      /* fall through */
    }
    return [];
  }, [artifact.content]);

  return (
    <div className="h-full overflow-auto p-4">
      <div className="mx-auto max-w-2xl">
        <div className="mb-2 flex items-center justify-between px-1">
          <p className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
            Workspace tree
          </p>
          <p className="font-mono text-[10px] text-muted-foreground">
            {files.length} roots
          </p>
        </div>
        <div className="overflow-hidden rounded-xl border border-border bg-surface shadow-elev-1">
          {files.map((file) => (
            <TreeRow depth={0} file={file} key={file.id} />
          ))}
          {files.length === 0 && (
            <p className="p-4 text-[12px] text-muted-foreground">
              No file tree data in this artifact.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
