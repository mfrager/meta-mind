import { useMemo } from "react";

import type { CallGroup } from "@/lib/store/llm-log.ts";

import { textBlocks } from "./llm-log-format.ts";

export function LlmLogText({ groups }: { groups: CallGroup[] }) {
  const blocks = useMemo(() => textBlocks(groups), [groups]);
  return (
    <div className="divide-border/60 divide-y" data-llm-log-text>
      {blocks.map((block) => (
        <div
          key={block.key}
          data-llm-log-text-block
          data-kind={block.kind}
          data-depth={block.depth}
          className="px-2 py-1"
        >
          {block.lines.map((line, index) => (
            <div
              key={index}
              data-llm-log-text-line
              className="overflow-hidden text-ellipsis whitespace-nowrap"
            >
              {line}
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}
