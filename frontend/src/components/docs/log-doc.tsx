import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import { lineFor } from "@/lib/logs.ts";

export function LogDocument({ count }: { count: number }) {
  const parent = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => parent.current,
    estimateSize: () => 22,
    overscan: 6,
    initialRect: { width: 640, height: 384 },
  });
  return (
    <div
      ref={parent}
      data-log-scroll
      data-log-count={count}
      role="log"
      aria-label="System log"
      className="pane-scroll h-96 overflow-auto font-mono text-value"
    >
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((item) => (
          <div
            key={item.key}
            data-log-row
            data-log-index={item.index}
            className="absolute right-0 left-0 flex h-[22px] items-center px-2"
            style={{ transform: `translateY(${item.start}px)` }}
          >
            {lineFor(item.index)}
          </div>
        ))}
      </div>
    </div>
  );
}
