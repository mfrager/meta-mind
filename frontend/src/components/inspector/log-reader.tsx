import { useQuery } from "@tanstack/react-query";
import { XIcon } from "lucide-react";

import { fetchLogs } from "@/lib/api/files.ts";
import { clockAt, errorMessage } from "@/lib/inspector/format.ts";
import { cn } from "@/lib/utils";

import { Chip, UnavailableRow } from "./section.tsx";

export function LogReader({
  sessionId,
  entryId,
  onClose,
}: {
  sessionId: string;
  entryId: string | null;
  onClose?: () => void;
}) {
  const logs = useQuery({
    queryKey: ["inspector", "logs", sessionId],
    queryFn: () => fetchLogs(sessionId, { source: "all", limit: 100 }),
  });
  const selected = entryId ? logs.data?.entries.find((entry) => entry.id === entryId) : null;
  return (
    <div className="flex min-h-0 flex-1 flex-col" data-panel="log">
      <div className="border-border/70 flex items-center gap-2 border-b px-3 py-2">
        <span className="flex-1 text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          RDF log
        </span>
        <Chip>{logs.data?.entries.length ?? 0} entries</Chip>
        {onClose ? (
          <button aria-label="Close log" className="text-muted-foreground hover:text-foreground" onClick={onClose} type="button">
            <XIcon className="size-3" />
          </button>
        ) : null}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {logs.isError ? <UnavailableRow section="log" reason={errorMessage(logs.error)} /> : null}
        {logs.isPending ? <p className="text-[12px] text-muted-foreground">Loading log entries…</p> : null}
        {selected ? (
          <article className="border-border mb-2 rounded-sm border p-2">
            <p className="flex items-center gap-1.5 font-mono text-[10px] text-muted-foreground">
              <span>{clockAt(selected.at)}</span>
              <Chip>{selected.level}</Chip>
              <Chip>{selected.kind}</Chip>
              <span className="ml-auto">{selected.source}</span>
            </p>
            <pre className="mt-1 whitespace-pre-wrap font-mono text-[11px]">{selected.message}</pre>
          </article>
        ) : entryId ? (
          <p className="mb-2 text-[11px] text-muted-foreground">
            Entry <span className="font-mono">{entryId}</span> is not in the latest page.
          </p>
        ) : null}
        {!logs.isPending && logs.data?.entries.length === 0 ? (
          <p className="text-[12px] text-muted-foreground">No log entries are recorded for this session.</p>
        ) : null}
        <ul className="flex flex-col gap-0.5">
          {logs.data?.entries.slice(0, 60).map((entry) => (
            <li
              className={cn(
                "flex items-baseline gap-1.5 rounded-sm px-1 py-0.5 text-[11px]",
                entry.id === entryId ? "bg-surface" : "hover:bg-muted/40",
              )}
              key={entry.id}
            >
              <span className="shrink-0 font-mono text-[9px] text-muted-foreground">{clockAt(entry.at)}</span>
              <Chip>{entry.level}</Chip>
              <span className="shrink-0 font-mono text-[9px]">{entry.kind}</span>
              <span className="min-w-0 flex-1 truncate text-muted-foreground" title={entry.message}>
                {entry.message}
              </span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
