"use client";

import { cn } from "@/agentgui/lib/utils";
import type { Artifact } from "@/agentgui/types/workspace";

export function TerminalView({ artifact }: { artifact: Artifact }) {
  const lines = artifact.content.replace(/\n$/, "").split("\n");

  return (
    <div className="flex h-full min-h-0 flex-col p-3">
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-[oklch(0.36_0.02_255)] bg-[oklch(0.22_0.018_255)] shadow-elev-2">
        <div className="flex items-center gap-1.5 border-b border-white/8 px-3 py-2">
          <span className="size-2 rounded-full bg-[oklch(0.65_0.18_25)]" />
          <span className="size-2 rounded-full bg-[oklch(0.75_0.14_85)]" />
          <span className="size-2 rounded-full bg-[oklch(0.65_0.13_150)]" />
          <span className="ml-2 font-mono text-[10px] text-white/45">
            agentgui — zsh
          </span>
        </div>
        <div className="min-h-0 flex-1 overflow-auto px-3 py-2.5">
          {lines.map((line, index) => (
            <div
              className={cn(
                "font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap",
                line.startsWith("$") && "text-[oklch(0.86_0.08_190)]",
                line.startsWith("✓") && "text-[oklch(0.82_0.13_155)]",
                line.startsWith("▸") && "text-white/65",
                !line.startsWith("$") &&
                  !line.startsWith("✓") &&
                  !line.startsWith("▸") &&
                  "text-white/55"
              )}
              key={`${line}-${index}`}
            >
              {line || "\u00a0"}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
