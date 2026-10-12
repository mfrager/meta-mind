import { useEffect, useState } from "react";

import { fetchHistory, type HistoryEntry } from "@/lib/api/focus.ts";
import { Drawer } from "@/components/ui/overlays.tsx";

export function HistoryDrawer({ id, open }: { id: string; open: boolean }) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void fetchHistory(id).then((value) => {
      if (!cancelled) setEntries(value);
    }).catch(() => {
      if (!cancelled) setEntries([]);
    });
    return () => {
      cancelled = true;
    };
  }, [id, open]);
  if (!open) return null;
  return (
    <Drawer title="History">
      {entries.length === 0 ? <p className="text-small text-muted-foreground">No saved versions yet.</p> : null}
      <ol className="flex flex-col gap-1">
        {entries.map((entry) => (
          <li key={entry.version} className="font-mono text-value">
            v{entry.version} · {entry.actor} · {entry.status}
          </li>
        ))}
      </ol>
    </Drawer>
  );
}

export function LineageDrawer({ open, events, createdBy }: { open: boolean; events: number; createdBy: string }) {
  if (!open) return null;
  return (
    <Drawer title="Lineage">
      <p className="text-small">
        {events} recorded {events === 1 ? "version" : "versions"} · created by {createdBy}
      </p>
      <p className="text-small text-muted-foreground mt-2">Provenance events are on the provenance tree.</p>
    </Drawer>
  );
}
