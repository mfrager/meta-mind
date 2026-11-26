"use client";

import type { Artifact } from "@/agentgui/types/workspace";

export function PreviewView({ artifact }: { artifact: Artifact }) {
  const isHtml = /^\s*<(!doctype|html|div|body)/i.test(artifact.content);

  if (!isHtml) {
    return (
      <div className="h-full overflow-auto p-5">
        <pre className="font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-foreground/80">
          {artifact.content}
        </pre>
      </div>
    );
  }

  return (
    <div className="h-full p-3">
      <iframe
        className="h-full w-full rounded-xl border border-border bg-white shadow-elev-1"
        sandbox="allow-scripts"
        srcDoc={artifact.content}
        title={artifact.title}
      />
    </div>
  );
}
