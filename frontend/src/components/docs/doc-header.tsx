import { IconChip } from "@/components/kit/icon-chip.tsx";
import { StatusChip } from "@/components/kit/status-chip.tsx";
import { DOC_KIND_META } from "@/components/docs/registry.tsx";
import type { FocusDoc } from "@/lib/wire/focus.ts";

const STATUS_TONE = {
  draft: "unavailable",
  saved: "ok",
  live: "ok",
  stale: "degraded",
  running: "degraded",
  verified: "ok",
  failed: "error",
  blocked: "error",
  unavailable: "unavailable",
  degraded: "degraded",
  "promotion-pending": "degraded",
  promoted: "ok",
  "rolled-back": "error",
} as const;

export function DocHeader({ doc, onAsk }: { doc: FocusDoc; onAsk?: () => void }) {
  const meta = DOC_KIND_META[doc.kind];
  return (
    <div className="flex items-start gap-2" data-doc-header>
      <IconChip kind={meta.chip} />
      <div className="min-w-0 flex-1">
        <h2 className="text-h2 truncate">{doc.title}</h2>
        {doc.subtitle ? <p className="font-mono text-value text-muted-foreground">{doc.subtitle}</p> : null}
      </div>
      <StatusChip tone={STATUS_TONE[doc.status]} label={doc.status} reason={doc.reason ?? undefined} />
      {onAsk ? (
        <button type="button" className="text-small rounded-md border border-border px-2 py-1" data-ask-about onClick={onAsk}>
          Ask about this
        </button>
      ) : null}
      <span className="micro-label text-muted-foreground">{meta.label}</span>
    </div>
  );
}
