"use client";

import {
  CodeBlock,
  CodeBlockCopyButton,
} from "@/agentgui/components/ai-elements/code-block";
import { Button } from "@/agentgui/components/ui/button";
import { Textarea } from "@/agentgui/components/ui/textarea";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import type { Artifact } from "@/agentgui/types/workspace";
import type { BundledLanguage } from "shiki";
import { CheckIcon, PencilIcon } from "lucide-react";
import { useState } from "react";

const languageAliases: Record<string, BundledLanguage> = {
  ts: "typescript",
  js: "javascript",
  sh: "bash",
  md: "markdown",
  json: "json",
  csv: "text" as BundledLanguage,
  text: "text" as BundledLanguage,
  plaintext: "text" as BundledLanguage,
};

function shikiLanguage(language?: string): BundledLanguage {
  if (!language) return "text" as BundledLanguage;
  return (
    languageAliases[language] ?? (language as BundledLanguage)
  );
}

export function CodeView({ artifact }: { artifact: Artifact }) {
  const update = useWorkspaceStore((state) => state.updateArtifactContent);
  const save = useWorkspaceStore((state) => state.saveArtifact);
  const [editing, setEditing] = useState(false);

  const lines = artifact.content.split("\n").length;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border/70 bg-surface-muted/40 px-3">
        <span className="rounded border border-border bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
          {artifact.language ?? "text"}
        </span>
        <span className="font-mono text-[10px] text-muted-foreground">
          {lines} lines
        </span>
        <div className="flex-1" />
        <Button
          className="h-6 gap-1 px-2 text-[10.5px]"
          onClick={() => setEditing((value) => !value)}
          size="sm"
          variant="ghost"
        >
          <PencilIcon className="size-3" />
          {editing ? "Preview" : "Edit"}
        </Button>
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

      {editing ? (
        <div className="min-h-0 flex-1 p-3">
          <Textarea
            className="h-full min-h-full resize-none border-border/70 bg-surface-muted/30 font-mono text-[12px] leading-relaxed shadow-none focus-visible:ring-1"
            onChange={(event) => update(artifact.id, event.target.value)}
            spellCheck={false}
            value={artifact.content}
          />
        </div>
      ) : (
        <div className="relative min-h-0 flex-1 overflow-auto p-3">
          <CodeBlock
            className="border-border/70 text-[12px]"
            code={artifact.content}
            language={shikiLanguage(artifact.language)}
            showLineNumbers
          >
            <CodeBlockCopyButton />
          </CodeBlock>
        </div>
      )}
    </div>
  );
}
