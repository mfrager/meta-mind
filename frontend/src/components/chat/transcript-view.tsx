import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import { InputRequest } from "@/components/chat/input-request.tsx";
import { Turn } from "@/components/chat/turn.tsx";
import type { TranscriptItem } from "@/lib/transcript/reducer.ts";

export function TranscriptView({
  items,
  onAnswer,
}: {
  items: TranscriptItem[];
  onAnswer: (id: string, value: string) => void;
}) {
  const liveKey = lastAssistantKey(items);
  if (items.length < 40) {
    return (
      <div role="log" aria-label="Transcript" data-transcript className="space-y-3 p-3">
        {items.map((item) => (
          <Row key={`${item.kind}:${item.id}`} item={item} live={liveKey === `${item.kind}:${item.id}`} onAnswer={onAnswer} />
        ))}
      </div>
    );
  }
  return <VirtualTranscript items={items} onAnswer={onAnswer} />;
}

function VirtualTranscript({
  items,
  onAnswer,
}: {
  items: TranscriptItem[];
  onAnswer: (id: string, value: string) => void;
}) {
  const parent = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => parent.current,
    estimateSize: () => 72,
    overscan: 4,
    initialRect: { width: 640, height: 384 },
  });
  return (
    <div ref={parent} role="log" aria-label="Transcript" data-transcript data-transcript-scroll className="h-full overflow-auto p-3">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((row) => {
          const item = items[row.index];
          if (!item) return null;
          return (
            <div
              key={`${item.kind}:${item.id}`}
              data-index={row.index}
              ref={virtualizer.measureElement}
              style={{ position: "absolute", top: 0, left: 0, width: "100%", transform: `translateY(${row.start}px)` }}
            >
              <Row item={item} onAnswer={onAnswer} />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function lastAssistantKey(items: TranscriptItem[]): string | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (item && (item.kind === "final" || item.kind === "turn")) return `${item.kind}:${item.id}`;
  }
  return null;
}

function Row({ item, live, onAnswer }: { item: TranscriptItem; live?: boolean; onAnswer: (id: string, value: string) => void }) {
  if (item.kind === "user") {
    return (
      <p data-item={`user:${item.content}`} className="text-sm">
        {item.content}
      </p>
    );
  }
  if (item.kind === "turn") {
    const op = item.ops.map((call) => `${call.name}:${reasonText(call.output)}`).join("|");
    return (
      <div data-item={`turn:${item.phase}:${item.text}${op ? `|${op}` : ""}`} aria-live={live ? "polite" : undefined}>
        <Turn item={item} />
      </div>
    );
  }
  if (item.kind === "input_request") {
    return (
      <div data-item={`input:${item.request.prompt ?? ""}`}>
        <InputRequest request={item.request} answered={item.answered} onAnswer={(value) => onAnswer(item.id, value)} />
      </div>
    );
  }
  if (item.kind === "final") {
    return (
      <p data-item={`final:${item.text}`} className="text-sm" aria-live={live ? "polite" : undefined}>
        {item.text}
      </p>
    );
  }
  return (
    <p data-item={`system:${item.message}`} className="text-sm text-muted-foreground">
      {item.message}
      {item.reason ? ` ${item.reason}` : ""}
    </p>
  );
}

function reasonText(output: unknown) {
  if (!output || typeof output !== "object") return "";
  const reason = (output as { reason?: unknown }).reason;
  return typeof reason === "string" ? reason : "";
}
