import { useEffect, useState } from "react";

import { Button } from "@/components/ui/actions.tsx";
import { fetchProvenance, type Provenance } from "@/lib/api/focus.ts";
import type { FocusDoc } from "@/lib/wire/focus.ts";

export function DocFooter({
  doc,
  dirty,
  onSaveDraft,
  onChangeset,
  onHistory,
  onLineage,
  onExport,
}: {
  doc: FocusDoc;
  dirty: number;
  onSaveDraft: () => void;
  onChangeset: () => void;
  onHistory: () => void;
  onLineage: () => void;
  onExport: (format: "json" | "csv" | "png" | "md") => void;
}) {
  const [provenance, setProvenance] = useState<Provenance | null>(null);
  useEffect(() => {
    let cancelled = false;
    void fetchProvenance(doc.id).then((value) => {
      if (!cancelled) setProvenance(value);
    }).catch(() => {
      if (!cancelled) setProvenance(null);
    });
    return () => {
      cancelled = true;
    };
  }, [doc.id, doc.version]);

  const production = Boolean((doc.snapshot as { production?: boolean } | null)?.production);
  return (
    <footer className="border-border flex flex-wrap items-center gap-2 border-t px-3 py-2" data-doc-footer>
      <p className="text-small text-muted-foreground font-mono">
        {provenance ? `events ${provenance.events}` : "events …"}
        {provenance?.traceId ? ` · trace ${provenance.traceId}` : ""}
        {` · ${provenance?.createdBy ?? doc.source.kind}`}
      </p>
      <span className="text-small">{dirty === 0 ? "0 unsaved fields" : `${dirty} unsaved fields`}</span>
      <span className="font-mono text-value">v{doc.version}</span>
      {production ? (
        <Button size="sm" onClick={onChangeset}>
          Save as change set…
        </Button>
      ) : doc.capabilities.includes("edit") ? (
        <Button size="sm" variant="outline" onClick={onSaveDraft}>
          Save draft
        </Button>
      ) : null}
      {doc.capabilities.includes("export") ? (
        <span className="flex gap-1">
          <Button size="xs" variant="ghost" onClick={() => onExport("json")}>JSON</Button>
          <Button size="xs" variant="ghost" onClick={() => onExport("csv")}>CSV</Button>
          <Button size="xs" variant="ghost" onClick={() => onExport("png")}>PNG</Button>
          <Button size="xs" variant="ghost" onClick={() => onExport("md")}>MD</Button>
        </span>
      ) : null}
      {doc.capabilities.includes("history") ? (
        <Button size="xs" variant="ghost" onClick={onHistory}>History</Button>
      ) : null}
      {doc.capabilities.includes("provenance") ? (
        <Button size="xs" variant="ghost" onClick={onLineage}>View lineage</Button>
      ) : null}
    </footer>
  );
}
