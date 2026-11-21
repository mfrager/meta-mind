"use client";

import { MessageResponse } from "@/agentgui/components/ai-elements/message";
import { Badge } from "@/agentgui/components/ui/badge";
import { Button } from "@/agentgui/components/ui/button";
import { Textarea } from "@/agentgui/components/ui/textarea";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn } from "@/agentgui/lib/utils";
import type { Artifact } from "@/agentgui/types/workspace";
import { CheckIcon, EyeIcon, PencilIcon } from "lucide-react";
import { useState } from "react";

export function DocumentView({ artifact }: { artifact: Artifact }) {
  const update = useWorkspaceStore((state) => state.updateArtifactContent);
  const save = useWorkspaceStore((state) => state.saveArtifact);
  const [mode, setMode] = useState<"preview" | "edit">("preview");

  const words = artifact.content.trim().split(/\s+/).filter(Boolean).length;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border/70 bg-surface-muted/40 px-3">
        <div className="flex items-center gap-0.5 rounded-md border border-border bg-surface p-0.5">
          <button
            className={cn(
              "flex items-center gap-1 rounded-[5px] px-1.5 py-0.5 text-[10.5px] font-medium transition-colors",
              mode === "preview"
                ? "bg-surface-sunken text-foreground"
                : "text-muted-foreground hover:text-foreground"
            )}
            onClick={() => setMode("preview")}
            type="button"
          >
            <EyeIcon className="size-3" /> Preview
          </button>
          <button
            className={cn(
              "flex items-center gap-1 rounded-[5px] px-1.5 py-0.5 text-[10.5px] font-medium transition-colors",
              mode === "edit"
                ? "bg-surface-sunken text-foreground"
                : "text-muted-foreground hover:text-foreground"
            )}
            onClick={() => setMode("edit")}
            type="button"
          >
            <PencilIcon className="size-3" /> Edit
          </button>
        </div>

        <span className="font-mono text-[10px] text-muted-foreground">
          {words} words
        </span>

        <div className="flex-1" />

        {artifact.dirty && (
          <Badge
            className="h-4 rounded px-1.5 text-[9.5px] font-normal"
            variant="secondary"
          >
            unsaved
          </Badge>
        )}

        <Button
          className="h-6 gap-1 px-2 text-[10.5px]"
          disabled={!artifact.dirty}
          onClick={() => save(artifact.id)}
          size="sm"
          variant={artifact.dirty ? "default" : "ghost"}
        >
          <CheckIcon className="size-3" /> Save
        </Button>
      </div>

      {mode === "edit" ? (
        <div className="min-h-0 flex-1 p-3">
          <Textarea
            className="h-full min-h-full resize-none border-border/70 bg-surface-muted/30 font-mono text-[12.5px] leading-relaxed shadow-none focus-visible:ring-1"
            onChange={(event) => update(artifact.id, event.target.value)}
            spellCheck={false}
            value={artifact.content}
          />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <article className="mx-auto w-full max-w-2xl px-6 py-6">
            <MessageResponse className="markdown-body">
              {artifact.content}
            </MessageResponse>
          </article>
        </div>
      )}
    </div>
  );
}
