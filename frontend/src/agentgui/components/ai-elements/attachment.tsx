"use client";

import { cn, formatBytes } from "@/agentgui/lib/utils";
import { FileTextIcon, ImageIcon, XIcon } from "lucide-react";
import type { ComponentProps } from "react";

export interface AttachmentFile {
  id: string;
  name: string;
  size?: number;
  kind?: "file" | "image";
}

export type AttachmentProps = ComponentProps<"div"> & {
  file: AttachmentFile;
  onRemove?: () => void;
};

export const Attachment = ({
  file,
  onRemove,
  className,
  ...props
}: AttachmentProps) => {
  const Icon = file.kind === "image" ? ImageIcon : FileTextIcon;

  return (
    <div
      className={cn(
        "flex max-w-56 items-center gap-2 rounded-lg border border-border bg-surface py-1.5 pr-1.5 pl-2 shadow-elev-1",
        className
      )}
      {...props}
    >
      <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-surface-sunken text-muted-foreground">
        <Icon className="size-3.5" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[12px] font-medium leading-tight">
          {file.name}
        </span>
        {file.size !== undefined && (
          <span className="block font-mono text-[10px] text-muted-foreground">
            {formatBytes(file.size)}
          </span>
        )}
      </span>
      {onRemove && (
        <button
          aria-label={`Remove ${file.name}`}
          className="flex size-5 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-surface-sunken hover:text-foreground"
          onClick={onRemove}
          type="button"
        >
          <XIcon className="size-3" />
        </button>
      )}
    </div>
  );
};

export type AttachmentListProps = ComponentProps<"div"> & {
  files: AttachmentFile[];
  onRemove?: (id: string) => void;
};

export const AttachmentList = ({
  files,
  onRemove,
  className,
  ...props
}: AttachmentListProps) => {
  if (files.length === 0) return null;

  return (
    <div className={cn("flex flex-wrap items-center gap-1.5", className)} {...props}>
      {files.map((file) => (
        <Attachment
          file={file}
          key={file.id}
          onRemove={onRemove ? () => onRemove(file.id) : undefined}
        />
      ))}
    </div>
  );
};
