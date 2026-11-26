import { useEffect, useRef } from "react";

import type { PassView, TranscriptItem } from "@/lib/transcript/reducer.ts";

/**
 * The transcript is a view of the event stream: user bubbles come from
 * `turn.started`, pass progress from `pass.*`/`llm.*`, the assistant bubble from
 * `agent.message`, and failures from `turn.failed`.
 */
export function TranscriptView({ items }: { items: TranscriptItem[] }) {
  const scroller = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    element.scrollTop = element.scrollHeight;
  }, [items]);

  return (
    <div
      ref={scroller}
      role="log"
      aria-label="Transcript"
      data-transcript
      className="h-full overflow-auto p-3"
    >
      {items.length === 0 ? (
        <p className="text-muted-foreground text-sm">Ask Metamind anything. Every step appears in the log below.</p>
      ) : null}
      <div className="space-y-3">
        {items.map((item) => (
          <Item key={`${item.kind}:${item.id}`} item={item} />
        ))}
      </div>
    </div>
  );
}

function Item({ item }: { item: TranscriptItem }) {
  if (item.kind === "user") {
    return (
      <p data-item={`user:${item.text}`} className="text-sm font-medium">
        {item.text}
      </p>
    );
  }
  if (item.kind === "passes") {
    return (
      <article data-item="passes" className="border-border space-y-1 rounded-md border p-2">
        {item.passes.map((pass) => (
          <PassRow key={`${pass.pass}:${pass.attempt}`} pass={pass} />
        ))}
      </article>
    );
  }
  if (item.kind === "assistant") {
    return (
      <div data-item={`assistant:${item.text}`}>
        <p className="text-sm whitespace-pre-wrap">{item.text}</p>
        {item.verified ? (
          <span className="mt-1 inline-block rounded-pill border border-emerald-300 bg-emerald-50 px-2 py-0.5 text-xs text-emerald-800">
            verified
          </span>
        ) : null}
      </div>
    );
  }
  return (
    <p data-item={`error:${item.message}`} className="text-sm text-red-700">
      {item.message}
      {item.detail ? ` (${item.detail})` : ""}
    </p>
  );
}

function PassRow({ pass }: { pass: PassView }) {
  return (
    <div data-pass={`${pass.pass}:${pass.attempt}`} className="text-sm">
      <p className="text-muted-foreground flex items-center gap-2 text-xs">
        <span className="font-mono uppercase">{pass.pass}</span>
        <span>{pass.status}</span>
        {pass.model ? <span className="font-mono">{pass.model}</span> : null}
        {pass.attempt > 1 ? <span>attempt {pass.attempt}</span> : null}
        {pass.durationMs !== undefined ? <span>{pass.durationMs} ms</span> : null}
      </p>
      {pass.text ? (
        <p className="whitespace-pre-wrap">
          {pass.text}
          {pass.streaming ? <span aria-hidden>▌</span> : null}
        </p>
      ) : null}
      {pass.retryReason ? <p className="text-xs text-amber-700">retry: {pass.retryReason}</p> : null}
    </div>
  );
}
