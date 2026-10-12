import { OpBody } from "@/components/chat/op-registry.tsx";
import { PhaseRail } from "@/components/chat/phase-rail.tsx";
import type { TranscriptItem } from "@/lib/transcript/reducer.ts";

export function Turn({ item }: { item: Extract<TranscriptItem, { kind: "turn" }> }) {
  return (
    <article data-turn={item.id} className="space-y-2 rounded-md border border-border p-2">
      <PhaseRail active={item.phase} />
      {item.text ? <p className="whitespace-pre-wrap text-sm">{item.text}</p> : null}
      {item.ops.map((op) => (
        <section key={op.id} data-op={op.name} className="rounded-md bg-muted/40 p-2 text-sm">
          <p className="font-mono text-xs">{op.name}</p>
          <OpBody name={op.name} output={op.output} />
        </section>
      ))}
    </article>
  );
}
